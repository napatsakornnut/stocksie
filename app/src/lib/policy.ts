// Reflex policy-gate types and pure helpers (plan 006).
//
// The modelless Reflex engine (https://reflex.gist.rs, loopback HTTP) acts as
// the household's free, deterministic policy floor between the LLM's draft and
// the human's on-chain signature:
//
//   draft ──▶ Reflex /decide ("is this covered by a standing household rule?")
//              ├─ yes, confidence ≥ floor ──▶ `covered` badge (still needs a signature)
//              └─ abstain / low confidence / no ──▶ `review` (the default anyway)
//
// Reflex can NEVER approve, block, or sign — it only annotates. Everything
// here is pure so the wire shapes and the verdict mapping are unit-tested
// without a running engine. Wire contract per the reflex-integration skill
// (riir-reflex 0.1.x): POST /decide `{state, questions[]}` →
// `{answers[], routing{lane, reason}, calibration{}}`, noul outcome
// `{"noul":{"yes":bool}}`, abstain = `outcome: null`.

// ---------------------------------------------------------------------------
// Request side
// ---------------------------------------------------------------------------

/** One draft row submitted for a policy check (mirrors an AgentClient row). */
export interface PolicyDraft {
  key: string;
  itemText: string;
  amountSol: string;
  unitCostText: string;
}

/** Read-only household facts that flavor the decision state. */
export interface PolicyContext {
  vaultBalanceSol: string;
  pendingRequestCount: number;
}

export interface PolicyRequestBody {
  drafts: PolicyDraft[];
  context: PolicyContext;
}

const MAX_POLICY_DRAFTS = 20;
const MAX_TEXT_LEN = 200;

function clampInt(value: unknown, min: number, max: number, fallback: number): number {
  const n = Math.round(Number(value));
  if (!Number.isFinite(n)) return fallback;
  return Math.min(max, Math.max(min, n));
}

function clampText(value: unknown): string {
  return typeof value === "string" ? value.trim().slice(0, MAX_TEXT_LEN) : "";
}

/** Validate the /api/policy request body; `null` when malformed. */
export function parsePolicyRequest(
  body: unknown
): PolicyRequestBody | { error: string } | null {
  if (body === null || typeof body !== "object") return null;
  const obj = body as Record<string, unknown>;
  if (!Array.isArray(obj.drafts) || obj.drafts.length === 0) {
    return { error: "drafts must be a non-empty array." };
  }
  const drafts: PolicyDraft[] = [];
  for (const entry of obj.drafts.slice(0, MAX_POLICY_DRAFTS)) {
    if (entry === null || typeof entry !== "object") continue;
    const raw = entry as Record<string, unknown>;
    const itemText = clampText(raw.itemText);
    if (itemText.length === 0) continue;
    drafts.push({
      key: clampText(raw.key) || `draft-${drafts.length}`,
      itemText,
      amountSol: clampText(raw.amountSol),
      unitCostText: clampText(raw.unitCostText),
    });
  }
  if (drafts.length === 0) {
    return { error: "no usable drafts (each needs an item text)." };
  }
  const context = (obj.context ?? {}) as Partial<PolicyContext>;
  return {
    drafts,
    context: {
      vaultBalanceSol:
        typeof context.vaultBalanceSol === "string"
          ? context.vaultBalanceSol
          : "unknown",
      pendingRequestCount: clampInt(context.pendingRequestCount ?? 0, 0, 1_000_000, 0),
    },
  };
}

// ---------------------------------------------------------------------------
// /decide wire builders
// ---------------------------------------------------------------------------

/**
 * The decision state — front-loaded with the facts (skill §4: "the corpus
 * gates read the state; a bare subject line starves them").
 */
export function policyStateText(draft: PolicyDraft, context: PolicyContext): string {
  return [
    "Household purchase draft awaiting review.",
    `Item: ${draft.itemText}.`,
    `Spending limit: ${draft.amountSol || "unspecified"} SOL.`,
    `Price note: ${draft.unitCostText || "none"}.`,
    `Household vault balance: ${context.vaultBalanceSol} SOL.`,
    `Pending purchase requests: ${context.pendingRequestCount}.`,
    "Decide against the household's standing purchase rules.",
  ].join(" ");
}

/** The single typed question asked about every draft. */
export function policyQuestion(): {
  id: string;
  kind: "noul";
  prompt: string;
} {
  return {
    id: "policy",
    kind: "noul",
    prompt:
      "Is this draft already covered by one of the household's standing purchase rules (routine staples, standing allowances, pre-authorized categories)?",
  };
}

/** Build the exact JSON body for one Reflex `/decide` call. */
export function buildDecideBody(
  draft: PolicyDraft,
  context: PolicyContext
): { state: string; questions: ReturnType<typeof policyQuestion>[] } {
  return {
    state: policyStateText(draft, context),
    questions: [policyQuestion()],
  };
}

// ---------------------------------------------------------------------------
// Response side
// ---------------------------------------------------------------------------

export type PolicyVerdict =
  | { verdict: "covered"; confidence: number; pYes: number; routing: string }
  | {
      verdict: "review";
      reason: "abstain" | "low-confidence" | "no" | "unparsable";
      confidence: number;
      pYes: number;
      routing: string;
    };

interface DecideAnswer {
  outcome?: unknown;
  probabilities?: unknown;
  confidence?: unknown;
}

interface DecideResponse {
  answers?: DecideAnswer[];
  routing?: { reason?: unknown };
}

/** Extract p(yes) for a noul answer — `probabilities[0]`, else 0. */
function pYesOf(answer: DecideAnswer): number {
  const p = Number(
    Array.isArray(answer.probabilities) ? answer.probabilities[0] : Number.NaN
  );
  return Number.isFinite(p) ? p : 0;
}

/**
 * Map one `/decide` response to a verdict. Falls back to `review` on any
 * surprise — the safe default is always the human review that stocksie
 * requires anyway. Returns `null` only when the response is so malformed
 * that even a `review` verdict has no honest numbers to show.
 */
export function parseDecideResponse(
  raw: unknown,
  floor: number
): PolicyVerdict | null {
  if (raw === null || typeof raw !== "object") return null;
  const res = raw as DecideResponse;
  const answer = res.answers?.[0];
  if (!answer) return null;
  const confidence = Number(answer.confidence);
  const safeConfidence = Number.isFinite(confidence) ? confidence : 0;
  const pYes = pYesOf(answer);
  const routing =
    typeof res.routing?.reason === "string" ? res.routing.reason : "";

  const outcome = answer.outcome as { noul?: { yes?: unknown } } | null;
  if (outcome === null || outcome === undefined) {
    return {
      verdict: "review",
      reason: "abstain",
      confidence: safeConfidence,
      pYes,
      routing,
    };
  }
  if (typeof outcome !== "object" || typeof outcome.noul?.yes !== "boolean") {
    return {
      verdict: "review",
      reason: "unparsable",
      confidence: safeConfidence,
      pYes,
      routing,
    };
  }
  if (!outcome.noul.yes) {
    return {
      verdict: "review",
      reason: "no",
      confidence: safeConfidence,
      pYes,
      routing,
    };
  }
  if (safeConfidence < floor) {
    return {
      verdict: "review",
      reason: "low-confidence",
      confidence: safeConfidence,
      pYes,
      routing,
    };
  }
  return {
    verdict: "covered",
    confidence: safeConfidence,
    pYes,
    routing,
  };
}

"use client";

// AgentClient — the AI household agent (plan 005).
//
// The security story in one line: **AI proposes, the household approves
// on-chain.** The model (reached via /api/ai, which holds the API key) has no
// chain access and no authority. It turns a receipt photo or a chat message
// into *draft* purchase requests; everything lands in one review tray where
// every field is editable, and the connected wallet is what signs the real
// `create_purchase_request` transactions — straight into the same approval
// flow the program already enforces (Owner/Parent approve, 0.0001–0.5 SOL
// circuit breaker, blake3 privacy hashes).
//
// Two input paths, one pipeline:
//   1. Receipt photo → vision model → draft rows
//   2. Chat (Thai/English) → intent JSON → draft rows or a grounded budget
//      answer computed ONLY from the read-only household snapshot this page
//      attaches to the request.
//
// Prices arrive in everyday money (THB satang) and are converted with the
// live CoinGecko THB/SOL rate, falling back to NEXT_PUBLIC_SOL_THB_FALLBACK.

import { useEffect, useMemo, useRef, useState } from "react";
import Link from "next/link";
import BN from "bn.js";
import type { PublicKey } from "@solana/web3.js";
import { useHouseholdContext } from "@/hooks/useHouseholdContext";
import { useHousehold } from "@/hooks/useHousehold";
import { useRefresh, useTransactionWithRefresh } from "@/hooks/useRefresh";
import { useProgram } from "@/lib/program";
import { SYSTEM_PROGRAM_ID, memberPda, purchasePda } from "@/lib/accounts";
import { toHash32 } from "@/lib/hashes";
import { solToLamports, lamportsToSol } from "@/lib/format";
import {
  MIN_REQUEST_LAMPORTS,
  MAX_REIMBURSEMENT_LAMPORTS,
} from "@/lib/constants";
import {
  solThbFallbackFromEnv,
} from "@/lib/fx";
import {
  draftFromParsedItem,
  type ParsedItem,
  type HouseholdContext,
} from "@/lib/ai";
import {
  type PolicyVerdict,
  type PolicyDraft,
} from "@/lib/policy";
import { statusFromAnchor, STATUS_LABELS, type Status } from "@/lib/types";
import { SectionCard } from "@/components/budget/SectionCard";
import { Field } from "@/components/ui/Field";
import { Button } from "@/components/ui/Button";
import { ResultBanner } from "@/components/ui/ResultBanner";
import { ConnectGate } from "@/components/ui/ConnectGate";
import { WalletButton } from "@/components/WalletButton";
import { ThemeToggle } from "@/components/ui/ThemeToggle";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/** One editable row in the review tray. Amount stays a string while editing. */
interface DraftRow {
  key: string;
  itemText: string;
  unitCostText: string;
  amountInput: string;
  selected: boolean;
}

interface ChatBubble {
  role: "user" | "assistant";
  text: string;
  addedDrafts?: number;
}

// ---------------------------------------------------------------------------
// API helper
// ---------------------------------------------------------------------------

type AgentApiResponse =
  | {
      ok: true;
      mode: "receipt";
      currency: string;
      items: ParsedItem[];
    }
  | {
      ok: true;
      mode: "chat";
      intent: "propose_purchases" | "budget_answer" | "clarify";
      reply: string;
      items?: ParsedItem[];
    }
  | { ok: false; error: string };

async function callAgent(body: unknown): Promise<AgentApiResponse> {
  try {
    const res = await fetch("/api/ai", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
    const json = (await res.json().catch(() => null)) as
      | (Record<string, unknown> & { ok?: boolean })
      | null;
    if (!res.ok || !json || json.ok !== true) {
      return {
        ok: false,
        error:
          (typeof json?.error === "string" && json.error) ||
          `AI request failed (${res.status}).`,
      };
    }
    return json as unknown as AgentApiResponse;
  } catch {
    return { ok: false, error: "Could not reach the AI endpoint." };
  }
}

// ---------------------------------------------------------------------------
// Policy gate (plan 006) — modelless Reflex check against household rules
// ---------------------------------------------------------------------------

interface PolicyApiResponse {
  ok?: boolean;
  available: boolean;
  results: { key: string; verdict: PolicyVerdict }[];
}

/** Ask the local Reflex engine which drafts match a standing household rule. */
async function callPolicy(
  drafts: PolicyDraft[],
  context: { vaultBalanceSol: string; pendingRequestCount: number }
): Promise<PolicyApiResponse> {
  try {
    const res = await fetch("/api/policy", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ drafts, context }),
    });
    const json = (await res.json().catch(() => null)) as
      | (PolicyApiResponse & Record<string, unknown>)
      | null;
    if (!res.ok || !json || json.ok !== true) {
      return { available: false, results: [] };
    }
    return json;
  } catch {
    return { available: false, results: [] };
  }
}

/** Short row badge for a policy verdict; `null` → show nothing. */
function policyBadgeLabel(verdict: PolicyVerdict): string {
  return verdict.verdict === "covered"
    ? "✓ matches a household rule"
    : "no rule match — manual review";
}

// ---------------------------------------------------------------------------
// Root
// ---------------------------------------------------------------------------

export function AgentClient() {
  // SSR-safety for wallet-derived UI (same pattern as BudgetDashboard).
  const [mounted, setMounted] = useState(false);
  useEffect(() => setMounted(true), []);

  // Live THB/SOL rate with a graceful offline fallback.
  const [solThb, setSolThb] = useState<number>(solThbFallbackFromEnv);
  const [rateLive, setRateLive] = useState(false);
  useEffect(() => {
    let alive = true;
    fetch(
      "https://api.coingecko.com/api/v3/simple/price?ids=solana&vs_currencies=thb"
    )
      .then((r) => (r.ok ? r.json() : null))
      .then((j: { solana?: { thb?: number } } | null) => {
        const thb = j?.solana?.thb;
        if (alive && typeof thb === "number" && thb > 0) {
          setSolThb(thb);
          setRateLive(true);
        }
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);
  const solMinorPerUnit = solThb * 100;

  return (
    <main className="mx-auto max-w-6xl px-6 py-10">
      <header className="flex flex-wrap items-start justify-between gap-4">
        <div className="flex flex-col gap-1">
          <Link
            href="/"
            className="inline-flex w-fit items-center gap-1 text-xs font-medium text-stone-500 transition-colors hover:text-stone-700 dark:text-slate-400 dark:hover:text-slate-200"
          >
            <span aria-hidden="true">←</span> Back to dashboard
          </Link>
          <h1 className="flex items-center gap-2 text-2xl font-semibold tracking-tight text-stone-800 dark:text-slate-100">
            <span aria-hidden="true">🤖</span>
            Agent
          </h1>
          <p className="text-sm text-stone-500 dark:text-slate-400">
            Snap a receipt or just ask — the AI drafts purchase requests, you
            review, the household approves on-chain.
          </p>
        </div>
        <div className="flex items-center gap-2">
          <ThemeToggle />
          <WalletButton />
        </div>
      </header>

      <section className="mt-4 flex flex-wrap items-center gap-2">
        <span className="inline-flex items-center gap-2 rounded-full border border-stone-300 bg-white px-3 py-1 text-xs text-stone-500 dark:border-slate-700 dark:bg-slate-900/40 dark:text-slate-400">
          <span aria-hidden="true">💱</span>
          1 SOL ≈ ฿{solThb.toLocaleString(undefined, { maximumFractionDigits: 0 })}
          <span className="text-stone-400 dark:text-slate-600">
            ({rateLive ? "live rate" : "fallback rate"})
          </span>
        </span>
        <span className="inline-flex items-center gap-2 rounded-full border border-violet-500/30 bg-violet-50 px-3 py-1 text-xs text-violet-700 dark:bg-violet-500/10 dark:text-violet-300">
          <span aria-hidden="true">🔐</span>
          AI can only propose — every request needs a human signature
        </span>
      </section>

      <div className="mt-6">
        <ConnectGate>
          <AgentWorkspace solMinorPerUnit={solMinorPerUnit} />
        </ConnectGate>
      </div>
    </main>
  );
}

// ---------------------------------------------------------------------------
// Workspace — chat + receipt + drafts, sharing one tray
// ---------------------------------------------------------------------------

function AgentWorkspace({ solMinorPerUnit }: { solMinorPerUnit: number }) {
  const { household, connectedWallet, callerMember } = useHouseholdContext();
  const { nonce } = useRefresh();
  const householdData = useHousehold(nonce);

  const [drafts, setDrafts] = useState<DraftRow[]>([]);
  const [chat, setChat] = useState<ChatBubble[]>([]);
  // Reflex policy verdicts per draft key (plan 006). Absent = unchecked or
  // engine unavailable — never a gate, purely an annotation.
  const [verdicts, setVerdicts] = useState<Record<string, PolicyVerdict>>({});

  // Read-only household snapshot shared by the chat grounding and the policy
  // gate — computed once here so both consumers see identical numbers.
  const householdSnapshot = useMemo(() => {
    const hh = householdData?.data?.household ?? null;
    const requests = householdData?.data?.requests ?? [];
    const pending = requests.filter(
      (r) => statusFromAnchor(r.account.status) === "pending"
    ).length;
    const recent = requests.slice(-5).map((r) => {
      const status = statusFromAnchor(r.account.status) as Status | null;
      return `#${r.account.requestId.toString(10)} ${
        status ? STATUS_LABELS[status] : "unknown"
      } for ${lamportsToSol(r.account.amountLamports.toString(10))} SOL`;
    });
    return {
      forAi: {
        vaultBalanceSol: hh
          ? lamportsToSol(hh.vaultBalance.toString(10))
          : "unknown",
        pendingRequestCount: pending,
        memberCount: hh?.memberCount ?? 0,
        recentRequests: recent,
      } satisfies HouseholdContext,
      forPolicy: {
        vaultBalanceSol: hh
          ? lamportsToSol(hh.vaultBalance.toString(10))
          : "unknown",
        pendingRequestCount: pending,
      },
    };
  }, [householdData]);

  /** Ask the Reflex policy gate about the given rows; stores verdicts. */
  const runPolicyCheck = (rows: DraftRow[]) => {
    if (rows.length === 0) return;
    const payload: PolicyDraft[] = rows.map((row) => ({
      key: row.key,
      itemText: row.itemText,
      amountSol: row.amountInput,
      unitCostText: row.unitCostText,
    }));
    void callPolicy(payload, householdSnapshot.forPolicy).then((res) => {
      if (!res.available) return;
      setVerdicts((prev) => {
        const next = { ...prev };
        for (const { key, verdict } of res.results) next[key] = verdict;
        return next;
      });
    });
  };

  const addItems = (
    items: ParsedItem[],
    currency: string,
    source: "receipt" | "estimate"
  ) => {
    const rows = items.map((item, i) =>
      rowFromItem(item, currency, solMinorPerUnit, drafts.length + i, source)
    );
    setDrafts((prev) => [...prev, ...rows]);
    // Policy check is an annotation, not a gate — fire and forget.
    runPolicyCheck(rows);
    return rows.length;
  };

  return (
    <div className="grid gap-6 lg:grid-cols-2">
      <ChatSection
        chat={chat}
        setChat={setChat}
        householdSnapshot={householdSnapshot.forAi}
        addItems={addItems}
      />
      <ReceiptSection
        addItems={addItems}
      />
      <DraftsTray
        drafts={drafts}
        setDrafts={setDrafts}
        verdicts={verdicts}
        onPolicyCheck={() => runPolicyCheck(drafts)}
        household={household}
        connectedWallet={connectedWallet}
        callerMember={callerMember}
      />
    </div>
  );
}

/** AI line → editable tray row (amount pre-filled in SOL, still editable). */
function rowFromItem(
  item: ParsedItem,
  currency: string,
  solMinorPerUnit: number,
  index: number,
  source: "receipt" | "estimate"
): DraftRow {
  const draft = draftFromParsedItem(item, currency, solMinorPerUnit, index, source);
  return {
    key: `${draft.key}-${index}`,
    itemText: draft.itemText,
    unitCostText: draft.unitCostText,
    amountInput: draft.amountLamports ? lamportsToSol(draft.amountLamports) : "",
    selected: true,
  };
}

// ---------------------------------------------------------------------------
// Chat section
// ---------------------------------------------------------------------------

function ChatSection({
  chat,
  setChat,
  householdSnapshot,
  addItems,
}: {
  chat: ChatBubble[];
  setChat: React.Dispatch<React.SetStateAction<ChatBubble[]>>;
  /** Read-only on-chain snapshot attached to every chat request (grounding). */
  householdSnapshot: HouseholdContext;
  addItems: (
    items: ParsedItem[],
    currency: string,
    source: "receipt" | "estimate"
  ) => number;
}) {
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    listRef.current?.scrollTo({ top: listRef.current.scrollHeight });
  }, [chat, busy]);

  const send = async () => {
    const text = input.trim();
    if (text.length === 0 || busy) return;
    setError(null);
    setInput("");
    setChat((prev) => [...prev, { role: "user", text }]);
    setBusy(true);
    const res = await callAgent({ mode: "chat", text, context: householdSnapshot });
    setBusy(false);
    if (!res.ok) {
      setError(res.error);
      return;
    }
    if (res.mode === "chat" && res.intent === "propose_purchases" && res.items) {
      const added = addItems(res.items, "THB", "estimate");
      setChat((prev) => [...prev, { role: "assistant", text: res.reply, addedDrafts: added }]);
    } else if (res.mode === "chat") {
      setChat((prev) => [...prev, { role: "assistant", text: res.reply }]);
    }
  };

  return (
    <SectionCard
      title="Ask the agent"
      icon="💬"
      description="“ซื้อนม 2 ขวด งบ 200 บาท” or “how much budget is left?” — proposals land in the review tray below; budget answers use only the live on-chain numbers."
    >
      <div
        ref={listRef}
        className="flex max-h-72 min-h-32 flex-col gap-2 overflow-y-auto rounded-lg border border-stone-200 bg-stone-50/60 p-3 dark:border-slate-800 dark:bg-slate-950/40"
      >
        {chat.length === 0 && !busy && (
          <p className="text-xs text-stone-400 dark:text-slate-600">
            Try: “ซื้อข้าว 5 กิโล กับนม 2 ขวด” — or ask about the balance.
          </p>
        )}
        {chat.map((bubble, i) => (
          <div
            key={i}
            className={
              bubble.role === "user"
                ? "self-end rounded-lg rounded-br-sm bg-emerald-600 px-3 py-2 text-xs text-white"
                : "max-w-[90%] self-start rounded-lg rounded-bl-sm bg-white px-3 py-2 text-xs text-stone-700 shadow-sm dark:bg-slate-900 dark:text-slate-200"
            }
          >
            {bubble.text}
            {bubble.addedDrafts ? (
              <span className="mt-1 block text-[11px] text-violet-600 dark:text-violet-300">
                ↓ added {bubble.addedDrafts} draft
                {bubble.addedDrafts > 1 ? "s" : ""} to the review tray
              </span>
            ) : null}
          </div>
        ))}
        {busy && (
          <div className="self-start rounded-lg bg-white px-3 py-2 text-xs text-stone-400 shadow-sm dark:bg-slate-900 dark:text-slate-500">
            Thinking…
          </div>
        )}
      </div>
      {error && (
        <p className="rounded-lg border border-rose-500/30 bg-rose-50 px-3 py-2 text-xs text-rose-700 dark:bg-rose-500/10 dark:text-rose-300">
          {error}
        </p>
      )}
      <div className="flex gap-2">
        <input
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              void send();
            }
          }}
          placeholder="Type a request or a question…"
          className="flex-1 rounded-lg border border-stone-200 bg-white px-3 py-2 text-sm text-stone-800 outline-none transition placeholder:text-stone-400 focus:border-violet-500/70 focus:ring-1 focus:ring-violet-500/40 dark:border-slate-800 dark:bg-slate-950/60 dark:text-slate-100 dark:placeholder:text-slate-600"
          disabled={busy}
        />
        <Button onClick={() => void send()} loading={busy} disabled={input.trim().length === 0}>
          Send
        </Button>
      </div>
    </SectionCard>
  );
}

// ---------------------------------------------------------------------------
// Receipt section
// ---------------------------------------------------------------------------

function ReceiptSection({
  addItems,
}: {
  addItems: (
    items: ParsedItem[],
    currency: string,
    source: "receipt" | "estimate"
  ) => number;
}) {
  const [preview, setPreview] = useState<string | null>(null);
  const [image, setImage] = useState<{ base64: string; mimeType: string } | null>(
    null
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [addedNote, setAddedNote] = useState<string | null>(null);
  const fileRef = useRef<HTMLInputElement>(null);

  const onFile = (file: File | null) => {
    setError(null);
    setAddedNote(null);
    if (!file) return;
    if (!["image/png", "image/jpeg", "image/webp"].includes(file.type)) {
      setError("Please pick a PNG, JPEG, or WebP image.");
      return;
    }
    const reader = new FileReader();
    reader.onload = () => {
      const dataUrl = String(reader.result ?? "");
      const comma = dataUrl.indexOf(",");
      if (comma === -1) {
        setError("Could not read that image.");
        return;
      }
      setPreview(dataUrl);
      setImage({ base64: dataUrl.slice(comma + 1), mimeType: file.type });
    };
    reader.readAsDataURL(file);
  };

  const parse = async () => {
    if (!image || busy) return;
    setBusy(true);
    setError(null);
    setAddedNote(null);
    const res = await callAgent({ mode: "receipt", image });
    setBusy(false);
    if (!res.ok) {
      setError(res.error);
      return;
    }
    if (res.mode !== "receipt") return; // receipt endpoint always answers receipt mode
    const added = addItems(res.items, res.currency, "receipt");
    setAddedNote(
      `Found ${added} item${added > 1 ? "s" : ""} — review the amounts below, then submit.`
    );
  };

  return (
    <SectionCard
      title="Scan a receipt"
      icon="🧾"
      description="Snap the receipt after a shopping run — the AI extracts each line as a draft purchase request so you can get reimbursed without typing."
    >
      <div className="flex items-start gap-3">
        <div
          className="flex h-32 w-32 shrink-0 cursor-pointer items-center justify-center overflow-hidden rounded-lg border border-dashed border-stone-300 bg-stone-50/60 dark:border-slate-700 dark:bg-slate-950/40"
          onClick={() => fileRef.current?.click()}
          role="button"
          tabIndex={0}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") fileRef.current?.click();
          }}
        >
          {preview ? (
            // eslint-disable-next-line @next/next/no-img-element
            <img src={preview} alt="Receipt preview" className="h-full w-full object-cover" />
          ) : (
            <span className="px-2 text-center text-[11px] text-stone-400 dark:text-slate-600">
              tap to pick a photo
            </span>
          )}
        </div>
        <input
          ref={fileRef}
          type="file"
          accept="image/png,image/jpeg,image/webp"
          className="hidden"
          onChange={(e) => onFile(e.target.files?.[0] ?? null)}
        />
        <div className="flex flex-1 flex-col gap-2">
          <p className="text-xs text-stone-500 dark:text-slate-400">
            Prices are converted to SOL with the live THB rate — adjust any
            draft before it goes on-chain.
          </p>
          <div className="flex gap-2">
            <Button onClick={() => void parse()} loading={busy} disabled={!image}>
              {busy ? "Reading…" : "Extract items"}
            </Button>
            {image && (
              <Button
                variant="ghost"
                size="sm"
                onClick={() => {
                  setPreview(null);
                  setImage(null);
                  setAddedNote(null);
                  setError(null);
                }}
              >
                Clear
              </Button>
            )}
          </div>
          {addedNote && (
            <p className="text-xs text-emerald-600 dark:text-emerald-400">
              {addedNote}
            </p>
          )}
          {error && (
            <p className="rounded-lg border border-rose-500/30 bg-rose-50 px-3 py-2 text-xs text-rose-700 dark:bg-rose-500/10 dark:text-rose-300">
              {error}
            </p>
          )}
        </div>
      </div>
    </SectionCard>
  );
}

// ---------------------------------------------------------------------------
// Drafts tray → on-chain
// ---------------------------------------------------------------------------

function DraftsTray({
  drafts,
  setDrafts,
  verdicts,
  onPolicyCheck,
  household,
  connectedWallet,
  callerMember,
}: {
  drafts: DraftRow[];
  setDrafts: React.Dispatch<React.SetStateAction<DraftRow[]>>;
  /** Reflex policy verdicts keyed by draft key (absent = unchecked). */
  verdicts: Record<string, PolicyVerdict>;
  /** Re-run the household-rules check over the current drafts. */
  onPolicyCheck: () => void;
  household: PublicKey | null;
  connectedWallet: PublicKey | null;
  callerMember: PublicKey | null;
}) {
  const program = useProgram();
  const tx = useTransactionWithRefresh();
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(
    null
  );

  const patch = (key: string, changes: Partial<DraftRow>) =>
    setDrafts((prev) =>
      prev.map((row) => (row.key === key ? { ...row, ...changes } : row))
    );

  const rowsWithErrors = useMemo(
    () =>
      drafts.map((row) => ({
        row,
        error: draftAmountError(row.amountInput),
      })),
    [drafts]
  );

  const selectedRows = rowsWithErrors.filter(
    ({ row, error }) => row.selected && !error
  );

  const canSubmit =
    !!program &&
    !!household &&
    !!connectedWallet &&
    !!callerMember &&
    selectedRows.length > 0 &&
    !tx.pending;

  const submitSelected = () => {
    if (!program || !household || !connectedWallet || !callerMember) return;
    const rows = selectedRows.map(({ row }) => row);
    const buyerMember = memberPda(household, connectedWallet);
    setProgress({ done: 0, total: rows.length });
    void tx.submit(async () => {
      let done = 0;
      let lastSignature: string | null = null;
      for (const row of rows) {
        const lamports = solToLamports(row.amountInput);
        if (lamports === null) throw new Error(`Invalid amount for “${row.itemText}”.`);
        // Re-read the counter per request so back-to-back creates never
        // collide on the PDA (same race-safe pattern as PurchasePanel).
        const householdAccount = await program.account.household.fetch(household);
        const nextId = BigInt(householdAccount.requestCounter.toString(10)) + 1n;
        const request = purchasePda(household, nextId);
        lastSignature = await program.methods
          .createPurchaseRequest(
            new BN(lamports.toString()),
            toHash32(row.itemText),
            toHash32(row.unitCostText),
            connectedWallet
          )
          .accountsStrict({
            household,
            callerMember,
            request,
            buyerMember,
            caller: connectedWallet,
            systemProgram: SYSTEM_PROGRAM_ID,
          })
          .rpc();
        done += 1;
        setProgress({ done, total: rows.length });
      }
      setDrafts((prev) =>
        prev.filter((row) => !rows.some((r) => r.key === row.key))
      );
      setProgress(null);
      if (lastSignature === null) {
        throw new Error("No requests were submitted.");
      }
      return lastSignature;
    });
  };

  return (
    <SectionCard
      title="Review & submit"
      icon="📝"
      description="Nothing goes on-chain until you submit. Amounts are spending limits in SOL; item text is kept private (blake3-hashed on-chain)."
      className="lg:col-span-2"
      badge={
        drafts.length > 0 ? (
          <span className="rounded-full bg-violet-100 px-2 py-0.5 text-[11px] font-medium text-violet-700 dark:bg-violet-500/10 dark:text-violet-300">
            {drafts.length} draft{drafts.length > 1 ? "s" : ""}
          </span>
        ) : null
      }
    >
      {drafts.length === 0 ? (
        <p className="rounded-lg border border-dashed border-stone-300 dark:border-slate-700 bg-stone-50/50 dark:bg-slate-950/30 px-4 py-6 text-center text-xs text-stone-500 dark:text-slate-400">
          No drafts yet — scan a receipt or ask the agent above.
        </p>
      ) : (
        <div className="flex flex-col gap-3">
          {rowsWithErrors.map(({ row, error }) => (
            <div
              key={row.key}
              className="grid gap-2 rounded-lg border border-stone-200 bg-white p-3 dark:border-slate-800 dark:bg-slate-950/40 sm:grid-cols-[auto_2fr_2fr_1fr_auto]"
            >
              <div className="flex flex-col items-start gap-1">
                <input
                  type="checkbox"
                  checked={row.selected}
                  onChange={(e) => patch(row.key, { selected: e.target.checked })}
                  className="h-4 w-4 accent-violet-600"
                />
                {verdicts[row.key] && (
                  <span
                    title={`confidence ${verdicts[row.key].confidence.toFixed(2)} · ${
                      verdicts[row.key].routing || "reflex"
                    }`}
                    className={
                      verdicts[row.key].verdict === "covered"
                        ? "rounded-full bg-emerald-100 px-2 py-0.5 text-[10px] font-medium text-emerald-700 dark:bg-emerald-500/10 dark:text-emerald-300"
                        : "rounded-full bg-stone-100 px-2 py-0.5 text-[10px] text-stone-500 dark:bg-slate-800 dark:text-slate-400"
                    }
                  >
                    {policyBadgeLabel(verdicts[row.key])}
                  </span>
                )}
              </div>
              <Field
                label="Item / quantity"
                value={row.itemText}
                onChange={(v) => patch(row.key, { itemText: v })}
                helpText="🔒 Private — hashed before it hits the chain."
              />
              <Field
                label="Price note"
                value={row.unitCostText}
                onChange={(v) => patch(row.key, { unitCostText: v })}
                helpText="🔒 Private."
              />
              <Field
                label="Limit"
                value={row.amountInput}
                onChange={(v) => patch(row.key, { amountInput: v })}
                type="number"
                suffix="SOL"
                min="0.0001"
                max="0.5"
                step="0.0001"
                mono
                error={error}
              />
              <button
                type="button"
                onClick={() =>
                  setDrafts((prev) => prev.filter((r) => r.key !== row.key))
                }
                className="self-end text-xs text-stone-400 transition-colors hover:text-rose-500 dark:text-slate-600"
                aria-label={`Remove draft ${row.itemText}`}
              >
                ✕
              </button>
            </div>
          ))}
        </div>
      )}

      <div className="flex flex-wrap items-center gap-3">
        <Button onClick={submitSelected} loading={tx.pending} disabled={!canSubmit}>
          Submit {selectedRows.length > 0 ? `${selectedRows.length} ` : ""}request
          {selectedRows.length === 1 ? "" : "s"}
        </Button>
        {progress && (
          <span className="text-xs text-stone-500 dark:text-slate-400">
            Submitting {progress.done}/{progress.total}…
          </span>
        )}
        {drafts.length > 0 && (
          <>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setDrafts((prev) => prev.map((r) => ({ ...r, selected: true })))}
            >
              Select all
            </Button>
            <Button variant="ghost" size="sm" onClick={onPolicyCheck}>
              <span aria-hidden="true">📋</span>
              Check household rules
            </Button>
          </>
        )}
      </div>
      <ResultBanner
        pending={tx.pending}
        signature={tx.signature}
        error={tx.error}
        onDismiss={tx.reset}
      />
    </SectionCard>
  );
}

/** Shared min/max validation for a draft's SOL amount input. */
function draftAmountError(input: string): string | null {
  if (input.trim().length === 0) return "Enter a limit";
  const lamports = solToLamports(input);
  if (lamports === null) return "Invalid SOL amount";
  if (lamports < BigInt(MIN_REQUEST_LAMPORTS)) return "Below 0.0001 SOL minimum";
  if (lamports > BigInt(MAX_REIMBURSEMENT_LAMPORTS))
    return "Above the 0.5 SOL ceiling";
  return null;
}

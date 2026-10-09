// /api/policy — Reflex policy gate proxy (plan 006).
//
// The modelless Reflex engine answers one typed question per draft: "is this
// already covered by a standing household rule?" — µs-tier, bit-deterministic,
// free, and it ABSTAINS (outcome: null) when the corpus doesn't ground an
// answer. Every non-`covered` outcome maps to `review`, which is stocksie's
// default anyway (the human signs; nothing here can approve, block, or send).
//
// Reflex binds to loopback with CORS closed (skill §2), so the browser cannot
// reach it — this server route is the only caller. If the engine isn't
// running the route answers `{available: false}` and the UI shows no badges;
// the flow is byte-identical to plan 005 without it.
//
// Env (server-side only):
//   REFLEX_URL              default http://127.0.0.1:7331
//   REFLEX_CONFIDENCE_FLOOR default 0.5 — a *starting* value; per the reflex
//                           skill, fit this from a labeled slice (≥16 cases)
//                           before quoting any accuracy number.

import { NextResponse } from "next/server";
import {
  parsePolicyRequest,
  buildDecideBody,
  parseDecideResponse,
  type PolicyDraft,
  type PolicyVerdict,
} from "@/lib/policy";

const DECIDE_TIMEOUT_MS = 2000;

export async function POST(request: Request) {
  let parsedBody;
  try {
    parsedBody = parsePolicyRequest(await request.json());
  } catch {
    return NextResponse.json({ error: "Invalid JSON body." }, { status: 400 });
  }
  if (parsedBody === null) {
    return NextResponse.json({ error: "Invalid request body." }, { status: 400 });
  }
  if ("error" in parsedBody) {
    return NextResponse.json({ error: parsedBody.error }, { status: 400 });
  }

  const baseUrl = (
    process.env.REFLEX_URL?.trim() || "http://127.0.0.1:7331"
  ).replace(/\/+$/, "");
  const floor = (() => {
    const n = Number(process.env.REFLEX_CONFIDENCE_FLOOR);
    return Number.isFinite(n) && n > 0 && n <= 1 ? n : 0.5;
  })();

  const decideOne = async (
    draft: PolicyDraft
  ): Promise<{ key: string; verdict: PolicyVerdict } | null> => {
    const res = await fetch(`${baseUrl}/decide`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(buildDecideBody(draft, parsedBody.context)),
      signal: AbortSignal.timeout(DECIDE_TIMEOUT_MS),
    });
    if (!res.ok) return null;
    const verdict = parseDecideResponse(await res.json(), floor);
    return verdict ? { key: draft.key, verdict } : null;
  };

  try {
    const settled = await Promise.allSettled(
      parsedBody.drafts.map(decideOne)
    );
    const results = settled
      .filter(
        (s): s is PromiseFulfilledResult<{ key: string; verdict: PolicyVerdict }> =>
          s.status === "fulfilled" && s.value !== null
      )
      .map((s) => s.value);

    if (results.length === 0) {
      // Engine not running, unreachable, or refused every draft — the
      // gate is simply absent and the UI falls back to plan-005 behavior.
      return NextResponse.json({ ok: true, available: false, results: [] });
    }
    return NextResponse.json({ ok: true, available: true, results });
  } catch {
    return NextResponse.json({ ok: true, available: false, results: [] });
  }
}

// Currency conversion helpers for the AI agent (plan 005).
//
// Receipts and chat proposals arrive in everyday money (THB satang / USD
// cents — the LLM returns the line total in the currency's *minor* unit) but
// the on-chain program speaks lamports. This module is the single place where
// that conversion happens so the math is unit-tested, not inlined in UI code.
//
// Pure functions only — the live THB/SOL rate is fetched by the caller
// (AgentClient) from CoinGecko and falls back to
// `NEXT_PUBLIC_SOL_THB_FALLBACK`.

/** Fallback THB-per-SOL when no live rate is available (offline demo, rate API down). */
export const DEFAULT_SOL_THB = 5600;

/** Read the fallback rate from the public env (0 / missing → DEFAULT_SOL_THB). */
export function solThbFallbackFromEnv(): number {
  const raw = process.env.NEXT_PUBLIC_SOL_THB_FALLBACK;
  if (!raw) return DEFAULT_SOL_THB;
  const parsed = Number(raw);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : DEFAULT_SOL_THB;
}

/**
 * Convert a line total in minor units (satang / cents) to lamports.
 *
 * `solMinorPerUnit` is how many minor units one SOL is worth, i.e.
 * `solThb * 100`. Rounded to the nearest lamport. Returns `null` for
 * non-finite or non-positive inputs (the caller treats that as "needs edit").
 */
export function minorToLamports(
  totalMinor: number,
  solMinorPerUnit: number
): bigint | null {
  if (!Number.isFinite(totalMinor) || !Number.isFinite(solMinorPerUnit)) {
    return null;
  }
  if (totalMinor <= 0 || solMinorPerUnit <= 0) return null;
  const sol = totalMinor / solMinorPerUnit;
  return BigInt(Math.round(sol * 1_000_000_000));
}

/** Inverse of {@link minorToLamports} — minor units, rounded to 1 minor unit. */
export function lamportsToMinor(
  lamports: bigint,
  solMinorPerUnit: number
): number {
  if (!Number.isFinite(solMinorPerUnit) || solMinorPerUnit <= 0) return 0;
  const sol = Number(lamports) / 1_000_000_000;
  return Math.round(sol * solMinorPerUnit);
}

/** Format a minor-unit amount with its currency's symbol for a draft hint. */
export function formatMinor(totalMinor: number, currency: string): string {
  const symbol = currency === "THB" ? "฿" : `${currency} `;
  return `${symbol}${(totalMinor / 100).toFixed(2)}`;
}

// AI agent types, validators, and prompt builders (plan 005).
//
// The LLM is only ever allowed to *propose* — it has no chain access and no
// authority. Everything it returns passes through the pure normalizers here
// before the UI will render it, and the UI (not the AI) is what signs and
// sends transactions. Prompts pin the reply to a strict JSON shape so the
// normalizers can be small and the demo can't be derailed by chatty output.
//
// `src/app/api/ai/route.ts` (server, holds the API key) and the Agent UI both
// import from this module; keeping it dependency-free lets vitest cover the
// validation logic directly.

import { minorToLamports, formatMinor } from './fx';

// ---------------------------------------------------------------------------
// Shared item shape
// ---------------------------------------------------------------------------

/**
 * One proposed purchase line. `totalMinor` is the LINE TOTAL in the
 * currency's minor unit (satang for THB, cents for USD) — unit price ×
 * quantity is deliberately collapsed into one number by the prompt so the
 * conversion math stays single-step.
 */
export interface ParsedItem {
  name: string;
  quantity: number;
  note?: string;
  totalMinor: number;
}

interface RawItem {
  name?: unknown;
  quantity?: unknown;
  note?: unknown;
  totalMinor?: unknown;
}

const MAX_ITEMS = 20;
const MAX_NAME_LEN = 80;
const MAX_NOTE_LEN = 60;

function clampInt(value: unknown, min: number, max: number, fallback: number): number {
  const n = Math.round(Number(value));
  if (!Number.isFinite(n)) return fallback;
  return Math.min(max, Math.max(min, n));
}

/**
 * Normalize an arbitrary `items` array from the model into safe ParsedItems.
 * Drops (rather than fixes) lines that are unusable — an empty/oversized name
 * or a non-positive total has no sensible repair, and silently editing the
 * model's line would surprise the reviewing user. Hard cap MAX_ITEMS.
 */
export function normalizeParsedItems(raw: unknown): ParsedItem[] {
  if (!Array.isArray(raw)) return [];
  const out: ParsedItem[] = [];
  for (const entry of raw.slice(0, MAX_ITEMS)) {
    if (entry === null || typeof entry !== "object") continue;
    const item = entry as RawItem;
    if (typeof item.name !== "string") continue;
    const name = item.name.trim().slice(0, MAX_NAME_LEN);
    if (name.length === 0) continue;
    const totalMinor = Math.round(Number(item.totalMinor));
    if (!Number.isFinite(totalMinor) || totalMinor <= 0) continue;
    const note =
      typeof item.note === "string" && item.note.trim().length > 0
        ? item.note.trim().slice(0, MAX_NOTE_LEN)
        : undefined;
    out.push({
      name,
      quantity: clampInt(item.quantity, 1, 99, 1),
      note,
      totalMinor,
    });
  }
  return out;
}

// ---------------------------------------------------------------------------
// Receipt mode
// ---------------------------------------------------------------------------

export interface ReceiptParse {
  currency: string;
  items: ParsedItem[];
}

const KNOWN_CURRENCIES = new Set(["THB", "USD", "EUR", "GBP", "JPY", "SGD"]);

/** Validate a receipt-mode model reply; `null` when nothing usable survived. */
export function parseReceiptReply(raw: unknown): ReceiptParse | null {
  if (raw === null || typeof raw !== "object") return null;
  const obj = raw as { currency?: unknown; items?: unknown };
  const currencyRaw =
    typeof obj.currency === "string" ? obj.currency.toUpperCase() : "THB";
  const currency = KNOWN_CURRENCIES.has(currencyRaw) ? currencyRaw : "THB";
  const items = normalizeParsedItems(obj.items);
  if (items.length === 0) return null;
  return { currency, items };
}

/** Receipt-mode system prompt. Data URLs go in as a `image_url` user part. */
export function receiptSystemPrompt(): string {
  return [
    "You extract household purchases from receipt or shopping-list photos.",
    "Return STRICT JSON only, no markdown, in this shape:",
    '{"currency":"THB","items":[{"name":"UHT milk 1L","quantity":2,"note":"blue pack","totalMinor":9400}]}',
    "Rules:",
    "- totalMinor is the LINE TOTAL (unit price × quantity) in the currency's minor unit (satang for THB, cents for USD).",
    "- Exclude non-purchasable lines: totals, VAT, change, store headers, loyalty points.",
    "- Merge duplicate lines of the same product into one item (sum the total, add the quantities).",
    "- Keep item names in the language printed on the receipt.",
    '- At most 20 items. If the image is not a receipt or shopping list, return {"currency":"THB","items":[]}.',
  ].join("\n");
}

// ---------------------------------------------------------------------------
// Chat mode
// ---------------------------------------------------------------------------

export type AgentIntent =
  | { intent: "propose_purchases"; reply: string; items: ParsedItem[] }
  | { intent: "budget_answer"; reply: string }
  | { intent: "clarify"; reply: string };

/** Read-only household snapshot the client attaches so answers stay grounded. */
export interface HouseholdContext {
  vaultBalanceSol: string;
  pendingRequestCount: number;
  memberCount: number;
  recentRequests: string[];
}

const MAX_REPLY_LEN = 480;

export function chatSystemPrompt(): string {
  return [
    "You are the purchasing assistant inside Stocksie, a Solana app where a household shares a permissioned on-chain vault and approves purchase requests together.",
    "You PROPOSE; the human approves on-chain. You can never execute anything and must not claim otherwise.",
    "The user's message includes a READ-ONLY CONTEXT block (vault balance, pending request count, members, recent requests). Treat it as ground truth — never invent balances or counts beyond it.",
    "Return STRICT JSON only, no markdown, exactly one of:",
    '{"intent":"propose_purchases","reply":"<one short sentence>","items":[{"name":"นมสด 1 ลิตร","quantity":2,"note":"","totalMinor":9400}]}',
    '{"intent":"budget_answer","reply":"<answer using ONLY the context numbers>"}',
    '{"intent":"clarify","reply":"<a short question back>"}',
    "Rules:",
    "- propose_purchases: the user asks to buy / add things (e.g. “ซื้อนม 2 ขวด งบ 200”). Reply in the user's language.",
    "- totalMinor is the estimated line total in THB satang unless the user clearly names another currency.",
    "- budget_answer: questions about money, balance, pending work, or household state.",
    "- clarify: the request is too vague to propose or answer.",
    "- Reply under 80 words. Item names in the user's language.",
  ].join("\n");
}

/** Validate a chat-mode model reply into the intent union; `null` if unusable. */
export function parseChatReply(raw: unknown): AgentIntent | null {
  if (raw === null || typeof raw !== "object") return null;
  const obj = raw as { intent?: unknown; reply?: unknown; items?: unknown };
  if (typeof obj.reply !== "string") return null;
  const reply = obj.reply.trim().slice(0, MAX_REPLY_LEN);
  if (reply.length === 0) return null;
  switch (obj.intent) {
    case "propose_purchases": {
      const items = normalizeParsedItems(obj.items);
      if (items.length === 0) return null;
      return { intent: "propose_purchases", reply, items };
    }
    case "budget_answer":
      return { intent: "budget_answer", reply };
    case "clarify":
      return { intent: "clarify", reply };
    default:
      return null;
  }
}

// ---------------------------------------------------------------------------
// API-route request validation
// ---------------------------------------------------------------------------

/** A data-URL-ready image payload as accepted by the API route. */
export interface AgentImage {
  /** Raw base64 (no `data:` prefix). */
  base64: string;
  mimeType: string;
}

const IMAGE_MIME = new Set(["image/png", "image/jpeg", "image/webp"]);
const MAX_IMAGE_BYTES = 8 * 1024 * 1024;
const MAX_TEXT_LEN = 2000;

export interface AgentRequestBody {
  mode: "receipt" | "chat";
  image?: AgentImage;
  text?: string;
  context?: HouseholdContext;
}

/** Validate the route's request body; `null` when malformed. */
export function parseAgentRequest(
  body: unknown
): AgentRequestBody | { error: string } | null {
  if (body === null || typeof body !== "object") return null;
  const obj = body as Record<string, unknown>;
  const mode = obj.mode;
  if (mode !== "receipt" && mode !== "chat") return null;

  if (mode === "receipt") {
    const image = obj.image;
    if (image === null || typeof image !== "object") {
      return { error: "Receipt mode requires an image." };
    }
    const img = image as { base64?: unknown; mimeType?: unknown };
    if (
      typeof img.base64 !== "string" ||
      img.base64.length === 0 ||
      img.base64.length > Math.ceil((MAX_IMAGE_BYTES * 4) / 3)
    ) {
      return { error: "Image is missing or too large (max 8 MB)." };
    }
    if (
      typeof img.mimeType !== "string" ||
      !IMAGE_MIME.has(img.mimeType)
    ) {
      return { error: "Image must be PNG, JPEG, or WebP." };
    }
    return {
      mode,
      image: { base64: img.base64, mimeType: img.mimeType },
    };
  }

  if (typeof obj.text !== "string" || obj.text.trim().length === 0) {
    return { error: "Chat mode requires a message." };
  }
  const context = (obj.context ?? {}) as Partial<HouseholdContext>;
  return {
    mode,
    text: obj.text.trim().slice(0, MAX_TEXT_LEN),
    context: {
      vaultBalanceSol:
        typeof context.vaultBalanceSol === "string"
          ? context.vaultBalanceSol
          : "unknown",
      pendingRequestCount: clampInt(
        context.pendingRequestCount ?? 0,
        0,
        1_000_000,
        0
      ),
      memberCount: clampInt(context.memberCount ?? 0, 0, 99, 0),
      recentRequests: Array.isArray(context.recentRequests)
        ? context.recentRequests
            .filter((r): r is string => typeof r === "string")
            .slice(0, 10)
        : [],
    },
  };
}

// ---------------------------------------------------------------------------
// Draft conversion (AI output → editable purchase-request draft)
// ---------------------------------------------------------------------------

/** One editable row in the agent's review tray before anything goes on-chain. */
export interface PurchaseDraft {
  key: string;
  itemText: string;
  unitCostText: string;
  amountLamports: bigint | null;
  selected: boolean;
}

/**
 * Convert a validated ParsedItem into a reviewable draft. `solMinorPerUnit`
 * is `solThb * 100` (see lib/fx). The AI's price is only ever the *starting
 * point* — every field is editable in the tray and the amount is
 * re-validated against the program's 0.0001–0.5 SOL guards at submit time.
 */
export function draftFromParsedItem(
  item: ParsedItem,
  currency: string,
  solMinorPerUnit: number,
  index: number,
  source: "receipt" | "estimate" = "receipt"
): PurchaseDraft {
  const lineParts = [item.name];
  if (item.quantity > 1) lineParts.push(`× ${item.quantity}`);
  if (item.note) lineParts.push(item.note);
  return {
    key: `${source}-${index}-${item.totalMinor}-${item.name.slice(0, 12)}`,
    itemText: lineParts.join(" "),
    unitCostText: `${formatMinor(item.totalMinor, currency)} (${
      source === "receipt" ? "from receipt" : "AI estimate"
    })`,
    amountLamports: minorToLamports(item.totalMinor, solMinorPerUnit),
    selected: true,
  };
}

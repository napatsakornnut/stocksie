import { describe, expect, it } from "vitest";
import {
  normalizeParsedItems,
  parseReceiptReply,
  parseChatReply,
  parseAgentRequest,
  draftFromParsedItem,
  receiptSystemPrompt,
  chatSystemPrompt,
} from "./ai";
import { DEFAULT_SOL_THB, minorToLamports as minorToLamportsRef } from "./fx";

const solMinorPerUnit = DEFAULT_SOL_THB * 100;

describe("normalizeParsedItems", () => {
  it("keeps valid lines and clamps quantity into 1..99", () => {
    const items = normalizeParsedItems([
      { name: "  UHT milk 1L  ", quantity: 0, totalMinor: 9400 },
      { name: "Eggs", quantity: 150, totalMinor: 12000 },
      { name: "Rice 5kg", note: "jasmine", totalMinor: 320_00 },
    ]);
    expect(items).toEqual([
      { name: "UHT milk 1L", quantity: 1, note: undefined, totalMinor: 9400 },
      { name: "Eggs", quantity: 99, note: undefined, totalMinor: 12000 },
      { name: "Rice 5kg", quantity: 1, note: "jasmine", totalMinor: 32_000 },
    ]);
  });

  it("drops unusable lines instead of inventing repairs", () => {
    const items = normalizeParsedItems([
      { name: "", totalMinor: 100 },
      { name: "no price", totalMinor: 0 },
      { name: "bad price", totalMinor: "many" },
      null,
      "not an object",
      { name: 42, totalMinor: 100 },
      { name: "ok", totalMinor: 100 },
    ]);
    expect(items).toHaveLength(1);
    expect(items[0].name).toBe("ok");
  });

  it("caps at 20 items", () => {
    const raw = Array.from({ length: 30 }, (_, i) => ({
      name: `item ${i}`,
      totalMinor: 100,
    }));
    expect(normalizeParsedItems(raw)).toHaveLength(20);
  });

  it("returns empty for non-array input", () => {
    expect(normalizeParsedItems(null)).toEqual([]);
    expect(normalizeParsedItems("items")).toEqual([]);
  });
});

describe("parseReceiptReply", () => {
  it("normalizes currency and passes items through the validator", () => {
    const parsed = parseReceiptReply({
      currency: "thb",
      items: [{ name: "Milk", quantity: 2, totalMinor: 9400 }],
    });
    expect(parsed).toEqual({
      currency: "THB",
      items: [{ name: "Milk", quantity: 2, note: undefined, totalMinor: 9400 }],
    });
  });

  it("falls back to THB for unknown currencies and null for empty parses", () => {
    expect(
      parseReceiptReply({ currency: "BTC", items: [{ name: "x", totalMinor: 1 }] })
        ?.currency
    ).toBe("THB");
    expect(parseReceiptReply({ currency: "THB", items: [] })).toBeNull();
    expect(parseReceiptReply("hi")).toBeNull();
  });
});

describe("parseChatReply", () => {
  it("accepts all three intents", () => {
    expect(
      parseChatReply({
        intent: "propose_purchases",
        reply: "Here you go",
        items: [{ name: "นม", totalMinor: 5_00 }],
      })
    ).toMatchObject({ intent: "propose_purchases" });
    expect(
      parseChatReply({ intent: "budget_answer", reply: "3.2 SOL left" })
    ).toEqual({ intent: "budget_answer", reply: "3.2 SOL left" });
    expect(parseChatReply({ intent: "clarify", reply: "How many?" })).toEqual({
      intent: "clarify",
      reply: "How many?",
    });
  });

  it("rejects proposals with no usable items and replies with no text", () => {
    expect(
      parseChatReply({ intent: "propose_purchases", reply: "x", items: [] })
    ).toBeNull();
    expect(parseChatReply({ intent: "budget_answer", reply: "  " })).toBeNull();
    expect(parseChatReply({ intent: "unknown_intent", reply: "x" })).toBeNull();
  });
});

describe("parseAgentRequest", () => {
  it("validates receipt images (mime + size)", () => {
    expect(parseAgentRequest({ mode: "receipt" })).toEqual({
      error: "Receipt mode requires an image.",
    });
    expect(
      parseAgentRequest({
        mode: "receipt",
        image: { base64: "aGVsbG8=", mimeType: "image/gif" },
      })
    ).toEqual({ error: "Image must be PNG, JPEG, or WebP." });
    expect(
      parseAgentRequest({
        mode: "receipt",
        image: { base64: "aGVsbG8=", mimeType: "image/png" },
      })
    ).toEqual({ mode: "receipt", image: { base64: "aGVsbG8=", mimeType: "image/png" } });
  });

  it("validates chat text and defaults the context", () => {
    expect(parseAgentRequest({ mode: "chat" })).toEqual({
      error: "Chat mode requires a message.",
    });
    const parsed = parseAgentRequest({ mode: "chat", text: "  ซื้อนม 2 ขวด  " });
    expect(parsed).toMatchObject({ mode: "chat", text: "ซื้อนม 2 ขวด" });
    if (parsed && !("error" in parsed) && parsed.mode === "chat") {
      expect(parsed.context?.vaultBalanceSol).toBe("unknown");
      expect(parsed.context?.recentRequests).toEqual([]);
    }
  });

  it("returns null for garbage bodies", () => {
    expect(parseAgentRequest(null)).toBeNull();
    expect(parseAgentRequest({ mode: "yolo" })).toBeNull();
    expect(parseAgentRequest("chat")).toBeNull();
  });
});

describe("draftFromParsedItem", () => {
  it("builds an editable draft with a converted lamport amount", () => {
    const draft = draftFromParsedItem(
      { name: "UHT milk 1L", quantity: 2, note: "blue", totalMinor: 18_800 },
      "THB",
      solMinorPerUnit,
      0
    );
    expect(draft.itemText).toBe("UHT milk 1L × 2 blue");
    expect(draft.unitCostText).toBe("฿188.00 (from receipt)");
    expect(draft.amountLamports).toBe(
      minorToLamportsRef(18_800, solMinorPerUnit)
    );
    expect(draft.selected).toBe(true);
  });

  it("labels chat proposals as estimates", () => {
    const draft = draftFromParsedItem(
      { name: "ข้าว 5 กก.", quantity: 1, totalMinor: 32_000 },
      "THB",
      solMinorPerUnit,
      1,
      "estimate"
    );
    expect(draft.itemText).toBe("ข้าว 5 กก.");
    expect(draft.unitCostText).toContain("AI estimate");
  });
});

describe("prompts pin the JSON contract", () => {
  it("mentions the exact reply shapes the parsers expect", () => {
    expect(receiptSystemPrompt()).toContain('"totalMinor"');
    expect(receiptSystemPrompt()).toContain("STRICT JSON");
    expect(chatSystemPrompt()).toContain('"propose_purchases"');
    expect(chatSystemPrompt()).toContain('"budget_answer"');
    expect(chatSystemPrompt()).toContain('"clarify"');
    expect(chatSystemPrompt()).toContain("PROPOSE; the human approves");
  });
});

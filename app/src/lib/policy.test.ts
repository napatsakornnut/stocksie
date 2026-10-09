import { describe, expect, it } from "vitest";
import {
  parsePolicyRequest,
  policyStateText,
  policyQuestion,
  buildDecideBody,
  parseDecideResponse,
  type PolicyDraft,
} from "./policy";

const draft: PolicyDraft = {
  key: "d1",
  itemText: "นมสด 1 ลิตร × 2",
  amountSol: "0.02",
  unitCostText: "฿94.00 (from receipt)",
};
const context = { vaultBalanceSol: "3.2", pendingRequestCount: 1 };

describe("parsePolicyRequest", () => {
  it("keeps usable drafts and defaults the context", () => {
    const parsed = parsePolicyRequest({
      drafts: [draft, { key: "", itemText: "  ", amountSol: "x" }],
      context: {},
    });
    expect(parsed && !("error" in parsed) ? parsed.drafts : null).toEqual([
      { ...draft, unitCostText: "฿94.00 (from receipt)" },
    ]);
    if (parsed && !("error" in parsed)) {
      expect(parsed.context.vaultBalanceSol).toBe("unknown");
      expect(parsed.context.pendingRequestCount).toBe(0);
    }
  });

  it("rejects empty or malformed bodies", () => {
    expect(parsePolicyRequest(null)).toBeNull();
    expect(parsePolicyRequest({ drafts: [] })).toEqual({
      error: "drafts must be a non-empty array.",
    });
    expect(parsePolicyRequest({ drafts: [{ key: "k", itemText: " " }] })).toEqual({
      error: "no usable drafts (each needs an item text).",
    });
  });

  it("caps drafts at 20", () => {
    const many = Array.from({ length: 30 }, (_, i) => ({
      key: `k${i}`,
      itemText: `item ${i}`,
    }));
    const parsed = parsePolicyRequest({ drafts: many });
    if (parsed && !("error" in parsed)) {
      expect(parsed.drafts).toHaveLength(20);
    } else {
      throw new Error("expected a parsed body");
    }
  });
});

describe("buildDecideBody", () => {
  it("front-loads the facts into the state", () => {
    const body = buildDecideBody(draft, context);
    expect(body.questions).toEqual([policyQuestion()]);
    expect(body.questions[0].kind).toBe("noul");
    expect(body.state).toContain("Item: นมสด 1 ลิตร × 2");
    expect(body.state).toContain("0.02 SOL");
    expect(body.state).toContain("3.2 SOL");
    expect(body.state).toContain("Pending purchase requests: 1");
    expect(body.state).toContain("standing purchase rules");
  });
});

describe("parseDecideResponse", () => {
  const base = { answers: [{ outcome: null, probabilities: [0.51], confidence: 0.002 }] };

  it("maps abstain to review (the safe default)", () => {
    const v = parseDecideResponse(
      { ...base, routing: { reason: "fused abstain armed" } },
      0.5
    );
    expect(v).toEqual({
      verdict: "review",
      reason: "abstain",
      confidence: 0.002,
      pYes: 0.51,
      routing: "fused abstain armed",
    });
  });

  it("maps a confident yes to covered", () => {
    const v = parseDecideResponse(
      {
        answers: [
          { outcome: { noul: { yes: true } }, probabilities: [0.93], confidence: 0.93 },
        ],
        routing: { reason: "modelless corpus routing: staples=3" },
      },
      0.5
    );
    expect(v).toMatchObject({ verdict: "covered", confidence: 0.93, pYes: 0.93 });
  });

  it("gates a yes below the floor to review", () => {
    const v = parseDecideResponse(
      {
        answers: [
          { outcome: { noul: { yes: true } }, probabilities: [0.55], confidence: 0.41 },
        ],
      },
      0.5
    );
    expect(v).toMatchObject({ verdict: "review", reason: "low-confidence" });
  });

  it("maps an explicit no to review", () => {
    const v = parseDecideResponse(
      {
        answers: [
          { outcome: { noul: { yes: false } }, probabilities: [0.2], confidence: 0.8 },
        ],
      },
      0.5
    );
    expect(v).toMatchObject({ verdict: "review", reason: "no" });
  });

  it("survives malformed shapes with review, or null when nothing is honest", () => {
    expect(
      parseDecideResponse({ answers: [{ outcome: { weird: 1 } }] }, 0.5)
    ).toMatchObject({ verdict: "review", reason: "unparsable" });
    expect(parseDecideResponse({ answers: [] }, 0.5)).toBeNull();
    expect(parseDecideResponse("nope", 0.5)).toBeNull();
  });
});

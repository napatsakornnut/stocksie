import { describe, expect, it } from "vitest";
import {
  minorToLamports,
  lamportsToMinor,
  formatMinor,
  DEFAULT_SOL_THB,
} from "./fx";

describe("minorToLamports", () => {
  it("converts satang to lamports at the given THB rate", () => {
    // ฿94.00 at ฿5,600/SOL → 0.016785714… SOL → 16,785,714 lamports (rounded).
    expect(minorToLamports(9400, 5600 * 100)).toBe(16_785_714n);
  });

  it("round-trips back to minor units", () => {
    const lamports = minorToLamports(12_500, 5600 * 100)!;
    expect(lamportsToMinor(lamports, 5600 * 100)).toBe(12_500);
  });

  it("rejects non-positive or non-finite inputs", () => {
    expect(minorToLamports(0, 5600 * 100)).toBeNull();
    expect(minorToLamports(-100, 5600 * 100)).toBeNull();
    expect(minorToLamports(Number.NaN, 5600 * 100)).toBeNull();
    expect(minorToLamports(9400, 0)).toBeNull();
    expect(minorToLamports(9400, Number.POSITIVE_INFINITY)).toBeNull();
  });
});

describe("formatMinor", () => {
  it("renders THB with the baht sign and two decimals", () => {
    expect(formatMinor(94_00, "THB")).toBe("฿94.00");
  });

  it("prefixes other currencies with their code", () => {
    expect(formatMinor(12_50, "USD")).toBe("USD 12.50");
  });
});

describe("DEFAULT_SOL_THB", () => {
  it("is a positive sanity rate for offline demos", () => {
    expect(DEFAULT_SOL_THB).toBeGreaterThan(0);
    expect(Number.isFinite(DEFAULT_SOL_THB)).toBe(true);
  });
});

import { describe, expect, it } from "vitest";
import { dirOf, fmtPct, fmtPoint, fmtPrice, round2 } from "../src/lib/format";

describe("fmtPrice", () => {
  it("uses 2 decimals and thousands separators", () => {
    expect(fmtPrice(7665.5)).toBe("7,665.50");
    expect(fmtPrice(66.96)).toBe("66.96");
    expect(fmtPrice(2849.89990234375)).toBe("2,849.90");
    expect(fmtPrice(52440)).toBe("52,440.00");
  });
});

describe("fmtPct", () => {
  it("signs with + and U+2212", () => {
    expect(fmtPct(1.1445)).toBe("+1.14%");
    expect(fmtPct(-2.6799)).toBe("−2.68%");
  });
  it("shows no sign when it rounds to 0.00", () => {
    expect(fmtPct(0)).toBe("0.00%");
    expect(fmtPct(0.004)).toBe("0.00%");
    expect(fmtPct(-0.004)).toBe("0.00%");
  });
});

describe("fmtPoint", () => {
  it("signs with + and U+2212", () => {
    expect(fmtPoint(24.75)).toBe("+24.75");
    expect(fmtPoint(-88.75)).toBe("−88.75");
    expect(fmtPoint(1234.5)).toBe("+1,234.50");
  });
  it("shows no sign for zero", () => {
    expect(fmtPoint(0)).toBe("0.00");
    expect(fmtPoint(-0.004)).toBe("0.00");
  });
});

describe("dirOf", () => {
  it("follows change_pct, flat when it rounds to 0.00", () => {
    expect(dirOf(0.5)).toBe("up");
    expect(dirOf(-0.5)).toBe("down");
    expect(dirOf(0.004)).toBe("flat");
    expect(dirOf(-0.004)).toBe("flat");
    expect(dirOf(0)).toBe("flat");
    expect(dirOf(0.005)).toBe("up");
  });
});

describe("round2", () => {
  it("rounds half away from zero, symmetrically", () => {
    expect(round2(0.125)).toBe(0.13);
    expect(round2(-0.125)).toBe(-0.13);
    expect(round2(0.005)).toBe(0.01);
    expect(round2(-0.005)).toBe(-0.01);
  });
  it("never returns negative zero", () => {
    expect(Object.is(round2(-0.001), 0)).toBe(true);
    expect(Object.is(round2(-0), 0)).toBe(true);
  });
});

describe("symmetry and edge cases", () => {
  it("formats mirrored values mirrored", () => {
    expect(fmtPct(0.125)).toBe("+0.13%");
    expect(fmtPct(-0.125)).toBe("\u22120.13%");
    expect(fmtPct(-0.005)).toBe("\u22120.01%");
    expect(dirOf(-0.005)).toBe("down");
    expect(dirOf(0.005)).toBe("up");
  });
  it("shows no sign or negative zero for tiny negatives", () => {
    expect(fmtPct(-0.001)).toBe("0.00%");
    expect(fmtPoint(-0.001)).toBe("0.00");
    expect(fmtPrice(-0.001)).toBe("0.00");
    expect(fmtPrice(-0)).toBe("0.00");
  });
  it("uses U+2212 for negative prices and never an ASCII hyphen", () => {
    expect(fmtPrice(-12.5)).toBe("\u221212.50");
    for (const s of [fmtPct(-3), fmtPoint(-3), fmtPrice(-3), fmtPct(-0.001)]) expect(s).not.toContain("-");
  });
  it("returns an em dash for non-finite input", () => {
    for (const n of [NaN, Infinity, -Infinity]) {
      expect(fmtPct(n)).toBe("\u2014");
      expect(fmtPrice(n)).toBe("\u2014");
      expect(fmtPoint(n)).toBe("\u2014");
      expect(dirOf(n)).toBe("flat");
    }
  });
});

describe("fmtPoint with dir", () => {
  it("shows a flat quote's point change unsigned", () => {
    expect(fmtPoint(0.3, "flat")).toBe("0.30");
    expect(fmtPoint(-0.3, "flat")).toBe("0.30");
  });
  it("puts the sign of dir on the magnitude", () => {
    expect(fmtPoint(24.75, "up")).toBe("+24.75");
    expect(fmtPoint(-88.75, "down")).toBe("\u221288.75");
    expect(fmtPoint(0.3, "down")).toBe("\u22120.30");
  });
});

import { describe, expect, it } from "vitest";
import { dirOf, fmtPct, fmtPoint, fmtPrice } from "../src/lib/format";

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

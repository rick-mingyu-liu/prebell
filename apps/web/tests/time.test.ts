import { describe, expect, it } from "vitest";
import { formatEt } from "../src/lib/time";

describe("formatEt", () => {
  it("formats in US Eastern for both languages", () => {
    expect(formatEt("2026-10-29T12:34:00Z", "en")).toBe("Oct 29, 2026, 8:34 AM ET");
    expect(formatEt("2026-10-29T12:34:00Z", "zh")).toBe("2026年10月29日 上午8:34（美东）");
  });

  it("follows DST (EST after Nov 1)", () => {
    expect(formatEt("2026-11-02T14:05:00Z", "en")).toBe("Nov 2, 2026, 9:05 AM ET");
  });

  it("returns empty for a missing timestamp", () => {
    expect(formatEt(null, "en")).toBe("");
  });
});

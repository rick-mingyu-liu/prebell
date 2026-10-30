import { describe, expect, it } from "vitest";
import { formatDayShort, formatEt, formatEtTime } from "../src/lib/time";

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

describe("formatEtTime", () => {
  it("formats the time of day in US Eastern", () => {
    expect(formatEtTime("2026-10-29T12:34:00Z", "en")).toBe("8:34 AM ET");
    expect(formatEtTime("2026-10-29T12:34:00Z", "zh")).toBe("上午8:34（美东）");
    expect(formatEtTime("2026-11-02T21:05:00Z", "en")).toBe("4:05 PM ET");
  });

  it("returns empty for a missing or bad timestamp", () => {
    expect(formatEtTime(null, "en")).toBe("");
    expect(formatEtTime("nope", "zh")).toBe("");
  });
});

describe("formatDayShort", () => {
  it("is a short date with the weekday", () => {
    expect(formatDayShort("2026-11-02", "en")).toBe("Mon, Nov 2");
    expect(formatDayShort("2026-11-02", "zh")).toBe("11月2日 周一");
  });
  it("returns a bad date unchanged", () => {
    expect(formatDayShort("nope", "en")).toBe("nope");
  });
});

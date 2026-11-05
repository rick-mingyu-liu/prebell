import { describe, expect, it } from "vitest";
import { editionAt, formatDate, formatDayShort, formatEt, formatEtTime } from "../src/lib/time";

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

describe("formatDate full", () => {
  it("adds the weekday and the long month", () => {
    expect(formatDate("2026-11-04", "en", "full")).toBe("Wednesday, November 4, 2026");
    expect(formatDate("2026-11-04", "zh", "full")).toBe("2026年11月4日星期三");
    expect(formatDate("2026-11-04", "en")).toBe("Nov 4, 2026");
  });
});

describe("editionAt", () => {
  it("is the premarket edition before 4:00 PM Eastern and the recap after", () => {
    expect(editionAt(new Date("2026-11-04T12:00:00Z"))).toBe("premarket"); // 7:00 AM ET
    expect(editionAt(new Date("2026-11-04T20:59:00Z"))).toBe("premarket"); // 3:59 PM ET
    expect(editionAt(new Date("2026-11-04T21:00:00Z"))).toBe("recap"); // 4:00 PM ET
    expect(editionAt(new Date("2026-11-05T04:30:00Z"))).toBe("recap"); // 11:30 PM ET
    expect(editionAt(new Date("2026-06-04T19:59:00Z"))).toBe("premarket"); // 3:59 PM EDT
    expect(editionAt(new Date("2026-06-04T20:00:00Z"))).toBe("recap"); // 4:00 PM EDT
  });
});

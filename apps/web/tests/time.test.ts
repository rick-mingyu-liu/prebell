import { describe, expect, it } from "vitest";
import { translator } from "../src/lib/translator";
import {
  datelineAt,
  editionAt,
  formatDate,
  formatDayShort,
  formatEt,
  formatEtDateTime,
  formatEtTime,
  formatMonthDay,
} from "../src/lib/time";

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

describe("formatMonthDay", () => {
  it("is the month and day without the weekday or year", () => {
    expect(formatMonthDay("2026-11-04", "en")).toBe("Nov 4");
    expect(formatMonthDay("2026-11-04", "zh")).toBe("11月4日");
    expect(formatMonthDay("nope", "en")).toBe("nope");
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

describe("datelineAt", () => {
  it("follows the edition on weekdays, in EST and EDT", () => {
    expect(datelineAt(new Date("2026-11-04T12:00:00Z"))).toBe("premarket"); // Wed 7:00 AM EST
    expect(datelineAt(new Date("2026-11-04T21:00:00Z"))).toBe("recap"); // Wed 4:00 PM EST
    expect(datelineAt(new Date("2026-06-05T19:59:00Z"))).toBe("premarket"); // Fri 3:59 PM EDT
    expect(datelineAt(new Date("2026-06-05T20:00:00Z"))).toBe("recap"); // Fri 4:00 PM EDT
  });

  it("is the weekend edition all Saturday and Sunday in Eastern time", () => {
    expect(datelineAt(new Date("2026-11-07T12:00:00Z"))).toBe("weekend"); // Sat 7:00 AM EST
    expect(datelineAt(new Date("2026-11-07T21:30:00Z"))).toBe("weekend"); // Sat 4:30 PM EST
    expect(datelineAt(new Date("2026-11-09T04:30:00Z"))).toBe("weekend"); // Sun 11:30 PM EST
    // Friday evening in New York is already Saturday in UTC: still Friday's recap.
    expect(datelineAt(new Date("2026-11-07T02:00:00Z"))).toBe("recap"); // Fri 9:00 PM EST
    // Monday early morning in UTC is still Sunday in New York.
    expect(datelineAt(new Date("2026-11-09T03:00:00Z"))).toBe("weekend"); // Sun 10:00 PM EST
    expect(datelineAt(new Date("2026-11-09T05:00:00Z"))).toBe("premarket"); // Mon 12:00 AM EST
  });

  it("has a label for every dateline in both locales", () => {
    expect(translator("en")("paper.dateline.weekend")).toBe("Weekend edition");
    expect(translator("zh")("paper.dateline.weekend")).toBe("周末版");
  });
});

describe("formatEtDateTime", () => {
  const now = new Date("2026-11-05T15:00:00Z"); // Thu Nov 5, 10:00 AM EST

  it("shows only the time when the timestamp is today in Eastern time", () => {
    expect(formatEtDateTime("2026-11-05T13:30:00Z", "en", now)).toBe("8:30 AM ET");
    expect(formatEtDateTime("2026-11-05T13:30:00Z", "zh", now)).toBe("上午8:30（美东）");
  });

  it("adds the month and day when the timestamp is an earlier Eastern day", () => {
    expect(formatEtDateTime("2026-11-04T21:46:00Z", "en", now)).toBe("Nov 4, 4:46 PM ET");
    expect(formatEtDateTime("2026-11-04T21:46:00Z", "zh", now)).toBe("11月4日 下午4:46（美东）");
  });

  it("uses the Eastern calendar day, not the UTC one", () => {
    // 11:30 PM EST on Nov 4 is Nov 5 in UTC, but still yesterday in New York.
    expect(formatEtDateTime("2026-11-05T04:30:00Z", "en", now)).toBe("Nov 4, 11:30 PM ET");
  });

  it("returns empty for a missing or bad timestamp", () => {
    expect(formatEtDateTime(null, "en", now)).toBe("");
    expect(formatEtDateTime("nope", "zh", now)).toBe("");
  });
});

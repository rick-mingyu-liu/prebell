import { describe, expect, it } from "vitest";
import { calendarAligned, earningsSearchTitle } from "../src/lib/searchtitle";

describe("earningsSearchTitle", () => {
  it("names a calendar quarter plainly and a fiscal one as fiscal", () => {
    expect(earningsSearchTitle({ slug: "cop-q3-fy2026-earnings", ticker: "COP", tradingDate: "2026-11-06" }, "en")).toBe(
      "ConocoPhillips Q3 2026 earnings results",
    );
    expect(earningsSearchTitle({ slug: "nvda-q3-fy2027-earnings", ticker: "NVDA", tradingDate: "2026-11-17" }, "en")).toBe(
      "Nvidia Q3 fiscal 2027 earnings results",
    );
    // Disney's fiscal Q4 2026 is reported before calendar Q4 2026 ends: never "Q4 2026".
    expect(earningsSearchTitle({ slug: "dis-q4-fy2026-earnings", ticker: "DIS", tradingDate: "2026-11-17" }, "en")).toBe(
      "Walt Disney Q4 fiscal 2026 earnings results",
    );
    // An accession suffix and a dotted ticker.
    expect(
      earningsSearchTitle({ slug: "brk-b-q3-fy2026-earnings-123456", ticker: "BRK.B", tradingDate: "2026-11-03" }, "en"),
    ).toMatch(/ Q3 2026 earnings results$/);
  });

  it("falls back to the date without a period, the ticker without a known name, and covers full years and Chinese", () => {
    expect(earningsSearchTitle({ slug: "qcom-2026-11-04-earnings", ticker: "QCOM", tradingDate: "2026-11-04" }, "en")).toBe(
      "Qualcomm earnings results, Nov 4, 2026",
    );
    expect(earningsSearchTitle({ slug: "zzzz-q1-fy2027-earnings", ticker: "ZZZZ", tradingDate: "2026-11-04" }, "en")).toBe(
      "ZZZZ Q1 fiscal 2027 earnings results",
    );
    expect(earningsSearchTitle({ slug: "cop-fy2026-earnings", ticker: "COP", tradingDate: "2027-02-05" }, "en")).toBe(
      "ConocoPhillips fiscal 2026 earnings results",
    );
    expect(earningsSearchTitle({ slug: "cop-q3-fy2026-earnings", ticker: "COP", tradingDate: "2026-11-06" }, "zh")).toBe(
      "ConocoPhillips 2026年第三季度财报",
    );
    expect(earningsSearchTitle({ slug: "nvda-q3-fy2027-earnings", ticker: "NVDA", tradingDate: "2026-11-17" }, "zh")).toBe(
      "Nvidia 2027财年第三季度财报",
    );
    expect(earningsSearchTitle({ slug: "qcom-2026-11-04-earnings", ticker: "QCOM", tradingDate: "2026-11-04" }, "zh")).toBe(
      "Qualcomm 财报，2026年11月4日",
    );
  });

  it("calendarAligned: the quarter ended, and within 120 days", () => {
    expect(calendarAligned(4, 2025, "2026-02-05")).toBe(true);
    expect(calendarAligned(3, 2026, "2026-09-30")).toBe(true);
    expect(calendarAligned(4, 2026, "2026-11-17")).toBe(false);
    expect(calendarAligned(3, 2026, "2027-03-01")).toBe(false);
  });
});

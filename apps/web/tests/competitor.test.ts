import { describe, expect, it } from "vitest";
import { getLatestReport, listRecentItems, tokenMatches } from "../src/lib/competitor";
import { memoryD1, migrations } from "./d1";

const report = (date: string, ticker: string) =>
  JSON.stringify({
    report_date: date,
    window_start: `${date}T11:30:00Z`,
    window_end: `${date}T11:30:00Z`,
    their_item_count: 2,
    our_post_count: 1,
    our_slugs: ["premarket-2026-10-29"],
    uncovered: [
      {
        ticker,
        items: [{ title: "Fixture", link: "https://c.test/1", category: "Earnings", published_at: `${date}T01:00:00Z` }],
      },
    ],
    covered: ["NVDA"],
    topics: [],
  });

const seed = `
INSERT INTO competitor_reports VALUES ('2026-10-29','a','b','${report("2026-10-29", "OLD")}','old digest','c');
INSERT INTO competitor_reports VALUES ('2026-10-30','a','b','${report("2026-10-30", "MSFT")}','new digest','c');
INSERT INTO competitor_items VALUES ('https://c.test/1','First','2026-10-29T01:00:00Z','Earnings','MSFT NVDA','s','s');
INSERT INTO competitor_items VALUES ('https://c.test/2','Second','2026-10-29T05:00:00Z',NULL,'','s','s');
INSERT INTO competitor_items VALUES ('https://c.test/3','Undated',NULL,'Tech','TSM','s','s');
`;

describe("tokenMatches", () => {
  it("accepts only the exact configured token", () => {
    expect(tokenMatches("abc", "abc")).toBe(true);
    expect(tokenMatches("abd", "abc")).toBe(false);
    expect(tokenMatches("ab", "abc")).toBe(false);
    expect(tokenMatches("abc", "")).toBe(false);
    expect(tokenMatches("", "")).toBe(false);
    expect(tokenMatches("abc", undefined)).toBe(false);
    expect(tokenMatches(["abc"], "abc")).toBe(false);
    expect(tokenMatches(undefined, "abc")).toBe(false);
  });
});

describe("competitor queries", () => {
  it("applies every migration", () => {
    expect(migrations).toContain("0004_competitor.sql");
    expect(migrations[0]).toBe("0001_init.sql");
  });

  it("returns the newest report with parsed gaps", async () => {
    const r = await getLatestReport(memoryD1(seed));
    expect(r).not.toBeNull();
    expect(r!.reportDate).toBe("2026-10-30");
    expect(r!.digest).toBe("new digest");
    expect(r!.report.uncovered[0].ticker).toBe("MSFT");
    expect(r!.report.covered).toEqual(["NVDA"]);
  });

  it("returns null without reports", async () => {
    expect(await getLatestReport(memoryD1(""))).toBeNull();
  });

  it("lists items newest first with split tickers, undated last", async () => {
    const items = await listRecentItems(memoryD1(seed), 10);
    expect(items.map((i) => i.title)).toEqual(["Second", "First", "Undated"]);
    expect(items[1].tickers).toEqual(["MSFT", "NVDA"]);
    expect(items[0].tickers).toEqual([]);
    expect(items[0].category).toBe("");
    expect(items[2].publishedAt).toBeNull();
  });

  it("honours the limit", async () => {
    expect(await listRecentItems(memoryD1(seed), 1)).toHaveLength(1);
  });
});

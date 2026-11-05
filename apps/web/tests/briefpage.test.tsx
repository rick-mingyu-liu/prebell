import { readFileSync } from "node:fs";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import BriefView, { type BriefViewProps } from "../src/components/BriefView";
import { loadBrief } from "../src/lib/brief";
import type { StripDay } from "../src/lib/earnings";
import { parseFacts, type R2Like } from "../src/lib/facts";
import type { PostListItem, PostView } from "../src/lib/posts";
import { memoryD1 } from "./d1";

const fixtureText = (name: string) => readFileSync(new URL(`./fixtures/${name}`, import.meta.url), "utf8");
const premarketFacts = () => parseFacts("premarket", JSON.parse(fixtureText("premarket-2026-10-30.json")))!;
const bodyEn = fixtureText("premarket-2026-10-30.en.md");
const sourcesEn = [...bodyEn.matchAll(/\]\((https:\/\/[^)]+)\)/g)].map((m) => ({ url: m[1], title: m[1] }));

const post = (over: Partial<PostView> = {}): PostView => ({
  slug: "premarket-2026-10-30",
  contentType: "premarket",
  tradingDate: "2026-10-30",
  publishedAt: "2026-10-30T13:20:43Z",
  title: "Tech earnings drag futures lower; Intel plunges premarket",
  summary: bodyEn.split("\n\n")[1],
  bodyMd: bodyEn,
  status: "published",
  sources: sourcesEn,
  hasOtherLang: true,
  factsKey: "premarket/2026-10-30.json",
  ...over,
});

const item = (slug: string, contentType: string, tradingDate: string): PostListItem => ({
  slug,
  contentType,
  tradingDate,
  publishedAt: `${tradingDate}T12:00:00Z`,
  title: slug,
  summary: "",
});

const strip: StripDay = {
  date: "2026-10-30",
  bmo: [{ ticker: "XOM", date: "2026-10-30", slot: "bmo", status: "confirmed", postSlug: null }],
  amc: [],
  unknown: [],
};

const props = (over: Partial<BriefViewProps> = {}): BriefViewProps => ({
  locale: "en",
  today: "2026-10-30",
  post: post(),
  facts: premarketFacts(),
  adjacent: { prev: item("recap-2026-10-29", "recap", "2026-10-29"), next: item("recap-2026-10-30", "recap", "2026-10-30") },
  strip,
  charts: [],
  ...over,
});

const render = (over: Partial<BriefViewProps> = {}) => renderToStaticMarkup(<BriefView {...props(over)} />);

/** Position of `needle` in `out`, failing the test when it is absent. */
const at = (out: string, needle: string | RegExp) => {
  const i = typeof needle === "string" ? out.indexOf(needle) : out.search(needle);
  expect(i, String(needle)).toBeGreaterThanOrEqual(0);
  return i;
};

describe("BriefView", () => {
  it("starts with the earnings strip, then the article grid with the sidebar on the right", () => {
    const out = render();
    expect(out.startsWith('<section class="estrip">')).toBe(true);
    expect(out).toContain('<div class="grid-article');
    expect(at(out, '<aside class="sidebar">')).toBeGreaterThan(at(out, "</article>"));
  });

  it("renders breadcrumb, ET publish time and the badge, then numbers, movers and the story in order", () => {
    const out = render();
    const crumb = at(out, 'href="/en/archive"');
    const crumbType = at(out, 'href="/en/archive?type=premarket"');
    const time = at(out, "Published Oct 30, 2026, 9:20 AM ET");
    const badge = at(out, "✓ Numbers checked against source data");
    const numbers = at(out, 'class="numbers numbers-page"');
    const movers = at(out, '<ul class="movers">');
    const title = at(out, "<h1");
    const futures = at(out, "<h2>Futures</h2>");
    const nav = at(out, 'rel="prev"');
    expect(crumb).toBeLessThan(crumbType);
    expect([crumbType, time, badge, numbers, movers, title, futures, nav]).toEqual(
      [crumbType, time, badge, numbers, movers, title, futures, nav].sort((a, b) => a - b),
    );
  });

  it("lists every mover with its price", () => {
    const out = render();
    for (const s of ["MDLZ", "REGN", "CL", "AZN", "EFX", "INTC", "HPQ", "DELL", "SYK", "ARM"])
      expect(out).toContain(`<b>${s}</b>`);
    expect(out).toContain("$68.13");
  });

  it("shows the title and summary once, the trimmed story, colored signed numbers, and the sources", () => {
    const out = render();
    expect(out.match(/<h1/g)).toHaveLength(1);
    expect(out).toContain("<h1 class=\"brief-title\">Tech earnings drag futures lower; Intel plunges premarket</h1>");
    expect(out.match(/US futures fell for a second day as weak tech earnings/g)).toHaveLength(1);
    expect(out).not.toContain("<table");
    expect(out).toContain("S&amp;P 500 futures were down 1.14%");
    expect(out).toContain('<a href="https://www.wsj.com/business/earnings/intel-shares-tumble-on-lower-profit-weak-guidance-a2bdcdd8"');
    expect(out).toContain("It is not investment advice.");
  });

  it("has no in-page language pill (the header has one)", () => {
    expect(render()).not.toContain("lang-pill");
  });

  it("links the previous and next briefs", () => {
    const out = render();
    expect(out).toMatch(/<a rel="prev" href="\/en\/p\/recap-2026-10-29"/);
    expect(out).toMatch(/<a class="next" rel="next" href="\/en\/p\/recap-2026-10-30"/);
    expect(render({ adjacent: { prev: null, next: null } })).not.toContain('class="adjacent"');
  });

  it("renders the full body as before when the facts are missing", () => {
    const out = render({ facts: null });
    expect(out).not.toContain("numbers-page");
    expect(out).not.toContain('class="movers"');
    expect(out).toContain("<table>");
    expect(out).toContain("<h1>Tech earnings drag futures lower; Intel plunges premarket</h1>");
    expect(out.match(/<h1/g)).toHaveLength(1);
    expect(out).toContain("✓ Numbers checked against source data");
  });

  it("keeps numbers and movers on a pending ZH page and puts the notice in place of the story", () => {
    const out = render({
      locale: "zh",
      post: post({ status: "pending", title: "", summary: "", bodyMd: "", sources: [] }),
    });
    expect(out).toContain("numbers-page");
    expect(out).toContain('<ul class="movers">');
    expect(out).toContain('role="status"');
    expect(out).toContain('href="/en/p/premarket-2026-10-30"');
    expect(out).toContain("<h1>盘前简报 2026年10月30日</h1>");
    expect(out).toContain("✓ 数字已与源数据核对");
    expect(out).not.toContain("post-body");
    expect(at(out, '<ul class="movers">')).toBeLessThan(at(out, 'role="status"'));
    expect(out).not.toContain("发布于");
  });

  it("keeps an earnings check's body and charts, without numbers, movers or the badge", () => {
    const out = render({
      post: post({ slug: "earnings-aapl-2026-10-30", contentType: "earnings", bodyMd: "# Apple Q4\n\n| A |\n|---|\n| 1 |\n" }),
      facts: null,
      adjacent: { prev: null, next: null },
      charts: [{ name: "revenue", src: "/charts/aapl/revenue.svg", alt: "Apple revenue" }],
    });
    expect(out.startsWith('<section class="estrip">')).toBe(true);
    expect(out).toContain("<h1>Apple Q4</h1>");
    expect(out).toContain("<table>");
    expect(out).toContain('<img src="/charts/aapl/revenue.svg" alt="Apple revenue"');
    expect(out).toContain('href="/en/archive?type=earnings"');
    expect(out).not.toContain("Numbers checked");
    expect(out).not.toContain("numbers-page");
    expect(at(out, "<table>")).toBeLessThan(at(out, 'class="charts"'));
  });

  it("renders no empty meta line on a pending earnings check", () => {
    const out = render({
      locale: "zh",
      post: post({ slug: "earnings-aapl-2026-10-30", contentType: "earnings", status: "pending", bodyMd: "", sources: [] }),
      facts: null,
      adjacent: { prev: null, next: null },
    });
    expect(out).toContain('role="status"');
    expect(out).not.toContain("brief-meta");
  });
});

/** Published EN+ZH briefs and an earnings check; `zh` sets the ZH status of the 10-30 brief. */
function seed(zh = "published"): string {
  const row = (id: number, type: string, date: string, ts: string, key: string | null, zhStatus = "published") =>
    `INSERT INTO runs VALUES ('${type}:${date}','${type}','${date}','published',1,NULL,'${ts}','${ts}');
INSERT INTO posts (id, run_key, content_type, template_id, trading_date, slug, facts_r2_key, published_at)
VALUES (${id},'${type}:${date}','${type}','${type}:v1','${date}','${type}-${date}',${key ? `'${key}'` : "NULL"},'${ts}');
INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at) VALUES (${id},'en','published','EN ${id}','Sum','# EN ${id}','${ts}');
INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at) VALUES (${id},'zh','${zhStatus}',${zhStatus === "published" ? `'ZH ${id}','摘要','# ZH ${id}'` : "NULL,NULL,NULL"},'${ts}');\n`;
  return (
    row(1, "recap", "2026-10-29", "2026-10-29T20:45:00Z", "recap/2026-10-29.json") +
    row(2, "premarket", "2026-10-30", "2026-10-30T13:20:43Z", "premarket/2026-10-30.json", zh) +
    row(3, "earnings", "2026-10-30", "2026-10-30T21:00:00Z", null) +
    `INSERT INTO post_charts (post_id, name, r2_key, alt_en, alt_zh, sort) VALUES (3,'revenue','charts/earnings/AAPL/0000320193-26-000123/revenue.svg','Revenue','营收',0);
INSERT INTO earnings_calendar (ticker, report_date, time_slot, status, source, first_seen_at, checked_at, updated_at)
VALUES ('XOM','2026-10-30','bmo','confirmed','ir','2026-10-29T00:00:00Z','2026-10-29T00:00:00Z','2026-10-29T00:00:00Z');`
  );
}

function spies(sql: string) {
  const db = memoryD1(sql);
  const queries: string[] = [];
  const gets: string[] = [];
  const bucket: R2Like = {
    async get(key) {
      gets.push(key);
      return key === "premarket/2026-10-30.json" ? { text: async () => fixtureText("premarket-2026-10-30.json") } : null;
    },
  };
  return { db: { prepare: (q: string) => (queries.push(q), db.prepare(q)) }, bucket, queries, gets };
}

describe("loadBrief", () => {
  it("reads the facts once, the neighbours in one query and the strip for the brief's own date", async () => {
    const s = spies(seed());
    const data = await loadBrief(s.db, s.bucket, post(), "en");
    expect(s.gets).toEqual(["premarket/2026-10-30.json"]);
    expect(data.facts?.lead.symbol).toBe("ES=F");
    expect(data.adjacent.prev?.slug).toBe("recap-2026-10-29");
    expect(data.adjacent.next).toBeNull();
    expect(data.strip?.date).toBe("2026-10-30");
    expect(data.strip?.bmo.map((e) => e.ticker)).toEqual(["XOM"]);
    expect(data.charts).toEqual([]);
    // adjacent (1) + strip (2); no chart query for a brief.
    expect(s.queries).toHaveLength(3);
    expect(s.queries.some((q) => q.includes("post_charts"))).toBe(false);
  });

  it("uses the strip's exact date: no entries that day means no strip", async () => {
    const s = spies(seed());
    const data = await loadBrief(s.db, s.bucket, post({ tradingDate: "2026-10-29" }), "en");
    expect(data.strip).toBeNull();
  });

  it("loads an earnings check's charts and no facts or neighbours", async () => {
    const s = spies(seed());
    const data = await loadBrief(
      s.db,
      s.bucket,
      post({ slug: "earnings-2026-10-30", contentType: "earnings", factsKey: null }),
      "zh",
    );
    expect(s.gets).toEqual([]);
    expect(data.facts).toBeNull();
    expect(data.adjacent).toEqual({ prev: null, next: null });
    expect(data.charts).toEqual([{ name: "revenue", src: "/charts/earnings/AAPL/0000320193-26-000123/revenue.svg", alt: "营收" }]);
  });

  it("falls back to no facts when R2 fails, and reports it", async () => {
    const s = spies(seed());
    const errors: unknown[] = [];
    const broken: R2Like = {
      async get() {
        throw new Error("r2 down");
      },
    };
    const data = await loadBrief(s.db, broken, post(), "en", (e) => errors.push(e));
    expect(data.facts).toBeNull();
    expect(errors).toHaveLength(1);
  });
});

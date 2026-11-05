import { readFileSync } from "node:fs";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import BriefView from "../src/components/BriefView";
import { loadBrief, type BriefData } from "../src/lib/brief";
import { parseFacts, type R2Like } from "../src/lib/facts";
import type { PostListItem, PostView } from "../src/lib/posts";
import { memoryD1 } from "./d1";

const fixtureText = (name: string) => readFileSync(new URL(`./fixtures/${name}`, import.meta.url), "utf8");
const premarketFacts = () => parseFacts("premarket", JSON.parse(fixtureText("premarket-2026-10-30.json")))!;
const recapFacts = () => parseFacts("recap", JSON.parse(fixtureText("recap-sample.json")))!;
const bodyEn = fixtureText("premarket-2026-10-30.en.md");
const bodyZh = fixtureText("premarket-2026-10-30.zh.md");
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
  coverKey: null,
  coverLines: null,
  ticker: null,
  ...over,
});

const item = (slug: string, contentType: string, tradingDate: string): PostListItem => ({
  slug,
  contentType,
  tradingDate,
  publishedAt: `${tradingDate}T12:00:00Z`,
  title: slug,
  summary: "",
  coverKey: null,
  coverLines: null,
  ticker: null,
});

const data = (over: Partial<BriefData> = {}): BriefData => ({
  post: post(),
  facts: premarketFacts(),
  adjacent: { prev: item("recap-2026-10-29", "recap", "2026-10-29"), next: item("recap-2026-10-30", "recap", "2026-10-30") },
  charts: [],
  ...over,
});

const render = (over: Partial<BriefData> = {}, locale: "en" | "zh" = "en", signup = true) =>
  renderToStaticMarkup(<BriefView data={data(over)} locale={locale} signup={signup} />);

/** Position of `needle` in `out`, failing the test when it is absent. */
const at = (out: string, needle: string | RegExp) => {
  const i = typeof needle === "string" ? out.indexOf(needle) : out.search(needle);
  expect(i, String(needle)).toBeGreaterThanOrEqual(0);
  return i;
};

/** True when the positions are strictly increasing. */
const inOrder = (xs: number[]) => xs.every((x, i) => i === 0 || xs[i - 1] < x);

/** An earnings check's markdown as the generator writes it: title, summary, meta line, key figures, sections, sources, disclaimer. */
const earningsMd = `# Amgen's third quarter 2026: revenue up 8% to $10.4 billion

Amgen reported third quarter 2026 revenue of $10.4 billion and GAAP diluted EPS of $7.08.

**Amgen (AMGN)** · Q3 FY2026 · Form 8-K filed Nov 3, 2026

| Key figure | This quarter |
|---|---:|
| Revenue | $10.4 billion |
| Diluted EPS (GAAP) | $7.08 |

## Results

Total revenues rose 8% to $10.4 billion, up +8% on the year.

## Sources

- [Form 8-K filing (SEC EDGAR)](https://www.sec.gov/Archives/edgar/data/318154/000031815426000153/0000318154-26-000153-index.htm)
- [Press release](https://www.sec.gov/Archives/edgar/data/318154/000031815426000153/ex991.htm)

This earnings summary is generated automatically from SEC filings for information only. It reports results and company guidance, not analyst estimates, and it is not investment advice.
`;
const earningsSources = [...earningsMd.matchAll(/\]\((https:\/\/[^)]+)\)/g)].map((m) => ({ url: m[1], title: m[1] }));

const earnings = (over: Partial<PostView> = {}): Partial<BriefData> => ({
  post: post({
    slug: "amgn-q3-fy2026-earnings",
    contentType: "earnings",
    tradingDate: "2026-11-03",
    publishedAt: "2026-11-04T23:41:00Z",
    title: "Amgen's third quarter 2026: revenue up 8% to $10.4 billion",
    summary: "Amgen reported third quarter 2026 revenue of $10.4 billion and GAAP diluted EPS of $7.08.",
    bodyMd: earningsMd,
    sources: earningsSources,
    factsKey: null,
    ticker: "AMGN",
    coverKey: "covers/amgn-q3-fy2026-earnings.png",
    coverLines: ["Revenue +8% to $10.4B", "GAAP EPS $7.08"],
    ...over,
  }),
  facts: null,
  adjacent: { prev: null, next: null },
  charts: [{ name: "revenue", src: "/charts/amgn/revenue.svg", alt: "Amgen quarterly revenue" }],
});

describe("BriefView", () => {
  it("renders kicker, title, meta with the checked tag, cover, body, movers table, sources, author block, signup and prev/next in order", () => {
    const out = render();
    // React hoists the author photo's preload ahead of the markup; Next moves it into <head>.
    expect(out.replace(/^(<link [^>]*\/>)*/, "").startsWith('<article class="story">')).toBe(true);
    const kicker = at(out, '<p class="kicker story-kicker">Premarket brief · <time dateTime="2026-10-30">Oct 30, 2026</time></p>');
    const title = at(out, '<h1 class="story-title">Tech earnings drag futures lower; Intel plunges premarket</h1>');
    const meta = at(out, '<p class="story-meta">');
    const time = at(out, "Published Oct 30, 2026, 9:20 AM ET");
    const checked = at(out, '<span class="checked">✓ Numbers checked against source data</span>');
    const cover = at(out, '<figure class="cover cover-article cover-flat" aria-label="Tech earnings drag futures lower; Intel plunges premarket">');
    const tag = at(out, '<span class="cover-tag">Oct 30 · Premarket</span>');
    const body = at(out, '<div class="story-body">');
    const lede = at(out, '<p class="story-lede">US futures fell for a second day as weak tech earnings');
    const movers = at(out, '<table class="table movers-table">');
    const futures = at(out, "<h2>Futures</h2>");
    const sources = at(out, '<section class="sources"');
    const disclaimer = at(out, "It is not investment advice.");
    const author = at(out, '<section class="author">');
    const signup = at(out, '<section class="signup">');
    const prev = at(out, 'rel="prev"');
    const partners = at(out, '<aside class="partners-strip"');
    expect(inOrder([kicker, title, meta, time, checked, cover, tag, body, lede, movers, futures, sources, disclaimer, author, signup, prev, partners])).toBe(true);

    // Title, summary, sources heading and disclaimer once each; the body's tables give way to the movers table.
    expect(out.match(/<h1/g)).toHaveLength(1);
    expect(out.match(/US futures fell for a second day as weak tech earnings/g)).toHaveLength(1);
    expect(out.match(/<table/g)).toHaveLength(1);
    expect(out.match(/>Sources</g)).toHaveLength(1);
    expect(out.match(/It is not investment advice\./g)).toHaveLength(1);
    expect(out).toMatch(/<th[^>]*>Company<\/th><th[^>]*>Reason<\/th><th[^>]*>Last<\/th><th[^>]*>Change<\/th>/);
    for (const s of ["MDLZ", "REGN", "CL", "AZN", "EFX", "INTC", "HPQ", "DELL", "SYK", "ARM"]) expect(out).toContain(`>${s}</span>`);
    expect(out).toContain("$68.13");
    expect(out).toContain("S&amp;P 500 futures were down 1.14%");
    expect(out).toContain('<a href="https://www.wsj.com/business/earnings/intel-shares-tumble-on-lower-profit-weak-guidance-a2bdcdd8"');
    expect(at(out, "intel-shares-tumble")).toBeGreaterThan(sources);
    expect(out).toContain('<p class="story-disclaimer">This brief is generated automatically from public data for information only. It is not investment advice.</p>');
    expect(out).toMatch(/<a rel="prev" href="\/en\/p\/recap-2026-10-29"/);
    expect(out).toMatch(/<a class="next" rel="next" href="\/en\/p\/recap-2026-10-30"/);
    expect(out).toContain('id="signup-email-post"');

    // No grid, sidebar, strip, breadcrumb, numbers block, pills or old movers list.
    for (const gone of ["grid-article", "sidebar", "estrip", "crumb", "numbers", 'class="movers"', "pill", "lang-pill", "brief-"])
      expect(out, gone).not.toContain(gone);
  });

  it("labels a recap's kicker, cover and price column for the close", () => {
    const out = render({
      post: post({ slug: "recap-2026-11-04", contentType: "recap", tradingDate: "2026-11-04", factsKey: "recap/2026-11-04.json" }),
      facts: recapFacts(),
    });
    expect(out).toContain('<p class="kicker story-kicker">After-close recap · <time dateTime="2026-11-04">Nov 4, 2026</time></p>');
    expect(out).toContain('<span class="cover-tag">Nov 4 · Recap</span>');
    expect(out).toMatch(/<th[^>]*>Company<\/th><th[^>]*>Reason<\/th><th[^>]*>Close<\/th><th[^>]*>Change<\/th>/);
  });

  it("renders an earnings check with the logo cover, body table untouched, grey charts, no movers", () => {
    const out = render(earnings());
    // React hoists the author photo's preload ahead of the markup; Next moves it into <head>.
    expect(out.replace(/^(<link [^>]*\/>)*/, "").startsWith('<article class="story">')).toBe(true);
    const kicker = at(out, '<p class="kicker story-kicker">Earnings check · AMGN</p>');
    const title = at(out, "<h1 class=\"story-title\">Amgen&#x27;s third quarter 2026: revenue up 8% to $10.4 billion</h1>");
    const checked = at(out, '<span class="checked">✓ Every figure traced to the filing</span>');
    const cover = at(out, '<span class="cover-tag"><img src="/logos/companies/AMGN.png" alt="AMGN"');
    const lede = at(out, '<p class="story-lede">Amgen reported third quarter 2026 revenue');
    const line = at(out, "<p><strong>Amgen (AMGN)</strong> · Q3 FY2026 · Form 8-K filed Nov 3, 2026</p>");
    const table = at(out, '<div class="table-wrap"><table class="table"><thead><tr><th>Key figure</th><th style="text-align:right">This quarter</th></tr></thead>');
    const results = at(out, "<h2>Results</h2>");
    const charts = at(out, '<section class="charts"');
    const sources = at(out, '<section class="sources"');
    const disclaimer = at(out, '<p class="story-disclaimer">This earnings summary is generated automatically');
    const author = at(out, '<section class="author">');
    const signup = at(out, '<section class="signup">');
    const partners = at(out, '<aside class="partners-strip"');
    expect(inOrder([kicker, title, checked, cover, lede, line, table, results, charts, sources, disclaimer, author, signup, partners])).toBe(true);

    // The key-figure table exactly as written: this quarter only, no derived columns.
    expect(out).toContain('<td>Revenue</td><td style="text-align:right">$10.4 billion</td>');
    expect(out).toContain('<td>Diluted EPS (GAAP)</td><td style="text-align:right">$7.08</td>');
    expect(out.match(/<th[ >]/g)).toHaveLength(2);
    expect(out).toContain('<img class="chart" src="/charts/amgn/revenue.svg" alt="Amgen quarterly revenue" width="640" height="280"');
    expect(out).toContain('<figcaption class="muted">Amgen quarterly revenue</figcaption>');
    expect(readFileSync(new URL("../src/app/globals.css", import.meta.url), "utf8")).toMatch(/\.charts img \{[^}]*filter: grayscale\(1\)/);
    expect(out).toContain('href="https://www.sec.gov/Archives/edgar/data/318154/000031815426000153/ex991.htm"');

    expect(out.match(/<h1/g)).toHaveLength(1);
    expect(out.match(/Amgen reported third quarter 2026 revenue/g)).toHaveLength(1);
    for (const gone of ["movers-table", "Numbers checked", "adjacent", "estrip", "sidebar", "grid-article"]) expect(out, gone).not.toContain(gone);
  });

  it("names an earnings check without a ticker by its type, in the kicker and the cover tag", () => {
    const out = render(earnings({ ticker: null }));
    expect(out).toContain('<p class="kicker story-kicker">Earnings check</p>');
    expect(out).toContain('<span class="cover-tag">Earnings check</span>');
  });

  it("ZH brief table has no reason column", () => {
    const zhSum = bodyZh.split("\n\n")[1];
    const out = render(
      {
        post: post({
          title: "科技股财报拖累，纳指期货盘前重挫逾2%",
          summary: zhSum,
          bodyMd: bodyZh,
          sources: [...bodyZh.matchAll(/\]\((https:\/\/[^)]+)\)/g)].map((m) => ({ url: m[1], title: m[1] })),
        }),
      },
      "zh",
    );
    expect(out).toContain('<p class="kicker story-kicker">盘前简报 · <time dateTime="2026-10-30">2026年10月30日</time></p>');
    expect(out).toContain('<span class="cover-tag">10月30日 · 盘前</span>');
    expect(out).toMatch(/<th[^>]*>公司<\/th><th[^>]*>最新价<\/th><th[^>]*>涨跌幅<\/th><\/tr>/);
    expect(out).not.toMatch(/<th[^>]*>原因<\/th>/);
    expect(out).not.toContain("Shares surged");
    expect(out).not.toContain("mover-why");
    expect(out).toContain("✓ 数字已与源数据核对");
    expect(out.match(/<table/g)).toHaveLength(1);
    expect(out).toContain('<section class="sources" aria-labelledby="sources"><h2 id="sources">来源</h2>');
    expect(out).toContain('<p class="story-disclaimer">本简报根据公开数据自动生成，仅供参考，不构成投资建议。</p>');
  });

  it("renders the full body with its tables when the facts are missing, title and summary once", () => {
    const out = render({ facts: null });
    expect(out).not.toContain("movers-table");
    expect(out.match(/<table class="table">/g)).toHaveLength(2);
    expect(out.match(/<h1/g)).toHaveLength(1);
    expect(out.match(/US futures fell for a second day as weak tech earnings/g)).toHaveLength(1);
    expect(out).toContain("✓ Numbers checked against source data");
    expect(at(out, '<p class="story-lede">')).toBeLessThan(at(out, "<table"));
  });

  it("keeps the movers table on a pending ZH page and puts the notice in place of the story", () => {
    const out = render({ post: post({ status: "pending", title: "", summary: "", bodyMd: "", sources: [] }) }, "zh");
    expect(out).toContain('<h1 class="story-title">盘前简报 2026年10月30日</h1>');
    expect(out).toContain("✓ 数字已与源数据核对");
    expect(out).not.toContain("发布于");
    expect(out).toContain('role="status"');
    expect(out).toContain('href="/en/p/premarket-2026-10-30"');
    expect(at(out, "movers-table")).toBeLessThan(at(out, 'role="status"'));
    expect(out).not.toContain("post-body");
    expect(out).not.toContain('class="sources"');
    expect(out).not.toContain("cover-stat");
  });

  it("renders no empty meta line on a pending earnings check", () => {
    const out = render(
      {
        post: post({ slug: "earnings-aapl-2026-10-30", contentType: "earnings", status: "pending", bodyMd: "", sources: [], ticker: "AAPL" }),
        facts: null,
        adjacent: { prev: null, next: null },
        charts: [],
      },
      "zh",
    );
    expect(out).toContain('role="status"');
    expect(out).not.toContain("story-meta");
    expect(out).not.toContain('class="charts"');
  });

  it("leaves the signup out until the Buttondown key is set, keeping the author block", () => {
    const out = render({}, "en", false);
    expect(out).not.toContain('class="signup"');
    expect(out).not.toContain('id="signup-email-post"');
    expect(out).toContain('class="author"');
    expect(out).toContain('class="adjacent"');
  });

  it("drops the adjacent links when there are no neighbours", () => {
    expect(render({ adjacent: { prev: null, next: null } })).not.toContain('class="adjacent"');
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
  it("reads the facts once and the neighbours in one query", async () => {
    const s = spies(seed());
    const data = await loadBrief(s.db, s.bucket, post(), "en");
    expect(s.gets).toEqual(["premarket/2026-10-30.json"]);
    expect(data.facts?.lead.symbol).toBe("ES=F");
    expect(data.adjacent.prev?.slug).toBe("recap-2026-10-29");
    expect(data.adjacent.next).toBeNull();
    expect(data.charts).toEqual([]);
    expect(s.queries).toHaveLength(1);
    expect(s.queries.some((q) => q.includes("post_charts"))).toBe(false);
  });

  it("loadBrief no longer loads the strip", async () => {
    const s = spies(seed());
    const data = await loadBrief(s.db, s.bucket, post(), "en");
    expect(Object.keys(data).sort()).toEqual(["adjacent", "charts", "facts", "post"]);
    expect(s.queries.some((q) => q.includes("earnings_calendar"))).toBe(false);
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

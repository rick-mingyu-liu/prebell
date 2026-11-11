/** Seed data and fakes shared by the MCP tests. */
import { readFileSync } from "node:fs";
import type { D1Like } from "../src/lib/posts";
import type { McpDeps } from "../src/lib/mcp";
import type { EdgeCache } from "../src/lib/sitemap";
import { memoryD1 } from "./d1";

export const fixture = (name: string): Record<string, unknown> =>
  JSON.parse(readFileSync(new URL(`./fixtures/${name}`, import.meta.url), "utf8"));

/** Fri 2026-10-30, 10:00 AM ET: that day's premarket brief is out. */
export const FRIDAY = new Date("2026-10-30T14:00:00Z");

const q = (s: string | null) => (s === null ? "NULL" : `'${s.replace(/'/g, "''")}'`);

let nextId = 1;

/** One post with its run and translations. `zh` is the ZH status, or null for none. */
export function post(o: {
  type: "premarket" | "recap" | "earnings";
  date: string;
  slug: string;
  factsKey: string | null;
  publishedAt: string;
  revisedAt?: string;
  zh?: "published" | "pending" | "failed" | null;
  runKey?: string;
}): string {
  const id = nextId++;
  const run = o.runKey ?? `${o.type}:${o.slug}`;
  const zh = o.zh === undefined ? "published" : o.zh;
  let sql = `INSERT INTO runs VALUES (${q(run)},'${o.type}','${o.date}','published',1,NULL,'${o.publishedAt}','${o.publishedAt}');
INSERT INTO posts (id, run_key, content_type, template_id, trading_date, slug, facts_r2_key, published_at, revised_at)
VALUES (${id},${q(run)},'${o.type}','${o.type}:v1','${o.date}',${q(o.slug)},${q(o.factsKey)},'${o.publishedAt}',${q(o.revisedAt ?? null)});
INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at)
VALUES (${id},'en','published',${q(`EN title ${o.slug}`)},${q(`EN summary ${o.slug}`)},${q(`# EN body ${o.slug}`)},'${o.publishedAt}');\n`;
  if (zh)
    sql += `INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at)
VALUES (${id},'zh','${zh}',${zh === "published" ? q(`中文标题 ${o.slug}`) : "NULL"},${zh === "published" ? q(`中文摘要 ${o.slug}`) : "NULL"},${zh === "published" ? q(`# 中文正文 ${o.slug}`) : "NULL"},'${o.publishedAt}');\n`;
  return sql;
}

/** An earnings check: filing row plus post, facts at `earnings/{TICKER}/{acc}.json`. */
export function check(ticker: string, cik: number, acc: string, date: string, slug: string, zh: "published" | "pending" | null = "published"): string {
  const run = `earnings:${ticker}:${acc}`;
  const ts = `${date}T21:00:00Z`;
  return (
    post({ type: "earnings", date, slug, factsKey: `earnings/${ticker.replace(".", "-")}/${acc}.json`, publishedAt: ts, zh, runKey: run }) +
    `INSERT INTO earnings_filings VALUES ('${acc}',${cik},'${ticker}','8-K','2.02','${date}','${date}','${ts}','x.htm','${run}','done','${ts}');\n`
  );
}

export function cal(ticker: string, date: string, status: string, slot = "amc", proof: string | null = null): string {
  return `INSERT INTO earnings_calendar VALUES ('${ticker}','${date}','${slot}','${status}','yahoo',${q(proof)},NULL,NULL,'t','t',NULL,'t');\n`;
}

export function macro(id: string, series: string, date: string, o: { ref?: string; startsAt?: string; tier?: number; status?: string } = {}): string {
  const proof = series === "closed" ? "https://www.nyse.com/markets/hours-calendars" : "https://www.federalreserve.gov/monetarypolicy/fomccalendars.htm";
  return `INSERT INTO macro_events (id, series, ref_period, variant, starts_at, et_date, tier, status, proof_url, release_url, moved_from, missing_count, first_seen_at, checked_at, updated_at)
VALUES ('${id}','${series}',${q(o.ref ?? null)},NULL,${q(o.startsAt ?? null)},'${date}',${o.tier ?? 1},'${o.status ?? "scheduled"}','${proof}',NULL,NULL,0,'t','t','t');\n`;
}

export const AAPL_ACC = "0000320193-26-000070";
export const AAPL_KEY = `earnings/AAPL/${AAPL_ACC}.json`;

/** The week of Mon 2026-10-26 with briefs, two checks, calendar rows of every kind, events and a closure. */
export function seed(): string {
  return (
    post({ type: "premarket", date: "2026-10-29", slug: "premarket-2026-10-29", factsKey: "premarket/2026-10-29.json", publishedAt: "2026-10-29T12:05:00Z" }) +
    post({ type: "recap", date: "2026-10-29", slug: "recap-2026-10-29", factsKey: "recap/2026-10-29.json", publishedAt: "2026-10-29T21:00:00Z" }) +
    post({
      type: "premarket",
      date: "2026-10-30",
      slug: "premarket-2026-10-30",
      factsKey: "premarket/2026-10-30.json",
      publishedAt: "2026-10-30T12:05:00Z",
      revisedAt: "2026-10-30T12:40:00Z",
    }) +
    check("AAPL", 320193, AAPL_ACC, "2026-10-30", "aapl-q4-fy2026-earnings") +
    check("MSFT", 789019, "0000789019-26-000050", "2026-10-28", "msft-q1-fy2027-earnings", "pending") +
    cal("AAPL", "2026-10-30", "reported", "amc", "https://www.sec.gov/Archives/edgar/data/320193/000032019326000070/0000320193-26-000070-index.htm") +
    cal("MSFT", "2026-10-28", "reported") +
    cal("XOM", "2026-10-30", "unconfirmed", "bmo") +
    cal("A", "2026-10-27", "reported", "amc", "https://www.sec.gov/Archives/edgar/data/1090872/000109087226000041/0001090872-26-000041-index.htm") +
    cal("A", "2027-02-20", "unconfirmed") +
    cal("TSLA", "2026-10-29", "quarantined") +
    cal("KO", "2026-11-03", "confirmed", "bmo") +
    cal("NVDA", "2026-11-18", "unconfirmed") +
    macro("fed:fomc:2026-10", "fomc", "2026-10-28", { ref: "2026-10", startsAt: "2026-10-28T18:00:00Z" }) +
    macro("nyse:closed:2026-11-26", "closed", "2026-11-26", { tier: 1 })
  );
}

/** The premarket fixture with themes, a watch list and a mover whose Why is PreBell's own check. */
export function premarketFacts(): Record<string, unknown> {
  const f = fixture("premarket-2026-10-30.json");
  f.themes = [
    {
      group: "Semiconductors",
      group_zh: "半导体",
      direction: "Down",
      median_pct: -4.1,
      members: [
        { ticker: "INTC", change_pct: -26.0 },
        { ticker: "ARM", change_pct: -10.08 },
      ],
      more: 2,
    },
  ];
  f.watch_list = {
    Value: [
      {
        kind: "event",
        id: "bls:cpi:2026-10",
        time_et: "8:30 AM ET",
        time_zh: "上午8:30（美东）",
        title: "October CPI",
        title_zh: "10月消费者物价指数（CPI）",
        status: "released",
        link: "https://www.bls.gov/news.release/archives/cpi_11102026.htm",
        figures: [{ key: "cpi_year", value: 3.1, quote: "The all items index rose 3.1 percent for the 12 months ending October." }],
      },
      { kind: "earnings", id: "earnings:AAPL", time_et: null, time_zh: null, title: "Apple earnings", title_zh: "苹果财报", status: "confirmed" },
    ],
  };
  const losers = f.losers as Record<string, unknown>[];
  losers[1].fallback = { kind: "results", text_en: "Reported results after Thursday's close", text_zh: "周四盘后公布业绩", post_slug: "hpq-q4-fy2026-earnings" };
  return f;
}

/** An R2 fake over a map of keys to JSON text, counting reads. */
export function bucket(objects: Record<string, string | object> = {}) {
  const gets: string[] = [];
  return {
    gets,
    async get(key: string) {
      gets.push(key);
      const v = objects[key];
      if (v === undefined) return null;
      return { text: async () => (typeof v === "string" ? v : JSON.stringify(v)) };
    },
  };
}

/** The default objects: both briefs' facts and AAPL's check facts. */
export function objects(): Record<string, string | object> {
  return {
    "premarket/2026-10-30.json": premarketFacts(),
    "recap/2026-10-29.json": fixture("recap-sample.json"),
    [AAPL_KEY]: fixture("earnings-AAPL-0000320193-26-000070.json"),
  };
}

/** A D1 wrapper counting `prepare` calls (one per read). */
export function counted(db: D1Like) {
  const sql: string[] = [];
  return {
    sql,
    db: {
      prepare(s: string) {
        sql.push(s);
        return db.prepare(s);
      },
    } as D1Like,
  };
}

/** An in-memory Cache API. */
export function memoryCache(): EdgeCache & { keys: string[] } {
  const store = new Map<string, string>();
  return {
    get keys() {
      return [...store.keys()];
    },
    async match(req: Request) {
      const v = store.get(req.url);
      return v === undefined ? undefined : new Response(v);
    },
    async put(req: Request, res: Response) {
      if (req.method !== "GET") throw new TypeError("Cache.put only stores GET requests");
      store.set(req.url, await res.text());
    },
  };
}

export function deps(over: Partial<McpDeps> = {}): McpDeps & { lines: string[] } {
  const lines: string[] = [];
  return {
    db: memoryD1(seed()),
    bucket: bucket(objects()),
    origin: new URL("https://prebell.site"),
    now: FRIDAY,
    enabled: true,
    siteUrl: "https://prebell.site",
    cache: memoryCache(),
    log: (l: string) => lines.push(l),
    lines,
    ...over,
  };
}

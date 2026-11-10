/**
 * The MCP server's three read-only tools (see docs/superpowers/specs/2026-11-10-mcp-server-design.md). They read
 * only what the site publishes: posts, the earnings calendar, macro events, and the facts the posts were written
 * from. Each call does at most three small D1 reads and one R2 read, and never fetches anything else.
 */

import { COMPANIES } from "./companies.generated";
import {
  addDays,
  isDate,
  listCalendarRange,
  listTickerCalendar,
  normalizeTicker,
  todayEt,
  weekStart,
  type CalendarEntry,
} from "./earnings";
import { loadEarningsFacts, METRIC_LABELS, periodFromSlug, periodLabel } from "./earningsfacts";
import { listEvents, type MacroEvent } from "./events";
import { parseFacts, type IndexView, type MoverView, type R2Like } from "./facts";
import { indexName } from "./indexnames";
import type { D1Like, Lang } from "./posts";
import { formatEt, formatEtTime, formatWeekday } from "./time";
import { translator } from "./translator";

export interface ToolContext {
  db: D1Like;
  bucket: R2Like | undefined;
  /** Site origin for `source` links (`SITE_URL`). */
  origin: URL;
  now: Date;
  /** Hears about R2, JSON and shape errors in stored facts. */
  onError?: (error: unknown) => void;
}

/** A tool call with valid arguments: the cache key of its normalized arguments, and the work. */
export interface PreparedCall {
  key: Record<string, string>;
  run: () => Promise<Record<string, unknown>>;
}

/** Arguments the caller got wrong: a tool execution error the model can correct. */
export class ArgumentError extends Error {}

export const NOTICE: Record<Lang, string> = {
  en: "Informational only: generated automatically from public data; not investment advice; not real-time.",
  zh: "仅供参考：根据公开数据自动生成，不构成投资建议，非实时数据。",
};

export const INSTRUCTIONS =
  "PreBell (https://prebell.site) publishes automatic US market briefs and earnings checks in English and Chinese. " +
  "These tools read only what the site has already published and never fetch live prices: a brief's prices are a " +
  "snapshot with its as_of time. Summaries, mover reasons and notes are machine-written from the linked sources; each " +
  "earnings figure carries the press-release sentence it came from. Cite the source URL. " +
  NOTICE.en;

/* ---------- schemas ---------- */

const LANG = { type: "string", enum: ["en", "zh"], description: "Language of titles, summaries and labels. Default en." };
const FULL_TEXT = { type: "boolean", description: "Also return the full article text (markdown). Default false." };
const DATE = { type: "string", pattern: "^\\d{4}-\\d{2}-\\d{2}$" };
const URL_OR_NULL = { type: ["string", "null"] };
const STR_OR_NULL = { type: ["string", "null"] };
const NUM_OR_NULL = { type: ["number", "null"] };
const COMMON = {
  notice: { type: "string", description: "Fixed disclaimer: informational, automatic, not advice, not real-time." },
  source: { type: "string", description: "The page on prebell.site this answer comes from." },
};
const READ_ONLY = { readOnlyHint: true, destructiveHint: false, idempotentHint: true, openWorldHint: false };

const NOTE = {
  type: "object",
  properties: { summary_en: { type: "string" }, quote: { type: "string" } },
  required: ["summary_en", "quote"],
};

const MOVER = {
  type: "object",
  properties: {
    ticker: { type: "string" },
    name: { type: "string" },
    last: { type: "number" },
    change_pct: { type: "number", description: "Percent: -4.2 is -4.2%." },
    reason_en: { ...STR_OR_NULL, description: "English only, as on the page. Null means no clear news was found." },
    reason_url: URL_OR_NULL,
    reason_check_url: { ...URL_OR_NULL, description: "PreBell's earnings check behind the reason, when there is one." },
  },
  required: ["ticker", "name", "last", "change_pct", "reason_en", "reason_url", "reason_check_url"],
};

export const TOOLS = [
  {
    name: "earnings_calendar",
    title: "Earnings calendar",
    description:
      "Who reports earnings on which days (US companies on PreBell's calendar), with that day's economic events " +
      "(Fed decisions, jobs report, CPI, elections, market closures). Status: unconfirmed = the date comes from a " +
      "data aggregator and the company has not confirmed it; confirmed = the company or an SEC filing confirmed it; " +
      "reported = the results are out (filing_url is the SEC 8-K when known). Slot: before_open, after_close or " +
      "unknown. writes_check tells whether PreBell writes an earnings check for the company; check_url is the " +
      "published check. Dates are US Eastern. Default range: the current week, Monday to Friday; at most 21 days.",
    inputSchema: {
      type: "object",
      properties: {
        from: { ...DATE, description: "First day, YYYY-MM-DD (US Eastern)." },
        to: { ...DATE, description: "Last day, YYYY-MM-DD; at most 21 days from `from`, inclusive." },
        tickers: {
          type: "array",
          items: { type: "string", pattern: "^[A-Za-z][A-Za-z0-9.-]{0,9}$" },
          maxItems: 50,
          description: "Only these companies (up to 50), e.g. [\"AAPL\", \"BRK.B\"].",
        },
        lang: LANG,
      },
      additionalProperties: false,
    },
    outputSchema: {
      type: "object",
      properties: {
        from: { type: "string" },
        to: { type: "string" },
        lang: { type: "string" },
        as_of: { type: "string", description: "When this was read (UTC)." },
        days: {
          type: "array",
          items: {
            type: "object",
            properties: {
              date: { type: "string" },
              weekday: { type: "string" },
              companies: {
                type: "array",
                items: {
                  type: "object",
                  properties: {
                    ticker: { type: "string" },
                    name: STR_OR_NULL,
                    date: { type: "string" },
                    slot: { type: "string", enum: ["before_open", "after_close", "unknown"] },
                    status: { type: "string", enum: ["unconfirmed", "confirmed", "reported"] },
                    writes_check: { type: "boolean" },
                    check_url: URL_OR_NULL,
                    filing_url: URL_OR_NULL,
                  },
                  required: ["ticker", "name", "date", "slot", "status", "writes_check", "check_url", "filing_url"],
                },
              },
              events: {
                type: "array",
                items: {
                  type: "object",
                  properties: {
                    title: { type: "string" },
                    note: STR_OR_NULL,
                    date: { type: "string" },
                    starts_at: { ...STR_OR_NULL, description: "UTC; null for all-day events." },
                    time_et: STR_OR_NULL,
                    status: { type: "string" },
                    importance: { type: "string", enum: ["major", "notable"] },
                    official_url: { type: "string" },
                    release_url: URL_OR_NULL,
                    moved_from: STR_OR_NULL,
                  },
                  required: ["title", "date", "status", "importance", "official_url"],
                },
              },
            },
            required: ["date", "weekday", "companies", "events"],
          },
        },
        tickers_without_dates: { type: "array", items: { type: "string" } },
        ...COMMON,
      },
      required: ["from", "to", "lang", "as_of", "days", "notice", "source"],
    },
    annotations: { title: "Earnings calendar", ...READ_ONLY },
  },
  {
    name: "earnings_report",
    title: "Earnings report",
    description:
      "One company's latest results as PreBell checked them: fiscal period, filing date, title and summary, key " +
      "figures (each with the amount as the company wrote it and the verbatim press-release sentence), guidance and " +
      "highlights with their quotes, the share move when the check recorded one, and links to the check, the SEC " +
      "filing and the press release. Without a published check it says whether the company has reported (with the " +
      "filing link) or its next calendar date, and whether PreBell writes checks for it. Quotes are English.",
    inputSchema: {
      type: "object",
      properties: {
        ticker: { type: "string", pattern: "^[A-Za-z][A-Za-z0-9.-]{0,9}$", description: "e.g. AAPL or BRK.B." },
        lang: LANG,
        full_text: FULL_TEXT,
      },
      required: ["ticker"],
      additionalProperties: false,
    },
    outputSchema: {
      type: "object",
      properties: {
        status: {
          type: "string",
          enum: ["check_published", "no_check", "unknown_ticker"],
          description: "check_published: the fields below describe PreBell's check. no_check: see message.",
        },
        ticker: { type: "string" },
        company: STR_OR_NULL,
        message: { type: "string" },
        writes_check: { type: "boolean" },
        title: { type: "string" },
        summary: { type: "string" },
        text_lang: { type: "string", enum: ["en", "zh"], description: "Language of title, summary and article." },
        url: { type: "string" },
        published_at: STR_OR_NULL,
        updated_at: STR_OR_NULL,
        fiscal_period: {
          type: ["object", "null"],
          properties: {
            quarter: { type: ["integer", "null"] },
            fiscal_year: { type: "integer" },
            label: { type: "string" },
            quote: STR_OR_NULL,
          },
          required: ["quarter", "fiscal_year", "label"],
        },
        filing_date: STR_OR_NULL,
        filing_url: URL_OR_NULL,
        press_release_url: URL_OR_NULL,
        figures_available: { type: "boolean" },
        figures: {
          type: "array",
          items: {
            type: "object",
            properties: {
              key: { type: "string" },
              label: { type: "string" },
              value: { type: "number", description: "Base units: dollars, dollars per share, or percent." },
              unit: { type: "string", enum: ["usd", "usd_per_share", "percent"] },
              amount: { type: "string", description: "As the company wrote it." },
              quote: { type: "string", description: "Verbatim press-release text containing the amount." },
            },
            required: ["key", "label", "value", "unit", "amount", "quote"],
          },
        },
        guidance: { type: "array", items: NOTE },
        highlights: { type: "array", items: NOTE },
        share_move: {
          type: ["object", "null"],
          properties: {
            session: { type: "string", enum: ["premarket", "regular", "after_hours"] },
            change_pct: { type: "number" },
            last: { type: "number" },
            as_of: { type: "string" },
          },
          required: ["session", "change_pct", "last", "as_of"],
        },
        article: { type: "string" },
        reported: { type: "boolean" },
        last_report: {
          type: ["object", "null"],
          properties: { date: { type: "string" }, filing_url: URL_OR_NULL },
          required: ["date", "filing_url"],
        },
        next_report: {
          type: ["object", "null"],
          properties: { date: { type: "string" }, slot: { type: "string" }, status: { type: "string" } },
          required: ["date", "slot", "status"],
        },
        as_of: { type: "string" },
        ...COMMON,
      },
      required: ["status", "ticker", "notice", "source"],
    },
    annotations: { title: "Earnings report", ...READ_ONLY },
  },
  {
    name: "latest_brief",
    title: "Latest market brief",
    description:
      "The newest premarket brief (published from 8:00 AM ET on trading days, rebuilt at 8:35 after the 8:30 data) " +
      "or after-close recap (from 4:45 PM ET): title, summary, index futures or closes, the biggest movers with the " +
      "published reason and its source link, industry themes, and the day's events to watch. Prices are a snapshot " +
      "taken at as_of, never live quotes. is_today is false when today's brief is not out yet; the newest one is " +
      "returned with a note saying when the next is due.",
    inputSchema: {
      type: "object",
      properties: {
        kind: {
          type: "string",
          enum: ["premarket", "recap"],
          description: "Premarket brief or after-close recap. Default: whichever is newest.",
        },
        lang: LANG,
        full_text: FULL_TEXT,
      },
      additionalProperties: false,
    },
    outputSchema: {
      type: "object",
      properties: {
        found: { type: "boolean" },
        message: { type: "string" },
        kind: { type: "string", enum: ["premarket", "recap"] },
        trading_date: { type: "string" },
        title: { type: "string" },
        summary: { type: "string" },
        text_lang: { type: "string", enum: ["en", "zh"] },
        url: { type: "string" },
        published_at: STR_OR_NULL,
        updated_at: STR_OR_NULL,
        as_of: { ...STR_OR_NULL, description: "When the prices were taken (UTC); null if the snapshot is unavailable." },
        snapshot_note: { type: "string" },
        is_today: { type: "boolean" },
        today_et: { type: "string" },
        not_today_note: { type: "string" },
        next_brief_due: {
          type: "object",
          properties: { date: STR_OR_NULL, time_et: { type: "string" }, text: { type: "string" } },
          required: ["date", "time_et", "text"],
        },
        market_data_available: { type: "boolean" },
        indexes: {
          type: "array",
          items: {
            type: "object",
            properties: {
              symbol: { type: "string" },
              name: { type: "string" },
              last: NUM_OR_NULL,
              change: NUM_OR_NULL,
              change_pct: NUM_OR_NULL,
            },
            required: ["symbol", "name", "last", "change", "change_pct"],
          },
        },
        gainers: { type: "array", items: MOVER },
        losers: { type: "array", items: MOVER },
        themes: {
          type: "array",
          items: {
            type: "object",
            properties: {
              group: { type: "string" },
              direction: { type: "string", enum: ["up", "down"] },
              median_pct: { type: "number" },
              members: {
                type: "array",
                items: {
                  type: "object",
                  properties: { ticker: { type: "string" }, change_pct: { type: "number" } },
                  required: ["ticker", "change_pct"],
                },
              },
              more: { type: "integer" },
            },
            required: ["group", "direction", "median_pct", "members", "more"],
          },
        },
        watch: {
          type: "array",
          items: {
            type: "object",
            properties: {
              kind: { type: "string", enum: ["event", "earnings"] },
              title: { type: "string" },
              time: STR_OR_NULL,
              note: STR_OR_NULL,
              status: { type: "string" },
              link: URL_OR_NULL,
              moved_from: STR_OR_NULL,
              figures: {
                type: "array",
                items: {
                  type: "object",
                  properties: { key: { type: "string" }, value: { type: "number" }, quote: { type: "string" } },
                  required: ["key", "value", "quote"],
                },
              },
            },
            required: ["kind", "title", "time", "status", "link"],
          },
        },
        article: { type: "string" },
        ...COMMON,
      },
      required: ["found", "notice", "source"],
    },
    annotations: { title: "Latest market brief", ...READ_ONLY },
  },
] as const;

export type ToolName = (typeof TOOLS)[number]["name"];
export const isToolName = (v: unknown): v is ToolName => TOOLS.some((t) => t.name === v);

/* ---------- argument checks ---------- */

function only(args: Record<string, unknown>, allowed: string[]): void {
  const extra = Object.keys(args).filter((k) => !allowed.includes(k));
  if (extra.length) throw new ArgumentError(`Unknown argument: ${extra[0].slice(0, 40)}. Allowed: ${allowed.join(", ")}.`);
}

function langArg(v: unknown): Lang {
  if (v === undefined) return "en";
  if (v === "en" || v === "zh") return v;
  throw new ArgumentError('lang must be "en" or "zh".');
}

function boolArg(v: unknown, name: string): boolean {
  if (v === undefined) return false;
  if (typeof v === "boolean") return v;
  throw new ArgumentError(`${name} must be true or false.`);
}

function tickerArg(v: unknown): string {
  const t = typeof v === "string" ? normalizeTicker(v.trim()) : null;
  if (!t) throw new ArgumentError(`Not a ticker: ${JSON.stringify(v)?.slice(0, 20)}. Use a symbol like AAPL or BRK.B.`);
  return t;
}

function dateArg(v: unknown, name: string): string | undefined {
  if (v === undefined) return undefined;
  if (typeof v === "string" && isDate(v)) return v;
  throw new ArgumentError(`${name} must be a date written YYYY-MM-DD.`);
}

/** Calendar range: at most 21 days counted inclusively. */
export const MAX_RANGE_DAYS = 21;
export const MAX_TICKERS = 50;

const days = (from: string, to: string) => Math.round((Date.parse(to) - Date.parse(from)) / 86_400_000);

/**
 * Checks a tool's arguments and returns its cache key and work, or throws `ArgumentError`. `args` is the
 * `arguments` object of the call.
 */
export function prepareCall(name: ToolName, args: Record<string, unknown>, ctx: ToolContext): PreparedCall {
  switch (name) {
    case "earnings_calendar": {
      only(args, ["from", "to", "tickers", "lang"]);
      const lang = langArg(args.lang);
      let from = dateArg(args.from, "from");
      let to = dateArg(args.to, "to");
      if (!from && !to) {
        from = weekStart(todayEt(ctx.now));
        to = addDays(from, 4);
      } else if (!from) from = addDays(to!, -6);
      else if (!to) to = addDays(from, 6);
      if (to! < from) throw new ArgumentError("to must not be before from.");
      if (days(from, to!) >= MAX_RANGE_DAYS)
        throw new ArgumentError(`The range is at most ${MAX_RANGE_DAYS} days; ask for several ranges instead.`);
      let tickers: string[] = [];
      if (args.tickers !== undefined) {
        if (!Array.isArray(args.tickers)) throw new ArgumentError("tickers must be a list of symbols.");
        if (args.tickers.length > MAX_TICKERS) throw new ArgumentError(`At most ${MAX_TICKERS} tickers per call.`);
        tickers = [...new Set(args.tickers.map(tickerArg))].sort();
      }
      const range = { from, to: to! };
      return {
        key: { from: range.from, to: range.to, tickers: tickers.join(","), lang },
        run: () => earningsCalendar(ctx, range.from, range.to, tickers, lang),
      };
    }
    case "earnings_report": {
      only(args, ["ticker", "lang", "full_text"]);
      if (args.ticker === undefined) throw new ArgumentError("ticker is required, e.g. AAPL.");
      const ticker = tickerArg(args.ticker);
      const lang = langArg(args.lang);
      const fullText = boolArg(args.full_text, "full_text");
      return {
        key: { ticker, lang, full_text: String(fullText) },
        run: () => earningsReport(ctx, ticker, lang, fullText),
      };
    }
    case "latest_brief": {
      only(args, ["kind", "lang", "full_text"]);
      const kind = args.kind;
      if (kind !== undefined && kind !== "premarket" && kind !== "recap")
        throw new ArgumentError('kind must be "premarket" or "recap".');
      const lang = langArg(args.lang);
      const fullText = boolArg(args.full_text, "full_text");
      return {
        key: { kind: kind ?? "", lang, full_text: String(fullText) },
        run: () => latestBrief(ctx, kind ?? null, lang, fullText),
      };
    }
  }
}

/* ---------- earnings_calendar ---------- */

const SLOT = { bmo: "before_open", amc: "after_close", unknown: "unknown" } as const;
const postUrl = (ctx: ToolContext, lang: Lang, slug: string) => new URL(`/${lang}/p/${slug}`, ctx.origin).href;

function company(ctx: ToolContext, lang: Lang, e: CalendarEntry) {
  return {
    ticker: e.ticker,
    name: e.name,
    date: e.date,
    slot: SLOT[e.slot],
    status: e.status,
    writes_check: e.writeUps,
    check_url: e.postSlug ? postUrl(ctx, lang, e.postSlug) : null,
    filing_url: e.filingUrl,
  };
}

function event(e: MacroEvent, lang: Lang) {
  return {
    title: e.title,
    note: e.note,
    date: e.date,
    starts_at: e.startsAt,
    time_et: e.startsAt ? formatEtTime(e.startsAt, lang) || null : null,
    status: e.status,
    importance: e.tier === 1 ? "major" : "notable",
    official_url: e.proofUrl,
    release_url: e.releaseUrl,
    moved_from: e.movedFrom,
  };
}

const isWeekday = (date: string) => {
  const d = new Date(`${date}T12:00:00Z`).getUTCDay();
  return d !== 0 && d !== 6;
};

async function earningsCalendar(ctx: ToolContext, from: string, to: string, tickers: string[], lang: Lang) {
  const [entries, events] = await Promise.all([
    listCalendarRange(ctx.db, from, to, lang, tickers),
    listEvents(ctx.db, from, to, lang),
  ]);
  const out = [];
  for (let d = from; d <= to; d = addDays(d, 1)) {
    const companies = entries.filter((e) => e.date === d).map((e) => company(ctx, lang, e));
    const evs = (events[d] ?? []).map((e) => event(e, lang));
    if (isWeekday(d) || companies.length || evs.length)
      out.push({ date: d, weekday: formatWeekday(d, lang), companies, events: evs });
  }
  const seen = new Set(entries.map((e) => e.ticker));
  return {
    from,
    to,
    lang,
    as_of: ctx.now.toISOString(),
    days: out,
    ...(tickers.length ? { tickers_without_dates: tickers.filter((t) => !seen.has(t)) } : {}),
    notice: NOTICE[lang],
    source: new URL(`/${lang}/earnings?week=${weekStart(from)}`, ctx.origin).href,
  };
}

/* ---------- earnings_report ---------- */

const MSG = {
  unknown: {
    en: (t: string) => `PreBell has no calendar date or earnings check for ${t}. It covers S&P 500 companies and a few more.`,
    zh: (t: string) => `盘前铃没有 ${t} 的财报日期或财报核查。它覆盖标普500成分股及少量其他公司。`,
  },
  calendarOnly: {
    en: "PreBell writes no earnings check for this company (calendar only).",
    zh: "盘前铃不为这家公司撰写财报核查（仅列入财报日历）。",
  },
  noneYet: {
    en: "PreBell has not published an earnings check for this company yet.",
    zh: "盘前铃尚未发布这家公司的财报核查。",
  },
  noFigures: {
    en: "The figures for this check are not available; read the check or the filing.",
    zh: "这篇核查的数据暂不可用，请阅读核查原文或公司文件。",
  },
} as const;

interface ReportRow {
  slug: string;
  trading_date: string;
  published_at: string | null;
  revised_at: string | null;
  facts_r2_key: string | null;
  cik: number;
  accession: string;
  filing_date: string;
  t_status: string | null;
  t_title: string | null;
  t_summary: string | null;
  t_body: string | null;
  en_title: string | null;
  en_summary: string | null;
  en_body: string | null;
}

/** The EDGAR filing index for an accession, the form the generator links. */
const filingIndex = (cik: number, acc: string) =>
  `https://www.sec.gov/Archives/edgar/data/${cik}/${acc.replace(/-/g, "")}/${acc}-index.htm`;

async function earningsReport(ctx: ToolContext, ticker: string, lang: Lang, fullText: boolean) {
  const body = fullText ? "t.body_md AS t_body, en.body_md AS en_body" : "NULL AS t_body, NULL AS en_body";
  // The newest check whose English text is published (the calendar's rule), with this language's text if any.
  const row = await ctx.db
    .prepare(
      `SELECT p.slug, p.trading_date, p.published_at, p.revised_at, p.facts_r2_key, f.cik, f.accession, f.filing_date,
         t.status AS t_status, t.title AS t_title, t.summary AS t_summary, en.title AS en_title, en.summary AS en_summary,
         ${body}
       FROM earnings_filings f
       CROSS JOIN posts p ON p.run_key = f.run_key
       CROSS JOIN post_translations en ON en.post_id = p.id AND en.lang = 'en' AND en.status = 'published'
       LEFT JOIN post_translations t ON t.post_id = p.id AND t.lang = ?
       WHERE f.ticker = ?
       ORDER BY p.trading_date DESC, p.published_at DESC LIMIT 1`,
    )
    .bind(lang, ticker)
    .first<ReportRow>();
  const known = COMPANIES[ticker];
  const base = { ticker, company: known?.[0] ?? null, notice: NOTICE[lang] };

  if (!row) {
    const rows = await listTickerCalendar(ctx.db, ticker, lang);
    if (!rows.length && !known)
      return {
        status: "unknown_ticker",
        ...base,
        message: MSG.unknown[lang](ticker),
        as_of: ctx.now.toISOString(),
        source: new URL(`/${lang}/earnings`, ctx.origin).href,
      };
    const today = todayEt(ctx.now);
    const last = rows.find((r) => r.status === "reported");
    // Rows are newest first: the last upcoming one is the nearest.
    const next = rows.filter((r) => r.date >= today && r.status !== "reported").at(-1);
    const writes = known?.[1] ?? false;
    return {
      status: "no_check",
      ...base,
      writes_check: writes,
      message: writes ? MSG.noneYet[lang] : MSG.calendarOnly[lang],
      reported: Boolean(last),
      last_report: last ? { date: last.date, filing_url: last.filingUrl } : null,
      next_report: next ? { date: next.date, slot: SLOT[next.slot], status: next.status } : null,
      as_of: ctx.now.toISOString(),
      source: new URL(`/${lang}/t/${encodeURIComponent(ticker)}`, ctx.origin).href,
    };
  }

  const translated = lang === "zh" && row.t_status === "published";
  const textLang: Lang = translated ? "zh" : "en";
  const facts = await loadEarningsFacts(ctx.bucket, row.facts_r2_key, ticker, ctx.onError);
  const p = facts?.fiscalPeriod ?? periodFromSlug(row.slug);
  const url = postUrl(ctx, lang, row.slug);
  return {
    status: "check_published",
    ...base,
    company: base.company ?? facts?.company ?? null,
    writes_check: known?.[1] ?? true,
    title: (translated ? row.t_title : row.en_title) ?? "",
    summary: (translated ? row.t_summary : row.en_summary) ?? "",
    text_lang: textLang,
    url,
    published_at: row.published_at,
    updated_at: row.revised_at ?? row.published_at,
    fiscal_period: p
      ? {
          quarter: p.quarter,
          fiscal_year: p.fiscalYear,
          label: periodLabel(p, lang),
          quote: facts?.fiscalPeriod?.quote ?? null,
        }
      : null,
    filing_date: facts?.filingDate ?? row.filing_date,
    filing_url: facts?.filingUrl ?? filingIndex(row.cik, row.accession),
    press_release_url: facts?.pressReleaseUrl ?? null,
    figures_available: facts !== null,
    ...(facts ? {} : { message: MSG.noFigures[lang] }),
    figures: (facts?.figures ?? []).map((f) => ({
      key: f.key,
      label: METRIC_LABELS[f.key][lang],
      value: f.value,
      unit: f.unit,
      amount: f.amount,
      quote: f.quote,
    })),
    guidance: (facts?.guidance ?? []).map((n) => ({ summary_en: n.summaryEn, quote: n.quote })),
    highlights: (facts?.highlights ?? []).map((n) => ({ summary_en: n.summaryEn, quote: n.quote })),
    share_move: facts?.shareMove
      ? {
          session: facts.shareMove.session,
          change_pct: facts.shareMove.changePct,
          last: facts.shareMove.last,
          as_of: facts.shareMove.asOf,
        }
      : null,
    ...(fullText ? { article: (translated ? row.t_body : row.en_body) ?? "" } : {}),
    source: url,
  };
}

/* ---------- latest_brief ---------- */

type Kind = "premarket" | "recap";

interface BriefRow {
  slug: string;
  content_type: Kind;
  trading_date: string;
  published_at: string | null;
  revised_at: string | null;
  facts_r2_key: string | null;
  t_status: string | null;
  t_title: string | null;
  t_summary: string | null;
  t_body: string | null;
  en_title: string | null;
  en_summary: string | null;
  en_body: string | null;
}

const BRIEF_KEY = /^(premarket|recap)\/\d{4}-\d{2}-\d{2}\.json$/;
const httpsUrl = (v: unknown): string | null => (typeof v === "string" && /^https:\/\/[^\s]+$/.test(v) ? v : null);
const text = (v: unknown): string | null => (typeof v === "string" && v.trim() !== "" ? v : null);

/** The facts' "What to watch" items (`events::watch::WatchItem`), leniently; null when the facts hold none. */
export function parseWatchList(json: unknown, lang: Lang) {
  const w = typeof json === "object" && json !== null ? (json as Record<string, unknown>).watch_list : null;
  const items = typeof w === "object" && w !== null && "Value" in w ? (w as { Value: unknown }).Value : null;
  if (!Array.isArray(items)) return null;
  const out = [];
  for (const i of items) {
    if (typeof i !== "object" || i === null) continue;
    const o = i as Record<string, unknown>;
    const kind = o.kind === "event" || o.kind === "earnings" ? o.kind : null;
    const title = text(lang === "zh" ? o.title_zh : o.title) ?? text(o.title);
    const status = text(o.status);
    if (!kind || !title || !status) continue;
    const figures = Array.isArray(o.figures)
      ? o.figures.flatMap((f) => {
          const r = (typeof f === "object" && f !== null ? f : {}) as Record<string, unknown>;
          const key = text(r.key);
          const quote = text(r.quote);
          return key && quote && typeof r.value === "number" && Number.isFinite(r.value)
            ? [{ key, value: r.value, quote }]
            : [];
        })
      : [];
    out.push({
      kind,
      title,
      time: text(lang === "zh" ? o.time_zh : o.time_et),
      note: text(lang === "zh" ? o.note_zh : o.note),
      status,
      link: httpsUrl(o.link),
      moved_from: text(o.moved_from),
      ...(figures.length ? { figures } : {}),
    });
  }
  return out.length ? out : null;
}

const etClock = new Intl.DateTimeFormat("en-US", {
  timeZone: "America/New_York",
  hour: "2-digit",
  minute: "2-digit",
  hourCycle: "h23",
});

/** When each kind is due in US Eastern ("HH:MM"), for the "next brief due" line. */
const DUE: Record<Kind, { at: string; en: string; zh: string }> = {
  premarket: { at: "08:00", en: "8:00 AM ET", zh: "美东时间上午8:00" },
  recap: { at: "16:45", en: "4:45 PM ET", zh: "美东时间下午4:45" },
};

/**
 * Upcoming closure dates from the events table, or null when it holds none (not loaded yet, or the table is
 * missing before migration 0008), in which case no date is promised.
 */
async function closures(db: D1Like, today: string): Promise<Set<string> | null> {
  try {
    const { results } = await db
      .prepare(`SELECT et_date FROM macro_events WHERE series = 'closed' AND et_date >= ? ORDER BY et_date LIMIT 10`)
      .bind(today)
      .all<{ et_date: string }>();
    return results.length ? new Set(results.map((r) => r.et_date)) : null;
  } catch {
    return null;
  }
}

async function nextDue(ctx: ToolContext, kind: Kind, lang: Lang) {
  const today = todayEt(ctx.now);
  const due = DUE[kind];
  const closed = await closures(ctx.db, today);
  const label = kind === "premarket" ? { en: "premarket brief", zh: "盘前简报" } : { en: "after-close recap", zh: "收盘回顾" };
  if (!closed) {
    return {
      date: null,
      time_et: due.at,
      text:
        lang === "en"
          ? `The next ${label.en} is due at ${due.en} on the next trading day.`
          : `下一期${label.zh}将在下一个交易日${due.zh}发布。`,
    };
  }
  const trading = (d: string) => isWeekday(d) && !closed.has(d);
  let date = today;
  if (!trading(today) || etClock.format(ctx.now) >= due.at) {
    // Today's is late (or there is none today): name today only when it is a trading day.
    if (trading(today)) {
      return {
        date: today,
        time_et: due.at,
        text:
          lang === "en"
            ? `Today's ${label.en} is normally out by ${due.en} but has not been published yet.`
            : `今天的${label.zh}通常在${due.zh}发布，目前尚未发布。`,
      };
    }
    do date = addDays(date, 1);
    while (!trading(date) && date < addDays(today, 14));
  }
  return {
    date,
    time_et: due.at,
    text: lang === "en" ? `The next ${label.en} is due at ${due.en} on ${date}.` : `下一期${label.zh}将于${date}${due.zh}发布。`,
  };
}

function indexRow(i: IndexView, t: ReturnType<typeof translator>) {
  return {
    symbol: i.symbol,
    name: indexName(t, i.symbol),
    last: i.quote?.last ?? null,
    change: i.quote?.pointChange ?? null,
    change_pct: i.quote?.changePct ?? null,
  };
}

/** A mover as the page prints it: last price and percent change only, never the previous close or volume. */
function moverRow(ctx: ToolContext, lang: Lang, m: MoverView) {
  return {
    ticker: m.symbol,
    name: m.name,
    last: m.quote.last,
    change_pct: m.quote.changePct,
    reason_en: m.reason,
    reason_url: m.reasonPost ? null : m.reasonUrl,
    reason_check_url: m.reasonPost ? postUrl(ctx, lang, m.reasonPost) : null,
  };
}

async function latestBrief(ctx: ToolContext, kind: Kind | null, lang: Lang, fullText: boolean) {
  const body = fullText ? "t.body_md AS t_body, en.body_md AS en_body" : "NULL AS t_body, NULL AS en_body";
  // Visible as the site shows it: published in this language, or (ZH) pending while the English is published.
  const row = await ctx.db
    .prepare(
      `SELECT p.slug, p.content_type, p.trading_date, p.published_at, p.revised_at, p.facts_r2_key,
         t.status AS t_status, t.title AS t_title, t.summary AS t_summary, en.title AS en_title, en.summary AS en_summary,
         ${body}
       FROM posts p
       JOIN post_translations t ON t.post_id = p.id AND t.lang = ?
       LEFT JOIN post_translations en ON en.post_id = p.id AND en.lang = 'en'
       WHERE p.content_type IN (?, ?)
         AND (t.status = 'published' OR (t.lang = 'zh' AND t.status = 'pending' AND en.status = 'published'))
       ORDER BY p.trading_date DESC, p.published_at DESC LIMIT 1`,
    )
    .bind(lang, kind ?? "premarket", kind ?? "recap")
    .first<BriefRow>();
  const today = todayEt(ctx.now);
  const nextKind: Kind = kind ?? "premarket";
  if (!row) {
    return {
      found: false,
      message: lang === "en" ? "PreBell has not published a brief of this kind yet." : "盘前铃尚未发布此类简报。",
      today_et: today,
      next_brief_due: await nextDue(ctx, nextKind, lang),
      as_of: null,
      notice: NOTICE[lang],
      source: new URL(`/${lang}`, ctx.origin).href,
    };
  }

  const translated = row.t_status === "published";
  const textLang: Lang = translated ? lang : "en";
  const report = (e: unknown) => {
    try {
      ctx.onError?.(e);
    } catch {}
  };
  // One R2 read gives both the page's facts view and the watch list (loadFacts keeps only the former).
  let factsJson: unknown = null;
  if (ctx.bucket && row.facts_r2_key && BRIEF_KEY.test(row.facts_r2_key)) {
    try {
      const obj = await ctx.bucket.get(row.facts_r2_key);
      if (obj) factsJson = JSON.parse(await obj.text());
    } catch (e) {
      report(e);
    }
  }
  const facts = factsJson === null ? null : parseFacts(row.content_type, factsJson);
  if (factsJson !== null && !facts) report(new Error(`facts ${row.facts_r2_key}: unexpected shape`));
  const watch = facts && row.content_type === "premarket" ? parseWatchList(factsJson, lang) : null;
  const isToday = row.trading_date === today;
  const t = translator(lang);
  const url = postUrl(ctx, lang, row.slug);
  const asOf = facts?.asOfUtc ?? null;
  const when = asOf ? formatEt(asOf, lang) : "";

  return {
    found: true,
    kind: row.content_type,
    trading_date: row.trading_date,
    title: (translated ? row.t_title : row.en_title) ?? "",
    summary: (translated ? row.t_summary : row.en_summary) ?? "",
    text_lang: textLang,
    url,
    published_at: row.published_at,
    updated_at: row.revised_at ?? row.published_at,
    as_of: asOf,
    snapshot_note: asOf
      ? lang === "en"
        ? `Prices are a snapshot taken at ${when}, not live quotes.`
        : `价格为${when}的快照，并非实时报价。`
      : lang === "en"
        ? "The price snapshot for this brief is not available."
        : "这期简报的价格快照暂不可用。",
    is_today: isToday,
    today_et: today,
    ...(isToday
      ? {}
      : {
          not_today_note:
            lang === "en"
              ? `This is the brief for ${row.trading_date}, not today (${today}).`
              : `这是${row.trading_date}的简报，不是今天（${today}）的。`,
          next_brief_due: await nextDue(ctx, nextKind, lang),
        }),
    market_data_available: facts !== null,
    ...(facts
      ? {
          indexes: [facts.lead, ...facts.others].map((i) => indexRow(i, t)),
          gainers: facts.gainers.map((m) => moverRow(ctx, lang, m)),
          losers: facts.losers.map((m) => moverRow(ctx, lang, m)),
          themes: facts.themes.map((th) => ({
            group: lang === "zh" ? th.groupZh : th.group,
            direction: th.direction,
            median_pct: th.medianPct,
            members: th.members.map((m) => ({ ticker: m.symbol, change_pct: m.changePct })),
            more: th.more,
          })),
        }
      : {}),
    ...(watch ? { watch } : {}),
    ...(fullText ? { article: (translated ? row.t_body : row.en_body) ?? "" } : {}),
    notice: NOTICE[lang],
    source: url,
  };
}

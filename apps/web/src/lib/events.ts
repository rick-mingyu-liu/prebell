/**
 * Read-only queries over the macro events calendar (migrations/0008_macro_events.sql): FOMC decisions
 * and minutes, the jobs report, CPI, elections and market closures, shown above each day's earnings.
 * Titles come from the glossary below, the same strings as crates/core/src/events/mod.rs (a Rust test
 * checks this file), never from the row.
 */

import type { D1Like, Lang } from "./posts";

export type EventStatus = "scheduled" | "confirmed" | "moved" | "postponed" | "released";

export interface MacroEvent {
  id: string;
  /** Glossary key: fomc, fomc_minutes, jobs, cpi, election, closed, early_close. */
  series: string;
  title: string;
  /** The line under the title (the FOMC press conference), or null. */
  note: string | null;
  /** UTC ISO start, or null for all-day events (elections, closures). */
  startsAt: string | null;
  date: string;
  /** 1 major, 2 notable. */
  tier: number;
  status: EventStatus;
  /** The official page that lists the date. */
  proofUrl: string;
  /** The release itself, once out. */
  releaseUrl: string | null;
  /** The previous date while the event is moved. */
  movedFrom: string | null;
}

/** [key, EN, ZH]. `{month}` is the English month name, `{m}` its number. Elections key on their variant. */
const GLOSSARY: [string, string, string][] = [
  ["fomc", "FOMC rate decision", "联邦公开市场委员会（FOMC）议息会议"],
  ["fomc_minutes", "FOMC minutes ({month} meeting)", "FOMC会议纪要（{m}月会议）"],
  ["jobs", "{month} jobs report", "{m}月非农就业报告"],
  ["cpi", "{month} CPI", "{m}月消费者物价指数（CPI）"],
  ["election:midterm", "US midterm elections", "美国中期选举"],
  ["election:general", "US general election", "美国大选"],
  ["closed", "US stock market closed", "美股休市"],
  ["early_close", "Early close, 1:00 PM ET", "美股提前收盘（美东13:00）"],
];
const SEP_SUFFIX: [string, string] = [" (with projections)", "（含经济预测）"];
const FOMC_NOTE: [string, string] = ["Press conference 2:30 PM ET", "美联储主席新闻发布会，美东14:30"];
const MONTHS = [
  "January",
  "February",
  "March",
  "April",
  "May",
  "June",
  "July",
  "August",
  "September",
  "October",
  "November",
  "December",
];

/** The event's title, or null for a series the glossary lacks (such rows are not shown). */
export function eventTitle(series: string, refPeriod: string | null, variant: string | null, lang: Lang): string | null {
  const key = series === "election" && variant ? `election:${variant}` : series;
  const g = GLOSSARY.find(([k]) => k === key);
  if (!g) return null;
  let t = lang === "en" ? g[1] : g[2];
  if (t.includes("{month}") || t.includes("{m}")) {
    const m = Number(refPeriod?.slice(5, 7));
    if (!Number.isInteger(m) || m < 1 || m > 12) return null;
    t = t.replace("{month}", MONTHS[m - 1]).replace("{m}", String(m));
  }
  if (series === "fomc" && variant === "sep") t += lang === "en" ? SEP_SUFFIX[0] : SEP_SUFFIX[1];
  return t;
}

/** Only these official sites are linked (proof and release pages the generator writes). */
const OFFICIAL = /^https:\/\/www\.(bls\.gov|federalreserve\.gov|govinfo\.gov|nyse\.com)\/[\w./?=&%-]*$/;
const official = (u: string | null): string | null => (u && OFFICIAL.test(u) ? u : null);
const STATUSES: EventStatus[] = ["scheduled", "confirmed", "moved", "postponed", "released"];

interface EventRow {
  id: string;
  series: string;
  ref_period: string | null;
  variant: string | null;
  starts_at: string | null;
  et_date: string;
  tier: number;
  status: string;
  proof_url: string;
  release_url: string | null;
  moved_from: string | null;
}

function toEvent(r: EventRow, lang: Lang): MacroEvent | null {
  const title = eventTitle(r.series, r.ref_period, r.variant, lang);
  const proofUrl = official(r.proof_url);
  if (!title || !proofUrl || !STATUSES.includes(r.status as EventStatus)) return null;
  return {
    id: r.id,
    series: r.series,
    title,
    note: r.series === "fomc" ? (lang === "en" ? FOMC_NOTE[0] : FOMC_NOTE[1]) : null,
    startsAt: r.starts_at,
    date: r.et_date,
    tier: r.tier,
    status: r.status as EventStatus,
    proofUrl,
    releaseUrl: official(r.release_url),
    movedFrom: r.moved_from,
  };
}

/**
 * The tier 1-2 events from `from` to `to` (ET dates), by date: all-day events first, then by time.
 * Before migration 0008 is applied the table is missing; the calendar then shows earnings alone.
 */
export async function listEvents(db: D1Like, from: string, to: string, lang: Lang): Promise<Record<string, MacroEvent[]>> {
  let rows: EventRow[];
  try {
    ({ results: rows } = await db
      .prepare(
        `SELECT id, series, ref_period, variant, starts_at, et_date, tier, status, proof_url, release_url, moved_from
         FROM macro_events WHERE et_date BETWEEN ? AND ? AND tier <= 2
         ORDER BY et_date, starts_at IS NOT NULL, starts_at, id`,
      )
      .bind(from, to)
      .all<EventRow>());
  } catch {
    return {};
  }
  const out: Record<string, MacroEvent[]> = {};
  for (const r of rows) {
    const e = toEvent(r, lang);
    if (e) (out[e.date] ??= []).push(e);
  }
  return out;
}

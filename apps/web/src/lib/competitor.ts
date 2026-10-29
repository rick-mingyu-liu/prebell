/**
 * Read-only queries for the internal competitor watch (migrations/0004_competitor.sql).
 * The generator computes each report; this module only reads and parses it.
 */
import type { D1Like } from "./posts";

/** Mirrors `core::competitor::report::ItemRef`. */
export interface ItemRef {
  title: string;
  link: string;
  category: string;
  published_at: string;
}

/** Mirrors `core::competitor::report::Report` (report_json). */
export interface Report {
  report_date: string;
  window_start: string;
  window_end: string;
  /** Our posts are compared from here (previous trading day) to window_end. */
  our_window_start: string;
  their_item_count: number;
  our_post_count: number;
  our_slugs: string[];
  uncovered: { ticker: string; items: ItemRef[] }[];
  covered: string[];
  topics: ItemRef[];
}

export interface CompetitorReport {
  reportDate: string;
  createdAt: string;
  digest: string;
  report: Report;
}

export interface CompetitorItem {
  link: string;
  title: string;
  publishedAt: string | null;
  category: string;
  tickers: string[];
}

/**
 * True only when `expected` is a non-empty string and `given` equals it. Compares every
 * character of equal-length strings so timing doesn't reveal a matching prefix.
 */
export function tokenMatches(given: unknown, expected: string | undefined): boolean {
  if (typeof given !== "string" || !expected) return false;
  if (given.length !== expected.length) return false;
  let diff = 0;
  for (let i = 0; i < expected.length; i++) diff |= given.charCodeAt(i) ^ expected.charCodeAt(i);
  return diff === 0;
}

export async function getLatestReport(db: D1Like): Promise<CompetitorReport | null> {
  const row = await db
    .prepare(
      `SELECT report_date, created_at, digest, report_json FROM competitor_reports
       ORDER BY report_date DESC LIMIT 1`,
    )
    .first<{ report_date: string; created_at: string; digest: string; report_json: string }>();
  if (!row) return null;
  return {
    reportDate: row.report_date,
    createdAt: row.created_at,
    digest: row.digest,
    report: JSON.parse(row.report_json) as Report,
  };
}

/** Stored items, newest first; items without a published time come last. */
export async function listRecentItems(db: D1Like, limit = 100): Promise<CompetitorItem[]> {
  const { results } = await db
    .prepare(
      `SELECT link, title, published_at, category, tickers FROM competitor_items
       ORDER BY published_at IS NULL, published_at DESC, last_seen_at DESC LIMIT ?`,
    )
    .bind(limit)
    .all<{ link: string; title: string; published_at: string | null; category: string | null; tickers: string }>();
  return results.map((r) => ({
    link: r.link,
    title: r.title,
    publishedAt: r.published_at,
    category: r.category ?? "",
    tickers: r.tickers.split(/\s+/).filter(Boolean),
  }));
}

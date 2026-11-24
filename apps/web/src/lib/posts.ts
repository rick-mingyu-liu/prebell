/** Read-only queries over the generator's D1 tables (migrations/0001_init.sql). */

import { COVER_KEY } from "./covers";

export type Lang = "en" | "zh";

/** Content types the site lists and filters by; `posts.content_type` values. */
export const CONTENT_TYPES = ["premarket", "recap", "earnings", "ipo", "week"] as const;
export type ContentType = (typeof CONTENT_TYPES)[number];

/** A known content type from a query parameter, or null. */
export function parseContentType(raw: string | string[] | undefined): ContentType | null {
  const v = Array.isArray(raw) ? raw[0] : raw;
  return CONTENT_TYPES.find((t) => t === v) ?? null;
}

/** The slice of the D1 API these queries use, so tests can pass an SQLite adapter. */
export interface D1StatementLike {
  bind(...values: unknown[]): D1StatementLike;
  first<T = Record<string, unknown>>(): Promise<T | null>;
  all<T = Record<string, unknown>>(): Promise<{ results: T[] }>;
}
export interface D1Like {
  prepare(sql: string): D1StatementLike;
}

/** What a post's cover and kicker need: the R2 cover image, its two lines in this language, the ticker. */
export interface PostCover {
  /** R2 key of the cover image (`FACTS` bucket, served at `/<key>`), if the generator stored one. */
  coverKey: string | null;
  /** The cover's two headline lines in this language (the second may be ""), or null without a cover. */
  coverLines: [string, string] | null;
  /** The earnings filing's ticker; null for briefs. */
  ticker: string | null;
}

export interface PostView extends PostCover {
  slug: string;
  contentType: string;
  tradingDate: string;
  publishedAt: string | null;
  title: string;
  summary: string;
  bodyMd: string;
  status: "published" | "pending";
  sources: { url: string; title: string }[];
  /** The other language's translation is published. */
  hasOtherLang: boolean;
  /** R2 key of the post's facts JSON (`FACTS` bucket), if the generator stored one. */
  factsKey: string | null;
  /** When the premarket update rewrote the post (UTC ISO), or null; `publishedAt` stays the first publish. */
  revisedAt: string | null;
}

/** A post's heading data: a `PostView` without the body and its sources. */
export type PostHead = Omit<PostView, "bodyMd" | "sources">;

export interface PostListItem extends PostCover {
  slug: string;
  contentType: string;
  tradingDate: string;
  publishedAt: string | null;
  title: string;
  summary: string;
}

export const PAGE_SIZE = 20;

/** The briefs published just before and just after one, in its language. */
export interface AdjacentPosts {
  prev: PostListItem | null;
  next: PostListItem | null;
}

const other = (lang: Lang): Lang => (lang === "en" ? "zh" : "en");

interface Row extends CoverRow {
  id: number;
  slug: string;
  content_type: string;
  trading_date: string;
  published_at: string | null;
  status: string;
  title: string | null;
  summary: string | null;
  body_md?: string | null;
  other_status: string | null;
  facts_r2_key: string | null;
  revised_at: string | null;
}

/** Columns every post SELECT adds; `f` is the LEFT JOINed `earnings_filings`. */
export const COVER_COLUMNS = `p.cover_key, t.cover_line1, t.cover_line2, f.ticker`;
export const JOIN_FILING = `LEFT JOIN earnings_filings f ON f.run_key = p.run_key`;

export interface CoverRow {
  cover_key: string | null;
  cover_line1: string | null;
  cover_line2: string | null;
  ticker: string | null;
}

/**
 * A row's cover fields. The key is kept only when it is one the covers route serves (`COVER_KEY`), so
 * the page never links an image that would 404; a missing first line (null or absent) means no lines.
 */
export function toCover(r: CoverRow): PostCover {
  return {
    coverKey: r.cover_key != null && COVER_KEY.test(r.cover_key) ? r.cover_key : null,
    coverLines: r.cover_line1 == null ? null : [r.cover_line1, r.cover_line2 ?? ""],
    ticker: r.ticker,
  };
}

/**
 * A post is shown in `lang` when that translation is published, or (ZH only) pending while the EN text is
 * published — the page then links to English. SQL over the aliases `t` (the `lang` translation) and `o` (the other
 * language's); since only the ZH case reads `o`, `o` may also be the EN row itself.
 */
export const VISIBLE = `(t.status = 'published' OR (t.lang = 'zh' AND t.status = 'pending' AND o.status = 'published'))`;

const HEAD_COLUMNS = `p.id, p.slug, p.content_type, p.trading_date, p.published_at, p.revised_at, p.facts_r2_key, t.status,
  t.title, t.summary, o.status AS other_status, ${COVER_COLUMNS}`;
const FROM_VIEW = `FROM posts p
JOIN post_translations t ON t.post_id = p.id AND t.lang = ?
LEFT JOIN post_translations o ON o.post_id = p.id AND o.lang = ?
${JOIN_FILING}`;
const SELECT_VIEW = `SELECT ${HEAD_COLUMNS}, t.body_md ${FROM_VIEW}`;

interface ListRow extends CoverRow {
  slug: string;
  content_type: string;
  trading_date: string;
  published_at: string | null;
  title: string | null;
  summary: string | null;
}

function toListItem(r: ListRow): PostListItem {
  return {
    slug: r.slug,
    contentType: r.content_type,
    tradingDate: r.trading_date,
    publishedAt: r.published_at,
    title: r.title ?? "",
    summary: r.summary ?? "",
    ...toCover(r),
  };
}

function toHead(row: Row): PostHead {
  return {
    slug: row.slug,
    contentType: row.content_type,
    tradingDate: row.trading_date,
    publishedAt: row.published_at,
    title: row.title ?? "",
    summary: row.summary ?? "",
    status: row.status === "pending" ? "pending" : "published",
    hasOtherLang: row.other_status === "published",
    factsKey: row.facts_r2_key,
    revisedAt: row.revised_at,
    ...toCover(row),
  };
}

async function toView(db: D1Like, row: Row | null): Promise<PostView | null> {
  if (!row) return null;
  const { results } = await db
    .prepare("SELECT url, title FROM sources WHERE post_id = ? ORDER BY rowid")
    .bind(row.id)
    .all<{ url: string; title: string | null }>();
  return {
    ...toHead(row),
    bodyMd: row.body_md ?? "",
    sources: results.map((s) => ({ url: s.url, title: s.title || s.url })),
  };
}

export async function getPostBySlug(db: D1Like, slug: string, lang: Lang): Promise<PostView | null> {
  const row = await db
    .prepare(`${SELECT_VIEW} WHERE p.slug = ? AND ${VISIBLE}`)
    .bind(lang, other(lang), slug)
    .first<Row>();
  return toView(db, row);
}

/**
 * The newest visible post of `type` in `lang`, without its body (one D1 read). `INDEXED BY`, CROSS JOIN (posts
 * outer) and the always-true range term make SQLite walk idx_posts_trading_date backwards and stop at the first
 * match, whatever its table statistics say, instead of reading every translation in the language
 * (tests/queryplan.test.ts).
 */
export async function getLatestPost(db: D1Like, lang: Lang, type: ContentType): Promise<PostHead | null> {
  const row = await db
    .prepare(
      `SELECT ${HEAD_COLUMNS} FROM posts p INDEXED BY idx_posts_trading_date
       CROSS JOIN post_translations t ON t.post_id = p.id AND t.lang = ?
       LEFT JOIN post_translations o ON o.post_id = p.id AND o.lang = ?
       ${JOIN_FILING}
       WHERE p.trading_date <= '9999-12-31' AND p.content_type = ? AND ${VISIBLE}
       ORDER BY p.trading_date DESC, p.published_at DESC LIMIT 1`,
    )
    .bind(lang, other(lang), type)
    .first<Row>();
  return row && toHead(row);
}

export function getLatestPremarket(db: D1Like, lang: Lang): Promise<PostHead | null> {
  return getLatestPost(db, lang, "premarket");
}

/**
 * Published posts in `lang`, newest first, of one `type`, or without one the daily briefs (premarket
 * and recap) and the Friday earnings week posts (the lead's call, 2026-11-24); `page` starts at 1.
 */
export async function listPosts(
  db: D1Like,
  lang: Lang,
  page: number,
  type: ContentType | null = null,
): Promise<PostListItem[]> {
  const p = Number.isFinite(page) && page >= 1 ? Math.floor(page) : 1;
  const { results } = await db
    .prepare(
      // CROSS JOIN keeps posts as the outer table, so SQLite walks idx_posts_trading_date newest first
      // and stops at the page, instead of reading every published translation in the language and sorting.
      `SELECT p.slug, p.content_type, p.trading_date, p.published_at, t.title, t.summary, ${COVER_COLUMNS}
       FROM posts p CROSS JOIN post_translations t ON t.post_id = p.id AND t.lang = ?
       ${JOIN_FILING}
       WHERE t.status = 'published' AND p.content_type IN (?, ?, ?)
       ORDER BY p.trading_date DESC, p.published_at DESC, p.id DESC
       LIMIT ? OFFSET ?`,
    )
    .bind(lang, type ?? "premarket", type ?? "recap", type ?? "week", PAGE_SIZE, (p - 1) * PAGE_SIZE)
    .all<ListRow>();
  return results.map(toListItem);
}

/**
 * The published premarket or recap briefs in `lang` right before and after `at`, in archive order
 * (`trading_date`, then `published_at`, then slug), in one D1 read. Earnings checks and unpublished
 * translations are skipped. Each side walks `idx_posts_trading_date` from `at` and stops at the first
 * match, so the rows read do not grow with history; a re-run brief keeps its trading date's place.
 */
export async function getAdjacentPosts(
  db: D1Like,
  lang: Lang,
  at: { tradingDate: string; publishedAt: string | null; slug: string },
): Promise<AdjacentPosts> {
  if (!at.publishedAt) return { prev: null, next: null };
  // CROSS JOIN keeps posts as the outer table, so SQLite starts from the date index.
  const side = (dir: "prev" | "next") => {
    const [cmp, order] = dir === "prev" ? ["<", "DESC"] : [">", "ASC"];
    return `SELECT * FROM (
       SELECT '${dir}' AS dir, p.slug, p.content_type, p.trading_date, p.published_at, t.title, t.summary, ${COVER_COLUMNS}
       FROM posts p CROSS JOIN post_translations t ON t.post_id = p.id AND t.lang = ?
       ${JOIN_FILING}
       WHERE t.status = 'published' AND p.content_type IN ('premarket', 'recap')
         AND p.trading_date ${cmp}= ? AND (p.trading_date, p.published_at, p.slug) ${cmp} (?, ?, ?)
       ORDER BY p.trading_date ${order}, p.published_at ${order}, p.slug ${order} LIMIT 1)`;
  };
  const key = [at.tradingDate, at.tradingDate, at.publishedAt, at.slug];
  const { results } = await db
    .prepare(`${side("prev")} UNION ALL ${side("next")}`)
    .bind(lang, ...key, lang, ...key)
    .all<ListRow & { dir: "prev" | "next" }>();
  const pick = (dir: "prev" | "next") => {
    const row = results.find((r) => r.dir === dir);
    return row ? toListItem(row) : null;
  };
  return { prev: pick("prev"), next: pick("next") };
}

/** Whether `slug` has a published translation in `lang` (one small D1 read). */
export async function isPublishedIn(db: D1Like, slug: string, lang: Lang): Promise<boolean> {
  const row = await db
    .prepare(
      `SELECT 1 AS hit FROM posts p JOIN post_translations t ON t.post_id = p.id AND t.lang = ?
       WHERE p.slug = ? AND t.status = 'published'`,
    )
    .bind(lang, slug)
    .first<{ hit: number }>();
  return row !== null;
}

/**
 * The post at `slug` in `lang`; or, when that translation is not visible but the other language's is
 * published (this one failed), a redirect to this language's home page, so the header's language
 * switch, which keeps the path, never lands on a 404. Null otherwise. The miss path costs one more read.
 */
export async function resolvePost(
  db: D1Like,
  slug: string,
  lang: Lang,
): Promise<{ post: PostView } | { redirectTo: string } | null> {
  const post = await getPostBySlug(db, slug, lang);
  if (post) return { post };
  return (await isPublishedIn(db, slug, other(lang))) ? { redirectTo: `/${lang}` } : null;
}

/** Read-only queries over the generator's D1 tables (migrations/0001_init.sql). */

export type Lang = "en" | "zh";

/** Content types the site lists and filters by; `posts.content_type` values. */
export const CONTENT_TYPES = ["premarket", "recap", "earnings"] as const;
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

export interface PostView {
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
}

/** A post's heading data: a `PostView` without the body and its sources. */
export type PostHead = Omit<PostView, "bodyMd" | "sources">;

export interface PostListItem {
  slug: string;
  contentType: string;
  tradingDate: string;
  publishedAt: string | null;
  title: string;
  summary: string;
}

export const PAGE_SIZE = 20;


const other = (lang: Lang): Lang => (lang === "en" ? "zh" : "en");

interface Row {
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
}

// A post is shown in `lang` when that translation is published, or (ZH only)
// pending while the EN text is published — the page then links to English.
const VISIBLE = `(t.status = 'published' OR (t.lang = 'zh' AND t.status = 'pending' AND o.status = 'published'))`;

const HEAD_COLUMNS = `p.id, p.slug, p.content_type, p.trading_date, p.published_at, p.facts_r2_key, t.status, t.title,
  t.summary, o.status AS other_status`;
const FROM_VIEW = `FROM posts p
JOIN post_translations t ON t.post_id = p.id AND t.lang = ?
LEFT JOIN post_translations o ON o.post_id = p.id AND o.lang = ?`;
const SELECT_VIEW = `SELECT ${HEAD_COLUMNS}, t.body_md ${FROM_VIEW}`;
const SELECT_HEAD = `SELECT ${HEAD_COLUMNS} ${FROM_VIEW}`;

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

/** The newest visible post of `type` in `lang`, without its body (one D1 read). */
export async function getLatestPost(db: D1Like, lang: Lang, type: ContentType): Promise<PostHead | null> {
  const row = await db
    .prepare(
      `${SELECT_HEAD} WHERE p.content_type = ? AND ${VISIBLE}
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
 * Published posts in `lang`, newest first, of one `type`, or the daily briefs (premarket and recap)
 * without one; `page` starts at 1.
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
      `SELECT p.slug, p.content_type, p.trading_date, p.published_at, t.title, t.summary
       FROM posts p JOIN post_translations t ON t.post_id = p.id AND t.lang = ?
       WHERE t.status = 'published' AND p.content_type IN (?, ?)
       ORDER BY p.trading_date DESC, p.published_at DESC, p.id DESC
       LIMIT ? OFFSET ?`,
    )
    .bind(lang, type ?? "premarket", type ?? "recap", PAGE_SIZE, (p - 1) * PAGE_SIZE)
    .all<{
      slug: string;
      content_type: string;
      trading_date: string;
      published_at: string | null;
      title: string | null;
      summary: string | null;
    }>();
  return results.map((r) => ({
    slug: r.slug,
    contentType: r.content_type,
    tradingDate: r.trading_date,
    publishedAt: r.published_at,
    title: r.title ?? "",
    summary: r.summary ?? "",
  }));
}

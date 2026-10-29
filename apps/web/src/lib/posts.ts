/** Read-only queries over the generator's D1 tables (migrations/0001_init.sql). */

export type Lang = "en" | "zh";

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
  tradingDate: string;
  publishedAt: string | null;
  title: string;
  summary: string;
  bodyMd: string;
  status: "published" | "pending";
  sources: { url: string; title: string }[];
  /** The other language's translation is published. */
  hasOtherLang: boolean;
}

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
  trading_date: string;
  published_at: string | null;
  status: string;
  title: string | null;
  summary: string | null;
  body_md: string | null;
  other_status: string | null;
}

// A post is shown in `lang` when that translation is published, or (ZH only)
// pending while the EN text is published — the page then links to English.
const VISIBLE = `(t.status = 'published' OR (t.lang = 'zh' AND t.status = 'pending' AND o.status = 'published'))`;

const SELECT_VIEW = `SELECT p.id, p.slug, p.trading_date, p.published_at, t.status, t.title, t.summary, t.body_md,
  o.status AS other_status
FROM posts p
JOIN post_translations t ON t.post_id = p.id AND t.lang = ?
LEFT JOIN post_translations o ON o.post_id = p.id AND o.lang = ?`;

async function toView(db: D1Like, row: Row | null): Promise<PostView | null> {
  if (!row) return null;
  const { results } = await db
    .prepare("SELECT url, title FROM sources WHERE post_id = ? ORDER BY rowid")
    .bind(row.id)
    .all<{ url: string; title: string | null }>();
  return {
    slug: row.slug,
    tradingDate: row.trading_date,
    publishedAt: row.published_at,
    title: row.title ?? "",
    summary: row.summary ?? "",
    bodyMd: row.body_md ?? "",
    status: row.status === "pending" ? "pending" : "published",
    sources: results.map((s) => ({ url: s.url, title: s.title || s.url })),
    hasOtherLang: row.other_status === "published",
  };
}

export async function getPostBySlug(db: D1Like, slug: string, lang: Lang): Promise<PostView | null> {
  const row = await db
    .prepare(`${SELECT_VIEW} WHERE p.slug = ? AND ${VISIBLE}`)
    .bind(lang, other(lang), slug)
    .first<Row>();
  return toView(db, row);
}

export async function getLatestPremarket(db: D1Like, lang: Lang): Promise<PostView | null> {
  const row = await db
    .prepare(
      `${SELECT_VIEW} WHERE p.content_type = 'premarket' AND ${VISIBLE}
       ORDER BY p.trading_date DESC, p.published_at DESC LIMIT 1`,
    )
    .bind(lang, other(lang))
    .first<Row>();
  return toView(db, row);
}

/** Published posts in `lang`, newest first; `page` starts at 1. */
export async function listPosts(db: D1Like, lang: Lang, page: number): Promise<PostListItem[]> {
  const p = Number.isFinite(page) && page >= 1 ? Math.floor(page) : 1;
  const { results } = await db
    .prepare(
      `SELECT p.slug, p.content_type, p.trading_date, p.published_at, t.title, t.summary
       FROM posts p JOIN post_translations t ON t.post_id = p.id AND t.lang = ?
       WHERE t.status = 'published'
       ORDER BY p.trading_date DESC, p.published_at DESC, p.id DESC
       LIMIT ? OFFSET ?`,
    )
    .bind(lang, PAGE_SIZE, (p - 1) * PAGE_SIZE)
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

/** A post page's data beyond the post itself: facts, neighbouring briefs and charts. */

import { getPostCharts, tickersWithPages, type PostChart } from "./earnings";
import { loadFacts, type FactsView, type R2Like } from "./facts";
import { getAdjacentPosts, type AdjacentPosts, type D1Like, type Lang, type PostView } from "./posts";

export interface BriefData {
  post: PostView;
  /** Premarket and recap facts; null (no facts, or an earnings check) renders the full markdown. */
  facts: FactsView | null;
  adjacent: AdjacentPosts;
  /** An earnings check's charts; empty for briefs and IPO write-ups. */
  charts: PostChart[];
  /** The movers whose ticker has a page on the site (their rows link to it); empty without movers. */
  tickerPages: string[];
}

/** The facts kind of a daily brief, or null for any other post. */
export function briefKind(contentType: string): FactsView["kind"] | null {
  return contentType === "premarket" || contentType === "recap" ? contentType : null;
}

/**
 * Everything a post page shows besides the post, in parallel: for a brief, one R2 read for its
 * facts and one D1 read for its neighbours; for a published earnings check, its charts; nothing for an IPO write-up.
 */
export async function loadBrief(
  db: D1Like,
  bucket: R2Like | undefined,
  post: PostView,
  lang: Lang,
  onError?: (error: unknown) => void,
): Promise<BriefData> {
  const kind = briefKind(post.contentType);
  const [facts, adjacent, charts] = await Promise.all([
    kind ? loadFacts(bucket, post.factsKey, kind, onError) : null,
    kind ? getAdjacentPosts(db, lang, post) : { prev: null, next: null },
    post.contentType === "earnings" && post.status === "published" ? getPostCharts(db, post.slug, lang) : [],
  ]);
  return { post, facts, adjacent, charts, tickerPages: await moverPages(db, facts, onError) };
}

/**
 * The movers in `facts` that have a ticker page, so the table can link them. One D1 read; a failed read only
 * costs the links.
 */
export async function moverPages(
  db: D1Like,
  facts: Pick<FactsView, "gainers" | "losers"> | null,
  onError?: (error: unknown) => void,
): Promise<string[]> {
  if (!facts) return [];
  const symbols = [...facts.gainers, ...facts.losers].map((m) => m.symbol);
  if (symbols.length === 0) return [];
  try {
    return [...(await tickersWithPages(db, symbols))];
  } catch (e) {
    onError?.(e);
    return [];
  }
}

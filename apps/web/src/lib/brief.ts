/** A post page's data beyond the post itself: facts, neighbouring briefs, the strip and charts. */

import { getPostCharts, stripDay, type PostChart, type StripDay } from "./earnings";
import { loadFacts, type FactsView, type R2Like } from "./facts";
import { getAdjacentPosts, type AdjacentPosts, type D1Like, type Lang, type PostView } from "./posts";

export interface BriefData {
  post: PostView;
  /** Premarket and recap facts; null (no facts, or an earnings check) renders the full markdown. */
  facts: FactsView | null;
  adjacent: AdjacentPosts;
  strip: StripDay | null;
  /** An earnings check's charts; empty for briefs. */
  charts: PostChart[];
}

/** The facts kind of a daily brief, or null for any other post. */
export function briefKind(contentType: string): FactsView["kind"] | null {
  return contentType === "premarket" || contentType === "recap" ? contentType : null;
}

/**
 * Everything a post page shows besides the post, in parallel: for a brief, one R2 read for its
 * facts and one D1 read for its neighbours; for a published earnings check, its charts; and the
 * earnings strip for the post's own trading date.
 */
export async function loadBrief(
  db: D1Like,
  bucket: R2Like | undefined,
  post: PostView,
  lang: Lang,
  onError?: (error: unknown) => void,
): Promise<BriefData> {
  const kind = briefKind(post.contentType);
  const [facts, adjacent, strip, charts] = await Promise.all([
    kind ? loadFacts(bucket, post.factsKey, kind, onError) : null,
    kind ? getAdjacentPosts(db, lang, post) : { prev: null, next: null },
    stripDay(db, post.tradingDate, { exact: true, lang }),
    !kind && post.status === "published" ? getPostCharts(db, post.slug, lang) : [],
  ]);
  return { post, facts, adjacent, strip, charts };
}

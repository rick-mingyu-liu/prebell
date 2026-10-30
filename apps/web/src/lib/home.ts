/** The homepage's data: which brief tab is shown, its post and facts, and the side lists. */

import { listLatestEarnings, listRecentBriefs, stripDay, type StripDay } from "./earnings";
import { loadFacts, type FactsView, type R2Like } from "./facts";
import { getLatestPost, type D1Like, type Lang, type PostHead, type PostListItem } from "./posts";

export type BriefTab = "premarket" | "recap";

/**
 * The brief tab to show: the requested one when it has a post, else the more recently
 * published of the two (premarket on a tie). Null when neither exists.
 */
export function selectTab(
  premarket: PostHead | null,
  recap: PostHead | null,
  requested: string | string[] | undefined,
): BriefTab | null {
  const want = Array.isArray(requested) ? requested[0] : requested;
  if (want === "premarket" && premarket) return "premarket";
  if (want === "recap" && recap) return "recap";
  if (!premarket || !recap) return premarket ? "premarket" : recap ? "recap" : null;
  return (recap.publishedAt ?? "") > (premarket.publishedAt ?? "") ? "recap" : "premarket";
}

export interface HomeData {
  /** The selected brief tab and its post; null when nothing is published. */
  tab: BriefTab | null;
  /** Tabs that have a post, in display order. */
  tabs: BriefTab[];
  post: PostHead | null;
  /** The selected post's facts; null falls back to title and summary only. */
  facts: FactsView | null;
  earnings: PostListItem[];
  /** Recent briefs other than the two behind the tabs. */
  recent: PostListItem[];
  strip: StripDay | null;
}

/**
 * Everything the homepage shows, in six small D1 reads and at most one R2 read: the facts of
 * the selected tab's post only. `today` is the ET date the earnings strip starts from.
 */
export async function loadHome(
  db: D1Like,
  bucket: R2Like | undefined,
  lang: Lang,
  requested: string | string[] | undefined,
  today: string,
  onError?: (error: unknown, key: string | null) => void,
): Promise<HomeData> {
  // Briefs and earnings checks are listed separately, so a busy earnings
  // season never pushes the briefs off the home page.
  const [premarket, recap, recent, earnings, strip] = await Promise.all([
    getLatestPost(db, lang, "premarket"),
    getLatestPost(db, lang, "recap"),
    listRecentBriefs(db, lang, 12),
    listLatestEarnings(db, lang, 3),
    stripDay(db, today, { exact: false, lang }),
  ]);
  const tab = selectTab(premarket, recap, requested);
  const post = tab === "recap" ? recap : tab === "premarket" ? premarket : null;
  const facts =
    tab && post ? await loadFacts(bucket, post.factsKey, tab, onError && ((e) => onError(e, post.factsKey))) : null;
  const shown = new Set([premarket?.slug, recap?.slug]);
  return {
    tab,
    tabs: (["premarket", "recap"] as const).filter((k) => (k === "premarket" ? premarket : recap)),
    post,
    facts,
    earnings,
    recent: recent.filter((p) => !shown.has(p.slug)).slice(0, 10),
    strip,
  };
}

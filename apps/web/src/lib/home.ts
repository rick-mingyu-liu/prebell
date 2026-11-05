/** The front page's data: the lead brief and its facts, the earnings checks, recent posts and who reports next. */

import { addDays, listLatestEarnings, listRecentBriefs, stripDay, type StripDay } from "./earnings";
import { loadFacts, type FactsView, type R2Like } from "./facts";
import { getLatestPost, type D1Like, type Lang, type PostHead, type PostListItem } from "./posts";

/** Earnings checks shown as covers; later ones fall into the recent list. */
export const EARNINGS_SHOWN = 3;
/** Rows in the recent list. */
export const RECENT_SHOWN = 8;

export interface HomeData {
  /** The later-published of the latest premarket and recap; null when neither exists. */
  lead: PostHead | null;
  /** The lead's facts; null when it has none or they cannot be read. */
  leadFacts: FactsView | null;
  /** The newest earnings checks, at most `EARNINGS_SHOWN`. */
  earnings: PostListItem[];
  /** Other briefs and older checks, newest published first, at most `RECENT_SHOWN`. */
  recent: PostListItem[];
  /** The next calendar day with reports, from today for a premarket lead and from tomorrow otherwise. */
  reporting: StripDay | null;
}

/** The more recently published of the two briefs; the premarket on a tie. */
function pickLead(premarket: PostHead | null, recap: PostHead | null): PostHead | null {
  if (!premarket || !recap) return premarket ?? recap;
  return (recap.publishedAt ?? "") > (premarket.publishedAt ?? "") ? recap : premarket;
}

const newestFirst = (a: PostListItem, b: PostListItem) => (b.publishedAt ?? "").localeCompare(a.publishedAt ?? "");

/**
 * Everything the front page shows, in six small D1 reads and at most one R2 read: the lead's facts.
 * `today` is the ET date the reporting block counts from; `onError` hears a facts read that failed.
 */
export async function loadHome(
  db: D1Like,
  bucket: R2Like | undefined,
  lang: Lang,
  today: string,
  onError?: (error: unknown, key: string | null) => void,
): Promise<HomeData> {
  // Briefs and earnings checks are listed separately, so a busy earnings
  // season never pushes the briefs off the front page.
  const [premarket, recap, briefs, checks] = await Promise.all([
    getLatestPost(db, lang, "premarket"),
    getLatestPost(db, lang, "recap"),
    listRecentBriefs(db, lang, RECENT_SHOWN),
    listLatestEarnings(db, lang, EARNINGS_SHOWN + RECENT_SHOWN),
  ]);
  const lead = pickLead(premarket, recap);
  const kind = lead?.contentType === "recap" ? "recap" : "premarket";
  const [leadFacts, reporting] = await Promise.all([
    lead ? loadFacts(bucket, lead.factsKey, kind, onError && ((e) => onError(e, lead.factsKey))) : null,
    stripDay(db, lead?.contentType === "premarket" ? today : addDays(today, 1), { exact: false, lang }),
  ]);
  const recent = [...briefs.filter((p) => p.slug !== lead?.slug), ...checks.slice(EARNINGS_SHOWN)]
    .sort(newestFirst)
    .slice(0, RECENT_SHOWN);
  return { lead, leadFacts, earnings: checks.slice(0, EARNINGS_SHOWN), recent, reporting };
}

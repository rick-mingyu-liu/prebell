/** The front page's data: the lead brief and its facts, the earnings checks, recent posts and who reports next. */

import { addDays, listLatestEarnings, listRecentBriefs, stripDay, type StripDay } from "./earnings";
import { moverPages } from "./brief";
import { loadFacts, type FactsView, type R2Like } from "./facts";
import { listHomeIpos, type HomeIpo } from "./ipo";
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
  /** The next calendar day with reports, from today for a premarket lead and from the day after a recap's date. */
  reporting: StripDay | null;
  /** Active tracked IPOs (not withdrawn, at most 30 days past listing); empty without any or before migration 0009. */
  ipos: HomeIpo[];
  /** The lead's movers whose ticker has a page on the site (their rows link to it). */
  tickerPages: string[];
}

/** The more recently published of the two briefs; the premarket on a tie. */
function pickLead(premarket: PostHead | null, recap: PostHead | null): PostHead | null {
  if (!premarket || !recap) return premarket ?? recap;
  return (recap.publishedAt ?? "") > (premarket.publishedAt ?? "") ? recap : premarket;
}

/**
 * The first day the reporting block may show: today under a premarket lead, else the day after the lead's
 * trading date. That is today while yesterday's recap still leads overnight, and never a day before today.
 */
function reportingFrom(lead: PostHead | null, today: string): string {
  if (!lead || lead.contentType === "premarket") return today;
  const next = addDays(lead.tradingDate, 1);
  return next > today ? next : today;
}

const newestFirst = (a: PostListItem, b: PostListItem) => (b.publishedAt ?? "").localeCompare(a.publishedAt ?? "");

/**
 * Everything the front page shows, in seven small D1 reads (six, plus one for the IPO section) and at most one R2
 * read: the lead's facts.
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
  const [premarket, recap, briefs, checks, ipos] = await Promise.all([
    getLatestPost(db, lang, "premarket"),
    getLatestPost(db, lang, "recap"),
    // One extra: the list may include the lead, which recent leaves out.
    listRecentBriefs(db, lang, RECENT_SHOWN + 1),
    listLatestEarnings(db, lang, EARNINGS_SHOWN + RECENT_SHOWN),
    listHomeIpos(db, lang, today),
  ]);
  const lead = pickLead(premarket, recap);
  const kind = lead?.contentType === "recap" ? "recap" : "premarket";
  const [leadFacts, reporting] = await Promise.all([
    lead ? loadFacts(bucket, lead.factsKey, kind, onError && ((e) => onError(e, lead.factsKey))) : null,
    stripDay(db, reportingFrom(lead, today), { exact: false, lang }),
  ]);
  const recent = [...briefs.filter((p) => p.slug !== lead?.slug), ...checks.slice(EARNINGS_SHOWN)]
    .sort(newestFirst)
    .slice(0, RECENT_SHOWN);
  const tickerPages = await moverPages(db, leadFacts, onError && ((e) => onError(e, "ticker pages")));
  return { lead, leadFacts, earnings: checks.slice(0, EARNINGS_SHOWN), recent, reporting, ipos, tickerPages };
}

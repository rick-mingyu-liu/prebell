import Link from "next/link";
import type { ReactNode } from "react";
import EarningsStrip from "./EarningsStrip";
import MoversList from "./MoversList";
import NumbersBlock from "./NumbersBlock";
import PendingNotice from "./PendingNotice";
import Sidebar from "./Sidebar";
import type { Locale } from "../i18n/routing";
import type { StripDay } from "../lib/earnings";
import type { FactsView } from "../lib/facts";
import type { PostListItem, PostView } from "../lib/posts";
import { formatDate } from "../lib/time";
import { translator } from "../lib/translator";

export type BriefTab = "premarket" | "recap";

/** Movers shown per side on the homepage (the brief page shows them all). */
const MOVERS_PER_SIDE = 3;

/**
 * The brief tab to show: the requested one when it has a post, else the more recently
 * published of the two (premarket on a tie). Null when neither exists.
 */
export function selectTab(
  premarket: PostView | null,
  recap: PostView | null,
  requested: string | string[] | undefined,
): BriefTab | null {
  const want = Array.isArray(requested) ? requested[0] : requested;
  if (want === "premarket" && premarket) return "premarket";
  if (want === "recap" && recap) return "recap";
  if (!premarket || !recap) return premarket ? "premarket" : recap ? "recap" : null;
  return (recap.publishedAt ?? "") > (premarket.publishedAt ?? "") ? "recap" : "premarket";
}

export interface HomeViewProps {
  locale: Locale;
  /** The selected brief tab and its post; null when nothing is published. */
  tab: BriefTab | null;
  /** Tabs that have a post, in display order. */
  tabs: BriefTab[];
  post: PostView | null;
  /** The selected post's facts; null falls back to title and summary only. */
  facts: FactsView | null;
  earnings: PostListItem[];
  strip: StripDay | null;
  /** Today's ET date, for the strip's label. */
  today: string;
  /** Recent posts, placed under the two main columns. */
  children?: ReactNode;
}

/**
 * The homepage: earnings strip, then the portal grid of the brief block, the movers and
 * earnings-check panels, recent posts and the sidebar. The brief tabs are links (`?tab=`);
 * the middle panels switch with radio inputs and CSS, so the page needs no client JS.
 */
export default function HomeView({ locale, tab, tabs, post, facts, earnings, strip, today, children }: HomeViewProps) {
  const t = translator(locale);
  const tabLabel = (k: BriefTab) => (k === "premarket" ? t("types.premarket") : t("types.recap"));
  const movers = facts ? [...facts.gainers.slice(0, MOVERS_PER_SIDE), ...facts.losers.slice(0, MOVERS_PER_SIDE)] : [];

  return (
    <>
      <EarningsStrip day={strip} locale={locale} today={today} />
      <div className="grid-portal home">
        <section className="home-brief">
          {tab && post ? (
            <>
              <nav className="tabs" aria-label={t("home.briefTabs")}>
                {tabs.map((k) => (
                  <Link key={k} href={`/${locale}?tab=${k}`} aria-current={k === tab ? "page" : undefined}>
                    {tabLabel(k)}
                  </Link>
                ))}
              </nav>
              {facts && <NumbersBlock facts={facts} variant="home" locale={locale} />}
              <p className="meta">
                <time dateTime={post.tradingDate}>{formatDate(post.tradingDate, locale)}</time>
              </p>
              {post.status === "pending" ? (
                <>
                  <h1 className="home-title">
                    {t("post.pendingTitle", { type: tabLabel(tab), date: formatDate(post.tradingDate, locale) })}
                  </h1>
                  <PendingNotice slug={post.slug} locale={locale} />
                </>
              ) : (
                <>
                  <h1 className="home-title">
                    <Link href={`/${locale}/p/${post.slug}`}>{post.title || post.slug}</Link>
                  </h1>
                  {post.summary && <p className="home-sum">{post.summary}</p>}
                  <Link className="more" href={`/${locale}/p/${post.slug}`}>
                    {t("home.readFull")}
                  </Link>
                </>
              )}
            </>
          ) : (
            <p>{t("home.empty")}</p>
          )}
        </section>

        <section className="home-panels" aria-label={t("home.panelTabs")}>
          <input type="radio" name="home-panel" id="home-panel-movers" className="ptab ptab-movers" defaultChecked />
          <input type="radio" name="home-panel" id="home-panel-earnings" className="ptab ptab-earnings" />
          <div className="tabs">
            <label htmlFor="home-panel-movers">{t("home.tabMovers")}</label>
            <label htmlFor="home-panel-earnings">{t("home.tabEarnings")}</label>
          </div>
          <div className="panel panel-movers">
            {movers.length > 0 ? (
              <MoversList movers={movers} locale={locale} />
            ) : (
              <p className="muted">{t("home.noMovers")}</p>
            )}
          </div>
          <div className="panel panel-earnings">
            {earnings.length > 0 ? (
              earnings.map((e) => (
                <article key={e.slug} className="ecard">
                  <p className="meta">
                    <time dateTime={e.tradingDate}>{formatDate(e.tradingDate, locale)}</time>
                  </p>
                  <Link className="ecard-title" href={`/${locale}/p/${e.slug}`}>
                    {e.title || e.slug}
                  </Link>
                  {e.summary && <p>{e.summary}</p>}
                </article>
              ))
            ) : (
              <p className="muted">{t("home.noEarnings")}</p>
            )}
            <Link className="more" href={`/${locale}/archive?type=earnings`}>
              {t("home.allEarnings")}
            </Link>
          </div>
        </section>

        {children}
        <Sidebar locale={locale} />
      </div>
    </>
  );
}

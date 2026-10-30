import Link from "next/link";
import type { ReactNode } from "react";
import EarningsStrip from "./EarningsStrip";
import MoversList from "./MoversList";
import NumbersBlock from "./NumbersBlock";
import PendingNotice from "./PendingNotice";
import Sidebar from "./Sidebar";
import type { Locale } from "../i18n/routing";
import type { BriefTab, HomeData } from "../lib/home";
import { pendingTitle } from "../lib/labels";
import { formatDate } from "../lib/time";
import { translator } from "../lib/translator";

/** Movers shown per side on the homepage (the brief page shows them all). */
const MOVERS_PER_SIDE = 3;

export type HomeViewProps = Omit<HomeData, "recent"> & {
  locale: Locale;
  /** Today's ET date, for the strip's label. */
  today: string;
  /** Recent posts, placed under the two main columns. */
  children?: ReactNode;
};

/**
 * The homepage: earnings strip, then the portal grid of the brief block, the movers and
 * earnings-check panels, recent posts and the sidebar. The brief tabs are links (`?tab=`).
 * From 1024px the two panels switch with radio inputs and CSS; below it they stack and the
 * radios are hidden. The page needs no client JS.
 */
export default function HomeView({ locale, tab, tabs, post, facts, earnings, strip, today, children }: HomeViewProps) {
  const t = translator(locale);
  const tabLabel = (k: BriefTab) => (k === "premarket" ? t("types.premarket") : t("types.recap"));
  const movers = facts ? [...facts.gainers.slice(0, MOVERS_PER_SIDE), ...facts.losers.slice(0, MOVERS_PER_SIDE)] : [];
  const postHref = post ? `/${locale}/p/${post.slug}` : null;

  return (
    <>
      <EarningsStrip day={strip} locale={locale} today={today} />
      <div className="grid-portal home">
        <section className="home-brief">
          {tab && post && postHref ? (
            <>
              <nav className="tabs" aria-label={t("home.briefTabs")}>
                {tabs.map((k) => (
                  <Link
                    key={k}
                    href={`/${locale}?tab=${k}`}
                    aria-current={k === tab ? "page" : undefined}
                    scroll={false}
                    prefetch={false}
                  >
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
                  <h1 className="home-title">{pendingTitle(t, post, locale)}</h1>
                  <PendingNotice slug={post.slug} locale={locale} />
                </>
              ) : (
                <>
                  <h1 className="home-title">
                    <Link href={postHref}>{post.title || post.slug}</Link>
                  </h1>
                  {post.summary && <p className="home-sum">{post.summary}</p>}
                  <Link className="more" href={postHref}>
                    {t("home.readFull")}
                  </Link>
                </>
              )}
            </>
          ) : (
            <p>{t("home.empty")}</p>
          )}
        </section>

        {/* Keyed by tab so a brief-tab navigation resets the panels to movers. */}
        <section key={tab ?? ""} className="home-panels">
          <div className="tabs" role="radiogroup" aria-label={t("home.panelTabs")}>
            <input type="radio" name="home-panel" id="home-panel-movers" className="ptab" defaultChecked />
            <label htmlFor="home-panel-movers">{t("home.tabMovers")}</label>
            <input type="radio" name="home-panel" id="home-panel-earnings" className="ptab" />
            <label htmlFor="home-panel-earnings">{t("home.tabEarnings")}</label>
          </div>
          <div className="panel panel-movers">
            <h2 className="panel-title">{t("home.tabMovers")}</h2>
            {movers.length > 0 ? (
              <MoversList movers={movers} locale={locale} />
            ) : facts ? (
              <p className="muted">{t("home.noMovers")}</p>
            ) : (
              postHref && (
                <Link className="more" href={postHref}>
                  {t("home.moversInBrief")}
                </Link>
              )
            )}
          </div>
          <div className="panel panel-earnings">
            <h2 className="panel-title">{t("home.tabEarnings")}</h2>
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

import Link from "next/link";
import Charts from "./Charts";
import EarningsStrip from "./EarningsStrip";
import MoversList from "./MoversList";
import NumbersBlock from "./NumbersBlock";
import PendingNotice from "./PendingNotice";
import PostBody from "./PostBody";
import Sidebar from "./Sidebar";
import type { Locale } from "../i18n/routing";
import { briefKind, type BriefData } from "../lib/brief";
import { pendingTitle, typeLabel } from "../lib/labels";
import type { PostListItem } from "../lib/posts";
import { formatDate, formatDayShort, formatEt } from "../lib/time";
import { translator } from "../lib/translator";
import { trimStory } from "../lib/trim";

export type BriefViewProps = BriefData & {
  locale: Locale;
  /** Today's ET date, for the strip's label. */
  today: string;
};

/**
 * A post page. For a premarket or recap brief: breadcrumb, ET publish time and the checked badge,
 * then the numbers, every mover, and the story (title, summary, and the body without what is shown
 * above it), then the neighbouring briefs. Without facts the full body renders as before. An
 * earnings check keeps its body and charts. All sit between the earnings strip and the sidebar.
 */
export default function BriefView({ locale, today, post, facts, adjacent, strip, charts }: BriefViewProps) {
  const t = translator(locale);
  const kind = briefKind(post.contentType);
  const pending = post.status === "pending";
  const sources = post.sources.map((s) => s.url);
  const neighbour = (p: PostListItem) => `${typeLabel(t, p.contentType)} · ${formatDayShort(p.tradingDate, locale)}`;

  return (
    <>
      <EarningsStrip day={strip} locale={locale} today={today} />
      <div className="grid-article">
        <article className="brief-main">
          <nav className="crumb" aria-label={t("brief.crumbLabel")}>
            {kind ? (
              <>
                <Link href={`/${locale}/archive`}>{t("site.briefs")}</Link>
                {" › "}
                <Link href={`/${locale}/archive?type=${kind}`}>{t(`brief.${kind}`)}</Link>
              </>
            ) : (
              <Link href={`/${locale}/archive?type=earnings`}>{t("site.earnings")}</Link>
            )}
            {" › "}
            <time dateTime={post.tradingDate}>{formatDate(post.tradingDate, locale)}</time>
          </nav>
          <p className="meta brief-meta">
            {/* A pending translation is not published yet; its publish time is the English one. */}
            {!pending && (
              <time dateTime={post.publishedAt ?? undefined}>
                {t("post.published", { time: formatEt(post.publishedAt, locale) })}
              </time>
            )}
            {kind && <span className="checked">{t("brief.checked")}</span>}
          </p>

          {facts && <NumbersBlock facts={facts} variant="page" locale={locale} />}
          {facts && facts.gainers.length + facts.losers.length > 0 && (
            <section className="brief-movers" aria-labelledby="movers">
              <h2 id="movers" className="lbl">
                {t("brief.movers")}
              </h2>
              <MoversList movers={[...facts.gainers, ...facts.losers]} locale={locale} showPrice />
            </section>
          )}

          {pending ? (
            <>
              <h1>{pendingTitle(t, post, locale)}</h1>
              <PendingNotice slug={post.slug} locale={locale} />
            </>
          ) : facts ? (
            <>
              <h1 className="brief-title">{post.title}</h1>
              {post.summary && <p className="brief-sum">{post.summary}</p>}
              <PostBody markdown={trimStory(post.bodyMd)} sources={sources} colorSigned />
            </>
          ) : (
            <PostBody markdown={post.bodyMd} sources={sources} />
          )}
          {!pending && <Charts charts={charts} locale={locale} />}

          {(adjacent.prev || adjacent.next) && (
            <nav className="adjacent" aria-label={t("brief.adjacent")}>
              {adjacent.prev && (
                <Link rel="prev" href={`/${locale}/p/${adjacent.prev.slug}`}>
                  <small>← {t("brief.prev")}</small>
                  {neighbour(adjacent.prev)}
                </Link>
              )}
              {adjacent.next && (
                <Link className="next" rel="next" href={`/${locale}/p/${adjacent.next.slug}`}>
                  <small>{t("brief.next")} →</small>
                  {neighbour(adjacent.next)}
                </Link>
              )}
            </nav>
          )}
        </article>
        <Sidebar locale={locale} />
      </div>
    </>
  );
}

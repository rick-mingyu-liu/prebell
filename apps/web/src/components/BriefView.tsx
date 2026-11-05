import Link from "next/link";
import AuthorBlock from "./AuthorBlock";
import Charts from "./Charts";
import Cover, { type CoverProps } from "./Cover";
import MoversTable from "./MoversTable";
import PartnersStrip from "./PartnersStrip";
import PendingNotice from "./PendingNotice";
import PostBody from "./PostBody";
import SignupForm from "./SignupForm";
import type { Locale } from "../i18n/routing";
import { briefKind, type BriefData } from "../lib/brief";
import { pendingTitle, typeLabel } from "../lib/labels";
import type { PostListItem } from "../lib/posts";
import { formatDate, formatDayShort, formatEt, formatMonthDay } from "../lib/time";
import { translator } from "../lib/translator";
import { splitTail, trimHead, trimStory } from "../lib/trim";

export interface BriefViewProps {
  data: BriefData;
  locale: Locale;
  /** Whether to show the weekly-digest signup: only once the Buttondown key is set. */
  signup: boolean;
}

/**
 * A post page as one reading column: kicker, serif headline, publish time with the checked tag, cover,
 * then the body. A brief with facts leads with its summary and the movers table, and its body loses
 * the tables the facts replace; without facts, and for an earnings check, the body keeps its tables
 * (the key-figure table exactly as written) and an earnings check adds its charts. Then the sources,
 * the post's own disclaimer, the author, the signup (when enabled), a brief's neighbours and the partner strip.
 */
export default function BriefView({ data, locale, signup }: BriefViewProps) {
  const { post, facts, adjacent, charts } = data;
  const t = translator(locale);
  const kind = briefKind(post.contentType);
  const pending = post.status === "pending";
  const title = pending ? pendingTitle(t, post, locale) : post.title;
  const sources = post.sources.map((s) => s.url);
  const movers = facts ? [...facts.gainers, ...facts.losers] : [];
  // The headline and lede show the title and summary, so the body starts after them.
  const story = pending ? null : splitTail(facts ? trimStory(post.bodyMd, post.summary) : trimHead(post.bodyMd, post.summary));
  const tag: CoverProps["tag"] = kind
    ? { kind: "label", text: `${formatMonthDay(post.tradingDate, locale)} · ${t(`paper.edition.${kind}`)}` }
    : post.ticker
      ? { kind: "logo", ticker: post.ticker }
      : { kind: "label", text: typeLabel(t, post.contentType) };
  const neighbour = (p: PostListItem) => `${typeLabel(t, p.contentType)} · ${formatDayShort(p.tradingDate, locale)}`;

  return (
    <article className="story">
      <p className="kicker story-kicker">
        {kind ? (
          <>
            {typeLabel(t, kind)} · <time dateTime={post.tradingDate}>{formatDate(post.tradingDate, locale)}</time>
          </>
        ) : post.ticker ? (
          t("brief.kickerEarnings", { ticker: post.ticker })
        ) : (
          typeLabel(t, post.contentType)
        )}
      </p>
      <h1 className="story-title">{title}</h1>
      {/* A pending translation is not published yet; its publish time is the English one. */}
      {(!pending || kind) && (
        <p className="story-meta">
          {!pending && (
            <time dateTime={post.publishedAt ?? undefined}>
              {t("post.published", { time: formatEt(post.publishedAt, locale) })}
            </time>
          )}
          <span className="checked">{kind ? t("brief.checked") : t("earnings.checkedFiling")}</span>
        </p>
      )}
      {/* A pending translation may carry stale lines; show the scene and the tag only. */}
      <Cover coverKey={post.coverKey} lines={pending ? null : post.coverLines} tag={tag} size="article" alt={title} />

      <div className="story-body">
        {story && post.summary && <p className="story-lede">{post.summary}</p>}
        {movers.length > 0 && (
          <section className="story-movers" aria-labelledby="movers">
            <h2 id="movers">{t("brief.movers")}</h2>
            <MoversTable
              movers={movers}
              locale={locale}
              variant="table"
              showPrice
              priceLabel={facts?.kind === "recap" ? "close" : "last"}
            />
          </section>
        )}
        {!story ? (
          <PendingNotice slug={post.slug} locale={locale} />
        ) : (
          story.body && <PostBody markdown={story.body} sources={sources} colorSigned={facts !== null} />
        )}
        {!pending && <Charts charts={charts} locale={locale} />}
      </div>

      {story?.sources && (
        <section className="sources" aria-labelledby="sources">
          <h2 id="sources">{t("post.sources")}</h2>
          <PostBody markdown={story.sources} sources={sources} />
        </section>
      )}
      {story?.disclaimer && <p className="story-disclaimer">{story.disclaimer}</p>}

      <AuthorBlock locale={locale} />
      {signup && <SignupForm source="post" locale={locale} />}
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
      <PartnersStrip locale={locale} />
    </article>
  );
}

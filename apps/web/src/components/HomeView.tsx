import Link from "next/link";
import Cover from "./Cover";
import { DateLine } from "./IpoView";
import MoversTable from "./MoversTable";
import PartnersStrip from "./PartnersStrip";
import PendingNotice from "./PendingNotice";
import SignupForm from "./SignupForm";
import type { Locale } from "../i18n/routing";
import type { CalendarEntry } from "../lib/earnings";
import { dirOf, fmtPct, fmtPrice } from "../lib/format";
import type { HomeData } from "../lib/home";
import { fmtSharePrice, fmtValuation, HOME_IPOS_SHOWN, type HomeIpo } from "../lib/ipo";
import { pendingTitle, typeLabel } from "../lib/labels";
import type { ThemeView } from "../lib/facts";
import type { PostListItem } from "../lib/posts";
import { formatDate, formatMonthDay } from "../lib/time";
import { translator } from "../lib/translator";

/** Movers shown per side in the column (the brief shows them all). */
const MOVERS_PER_SIDE = 3;
/** Themes shown in the column, and tickers named per theme (the rest are counted). */
const THEMES_SHOWN = 3;
const THEME_TICKERS = 3;
const ARROW = { up: "▲", down: "▼" } as const;
/** Tickers listed per reporting slot; the rest are counted and left to the calendar. */
const TICKERS_PER_SLOT = 12;

type T = ReturnType<typeof translator>;

/** The lead story: edition kicker, wide cover, serif headline, summary and the read link. */
function Lead({ lead, locale, t }: { lead: NonNullable<HomeData["lead"]>; locale: Locale; t: T }) {
  const edition = lead.contentType === "recap" ? "recap" : "premarket";
  const href = `/${locale}/p/${lead.slug}`;
  const pending = lead.status === "pending";
  const title = pending ? pendingTitle(t, lead, locale) : lead.title || lead.slug;
  return (
    <section className="front-lead">
      <p className="kicker">
        {t(`paper.dateline.${edition}`)} · <time dateTime={lead.tradingDate}>{formatDate(lead.tradingDate, locale)}</time>
      </p>
      <Cover
        coverKey={lead.coverKey}
        // A pending translation may carry stale lines; show the graphic and label only.
        lines={pending ? null : lead.coverLines}
        tag={{ kind: "label", text: `${formatMonthDay(lead.tradingDate, locale)} · ${t(`paper.edition.${edition}`)}` }}
        size="lead"
        alt={title}
      />
      {pending ? (
        <>
          <h1 className="story-title">{title}</h1>
          <PendingNotice slug={lead.slug} locale={locale} />
        </>
      ) : (
        <>
          <h1 className="story-title">
            <Link href={href}>{title}</Link>
          </h1>
          {lead.summary && <p className="story-sum">{lead.summary}</p>}
          <Link className="more" href={href}>
            {edition === "recap" ? t("home.leadRead") : t("home.readFull")}
          </Link>
        </>
      )}
    </section>
  );
}

/** One earnings check: a small cover tagged with the company logo (or "Earnings check"), and its serif headline. */
function Check({ post, locale, t }: { post: PostListItem; locale: Locale; t: T }) {
  const title = post.title || post.slug;
  const tag = post.ticker
    ? ({ kind: "logo", ticker: post.ticker } as const)
    : ({ kind: "label", text: typeLabel(t, "earnings") } as const);
  return (
    <li className="front-check">
      <Cover coverKey={post.coverKey} lines={post.coverLines} tag={tag} size="small" alt={title} />
      <h3 className="story-title">
        <Link href={`/${locale}/p/${post.slug}`}>{title}</Link>
      </h3>
    </li>
  );
}

/** One reporting slot: its label and the tickers, each linking to the company's page. */
function Slot({ label, entries, locale, t }: { label: string; entries: CalendarEntry[]; locale: Locale; t: T }) {
  if (entries.length === 0) return null;
  const shown = entries.slice(0, TICKERS_PER_SLOT);
  const rest = entries.length - shown.length;
  return (
    <>
      <dt>{label}</dt>
      <dd>
        {shown.map((e, i) => (
          <span key={e.ticker}>
            {i > 0 && ", "}
            <Link href={`/${locale}/t/${encodeURIComponent(e.ticker)}`}>{e.ticker}</Link>
          </span>
        ))}
        {rest > 0 && (
          <>
            {", "}
            <Link className="muted" href={`/${locale}/earnings`}>
              {t("earnings.stripMore", { count: rest })}
            </Link>
          </>
        )}
      </dd>
    </>
  );
}

/** One theme: the group, its median move, and the first few tickers; the whole row links to the lead post. */
function Theme({ theme, href, locale, t }: { theme: ThemeView; href: string; locale: Locale; t: T }) {
  const shown = theme.members.slice(0, THEME_TICKERS);
  // The generator's `more` counts members past the ones it lists; the ones listed but not shown here count too.
  const more = theme.more + (theme.members.length - shown.length);
  return (
    <div className="theme-row">
      <Link className="theme-link" href={href}>
        <span className="theme-head">
          <b className="theme-name">{locale === "zh" ? theme.groupZh : theme.group}</b>
          <span className={`num ${theme.direction}`}>
            <span aria-hidden="true">{ARROW[theme.direction]}</span>
            <span className="sr-only">{t(`paper.index.${theme.direction}`)} </span>
            {fmtPct(theme.medianPct)}
          </span>
        </span>
        <span className="theme-members muted">
          {shown.map((m) => (
            <span key={m.symbol} className="theme-member">
              {m.symbol} <span className="num">{fmtPct(m.changePct)}</span>
            </span>
          ))}
          {more > 0 && <span className="theme-more">{t("home.themesMore", { n: more })}</span>}
        </span>
      </Link>
    </div>
  );
}

/**
 * One active IPO in the home section: its status as the kicker, the name linking to its IPO page, symbol and
 * exchange, the price with the filing it came from, the first trading day worded as the IPO page words it, the newest
 * reported valuation with its outlet, after listing the latest close and its change from the IPO price (both inputs
 * shown, Yahoo named), the newest write-up, and the link to the IPO page.
 */
function HomeIpoItem({ ipo, locale, t }: { ipo: HomeIpo; locale: Locale; t: T }) {
  const page = `/${locale}/ipo/${ipo.slug}`;
  const meta = [ipo.symbol, ipo.exchange].filter((s): s is string => s !== null);
  const s = ipo.latest;
  const sep = locale === "zh" ? "，" : ", ";
  return (
    <article className="front-ipo-item">
      {/* The status is read with the name, in the heading; the kicker above it is the visual copy. */}
      <p className="kicker" aria-hidden="true">
        {t(`ipo.status.${ipo.status}`)}
      </p>
      <h3 className="story-title">
        <Link href={page}>{ipo.name}</Link>
        <span className="sr-only">
          {locale === "zh" ? "，" : ", "}
          {t(`ipo.status.${ipo.status}`)}
        </span>
      </h3>
      {meta.length > 0 && <p className="front-ipo-meta">{meta.join(" · ")}</p>}
      <dl className="front-ipo-facts">
        {ipo.price && ipo.priceFiling && (
          <>
            <dt>{t("final" in ipo.price ? "ipo.offering.priceFinal" : "ipo.offering.priceRange")}</dt>
            <dd>
              <b>
                {"final" in ipo.price
                  ? t("ipo.offering.price", { price: fmtSharePrice(ipo.price.final) })
                  : t("ipo.offering.range", { low: fmtSharePrice(ipo.price.low), high: fmtSharePrice(ipo.price.high) })}
              </b>{" "}
              <span className="muted">
                {t("ipo.home.from", { form: ipo.priceFiling.form, date: formatDate(ipo.priceFiling.filingDate, locale) })}
              </span>
            </dd>
          </>
        )}
        {ipo.valuation && (
          <>
            <dt>{t("ipo.home.valuation")}</dt>
            <dd>
              <b>{fmtValuation(ipo.valuation.usd, locale)}</b>
              {sep}
              {t("ipo.home.reportedBy", { outlet: ipo.valuation.outlet, date: formatDate(ipo.valuation.publishedOn, locale) })}
              {ipo.valuation.url && (
                <>
                  {" "}
                  <a
                    href={ipo.valuation.url}
                    rel="nofollow noopener noreferrer"
                    aria-label={t("ipo.home.articleBy", { outlet: ipo.valuation.outlet })}
                  >
                    ↗
                  </a>
                </>
              )}
              {/* The article's own sentence, word for word: any hedge in it ("about", "roughly") stays with the figure. */}
              <blockquote className="front-ipo-quote">“{ipo.valuation.quote}”</blockquote>
            </dd>
          </>
        )}
        {s && (
          <>
            <dt>{t("ipo.home.lastClose")}</dt>
            <dd>
              <b className="num">{t("ipo.trading.money", { n: fmtPrice(s.close) })}</b>
              {sep}
              <time dateTime={s.sessionDate}>{formatDate(s.sessionDate, locale)}</time>
              {s.changePct !== null && s.ipoPrice !== null && (
                <>
                  {" · "}
                  <span className={`num ${dirOf(s.changePct)}`}>{fmtPct(s.changePct)}</span>
                  {locale === "zh" ? "" : " "}
                  <span className="muted">
                    {locale === "zh" ? "（" : "("}
                    {t("ipo.trading.changeInputs", { close: fmtPrice(s.close), ipo: fmtPrice(s.ipoPrice) })}
                    {locale === "zh" ? "）" : ")"}
                  </span>
                </>
              )}{" "}
              <span className="muted">{t("ipo.trading.source", { source: "Yahoo Finance" })}</span>
            </dd>
          </>
        )}
      </dl>
      <DateLine date={ipo.date} locale={locale} t={t} compact />
      {/* A date from a news report shows the report's sentence too, as the valuation does. */}
      {ipo.date?.source === "news" && ipo.date.quote && <blockquote className="front-ipo-quote">“{ipo.date.quote}”</blockquote>}
      {ipo.writeup && (
        <p className="front-ipo-writeup">
          <span className="muted">{t("ipo.home.writeup")}</span>{" "}
          <Link href={`/${locale}/p/${ipo.writeup.slug}`}>{ipo.writeup.title}</Link>
        </p>
      )}
      <Link className="more" href={page}>
        {t("ipo.home.page")} →
      </Link>
    </article>
  );
}

/**
 * The front page: on the left the lead story, the earnings-check covers and the recent list; on the right
 * a ruled column of top movers and who reports next. Below 721px it stacks lead, movers, earnings, recent,
 * reporting (CSS order). The weekly-digest signup (only when `signup`, i.e. the Buttondown key is set)
 * and the partner strip close the page. No client JS beyond those two islands.
 */
export default function HomeView({ data, locale, signup }: { data: HomeData; locale: Locale; signup: boolean }) {
  const { lead, leadFacts, earnings, recent, reporting, ipos } = data;
  const t = translator(locale);
  const movers = leadFacts
    ? [...leadFacts.gainers.slice(0, MOVERS_PER_SIDE), ...leadFacts.losers.slice(0, MOVERS_PER_SIDE)]
    : [];

  const themes = leadFacts?.themes.slice(0, THEMES_SHOWN) ?? [];

  return (
    <>
      <div className="front">
        {lead ? (
          <Lead lead={lead} locale={locale} t={t} />
        ) : (
          <section className="front-lead">
            <p>{t("home.empty")}</p>
          </section>
        )}

        <section className="front-earnings">
          <h2 className="section-title">{t("home.earningsChecks")}</h2>
          {earnings.length > 0 ? (
            <>
              <ul className="front-checks">
                {earnings.map((e) => (
                  <Check key={e.slug} post={e} locale={locale} t={t} />
                ))}
              </ul>
              <Link className="more" href={`/${locale}/archive?type=earnings`}>
                {t("home.allEarnings")}
              </Link>
            </>
          ) : (
            <p className="muted">{t("home.noEarnings")}</p>
          )}
        </section>

        {recent.length > 0 && (
          <section className="front-recent">
            <h2 className="section-title">{t("home.recentSection")}</h2>
            <ul className="front-rows">
              {recent.map((p) => (
                <li key={p.slug} className="front-row">
                  <p className="kicker">{typeLabel(t, p.contentType)}</p>
                  <h3 className="story-title">
                    <Link href={`/${locale}/p/${p.slug}`}>{p.title || p.slug}</Link>
                  </h3>
                  <time className="story-meta" dateTime={p.tradingDate}>
                    {formatDate(p.tradingDate, locale)}
                  </time>
                </li>
              ))}
            </ul>
            <Link className="more" href={`/${locale}/archive`}>
              {t("home.allBriefs")} →
            </Link>
          </section>
        )}

        <aside className="front-side">
          {ipos.length > 0 && (
            <section className="front-ipo" aria-labelledby="front-ipo">
              <h2 className="section-title" id="front-ipo">
                {t("ipo.watchTitle")}
              </h2>
              {ipos.slice(0, HOME_IPOS_SHOWN).map((i) => (
                <HomeIpoItem key={i.slug} ipo={i} locale={locale} t={t} />
              ))}
              {ipos.length > HOME_IPOS_SHOWN && (
                <Link className="more front-ipo-more" href={`/${locale}/earnings`}>
                  {t("ipo.home.more", { count: ipos.length - HOME_IPOS_SHOWN })}
                </Link>
              )}
            </section>
          )}
          {lead && (
            <section className="front-movers">
              <h2 className="section-title">{t("home.topMovers")}</h2>
              {movers.length > 0 ? (
                <MoversTable movers={movers} locale={locale} variant="column" />
              ) : leadFacts ? (
                <p className="muted">{t("home.noMovers")}</p>
              ) : (
                <Link className="more" href={`/${locale}/p/${lead.slug}`}>
                  {t("home.moversInBrief")}
                </Link>
              )}
            </section>
          )}
          {lead && themes.length > 0 && (
            <section className="front-themes">
              <h2 className="section-title">{t("home.themes")}</h2>
              {themes.map((th) => (
                <Theme key={th.group} theme={th} href={`/${locale}/p/${lead.slug}`} locale={locale} t={t} />
              ))}
            </section>
          )}
          {reporting && (
            <section className="front-reporting">
              <h2 className="section-title">{t("home.reporting", { date: formatDate(reporting.date, locale) })}</h2>
              <dl className="front-slots">
                <Slot label={t("home.beforeOpen")} entries={reporting.bmo} locale={locale} t={t} />
                <Slot label={t("home.afterClose")} entries={reporting.amc} locale={locale} t={t} />
                <Slot label={t("home.timeTbd")} entries={reporting.unknown} locale={locale} t={t} />
              </dl>
              <Link className="more" href={`/${locale}/earnings`}>
                {t("earnings.stripAll")}
              </Link>
            </section>
          )}
        </aside>
      </div>
      {signup && <SignupForm source="home" locale={locale} />}
      <PartnersStrip locale={locale} />
    </>
  );
}

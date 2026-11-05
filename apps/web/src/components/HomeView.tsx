import Link from "next/link";
import Cover from "./Cover";
import MoversTable from "./MoversTable";
import PartnersStrip from "./PartnersStrip";
import PendingNotice from "./PendingNotice";
import SignupForm from "./SignupForm";
import type { Locale } from "../i18n/routing";
import type { CalendarEntry } from "../lib/earnings";
import type { HomeData } from "../lib/home";
import { pendingTitle, typeLabel } from "../lib/labels";
import type { PostListItem } from "../lib/posts";
import { formatDate, formatMonthDay } from "../lib/time";
import { translator } from "../lib/translator";

/** Movers shown per side in the column (the brief shows them all). */
const MOVERS_PER_SIDE = 3;
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
        // A pending translation may carry stale lines; show the scene and label only.
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

/**
 * The front page: on the left the lead story, the earnings-check covers and the recent list; on the right
 * a ruled column of top movers and who reports next. Below 721px it stacks lead, movers, earnings, recent,
 * reporting (CSS order). The weekly-digest signup (only when `signup`, i.e. the Buttondown key is set)
 * and the partner strip close the page. No client JS beyond those two islands.
 */
export default function HomeView({ data, locale, signup }: { data: HomeData; locale: Locale; signup: boolean }) {
  const { lead, leadFacts, earnings, recent, reporting } = data;
  const t = translator(locale);
  const movers = leadFacts
    ? [...leadFacts.gainers.slice(0, MOVERS_PER_SIDE), ...leadFacts.losers.slice(0, MOVERS_PER_SIDE)]
    : [];

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

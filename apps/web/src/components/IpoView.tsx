import Link from "next/link";
import type { ReactNode } from "react";
import type { Locale } from "../i18n/routing";
import { fmtPct, fmtPrice, dirOf } from "../lib/format";
import {
  type FactKind,
  type IpoDate,
  type IpoFact,
  type IpoItem,
  type IpoPage,
  type IpoSession,
  fmtShares,
  fmtSharePrice,
  fmtValuation,
  formLabel,
} from "../lib/ipo";
import { formatDate, formatDayShort } from "../lib/time";
import { translator } from "../lib/translator";

type T = ReturnType<typeof translator>;

/** Links to articles: third-party pages PreBell does not vouch for. */
const ARTICLE_REL = "nofollow noopener noreferrer";

/**
 * One IPO as a reading column, sections in the design's order: the header (name, symbol, exchange, status, the
 * date line with its source), the offering from the filing cover, the filing timeline, valuation as reported,
 * recent moves, what others say, the numbers after listing, then the disclaimer. A section with nothing to show is
 * left out. Every string from the database is rendered as text; a link only when the reader made it `https://`.
 * PreBell's own words are the labels: figures not from a filing are attributed, and a view appears only as a
 * quotation with its speaker and outlet.
 */
export default function IpoView({ ipo, locale }: { ipo: IpoPage; locale: Locale }) {
  const t = translator(locale);
  const day = (d: string) => formatDate(d, locale);
  const rw = ipo.filings.find((f) => f.form === "RW");
  const meta = [ipo.symbol, ipo.exchange].filter((s): s is string => s !== null);

  return (
    <article className="prose prose-ipo">
      <p className="kicker">{t("ipo.kicker")}</p>
      <h1>{t("ipo.title", { name: ipo.name })}</h1>
      <p className="ipo-meta">
        {meta.length > 0 && <span>{meta.join(" · ")}</span>}
        <span className="ipo-status">{t(`ipo.status.${ipo.status}`)}</span>
      </p>
      {ipo.status === "withdrawn" && (
        <p className="notice ipo-withdrawn">
          {t("ipo.withdrawn", { hasDay: rw ? "yes" : "no", day: rw ? day(rw.filingDate) : "" })}
        </p>
      )}
      {/* A withdrawn IPO has no expected date. */}
      {ipo.status !== "withdrawn" && <DateLine date={ipo.date} locale={locale} t={t} />}

      {ipo.writeups.length > 0 && (
        <>
          <h2 id="writeups">{t("ipo.writeups.heading")}</h2>
          <ul className="date-list ipo-writeups">
            {ipo.writeups.map((w) => (
              <li key={w.slug} className="date-row">
                <span>
                  <time dateTime={w.tradingDate}>{day(w.tradingDate)}</time>{" "}
                  <Link className="ipo-writeup-link" href={`/${locale}/p/${w.slug}`}>
                    {w.title}
                  </Link>
                </span>
              </li>
            ))}
          </ul>
        </>
      )}

      <h2 id="offering">{t("ipo.offering.heading")}</h2>
      <Offering ipo={ipo} locale={locale} t={t} />

      {ipo.filings.length > 0 && (
        <>
          <h2 id="filings">{t("ipo.timeline.heading")}</h2>
          <ul className="date-list ipo-timeline">
            {ipo.filings.map((f) => (
              <li key={f.accession} className="date-row">
                <span>
                  <time dateTime={f.filingDate}>{day(f.filingDate)}</time>{" "}
                  <span className="ipo-form">{formLabel(f.form, locale) ?? f.form}</span>
                </span>
                {f.indexUrl && (
                  <span className="date-end">
                    <a className="more" href={f.indexUrl} rel="noopener noreferrer">
                      {t("ipo.timeline.filing")} ↗
                    </a>
                  </span>
                )}
              </li>
            ))}
          </ul>
        </>
      )}

      {ipo.valuations.length > 0 && (
        <>
          <h2 id="valuation">{t("ipo.valuation.heading")}</h2>
          <p className="ipo-note">{t("ipo.valuation.note")}</p>
          <Items
            items={ipo.valuations}
            head={(i) => {
              // A missing or implausible figure is not printed; the attributed quote below still is.
              const figure = i.valueUsd === null ? null : fmtValuation(i.valueUsd, locale);
              return figure && <strong className="ipo-figure">{figure}</strong>;
            }}
            locale={locale}
            t={t}
          />
        </>
      )}

      {ipo.moves.length > 0 && (
        <>
          <h2 id="moves">{t("ipo.moves.heading")}</h2>
          <Items items={ipo.moves} head={(i) => i.statement} locale={locale} t={t} />
        </>
      )}

      {ipo.views.length > 0 && (
        <>
          <h2 id="views">{t("ipo.views.heading")}</h2>
          <ul className="ipo-items">
            {ipo.views.map((v) => (
              <li key={v.url ?? `${v.outlet}:${v.publishedOn}:${v.quote}`} className="ipo-item ipo-view">
                <p className="ipo-by">
                  {t("ipo.views.by", { speaker: v.speaker ?? "", outlet: v.outlet, date: day(v.publishedOn) })}
                </p>
                <blockquote className="ipo-quote">“{v.quote}”</blockquote>
                <ArticleLink url={v.url} t={t} />
              </li>
            ))}
          </ul>
        </>
      )}

      {ipo.trading && (
        <>
          <h2 id="trading">{t("ipo.trading.heading")}</h2>
          {/* "First trading day" only when the date line says this session was day one; else each by its date. */}
          {ipo.trading.first && (
            <Session label={ipo.trading.firstIsDayOne ? t("ipo.trading.first") : null} s={ipo.trading.first} locale={locale} t={t} />
          )}
          {ipo.trading.latest && (
            <Session label={ipo.trading.firstIsDayOne ? t("ipo.trading.latest") : null} s={ipo.trading.latest} locale={locale} t={t} />
          )}
          <TradingSource url={(ipo.trading.latest ?? ipo.trading.first)?.sourceUrl ?? null} t={t} />
        </>
      )}

      <hr className="rule" />
      <p className="ipo-disclaimer">
        {t("ipo.disclaimer")}
        {locale === "zh" ? "" : " "}
        {t("ipo.asReported")}
      </p>
      <p>
        <Link className="more" href={`/${locale}/earnings`}>
          {t("earnings.calendarTitle")} →
        </Link>
      </p>
    </article>
  );
}

/**
 * The first trading day with who says so, the unconfirmed or confirmed mark, the source link and its sentence. The
 * home page's section uses it too (`compact`: without the source's words), so both say the date the same way.
 */
export function DateLine({ date, locale, t, compact = false }: { date: IpoDate | null; locale: Locale; t: T; compact?: boolean }) {
  if (!date) return <p className="ipo-date">{t("ipo.date.notSet")}</p>;
  const when = formatDayShort(date.date, locale) + (locale === "en" ? `, ${date.date.slice(0, 4)}` : "");
  const vars = {
    date: locale === "en" ? when : formatDate(date.date, locale),
    hasDay: date.asOf ? "yes" : "no",
    day: date.asOf ? formatDate(date.asOf, locale) : "",
  };
  const key =
    date.status === "reported"
      ? "reported"
      : date.status === "expected"
        ? "expected"
        : date.source === "trading"
          ? "confirmedTrading"
          : "confirmedFiling";
  const sentence = t(`ipo.date.${key}`, { ...vars, source: date.sourceName });
  const confirmed = date.status === "confirmed";
  return (
    <div className="ipo-date">
      <p>
        <span className={confirmed ? "ipo-flag ipo-flag-solid" : "ipo-flag"}>
          {t(confirmed ? "earnings.confirmed" : "earnings.unconfirmed")}
        </span>{" "}
        {sentence}
        {date.sourceUrl && (
          <>
            {" "}
            <a className="more" href={date.sourceUrl} rel={date.source === "news" ? ARTICLE_REL : "noopener noreferrer"}>
              {t(date.source === "news" ? "ipo.article" : date.source === "filing" ? "ipo.secFiling" : "ipo.trading.source", {
                source: "Yahoo Finance",
              })}{" "}
              ↗
            </a>
          </>
        )}
      </p>
      {date.quote && !compact && <Quote quote={date.quote} summary={t("ipo.sourceWords")} />}
    </div>
  );
}

/** A source sentence behind a native disclosure, in quotation marks. */
function Quote({ quote, summary }: { quote: string; summary: string }) {
  return (
    <details className="ipo-src-quote">
      <summary>{summary}</summary>
      <blockquote className="ipo-quote">“{quote}”</blockquote>
    </details>
  );
}

type OfferingLabel =
  | "exchange"
  | "symbol"
  | "sharesOffered"
  | "sharesCompany"
  | "sharesSelling"
  | "optionShares"
  | "priceRange"
  | "priceFinal"
  | "prospectusDate";

const SHARE_PRICE_ROWS = new Set(["shares_offered", "shares_company", "shares_selling", "option_shares", "price_range", "price_final"]);

const SHARE_ROWS: [FactKind, OfferingLabel][] = [
  ["shares_offered", "sharesOffered"],
  ["shares_company", "sharesCompany"],
  ["shares_selling", "sharesSelling"],
];

/** The cover facts with the filing they came from, each with its sentence; says plainly what is not there yet. */
function Offering({ ipo, locale, t }: { ipo: IpoPage; locale: Locale; t: T }) {
  if (!ipo.facts) return <p>{t("ipo.offering.none")}</p>;
  const { filing, facts } = ipo.facts;
  const rows: { key: string; label: string; value: ReactNode; fact: IpoFact }[] = [];
  const add = (key: string, fact: IpoFact | undefined, label: OfferingLabel, value: (f: IpoFact) => ReactNode) => {
    if (fact) rows.push({ key, label: t(`ipo.offering.${label}`), value: value(fact), fact });
  };
  add("exchange", facts.exchange, "exchange", (f) => f.text);
  add("symbol", facts.symbol, "symbol", (f) => f.text);
  for (const [kind, label] of SHARE_ROWS) add(kind, facts[kind], label, (f) => t("ipo.offering.shares", { n: fmtShares(f.num!) }));
  add("option_shares", facts.option_shares, "optionShares", (f) => t("ipo.offering.option", { n: fmtShares(f.num!) }));
  if (facts.price_low && facts.price_high) {
    add("price_range", facts.price_low, "priceRange", () =>
      t("ipo.offering.range", { low: fmtSharePrice(facts.price_low!.num!), high: fmtSharePrice(facts.price_high!.num!) }),
    );
  }
  add("price_final", facts.price_final, "priceFinal", (f) => t("ipo.offering.price", { price: fmtSharePrice(f.num!) }));
  add("prospectus_date", facts.prospectus_date, "prospectusDate", (f) => formatDate(f.text!, locale));
  // Said only when no share or price row is shown, so it never contradicts a row above it.
  const terms = rows.some((r) => SHARE_PRICE_ROWS.has(r.key));

  return (
    <>
      <p className="ipo-src">
        {t("ipo.offering.from", { form: filing.form, date: formatDate(filing.filingDate, locale) })}
        {filing.indexUrl && (
          <>
            {" · "}
            <a href={filing.indexUrl} rel="noopener noreferrer">
              {t("ipo.secFiling")} ↗
            </a>
          </>
        )}
      </p>
      {rows.length > 0 && (
        <dl className="ipo-facts">
          {rows.map((r) => (
            <div key={r.key} className="ipo-fact">
              <dt>{r.label}</dt>
              <dd>
                <span className="ipo-value">{r.value}</span>
                <Quote quote={r.fact.quote} summary={t("ipo.filingWords")} />
              </dd>
            </div>
          ))}
        </dl>
      )}
      {!terms && <p className="ipo-note">{t("ipo.offering.noTerms")}</p>}
    </>
  );
}

/**
 * As-reported items, newest first: the lead (a figure or a move's statement), who reported it and when, the quote
 * and the link. With no lead (v1 stores no statements), the attribution introduces the quote in the ink of a view's
 * line ("Sample Times reported on Nov 4, 2026:"), so the quote reads as the item.
 */
function Items({ items, head, locale, t }: { items: IpoItem[]; head: (i: IpoItem) => ReactNode; locale: Locale; t: T }) {
  return (
    <ul className="ipo-items">
      {items.map((i) => {
        const lead = head(i);
        return (
          <li key={i.url ?? `${i.outlet}:${i.publishedOn}:${i.quote}`} className="ipo-item">
            {lead && <p className="ipo-lead">{lead}</p>}
            {lead ? (
              <p className="ipo-by">{t("ipo.reportedBy", { outlet: i.outlet, date: formatDate(i.publishedOn, locale) })}</p>
            ) : (
              <p className="ipo-by ipo-by-lead">{t("ipo.reportedByLead", { outlet: i.outlet, date: formatDate(i.publishedOn, locale) })}</p>
            )}
            <blockquote className="ipo-quote">“{i.quote}”</blockquote>
            <ArticleLink url={i.url} t={t} />
          </li>
        );
      })}
    </ul>
  );
}

/** Yahoo Finance, named as the source of every after-listing number, linked when the quote page is stored. */
function TradingSource({ url, t }: { url: string | null; t: T }) {
  const label = t("ipo.trading.source", { source: "Yahoo Finance" });
  return (
    <p className="ipo-src">
      {url ? (
        <a href={url} rel="noopener noreferrer">
          {label} ↗
        </a>
      ) : (
        label
      )}
    </p>
  );
}

function ArticleLink({ url, t }: { url: string | null; t: T }) {
  if (!url) return null;
  return (
    <p className="ipo-link">
      <a className="more" href={url} rel={ARTICLE_REL}>
        {t("ipo.article")} ↗
      </a>
    </p>
  );
}

/**
 * One trading session: close, volume (open only when stored) and the change from the IPO price beside both inputs.
 * Headed by its label and day, or, with no label, by its full date alone.
 */
function Session({ label, s, locale, t }: { label: string | null; s: IpoSession; locale: Locale; t: T }) {
  const full = formatDate(s.sessionDate, locale);
  return (
    <section className="ipo-session" aria-label={label ? `${label}, ${full}` : full}>
      <h3>
        {label ? (
          <>
            {label} · <time dateTime={s.sessionDate}>{formatDayShort(s.sessionDate, locale)}</time>
          </>
        ) : (
          <time dateTime={s.sessionDate}>{full}</time>
        )}
      </h3>
      <dl className="ipo-facts">
        {s.open !== null && (
          <div className="ipo-fact">
            <dt>{t("ipo.trading.open")}</dt>
            <dd className="num">{t("ipo.trading.money", { n: fmtPrice(s.open) })}</dd>
          </div>
        )}
        <div className="ipo-fact">
          <dt>{t("ipo.trading.close")}</dt>
          <dd className="num">{t("ipo.trading.money", { n: fmtPrice(s.close) })}</dd>
        </div>
        {s.volume !== null && (
          <div className="ipo-fact">
            <dt>{t("ipo.trading.volume")}</dt>
            <dd className="num">{t("ipo.trading.volumeValue", { n: fmtShares(s.volume) })}</dd>
          </div>
        )}
        {s.changePct !== null && s.ipoPrice !== null && (
          <div className="ipo-fact">
            <dt>{t("ipo.trading.change")}</dt>
            <dd>
              <span className={`num ${dirOf(s.changePct)}`}>{fmtPct(s.changePct)}</span>
              {locale === "zh" ? "" : " "}
              <span className="ipo-inputs">
                {locale === "zh" ? "（" : "("}
                {t("ipo.trading.changeInputs", { close: fmtPrice(s.close), ipo: fmtPrice(s.ipoPrice) })}
                {locale === "zh" ? "）" : ")"}
              </span>
            </dd>
          </div>
        )}
      </dl>
    </section>
  );
}

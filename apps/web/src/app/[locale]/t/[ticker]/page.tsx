import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";
import { getTranslations, setRequestLocale } from "next-intl/server";
import PostList from "@/components/PostList";
import type { Locale } from "@/i18n/routing";
import { getDb } from "@/lib/db";
import {
  companyName,
  listTickerCalendar,
  listTickerPosts,
  normalizeTicker,
  tickerHasCheck,
  tickerKnown,
} from "@/lib/earnings";
import { pairedAlternates } from "@/lib/seo";
import { formatDate } from "@/lib/time";

export async function generateMetadata({ params }: PageProps<"/[locale]/t/[ticker]">): Promise<Metadata> {
  const { locale, ticker: raw } = await params;
  const ticker = normalizeTicker(raw);
  if (!ticker) return {};
  const t = await getTranslations({ locale, namespace: "earnings" });
  // A calendar-only company's page is a list of dates: not indexed until it has a check.
  const checked = await tickerHasCheck(getDb(), ticker);
  return {
    title: t("tickerTitle", { ticker }),
    alternates: pairedAlternates(locale, `/t/${encodeURIComponent(ticker)}`),
    ...(checked ? {} : { robots: { index: false } }),
  };
}

export default async function TickerArchive({ params }: PageProps<"/[locale]/t/[ticker]">) {
  const { ticker: raw } = await params;
  const locale = (await params).locale as Locale;
  setRequestLocale(locale);
  const ticker = normalizeTicker(raw);
  const db = getDb();
  if (!ticker || !(await tickerKnown(db, ticker))) notFound();
  const t = await getTranslations("earnings");
  const [posts, dates] = await Promise.all([
    listTickerPosts(db, ticker, locale),
    listTickerCalendar(db, ticker, locale),
  ]);
  const status: Record<string, string> = {
    unconfirmed: t("unconfirmed"),
    confirmed: t("confirmed"),
    reported: t("reported"),
  };
  const slot: Record<string, string> = { bmo: t("bmo"), amc: t("amc"), unknown: t("timeUnknown") };
  const name = companyName(ticker);

  return (
    <div className="list-page">
      <h1>{t("tickerTitle", { ticker })}</h1>
      {name && <p className="meta">{name}</p>}
      <section aria-labelledby="checks">
        <h2 className="section-title" id="checks">
          {t("checks")}
        </h2>
        {posts.length === 0 ? <p className="muted">{t("noChecks")}</p> : <PostList locale={locale} items={posts} />}
      </section>
      {dates.length > 0 && (
        <section aria-labelledby="dates">
          <h2 className="section-title" id="dates">
            {t("dates")}
          </h2>
          <ul className="date-list">
            {dates.map((d) => (
              <li key={d.date} className="date-row">
                <span>
                  <time dateTime={d.date}>{formatDate(d.date, locale)}</time>{" "}
                  <span className="cal-slot">{slot[d.slot]}</span>
                </span>
                <span className="date-end">
                  <span className={d.status === "unconfirmed" ? "muted" : undefined}>
                    {d.status === "reported" && (
                      <>
                        <span className="num up" aria-hidden="true">
                          ✓
                        </span>{" "}
                      </>
                    )}
                    {status[d.status]}
                  </span>
                  {d.postSlug ? (
                    <Link
                      className="more"
                      href={`/${locale}/p/${d.postSlug}`}
                      aria-label={`${t("readCheck")}: ${formatDate(d.date, locale)}`}
                    >
                      {t("readCheck")} →
                    </Link>
                  ) : (
                    d.filingUrl && (
                      <a
                        className="more"
                        href={d.filingUrl}
                        rel="noopener noreferrer"
                        aria-label={`${t("secFiling")}: ${formatDate(d.date, locale)}`}
                      >
                        {t("secFiling")} ↗
                      </a>
                    )
                  )}
                </span>
              </li>
            ))}
          </ul>
        </section>
      )}
      <p>
        <Link className="more" href={`/${locale}/earnings`}>
          {t("calendarTitle")} →
        </Link>
      </p>
    </div>
  );
}

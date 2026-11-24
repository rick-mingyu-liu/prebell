import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";
import { getTranslations, setRequestLocale } from "next-intl/server";
import CalendarWeek from "@/components/CalendarWeek";
import IpoWatchList from "@/components/IpoWatchList";
import ShareButton from "@/components/ShareButton";
import type { Locale } from "@/i18n/routing";
import { getDb } from "@/lib/db";
import { addDays, calendarWeeks, INDEXED_WEEKS, isDate, isWeekend, listCalendarWeek, readingWeek, todayEt, weekStart } from "@/lib/earnings";
import { listEvents } from "@/lib/events";
import { iposByDate, listIposBetween, listIpoWatch } from "@/lib/ipo";
import { pairedAlternates, shareMetadata } from "@/lib/seo";
import { formatDate } from "@/lib/time";

/** The `?week=` date, when there is a valid one. */
const weekParam = (raw: string | string[] | undefined) => {
  const v = Array.isArray(raw) ? raw[0] : raw;
  return isDate(v) ? v : null;
};

/** The Monday of the week shown: the `?week=` date's week, else the reading week (the coming week at weekends). */
function monday(raw: string | string[] | undefined, today: string): string {
  const v = weekParam(raw);
  return v ? weekStart(v) : readingWeek(today);
}

/** A 404 for a week outside the calendar's range, before anything is read. */
function inRange(start: string, today: string): { first: string; last: string } {
  const range = calendarWeeks(today);
  if (start < range.first || start > range.last) notFound();
  return range;
}

export async function generateMetadata({ params, searchParams }: PageProps<"/[locale]/earnings">): Promise<Metadata> {
  const { locale } = await params;
  const week = (await searchParams).week;
  const t = await getTranslations({ locale });
  const today = todayEt();
  const start = monday(week, today);
  inRange(start, today);
  const query = weekParam(week) ? `?week=${start}` : "";
  // Only weeks near today are worth indexing (measured from this week, weekends included).
  const distance = Math.abs(Date.parse(start) - Date.parse(weekStart(today))) / (7 * 86_400_000);
  // A dated week names its week, so the weeks' titles differ; the undated page moves with today.
  const title = query
    ? `${t("earnings.calendarTitle")} · ${t("earnings.boardTitle", { date: formatDate(start, locale as Locale) })}`
    : t("earnings.calendarTitle");
  const description = t("earnings.calendarDescription");
  return {
    title,
    description,
    alternates: pairedAlternates(locale, `/earnings${query}`),
    ...shareMetadata({
      locale: locale as Locale,
      url: `/${locale}/earnings${query}`,
      title,
      description,
      siteName: t("site.name"),
    }),
    ...(distance > INDEXED_WEEKS ? { robots: { index: false } } : {}),
  };
}

export default async function EarningsCalendar({ params, searchParams }: PageProps<"/[locale]/earnings">) {
  const locale = (await params).locale as Locale;
  setRequestLocale(locale);
  const week = (await searchParams).week;
  const today = todayEt();
  const start = monday(week, today);
  const t = await getTranslations("earnings");
  const te = await getTranslations("events");
  const ti = await getTranslations("ipo");
  // At weekends the undated page is the coming week, and says so; the way back from another week goes there.
  const weekend = isWeekend(today);
  const range = inRange(start, today);
  const db = getDb();
  const [days, events, weekIpos, watch] = await Promise.all([
    listCalendarWeek(db, start, locale),
    listEvents(db, start, addDays(start, 4), locale),
    listIposBetween(db, start, addDays(start, 4), locale, today),
    listIpoWatch(db, today, locale),
  ]);
  const href = (d: string) => `/${locale}/earnings?week=${d}`;
  // Companies, not rows: a ticker with two dates in one week counts once.
  const companies = new Set(days.flatMap((d) => d.entries.map((e) => e.ticker))).size;
  const eventCount = Object.values(events).reduce((n, e) => n + e.length, 0);
  // With the year: the week links go on forever, and a shared "Week of Dec 28" would be ambiguous.
  const title = t("boardTitle", { date: formatDate(start, locale) });

  return (
    <div className="cal-page">
      <header className="cal-head">
        <div className="cal-head-main">
          <h1 className="cal-title">
            <span className="kicker">{weekend && !weekParam(week) ? `${t("calendarTitle")} · ${t("comingWeek")}` : t("calendarTitle")}</span>
            {title}
          </h1>
          <p className="cal-count">
            {t("countCompanies", { count: companies })}
            {eventCount > 0 && ` · ${t("countEvents", { count: eventCount })}`}
          </p>
        </div>
        <div className="cal-tools">
          <nav className="cal-weeks" aria-label={t("weeks")}>
            {start > range.first && (
              <Link className="cal-btn cal-arrow" href={href(addDays(start, -7))} rel="prev">
                <span aria-hidden="true">←</span>
                <span className="sr-only">{t("prevWeek")}</span>
              </Link>
            )}
            {start !== readingWeek(today) && (
              <Link className="cal-btn cal-this-week" href={`/${locale}/earnings`}>
                {weekend ? t("comingWeek") : t("thisWeek")}
              </Link>
            )}
            {start < range.last && (
              <Link className="cal-btn cal-arrow" href={href(addDays(start, 7))} rel="next">
                <span aria-hidden="true">→</span>
                <span className="sr-only">{t("nextWeek")}</span>
              </Link>
            )}
          </nav>
          <ShareButton
            path={href(start)}
            title={`${t("calendarTitle")} · ${title}`}
            label={t("share")}
            copied={t("shareCopied")}
            failed={t("shareFailed")}
          />
        </div>
      </header>
      <IpoWatchList ipos={watch} locale={locale} />
      <CalendarWeek days={days} locale={locale} events={events} ipos={iposByDate(weekIpos)} today={today} />
      <p className="legend">
        {t("timesEt")}
        {locale === "zh" ? "。" : ". "}
        {te("legend")}
        {locale === "zh" ? "" : " "}
        {t("legend")}
        {(watch.length > 0 || weekIpos.length > 0) && (
          <>
            {locale === "zh" ? "" : " "}
            {ti("legend")}
          </>
        )}
      </p>
      <nav className="pager">
        {start > range.first && <Link href={href(addDays(start, -7))}>← {t("prevWeek")}</Link>}
        {start < range.last && (
          <Link className="next" href={href(addDays(start, 7))}>
            {t("nextWeek")} →
          </Link>
        )}
      </nav>
    </div>
  );
}

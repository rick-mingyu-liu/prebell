import type { Metadata } from "next";
import Link from "next/link";
import { getTranslations, setRequestLocale } from "next-intl/server";
import CalendarWeek from "@/components/CalendarWeek";
import IpoWatchList from "@/components/IpoWatchList";
import ShareButton from "@/components/ShareButton";
import type { Locale } from "@/i18n/routing";
import { getDb } from "@/lib/db";
import { addDays, INDEXED_WEEKS, isDate, listCalendarWeek, todayEt, weekStart } from "@/lib/earnings";
import { listEvents } from "@/lib/events";
import { iposByDate, listIposBetween, listIpoWatch } from "@/lib/ipo";
import { pairedAlternates, shareMetadata } from "@/lib/seo";
import { formatDate } from "@/lib/time";

function monday(raw: string | string[] | undefined): string {
  const v = Array.isArray(raw) ? raw[0] : raw;
  return weekStart(isDate(v) ? v : todayEt());
}

export async function generateMetadata({ params, searchParams }: PageProps<"/[locale]/earnings">): Promise<Metadata> {
  const { locale } = await params;
  const week = (await searchParams).week;
  const t = await getTranslations({ locale });
  const start = monday(week);
  const query = isDate(Array.isArray(week) ? week[0] : week) ? `?week=${start}` : "";
  // The week links go on forever; only weeks near today are worth indexing.
  const distance = Math.abs(Date.parse(start) - Date.parse(weekStart(todayEt()))) / (7 * 86_400_000);
  return {
    title: t("earnings.calendarTitle"),
    alternates: pairedAlternates(locale, `/earnings${query}`),
    ...shareMetadata({
      locale: locale as Locale,
      url: `/${locale}/earnings${query}`,
      title: t("earnings.calendarTitle"),
      description: t("earnings.calendarDescription"),
      siteName: t("site.name"),
    }),
    ...(distance > INDEXED_WEEKS ? { robots: { index: false } } : {}),
  };
}

export default async function EarningsCalendar({ params, searchParams }: PageProps<"/[locale]/earnings">) {
  const locale = (await params).locale as Locale;
  setRequestLocale(locale);
  const start = monday((await searchParams).week);
  const t = await getTranslations("earnings");
  const te = await getTranslations("events");
  const ti = await getTranslations("ipo");
  const db = getDb();
  const today = todayEt();
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
            <span className="kicker">{t("calendarTitle")}</span>
            {title}
          </h1>
          <p className="cal-count">
            {t("countCompanies", { count: companies })}
            {eventCount > 0 && ` · ${t("countEvents", { count: eventCount })}`}
          </p>
        </div>
        <div className="cal-tools">
          <nav className="cal-weeks" aria-label={t("weeks")}>
            <Link className="cal-btn cal-arrow" href={href(addDays(start, -7))} rel="prev">
              <span aria-hidden="true">←</span>
              <span className="sr-only">{t("prevWeek")}</span>
            </Link>
            {start !== weekStart(today) && (
              <Link className="cal-btn cal-this-week" href={`/${locale}/earnings`}>
                {t("thisWeek")}
              </Link>
            )}
            <Link className="cal-btn cal-arrow" href={href(addDays(start, 7))} rel="next">
              <span aria-hidden="true">→</span>
              <span className="sr-only">{t("nextWeek")}</span>
            </Link>
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
        <Link href={href(addDays(start, -7))}>← {t("prevWeek")}</Link>
        <Link href={href(addDays(start, 7))}>{t("nextWeek")} →</Link>
      </nav>
    </div>
  );
}

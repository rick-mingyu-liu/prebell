import type { Metadata } from "next";
import Link from "next/link";
import { getTranslations, setRequestLocale } from "next-intl/server";
import CalendarWeek from "@/components/CalendarWeek";
import LangSwitch from "@/components/LangSwitch";
import type { Locale } from "@/i18n/routing";
import { getDb } from "@/lib/db";
import { addDays, INDEXED_WEEKS, isDate, listCalendarWeek, todayEt, weekStart } from "@/lib/earnings";
import { pairedAlternates } from "@/lib/seo";
import { formatDate } from "@/lib/time";

function monday(raw: string | string[] | undefined): string {
  const v = Array.isArray(raw) ? raw[0] : raw;
  return weekStart(isDate(v) ? v : todayEt());
}

export async function generateMetadata({ params, searchParams }: PageProps<"/[locale]/earnings">): Promise<Metadata> {
  const { locale } = await params;
  const week = (await searchParams).week;
  const t = await getTranslations({ locale, namespace: "earnings" });
  const start = monday(week);
  const query = isDate(Array.isArray(week) ? week[0] : week) ? `?week=${start}` : "";
  // The week links go on forever; only weeks near today are worth indexing.
  const distance = Math.abs(Date.parse(start) - Date.parse(weekStart(todayEt()))) / (7 * 86_400_000);
  return {
    title: t("calendarTitle"),
    alternates: pairedAlternates(locale, `/earnings${query}`),
    ...(distance > INDEXED_WEEKS ? { robots: { index: false } } : {}),
  };
}

export default async function EarningsCalendar({ params, searchParams }: PageProps<"/[locale]/earnings">) {
  const locale = (await params).locale as Locale;
  setRequestLocale(locale);
  const start = monday((await searchParams).week);
  const t = await getTranslations("earnings");
  const days = await listCalendarWeek(getDb(), start, locale);
  const href = (d: string) => `/${locale}/earnings?week=${d}`;

  return (
    <>
      <div className="page-tools">
        <h1>{t("calendarTitle")}</h1>
        <LangSwitch locale={locale} path={`/earnings?week=${start}`} />
      </div>
      <p className="meta">
        {t("weekOf", { date: formatDate(start, locale) })} · {t("timesEt")}
      </p>
      <p className="muted">{t("legend")}</p>
      <CalendarWeek days={days} locale={locale} />
      <nav className="pager">
        <Link href={href(addDays(start, -7))}>← {t("prevWeek")}</Link>
        <Link href={href(addDays(start, 7))}>{t("nextWeek")} →</Link>
      </nav>
    </>
  );
}

import Link from "next/link";
import { getTranslations } from "next-intl/server";
import type { Locale } from "@/i18n/routing";
import type { CalendarDay, CalendarEntry } from "@/lib/earnings";
import { formatDate } from "@/lib/time";

function Entry({ e, locale, labels }: { e: CalendarEntry; locale: Locale; labels: Record<string, string> }) {
  const ticker = e.postSlug ? (
    <Link href={`/${locale}/p/${e.postSlug}`}>{e.ticker}</Link>
  ) : (
    <Link href={`/${locale}/t/${encodeURIComponent(e.ticker)}`}>{e.ticker}</Link>
  );
  return (
    <li className={`cal-entry cal-${e.status}`}>
      {ticker} <span className="muted">{labels[e.slot]}</span>{" "}
      <span className={`badge badge-${e.status}`}>{labels[e.status]}</span>
    </li>
  );
}

/** One week of report dates; confirmed, unconfirmed and reported are marked in words, not color alone. */
export default async function CalendarWeek({ days, locale }: { days: CalendarDay[]; locale: Locale }) {
  const t = await getTranslations("earnings");
  const labels: Record<string, string> = {
    bmo: t("bmo"),
    amc: t("amc"),
    unknown: t("timeUnknown"),
    unconfirmed: t("unconfirmed"),
    confirmed: t("confirmed"),
    reported: t("reported"),
  };
  return (
    <div className="cal-week">
      {days.map((d) => (
        <section key={d.date} className="cal-day" aria-label={formatDate(d.date, locale)}>
          <h2>
            <time dateTime={d.date}>{formatDate(d.date, locale)}</time>
          </h2>
          {d.entries.length === 0 ? (
            <p className="muted">{t("none")}</p>
          ) : (
            <ul>
              {d.entries.map((e) => (
                <Entry key={e.ticker} e={e} locale={locale} labels={labels} />
              ))}
            </ul>
          )}
        </section>
      ))}
    </div>
  );
}

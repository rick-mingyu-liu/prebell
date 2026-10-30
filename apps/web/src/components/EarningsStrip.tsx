import Link from "next/link";
import type { Locale } from "../i18n/routing";
import { todayEt, type CalendarEntry, type StripDay } from "../lib/earnings";
import { formatDayShort } from "../lib/time";
import { translator } from "../lib/translator";

/** The green band under the header: one day's report entries by slot, as ticker chips. Nothing for no day. */
export default function EarningsStrip({
  day,
  locale,
  today = todayEt(),
}: {
  day: StripDay | null;
  locale: Locale;
  today?: string;
}) {
  if (!day) return null;
  const t = translator(locale);
  const label =
    day.date === today ? t("earnings.stripToday") : t("earnings.stripLabel", { date: formatDayShort(day.date, locale) });
  const groups: [string, CalendarEntry[]][] = [
    [t("earnings.bmo"), day.bmo],
    [t("earnings.amc"), day.amc],
    [t("earnings.stripUnknown"), day.unknown],
  ];
  return (
    <section className="estrip" aria-label={label}>
      <div className="wrap estrip-in">
        <time className="estrip-title" dateTime={day.date}>
          {label}
        </time>
        {groups.map(
          ([name, entries]) =>
            entries.length > 0 && (
              <div key={name} className="estrip-group">
                <span className="estrip-slot">{name}</span>
                {entries.map((e) => (
                  <Link
                    key={e.ticker}
                    className="chip"
                    href={e.postSlug ? `/${locale}/p/${e.postSlug}` : `/${locale}/t/${encodeURIComponent(e.ticker)}`}
                  >
                    {e.ticker}
                  </Link>
                ))}
              </div>
            ),
        )}
        <Link className="estrip-all" href={`/${locale}/earnings`}>
          {t("earnings.stripAll")}
        </Link>
      </div>
    </section>
  );
}

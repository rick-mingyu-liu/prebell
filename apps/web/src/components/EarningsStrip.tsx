import Link from "next/link";
import TickerLogo from "./TickerLogo";
import type { Locale } from "../i18n/routing";
import { todayEt, type CalendarEntry, type StripDay } from "../lib/earnings";
import { formatDayShort } from "../lib/time";
import { translator } from "../lib/translator";

/** Chips shown per group; the rest collapse into a "+K more" chip. */
const MAX_CHIPS = 8;

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
    <section className="estrip">
      <div className="wrap estrip-in">
        <time className="estrip-title" dateTime={day.date}>
          {label}
        </time>
        {groups.map(
          ([name, entries]) =>
            entries.length > 0 && (
              <div key={name} className="estrip-group">
                <span className="estrip-slot">{name}</span>
                {entries.slice(0, MAX_CHIPS).map((e) => (
                  <Link
                    key={e.ticker}
                    className="estrip-chip"
                    href={e.postSlug ? `/${locale}/p/${e.postSlug}` : `/${locale}/t/${encodeURIComponent(e.ticker)}`}
                  >
                    <TickerLogo ticker={e.ticker} />
                    {e.ticker}
                  </Link>
                ))}
                {entries.length > MAX_CHIPS && (
                  <Link className="estrip-chip estrip-more" href={`/${locale}/earnings`}>
                    {t("earnings.stripMore", { count: entries.length - MAX_CHIPS })}
                  </Link>
                )}
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

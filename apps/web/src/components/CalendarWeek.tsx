import Link from "next/link";
import type { Locale } from "../i18n/routing";
import type { CalendarDay, CalendarEntry, Slot } from "../lib/earnings";
import { formatDayShort } from "../lib/time";
import { translator } from "../lib/translator";

const SLOTS: Slot[] = ["bmo", "amc", "unknown"];

/**
 * One week as ruled day rows: the date, then each report as a ticker link and its slot in small caps.
 * Colour carries nothing alone: a reported entry gets the one coloured mark, a tick, and an unconfirmed
 * one is muted; both are also written as text for screen readers.
 */
export default function CalendarWeek({ days, locale }: { days: CalendarDay[]; locale: Locale }) {
  const t = translator(locale);
  const slotLabel: Record<Slot, string> = {
    bmo: t("earnings.bmo"),
    amc: t("earnings.amc"),
    unknown: t("earnings.timeUnknown"),
  };
  const statusLabel: Record<CalendarEntry["status"], string> = {
    unconfirmed: t("earnings.unconfirmed"),
    confirmed: t("earnings.confirmed"),
    reported: t("earnings.reported"),
  };
  return (
    <div className="cal-week">
      {days.map((d) => {
        const entries = SLOTS.flatMap((slot) => d.entries.filter((e) => e.slot === slot));
        return (
          <section key={d.date} className="cal-day" aria-labelledby={`cal-${d.date}`}>
            <h2 className="cal-date" id={`cal-${d.date}`}>
              <time dateTime={d.date}>{formatDayShort(d.date, locale)}</time>
            </h2>
            {entries.length === 0 ? (
              <p className="muted cal-none">{t("earnings.none")}</p>
            ) : (
              <ul className="cal-list">
                {entries.map((e) => (
                  <li key={e.ticker} className={`cal-entry cal-${e.status}`}>
                    <Link className="cal-ticker" href={`/${locale}/t/${encodeURIComponent(e.ticker)}`}>
                      {e.ticker}
                    </Link>
                    <span className="cal-slot">{slotLabel[e.slot]}</span>
                    {e.status === "reported" && (
                      <>
                        <span className="num up" aria-hidden="true">
                          ✓
                        </span>
                        <span className="sr-only">{statusLabel.reported}</span>
                      </>
                    )}
                    {e.status === "unconfirmed" && <span className="sr-only">{statusLabel.unconfirmed}</span>}
                    {e.postSlug && (
                      <Link className="cal-read" href={`/${locale}/p/${e.postSlug}`}>
                        {t("earnings.readCheck")} →
                      </Link>
                    )}
                  </li>
                ))}
              </ul>
            )}
          </section>
        );
      })}
    </div>
  );
}

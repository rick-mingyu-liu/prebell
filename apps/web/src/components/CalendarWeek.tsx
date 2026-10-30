import Link from "next/link";
import TickerLogo from "./TickerLogo";
import type { Locale } from "../i18n/routing";
import type { CalendarDay, CalendarEntry, Slot } from "../lib/earnings";
import { formatDayShort } from "../lib/time";
import { translator } from "../lib/translator";

const SLOTS: Slot[] = ["bmo", "amc", "unknown"];

/** One week of report dates as ticker chips by slot; the status is written on each chip, not shown by color alone. */
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
      {days.map((d) => (
        <section key={d.date} className="cal-day" aria-labelledby={`cal-${d.date}`}>
          <h2 className="cal-date" id={`cal-${d.date}`}>
            <time dateTime={d.date}>{formatDayShort(d.date, locale)}</time>
          </h2>
          {d.entries.length === 0 ? (
            <p className="muted cal-none">{t("earnings.none")}</p>
          ) : (
            <div className="cal-slots">
              {SLOTS.map((slot) => {
                const entries = d.entries.filter((e) => e.slot === slot);
                return (
                  entries.length > 0 && (
                    <div key={slot} className="cal-slot">
                      <h3 className="lbl">{slotLabel[slot]}</h3>
                      <ul className="cal-chips">
                        {entries.map((e) => (
                          <li key={e.ticker}>
                            <Link
                              className={`cal-chip cal-${e.status}`}
                              href={
                                e.postSlug
                                  ? `/${locale}/p/${e.postSlug}`
                                  : `/${locale}/t/${encodeURIComponent(e.ticker)}`
                              }
                            >
                              <TickerLogo ticker={e.ticker} />
                              <b>{e.ticker}</b>
                              <span className={`badge badge-${e.status}`}>{statusLabel[e.status]}</span>
                            </Link>
                          </li>
                        ))}
                      </ul>
                    </div>
                  )
                );
              })}
            </div>
          )}
        </section>
      ))}
    </div>
  );
}

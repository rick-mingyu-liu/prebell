import type { Locale } from "../i18n/routing";
import type { MacroEvent } from "../lib/events";
import { formatEtTime, formatMonthDay } from "../lib/time";
import { translator } from "../lib/translator";

/**
 * A calendar day's "Economy & Fed" group, above its earnings: each event's ET time (none for all-day
 * events), title, a "Major" mark for tier 1, its status, and the official link (the release once out,
 * else the schedule that lists the date). Colour carries nothing alone: the major mark and the
 * statuses are words.
 */
export default function EconomyEvents({ events, locale }: { events: MacroEvent[]; locale: Locale }) {
  if (events.length === 0) return null;
  const t = translator(locale);
  return (
    <div className="cal-econ">
      <h3 className="cal-slot">{t("events.group")}</h3>
      <ul className="cal-econ-list">
        {events.map((e) => (
          <li key={e.id} className={`cal-event cal-event-${e.status}`}>
            {e.startsAt && <span className="cal-event-time num">{formatEtTime(e.startsAt, locale)}</span>}
            <span className="cal-event-title">{e.title}</span>
            {e.tier === 1 && <span className="cal-event-major">{t("events.major")}</span>}
            {e.status === "released" && (
              <>
                <span className="num up" aria-hidden="true">
                  ✓
                </span>
                <span className="sr-only">{t("events.released")}</span>
              </>
            )}
            {e.status === "postponed" && <span className="cal-event-flag">{t("events.postponed")}</span>}
            {e.status === "moved" && e.movedFrom && (
              <span className="cal-event-flag">{t("events.movedFrom", { date: formatMonthDay(e.movedFrom, locale) })}</span>
            )}
            {e.note && <span className="cal-event-note">{e.note}</span>}
            <a className="cal-read" href={e.releaseUrl ?? e.proofUrl} rel="noopener noreferrer">
              {e.releaseUrl ? t("events.release") : t("events.schedule")} ↗
            </a>
          </li>
        ))}
      </ul>
    </div>
  );
}

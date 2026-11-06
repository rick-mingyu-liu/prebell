import Link from "next/link";
import type { Locale } from "../i18n/routing";
import type { CalendarDay, CalendarEntry, Slot } from "../lib/earnings";
import { formatDayShort } from "../lib/time";
import { translator } from "../lib/translator";

const SLOTS: Slot[] = ["bmo", "amc", "unknown"];
/** Rows a slot shows before the rest fold into "Show N more" (a busy day has 60 reports). */
export const ROWS_PER_SLOT = 12;

type T = ReturnType<typeof translator>;

/**
 * One week as ruled day rows: the date, then each slot (before open, after close, time n/a) with its
 * reports, write-up companies first. Each report is a ticker link, the company name, and either the
 * check or, once reported, the SEC filing. Colour carries nothing alone: a reported entry gets the one
 * coloured mark, a tick, and an unconfirmed one is muted; both are also written as text for screen
 * readers. Long slots fold into a native <details>, so the page needs no client script.
 */
export default function CalendarWeek({ days, locale }: { days: CalendarDay[]; locale: Locale }) {
  const t = translator(locale);
  const slotLabel: Record<Slot, string> = {
    bmo: t("earnings.bmo"),
    amc: t("earnings.amc"),
    unknown: t("earnings.timeUnknown"),
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
                if (entries.length === 0) return null;
                const shown = entries.slice(0, ROWS_PER_SLOT);
                const rest = entries.slice(ROWS_PER_SLOT);
                return (
                  <div key={slot} className="cal-slot-group">
                    <h3 className="cal-slot">
                      {slotLabel[slot]} <span className="num">{entries.length}</span>
                    </h3>
                    <Entries entries={shown} locale={locale} t={t} />
                    {rest.length > 0 && (
                      <details className="cal-more">
                        <summary>{t("earnings.showMore", { count: rest.length })}</summary>
                        <Entries entries={rest} locale={locale} t={t} />
                      </details>
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </section>
      ))}
    </div>
  );
}

function Entries({ entries, locale, t }: { entries: CalendarEntry[]; locale: Locale; t: T }) {
  return (
    <ul className="cal-list">
      {entries.map((e) => (
        <li key={e.ticker} className={`cal-entry cal-${e.status}`}>
          <Link className="cal-ticker" href={`/${locale}/t/${encodeURIComponent(e.ticker)}`}>
            {e.ticker}
          </Link>
          {e.name && <span className="cal-name">{e.name}</span>}
          {e.status === "reported" && (
            <>
              <span className="num up" aria-hidden="true">
                ✓
              </span>
              <span className="sr-only">{t("earnings.reported")}</span>
            </>
          )}
          {e.status === "unconfirmed" && <span className="sr-only">{t("earnings.unconfirmed")}</span>}
          {e.postSlug ? (
            <Link
              className="cal-read"
              href={`/${locale}/p/${e.postSlug}`}
              aria-label={`${t("earnings.readCheck")}: ${e.ticker}`}
            >
              {t("earnings.readCheck")} →
            </Link>
          ) : (
            e.filingUrl && (
              <a
                className="cal-read"
                href={e.filingUrl}
                rel="noopener noreferrer"
                aria-label={`${t("earnings.secFiling")}: ${e.ticker}`}
              >
                {t("earnings.secFiling")} ↗
              </a>
            )
          )}
        </li>
      ))}
    </ul>
  );
}

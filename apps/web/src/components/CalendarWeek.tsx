import Link from "next/link";
import type { Locale } from "../i18n/routing";
import type { CalendarDay, CalendarEntry, Slot } from "../lib/earnings";
import type { MacroEvent } from "../lib/events";
import { companyLogo } from "../lib/logos";
import { formatDayShort } from "../lib/time";
import { translator } from "../lib/translator";
import EconomyEvents from "./EconomyEvents";

const SLOTS: Slot[] = ["bmo", "amc", "unknown"];
/** Text rows (calendar-only companies) a slot shows before the rest fold into "Show N more" (a busy day has 60 reports). */
export const ROWS_PER_SLOT = 12;
/** Logos loaded eagerly: the first rows of tiles on the page (roughly the first screen); the rest are lazy. */
export const EAGER_LOGOS = 6;

type T = ReturnType<typeof translator>;

/**
 * One week as ruled day rows: the date, then each slot (before open, after close, time n/a) with its
 * reports. Write-up companies lead each slot as a grid of logo tiles (the ticker as a text tile when
 * there is no logo), linking to the write-up when there is one, else the ticker page; the calendar-only
 * rest follow as compact text rows: a ticker link, the company name, and the SEC filing once reported.
 * Colour carries nothing alone: a reported entry gets the one coloured mark, a tick, and an
 * unconfirmed one is muted; both are also written as text for screen readers. Long lists of text rows
 * fold into a native <details>, so the page needs no client script.
 */
export default function CalendarWeek({
  days,
  locale,
  events = {},
}: {
  days: CalendarDay[];
  locale: Locale;
  /** The week's macro events by date, shown above each day's earnings. */
  events?: Record<string, MacroEvent[]>;
}) {
  const t = translator(locale);
  const slotLabel: Record<Slot, string> = {
    bmo: t("earnings.bmo"),
    amc: t("earnings.amc"),
    unknown: t("earnings.timeUnknown"),
  };
  // Tiles render in page order, so the first EAGER_LOGOS logos on the page are the ones above the fold.
  const eager = new Set(
    days
      .flatMap((d) => SLOTS.flatMap((slot) => d.entries.filter((e) => e.slot === slot && e.writeUps)))
      .filter((e) => companyLogo(e.ticker))
      .slice(0, EAGER_LOGOS),
  );
  return (
    <div className="cal-week">
      {days.map((d) => (
        <section key={d.date} className="cal-day" aria-labelledby={`cal-${d.date}`}>
          <h2 className="cal-date" id={`cal-${d.date}`}>
            <time dateTime={d.date}>{formatDayShort(d.date, locale)}</time>
          </h2>
          <EconomyEvents events={events[d.date] ?? []} locale={locale} />
          {d.entries.length === 0 ? (
            <p className="muted cal-none">{t(events[d.date]?.length ? "earnings.noneWithEvents" : "earnings.none")}</p>
          ) : (
            <div className="cal-slots">
              {SLOTS.map((slot) => {
                const entries = d.entries.filter((e) => e.slot === slot);
                if (entries.length === 0) return null;
                const tiles = entries.filter((e) => e.writeUps);
                const rows = entries.filter((e) => !e.writeUps);
                const shown = rows.slice(0, ROWS_PER_SLOT);
                const rest = rows.slice(ROWS_PER_SLOT);
                return (
                  <div key={slot} className="cal-slot-group">
                    <h3 className="cal-slot">
                      {slotLabel[slot]} <span className="num">{entries.length}</span>
                    </h3>
                    {tiles.length > 0 && <Tiles entries={tiles} locale={locale} t={t} eager={eager} />}
                    {shown.length > 0 && <Entries entries={shown} locale={locale} t={t} />}
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

/** One write-up company: its logo (alt = the company name) or the ticker as a text tile, with the ticker beneath. */
function Tiles({ entries, locale, t, eager }: { entries: CalendarEntry[]; locale: Locale; t: T; eager: Set<CalendarEntry> }) {
  return (
    <ul className="cal-tiles">
      {entries.map((e) => {
        const logo = companyLogo(e.ticker);
        const name = e.name ?? e.ticker;
        return (
          <li key={e.ticker} className={`cal-tile cal-${e.status}`}>
            <Link
              className="cal-tile-link"
              href={e.postSlug ? `/${locale}/p/${e.postSlug}` : `/${locale}/t/${encodeURIComponent(e.ticker)}`}
            >
              <span className="cal-mark">
                {logo ? (
                  <img
                    src={logo}
                    alt={name}
                    width={48}
                    height={48}
                    loading={eager.has(e) ? "eager" : "lazy"}
                    decoding="async"
                  />
                ) : (
                  <span className="cal-chip">
                    <span className="sr-only">{name}</span>
                    {e.ticker}
                  </span>
                )}
              </span>
              <span className="cal-sym">
                {logo && <span className="cal-sym-t">{e.ticker}</span>}
                {e.status === "reported" && (
                  <span className="num up" aria-hidden="true">
                    ✓
                  </span>
                )}
              </span>
              {e.status === "reported" && <span className="sr-only">{t("earnings.reported")}</span>}
              {e.status === "unconfirmed" && <span className="sr-only">{t("earnings.unconfirmed")}</span>}
              {e.postSlug && <span className="sr-only">{t("earnings.readCheck")}</span>}
            </Link>
          </li>
        );
      })}
    </ul>
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

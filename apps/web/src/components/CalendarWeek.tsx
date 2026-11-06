import Link from "next/link";
import type { Locale } from "../i18n/routing";
import { type CalendarDay, type CalendarEntry, type Slot, todayEt } from "../lib/earnings";
import type { MacroEvent } from "../lib/events";
import { companyLogoInfo } from "../lib/logos";
import { formatBoardDay, formatWeekday } from "../lib/time";
import { translator } from "../lib/translator";
import BoardStart from "./BoardStart";
import EconomyEvents from "./EconomyEvents";

/** The two timed slots, side by side in each day's column. "Time n/a" sits under them, only when it has reports. */
const HALVES: Slot[] = ["bmo", "amc"];
/** Cards a slot shows before the rest fold into "Show N more": a column holds one card per row, so eight is a screen. */
export const CARDS_PER_SLOT = 8;
/** Width over height from which a logo is a wordmark set across the card, not an icon beside the ticker. */
export const WIDE_LOGO = 1.8;
/**
 * A wordmark's height in its card: about the same ink area for every mark, so a long one sits lower and a
 * stubby one does not shout (2:1 and wider up to 30px tall, 5:1 20px), never under 16px. The card's width caps it too.
 */
export const wordmarkHeight = (ratio: number) => Math.round(Math.min(30, Math.max(16, Math.sqrt(2000 / ratio))));
/** Logos loaded eagerly: the first cards of the before-open and after-close lists of the day the board opens on. */
export const EAGER_PER_SLOT = 1;
/** An icon's box; a smaller image keeps its own size, so no logo is drawn above its natural size. */
const ICON = 28;

type T = ReturnType<typeof translator>;

/** Write-up companies first, each group in the calendar's own order. */
const writeUpsFirst = (entries: CalendarEntry[]) => [...entries.filter((e) => e.writeUps), ...entries.filter((e) => !e.writeUps)];

/** The id of a day's column, the target of the day chips and of the phone's start-at-today scroll. */
export const dayId = (date: string) => `d-${date}`;

/**
 * One week as a board of day columns, Monday to Friday. Each column is a labelled region: the day, the
 * day's economy & Fed events as a strip, then before open and after close side by side (stacked when the
 * column is narrow), and time n/a under them when it has reports. Every company is a wide card: its logo
 * (a wordmark across the card, or a square icon beside the ticker), or the ticker and name set in type.
 * A card links to the write-up when there is one, else the ticker page (which carries the SEC filing).
 * Write-up companies lead each list; long lists fold into a native <details>.
 * Colour carries nothing alone: reported gets the one coloured mark, a tick, unconfirmed a dashed, muted
 * card, and both are also written as text. Today's column is marked in words; past columns are quieter.
 * On a phone the columns are a sideways scroll-snap row, one day per screen, with day chips to jump.
 */
export default function CalendarWeek({
  days,
  locale,
  events = {},
  today = todayEt(),
}: {
  days: CalendarDay[];
  locale: Locale;
  /** The week's macro events by date, shown at the top of each day's column. */
  events?: Record<string, MacroEvent[]>;
  /** Today's US Eastern date, which decides the highlighted and the past columns. */
  today?: string;
}) {
  const t = translator(locale);
  const slotLabel: Record<Slot, string> = {
    bmo: t("earnings.bmo"),
    amc: t("earnings.amc"),
    unknown: t("earnings.timeUnknown"),
  };
  const lists = new Map(days.map((d) => [d.date, slotLists(d.entries)]));
  const hasToday = days.some((d) => d.date === today);
  // The phone opens on today's column (else Monday): the top logos there are on the first screen. The rest
  // load lazily, which on a desktop, where the whole week shows, still means at once.
  const start = lists.get(hasToday ? today : days[0]?.date) ?? null;
  const eager = new Set(
    start ? HALVES.flatMap((slot) => start[slot].filter((e) => companyLogoInfo(e.ticker)).slice(0, EAGER_PER_SLOT)) : [],
  );

  return (
    <div className="cal-week">
      <nav className="cal-chips" aria-label={t("earnings.days")}>
        {days.map((d) => (
          <a key={d.date} className="cal-chip-day" href={`#${dayId(d.date)}`} aria-current={d.date === today ? "date" : undefined}>
            <span>{formatWeekday(d.date, locale)}</span> <span className="num">{Number(d.date.slice(8))}</span>
          </a>
        ))}
      </nav>
      <div className="cal-board">
        {days.map((d) => {
          const when = d.date === today ? "is-today" : d.date < today ? "is-past" : "is-ahead";
          const dayEvents = events[d.date] ?? [];
          const slots = lists.get(d.date)!;
          return (
            <section key={d.date} id={dayId(d.date)} className={`cal-col ${when}`} aria-labelledby={`cal-${d.date}`}>
              <h2 className="cal-date" id={`cal-${d.date}`}>
                <time dateTime={d.date}>{formatBoardDay(d.date, locale)}</time>
                {when === "is-today" && <span className="cal-today">{t("earnings.today")}</span>}
                {when === "is-past" && <span className="sr-only">{t("earnings.past")}</span>}
              </h2>
              <EconomyEvents events={dayEvents} locale={locale} />
              {d.entries.length === 0 ? (
                <p className="muted cal-none">{t(dayEvents.length ? "earnings.noneWithEvents" : "earnings.none")}</p>
              ) : (
                <>
                  <div className="cal-halves">
                    {HALVES.map((slot) => (
                      <SlotList key={slot} slot={slot} label={slotLabel[slot]} entries={slots[slot]} locale={locale} t={t} eager={eager} />
                    ))}
                  </div>
                  {slots.unknown.length > 0 && (
                    <SlotList slot="unknown" label={slotLabel.unknown} entries={slots.unknown} locale={locale} t={t} eager={eager} />
                  )}
                </>
              )}
            </section>
          );
        })}
      </div>
      {hasToday && <BoardStart target={dayId(today)} />}
    </div>
  );
}

function slotLists(entries: CalendarEntry[]): Record<Slot, CalendarEntry[]> {
  const of = (slot: Slot) => writeUpsFirst(entries.filter((e) => e.slot === slot));
  return { bmo: of("bmo"), amc: of("amc"), unknown: of("unknown") };
}

function SlotList({
  slot,
  label,
  entries,
  locale,
  t,
  eager,
}: {
  slot: Slot;
  label: string;
  entries: CalendarEntry[];
  locale: Locale;
  t: T;
  eager: Set<CalendarEntry>;
}) {
  const shown = entries.slice(0, CARDS_PER_SLOT);
  const rest = entries.slice(CARDS_PER_SLOT);
  return (
    <div className={`cal-half cal-half-${slot}`}>
      <h3 className="cal-slot">
        <SlotIcon slot={slot} />
        {label} <span className="num">{entries.length}</span>
      </h3>
      {entries.length === 0 ? (
        <p className="cal-empty">
          <span aria-hidden="true">—</span>
          <span className="sr-only">{t("earnings.emptySlot")}</span>
        </p>
      ) : (
        <Cards entries={shown} locale={locale} t={t} eager={eager} />
      )}
      {rest.length > 0 && (
        <details className="cal-more">
          <summary>{t("earnings.showMore", { count: rest.length })}</summary>
          <Cards entries={rest} locale={locale} t={t} eager={eager} />
        </details>
      )}
    </div>
  );
}

/** A small sun for before the open, a moon for after the close, a clock face for time n/a; decoration only. */
function SlotIcon({ slot }: { slot: Slot }) {
  return (
    <svg className="cal-slot-icon" viewBox="0 0 16 16" width="12" height="12" aria-hidden="true" focusable="false">
      {slot === "bmo" ? (
        <g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
          <circle cx="8" cy="8" r="3" />
          <path d="M8 1v1.5M8 13.5V15M1 8h1.5M13.5 8H15M3 3l1 1M12 12l1 1M3 13l1-1M12 4l1-1" />
        </g>
      ) : slot === "amc" ? (
        <path fill="currentColor" d="M6.2 1.6a6.6 6.6 0 1 0 8.2 8.2A5.4 5.4 0 0 1 6.2 1.6Z" />
      ) : (
        <g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
          <circle cx="8" cy="8" r="6" />
          <path d="M8 4.5V8l2.5 1.5" />
        </g>
      )}
    </svg>
  );
}

/**
 * One company per card. A wide logo (a wordmark) fills the card and carries the name as its alt; a square
 * one sits beside the ticker and name. Without a logo the ticker and name are set in type. The name is the
 * link's accessible name (the ticker beside it is hidden from screen readers when there is a name).
 */
function Cards({ entries, locale, t, eager }: { entries: CalendarEntry[]; locale: Locale; t: T; eager: Set<CalendarEntry> }) {
  return (
    <ul className="cal-cards">
      {entries.map((e) => {
        const logo = companyLogoInfo(e.ticker);
        const name = e.name ?? e.ticker;
        const ratio = logo ? logo.width / logo.height : 0;
        const wide = ratio >= WIDE_LOGO;
        // Never above the natural size: a wordmark no taller than its own height, an icon no bigger than itself.
        const size = !logo
          ? undefined
          : wide
            ? { height: Math.min(wordmarkHeight(ratio), logo.height) }
            : Math.max(logo.width, logo.height) < ICON
              ? { width: logo.width, height: logo.height }
              : undefined;
        const kind = logo ? (wide ? "wide" : "icon") : "text";
        const img = logo && (
          <img
            className={wide ? "cal-logo-wide" : "cal-logo-icon"}
            src={logo.src}
            alt={wide ? name : ""}
            width={logo.width}
            height={logo.height}
            style={size}
            loading={eager.has(e) ? "eager" : "lazy"}
            decoding="async"
          />
        );
        return (
          <li key={e.ticker} className={`cal-card cal-card-${kind} cal-${e.status}`}>
            <Link
              className="cal-card-link"
              href={e.postSlug ? `/${locale}/p/${e.postSlug}` : `/${locale}/t/${encodeURIComponent(e.ticker)}`}
              title={e.name ?? undefined}
            >
              {img}
              {kind !== "wide" && (
                <span className="cal-card-body">
                  <span className="cal-card-sym" aria-hidden={e.name ? "true" : undefined}>
                    {e.ticker}
                  </span>
                  {e.name && <span className="cal-card-name">{e.name}</span>}
                </span>
              )}
              {e.status === "reported" && (
                <span className="cal-card-tick" aria-hidden="true">
                  ✓
                </span>
              )}
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

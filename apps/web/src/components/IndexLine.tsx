import { Fragment } from "react";
import type { Locale } from "@/i18n/routing";
import { fmtPrice } from "@/lib/format";
import { indexName } from "@/lib/indexnames";
import type { IndexLineView } from "@/lib/indexline";
import { formatEtDateTime } from "@/lib/time";
import { translator } from "@/lib/translator";

const ARROW = { up: "▲", down: "▼", flat: "" } as const;

/** The divider between items: one collapsible space, painted as a hairline in the stylesheet and dropped by the browser where the line wraps. */
const Sep = () => <span className="ix-sep" aria-hidden="true"> </span>;

/**
 * One quiet line under the masthead: each index (or future) with its level and the day's move, then
 * the as-of time (with the date when the snapshot is from an earlier Eastern day). The arrow carries the direction (and hidden text for screen readers), so the
 * percentage is shown unsigned. Renders
 * nothing when the generator has not written a snapshot yet.
 */
export default function IndexLine({ line, locale }: { line: IndexLineView | null; locale: Locale }) {
  if (!line) return null;
  const t = translator(locale);
  const time = formatEtDateTime(line.asOfUtc, locale);
  return (
    <p className="index-line">
      {line.entries.map((e, i) => (
        <Fragment key={e.symbol}>
          {i > 0 && <Sep />}
          <span className="ix">
            {indexName(t, e.symbol)} {fmtPrice(e.last)}{" "}
            <span className={`num ${e.dir}`}>
              {e.dir !== "flat" && (
                <>
                  <span aria-hidden="true">{ARROW[e.dir]}</span>
                  <span className="sr-only">{t(`paper.index.${e.dir}`)} </span>
                </>
              )}
              {fmtPrice(Math.abs(e.changePct))}%
            </span>
          </span>
        </Fragment>
      ))}
      {time && (
        <>
          <Sep />
          <time className="ix ix-asof" dateTime={line.asOfUtc}>
            {t("paper.index.asOf", { time })}
          </time>
        </>
      )}
    </p>
  );
}

import type { Locale } from "@/i18n/routing";
import { fmtPrice } from "@/lib/format";
import { indexName } from "@/lib/indexnames";
import type { IndexLineView } from "@/lib/indexline";
import { formatEtTime } from "@/lib/time";
import { translator } from "@/lib/translator";

const ARROW = { up: "▲", down: "▼", flat: "" } as const;

/**
 * One quiet line under the masthead: each index (or future) with its level and the day's move, then
 * the as-of time. The arrow carries the direction, so the percentage is shown unsigned. Renders
 * nothing when the generator has not written a snapshot yet.
 */
export default function IndexLine({ line, locale }: { line: IndexLineView | null; locale: Locale }) {
  if (!line) return null;
  const t = translator(locale);
  const time = formatEtTime(line.asOfUtc, locale);
  return (
    <p className="index-line">
      {line.entries.map((e) => (
        <span className="ix" key={e.symbol}>
          {indexName(t, e.symbol)} {fmtPrice(e.last)}{" "}
          <span className={`num ${e.dir}`}>
            {ARROW[e.dir]}
            {fmtPrice(Math.abs(e.changePct))}%
          </span>
        </span>
      ))}
      {time && (
        <time className="ix ix-asof" dateTime={line.asOfUtc}>
          {t("paper.index.asOf", { time })}
        </time>
      )}
    </p>
  );
}

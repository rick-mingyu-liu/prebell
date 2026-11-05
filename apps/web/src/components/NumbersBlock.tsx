import type { Locale } from "../i18n/routing";
import type { FactsView, IndexView } from "../lib/facts";
import { fmtPct, fmtPoint, fmtPrice } from "../lib/format";
import { indexName } from "../lib/indexnames";
import { formatEtTime } from "../lib/time";
import { translator } from "../lib/translator";

/** The lead index as a big number with its point change and percent; a Missing value is a grey dash. */
function Lead({ index, name, time, asOf }: { index: IndexView; name: string; time: string; asOf: string }) {
  const q = index.quote;
  return (
    <>
      <p className="lbl">
        {name}
        {time && " · "}
        {time && <time dateTime={asOf}>{time}</time>}
      </p>
      {q ? (
        <>
          <div className="big">{fmtPrice(q.last)}</div>
          <div className={`num ${q.dir}`}>
            {fmtPoint(q.pointChange, q.dir)} ({fmtPct(q.changePct)})
          </div>
        </>
      ) : (
        <div className="big num flat">—</div>
      )}
    </>
  );
}

function Tile({ index, name }: { index: IndexView; name: string }) {
  const q = index.quote;
  return (
    <div className="tile">
      <div className="k">{name}</div>
      <div className={q ? "v" : "v muted"}>{q ? fmtPrice(q.last) : "—"}</div>
      <div className={`num ${q ? q.dir : "flat"}`}>{q ? fmtPct(q.changePct) : "—"}</div>
    </div>
  );
}

function IndexRow({ index, name }: { index: IndexView; name: string }) {
  const q = index.quote;
  return (
    <div className="row">
      <span>{name}</span>
      <span className="row-end">
        <span className={q ? "v" : "v muted"}>{q ? fmtPrice(q.last) : "—"}</span>
        <span className={`num ${q ? q.dir : "flat"}`}>{q ? fmtPct(q.changePct) : "—"}</span>
      </span>
    </div>
  );
}

/**
 * The index numbers of a brief: the lead as a big number, a reserved chart
 * area, and the other three as tiles (home) or rows (brief page). Index names
 * come from the quote's symbol; the time is `asOfUtc` in US Eastern.
 */
export default function NumbersBlock({
  facts,
  variant,
  locale,
}: {
  facts: FactsView;
  variant: "home" | "page";
  locale: Locale;
}) {
  const t = translator(locale);
  const name = (i: IndexView) => indexName(t, i.quote?.symbol ?? i.symbol);
  const Item = variant === "home" ? Tile : IndexRow;
  return (
    <div className={`numbers numbers-${variant}`}>
      <div className="numbers-lead">
        <Lead index={facts.lead} name={name(facts.lead)} time={formatEtTime(facts.asOfUtc, locale)} asOf={facts.asOfUtc} />
        <div className="chart-slot" aria-hidden="true"></div>
      </div>
      <div className={variant === "home" ? "tiles" : "index-rows"}>
        {facts.others.map((i) => (
          <Item key={i.symbol} index={i} name={name(i)} />
        ))}
      </div>
    </div>
  );
}

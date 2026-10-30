import type { Locale } from "../i18n/routing";
import type { FactsView, IndexView } from "../lib/facts";
import { fmtPct, fmtPoint, fmtPrice } from "../lib/format";
import { formatEtTime } from "../lib/time";
import { translator } from "../lib/translator";

/** `=` and `^` are awkward in message keys, so symbols map to `index.*` keys here. */
type NameKey = "index.es" | "index.nq" | "index.ym" | "index.rty" | "index.gspc" | "index.ndx" | "index.ixic" | "index.dji" | "index.rut";

const NAME_KEY: Record<string, NameKey> = {
  "ES=F": "index.es",
  "NQ=F": "index.nq",
  "YM=F": "index.ym",
  "RTY=F": "index.rty",
  "^GSPC": "index.gspc",
  "^NDX": "index.ndx",
  "^IXIC": "index.ixic",
  "^DJI": "index.dji",
  "^RUT": "index.rut",
};

type Translate = ReturnType<typeof translator>;

function indexName(t: Translate, symbol: string): string {
  const key = NAME_KEY[symbol];
  return key ? t(key) : symbol;
}

/** The lead index as a big number with its point change and percent; a Missing value is a grey dash. */
function Lead({ index, name, time }: { index: IndexView; name: string; time: string }) {
  const q = index.quote;
  return (
    <>
      <p className="lbl">
        {name}
        {time && " · "}
        {time && <time>{time}</time>}
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
      <div className="v">{q ? fmtPrice(q.last) : "—"}</div>
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
        <span className="v">{q ? fmtPrice(q.last) : "—"}</span>
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
      <Lead index={facts.lead} name={name(facts.lead)} time={formatEtTime(facts.asOfUtc, locale)} />
      <div className="chart-slot" aria-hidden="true"></div>
      <div className={variant === "home" ? "tiles" : "index-rows"}>
        {facts.others.map((i) => (
          <Item key={i.symbol} index={i} name={name(i)} />
        ))}
      </div>
    </div>
  );
}

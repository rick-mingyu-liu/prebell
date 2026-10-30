import Pill from "./Pill";
import type { Locale } from "../i18n/routing";
import type { MoverView } from "../lib/facts";
import { fmtPrice } from "../lib/format";
import { translator } from "../lib/translator";

/** Mover reasons are English-only, so ZH pages show no reason line. */
export default function MoversList({
  movers,
  locale,
  limit,
  showPrice = false,
}: {
  movers: MoverView[];
  locale: Locale;
  limit?: number;
  showPrice?: boolean;
}) {
  const shown = limit === undefined ? movers : movers.slice(0, limit);
  if (shown.length === 0) return null;
  const t = translator(locale);
  return (
    <ul className="movers">
      {shown.map((m) => (
        <li key={m.symbol} className="row">
          <div className="mover-main">
            <div>
              <b>{m.symbol}</b> <span className="muted">{m.name}</span>
            </div>
            {locale === "en" && (
              <div className="why">{m.reason || t("movers.noNews")}</div>
            )}
          </div>
          <div className="mover-end">
            {showPrice && <span className="num">${fmtPrice(m.quote.last)}</span>}
            <Pill pct={m.quote.changePct} />
          </div>
        </li>
      ))}
    </ul>
  );
}

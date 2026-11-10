import Link from "next/link";
import type { ReactNode } from "react";
import type { Locale } from "../i18n/routing";
import type { MoverView } from "../lib/facts";
import { fmtPct, fmtPrice } from "../lib/format";
import { translator } from "../lib/translator";

export interface MoversTableProps {
  movers: MoverView[];
  locale: Locale;
  /** `column`: ruled rows for the front page's side column. `table`: the brief's ruled table. */
  variant: "column" | "table";
  /** Show the price before the change. */
  showPrice?: boolean;
  /** The price column's heading: the last trade (premarket) or the close (recap). */
  priceLabel?: "last" | "close";
}

/**
 * The reason (a news summary, or the generator's fallback) in grey, linked to the site's earnings check or to its
 * source when that is an http(s) URL; "No clear news" in grey italics without one.
 */
function why(m: MoverView, noNews: string, locale: Locale): ReactNode {
  if (!m.reason) return <i>{noNews}</i>;
  if (m.reasonPost) return <Link href={`/${locale}/p/${m.reasonPost}`}>{m.reason}</Link>;
  if (!m.reasonUrl || !/^https?:\/\//i.test(m.reasonUrl)) return m.reason;
  return (
    <a href={m.reasonUrl} rel="nofollow noopener" target="_blank">
      {m.reason}
    </a>
  );
}

const pct = (m: MoverView) => <span className={`num ${m.quote.dir}`}>{fmtPct(m.quote.changePct)}</span>;

/**
 * Movers as ruled rows or a ruled table: company name, ticker in grey, the move in the number colours and
 * the one-line reason. Reasons are English-only, so ZH pages show no reason line or column. No logos, no pills.
 */
export default function MoversTable({ movers, locale, variant, showPrice = false, priceLabel = "last" }: MoversTableProps) {
  if (movers.length === 0) return null;
  const t = translator(locale);
  const reasons = locale === "en";
  const noNews = t("movers.noNews");

  if (variant === "column") {
    return (
      <ol className="movers-col">
        {movers.map((m) => (
          <li key={m.symbol} className="mover-row">
            <p className="mover-head">
              <span className="mover-co">
                <b className="mover-name">{m.name}</b> <span className="mover-ticker muted">{m.symbol}</span>
              </span>
              <span className="mover-move">
                {showPrice && <span className="mover-price">${fmtPrice(m.quote.last)}</span>}
                {pct(m)}
              </span>
            </p>
            {reasons && <p className="mover-why muted">{why(m, noNews, locale)}</p>}
          </li>
        ))}
      </ol>
    );
  }

  return (
    <div className="table-wrap">
      <table className="table movers-table">
        <thead>
          <tr>
            <th scope="col">{t("movers.company")}</th>
            {reasons && <th scope="col">{t("movers.reason")}</th>}
            {showPrice && (
              <th scope="col" className="r">
                {t(`movers.${priceLabel}`)}
              </th>
            )}
            <th scope="col" className="r">
              {t("movers.change")}
            </th>
          </tr>
        </thead>
        <tbody>
          {movers.map((m) => (
            <tr key={m.symbol}>
              <td>
                <b className="mover-name">{m.name}</b> <span className="mover-ticker muted">{m.symbol}</span>
              </td>
              {reasons && <td className="mover-why muted">{why(m, noNews, locale)}</td>}
              {showPrice && <td className="r num">${fmtPrice(m.quote.last)}</td>}
              <td className="r">{pct(m)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

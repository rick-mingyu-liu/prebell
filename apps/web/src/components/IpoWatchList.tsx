import Link from "next/link";
import type { Locale } from "../i18n/routing";
import type { IpoSummary } from "../lib/ipo";
import { formatDate } from "../lib/time";
import { translator } from "../lib/translator";

/**
 * The calendar page's "IPO watch": one line per tracked IPO, its name linking to its page, the symbol, the status
 * in words and the first trading day (marked unconfirmed until a filing or the first trade confirms it) or "date
 * not set". Nothing renders when no IPO is tracked.
 */
export default function IpoWatchList({ ipos, locale }: { ipos: IpoSummary[]; locale: Locale }) {
  if (ipos.length === 0) return null;
  const t = translator(locale);
  return (
    <section className="ipo-watch" aria-labelledby="ipo-watch">
      <h2 className="cal-slot" id="ipo-watch">
        {t("ipo.watchTitle")}
      </h2>
      <ul className="ipo-watch-list">
        {ipos.map((i) => (
          <li key={i.slug} className="ipo-watch-row">
            <Link className="ipo-watch-name" href={`/${locale}/ipo/${i.slug}`}>
              {i.name}
            </Link>
            {i.symbol && <span className="ipo-watch-sym">{i.symbol}</span>}
            <span className="ipo-watch-status">{t(`ipo.status.${i.status}`)}</span>
            <span className="ipo-watch-date">
              {i.date ? (
                <>
                  <time dateTime={i.date.date}>{formatDate(i.date.date, locale)}</time>
                  {i.date.status !== "confirmed" && <span className="ipo-flag">{t("earnings.unconfirmed")}</span>}
                </>
              ) : (
                t("ipo.dateNotSet")
              )}
            </span>
          </li>
        ))}
      </ul>
    </section>
  );
}

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
    <section className="cal-ipo-watch" aria-labelledby="cal-ipo-watch">
      <h2 className="cal-slot" id="cal-ipo-watch">
        {t("ipo.watchTitle")}
      </h2>
      <ul className="cal-ipo-watch-list">
        {ipos.map((i) => (
          <li key={i.slug} className="cal-ipo-watch-row">
            <Link className="cal-ipo-watch-name" href={`/${locale}/ipo/${i.slug}`}>
              {i.name}
            </Link>
            {i.symbol && <span className="cal-ipo-watch-sym">{i.symbol}</span>}
            <span className="cal-ipo-watch-status">{t(`ipo.status.${i.status}`)}</span>
            <span className="cal-ipo-watch-date">
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

import PartnerLinks from "./PartnerLinks";
import type { Locale } from "../i18n/routing";
import { TERMPOCKET_URL } from "../lib/partners";
import { translator } from "../lib/translator";

/**
 * Right column of `.grid-portal` / `.grid-article`. Only the TermPocket card is server-rendered;
 * the partner boxes and their disclosure come from the client island, below the card so nothing
 * above them moves when they load.
 */
export default function Sidebar({ locale }: { locale: Locale }) {
  const t = translator(locale);
  return (
    <aside className="sidebar">
      <div className="tp-card">
        <p className="tp-name">{t("termpocket.name")}</p>
        <p>{t("termpocket.copy")}</p>
        <a className="tp-cta" href={TERMPOCKET_URL} target="_blank" rel="noopener">
          {t("termpocket.cta")}
        </a>
      </div>
      <PartnerLinks
        labels={{
          brokers: t("partners.brokers"),
          crypto: t("partners.crypto"),
          disclosure: t("partners.disclosure"),
        }}
      />
    </aside>
  );
}

import PartnerLinks from "./PartnerLinks";
import type { Locale } from "../i18n/routing";
import { TERMPOCKET_URL } from "../lib/partners";
import { translator } from "../lib/translator";

/** Right column of `.grid-portal` / `.grid-article`. Only the disclosure and TermPocket are server-rendered. */
export default function Sidebar({ locale }: { locale: Locale }) {
  const t = translator(locale);
  return (
    <aside className="sidebar">
      <PartnerLinks titles={{ brokers: t("partners.brokers"), crypto: t("partners.crypto") }} />
      <p className="side-disc">{t("partners.disclosure")}</p>
      <div className="tp-card">
        <p className="tp-name">{t("termpocket.name")}</p>
        <p>{t("termpocket.copy")}</p>
        <a className="tp-cta" href={TERMPOCKET_URL} target="_blank" rel="noopener">
          {t("termpocket.cta")}
        </a>
      </div>
    </aside>
  );
}

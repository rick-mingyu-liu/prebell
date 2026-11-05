import type { Locale } from "../i18n/routing";
import { translator } from "../lib/translator";
import PartnerLinks from "./PartnerLinks";

/**
 * One line above the footer: the label, the partner names (a client island, fetched per visitor) and
 * the disclosure at the right. The label and note are server-rendered; CSS hides the whole strip
 * until the island has put at least one link in it, so visitors with no links never see a bare label.
 */
export default function PartnersStrip({ locale }: { locale: Locale }) {
  const t = translator(locale);
  return (
    <aside className="partners-strip" aria-label={t("partners.from")}>
      <span className="partners-label">{t("partners.from")}</span>
      <PartnerLinks
        variant="strip"
        labels={{ brokers: t("partners.brokers"), crypto: t("partners.crypto"), disclosure: t("partners.disclosure") }}
      />
      <p className="partners-note">{t("partners.affiliateNote")}</p>
    </aside>
  );
}

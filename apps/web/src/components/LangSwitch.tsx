import { htmlLang, type Locale } from "@/i18n/routing";
import { translator } from "@/lib/translator";
import LangSwitchLink from "./LangSwitchLink";

/** Link to the same page in the other language, for the shared masthead and footer; it follows the current URL on the client. */
export default function LangSwitch({ locale, className = "lang-switch" }: { locale: Locale; className?: string }) {
  const t = translator(locale);
  const target: Locale = locale === "en" ? "zh" : "en";
  return (
    <LangSwitchLink
      target={target}
      hrefLang={htmlLang(target)}
      label={t("site.otherLang")}
      title={t("site.otherLangLabel")}
      className={className}
    />
  );
}

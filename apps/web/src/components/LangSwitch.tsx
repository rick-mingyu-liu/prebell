import { getTranslations } from "next-intl/server";
import { htmlLang, type Locale } from "@/i18n/routing";
import LangSwitchLink from "./LangSwitchLink";

/** Pill link to the same page in the other language, for the shared header and footer; it follows the current URL on the client. */
export default async function LangSwitch({ locale, className = "lang-pill" }: { locale: Locale; className?: string }) {
  const t = await getTranslations("site");
  const target: Locale = locale === "en" ? "zh" : "en";
  return (
    <LangSwitchLink
      target={target}
      hrefLang={htmlLang(target)}
      label={t("otherLang")}
      title={t("otherLangLabel")}
      className={className}
    />
  );
}

import Link from "next/link";
import { getTranslations } from "next-intl/server";
import { htmlLang, type Locale } from "@/i18n/routing";
import LangSwitchLink from "./LangSwitchLink";

/**
 * Pill link to the same page in the other language; `path` is the part after the locale.
 * Without `path` (the shared header and footer) it follows the current URL on the client.
 */
export default async function LangSwitch({
  locale,
  path,
  className = "lang-pill",
}: {
  locale: Locale;
  path?: string;
  className?: string;
}) {
  const t = await getTranslations("site");
  const target: Locale = locale === "en" ? "zh" : "en";
  if (path === undefined)
    return (
      <LangSwitchLink
        target={target}
        hrefLang={htmlLang(target)}
        label={t("otherLang")}
        title={t("otherLangLabel")}
        className={className}
      />
    );
  return (
    <Link
      className={className}
      href={`/${target}${path}`}
      hrefLang={htmlLang(target)}
      lang={htmlLang(target)}
      title={t("otherLangLabel")}
    >
      {t("otherLang")}
    </Link>
  );
}

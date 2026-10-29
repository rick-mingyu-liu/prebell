import Link from "next/link";
import { getTranslations } from "next-intl/server";
import { htmlLang, type Locale } from "@/i18n/routing";

/** Link to the same page in the other language; `path` is the part after the locale. */
export default async function LangSwitch({ locale, path }: { locale: Locale; path: string }) {
  const t = await getTranslations("site");
  const target: Locale = locale === "en" ? "zh" : "en";
  return (
    <Link
      className="lang-switch"
      href={`/${target}${path}`}
      hrefLang={htmlLang(target)}
      lang={htmlLang(target)}
      aria-label={t("otherLangLabel")}
    >
      {t("otherLang")}
    </Link>
  );
}

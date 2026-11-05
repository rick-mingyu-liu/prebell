import Link from "next/link";
import type { Locale } from "@/i18n/routing";
import { translator } from "@/lib/translator";
import LangSwitch from "./LangSwitch";

/**
 * The paper's nameplate: the name in the serif with its other-language twin beside it in small grey
 * (the Chinese edition leads with 盘前铃), the dateline, and the navigation, under a double rule.
 * `dateline` is computed by the caller so a post page can show its own edition.
 */
export default function Masthead({ locale, dateline }: { locale: Locale; dateline: string }) {
  const t = translator(locale);
  return (
    <header className="masthead">
      <Link href={`/${locale}`} className="masthead-name">
        {t("paper.masthead.main")} <small lang={locale === "en" ? "zh-Hans" : "en"}>{t("paper.masthead.sub")}</small>
      </Link>
      <p className="dateline">{dateline}</p>
      <nav className="site-nav" aria-label={t("site.navLabel")}>
        <Link href={`/${locale}/archive`}>{t("site.briefs")}</Link>
        <Link href={`/${locale}/archive?type=earnings`}>{t("site.earnings")}</Link>
        <Link href={`/${locale}/earnings`}>{t("site.calendar")}</Link>
        <Link href={`/${locale}/about`}>{t("site.about")}</Link>
        <LangSwitch locale={locale} />
      </nav>
    </header>
  );
}

import Link from "next/link";
import type { Locale } from "@/i18n/routing";
import { translator } from "@/lib/translator";
import LangSwitch from "./LangSwitch";

/**
 * The paper's nameplate: the name in the serif with its other-language twin beside it in small grey
 * (the Chinese edition leads with 盘前铃), the dateline, and the navigation, under a double rule.
 * `dateline` is computed by the caller.
 */
export default function Masthead({ locale, dateline }: { locale: Locale; dateline: string }) {
  const t = translator(locale);
  return (
    <header className="masthead">
      <Link href={`/${locale}`} className="masthead-name">
        <svg className="masthead-mark" viewBox="12 8 40 50" width="34" height="42" aria-hidden="true" focusable="false">
          <path d="M32 12c-1.9 0-3.4 1.5-3.4 3.4v1.2C22.7 18.2 19 23.7 19 30.2v7.6l-4.2 6.1c-.7 1 0 2.4 1.2 2.4h32c1.2 0 1.9-1.4 1.2-2.4L45 37.8v-7.6c0-6.5-3.7-12-9.6-13.6v-1.2c0-1.9-1.5-3.4-3.4-3.4z" fill="none" stroke="currentColor" strokeWidth="3.4" strokeLinejoin="round" />
          <circle cx="32" cy="52.5" r="3.6" fill="var(--cover-bar)" />
          <path d="M47 15.5l4.5-4.5M17 15.5l-4.5-4.5" stroke="currentColor" strokeWidth="2.6" strokeLinecap="round" />
        </svg>
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

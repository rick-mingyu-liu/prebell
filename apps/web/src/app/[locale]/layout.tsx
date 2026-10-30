import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";
import { hasLocale } from "next-intl";
import { getTranslations, setRequestLocale } from "next-intl/server";
import Disclaimer from "@/components/Disclaimer";
import LangSwitch from "@/components/LangSwitch";
import { htmlLang, routing } from "@/i18n/routing";
import { inter } from "@/lib/fonts";
import { siteOrigin } from "@/lib/seo";
import "../globals.css";

// Every page reads D1 per request; the edge caches them for 60s (next.config headers).
export const dynamic = "force-dynamic";

export async function generateMetadata({ params }: LayoutProps<"/[locale]">): Promise<Metadata> {
  const { locale } = await params;
  if (!hasLocale(routing.locales, locale)) return {};
  const t = await getTranslations({ locale, namespace: "site" });
  return {
    metadataBase: await siteOrigin(),
    title: { default: t("name"), template: `%s · ${t("name")}` },
    description: t("tagline"),
  };
}

export default async function LocaleLayout({ children, params }: LayoutProps<"/[locale]">) {
  const { locale } = await params;
  if (!hasLocale(routing.locales, locale)) notFound();
  setRequestLocale(locale);
  const t = await getTranslations("site");
  const beacon = process.env.CF_BEACON_TOKEN;

  return (
    <html lang={htmlLang(locale)} className={inter.variable}>
      <body>
        <header className="site-header">
          <div className="wrap">
            <Link href={`/${locale}`} className="wordmark">
              <span>{t.rich("wordmark", { b: (chunks) => <b>{chunks}</b> })}</span>
              <small lang="zh-Hans">{t("wordmarkSub")}</small>
            </Link>
            <nav className="site-nav" aria-label={t("navLabel")}>
              <Link href={`/${locale}/archive`}>{t("briefs")}</Link>
              <Link href={`/${locale}/archive?type=earnings`}>{t("earnings")}</Link>
              <Link href={`/${locale}/earnings`}>{t("calendar")}</Link>
              <Link href={`/${locale}/about`}>{t("about")}</Link>
              <LangSwitch locale={locale} />
            </nav>
          </div>
        </header>
        <main className="wrap">{children}</main>
        <footer className="site-footer">
          <div className="wrap">
            <Disclaimer />
            <p className="footer-links">
              <Link href={`/${locale}/about`}>{t("about")}</Link>
              <LangSwitch locale={locale} className="footer-lang" />
            </p>
          </div>
        </footer>
        {beacon && (
          <script
            defer
            src="https://static.cloudflareinsights.com/beacon.min.js"
            data-cf-beacon={JSON.stringify({ token: beacon })}
          />
        )}
      </body>
    </html>
  );
}

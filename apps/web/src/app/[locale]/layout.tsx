import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";
import { hasLocale } from "next-intl";
import { getTranslations, setRequestLocale } from "next-intl/server";
import Disclaimer from "@/components/Disclaimer";
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
          <Link href={`/${locale}`} className="brand">
            {t("name")}
          </Link>
          <nav>
            <Link href={`/${locale}/archive`}>{t("archive")}</Link>
            <Link href={`/${locale}/about`}>{t("about")}</Link>
          </nav>
        </header>
        <main>{children}</main>
        <footer className="site-footer">
          <Disclaimer />
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

import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { hasLocale } from "next-intl";
import { getTranslations, setRequestLocale } from "next-intl/server";
import FooterLinks from "@/components/FooterLinks";
import IndexLine from "@/components/IndexLine";
import Masthead from "@/components/Masthead";
import { getDb } from "@/lib/db";
import { todayEt } from "@/lib/earnings";
import { inter, newsreader } from "@/lib/fonts";
import { type IndexLineView, loadIndexLine } from "@/lib/indexline";
import { siteOrigin } from "@/lib/seo";
import { datelineAt, formatDate } from "@/lib/time";
import { translator } from "@/lib/translator";
import { htmlLang, routing, type Locale } from "@/i18n/routing";
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

/** "Wednesday, November 4, 2026 · After the close": today's ET date and the edition of the moment ("Weekend edition" on Saturday and Sunday). */
function defaultDateline(locale: Locale): string {
  const t = translator(locale);
  const edition = t(`paper.dateline.${datelineAt()}`);
  return `${formatDate(todayEt(), locale, "full")} · ${edition}`;
}

/** The index snapshot, or null: a missing or unreadable one must not take a page down. */
async function indexLine(): Promise<IndexLineView | null> {
  try {
    return await loadIndexLine(getDb());
  } catch (err) {
    console.error("index line unavailable", err);
    return null;
  }
}

export default async function LocaleLayout({ children, params }: LayoutProps<"/[locale]">) {
  const { locale } = await params;
  if (!hasLocale(routing.locales, locale)) notFound();
  setRequestLocale(locale);
  const beacon = process.env.CF_BEACON_TOKEN;
  const line = await indexLine();

  return (
    <html lang={htmlLang(locale)} className={`${inter.variable} ${newsreader.variable}`}>
      <body>
        <div className="wrap">
          <Masthead locale={locale} dateline={defaultDateline(locale)} />
          <IndexLine line={line} locale={locale} />
        </div>
        <main className="wrap">{children}</main>
        <footer className="site-footer">
          <div className="wrap">
            <FooterLinks locale={locale} />
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

import type { Metadata } from "next";
import { getCloudflareContext } from "@opennextjs/cloudflare";
import { getTranslations, setRequestLocale } from "next-intl/server";
import HomeView from "@/components/HomeView";
import { htmlLang, type Locale } from "@/i18n/routing";
import { getDb } from "@/lib/db";
import { todayEt } from "@/lib/earnings";
import { loadHome } from "@/lib/home";
import { homeLd, jsonLd, pairedAlternates, shareMetadata, siteOrigin } from "@/lib/seo";
import { signupEnabled } from "@/lib/signup";

export async function generateMetadata({ params }: PageProps<"/[locale]">): Promise<Metadata> {
  const { locale } = await params;
  const t = await getTranslations({ locale, namespace: "site" });
  return {
    alternates: pairedAlternates(locale, ""),
    ...shareMetadata({
      locale: locale as Locale,
      url: `/${locale}`,
      title: t("name"),
      description: t("tagline"),
      siteName: t("name"),
    }),
  };
}

/** The front page. An old `?tab=` query is ignored; the page is the same for every query. */
export default async function Home({ params }: PageProps<"/[locale]">) {
  const locale = (await params).locale as Locale;
  setRequestLocale(locale);
  const data = await loadHome(getDb(), getCloudflareContext().env.FACTS, locale, todayEt(), (e, key) =>
    console.error(`home facts ${key}:`, e),
  );
  const t = await getTranslations({ locale, namespace: "site" });
  const ld = homeLd(await siteOrigin(), {
    url: `/${locale}`,
    name: t("name"),
    description: t("tagline"),
    lang: htmlLang(locale),
  });
  return (
    <>
      <script type="application/ld+json" dangerouslySetInnerHTML={{ __html: jsonLd(ld) }} />
      <HomeView data={data} locale={locale} signup={signupEnabled()} />
    </>
  );
}

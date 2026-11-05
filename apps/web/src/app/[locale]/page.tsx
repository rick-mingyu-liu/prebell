import type { Metadata } from "next";
import { getCloudflareContext } from "@opennextjs/cloudflare";
import { setRequestLocale } from "next-intl/server";
import HomeView from "@/components/HomeView";
import type { Locale } from "@/i18n/routing";
import { getDb } from "@/lib/db";
import { todayEt } from "@/lib/earnings";
import { loadHome } from "@/lib/home";
import { pairedAlternates } from "@/lib/seo";

export async function generateMetadata({ params }: PageProps<"/[locale]">): Promise<Metadata> {
  const { locale } = await params;
  return { alternates: pairedAlternates(locale, "") };
}

/** The front page. An old `?tab=` query is ignored; the page is the same for every query. */
export default async function Home({ params }: PageProps<"/[locale]">) {
  const locale = (await params).locale as Locale;
  setRequestLocale(locale);
  const data = await loadHome(getDb(), getCloudflareContext().env.FACTS, locale, todayEt(), (e, key) =>
    console.error(`home facts ${key}:`, e),
  );
  return <HomeView data={data} locale={locale} />;
}

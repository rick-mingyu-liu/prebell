import type { Metadata } from "next";
import Link from "next/link";
import { getCloudflareContext } from "@opennextjs/cloudflare";
import { getTranslations, setRequestLocale } from "next-intl/server";
import HomeView from "@/components/HomeView";
import PostList from "@/components/PostList";
import type { Locale } from "@/i18n/routing";
import { getDb } from "@/lib/db";
import { todayEt } from "@/lib/earnings";
import { loadHome } from "@/lib/home";
import { pairedAlternates } from "@/lib/seo";

export async function generateMetadata({ params }: PageProps<"/[locale]">): Promise<Metadata> {
  const { locale } = await params;
  return { alternates: pairedAlternates(locale, "") };
}

export default async function Home({ params, searchParams }: PageProps<"/[locale]">) {
  const locale = (await params).locale as Locale;
  setRequestLocale(locale);
  const t = await getTranslations();
  const today = todayEt();
  const { recent, ...home } = await loadHome(
    getDb(),
    getCloudflareContext().env.FACTS,
    locale,
    (await searchParams).tab,
    today,
    (e, key) => console.error(`home facts ${key}:`, e),
  );

  return (
    <HomeView locale={locale} today={today} {...home}>
      {recent.length > 0 && (
        <section className="home-recent" aria-labelledby="recent">
          <h2 id="recent">{t("home.recent")}</h2>
          <PostList locale={locale} items={recent} />
          <Link className="more" href={`/${locale}/archive`}>
            {t("home.allBriefs")} →
          </Link>
        </section>
      )}
    </HomeView>
  );
}

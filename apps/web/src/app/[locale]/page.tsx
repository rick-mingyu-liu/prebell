import type { Metadata } from "next";
import Link from "next/link";
import { getCloudflareContext } from "@opennextjs/cloudflare";
import { getTranslations, setRequestLocale } from "next-intl/server";
import HomeView, { selectTab, type BriefTab } from "@/components/HomeView";
import PostList from "@/components/PostList";
import type { Locale } from "@/i18n/routing";
import { getDb } from "@/lib/db";
import { listLatestEarnings, listRecentBriefs, stripDay, todayEt } from "@/lib/earnings";
import { loadFacts } from "@/lib/facts";
import { getLatestPost } from "@/lib/posts";
import { pairedAlternates } from "@/lib/seo";

export async function generateMetadata({ params }: PageProps<"/[locale]">): Promise<Metadata> {
  const { locale } = await params;
  return { alternates: pairedAlternates(locale, "") };
}

export default async function Home({ params, searchParams }: PageProps<"/[locale]">) {
  const locale = (await params).locale as Locale;
  setRequestLocale(locale);
  const t = await getTranslations();
  const db = getDb();
  const today = todayEt();
  // Briefs and earnings checks are listed separately, so a busy earnings
  // season never pushes the briefs off the home page.
  const [premarket, recap, recent, earnings, strip] = await Promise.all([
    getLatestPost(db, locale, "premarket"),
    getLatestPost(db, locale, "recap"),
    listRecentBriefs(db, locale, 12),
    listLatestEarnings(db, locale, 3),
    stripDay(db, today, { exact: false, lang: locale }),
  ]);
  const tab = selectTab(premarket, recap, (await searchParams).tab);
  const post = tab === "recap" ? recap : tab === "premarket" ? premarket : null;
  // Only the selected tab's facts: one R2 read per view.
  const facts =
    tab && post
      ? await loadFacts(getCloudflareContext().env.FACTS, post.factsKey, tab, (e) =>
          console.error(`home facts ${post.factsKey}:`, e),
        )
      : null;
  const tabs = (["premarket", "recap"] as BriefTab[]).filter((k) => (k === "premarket" ? premarket : recap));
  const shown = new Set([premarket?.slug, recap?.slug]);
  const others = recent.filter((p) => !shown.has(p.slug)).slice(0, 10);

  return (
    <HomeView
      locale={locale}
      tab={tab}
      tabs={tabs}
      post={post}
      facts={facts}
      earnings={earnings}
      strip={strip}
      today={today}
    >
      {others.length > 0 && (
        <section className="home-recent" aria-labelledby="recent">
          <h2 id="recent">{t("home.recent")}</h2>
          <PostList locale={locale} items={others} />
          <Link className="more" href={`/${locale}/archive`}>
            {t("home.allBriefs")} →
          </Link>
        </section>
      )}
    </HomeView>
  );
}

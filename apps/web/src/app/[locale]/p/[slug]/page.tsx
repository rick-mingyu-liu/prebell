import type { Metadata } from "next";
import { notFound, redirect } from "next/navigation";
import { cache } from "react";
import { getCloudflareContext } from "@opennextjs/cloudflare";
import { getTranslations, setRequestLocale } from "next-intl/server";
import BriefView from "@/components/BriefView";
import type { Locale } from "@/i18n/routing";
import { loadBrief } from "@/lib/brief";
import { getDb } from "@/lib/db";
import { todayEt } from "@/lib/earnings";
import { pendingTitle } from "@/lib/labels";
import { getPostBySlug, isPublishedIn } from "@/lib/posts";
import { pairedAlternates } from "@/lib/seo";

// One D1 read shared by generateMetadata and the page.
const load = cache((slug: string, locale: Locale) => getPostBySlug(getDb(), slug, locale));

export async function generateMetadata({ params }: PageProps<"/[locale]/p/[slug]">): Promise<Metadata> {
  const { locale, slug } = await params;
  const post = await load(slug, locale as Locale);
  if (!post) return {};
  const t = await getTranslations({ locale });
  if (post.status === "pending") {
    // Not indexed and not paired until the Chinese text is published.
    return { title: pendingTitle(t, post, locale as Locale), robots: { index: false } };
  }
  return {
    title: post.title,
    description: post.summary,
    alternates: post.hasOtherLang ? pairedAlternates(locale, `/p/${slug}`) : { canonical: `/${locale}/p/${slug}` },
  };
}

export default async function PostPage({ params }: PageProps<"/[locale]/p/[slug]">) {
  const { slug } = await params;
  const locale = (await params).locale as Locale;
  setRequestLocale(locale);
  const db = getDb();
  const post = await load(slug, locale);
  if (!post) {
    // The header's language switch keeps the path, so it lands here for a post whose text in this
    // language failed; that reader goes to this language's home page instead of a 404.
    if (await isPublishedIn(db, slug, locale === "en" ? "zh" : "en")) redirect(`/${locale}`);
    notFound();
  }
  const data = await loadBrief(db, getCloudflareContext().env.FACTS, post, locale, (e) =>
    console.error(`post facts ${post.factsKey}:`, e),
  );
  return <BriefView locale={locale} today={todayEt()} {...data} />;
}

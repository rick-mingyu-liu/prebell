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
import { resolvePost } from "@/lib/posts";
import { pairedAlternates } from "@/lib/seo";

// One lookup shared by generateMetadata and the page.
const resolve = cache((slug: string, locale: Locale) => resolvePost(getDb(), slug, locale));

export async function generateMetadata({ params }: PageProps<"/[locale]/p/[slug]">): Promise<Metadata> {
  const { locale, slug } = await params;
  const found = await resolve(slug, locale as Locale);
  if (!found || !("post" in found)) return {};
  const { post } = found;
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
  const found = await resolve(slug, locale);
  if (!found) notFound();
  // A real 307/404 only while no loading.tsx or Suspense boundary sits above this page; with one,
  // the response would already be streaming and these would become a meta refresh and a soft 404.
  if ("redirectTo" in found) redirect(found.redirectTo);
  const { post } = found;
  const data = await loadBrief(getDb(), getCloudflareContext().env.FACTS, post, locale, (e) =>
    console.error(`post facts ${post.factsKey}:`, e),
  );
  return <BriefView locale={locale} today={todayEt()} {...data} />;
}

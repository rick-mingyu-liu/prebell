import type { Metadata } from "next";
import { notFound, redirect } from "next/navigation";
import { cache } from "react";
import { getCloudflareContext } from "@opennextjs/cloudflare";
import { getTranslations, setRequestLocale } from "next-intl/server";
import BriefView from "@/components/BriefView";
import { htmlLang, type Locale } from "@/i18n/routing";
import { loadBrief } from "@/lib/brief";
import { getDb } from "@/lib/db";
import { pendingTitle } from "@/lib/labels";
import { resolvePost } from "@/lib/posts";
import { postCard } from "@/lib/covers";
import { earningsSearchTitle } from "@/lib/searchtitle";
import { jsonLd, newsArticleLd, pairedAlternates, shareMetadata, siteOrigin } from "@/lib/seo";
import { signupEnabled } from "@/lib/signup";

// One lookup shared by generateMetadata and the page.
const resolve = cache((slug: string, locale: Locale) => resolvePost(getDb(), slug, locale));
// The share card, once per request: the post's own only after the generator has rendered it.
const card = cache((coverKey: string | null) => postCard(getCloudflareContext().env.FACTS, coverKey));

export async function generateMetadata({ params }: PageProps<"/[locale]/p/[slug]">): Promise<Metadata> {
  const { locale, slug } = await params;
  const found = await resolve(slug, locale as Locale);
  if (!found || !("post" in found)) return {};
  const { post } = found;
  const t = await getTranslations({ locale });
  if (post.status === "pending") {
    // Not indexed and not paired until the Chinese text is published.
    const title = pendingTitle(t, post, locale as Locale);
    return {
      title,
      robots: { index: false },
      ...shareMetadata({
        locale: locale as Locale,
        url: `/${locale}/p/${slug}`,
        title,
        description: t("site.tagline"),
        siteName: t("site.name"),
        image: await card(post.coverKey),
      }),
    };
  }
  // An earnings check's search-result title names the company and period (what people search); its
  // page, share card and JSON-LD keep the headline.
  const searchTitle =
    post.contentType === "earnings" && post.ticker
      ? earningsSearchTitle({ slug, ticker: post.ticker, tradingDate: post.tradingDate }, locale as Locale)
      : post.title;
  return {
    title: searchTitle,
    description: post.summary,
    alternates: post.hasOtherLang ? pairedAlternates(locale, `/p/${slug}`) : { canonical: `/${locale}/p/${slug}` },
    ...shareMetadata({
      locale: locale as Locale,
      url: `/${locale}/p/${slug}`,
      title: post.title,
      description: post.summary,
      siteName: t("site.name"),
      image: await card(post.coverKey),
      article: { publishedTime: post.publishedAt, modifiedTime: post.revisedAt ?? post.publishedAt },
    }),
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
  const ld =
    post.status === "published"
      ? newsArticleLd(await siteOrigin(), {
          url: `/${locale}/p/${slug}`,
          headline: post.title,
          description: post.summary,
          image: await card(post.coverKey),
          lang: htmlLang(locale),
          publishedAt: post.publishedAt,
          revisedAt: post.revisedAt,
        })
      : null;
  return (
    <>
      {ld && <script type="application/ld+json" dangerouslySetInnerHTML={{ __html: jsonLd(ld) }} />}
      <BriefView data={data} locale={locale} signup={signupEnabled()} />
    </>
  );
}

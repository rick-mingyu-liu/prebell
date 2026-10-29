import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { cache } from "react";
import { getTranslations, setRequestLocale } from "next-intl/server";
import LangSwitch from "@/components/LangSwitch";
import PendingNotice from "@/components/PendingNotice";
import PostBody from "@/components/PostBody";
import type { Locale } from "@/i18n/routing";
import { getDb } from "@/lib/db";
import { getPostBySlug } from "@/lib/posts";
import { pairedAlternates } from "@/lib/seo";
import { formatDate, formatEt } from "@/lib/time";

// One D1 read shared by generateMetadata and the page.
const load = cache((slug: string, locale: Locale) => getPostBySlug(getDb(), slug, locale));

export async function generateMetadata({ params }: PageProps<"/[locale]/p/[slug]">): Promise<Metadata> {
  const { locale, slug } = await params;
  const post = await load(slug, locale as Locale);
  if (!post) return {};
  const t = await getTranslations({ locale, namespace: "post" });
  if (post.status === "pending") {
    // Not indexed and not paired until the Chinese text is published.
    return { title: t("pendingTitle", { date: formatDate(post.tradingDate, locale as Locale) }), robots: { index: false } };
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
  const post = await load(slug, locale);
  if (!post) notFound();
  const t = await getTranslations("post");

  if (post.status === "pending") {
    return (
      <article>
        <h1>{t("pendingTitle", { date: formatDate(post.tradingDate, locale as Locale) })}</h1>
        <PendingNotice slug={post.slug} />
      </article>
    );
  }

  return (
    <article>
      <div className="page-tools">
        <p className="meta">
          <time dateTime={post.publishedAt ?? undefined}>
            {t("published", { time: formatEt(post.publishedAt, locale) })}
          </time>
        </p>
        {post.hasOtherLang && <LangSwitch locale={locale} path={`/p/${post.slug}`} />}
      </div>
      <PostBody markdown={post.bodyMd} />
    </article>
  );
}

import type { Metadata } from "next";
import Link from "next/link";
import { getTranslations, setRequestLocale } from "next-intl/server";
import LangSwitch from "@/components/LangSwitch";
import PendingNotice from "@/components/PendingNotice";
import PostBody from "@/components/PostBody";
import PostList from "@/components/PostList";
import type { Locale } from "@/i18n/routing";
import { getDb } from "@/lib/db";
import { pendingTitle } from "@/lib/labels";
import { getLatestPost, listPosts } from "@/lib/posts";
import { pairedAlternates } from "@/lib/seo";
import { formatDate, formatEt } from "@/lib/time";

export async function generateMetadata({ params }: PageProps<"/[locale]">): Promise<Metadata> {
  const { locale } = await params;
  return { alternates: pairedAlternates(locale, "") };
}

export default async function Home({ params }: PageProps<"/[locale]">) {
  const locale = (await params).locale as Locale;
  setRequestLocale(locale);
  const t = await getTranslations();
  const db = getDb();
  const [latest, recap, recent] = await Promise.all([
    getLatestPost(db, locale, "premarket"),
    getLatestPost(db, locale, "recap"),
    listPosts(db, locale, 1),
  ]);
  const shown = new Set([latest?.slug, recap?.slug]);
  const others = recent.filter((p) => !shown.has(p.slug)).slice(0, 10);
  // The recap title is the page's top-level heading when there is no premarket brief.
  const RecapHeading = latest ? "h3" : "h1";

  return (
    <>
      <div className="page-tools">
        <LangSwitch locale={locale} path="" />
      </div>
      {!latest && !recap && <p>{t("home.empty")}</p>}
      {latest && (
        <section aria-labelledby="latest">
          <h2 id="latest" className="eyebrow">
            {t("home.latest")}
          </h2>
          {latest?.status === "pending" && (
            <>
              <h1>{pendingTitle(t, latest, locale)}</h1>
              <PendingNotice slug={latest.slug} />
            </>
          )}
          {latest?.status === "published" && (
            <article>
              <p className="meta">
                <time dateTime={latest.publishedAt ?? undefined}>
                  {t("post.published", { time: formatEt(latest.publishedAt, locale) })}
                </time>
                {" · "}
                <Link href={`/${locale}/p/${latest.slug}`}>{t("post.permalink")}</Link>
              </p>
              <PostBody markdown={latest.bodyMd} sources={latest.sources.map((s) => s.url)} />
            </article>
          )}
        </section>
      )}
      {recap && (
        <section aria-labelledby="recap">
          <h2 id="recap" className="eyebrow">
            {t("home.latestRecap")}
          </h2>
          <article>
            <RecapHeading>
              <Link href={`/${locale}/p/${recap.slug}`}>
                {recap.status === "published" ? recap.title : pendingTitle(t, recap, locale)}
              </Link>
            </RecapHeading>
            <p className="meta">
              <time dateTime={recap.tradingDate}>{formatDate(recap.tradingDate, locale)}</time>
            </p>
            {recap.status === "published" ? recap.summary && <p>{recap.summary}</p> : <PendingNotice slug={recap.slug} />}
          </article>
        </section>
      )}
      {others.length > 0 && (
        <section aria-labelledby="recent">
          <h2 id="recent">{t("home.recent")}</h2>
          <PostList locale={locale} items={others} />
          <p>
            <Link href={`/${locale}/archive`}>{t("home.allBriefs")} →</Link>
          </p>
        </section>
      )}
    </>
  );
}

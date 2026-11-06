import type { Metadata } from "next";
import Link from "next/link";
import { getTranslations, setRequestLocale } from "next-intl/server";
import PostList from "@/components/PostList";
import type { Locale } from "@/i18n/routing";
import { getDb } from "@/lib/db";
import { typeLabel } from "@/lib/labels";
import { CONTENT_TYPES, type ContentType, listPosts, PAGE_SIZE, parseContentType } from "@/lib/posts";
import { pairedAlternates, shareMetadata } from "@/lib/seo";

function pageNumber(raw: string | string[] | undefined): number {
  const n = Number(Array.isArray(raw) ? raw[0] : raw);
  return Number.isInteger(n) && n >= 1 ? n : 1;
}

/** The archive path after the locale, keeping only meaningful parameters. */
function archivePath(type: ContentType | null, page: number): string {
  const q = new URLSearchParams();
  if (type) q.set("type", type);
  if (page > 1) q.set("page", String(page));
  const s = q.toString();
  return s ? `/archive?${s}` : "/archive";
}

export async function generateMetadata({ params, searchParams }: PageProps<"/[locale]/archive">): Promise<Metadata> {
  const { locale } = await params;
  const sp = await searchParams;
  const page = pageNumber(sp.page);
  const type = parseContentType(sp.type);
  const t = await getTranslations({ locale });
  const title = type ? `${t("archive.title")} · ${typeLabel(t, type)}` : t("archive.title");
  const path = archivePath(type, page);
  return {
    title,
    alternates: pairedAlternates(locale, path),
    ...shareMetadata({
      locale: locale as Locale,
      url: `/${locale}${path}`,
      title,
      description: t("site.tagline"),
      siteName: t("site.name"),
    }),
  };
}

export default async function Archive({ params, searchParams }: PageProps<"/[locale]/archive">) {
  const locale = (await params).locale as Locale;
  setRequestLocale(locale);
  const sp = await searchParams;
  const page = pageNumber(sp.page);
  const type = parseContentType(sp.type);
  const t = await getTranslations();
  const items = await listPosts(getDb(), locale, page, type);
  const href = (ty: ContentType | null, n: number) => `/${locale}${archivePath(ty, n)}`;

  return (
    <div className="list-page">
      <h1>
        {t("archive.title")}
        {type && <span className="muted"> · {typeLabel(t, type)}</span>}
        {page > 1 && <span className="muted"> · {t("archive.page", { page })}</span>}
      </h1>
      <nav className="filters" aria-label={t("archive.filter")}>
        {[null, ...CONTENT_TYPES].map((ty) => (
          <Link
            key={ty ?? "all"}
            href={href(ty, 1)}
            aria-current={ty === type ? (page === 1 ? "page" : "true") : undefined}
          >
            {ty ? typeLabel(t, ty) : t("archive.all")}
          </Link>
        ))}
      </nav>
      {items.length === 0 ? <p>{t("archive.empty")}</p> : <PostList locale={locale} items={items} headingLevel="h2" />}
      <nav className="pager">
        {page > 1 && <Link href={href(type, page - 1)}>← {t("archive.newer")}</Link>}
        {items.length === PAGE_SIZE && <Link href={href(type, page + 1)}>{t("archive.older")} →</Link>}
      </nav>
    </div>
  );
}

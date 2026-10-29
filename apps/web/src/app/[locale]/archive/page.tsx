import type { Metadata } from "next";
import Link from "next/link";
import { getTranslations, setRequestLocale } from "next-intl/server";
import LangSwitch from "@/components/LangSwitch";
import PostList from "@/components/PostList";
import type { Locale } from "@/i18n/routing";
import { getDb } from "@/lib/db";
import { listPosts, PAGE_SIZE } from "@/lib/posts";
import { pairedAlternates } from "@/lib/seo";

function pageNumber(raw: string | string[] | undefined): number {
  const n = Number(Array.isArray(raw) ? raw[0] : raw);
  return Number.isInteger(n) && n >= 1 ? n : 1;
}

export async function generateMetadata({ params, searchParams }: PageProps<"/[locale]/archive">): Promise<Metadata> {
  const { locale } = await params;
  const page = pageNumber((await searchParams).page);
  const t = await getTranslations({ locale, namespace: "archive" });
  const query = page > 1 ? `?page=${page}` : "";
  return { title: t("title"), alternates: pairedAlternates(locale, `/archive${query}`) };
}

export default async function Archive({ params, searchParams }: PageProps<"/[locale]/archive">) {
  const locale = (await params).locale as Locale;
  setRequestLocale(locale);
  const page = pageNumber((await searchParams).page);
  const t = await getTranslations("archive");
  const items = await listPosts(getDb(), locale, page);
  const href = (n: number) => (n > 1 ? `/${locale}/archive?page=${n}` : `/${locale}/archive`);

  return (
    <>
      <div className="page-tools">
        <h1>
          {t("title")}
          {page > 1 && <span className="muted"> · {t("page", { page })}</span>}
        </h1>
        <LangSwitch locale={locale} path="/archive" />
      </div>
      {items.length === 0 ? <p>{t("empty")}</p> : <PostList locale={locale} items={items} />}
      <nav className="pager">
        {page > 1 && <Link href={href(page - 1)}>← {t("newer")}</Link>}
        {items.length === PAGE_SIZE && <Link href={href(page + 1)}>{t("older")} →</Link>}
      </nav>
    </>
  );
}

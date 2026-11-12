import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { cache } from "react";
import { getTranslations, setRequestLocale } from "next-intl/server";
import IpoView from "@/components/IpoView";
import type { Locale } from "@/i18n/routing";
import { getDb } from "@/lib/db";
import { todayEt } from "@/lib/earnings";
import { loadIpo } from "@/lib/ipo";
import { pairedAlternates, shareMetadata } from "@/lib/seo";

// One load shared by generateMetadata and the page: five D1 reads, no R2. The clock is read here, once per request,
// and handed to the reader as the ET date (a passed reported date reads "date not set").
const load = cache((slug: string, locale: Locale) => loadIpo(getDb(), slug, locale, todayEt()));

export async function generateMetadata({ params }: PageProps<"/[locale]/ipo/[slug]">): Promise<Metadata> {
  const { locale, slug } = await params;
  const ipo = await load(slug, locale as Locale);
  if (!ipo) return {};
  const t = await getTranslations({ locale });
  const title = t("ipo.title", { name: ipo.name });
  const description = t("ipo.description", { name: ipo.name });
  return {
    title,
    description,
    alternates: pairedAlternates(locale, `/ipo/${slug}`),
    ...shareMetadata({
      locale: locale as Locale,
      url: `/${locale}/ipo/${slug}`,
      title,
      description,
      siteName: t("site.name"),
    }),
  };
}

export default async function IpoPage({ params }: PageProps<"/[locale]/ipo/[slug]">) {
  const { slug } = await params;
  const locale = (await params).locale as Locale;
  setRequestLocale(locale);
  const ipo = await load(slug, locale);
  // A real 404 only while no loading.tsx or Suspense boundary sits above this page.
  if (!ipo) notFound();
  return <IpoView ipo={ipo} locale={locale} />;
}

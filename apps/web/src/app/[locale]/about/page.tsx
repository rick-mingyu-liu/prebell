import type { Metadata } from "next";
import { getTranslations, setRequestLocale } from "next-intl/server";
import Disclaimer from "@/components/Disclaimer";
import type { Locale } from "@/i18n/routing";
import { pairedAlternates } from "@/lib/seo";

export async function generateMetadata({ params }: PageProps<"/[locale]/about">): Promise<Metadata> {
  const { locale } = await params;
  const t = await getTranslations({ locale, namespace: "about" });
  return { title: t("title"), alternates: pairedAlternates(locale, "/about") };
}

export default async function About({ params }: PageProps<"/[locale]/about">) {
  const locale = (await params).locale as Locale;
  setRequestLocale(locale);
  const t = await getTranslations("about");
  return (
    <article className="prose">
      <h1>{t("title")}</h1>
      <p>{t("p1")}</p>
      <p>{t("p2")}</p>
      <p>{t("p3")}</p>
      <Disclaimer />
    </article>
  );
}

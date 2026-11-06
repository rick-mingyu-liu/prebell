import type { Metadata } from "next";
import { Fragment } from "react";
import { getTranslations, setRequestLocale } from "next-intl/server";
import type { Locale } from "@/i18n/routing";
import { pairedAlternates, shareMetadata } from "@/lib/seo";
import { translator } from "@/lib/translator";

type Section = { h: string; p: string[]; ul?: string[] };

export async function generateMetadata({ params }: PageProps<"/[locale]/legal">): Promise<Metadata> {
  const { locale } = await params;
  const t = await getTranslations({ locale, namespace: "legal" });
  const site = translator(locale as Locale);
  return {
    title: t("title"),
    alternates: pairedAlternates(locale, "/legal"),
    ...shareMetadata({
      locale: locale as Locale,
      url: `/${locale}/legal`,
      title: t("title"),
      description: site("site.tagline"),
      siteName: site("site.name"),
    }),
  };
}

export default async function Legal({ params }: PageProps<"/[locale]/legal">) {
  const locale = (await params).locale as Locale;
  setRequestLocale(locale);
  const t = translator(locale);
  const groups = [t.raw("legal.privacy") as Section[], t.raw("legal.terms") as Section[]];
  // Headings, paragraphs and lists stay direct children of the article: the .prose rules are direct-child rules.
  return (
    <article className="prose prose-legal">
      <h1>{t("legal.title")}</h1>
      <p className="muted">{t("legal.updated")}</p>
      {groups.map((sections, g) => (
        <Fragment key={g}>
          {g > 0 && <hr className="rule-strong" />}
          {sections.map((s) => (
            <Fragment key={s.h}>
              <h2>{s.h}</h2>
              {s.p.map((text) => (
                <p key={text}>{text}</p>
              ))}
              {s.ul && (
                <ul>
                  {s.ul.map((text) => (
                    <li key={text}>{text}</li>
                  ))}
                </ul>
              )}
            </Fragment>
          ))}
        </Fragment>
      ))}
    </article>
  );
}

import type { Metadata } from "next";
import { getTranslations, setRequestLocale } from "next-intl/server";
import { companyName } from "@/lib/earnings";
import type { Locale } from "@/i18n/routing";
import { photoCredits } from "@/lib/photos";
import { translator } from "@/lib/translator";

// The credits for the company photos, one row per photo, anchored by ticker: what the licences ask a reuser to
// give, as text, so it reads where Wikimedia does not load. Linked only from the credit under each photo and
// from the legal page; not indexed and not in the sitemap. It loads nothing from a third party.
export async function generateMetadata({ params }: PageProps<"/[locale]/photo-credits">): Promise<Metadata> {
  const { locale } = await params;
  const t = await getTranslations({ locale, namespace: "photoCredits" });
  return { title: t("title"), robots: { index: false, follow: false } };
}

export default async function PhotoCredits({ params }: PageProps<"/[locale]/photo-credits">) {
  const locale = (await params).locale as Locale;
  setRequestLocale(locale);
  const t = translator(locale);
  const credits = photoCredits(locale);
  /** A URL written out as text, also a link. */
  const url = (u: string) => (
    <a href={u} rel="noopener">
      {u}
    </a>
  );
  return (
    <article className="prose photo-credits">
      <h1>{t("photoCredits.title")}</h1>
      <p>{t("photoCredits.intro")}</p>
      {credits.map((c) => (
        <section key={c.ticker} id={c.ticker} className="photo-credit-row">
          <h2>
            {companyName(c.ticker) ?? c.ticker} ({c.ticker})
          </h2>
          <p className="muted">{c.caption}</p>
          <dl>
            <dt>{t("photoCredits.work")}</dt>
            <dd>{c.file}</dd>
            {c.sourceTitle && (
              <>
                <dt>{t("photoCredits.flickrTitle")}</dt>
                <dd>{c.sourceTitle}</dd>
              </>
            )}
            <dt>{t("photoCredits.author")}</dt>
            <dd>{c.author}</dd>
            {c.notices.length > 0 && (
              <>
                <dt>{t("photoCredits.notices")}</dt>
                {c.notices.map((n) => (
                  <dd key={n} lang="en">
                    {n}
                  </dd>
                ))}
              </>
            )}
            <dt>{t("photoCredits.commons")}</dt>
            <dd>{url(c.page)}</dd>
            {c.sourceUrl && (
              <>
                <dt>{t("photoCredits.original")}</dt>
                <dd>{url(c.sourceUrl)}</dd>
              </>
            )}
            <dt>{t("photoCredits.licence")}</dt>
            <dd>
              {c.licence === "Public domain" ? t("photoCredits.publicDomain") : c.licence}
              {c.licenceUrl && <> · {url(c.licenceUrl)}</>}
            </dd>
            <dt>{t("photoCredits.change")}</dt>
            <dd>
              {t("photoCredits.changed", { width: c.width, height: c.height })}
              {c.derived && (
                <>
                  {" "}
                  {c.derived.from
                    ? t("photoCredits.cropOf", { file: c.derived.from, author: c.author })
                    : t("photoCredits.crop", { author: c.author })}
                </>
              )}
            </dd>
          </dl>
        </section>
      ))}
    </article>
  );
}

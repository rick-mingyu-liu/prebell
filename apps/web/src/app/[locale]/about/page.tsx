import type { Metadata } from "next";
import { getTranslations, setRequestLocale } from "next-intl/server";
import AuthorBlock from "@/components/AuthorBlock";
import Disclaimer from "@/components/Disclaimer";
import SignupForm from "@/components/SignupForm";
import type { Locale } from "@/i18n/routing";
import { TERMPOCKET_URL } from "@/lib/partners";
import { pairedAlternates } from "@/lib/seo";
import { signupEnabled } from "@/lib/signup";

export async function generateMetadata({ params }: PageProps<"/[locale]/about">): Promise<Metadata> {
  const { locale } = await params;
  const t = await getTranslations({ locale, namespace: "about" });
  return { title: t("title"), alternates: pairedAlternates(locale, "/about") };
}

export default async function About({ params }: PageProps<"/[locale]/about">) {
  const locale = (await params).locale as Locale;
  setRequestLocale(locale);
  const t = await getTranslations();
  return (
    <article className="prose">
      <h1>{t("about.title")}</h1>
      <p>{t("about.p1")}</p>
      <p>{t("about.p2")}</p>
      <p>{t("about.p3")}</p>
      <hr className="rule-strong" />
      <h2>{t("about.whoHeading")}</h2>
      <AuthorBlock locale={locale} />
      <p className="about-tp">
        {t("termpocket.copy")}{" "}
        <a className="more" href={TERMPOCKET_URL} target="_blank" rel="noopener">
          {t("termpocket.cta")}
        </a>
      </p>
      {signupEnabled() && <SignupForm source="about" locale={locale} />}
      <Disclaimer />
    </article>
  );
}

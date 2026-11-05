import Link from "next/link";
import { getLocale, getTranslations } from "next-intl/server";

export default async function NotFound() {
  const locale = await getLocale();
  const t = await getTranslations("notFound");
  return (
    <section className="notfound">
      <p className="kicker" aria-hidden="true">
        404
      </p>
      <h1>{t("title")}</h1>
      <p>
        <Link className="more" href={`/${locale}`}>
          {t("back")} →
        </Link>
      </p>
    </section>
  );
}

import Link from "next/link";
import { getLocale, getTranslations } from "next-intl/server";

export default async function NotFound() {
  const locale = await getLocale();
  const t = await getTranslations("notFound");
  return (
    <>
      <h1>{t("title")}</h1>
      <p>
        <Link href={`/${locale}`}>{t("back")}</Link>
      </p>
    </>
  );
}

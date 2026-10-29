import Link from "next/link";
import { getTranslations } from "next-intl/server";

/** Shown on a Chinese page while only the English text is published. */
export default async function PendingNotice({ slug }: { slug: string }) {
  const t = await getTranslations("post");
  return (
    <div className="notice" role="status">
      <p>{t("pending")}</p>
      <p>
        <Link href={`/en/p/${slug}`} hrefLang="en" lang="en">
          {t("readEnglish")}
        </Link>
      </p>
    </div>
  );
}

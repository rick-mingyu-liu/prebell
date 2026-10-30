import Link from "next/link";
import type { Locale } from "../i18n/routing";
import { translator } from "../lib/translator";

/** Shown on a Chinese page while only the English text is published. */
export default function PendingNotice({ slug, locale }: { slug: string; locale: Locale }) {
  const t = translator(locale);
  return (
    <div className="notice" role="status">
      <p>{t("post.pending")}</p>
      <p>
        <Link href={`/en/p/${slug}`} hrefLang="en" lang="en">
          {t("post.readEnglish")}
        </Link>
      </p>
    </div>
  );
}

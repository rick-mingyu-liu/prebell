import Link from "next/link";
import type { Locale } from "../i18n/routing";
import { hasAuthorPhoto } from "../lib/author";
import { translator } from "../lib/translator";

const LINKS = [
  { key: "website", href: "https://rickmingyuliu.com" },
  { key: "github", href: "https://github.com/rick-mingyu-liu" },
  { key: "linkedin", href: "https://www.linkedin.com/in/rick-mingyu-liu/" },
] as const;

/** Who is behind the site: greyscale photo (when shipped), name in the serif, one-line bio, links, archive link. */
export default function AuthorBlock({ locale }: { locale: Locale }) {
  const t = translator(locale);
  return (
    <section className="author">
      {/* Decorative: the name beside it says who this is. */}
      {hasAuthorPhoto && <img className="author-photo" src="/author.jpg" alt="" width={72} height={72} />}
      <div className="author-body">
        <p className="author-name">{t("author.name")}</p>
        <p className="author-bio">{t("author.bio")}</p>
        <p className="author-links">
          {LINKS.map((l) => (
            <a key={l.key} href={l.href} target="_blank" rel="noopener noreferrer">
              {t(`author.${l.key}`)}
            </a>
          ))}
        </p>
      </div>
      <Link className="author-all" href={`/${locale}/archive`}>
        {t("author.allBriefs")}
      </Link>
    </section>
  );
}

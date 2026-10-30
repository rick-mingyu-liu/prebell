import Link from "next/link";
import type { Locale } from "../i18n/routing";
import { typeLabel } from "../lib/labels";
import type { PostListItem } from "../lib/posts";
import { formatDate } from "../lib/time";
import { translator } from "../lib/translator";

/** Posts as rows: date, title and summary on the left, the type tag on the right. */
export default function PostList({ locale, items }: { locale: Locale; items: PostListItem[] }) {
  const t = translator(locale);
  return (
    <ul className="post-list">
      {items.map((p) => (
        <li key={p.slug} className="row post-row">
          <div className="post-main">
            <time className="post-meta" dateTime={p.tradingDate}>
              {formatDate(p.tradingDate, locale)}
            </time>
            <Link href={`/${locale}/p/${p.slug}`}>{p.title || p.slug}</Link>
            {p.summary && <p>{p.summary}</p>}
          </div>
          <span className="tag">{typeLabel(t, p.contentType)}</span>
        </li>
      ))}
    </ul>
  );
}

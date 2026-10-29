import Link from "next/link";
import type { PostListItem } from "@/lib/posts";
import type { Locale } from "@/i18n/routing";
import { formatDate } from "@/lib/time";

export default function PostList({ locale, items }: { locale: Locale; items: PostListItem[] }) {
  return (
    <ul className="post-list">
      {items.map((p) => (
        <li key={p.slug}>
          <time dateTime={p.tradingDate}>{formatDate(p.tradingDate, locale)}</time>
          <Link href={`/${locale}/p/${p.slug}`}>{p.title || p.slug}</Link>
          {p.summary && <p>{p.summary}</p>}
        </li>
      ))}
    </ul>
  );
}

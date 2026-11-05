import Link from "next/link";
import Cover from "./Cover";
import type { Locale } from "../i18n/routing";
import { typeLabel } from "../lib/labels";
import type { PostListItem } from "../lib/posts";
import { formatDate, formatMonthDay } from "../lib/time";
import { translator } from "../lib/translator";

type T = ReturnType<typeof translator>;

/** The cover's tag: the company logo for an earnings check, else "Nov 4 · Recap" for a brief or the type. */
function coverTag(p: PostListItem, locale: Locale, t: T) {
  if (p.contentType === "recap" || p.contentType === "premarket") {
    return { kind: "label", text: `${formatMonthDay(p.tradingDate, locale)} · ${t(`paper.edition.${p.contentType}`)}` } as const;
  }
  return p.ticker
    ? ({ kind: "logo", ticker: p.ticker } as const)
    : ({ kind: "label", text: typeLabel(t, p.contentType) } as const);
}

/**
 * Posts as ruled rows: a small cover on the left (flat, tag only, for a post with no scene or lines), then
 * the kicker, the serif headline, a one-line summary and the date. The type is the kicker, not a tag.
 */
export default function PostList({ locale, items }: { locale: Locale; items: PostListItem[] }) {
  const t = translator(locale);
  return (
    <ul className="post-list">
      {items.map((p) => {
        const title = p.title || p.slug;
        return (
          <li key={p.slug} className="post-row">
            <Cover coverKey={p.coverKey} lines={p.coverLines} tag={coverTag(p, locale, t)} size="small" alt={title} />
            <div className="post-main">
              <p className="kicker">{typeLabel(t, p.contentType)}</p>
              <h3 className="story-title">
                <Link href={`/${locale}/p/${p.slug}`}>{title}</Link>
              </h3>
              {p.summary && <p className="story-sum">{p.summary}</p>}
              <time className="story-meta" dateTime={p.tradingDate}>
                {formatDate(p.tradingDate, locale)}
              </time>
            </div>
          </li>
        );
      })}
    </ul>
  );
}

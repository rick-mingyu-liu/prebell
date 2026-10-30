"use client";

import Link from "next/link";
import { usePathname, useSearchParams } from "next/navigation";
import { type Locale, routing } from "@/i18n/routing";

const LOCALE_PREFIX = new RegExp(`^/(${routing.locales.join("|")})(?=/|$)`);

/**
 * The current URL with its locale prefix swapped for `target`, for the shared header and footer.
 * `page` is dropped: the other language can have fewer posts, so the same page number may be empty.
 */
export default function LangSwitchLink({
  target,
  hrefLang,
  label,
  title,
  className,
}: {
  target: Locale;
  hrefLang: string;
  label: string;
  title: string;
  className: string;
}) {
  const rest = usePathname().replace(LOCALE_PREFIX, "");
  const params = new URLSearchParams(useSearchParams());
  params.delete("page");
  const query = params.toString();
  return (
    <Link
      className={className}
      href={`/${target}${rest}${query ? `?${query}` : ""}`}
      hrefLang={hrefLang}
      lang={hrefLang}
      title={title}
    >
      {label}
    </Link>
  );
}

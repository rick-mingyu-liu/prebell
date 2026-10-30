"use client";

import Link from "next/link";
import { usePathname, useSearchParams } from "next/navigation";
import type { Locale } from "@/i18n/routing";

/** The current URL with its locale prefix swapped for `target`, for the shared header and footer. */
export default function LangSwitchLink({
  target,
  hrefLang,
  label,
  ariaLabel,
  className,
}: {
  target: Locale;
  hrefLang: string;
  label: string;
  ariaLabel: string;
  className: string;
}) {
  const rest = usePathname().replace(/^\/(en|zh)(?=\/|$)/, "");
  const query = useSearchParams().toString();
  return (
    <Link
      className={className}
      href={`/${target}${rest}${query ? `?${query}` : ""}`}
      hrefLang={hrefLang}
      lang={hrefLang}
      aria-label={ariaLabel}
    >
      {label}
    </Link>
  );
}

import Link from "next/link";
import type { Locale } from "@/i18n/routing";
import { mcpEnabled } from "@/lib/mcpswitch";
import { translator } from "@/lib/translator";

/**
 * The footer: a link row (About, For AI agents only while the MCP server is on, Privacy & Terms) and the
 * copyright line. The language switch lives in the masthead nav only.
 */
export default function FooterLinks({ locale, year = new Date().getUTCFullYear() }: { locale: Locale; year?: number }) {
  const t = translator(locale);
  return (
    <>
      <p className="footer-links">
        <Link href={`/${locale}/about`}>{t("site.about")}</Link>
        {mcpEnabled() && <Link href={`/${locale}/mcp`}>{t("site.mcp")}</Link>}
        <Link href={`/${locale}/legal`}>{t("site.legal")}</Link>
      </p>
      <p className="footer-copy">
        © {year} {t("site.name")} · prebell.site
      </p>
    </>
  );
}

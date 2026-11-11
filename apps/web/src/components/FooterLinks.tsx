import Link from "next/link";
import type { Locale } from "@/i18n/routing";
import { mcpEnabled } from "@/lib/mcpswitch";
import { translator } from "@/lib/translator";
import LangSwitch from "./LangSwitch";

/** The footer's link row: About, For AI agents (only while the MCP server is on), Privacy & Terms, and the language switch. */
export default function FooterLinks({ locale }: { locale: Locale }) {
  const t = translator(locale);
  return (
    <p className="footer-links">
      <Link href={`/${locale}/about`}>{t("site.about")}</Link>
      {mcpEnabled() && <Link href={`/${locale}/mcp`}>{t("site.mcp")}</Link>}
      <Link href={`/${locale}/legal`}>{t("site.legal")}</Link>
      <LangSwitch locale={locale} />
    </p>
  );
}

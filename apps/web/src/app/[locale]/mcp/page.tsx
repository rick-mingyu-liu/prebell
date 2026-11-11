import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { setRequestLocale } from "next-intl/server";
import type { Locale } from "@/i18n/routing";
import { mcpEnabled } from "@/lib/mcpswitch";
import { NOTICE } from "@/lib/mcptools";
import { pairedAlternates, shareMetadata, siteOrigin } from "@/lib/seo";
import { translator } from "@/lib/translator";

const TOOL_NAMES = ["earnings_calendar", "earnings_report", "latest_brief"] as const;

export async function generateMetadata({ params }: PageProps<"/[locale]/mcp">): Promise<Metadata> {
  const { locale } = await params;
  if (!mcpEnabled()) return {};
  const t = translator(locale as Locale);
  return {
    title: t("mcp.title"),
    description: t("mcp.description"),
    alternates: pairedAlternates(locale, "/mcp"),
    ...shareMetadata({
      locale: locale as Locale,
      url: `/${locale}/mcp`,
      title: t("mcp.title"),
      description: t("mcp.description"),
      siteName: t("site.name"),
    }),
  };
}

/** How a visitor connects their own agent to the MCP server; 404 while the server is off. */
export default async function Mcp({ params }: PageProps<"/[locale]/mcp">) {
  const locale = (await params).locale as Locale;
  if (!mcpEnabled()) notFound();
  setRequestLocale(locale);
  const t = translator(locale);
  const address = new URL("/mcp", await siteOrigin()).href;
  const sep = locale === "zh" ? "：" : ": ";
  // Headings, paragraphs, lists and code blocks stay direct children of the article: the .prose rules are direct-child rules.
  return (
    <article className="prose prose-mcp">
      <h1>{t("mcp.title")}</h1>
      <p>{t("mcp.intro")}</p>
      <h2>{t("mcp.addressHeading")}</h2>
      <pre>
        <code>{address}</code>
      </pre>
      <h2>{t("mcp.connectHeading")}</h2>
      <p>{t("mcp.claudeCode")}</p>
      <pre>
        <code>{`claude mcp add --transport http prebell ${address}`}</code>
      </pre>
      <p>{t("mcp.otherClients")}</p>
      <h2>{t("mcp.toolsHeading")}</h2>
      <ul>
        {TOOL_NAMES.map((name) => (
          <li key={name}>
            <code>{name}</code>
            {sep}
            {t(`mcp.tools.${name}`)}
          </li>
        ))}
      </ul>
      <p>{t("mcp.toolsLang")}</p>
      <h2>{t("mcp.askHeading")}</h2>
      <ul>
        {(t.raw("mcp.ask") as string[]).map((q) => (
          <li key={q}>{q}</li>
        ))}
      </ul>
      <h2>{t("mcp.limitsHeading")}</h2>
      <ul>
        {[...(t.raw("mcp.limits") as string[]), NOTICE[locale]].map((text) => (
          <li key={text}>{text}</li>
        ))}
      </ul>
    </article>
  );
}

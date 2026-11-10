import { getCloudflareContext } from "@opennextjs/cloudflare";
import { getDb } from "@/lib/db";
import { handleMcpGet, handleMcpPost } from "@/lib/mcp";
import { resolveOrigin } from "@/lib/seo";
import type { EdgeCache } from "@/lib/sitemap";

/**
 * The read-only MCP server (src/lib/mcp.ts). `MCP_ENABLED` other than "true" turns it off (404), and only
 * `SITE_URL`'s host (and loopback, for local dev) is served.
 */
export function GET(req: Request) {
  const { env } = getCloudflareContext();
  return handleMcpGet(req, { enabled: env.MCP_ENABLED === "true", siteUrl: env.SITE_URL });
}

export async function POST(req: Request) {
  const { env, ctx } = getCloudflareContext();
  return handleMcpPost(req, {
    enabled: env.MCP_ENABLED === "true",
    db: getDb(),
    bucket: env.FACTS,
    origin: resolveOrigin(env.SITE_URL, req.headers),
    siteUrl: env.SITE_URL,
    now: new Date(),
    // Workers' `caches.default`; absent under `next dev`.
    cache: (globalThis.caches as unknown as { default?: EdgeCache } | undefined)?.default ?? null,
    waitUntil: (p) => ctx.waitUntil(p),
    log: (line) => console.log(line),
    onError: (e) => console.error("mcp facts:", e instanceof Error ? e.message.slice(0, 200) : "unknown"),
  });
}

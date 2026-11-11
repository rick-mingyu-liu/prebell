import "server-only";
import { getCloudflareContext } from "@opennextjs/cloudflare";

/**
 * Server-only. Whether the MCP server at `/mcp` is on: `MCP_ENABLED` exactly "true", as the route reads it.
 * The `/{locale}/mcp` page, its footer link and its sitemap entries follow it, so the site never points at a
 * server that is off. False when there is no Cloudflare context.
 */
export function mcpEnabled(): boolean {
  try {
    return getCloudflareContext().env.MCP_ENABLED === "true";
  } catch {
    return false;
  }
}

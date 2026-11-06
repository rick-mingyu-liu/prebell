import type { MetadataRoute } from "next";
import { getDb } from "@/lib/db";
import { siteOrigin } from "@/lib/seo";
import { sitemapEntries } from "@/lib/sitemap";

// Read from D1 per request (one query), never at build time; next.config caches it at the edge.
export const dynamic = "force-dynamic";

export default async function sitemap(): Promise<MetadataRoute.Sitemap> {
  return sitemapEntries(getDb(), await siteOrigin());
}

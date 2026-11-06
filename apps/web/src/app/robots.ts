import type { MetadataRoute } from "next";
import { siteOrigin } from "@/lib/seo";

// The origin comes from SITE_URL at request time. On prebell.site, Cloudflare's managed robots.txt
// (content signals) is prepended to this file.
export const dynamic = "force-dynamic";

export default async function robots(): Promise<MetadataRoute.Robots> {
  return {
    rules: { userAgent: "*", allow: "/" },
    sitemap: new URL("/sitemap.xml", await siteOrigin()).href,
  };
}

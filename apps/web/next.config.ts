import { existsSync } from "node:fs";
import type { NextConfig } from "next";
import createNextIntlPlugin from "next-intl/plugin";
import { initOpenNextCloudflareForDev } from "@opennextjs/cloudflare";

const nextConfig: NextConfig = {
  poweredByHeader: false,
  // Whether the author photo is shipped, decided once here because the Worker has no file system.
  env: { HAS_AUTHOR_PHOTO: existsSync("public/author.jpg") ? "1" : "0" },
  experimental: { globalNotFound: true },
  async redirects() {
    return [
      // www is a second custom domain on this Worker; every www URL moves to the apex.
      {
        source: "/:path*",
        has: [{ type: "host", value: "www.prebell.site" }],
        destination: "https://prebell.site/:path*",
        permanent: true,
      },
      // Permanent, so search engines fold "/" into "/en" (the site does no language detection).
      { source: "/", destination: "/en", permanent: true },
    ];
  },
  async headers() {
    // No ISR: every page is rendered per request and kept at the edge for its s-maxage by worker.ts
    // (src/lib/edgecache.ts). Posts change rarely after publishing (the 8:35 update, a revision), so they
    // are kept for five minutes; when two rules match, the later one's header wins.
    return [
      {
        source: "/:locale(en|zh)/:path*",
        headers: [{ key: "Cache-Control", value: "public, s-maxage=60" }],
      },
      {
        source: "/:locale(en|zh)/p/:slug",
        headers: [{ key: "Cache-Control", value: "public, s-maxage=300" }],
      },
      {
        // robots.txt reads SITE_URL per request and rarely changes. (The sitemap sets its own header.)
        source: "/robots.txt",
        headers: [{ key: "Cache-Control", value: "public, s-maxage=3600" }],
      },
      {
        // Partner links depend on the visitor's country, so no shared cache may keep them.
        source: "/api/partners",
        headers: [{ key: "Cache-Control", value: "private, no-store" }],
      },
      {
        // A signup response is per request; nothing may keep it.
        source: "/api/subscribe",
        headers: [{ key: "Cache-Control", value: "private, no-store" }],
      },
    ];
  },
};

export default createNextIntlPlugin("./src/i18n/request.ts")(nextConfig);

initOpenNextCloudflareForDev();

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
      { source: "/", destination: "/en", permanent: false },
    ];
  },
  async headers() {
    // No ISR in Plan 1: every page is rendered per request and cached briefly at the edge.
    return [
      {
        source: "/:locale(en|zh)/:path*",
        headers: [{ key: "Cache-Control", value: "public, s-maxage=60" }],
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

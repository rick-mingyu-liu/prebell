import type { NextConfig } from "next";
import createNextIntlPlugin from "next-intl/plugin";
import { initOpenNextCloudflareForDev } from "@opennextjs/cloudflare";

const nextConfig: NextConfig = {
  poweredByHeader: false,
  experimental: { globalNotFound: true },
  async redirects() {
    return [{ source: "/", destination: "/en", permanent: false }];
  },
  async headers() {
    // No ISR in Plan 1: every page is rendered per request and cached briefly at the edge.
    return [
      {
        source: "/:locale(en|zh)/:path*",
        headers: [{ key: "Cache-Control", value: "public, s-maxage=60" }],
      },
      {
        // Internal pages: the token is in the URL, so never cache, index or leak it as a referrer.
        source: "/admin/:path*",
        headers: [
          { key: "Cache-Control", value: "private, no-store" },
          { key: "X-Robots-Tag", value: "noindex, nofollow" },
          { key: "Referrer-Policy", value: "no-referrer" },
        ],
      },
    ];
  },
};

export default createNextIntlPlugin("./src/i18n/request.ts")(nextConfig);

initOpenNextCloudflareForDev();

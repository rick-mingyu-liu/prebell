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
        // Partner links depend on the visitor's country, so no shared cache may keep them.
        source: "/api/partners",
        headers: [{ key: "Cache-Control", value: "private, no-store" }],
      },
    ];
  },
};

export default createNextIntlPlugin("./src/i18n/request.ts")(nextConfig);

initOpenNextCloudflareForDev();

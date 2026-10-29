import { defineCloudflareConfig } from "@opennextjs/cloudflare";

// No incremental cache: pages are rendered per request (Cache-Control s-maxage=60).
export default defineCloudflareConfig({});

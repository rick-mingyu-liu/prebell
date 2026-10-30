// Bindings from wrangler.jsonc, as seen by getCloudflareContext().env.
interface CloudflareEnv {
  DB: import("./src/lib/posts").D1Like;
  /** Generator's R2 bucket; the site reads `charts/` SVGs and posts' facts JSON from it. */
  FACTS: import("./src/lib/earnings").R2Like & import("./src/lib/facts").R2Like;
  ASSETS: { fetch(request: Request): Promise<Response> };
  SITE_URL?: string;
  CF_BEACON_TOKEN?: string;
  /** Secret for /admin/competitor; unset means the page always 404s. */
  ADMIN_TOKEN?: string;
}

// Bindings from wrangler.jsonc, as seen by getCloudflareContext().env.
interface CloudflareEnv {
  DB: import("./src/lib/posts").D1Like;
  ASSETS: { fetch(request: Request): Promise<Response> };
  SITE_URL?: string;
  CF_BEACON_TOKEN?: string;
}

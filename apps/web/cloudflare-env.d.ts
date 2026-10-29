// Bindings from wrangler.jsonc, as seen by getCloudflareContext().env.
interface CloudflareEnv {
  DB: import("./src/lib/posts").D1Like;
  ASSETS: { fetch(request: Request): Promise<Response> };
  CF_BEACON_TOKEN?: string;
}

// The Worker's entry (`main` in wrangler.jsonc): the OpenNext worker, with page HTML kept in the edge cache
// and plain HTTP redirected to HTTPS first (src/lib/edgecache.ts). See OpenNext's "custom worker" guide.
// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-ignore: built by `opennextjs-cloudflare build`, absent before it
import { default as handler } from "./.open-next/worker.js";
import { cachedPage, httpsRedirect, type PageCache } from "./src/lib/edgecache";

/** The slice of the Workers execution context used here (the Workers runtime types are not loaded in this app). */
interface Ctx {
  waitUntil(p: Promise<unknown>): void;
}

const worker = {
  async fetch(request: Request, env: CloudflareEnv, ctx: Ctx): Promise<Response> {
    const redirect = httpsRedirect(request, env.SITE_URL);
    if (redirect) return redirect;
    const cache = (globalThis.caches as unknown as { default?: PageCache } | undefined)?.default;
    // The deployed version (`version_metadata` binding) keys the page cache; see pageCacheKey.
    const version = (env as { CF_VERSION_METADATA?: { id?: string } }).CF_VERSION_METADATA?.id;
    return cachedPage(
      request,
      cache,
      version,
      (p) => ctx.waitUntil(p),
      () => handler.fetch(request, env, ctx),
    );
  },
};

export default worker;

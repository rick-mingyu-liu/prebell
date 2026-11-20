/**
 * Page HTML in this colo's edge cache, and the HTTP to HTTPS redirect, run by `worker.ts` before the
 * OpenNext handler. A Worker's own responses are not cached by `Cache-Control` alone, so before this every
 * page view and every crawler hit rendered the page again (26 to 334 ms of CPU each against the Free plan's
 * 10 ms; a burst of about 300 requests on 2026-11-18 got 503 "error code: 1102" at one colo).
 */

/** The slice of the Workers Cache API (`caches.default`) used here. */
export interface PageCache {
  match(request: Request): Promise<Response | undefined>;
  put(request: Request, response: Response): Promise<void>;
}

/** Router and server-action requests carry one of these and get a different body from the same URL. */
const NEXT_HEADERS = [
  "rsc",
  "next-action",
  "next-router-prefetch",
  "next-router-state-tree",
  "next-router-segment-prefetch",
] as const;

/**
 * The query parameters a page reads, by path after the locale; every other parameter (tracking, cache busters)
 * is dropped from the key, since the page renders the same without it.
 */
const PAGE_PARAMS: Record<string, readonly string[]> = {
  "/earnings": ["week"],
  "/archive": ["type", "page"],
};

/** The page's own `Cache-Control`, kept on the stored copy and put back on a hit. */
const ORIGINAL_CC = "x-page-cache-control";

/**
 * The cache key for a page request, or null when it must not be cached: anything but a plain GET or HEAD for a
 * page under `/en` or `/zh`. The key is the URL with only the parameters its page reads (sorted), plus the
 * deployed version (`v`), so a new deploy never serves HTML that names the old build's script and style files
 * (they are gone with the old version). No headers: a page's HTML is the same for every visitor.
 */
export function pageCacheKey(req: Request, version: string | undefined): Request | null {
  if (req.method !== "GET" && req.method !== "HEAD") return null;
  const url = new URL(req.url);
  const m = /^\/(en|zh)(\/.*)?$/.exec(url.pathname);
  if (!m) return null;
  if (url.searchParams.has("_rsc") || NEXT_HEADERS.some((h) => req.headers.has(h))) return null;
  const keep = PAGE_PARAMS[m[2] ?? ""] ?? [];
  const query = new URLSearchParams();
  for (const k of keep) {
    const v = url.searchParams.get(k);
    if (v !== null) query.set(k, v);
  }
  if (version) query.set("v", version);
  url.search = query.toString();
  url.hash = "";
  return new Request(url.toString());
}

/**
 * How long the edge may keep a response, from its own `s-maxage` (set per route in `next.config.ts`), or 0:
 * only a 200 HTML page with no cookie and a public `s-maxage` is kept.
 */
export function pageTtl(res: Response): number {
  if (res.status !== 200 || res.headers.has("set-cookie")) return 0;
  if (!(res.headers.get("content-type") ?? "").startsWith("text/html")) return 0;
  const cc = res.headers.get("cache-control") ?? "";
  if (/\b(private|no-store|no-cache)\b/.test(cc)) return 0;
  const m = /\bs-maxage=(\d+)/.exec(cc);
  return m ? Number(m[1]) : 0;
}

/**
 * A page from the edge cache, else `render()` (stored on the way out when `pageTtl` allows). The stored copy
 * carries `max-age`, which the Cache API reads; a hit gets the page's own `Cache-Control` back, so browsers
 * never keep a page longer than before. Cache errors fall through to rendering.
 */
export async function cachedPage(
  req: Request,
  cache: PageCache | undefined,
  version: string | undefined,
  waitUntil: (p: Promise<unknown>) => void,
  render: () => Promise<Response>,
): Promise<Response> {
  const key = cache ? pageCacheKey(req, version) : null;
  if (!cache || !key) return render();
  const hit = await cache.match(key).catch(() => undefined);
  if (hit) {
    // HEAD (uptime and link checkers) gets the stored page's headers without its body.
    const out = new Response(req.method === "HEAD" ? null : hit.body, hit);
    const cc = out.headers.get(ORIGINAL_CC);
    if (cc) out.headers.set("cache-control", cc);
    out.headers.delete(ORIGINAL_CC);
    out.headers.set("x-page-cache", "hit");
    return out;
  }
  const res = await render();
  const ttl = pageTtl(res);
  // A HEAD render has no body to store.
  if (ttl <= 0 || !res.body || req.method !== "GET") return res;
  const [mine, theirs] = res.body.tee();
  const stored = new Response(mine, res);
  stored.headers.set(ORIGINAL_CC, res.headers.get("cache-control") ?? "");
  stored.headers.set("cache-control", `public, max-age=${ttl}`);
  waitUntil(cache.put(key, stored).catch((e: unknown) => console.error("page cache put:", e)));
  const out = new Response(theirs, res);
  out.headers.set("x-page-cache", "miss");
  return out;
}

/**
 * A permanent redirect (308, so a POST stays a POST) to HTTPS on the apex for a plain-HTTP request to the
 * site's own hosts (its domain and www, which goes straight to the apex), else null. Other hosts (workers.dev,
 * localhost) are left alone. On Cloudflare an HTTP request has an `http:` URL and says so in
 * `x-forwarded-proto` and `cf-visitor`; `wrangler dev` rewrites the URL to the route's host over http but sends
 * neither header, so requiring both keeps local runs from redirecting to themselves. A client that forges a
 * header only gets sent to HTTPS.
 */
export function httpsRedirect(req: Request, siteUrl: string | undefined): Response | null {
  if (!siteUrl) return null;
  const url = new URL(req.url);
  if (url.protocol !== "http:") return null;
  const forwarded =
    req.headers.get("x-forwarded-proto") === "http" || /"scheme"\s*:\s*"http"/.test(req.headers.get("cf-visitor") ?? "");
  if (!forwarded) return null;
  const apex = new URL(siteUrl).hostname;
  if (url.hostname !== apex && url.hostname !== `www.${apex}`) return null;
  url.protocol = "https:";
  url.hostname = apex;
  return new Response(null, { status: 308, headers: { location: url.toString() } });
}

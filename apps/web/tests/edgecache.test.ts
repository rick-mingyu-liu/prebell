import { describe, expect, it, vi } from "vitest";
import { cachedPage, httpsRedirect, pageCacheKey, pageTtl, type PageCache } from "../src/lib/edgecache";

const html = (cc: string, init: ResponseInit = {}) =>
  new Response("<html>page</html>", {
    status: 200,
    ...init,
    headers: { "content-type": "text/html; charset=utf-8", "cache-control": cc, ...(init.headers ?? {}) },
  });

/** An in-memory stand-in for `caches.default`, keyed by URL. */
function memoryCache() {
  const store = new Map<string, Response>();
  const cache: PageCache = {
    match: async (r) => store.get(r.url)?.clone(),
    put: async (r, res) => {
      store.set(r.url, new Response(await res.text(), res));
    },
  };
  return { cache, store };
}

describe("pageCacheKey", () => {
  it("keys plain page GETs by URL, without tracking parameters and with the query sorted", () => {
    const key = pageCacheKey(
      new Request("https://prebell.site/en/earnings?week=2026-11-09&utm_source=x&fbclid=1&a=2", {
        headers: { "accept-language": "zh" },
      }),
    );
    expect(key?.url).toBe("https://prebell.site/en/earnings?a=2&week=2026-11-09");
    expect([...(key?.headers.keys() ?? [])]).toEqual([]);
    expect(pageCacheKey(new Request("https://prebell.site/zh"))?.url).toBe("https://prebell.site/zh");
  });

  it("skips non-GET, non-page paths and Next router requests", () => {
    expect(pageCacheKey(new Request("https://prebell.site/en", { method: "POST", body: "x" }))).toBeNull();
    expect(pageCacheKey(new Request("https://prebell.site/en", { method: "HEAD" }))).toBeNull();
    for (const p of ["/", "/mcp", "/sitemap.xml", "/api/partners", "/covers/a.svg", "/english"]) {
      expect(pageCacheKey(new Request(`https://prebell.site${p}`))).toBeNull();
    }
    expect(pageCacheKey(new Request("https://prebell.site/en/archive?_rsc=abc"))).toBeNull();
    expect(pageCacheKey(new Request("https://prebell.site/en", { headers: { rsc: "1" } }))).toBeNull();
    expect(pageCacheKey(new Request("https://prebell.site/en", { headers: { "next-action": "x" } }))).toBeNull();
  });
});

describe("pageTtl", () => {
  it("reads s-maxage from a public 200 HTML page", () => {
    expect(pageTtl(html("public, s-maxage=60"))).toBe(60);
    expect(pageTtl(html("public, s-maxage=300"))).toBe(300);
  });

  it("keeps nothing that is not a cacheable 200 HTML page", () => {
    expect(pageTtl(html("public, s-maxage=60", { status: 404 }))).toBe(0);
    expect(pageTtl(html("private, no-cache, no-store, max-age=0, must-revalidate"))).toBe(0);
    expect(pageTtl(html("public, max-age=60"))).toBe(0);
    expect(pageTtl(html("public, s-maxage=60", { headers: { "set-cookie": "a=b" } }))).toBe(0);
    expect(
      pageTtl(new Response("{}", { headers: { "content-type": "application/json", "cache-control": "s-maxage=60" } })),
    ).toBe(0);
  });
});

describe("cachedPage", () => {
  it("renders once, stores the page with max-age, and serves the next request from the cache", async () => {
    const { cache, store } = memoryCache();
    const pending: Promise<unknown>[] = [];
    const render = vi.fn(async () => html("public, s-maxage=300"));
    const first = await cachedPage(new Request("https://prebell.site/en/p/x"), cache, (p) => pending.push(p), render);
    expect(first.headers.get("x-page-cache")).toBe("miss");
    expect(await first.text()).toBe("<html>page</html>");
    await Promise.all(pending);
    expect(store.get("https://prebell.site/en/p/x")?.headers.get("cache-control")).toBe("public, max-age=300");

    const second = await cachedPage(new Request("https://prebell.site/en/p/x?utm_source=t"), cache, () => {}, render);
    expect(render).toHaveBeenCalledTimes(1);
    expect(second.headers.get("x-page-cache")).toBe("hit");
    // Browsers get the page's own header back, never the stored max-age.
    expect(second.headers.get("cache-control")).toBe("public, s-maxage=300");
    expect(second.headers.has("x-page-cache-control")).toBe(false);
    expect(await second.text()).toBe("<html>page</html>");
  });

  it("does not store a 404 or a router request, and renders when the cache fails", async () => {
    const { cache, store } = memoryCache();
    await cachedPage(new Request("https://prebell.site/en/p/none"), cache, () => {}, async () =>
      html("public, s-maxage=60", { status: 404 }),
    );
    await cachedPage(new Request("https://prebell.site/en", { headers: { rsc: "1" } }), cache, () => {}, async () =>
      html("public, s-maxage=60"),
    );
    expect(store.size).toBe(0);

    const broken: PageCache = {
      match: async () => {
        throw new Error("down");
      },
      put: async () => {
        throw new Error("down");
      },
    };
    const pending: Promise<unknown>[] = [];
    const errors = vi.spyOn(console, "error").mockImplementation(() => {});
    const res = await cachedPage(new Request("https://prebell.site/en"), broken, (p) => pending.push(p), async () =>
      html("public, s-maxage=60"),
    );
    expect(await res.text()).toBe("<html>page</html>");
    await Promise.all(pending);
    expect(errors).toHaveBeenCalled();
    errors.mockRestore();
  });

  it("renders directly without a cache (next dev)", async () => {
    const res = await cachedPage(new Request("https://prebell.site/en"), undefined, () => {}, async () =>
      html("public, s-maxage=60"),
    );
    expect(res.headers.has("x-page-cache")).toBe(false);
  });
});

describe("httpsRedirect", () => {
  const http = (url: string) => new Request(url, { headers: { "x-forwarded-proto": "http" } });

  it("moves plain HTTP on the site's hosts to HTTPS, path and query kept", () => {
    const r = httpsRedirect(http("http://prebell.site/en/p/x?week=1"), "https://prebell.site");
    expect(r?.status).toBe(301);
    expect(r?.headers.get("location")).toBe("https://prebell.site/en/p/x?week=1");
    expect(httpsRedirect(http("http://www.prebell.site/"), "https://prebell.site")?.headers.get("location")).toBe(
      "https://www.prebell.site/",
    );
  });

  it("leaves HTTPS, other hosts, an unset SITE_URL and wrangler dev (no forwarded scheme) alone", () => {
    expect(httpsRedirect(new Request("https://prebell.site/en", { headers: { "x-forwarded-proto": "https" } }), "https://prebell.site")).toBeNull();
    expect(httpsRedirect(http("http://localhost:8787/en"), "https://prebell.site")).toBeNull();
    expect(httpsRedirect(http("http://prebell-web.x.workers.dev/en"), "https://prebell.site")).toBeNull();
    expect(httpsRedirect(http("http://prebell.site/en"), undefined)).toBeNull();
    expect(httpsRedirect(new Request("http://prebell.site/en"), "https://prebell.site")).toBeNull();
  });
});

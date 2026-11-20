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
  it("keys a page by URL with only the parameters its page reads, sorted, plus the version, and no headers", () => {
    const key = pageCacheKey(
      new Request("https://prebell.site/en/earnings?week=2026-11-09&utm_source=x&fbclid=1&cb=123", {
        headers: { "accept-language": "zh", "user-agent": "Twitterbot" },
      }),
      "ver1",
    );
    expect(key?.url).toBe("https://prebell.site/en/earnings?week=2026-11-09&v=ver1");
    expect([...(key?.headers.keys() ?? [])]).toEqual([]);
    expect(pageCacheKey(new Request("https://prebell.site/zh/archive?page=2&x=1&type=recap"), "v")?.url).toBe(
      "https://prebell.site/zh/archive?type=recap&page=2&v=v",
    );
    // A page that reads no parameters drops them all; a cache buster cannot force a render.
    expect(pageCacheKey(new Request("https://prebell.site/en/p/x?nocache=8&week=1"), "v")?.url).toBe(
      "https://prebell.site/en/p/x?v=v",
    );
    expect(pageCacheKey(new Request("https://prebell.site/zh"), undefined)?.url).toBe("https://prebell.site/zh");
    // HEAD shares the GET key.
    expect(pageCacheKey(new Request("https://prebell.site/en", { method: "HEAD" }), "v")?.url).toBe(
      "https://prebell.site/en?v=v",
    );
  });

  it("a new version is a new key, so a deploy never serves the old build's HTML", () => {
    const r = new Request("https://prebell.site/en");
    expect(pageCacheKey(r, "a")?.url).not.toBe(pageCacheKey(r, "b")?.url);
  });

  it("skips non-GET/HEAD, non-page paths and Next router requests", () => {
    expect(pageCacheKey(new Request("https://prebell.site/en", { method: "POST", body: "x" }), "v")).toBeNull();
    for (const p of ["/", "/mcp", "/sitemap.xml", "/api/partners", "/covers/a.svg", "/english"]) {
      expect(pageCacheKey(new Request(`https://prebell.site${p}`), "v")).toBeNull();
    }
    expect(pageCacheKey(new Request("https://prebell.site/en/archive?_rsc=abc"), "v")).toBeNull();
    expect(pageCacheKey(new Request("https://prebell.site/en", { headers: { rsc: "1" } }), "v")).toBeNull();
    expect(pageCacheKey(new Request("https://prebell.site/en", { headers: { "next-action": "x" } }), "v")).toBeNull();
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
    const first = await cachedPage(new Request("https://prebell.site/en/p/x"), cache, "v1", (p) => pending.push(p), render);
    expect(first.headers.get("x-page-cache")).toBe("miss");
    expect(await first.text()).toBe("<html>page</html>");
    await Promise.all(pending);
    expect(store.get("https://prebell.site/en/p/x?v=v1")?.headers.get("cache-control")).toBe("public, max-age=300");

    const second = await cachedPage(new Request("https://prebell.site/en/p/x?utm_source=t"), cache, "v1", () => {}, render);
    expect(render).toHaveBeenCalledTimes(1);
    expect(second.headers.get("x-page-cache")).toBe("hit");
    // Browsers get the page's own header back, never the stored max-age.
    expect(second.headers.get("cache-control")).toBe("public, s-maxage=300");
    expect(second.headers.has("x-page-cache-control")).toBe(false);
    expect(await second.text()).toBe("<html>page</html>");
  });

  it("does not store a 404 or a router request, and renders when the cache fails", async () => {
    const { cache, store } = memoryCache();
    await cachedPage(new Request("https://prebell.site/en/p/none"), cache, "v1", () => {}, async () =>
      html("public, s-maxage=60", { status: 404 }),
    );
    await cachedPage(new Request("https://prebell.site/en", { headers: { rsc: "1" } }), cache, "v1", () => {}, async () =>
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
    const res = await cachedPage(new Request("https://prebell.site/en"), broken, "v1", (p) => pending.push(p), async () =>
      html("public, s-maxage=60"),
    );
    expect(await res.text()).toBe("<html>page</html>");
    await Promise.all(pending);
    expect(errors).toHaveBeenCalled();
    errors.mockRestore();
  });

  it("answers HEAD from a stored page without a body, and never stores a HEAD render", async () => {
    const { cache, store } = memoryCache();
    const render = vi.fn(async () => html("public, s-maxage=60"));
    const head = await cachedPage(new Request("https://prebell.site/en", { method: "HEAD" }), cache, "v1", () => {}, render);
    expect(head.headers.has("x-page-cache")).toBe(false);
    expect(store.size).toBe(0);
    const pending: Promise<unknown>[] = [];
    await (await cachedPage(new Request("https://prebell.site/en"), cache, "v1", (p) => pending.push(p), render)).text();
    await Promise.all(pending);
    const hit = await cachedPage(new Request("https://prebell.site/en", { method: "HEAD" }), cache, "v1", () => {}, render);
    expect(hit.headers.get("x-page-cache")).toBe("hit");
    expect(hit.body).toBeNull();
    expect(render).toHaveBeenCalledTimes(2);
  });

  it("does not store a body that fails mid-stream", async () => {
    const { cache, store } = memoryCache();
    const pending: Promise<unknown>[] = [];
    const errors = vi.spyOn(console, "error").mockImplementation(() => {});
    const body = new ReadableStream<Uint8Array>({
      start(c) {
        c.enqueue(new TextEncoder().encode("<html>"));
        c.error(new Error("render failed"));
      },
    });
    const res = await cachedPage(new Request("https://prebell.site/en"), cache, "v1", (p) => pending.push(p), async () =>
      new Response(body, { headers: { "content-type": "text/html", "cache-control": "public, s-maxage=60" } }),
    );
    await expect(res.text()).rejects.toThrow("render failed");
    await Promise.all(pending);
    expect(store.size).toBe(0);
    errors.mockRestore();
  });

  it("renders directly without a cache (next dev)", async () => {
    const res = await cachedPage(new Request("https://prebell.site/en"), undefined, "v1", () => {}, async () =>
      html("public, s-maxage=60"),
    );
    expect(res.headers.has("x-page-cache")).toBe(false);
  });
});

describe("httpsRedirect", () => {
  const http = (url: string, headers: Record<string, string> = { "x-forwarded-proto": "http" }) =>
    new Request(url, { headers });

  it("moves plain HTTP on the site's hosts to HTTPS on the apex with a 308, path and query kept", () => {
    const r = httpsRedirect(http("http://prebell.site/en/p/x?week=1"), "https://prebell.site");
    expect(r?.status).toBe(308);
    expect(r?.headers.get("location")).toBe("https://prebell.site/en/p/x?week=1");
    expect(httpsRedirect(http("http://www.prebell.site/zh"), "https://prebell.site")?.headers.get("location")).toBe(
      "https://prebell.site/zh",
    );
    // Cloudflare's own scheme header is enough.
    expect(
      httpsRedirect(http("http://prebell.site/en", { "cf-visitor": '{"scheme":"http"}' }), "https://prebell.site")?.status,
    ).toBe(308);
  });

  it("leaves HTTPS, other hosts, an unset SITE_URL and wrangler dev (no forwarded scheme) alone", () => {
    expect(httpsRedirect(http("https://prebell.site/en", { "x-forwarded-proto": "https" }), "https://prebell.site")).toBeNull();
    expect(httpsRedirect(http("http://localhost:8787/en"), "https://prebell.site")).toBeNull();
    expect(httpsRedirect(http("http://prebell-web.x.workers.dev/en"), "https://prebell.site")).toBeNull();
    expect(httpsRedirect(http("http://prebell.site/en"), undefined)).toBeNull();
    expect(httpsRedirect(http("http://prebell.site/en", {}), "https://prebell.site")).toBeNull();
    expect(
      httpsRedirect(http("http://prebell.site/en", { "cf-visitor": '{"scheme":"https"}' }), "https://prebell.site"),
    ).toBeNull();
  });
});

import { existsSync } from "node:fs";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { getPathMatch } from "next/dist/shared/lib/router/utils/path-match";
import { beforeEach, describe, expect, it, vi } from "vitest";
import nextConfig from "../next.config";
import { GET } from "../src/app/api/partners/route";
import { PartnerNames } from "../src/components/PartnerLinks";
import PartnersStrip from "../src/components/PartnersStrip";
import { countryOf, partnersFor, type PartnersResponse } from "../src/lib/partners";

const cf = vi.hoisted(() => ({
  value: undefined as { country?: unknown } | undefined,
  throws: false,
}));
vi.mock("@opennextjs/cloudflare", () => ({
  getCloudflareContext: () => {
    if (cf.throws) throw new Error("no Cloudflare context");
    return { cf: cf.value };
  },
  initOpenNextCloudflareForDev: () => {},
}));

const ids = (country: string | null) => partnersFor(country).map((p) => p.id);
const req = (headers: Record<string, string> = {}) =>
  new Request("https://prebell.example/api/partners", { headers });

describe("partnersFor", () => {
  it("shows nothing in mainland China", () => {
    expect(ids("CN")).toEqual([]);
  });
  it("shows all four in Canada", () => {
    expect(ids("CA")).toEqual(["moomoo", "wealthsimple", "okx", "cryptocom"]);
  });
  it("shows everything but Wealthsimple in the US", () => {
    expect(ids("US")).toEqual(["moomoo", "okx", "cryptocom"]);
  });
  it("treats an unknown country like the US", () => {
    expect(ids(null)).toEqual(ids("US"));
  });
  it("uses the partner URLs verbatim", () => {
    expect(Object.fromEntries(partnersFor("CA").map((p) => [p.id, p.url]))).toEqual({
      moomoo: "https://j.moomoo.com/0J1BGf",
      wealthsimple: "https://wealthsimple.com/invite/QDBQ5Z",
      okx: "https://okx.com/join/AIAFMP3U",
      cryptocom: "https://crypto.com/app/smqqda7396",
    });
  });
});

describe("countryOf", () => {
  it("reads and normalizes the runtime cf country", () => {
    expect(countryOf(req(), { country: "ca" })).toBe("CA");
    expect(countryOf(req(), { country: " CN " })).toBe("CN");
    const withCf = Object.assign(req(), { cf: { country: "US" } });
    expect(countryOf(withCf)).toBe("US");
  });
  it("prefers the runtime cf country, which the client can't forge, over the header", () => {
    expect(countryOf(req({ "cf-ipcountry": "US" }), { country: "CN" })).toBe("CN");
  });
  it("falls back to a normalized cf-ipcountry header when cf has no country", () => {
    expect(countryOf(req({ "cf-ipcountry": "ca" }))).toBe("CA");
    expect(countryOf(req({ "cf-ipcountry": "ca" }), {})).toBe("CA");
    expect(countryOf(req({ "cf-ipcountry": "CN" }), { country: "XX" })).toBe("CN");
  });
  it("ignores unknown, Tor, empty and missing values", () => {
    expect(countryOf(req({ "cf-ipcountry": "XX" }))).toBeNull();
    expect(countryOf(req({ "cf-ipcountry": "T1" }))).toBeNull();
    expect(countryOf(req({ "cf-ipcountry": "" }))).toBeNull();
    expect(countryOf(req())).toBeNull();
    expect(countryOf(req(), { country: "T1" })).toBeNull();
    expect(countryOf(req({ "cf-ipcountry": "XX" }), { country: "XX" })).toBeNull();
  });
});

describe("GET /api/partners", () => {
  beforeEach(() => {
    cf.value = undefined;
    cf.throws = false;
  });

  it("is never cached and returns the grouped links", async () => {
    const res = await GET(req({ "cf-ipcountry": "CA" }));
    expect(res.headers.get("Cache-Control")).toBe("private, no-store");
    expect(await res.json()).toEqual({
      brokers: [
        { id: "moomoo", name: "moomoo", url: "https://j.moomoo.com/0J1BGf", logo: "/logos/partners/moomoo.png" },
        {
          id: "wealthsimple",
          name: "Wealthsimple",
          url: "https://wealthsimple.com/invite/QDBQ5Z",
          logo: "/logos/partners/wealthsimple.png",
        },
      ],
      crypto: [
        { id: "okx", name: "OKX", url: "https://okx.com/join/AIAFMP3U", logo: "/logos/partners/okx.svg" },
        {
          id: "cryptocom",
          name: "Crypto.com",
          url: "https://crypto.com/app/smqqda7396",
          logo: "/logos/partners/cryptocom.png",
        },
      ],
    });
  });

  it("returns empty groups in mainland China", async () => {
    cf.value = { country: "CN" };
    const res = await GET(req());
    expect(await res.json()).toEqual({ brokers: [], crypto: [] });
  });

  it("uses the Cloudflare context country over a forged header", async () => {
    cf.value = { country: "CN" };
    const res = await GET(req({ "cf-ipcountry": "US" }));
    expect(await res.json()).toEqual({ brokers: [], crypto: [] });
  });

  it("shows everything but Wealthsimple when the country is unknown", async () => {
    const body = await (await GET(req())).json();
    expect(body.brokers.map((p: { id: string }) => p.id)).toEqual(["moomoo"]);
    expect(body.crypto.map((p: { id: string }) => p.id)).toEqual(["okx", "cryptocom"]);
  });

  it("treats a missing Cloudflare context as no cf country instead of failing", async () => {
    cf.throws = true;
    const unknown = await GET(req());
    expect(unknown.status).toBe(200);
    expect(unknown.headers.get("Cache-Control")).toBe("private, no-store");
    expect((await unknown.json()).brokers.map((p: { id: string }) => p.id)).toEqual(["moomoo"]);
    const cn = await GET(req({ "cf-ipcountry": "CN" }));
    expect(await cn.json()).toEqual({ brokers: [], crypto: [] });
  });
});

describe("next.config headers", () => {
  const rulesFor = async (path: string) =>
    (await nextConfig.headers!()).filter((rule) => getPathMatch(rule.source)(path) !== false);

  it("marks /api/subscribe private, no-store too", async () => {
    const rules = await rulesFor("/api/subscribe");
    const values = rules.flatMap((r) => r.headers).filter((h) => h.key.toLowerCase() === "cache-control");
    expect(values.map((h) => h.value)).toEqual(["private, no-store"]);
  });

  it("marks /api/partners private, no-store and keeps the cacheable locale rule off it", async () => {
    const rules = await rulesFor("/api/partners");
    const values = rules.flatMap((r) => r.headers).filter((h) => h.key.toLowerCase() === "cache-control");
    expect(values.map((h) => h.value)).toEqual(["private, no-store"]);
  });

  it("still caches locale pages at the edge", async () => {
    const rules = await rulesFor("/en/archive");
    expect(rules.flatMap((r) => r.headers)).toContainEqual({ key: "Cache-Control", value: "public, s-maxage=60" });
  });
});

describe("PartnersStrip", () => {
  const PARTNER_HOSTS = ["moomoo.com", "okx.com", "crypto.com", "wealthsimple.com"];

  it("renders the heading and the disclosure, no partner hosts server-side", () => {
    const out = renderToStaticMarkup(createElement(PartnersStrip, { locale: "en" }));
    expect(out).toContain('<aside class="partners-strip"');
    expect(out).toContain("From our partners");
    expect(out).toContain('<p class="partners-note">Affiliate links: PreBell may earn a reward if you sign up. Not investment advice.</p>');
    for (const host of PARTNER_HOSTS) expect(out).not.toContain(host);
    expect(out).not.toContain("/logos/partners/");
  });

  it("uses the Chinese copy on ZH pages", () => {
    const out = renderToStaticMarkup(createElement(PartnersStrip, { locale: "zh" }));
    expect(out).toContain("合作伙伴");
    expect(out).toContain("推广链接：通过链接注册，PreBell 可能获得奖励。不构成投资建议。");
  });

  it("lists the partner names as plain sponsored links, no logos, no group headings", () => {
    const out = renderToStaticMarkup(
      createElement(PartnerNames, {
        data: {
          brokers: [{ id: "moomoo", name: "moomoo", url: "https://j.moomoo.com/0J1BGf", logo: "/logos/partners/moomoo.png" }],
          crypto: [{ id: "okx", name: "OKX", url: "https://okx.com/join/AIAFMP3U" }],
        },
      }),
    );
    expect(out).toContain('<a href="https://j.moomoo.com/0J1BGf" target="_blank" rel="sponsored noopener noreferrer">moomoo</a>');
    expect(out).toContain('rel="sponsored noopener noreferrer">OKX</a>');
    expect(out).not.toContain("<img");
    expect(out).not.toContain("Brokers");
  });
});

describe("PartnerNames", () => {
  const okx = { id: "okx", name: "OKX", url: "https://okx.com/join/AIAFMP3U" };
  const render = (data: PartnersResponse | null) => renderToStaticMarkup(createElement(PartnerNames, { data }));

  it("drops an empty group and keeps the other's links", () => {
    const out = render({ brokers: [], crypto: [okx] });
    expect(out).toBe('<a href="https://okx.com/join/AIAFMP3U" target="_blank" rel="sponsored noopener noreferrer">OKX</a>');
  });

  it("keeps every bundled partner logo on disk", () => {
    for (const p of partnersFor("CA"))
      for (const path of [p.logo, p.logoDark].filter(Boolean))
        expect(existsSync(new URL(`../public${path}`, import.meta.url))).toBe(true);
  });

  it("renders nothing when both groups are empty", () => {
    expect(render({ brokers: [], crypto: [] })).toBe("");
  });

  it("renders nothing before data arrives or after a failed load", () => {
    expect(render(null)).toBe("");
  });
});

describe("loadPartners", () => {
  const good: PartnersResponse = {
    brokers: [{ id: "moomoo", name: "moomoo", url: "https://j.moomoo.com/0J1BGf" }],
    crypto: [],
  };
  const json = (body: unknown, status = 200) => Promise.resolve(Response.json(body, { status }));

  // A fresh module per test, so the module-level memo starts empty.
  const load = async () => (await import("../src/components/PartnerLinks")).loadPartners;
  beforeEach(() => {
    vi.resetModules();
  });

  it("fails closed when the fetch rejects", async () => {
    const loadPartners = await load();
    const fetchImpl = vi.fn(() => Promise.reject(new TypeError("offline")));
    await expect(loadPartners(fetchImpl)).resolves.toBeNull();
  });

  it("fails closed on a non-OK response or a malformed body", async () => {
    const loadPartners = await load();
    await expect(loadPartners(() => json(good, 500))).resolves.toBeNull();
    await expect(loadPartners(() => json({ brokers: "x" }))).resolves.toBeNull();
    await expect(loadPartners(() => Promise.resolve(new Response("not json")))).resolves.toBeNull();
  });

  it("retries on a later mount after a failure", async () => {
    const loadPartners = await load();
    const fetchImpl = vi
      .fn<() => Promise<Response>>()
      .mockImplementationOnce(() => Promise.reject(new TypeError("offline")))
      .mockImplementationOnce(() => json(good));
    await expect(loadPartners(fetchImpl)).resolves.toBeNull();
    await expect(loadPartners(fetchImpl)).resolves.toEqual(good);
    expect(fetchImpl).toHaveBeenCalledTimes(2);
  });

  it("fetches once per page load and shares the result across mounts", async () => {
    const loadPartners = await load();
    const fetchImpl = vi.fn(() => json(good));
    const [a, b] = await Promise.all([loadPartners(fetchImpl), loadPartners(fetchImpl)]);
    const c = await loadPartners(fetchImpl);
    expect(a).toEqual(good);
    expect(b).toBe(a);
    expect(c).toBe(a);
    expect(fetchImpl).toHaveBeenCalledTimes(1);
    expect(fetchImpl).toHaveBeenCalledWith("/api/partners");
  });
});

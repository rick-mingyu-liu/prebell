import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { GET } from "../src/app/api/partners/route";
import { PartnerBoxes } from "../src/components/PartnerLinks";
import Sidebar from "../src/components/Sidebar";
import { countryOf, partnersFor } from "../src/lib/partners";

const cf = vi.hoisted(() => ({ value: undefined as { country?: unknown } | undefined }));
vi.mock("@opennextjs/cloudflare", () => ({ getCloudflareContext: () => ({ cf: cf.value }) }));

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
  it("reads and normalizes cf-ipcountry", () => {
    expect(countryOf(req({ "cf-ipcountry": "ca" }))).toBe("CA");
    expect(countryOf(req({ "cf-ipcountry": " CN " }))).toBe("CN");
  });
  it("ignores unknown, Tor and empty values", () => {
    expect(countryOf(req({ "cf-ipcountry": "XX" }))).toBeNull();
    expect(countryOf(req({ "cf-ipcountry": "T1" }))).toBeNull();
    expect(countryOf(req({ "cf-ipcountry": "" }))).toBeNull();
    expect(countryOf(req())).toBeNull();
  });
  it("falls back to the request's cf country", () => {
    expect(countryOf(req(), { country: "ca" })).toBe("CA");
    expect(countryOf(req({ "cf-ipcountry": "XX" }), { country: "CN" })).toBe("CN");
    expect(countryOf(req(), { country: "XX" })).toBeNull();
    const withCf = Object.assign(req(), { cf: { country: "US" } });
    expect(countryOf(withCf)).toBe("US");
  });
  it("prefers the header over cf", () => {
    expect(countryOf(req({ "cf-ipcountry": "CN" }), { country: "CA" })).toBe("CN");
  });
});

describe("GET /api/partners", () => {
  beforeEach(() => {
    cf.value = undefined;
  });

  it("is never cached and returns the grouped links", async () => {
    const res = await GET(req({ "cf-ipcountry": "CA" }));
    expect(res.headers.get("Cache-Control")).toBe("private, no-store");
    expect(await res.json()).toEqual({
      brokers: [
        { id: "moomoo", name: "moomoo", url: "https://j.moomoo.com/0J1BGf" },
        { id: "wealthsimple", name: "Wealthsimple", url: "https://wealthsimple.com/invite/QDBQ5Z" },
      ],
      crypto: [
        { id: "okx", name: "OKX", url: "https://okx.com/join/AIAFMP3U" },
        { id: "cryptocom", name: "Crypto.com", url: "https://crypto.com/app/smqqda7396" },
      ],
    });
  });

  it("returns empty groups in mainland China", async () => {
    const res = await GET(req({ "cf-ipcountry": "CN" }));
    expect(await res.json()).toEqual({ brokers: [], crypto: [] });
  });

  it("falls back to the Cloudflare context country", async () => {
    cf.value = { country: "CN" };
    const res = await GET(req());
    expect(await res.json()).toEqual({ brokers: [], crypto: [] });
  });

  it("shows everything but Wealthsimple when the country is unknown", async () => {
    const body = await (await GET(req())).json();
    expect(body.brokers.map((p: { id: string }) => p.id)).toEqual(["moomoo"]);
    expect(body.crypto.map((p: { id: string }) => p.id)).toEqual(["okx", "cryptocom"]);
  });
});

describe("Sidebar", () => {
  const PARTNER_HOSTS = ["moomoo.com", "okx.com", "crypto.com", "wealthsimple.com"];

  it("server-renders no partner link, but the disclosure and TermPocket card", () => {
    const out = renderToStaticMarkup(createElement(Sidebar, { locale: "en" }));
    for (const host of PARTNER_HOSTS) expect(out).not.toContain(host);
    expect(out).toContain('href="https://www.termpocket.com"');
    expect(out).toContain("Affiliate links: PreBell may earn a reward if you sign up. Not investment advice.");
    expect(out).toContain("From the PreBell team. Plan your goals and habits with Pocky, your AI assistant.");
  });

  it("uses the Chinese copy on ZH pages", () => {
    const out = renderToStaticMarkup(createElement(Sidebar, { locale: "zh" }));
    for (const host of PARTNER_HOSTS) expect(out).not.toContain(host);
    expect(out).toContain("推广链接：通过链接注册，PreBell 可能获得奖励。不构成投资建议。");
    expect(out).toContain("来自 PreBell 团队。用 AI 助手 Pocky 规划你的目标与习惯。");
  });
});

describe("PartnerBoxes", () => {
  const titles = { brokers: "Brokers", crypto: "Crypto exchanges" };
  const okx = { id: "okx", name: "OKX", url: "https://okx.com/join/AIAFMP3U" };

  it("opens links in a new tab as sponsored", () => {
    const out = renderToStaticMarkup(
      createElement(PartnerBoxes, { data: { brokers: [], crypto: [okx] }, titles }),
    );
    expect(out).toContain('href="https://okx.com/join/AIAFMP3U"');
    expect(out).toContain('target="_blank"');
    expect(out).toContain('rel="sponsored noopener noreferrer"');
  });

  it("drops a box with no links", () => {
    const out = renderToStaticMarkup(
      createElement(PartnerBoxes, { data: { brokers: [], crypto: [okx] }, titles }),
    );
    expect(out).toContain("Crypto exchanges");
    expect(out).not.toContain("Brokers");
  });

  it("renders nothing when both groups are empty", () => {
    const out = renderToStaticMarkup(
      createElement(PartnerBoxes, { data: { brokers: [], crypto: [] }, titles }),
    );
    expect(out).toBe("");
  });
});

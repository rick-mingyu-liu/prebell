import { describe, expect, it, vi } from "vitest";
import { cardResponse, postCard } from "../src/lib/covers";

describe("share card route", () => {
  const png = new Uint8Array([0x89, 0x50, 0x4e, 0x47]);
  const assets = { fetch: vi.fn(async (r: Request) => new Response(`default ${new URL(r.url).pathname}`)) };
  const bucket = (keys: string[]) => ({
    get: vi.fn(async (k: string) =>
      keys.includes(k) ? { body: new Blob([png]).stream(), size: png.length, httpEtag: '"e1"' } : null,
    ),
  });

  it("serves a rendered card from R2 for a day", async () => {
    const b = bucket(["og/recap-2026-11-04.png"]);
    const res = await cardResponse(b, assets, "recap-2026-11-04.png", "https://prebell.site/og/recap-2026-11-04.png");
    expect(b.get).toHaveBeenCalledWith("og/recap-2026-11-04.png");
    expect(res.headers.get("Content-Type")).toBe("image/png");
    expect(res.headers.get("Cache-Control")).toBe("public, max-age=86400, s-maxage=86400");
    expect(res.headers.get("Content-Length")).toBe("4");
    expect(res.headers.get("ETag")).toBe('"e1"');
    expect(new Uint8Array(await res.arrayBuffer())).toEqual(png);
  });

  it("falls back to the default card, briefly cached, until the card is rendered", async () => {
    const res = await cardResponse(bucket([]), assets, "recap-2026-11-04.png", "https://prebell.site/og/recap-2026-11-04.png");
    expect(await res.text()).toBe("default /og-default.png");
    expect(res.headers.get("Content-Type")).toBe("image/png");
    expect(res.headers.get("Cache-Control")).toBe("public, max-age=300, s-maxage=300");
  });

  it("reads only card keys", async () => {
    const b = bucket([]);
    for (const f of ["../facts.json", "x.svg", "X.png", "a/b.png"]) {
      expect((await cardResponse(b, assets, f, "https://prebell.site/og/x")).status, f).toBe(404);
    }
    expect(b.get).not.toHaveBeenCalled();
  });
});

describe("which card a post names", () => {
  const head = (keys: string[]) => ({ head: vi.fn(async (k: string) => (keys.includes(k) ? { key: k } : null)) });

  it("its own card once R2 has it, else the default card", async () => {
    const b = head(["og/recap-2026-11-04.png"]);
    expect(await postCard(b, "covers/recap-2026-11-04.svg")).toBe("/og/recap-2026-11-04.png");
    expect(b.head).toHaveBeenCalledWith("og/recap-2026-11-04.png");
    expect(await postCard(b, "covers/recap-2026-11-05.svg")).toBe("/og-default.png");
  });

  it("never looks up a post without an SVG cover, and a failed lookup gives the default", async () => {
    const b = head([]);
    expect(await postCard(b, "covers/recap-2026-10-30.png")).toBe("/og-default.png");
    expect(await postCard(b, null)).toBe("/og-default.png");
    expect(b.head).not.toHaveBeenCalled();
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    const failing = { head: async () => Promise.reject(new Error("r2 down")) };
    expect(await postCard(failing, "covers/recap-2026-11-04.svg")).toBe("/og-default.png");
    expect(error).toHaveBeenCalledOnce();
    error.mockRestore();
  });
});

import { describe, expect, it } from "vitest";
import { pairedAlternates, resolveOrigin } from "../src/lib/seo";

const req = (h: Record<string, string>) => new Headers(h);

describe("resolveOrigin", () => {
  it("uses SITE_URL and ignores x-forwarded-host/proto", () => {
    const base = resolveOrigin(
      "https://marketsite-web.workers.dev",
      req({ host: "marketsite-web.workers.dev", "x-forwarded-host": "evil.test", "x-forwarded-proto": "http" }),
    );
    const alt = pairedAlternates("en", "/p/premarket-2026-10-29")!;
    const urls = [alt.canonical as string, ...Object.values(alt.languages as Record<string, string>)].map(
      (u) => new URL(u, base).href,
    );
    expect(urls).toEqual([
      "https://marketsite-web.workers.dev/en/p/premarket-2026-10-29",
      "https://marketsite-web.workers.dev/en/p/premarket-2026-10-29",
      "https://marketsite-web.workers.dev/zh/p/premarket-2026-10-29",
      "https://marketsite-web.workers.dev/en/p/premarket-2026-10-29",
    ]);
    expect(urls.join(" ")).not.toContain("evil.test");
  });

  it("keeps only the SITE_URL origin", () => {
    expect(resolveOrigin("https://example.com/some/path", req({})).href).toBe("https://example.com/");
  });

  it("falls back to Host (never x-forwarded-*) when SITE_URL is unset", () => {
    expect(resolveOrigin(undefined, req({ host: "site.test", "x-forwarded-host": "evil.test" })).href).toBe(
      "https://site.test/",
    );
    expect(resolveOrigin("", req({ host: "localhost:8788" })).href).toBe("http://localhost:8788/");
  });
});

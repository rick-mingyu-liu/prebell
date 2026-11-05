import { describe, expect, it } from "vitest";
import { COVER_KEY, coverResponse } from "../src/lib/covers";

function bucketWith(stored: Record<string, string>) {
  const seen: string[] = [];
  return {
    seen,
    async get(key: string) {
      seen.push(key);
      return key in stored ? { body: stored[key] } : null;
    },
  };
}

describe("covers", () => {
  it("serves png and jpg with cache headers", async () => {
    const bucket = bucketWith({ "covers/amgn-q3.png": "PNG", "covers/brief-1.jpg": "JPG" });
    const png = await coverResponse(bucket, ["covers", "amgn-q3.png"]);
    expect(png.status).toBe(200);
    expect(png.headers.get("content-type")).toBe("image/png");
    expect(png.headers.get("cache-control")).toBe("public, max-age=86400, s-maxage=86400");
    expect(png.headers.get("x-content-type-options")).toBe("nosniff");
    expect(png.headers.get("content-security-policy")).toBe("sandbox");
    expect(await png.text()).toBe("PNG");
    const jpg = await coverResponse(bucket, ["covers", "brief-1.jpg"]);
    expect(jpg.headers.get("content-type")).toBe("image/jpeg");
    expect(bucket.seen).toEqual(["covers/amgn-q3.png", "covers/brief-1.jpg"]);
  });

  it("serves svg as image/svg+xml in a sandbox", async () => {
    const res = await coverResponse(bucketWith({ "covers/premarket-2026-11-04.svg": "<svg/>" }), ["covers", "premarket-2026-11-04.svg"]);
    expect(res.status).toBe(200);
    expect(res.headers.get("content-type")).toBe("image/svg+xml");
    expect(res.headers.get("content-security-policy")).toBe("sandbox");
    expect(res.headers.get("x-content-type-options")).toBe("nosniff");
    expect(res.headers.get("cache-control")).toBe("public, max-age=86400, s-maxage=86400");
    expect(await res.text()).toBe("<svg/>");
  });

  it("rejects keys outside covers/", async () => {
    const bucket = bucketWith({ "charts/x.png": "x", "covers/a.SVG": "x", "covers/a.svgz": "x", "covers/A.png": "x" });
    for (const bad of [
      ["charts", "x.png"],
      ["..", "covers", "a.png"],
      ["covers", "..", "charts", "x.png"],
      ["covers", "a.SVG"],
      ["covers", "a.svgz"],
      ["covers", "A.png"],
      ["covers", "a", "b.png"],
      ["covers", `${"a".repeat(81)}.png`],
      [],
    ]) {
      expect((await coverResponse(bucket, bad)).status).toBe(404);
    }
    expect(bucket.seen).toEqual([]);
    expect(COVER_KEY.test("covers/a-1.jpg")).toBe(true);
    expect(COVER_KEY.test("covers/a-1.svg")).toBe(true);
  });

  it("404 when missing", async () => {
    const res = await coverResponse(bucketWith({}), ["covers", "gone.png"]);
    expect(res.status).toBe(404);
    expect(await res.text()).toBe("not found");
  });
});

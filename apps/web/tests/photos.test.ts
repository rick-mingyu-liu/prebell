import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import sharp from "sharp";
import { describe, expect, it } from "vitest";
import { companyPhoto } from "../src/lib/photos";
import { COMPANY_PHOTOS } from "../src/lib/photos.generated";

// These run over the real manifest that scripts/build-photos.mjs wrote from data/photos.tsv.
const publicFile = (path: string) => new URL(`../public${path}`, import.meta.url);
const filePath = (url: string) => url.split("?")[0];
const watchlist = readFileSync(new URL("../../../crates/core/data/watchlist.tsv", import.meta.url), "utf8")
  .split("\n")
  .filter((l) => l.trim() && !l.startsWith("#"))
  .map((l) => l.split("\t")[0].trim());
const pinned = readFileSync(new URL("../data/photos.tsv", import.meta.url), "utf8")
  .split("\n")
  .filter((l) => l.trim() && !l.startsWith("#"))
  .map((l) => l.split("\t").map((s) => s.trim()));
const entries = Object.entries(COMPANY_PHOTOS);
/** The build's allow-list: public domain, CC0, CC BY and CC BY-SA, any version or port; nothing with NC or ND. */
const LICENCE = /^(Public domain|CC0( 1\.0)?|CC BY(-SA)? \d\.\d( [a-z]{2}(-[a-z]{2})?)?)$/i;

describe("photo manifest", () => {
  it("holds exactly the pinned rows of data/photos.tsv, each a watchlist ticker with its captions", () => {
    expect(entries.length).toBeGreaterThan(0);
    expect(Object.keys(COMPANY_PHOTOS).sort()).toEqual(pinned.map((r) => r[0]).sort());
    for (const [ticker, file, en, zh] of pinned) {
      expect(watchlist).toContain(ticker);
      expect(file).toMatch(/\.(jpe?g|png)$/i);
      expect(COMPANY_PHOTOS[ticker].en).toBe(en);
      expect(COMPANY_PHOTOS[ticker].zh).toBe(zh);
      expect(zh).toMatch(/[一-鿿]/);
    }
  });

  it("names a WebP in /photos with its content hash for every entry, and the folder has nothing else", async () => {
    for (const [ticker, p] of entries) {
      expect(p.src).toMatch(new RegExp(`^/photos/${ticker.replace(".", "\\.")}\\.webp\\?v=[0-9a-f]{8}$`));
      const buf = readFileSync(publicFile(filePath(p.src)));
      expect(p.src.split("?v=")[1]).toBe(createHash("sha1").update(buf).digest("hex").slice(0, 8));
      const meta = await sharp(buf).metadata();
      expect(meta.format).toBe("webp");
      expect([meta.width, meta.height]).toEqual([p.width, p.height]);
      expect(Math.max(p.width, p.height)).toBeLessThanOrEqual(1440);
      expect(p.width / p.height).toBeGreaterThanOrEqual(0.8);
      expect(p.width / p.height).toBeLessThanOrEqual(2.4);
    }
    const files = readdirSync(publicFile("/photos")).filter((f) => f !== "LICENSES.md");
    expect(files.map((f) => `/photos/${f}`).sort()).toEqual(entries.map(([, p]) => filePath(p.src)).sort());
  });

  it("has a LICENSES.md row for every file, naming its Commons page, author and licence", () => {
    const licences = readFileSync(publicFile("/photos/LICENSES.md"), "utf8");
    expect(licences).toContain("not covered by the repository's MIT licence");
    expect(licences).toContain("resized");
    for (const [ticker, p] of entries) {
      const row = licences.split("\n").find((l) => l.startsWith(`| \`${ticker}.webp\` |`));
      expect(row, ticker).toBeDefined();
      expect(row).toContain(p.page);
      expect(row).toContain(p.licence);
      expect(row).toContain("resized");
    }
  });

  it("only carries allowed licences, a plain author name and a Commons page for the credit", () => {
    for (const [ticker, p] of entries) {
      expect(p.licence, ticker).toMatch(LICENCE);
      expect(p.licence).not.toMatch(/NC|ND/);
      if (p.licence.startsWith("CC BY")) expect(p.licenceUrl, ticker).toMatch(/^https?:\/\/creativecommons\.org\//);
      expect(p.author.trim()).toBe(p.author);
      expect(p.author.length).toBeGreaterThan(0);
      expect(p.author).not.toMatch(/[<>]|https?:|&[a-z#0-9]+;/i);
      expect(p.page).toMatch(/^https:\/\/commons\.wikimedia\.org\/wiki\/File:/);
    }
  });

  it("keeps the folder small and serves it immutable like /logos", () => {
    const bytes = readdirSync(publicFile("/photos")).reduce((a, f) => a + statSync(publicFile(`/photos/${f}`)).size, 0);
    expect(bytes).toBeLessThan(40_000_000);
    expect(readFileSync(publicFile("/_headers"), "utf8")).toContain("/photos/*\n  Cache-Control: public,max-age=31536000,immutable");
    expect(existsSync(publicFile("/photos/LICENSES.md"))).toBe(true);
  });
});

describe("companyPhoto", () => {
  it("finds a pinned photo by ticker in any case, with the caption in the page's language", () => {
    const [ticker, p] = entries[0];
    expect(companyPhoto(ticker.toLowerCase(), "en")).toEqual({
      src: p.src,
      width: p.width,
      height: p.height,
      caption: p.en,
      author: p.author,
      licence: p.licence,
      licenceUrl: p.licenceUrl,
      page: p.page,
    });
    expect(companyPhoto(` ${ticker} `, "zh")?.caption).toBe(p.zh);
  });
  it("is null for a company without one and for odd input", () => {
    for (const t of ["ZZZZ", "", null, undefined, "constructor", "__proto__", "toString"]) expect(companyPhoto(t, "en")).toBeNull();
  });
});

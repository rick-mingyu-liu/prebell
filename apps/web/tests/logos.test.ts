import { readFileSync, readdirSync, statSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { companyLogo, companyLogoInfo } from "../src/lib/logos";
import { COMPANY_LOGOS, COMPANY_LOGO_SIZES } from "../src/lib/logos.generated";

const publicFile = (path: string) => new URL(`../public${path}`, import.meta.url);
const watchlist = readFileSync(new URL("../../../crates/core/data/watchlist.tsv", import.meta.url), "utf8")
  .split("\n")
  .filter((l) => l.trim() && !l.startsWith("#"))
  .map((l) => l.split("\t")[0].trim());

describe("companyLogo", () => {
  it("finds a bundled tile by ticker, in any case", () => {
    expect(companyLogo("AAPL")).toMatch(/^\/logos\/companies\/AAPL\.(svg|png)$/);
    expect(companyLogo(" aapl ")).toBe(companyLogo("AAPL"));
  });
  it("is null for tickers without a logo, unknown tickers and odd input", () => {
    expect(companyLogo("AVGO")).toBeNull();
    expect(companyLogo("VMRK")).toBeNull(); // the script's SKIP list
    expect(companyLogo("ZZZZ")).toBeNull();
    expect(companyLogo("")).toBeNull();
    expect(companyLogo("constructor")).toBeNull();
    expect(companyLogo("__proto__")).toBeNull();
  });
});

describe("companyLogoInfo", () => {
  it("returns the file with its natural size, which fits a 128x48 slot", () => {
    const info = companyLogoInfo(" aapl ");
    expect(info).toEqual({ src: companyLogo("AAPL"), width: COMPANY_LOGO_SIZES.AAPL[0], height: COMPANY_LOGO_SIZES.AAPL[1] });
    for (const [width, height] of Object.values(COMPANY_LOGO_SIZES)) {
      expect(width).toBeGreaterThan(0);
      expect(height).toBeGreaterThan(0);
      expect(width).toBeLessThanOrEqual(128);
      expect(height).toBeLessThanOrEqual(48);
    }
  });
  it("is null where companyLogo is", () => {
    expect(companyLogoInfo("AVGO")).toBeNull();
    expect(companyLogoInfo("ZZZZ")).toBeNull();
    expect(companyLogoInfo("__proto__")).toBeNull();
  });
});

describe("logo manifest", () => {
  it("only maps watchlist tickers, each to an SVG or PNG tile that exists", () => {
    for (const [ticker, path] of Object.entries(COMPANY_LOGOS)) {
      expect(watchlist).toContain(ticker);
      expect(path).toMatch(new RegExp(`^/logos/companies/${ticker.replace(".", "\\.")}\\.(svg|png)$`));
      const file = readFileSync(publicFile(path));
      if (path.endsWith(".svg")) expect(file.toString("utf8")).toMatch(/^<svg xmlns="http:\/\/www\.w3\.org\/2000\/svg"/);
      else {
        expect(file.subarray(0, 8).toString("hex")).toBe("89504e470d0a1a0a");
        // The IHDR size is twice the natural size recorded for the card (rounded).
        const [w, h] = COMPANY_LOGO_SIZES[ticker];
        expect(Math.abs(file.readUInt32BE(16) - 2 * w)).toBeLessThanOrEqual(1);
        expect(Math.abs(file.readUInt32BE(20) - 2 * h)).toBeLessThanOrEqual(1);
      }
    }
  });
  it("records a natural size for every logo and no others", () => {
    expect(Object.keys(COMPANY_LOGO_SIZES).sort()).toEqual(Object.keys(COMPANY_LOGOS).sort());
  });
  it("has no stray files and every file has a licence entry", () => {
    const licences = readFileSync(publicFile("/logos/LICENSES.md"), "utf8");
    const files = readdirSync(publicFile("/logos/companies")).map((f) => `/logos/companies/${f}`);
    expect(files.sort()).toEqual(Object.values(COMPANY_LOGOS).sort());
    const partners = readdirSync(publicFile("/logos/partners")).map((f) => `partners/${f}`);
    for (const f of [...files.map((p) => p.replace("/logos/", "")), ...partners]) expect(licences).toContain(`\`${f}\``);
  });
  it("keeps the bundled assets small and free of scripts or remote references", () => {
    let bytes = 0;
    for (const dir of ["/logos/companies", "/logos/partners"])
      for (const f of readdirSync(publicFile(dir))) {
        const path = publicFile(`${dir}/${f}`);
        bytes += statSync(path).size;
        if (f.endsWith(".svg")) expect(readFileSync(path, "utf8")).not.toMatch(/<script|href=|on\w+=/i);
        else expect(f).toMatch(/\.png$/);
      }
    expect(bytes).toBeLessThan(2_500_000);
  });
});

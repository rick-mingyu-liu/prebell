import * as generated from "./logos.generated";

const { COMPANY_LOGOS } = generated;
/** Natural CSS sizes by ticker, when the manifest carries them (wide wordmarks); a tile without one is square. */
const SIZES: Readonly<Record<string, readonly [number, number]>> =
  (generated as { COMPANY_LOGO_SIZES?: Readonly<Record<string, readonly [number, number]>> }).COMPANY_LOGO_SIZES ?? {};

/** The bundled logo for a ticker (a static path under /logos), or null to show the text chip. */
export function companyLogo(ticker: string): string | null {
  const key = ticker.trim().toUpperCase();
  return Object.hasOwn(COMPANY_LOGOS, key) ? COMPANY_LOGOS[key] : null;
}

export interface CompanyLogoInfo {
  src: string;
  /** Natural size in CSS px, so a card can tell a wide wordmark from a square icon and size it. */
  width: number;
  height: number;
}

/** A ticker's logo with its natural size, or null to show the text chip. */
export function companyLogoInfo(ticker: string): CompanyLogoInfo | null {
  const src = companyLogo(ticker);
  if (!src) return null;
  const key = ticker.trim().toUpperCase();
  const [width, height] = Object.hasOwn(SIZES, key) ? SIZES[key] : [48, 48];
  return { src, width, height };
}

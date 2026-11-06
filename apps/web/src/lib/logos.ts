import { COMPANY_LOGOS, COMPANY_LOGO_SIZES } from "./logos.generated";

/** The bundled logo for a ticker (a static path under /logos), or null to show the text chip. */
export function companyLogo(ticker: string): string | null {
  const key = ticker.trim().toUpperCase();
  return Object.hasOwn(COMPANY_LOGOS, key) ? COMPANY_LOGOS[key] : null;
}

export interface CompanyLogoInfo {
  src: string;
  /** Natural size in CSS px (the file is 2x): at most 128 x 48, as wide as the mark is, so a card can size it. */
  width: number;
  height: number;
}

/** A ticker's logo with its natural size, or null to show the text chip. */
export function companyLogoInfo(ticker: string): CompanyLogoInfo | null {
  const src = companyLogo(ticker);
  if (!src) return null;
  const key = ticker.trim().toUpperCase();
  const [width, height] = COMPANY_LOGO_SIZES[key] ?? [48, 48];
  return { src, width, height };
}

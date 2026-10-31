import { COMPANY_LOGOS } from "./logos.generated";

/** The bundled logo tile for a ticker (a static path under /logos), or null to show the text chip. */
export function companyLogo(ticker: string): string | null {
  const key = ticker.trim().toUpperCase();
  return Object.hasOwn(COMPANY_LOGOS, key) ? COMPANY_LOGOS[key] : null;
}

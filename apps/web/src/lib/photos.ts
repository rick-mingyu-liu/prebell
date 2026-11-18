import type { Locale } from "../i18n/routing";
import { COMPANY_PHOTOS } from "./photos.generated";

/** A company's photo as a page shows it: the bundled file, its size in px, the caption and the credit. */
export interface CompanyPhoto {
  /** A static path under /photos with a `?v=` content hash. */
  src: string;
  width: number;
  height: number;
  /** What the photo shows, in the page's language; also the image's alt text. */
  caption: string;
  /** The author's plain name, as Commons gives it. */
  author: string;
  /** The licence's short name ("CC BY-SA 4.0"), or "Public domain". */
  licence: string;
  licenceUrl: string | null;
  /** The file's page on Wikimedia Commons. */
  page: string;
}

/** The pinned photo for a ticker (built by scripts/build-photos.mjs), or null when the company has none. */
export function companyPhoto(ticker: string | null | undefined, locale: Locale): CompanyPhoto | null {
  const key = (ticker ?? "").trim().toUpperCase();
  if (!Object.hasOwn(COMPANY_PHOTOS, key)) return null;
  const p = COMPANY_PHOTOS[key];
  return {
    src: p.src,
    width: p.width,
    height: p.height,
    caption: locale === "zh" ? p.zh : p.en,
    author: p.author,
    licence: p.licence,
    licenceUrl: p.licenceUrl,
    page: p.page,
  };
}

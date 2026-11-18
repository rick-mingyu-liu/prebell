import type { Locale } from "../i18n/routing";
import { COMPANY_PHOTOS } from "./photos.generated";

/** A company's photo as a page shows it: the bundled file, its size in px, the caption and the credit. */
export interface CompanyPhoto {
  /** The watchlist ticker, upper case: the photo's row on the photo credits page. */
  ticker: string;
  /** A static path under /photos with a `?v=` content hash. */
  src: string;
  width: number;
  height: number;
  /** What the photo shows, in the page's language; also the image's alt text. */
  caption: string;
  /** The author as the credit names them (Commons' requested form where there is one). */
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
    ticker: key,
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

/** One row of the photo credits page: everything the licences ask a reuser to give, as text. */
export interface PhotoCredit {
  ticker: string;
  caption: string;
  /** The Commons file name (the work's title there), and the original title on Flickr or another source. */
  file: string;
  sourceTitle: string | null;
  author: string;
  /** The author's requested attribution, conditions and copyright notices, exactly as given. */
  notices: readonly string[];
  page: string;
  sourceUrl: string | null;
  licence: string;
  licenceUrl: string | null;
  width: number;
  height: number;
  /** Set when the Commons file is itself a crop: `from` names the file it was cropped from, when Commons says. */
  derived: { from: string | null } | null;
}

/** Every pinned photo's credit, by ticker. */
export function photoCredits(locale: Locale): PhotoCredit[] {
  return Object.keys(COMPANY_PHOTOS)
    .sort()
    .map((ticker) => {
      const p = COMPANY_PHOTOS[ticker];
      return {
        ticker,
        caption: locale === "zh" ? p.zh : p.en,
        file: p.file,
        sourceTitle: p.sourceTitle,
        author: p.author,
        notices: p.notices,
        page: p.page,
        sourceUrl: p.sourceUrl,
        licence: p.licence,
        licenceUrl: p.licenceUrl,
        width: p.width,
        height: p.height,
        derived: p.derived,
      };
    });
}

/** Cover images the generator writes to R2 (`FACTS` bucket, `covers/` prefix). */

import type { R2Like } from "./earnings";

/** R2 keys the generator writes: covers/{slug}.svg (current), and .png or .jpg (older scene covers). */
export const COVER_KEY = /^covers\/[a-z0-9-]{1,80}\.(png|jpg|svg)$/;

const CONTENT_TYPE: Record<string, string> = { png: "image/png", jpg: "image/jpeg", svg: "image/svg+xml" };

/** The response for the R2 key `segments.join("/")`: only generator-written cover keys, as a cached image. An SVG is served sandboxed so it can never run script. */
export async function coverResponse(bucket: R2Like, segments: string[]): Promise<Response> {
  const key = segments.join("/");
  const ext = COVER_KEY.exec(key)?.[1];
  if (!ext) return new Response("not found", { status: 404 });
  const obj = await bucket.get(key);
  if (!obj) return new Response("not found", { status: 404 });
  return new Response(obj.body, {
    headers: {
      "Content-Type": CONTENT_TYPE[ext]!,
      "Content-Security-Policy": "sandbox",
      "X-Content-Type-Options": "nosniff",
      // A cover key names one post and never changes.
      "Cache-Control": "public, max-age=86400, s-maxage=86400",
    },
  });
}

/** Share cards the generator renders from SVG covers: R2 `og/<stem>.png`, requested as `/og/<stem>.png`. */
export const CARD_FILE = /^[a-z0-9-]{1,80}\.png$/;

/** The share card for pages without one of their own (1200×630, `public/og-default.png`). */
export const DEFAULT_CARD = "/og-default.png";

/**
 * Where a post's card would be: the generator renders `covers/<stem>.svg` to `og/<stem>.png` in R2.
 * Older raster covers and posts without a cover have none.
 */
export function cardPath(coverKey: string | null): string {
  const stem = coverKey && /^covers\/([a-z0-9-]{1,80})\.svg$/.exec(coverKey)?.[1];
  return stem ? `/og/${stem}.png` : DEFAULT_CARD;
}

/** The slice of the R2 binding the card lookups use. */
export interface CardBucket {
  get(key: string): Promise<{ body: ReadableStream | string; size?: number; httpEtag?: string } | null>;
  head(key: string): Promise<unknown>;
}

/**
 * The card a post's page should name: its own once the generator has rendered it, else the default.
 * A share made before the card exists then caches the default card's URL, never a post card URL that
 * would later change what it shows. One R2 head; a failed lookup gives the default.
 */
export async function postCard(bucket: Pick<CardBucket, "head">, coverKey: string | null): Promise<string> {
  const path = cardPath(coverKey);
  if (path === DEFAULT_CARD) return path;
  try {
    return (await bucket.head(path.slice(1))) ? path : DEFAULT_CARD;
  } catch (err) {
    console.error(`card lookup ${path}:`, err);
    return DEFAULT_CARD;
  }
}

/** The slice of the static-assets binding the card route uses. */
export interface AssetsLike {
  fetch(request: Request): Promise<Response>;
}

/**
 * The response for `GET /og/<file>`: the rendered card from R2, kept for a day like the cover it shows;
 * until the generator has rendered it (a few minutes after publishing, or never for old posts), the
 * default card, cached briefly so the real one replaces it.
 */
export async function cardResponse(
  bucket: Pick<CardBucket, "get">,
  assets: AssetsLike,
  file: string,
  requestUrl: string,
): Promise<Response> {
  if (!CARD_FILE.test(file)) return new Response("not found", { status: 404 });
  const obj = await bucket.get(`og/${file}`);
  if (obj) {
    return new Response(obj.body, {
      headers: {
        "Content-Type": "image/png",
        "X-Content-Type-Options": "nosniff",
        "Cache-Control": "public, max-age=86400, s-maxage=86400",
        ...(obj.size !== undefined ? { "Content-Length": String(obj.size) } : {}),
        ...(obj.httpEtag ? { ETag: obj.httpEtag } : {}),
      },
    });
  }
  const fallback = await assets.fetch(new Request(new URL("/og-default.png", requestUrl)));
  if (!fallback.ok) return new Response("not found", { status: 404 });
  return new Response(fallback.body, {
    headers: {
      "Content-Type": "image/png",
      "X-Content-Type-Options": "nosniff",
      "Cache-Control": "public, max-age=300, s-maxage=300",
    },
  });
}

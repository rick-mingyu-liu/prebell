/** Cover images the generator writes to R2 (`FACTS` bucket, `covers/` prefix). */

import type { R2Like } from "./earnings";

/** R2 keys the generator writes: covers/{slug}.png or .jpg */
export const COVER_KEY = /^covers\/[a-z0-9-]{1,80}\.(png|jpg)$/;

/** The response for the R2 key `segments.join("/")`: only generator-written cover keys, as a cached image. */
export async function coverResponse(bucket: R2Like, segments: string[]): Promise<Response> {
  const key = segments.join("/");
  const ext = COVER_KEY.exec(key)?.[1];
  if (!ext) return new Response("not found", { status: 404 });
  const obj = await bucket.get(key);
  if (!obj) return new Response("not found", { status: 404 });
  return new Response(obj.body, {
    headers: {
      "Content-Type": ext === "png" ? "image/png" : "image/jpeg",
      "X-Content-Type-Options": "nosniff",
      // A cover key names one post and never changes.
      "Cache-Control": "public, max-age=86400, s-maxage=86400",
    },
  });
}

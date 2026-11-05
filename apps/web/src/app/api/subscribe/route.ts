import { getCloudflareContext } from "@opennextjs/cloudflare";
import { subscribe, validEmail } from "../../../lib/subscribe";

const SOURCES = ["post", "home", "about"];
const LANGS = ["en", "zh"];
const HEADERS = { "Cache-Control": "private, no-store" };

const reply = (body: { ok: boolean }, status: number) => Response.json(body, { status, headers: HEADERS });

/** The Buttondown key from the Worker secrets, or null when unset or there is no Cloudflare context. */
function apiKey(): string | null {
  try {
    const key = (getCloudflareContext().env as { BUTTONDOWN_API_KEY?: unknown }).BUTTONDOWN_API_KEY;
    return typeof key === "string" && key ? key : null;
  } catch {
    return null;
  }
}

/**
 * Weekly-digest signup. 400 for a malformed request or a filled honeypot (`website`), 503 until the
 * key is configured, 502 when Buttondown fails. Already-subscribed is a success. Responses carry no
 * upstream text.
 */
export async function POST(req: Request) {
  let body: unknown;
  try {
    body = await req.json();
  } catch {
    return reply({ ok: false }, 400);
  }
  if (typeof body !== "object" || body === null) return reply({ ok: false }, 400);
  const { email, source, lang, website } = body as Record<string, unknown>;
  if (typeof website === "string" && website !== "") return reply({ ok: false }, 400);
  if (
    typeof email !== "string" ||
    !validEmail(email.trim()) ||
    typeof source !== "string" ||
    !SOURCES.includes(source) ||
    typeof lang !== "string" ||
    !LANGS.includes(lang)
  )
    return reply({ ok: false }, 400);

  const key = apiKey();
  if (!key) return reply({ ok: false }, 503);

  const result = await subscribe(key, email.trim(), ["prebell", `lang:${lang}`, `source:${source}`]);
  return result === "error" ? reply({ ok: false }, 502) : reply({ ok: true }, 200);
}

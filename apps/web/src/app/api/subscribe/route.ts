import { buttondownKey } from "../../../lib/signup";
import { subscribe, validEmail } from "../../../lib/subscribe";

const HEADERS = { "Cache-Control": "private, no-store" };

const reply = (body: { ok: boolean; already?: true }, status: number) => Response.json(body, { status, headers: HEADERS });

/**
 * Weekly-digest signup (one English email to every subscriber, so no tags). 400 for a malformed
 * request or a filled honeypot (`website`), 503 until the key is configured, 502 when Buttondown
 * fails. Already-subscribed is a success flagged `already: true`, so the form can say so. Responses
 * carry no upstream text.
 */
export async function POST(req: Request) {
  let body: unknown;
  try {
    body = await req.json();
  } catch {
    return reply({ ok: false }, 400);
  }
  if (typeof body !== "object" || body === null) return reply({ ok: false }, 400);
  const { email, website } = body as Record<string, unknown>;
  if (typeof website === "string" && website !== "") return reply({ ok: false }, 400);
  if (typeof email !== "string" || !validEmail(email.trim())) return reply({ ok: false }, 400);

  const key = buttondownKey();
  if (!key) return reply({ ok: false }, 503);

  const result = await subscribe(key, email.trim());
  if (result === "error") return reply({ ok: false }, 502);
  return reply(result === "already" ? { ok: true, already: true } : { ok: true }, 200);
}

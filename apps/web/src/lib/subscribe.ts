/** Newsletter signup against Buttondown. Server-only: the API key never reaches the browser. */

const ENDPOINT = "https://api.buttondown.com/v1/subscribers";

export type SubscribeResult = "ok" | "already" | "error";

/** A deliberately loose shape check (one @, a dotted domain, no spaces); Buttondown does the real validation. */
export function validEmail(s: string): boolean {
  return s.length <= 254 && /^[^\s@]+@[^\s@.]+(\.[^\s@.]+)+$/.test(s) && !/\.\./.test(s);
}

/**
 * Create an unconfirmed subscriber (Buttondown sends the confirmation email). No tags: Buttondown
 * creates a tag that does not exist yet only on a plan with tags (Basic or higher), so on the free
 * plan a tagged signup could fail. 201 is ok, a 400 that says the address is already subscribed
 * counts as success, anything else is an error. Only the status is logged; the upstream body stays
 * out of logs and responses.
 */
export async function subscribe(apiKey: string, email: string, fetchImpl: typeof fetch = fetch): Promise<SubscribeResult> {
  try {
    const res = await fetchImpl(ENDPOINT, {
      method: "POST",
      headers: { Authorization: `Token ${apiKey}`, "Content-Type": "application/json" },
      body: JSON.stringify({ email_address: email, type: "unactivated" }),
    });
    if (res.status === 201) return "ok";
    if (res.status === 400 && /already/i.test(await res.text().catch(() => ""))) return "already";
    console.error("buttondown subscribe failed, status", res.status);
  } catch {
    console.error("buttondown subscribe failed, network error");
  }
  return "error";
}

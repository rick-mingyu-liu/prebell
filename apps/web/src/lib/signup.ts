import "server-only";
import { getCloudflareContext } from "@opennextjs/cloudflare";

/**
 * Server-only. The Buttondown key from the Worker secrets, or null when it is unset, empty or there
 * is no Cloudflare context. The key itself never leaves the server.
 */
export function buttondownKey(): string | null {
  try {
    const key = (getCloudflareContext().env as { BUTTONDOWN_API_KEY?: unknown }).BUTTONDOWN_API_KEY;
    return typeof key === "string" && key ? key : null;
  } catch {
    return null;
  }
}

/**
 * Server-only. Whether a page shows the weekly-digest form: only once the Buttondown key is set.
 * Pages in both languages show it; the Chinese form says the email is in English.
 */
export function signupEnabled(): boolean {
  return buttondownKey() !== null;
}

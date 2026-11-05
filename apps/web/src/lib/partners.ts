/** Partner invite links and where each may be shown. Change a link or a rule here only. */
// Server-only so the gated URLs can never be bundled into client JS; client code imports types only.
import "server-only";

export type PartnerGroup = "brokers" | "crypto";

export interface Partner {
  id: string;
  name: string;
  url: string;
  group: PartnerGroup;
  /** A square logo tile under /public. The partner strip shows names only, so it is not drawn; `logoDark` is its dark variant. Optional. */
  logo?: string;
  logoDark?: string;
  /** `country` is an ISO 3166 alpha-2 code, or null when Cloudflare doesn't know it. */
  allowedIn(country: string | null): boolean;
}

export type PartnerLink = Pick<Partner, "id" | "name" | "url" | "logo" | "logoDark">;
export type PartnersResponse = Record<PartnerGroup, PartnerLink[]>;

const notInChina = (country: string | null) => country !== "CN";

const PARTNERS: Partner[] = [
  {
    id: "moomoo",
    name: "moomoo",
    url: "https://j.moomoo.com/0J1BGf",
    group: "brokers",
    logo: "/logos/partners/moomoo.png",
    allowedIn: notInChina,
  },
  {
    id: "wealthsimple",
    name: "Wealthsimple",
    url: "https://wealthsimple.com/invite/QDBQ5Z",
    group: "brokers",
    logo: "/logos/partners/wealthsimple.png",
    allowedIn: (country) => country === "CA",
  },
  {
    id: "okx",
    name: "OKX",
    url: "https://okx.com/join/AIAFMP3U",
    group: "crypto",
    logo: "/logos/partners/okx.svg",
    allowedIn: notInChina,
  },
  {
    id: "cryptocom",
    name: "Crypto.com",
    url: "https://crypto.com/app/smqqda7396",
    group: "crypto",
    logo: "/logos/partners/cryptocom.png",
    allowedIn: notInChina,
  },
];

export const TERMPOCKET_URL = "https://www.termpocket.com";

export function partnersFor(country: string | null): Partner[] {
  return PARTNERS.filter((p) => p.allowedIn(country));
}

export function groupedPartners(country: string | null): PartnersResponse {
  const out: PartnersResponse = { brokers: [], crypto: [] };
  for (const { id, name, url, group, logo, logoDark } of partnersFor(country))
    out[group].push({ id, name, url, ...(logo && { logo }), ...(logoDark && { logoDark }) });
  return out;
}

/** A two-letter country code, or null for Cloudflare's "unknown" (XX), Tor (T1) and empty values. */
function normalize(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const code = value.trim().toUpperCase();
  return /^[A-Z]{2}$/.test(code) && code !== "XX" ? code : null;
}

/**
 * The visitor's country from Cloudflare: `request.cf.country`, else the `cf-ipcountry` header.
 * `cf` comes first because the Workers runtime sets it and a client can't forge it; the header
 * covers local dev. On OpenNext the route gets a rebuilt request without `cf`, so the caller
 * passes `getCloudflareContext().cf` (the original request's `cf`).
 */
export function countryOf(
  req: Request,
  cf: { country?: unknown } | undefined = (req as { cf?: { country?: unknown } }).cf,
): string | null {
  return normalize(cf?.country) ?? normalize(req.headers.get("cf-ipcountry"));
}

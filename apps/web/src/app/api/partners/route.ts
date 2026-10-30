import { getCloudflareContext } from "@opennextjs/cloudflare";
import { countryOf, groupedPartners } from "../../../lib/partners";

/** Invite links allowed in the visitor's country. Per visitor, so never cached; no D1/R2 reads. */
export function GET(req: Request) {
  let cf: { country?: unknown } | undefined;
  try {
    cf = getCloudflareContext().cf;
  } catch {
    // No Cloudflare context (not running on OpenNext): fall back to the header, else unknown.
  }
  return Response.json(groupedPartners(countryOf(req, cf)), {
    headers: { "Cache-Control": "private, no-store" },
  });
}

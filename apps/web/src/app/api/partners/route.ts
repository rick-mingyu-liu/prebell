import { getCloudflareContext } from "@opennextjs/cloudflare";
import { countryOf, groupedPartners } from "../../../lib/partners";

/** Invite links allowed in the visitor's country. Per visitor, so never cached; no D1/R2 reads. */
export function GET(req: Request) {
  const country = countryOf(req, getCloudflareContext().cf);
  return Response.json(groupedPartners(country), {
    headers: { "Cache-Control": "private, no-store" },
  });
}

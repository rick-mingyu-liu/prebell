import { getCloudflareContext } from "@opennextjs/cloudflare";
import type { D1Like } from "./posts";

/** The generator's D1 database (binding `DB`), for the current request. */
export function getDb(): D1Like {
  return getCloudflareContext().env.DB;
}

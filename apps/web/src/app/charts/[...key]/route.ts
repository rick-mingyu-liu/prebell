import { getCloudflareContext } from "@opennextjs/cloudflare";
import { chartResponse } from "@/lib/earnings";

/** Earnings chart SVGs from R2 (`FACTS` bucket, `charts/` prefix). */
export async function GET(_req: Request, { params }: RouteContext<"/charts/[...key]">) {
  const { key } = await params;
  return chartResponse(getCloudflareContext().env.FACTS, key);
}

import { getCloudflareContext } from "@opennextjs/cloudflare";
import { coverResponse } from "@/lib/covers";

/** Post cover images from R2 (`FACTS` bucket, `covers/` prefix). */
export async function GET(_req: Request, { params }: RouteContext<"/covers/[...key]">) {
  const { key } = await params;
  return coverResponse(getCloudflareContext().env.FACTS, ["covers", ...key]);
}

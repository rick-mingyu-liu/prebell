import { getCloudflareContext } from "@opennextjs/cloudflare";
import { cardResponse } from "@/lib/covers";

/** Post share cards (Open Graph images) from R2 (`FACTS` bucket, `og/` prefix), else the default card. */
export async function GET(req: Request, { params }: RouteContext<"/og/[file]">) {
  const { file } = await params;
  const { env } = getCloudflareContext();
  return cardResponse(env.FACTS, env.ASSETS, file, req.url);
}

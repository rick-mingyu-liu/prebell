import { readFileSync } from "node:fs";
import { describe, expect, it, vi } from "vitest";
import { memoryD1 } from "./d1";
import { bucket, objects, seed } from "./mcpseed";

// The route reads its switch, D1, R2 and SITE_URL from the Cloudflare context.
const env = vi.hoisted(() => ({ vars: {} as Record<string, unknown>, waited: [] as Promise<unknown>[] }));
vi.mock("@opennextjs/cloudflare", () => ({
  getCloudflareContext: () => ({ env: env.vars, ctx: { waitUntil: (p: Promise<unknown>) => env.waited.push(p) } }),
}));

const { GET, POST } = await import("../src/app/mcp/route");

const call = () =>
  new Request("https://prebell.site/mcp", {
    method: "POST",
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/call", params: { name: "latest_brief", arguments: {} } }),
  });

describe("/mcp route", () => {
  it("is on in wrangler.jsonc", () => {
    const text = readFileSync(new URL("../wrangler.jsonc", import.meta.url), "utf8");
    expect(text).toMatch(/^\s*"MCP_ENABLED": "true"$/m);
  });

  it("answers 404 unless MCP_ENABLED is exactly \"true\"", async () => {
    for (const v of [undefined, "false", "TRUE", "1", ""]) {
      env.vars = { MCP_ENABLED: v, DB: memoryD1(seed()), SITE_URL: "https://prebell.site" };
      expect(GET(new Request("https://prebell.site/mcp")).status).toBe(404);
      expect((await POST(call())).status).toBe(404);
    }
  });

  it("serves the tools when on, with source links on SITE_URL", async () => {
    env.vars = { MCP_ENABLED: "true", DB: memoryD1(seed()), FACTS: bucket(objects()), SITE_URL: "https://prebell.site" };
    const log = vi.spyOn(console, "log").mockImplementation(() => {});
    const res = await POST(call());
    log.mockRestore();
    const body = await res.json();
    expect(res.status).toBe(200);
    expect(body.result.structuredContent.found).toBe(true);
    expect(body.result.structuredContent.source).toMatch(/^https:\/\/prebell\.site\/en\/p\/premarket-/);
    expect(GET(new Request("https://prebell.site/mcp")).status).toBe(200);
  });
});

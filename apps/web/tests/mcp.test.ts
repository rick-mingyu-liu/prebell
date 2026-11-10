import { describe, expect, it, vi } from "vitest";
import { handleMcpGet, handleMcpPost, MAX_BODY_BYTES, originAllowed } from "../src/lib/mcp";
import type { D1Like } from "../src/lib/posts";
import { counted, deps, memoryCache } from "./mcpseed";
import { memoryD1 } from "./d1";
import { seed } from "./mcpseed";

const URL_ = "https://prebell.site/mcp";
const MODERN = "2026-07-28";
const META = {
  "io.modelcontextprotocol/protocolVersion": MODERN,
  "io.modelcontextprotocol/clientInfo": { name: "test", version: "1" },
  "io.modelcontextprotocol/clientCapabilities": {},
};

function req(body: unknown, headers: Record<string, string> = {}): Request {
  return new Request(URL_, {
    method: "POST",
    headers: { "Content-Type": "application/json", Accept: "application/json, text/event-stream", ...headers },
    body: typeof body === "string" ? body : JSON.stringify(body),
  });
}

/** A 2025-era request (handshake revisions). */
const legacy = (method: string, params?: unknown, id: unknown = 1) => ({ jsonrpc: "2.0", id, method, ...(params === undefined ? {} : { params }) });

/** A 2026-07-28 request with its required _meta and headers. */
function modern(method: string, params: Record<string, unknown> = {}, extra: Record<string, string> = {}) {
  const headers: Record<string, string> = { "MCP-Protocol-Version": MODERN, "Mcp-Method": method, ...extra };
  if (method === "tools/call" && typeof params.name === "string" && !("Mcp-Name" in extra)) headers["Mcp-Name"] = params.name;
  return req({ jsonrpc: "2.0", id: 7, method, params: { ...params, _meta: META } }, headers);
}

async function send(r: Request, d = deps()) {
  const res = await handleMcpPost(r, d);
  const text = await res.text();
  return { status: res.status, headers: res.headers, body: text ? JSON.parse(text) : null, d };
}

describe("legacy handshake (2025-11-25 and earlier)", () => {
  it("initialize answers with the requested supported version, the tools capability and instructions", async () => {
    const { status, body, headers } = await send(req(legacy("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "c", version: "1" } })));
    expect(status).toBe(200);
    expect(headers.get("content-type")).toBe("application/json");
    expect(headers.get("mcp-session-id")).toBeNull();
    expect(body.id).toBe(1);
    expect(body.result.protocolVersion).toBe("2025-06-18");
    expect(body.result.capabilities).toEqual({ tools: { listChanged: false } });
    expect(body.result.serverInfo.name).toBe("prebell");
    expect(body.result.instructions).toMatch(/not investment advice/);
  });

  it("initialize with a version it does not support answers with its latest legacy version", async () => {
    const { body } = await send(req(legacy("initialize", { protocolVersion: "2024-11-05", capabilities: {} })));
    expect(body.result.protocolVersion).toBe("2025-11-25");
  });

  it("notifications/initialized is accepted with 202 and no body", async () => {
    const res = await handleMcpPost(req({ jsonrpc: "2.0", method: "notifications/initialized" }), deps());
    expect(res.status).toBe(202);
    expect(await res.text()).toBe("");
  });

  it("ping and tools/list", async () => {
    const h = { "MCP-Protocol-Version": "2025-11-25" };
    expect((await send(req(legacy("ping"), h))).body).toEqual({ jsonrpc: "2.0", id: 1, result: {} });
    const { body } = await send(req(legacy("tools/list", undefined, "a"), h));
    expect(body.id).toBe("a");
    expect(body.result.resultType).toBeUndefined();
    const tools = body.result.tools;
    expect(tools.map((t: { name: string }) => t.name)).toEqual(["earnings_calendar", "earnings_report", "latest_brief"]);
    for (const t of tools) {
      expect(t.inputSchema.type).toBe("object");
      expect(t.inputSchema.additionalProperties).toBe(false);
      expect(t.outputSchema.type).toBe("object");
      expect(t.annotations).toMatchObject({ readOnlyHint: true, destructiveHint: false, openWorldHint: false });
    }
  });

  it("a request without the version header is taken as 2025-03-26", async () => {
    expect((await send(req(legacy("ping")))).status).toBe(200);
  });

  it("an unsupported version header is 400", async () => {
    const { status, body } = await send(req(legacy("ping"), { "MCP-Protocol-Version": "1999-01-01" }));
    expect(status).toBe(400);
    expect(body.error.code).toBe(-32022);
    expect(body.error.data.supported).toEqual(["2026-07-28", "2025-11-25", "2025-06-18", "2025-03-26"]);
  });

  it("the modern version without _meta is invalid params", async () => {
    const { status, body } = await send(req(legacy("tools/list"), { "MCP-Protocol-Version": MODERN }));
    expect(status).toBe(400);
    expect(body.error.code).toBe(-32602);
  });

  it("an unknown method is a JSON-RPC method-not-found error", async () => {
    const { status, body } = await send(req(legacy("resources/list")));
    expect(status).toBe(200);
    expect(body.error.code).toBe(-32601);
    expect(body.id).toBe(1);
  });
});

describe("2026-07-28 requests", () => {
  it("server/discover lists versions, capabilities, identity and caching hints", async () => {
    const { status, body } = await send(modern("server/discover"));
    expect(status).toBe(200);
    expect(body.result).toMatchObject({
      resultType: "complete",
      supportedVersions: ["2026-07-28", "2025-11-25", "2025-06-18", "2025-03-26"],
      capabilities: { tools: { listChanged: false } },
      cacheScope: "public",
    });
    expect(body.result.ttlMs).toBeGreaterThan(0);
    expect(body.result._meta["io.modelcontextprotocol/serverInfo"].name).toBe("prebell");
  });

  it("tools/list carries resultType and caching hints", async () => {
    const { body } = await send(modern("tools/list"));
    expect(body.result.resultType).toBe("complete");
    expect(body.result.tools).toHaveLength(3);
    expect(body.result.cacheScope).toBe("public");
  });

  it("tools/call answers with structured content and its JSON as text", async () => {
    const { body } = await send(modern("tools/call", { name: "latest_brief", arguments: { lang: "en" } }));
    expect(body.result.resultType).toBe("complete");
    expect(body.result.isError).toBe(false);
    expect(body.result.structuredContent.found).toBe(true);
    expect(JSON.parse(body.result.content[0].text)).toEqual(body.result.structuredContent);
  });

  it("accepts a base64 Mcp-Name", async () => {
    const enc = `=?base64?${btoa("latest_brief")}?=`;
    const { status } = await send(modern("tools/call", { name: "latest_brief", arguments: {} }, { "Mcp-Name": enc }));
    expect(status).toBe(200);
  });

  it("rejects missing or mismatched headers with HeaderMismatch", async () => {
    const cases: Request[] = [
      req({ jsonrpc: "2.0", id: 7, method: "tools/list", params: { _meta: META } }, { "Mcp-Method": "tools/list" }),
      modern("tools/list", {}, { "MCP-Protocol-Version": "2025-11-25" }),
      modern("tools/list", {}, { "Mcp-Method": "tools/call" }),
      req({ jsonrpc: "2.0", id: 7, method: "tools/list", params: { _meta: META } }, { "MCP-Protocol-Version": MODERN }),
      modern("tools/call", { name: "latest_brief", arguments: {} }, { "Mcp-Name": "earnings_report" }),
      modern("tools/call", { name: "latest_brief", arguments: {} }, { "Mcp-Name": "=?base64?!!!?=" }),
    ];
    for (const r of cases) {
      const { status, body } = await send(r);
      expect(status).toBe(400);
      expect(body.error.code).toBe(-32020);
      expect(body.id).toBe(7);
    }
  });

  it("an unsupported version is 400 with the supported list", async () => {
    const r = req(
      { jsonrpc: "2.0", id: 7, method: "tools/list", params: { _meta: { ...META, "io.modelcontextprotocol/protocolVersion": "2099-01-01" } } },
      { "MCP-Protocol-Version": "2099-01-01", "Mcp-Method": "tools/list" },
    );
    const { status, body } = await send(r);
    expect(status).toBe(400);
    expect(body.error).toMatchObject({ code: -32022, data: { requested: "2099-01-01" } });
    expect(body.error.data.supported).toContain(MODERN);
  });

  it("missing clientCapabilities is invalid params", async () => {
    const meta = { "io.modelcontextprotocol/protocolVersion": MODERN };
    const r = req({ jsonrpc: "2.0", id: 7, method: "tools/list", params: { _meta: meta } }, { "MCP-Protocol-Version": MODERN, "Mcp-Method": "tools/list" });
    const { status, body } = await send(r);
    expect(status).toBe(400);
    expect(body.error.code).toBe(-32602);
  });

  it("an unknown method is 404 with method-not-found", async () => {
    const { status, body } = await send(modern("prompts/list"));
    expect(status).toBe(404);
    expect(body.error.code).toBe(-32601);
  });
});

describe("malformed and oversized requests", () => {
  it("bad JSON is a parse error", async () => {
    const { status, body } = await send(req("{not json"));
    expect(status).toBe(400);
    expect(body).toEqual({ jsonrpc: "2.0", id: null, error: { code: -32700, message: "Parse error" } });
  });

  it("a body that is not UTF-8 is a parse error", async () => {
    const r = new Request(URL_, { method: "POST", body: new Uint8Array([0x7b, 0xff, 0xfe, 0x7d]) });
    expect((await send(r)).body.error.code).toBe(-32700);
  });

  it("batches are refused", async () => {
    const { status, body } = await send(req([legacy("ping"), legacy("ping", undefined, 2)]));
    expect(status).toBe(400);
    expect(body.error.code).toBe(-32600);
  });

  it("not JSON-RPC 2.0, a bad id or non-object params is an invalid request", async () => {
    for (const m of [{ id: 1, method: "ping" }, { jsonrpc: "2.0", id: null, method: "ping" }, { jsonrpc: "2.0", id: { a: 1 }, method: "ping" }, { jsonrpc: "2.0", id: 1, method: 5 }]) {
      const { status, body } = await send(req(m));
      expect(status).toBe(400);
      expect(body.error.code).toBe(-32600);
    }
    const { body } = await send(req({ jsonrpc: "2.0", id: 1, method: "tools/list", params: [1] }));
    expect(body.error.code).toBe(-32602);
  });

  it("a body over 16 KB is refused, by header or as read, before any data is read", async () => {
    const big = JSON.stringify(legacy("tools/call", { name: "latest_brief", arguments: { pad: "x".repeat(MAX_BODY_BYTES) } }));
    const c = counted(memoryD1(seed()));
    const d = deps({ db: c.db });
    expect((await send(req(big), d)).status).toBe(413);
    const stream = new ReadableStream({
      start(ctl) {
        ctl.enqueue(new TextEncoder().encode(big));
        ctl.close();
      },
    });
    const chunked = new Request(URL_, { method: "POST", body: stream, duplex: "half" } as RequestInit);
    expect(chunked.headers.get("content-length")).toBeNull();
    const { status, body } = await send(chunked, d);
    expect(status).toBe(413);
    expect(body.error.code).toBe(-32600);
    expect(c.sql).toHaveLength(0);
  });

  it("a client's JSON-RPC response is accepted with 202", async () => {
    expect((await handleMcpPost(req({ jsonrpc: "2.0", id: 3, result: {} }), deps())).status).toBe(202);
  });
});

describe("tool errors", () => {
  it("an unknown tool is a protocol error", async () => {
    const { body } = await send(req(legacy("tools/call", { name: "delete_everything", arguments: {} })));
    expect(body.error.code).toBe(-32602);
    expect(body.error.message).toBe("Unknown tool: delete_everything");
  });

  it("arguments that are not an object are a protocol error", async () => {
    const { body } = await send(req(legacy("tools/call", { name: "latest_brief", arguments: "kind=recap" })));
    expect(body.error.code).toBe(-32602);
  });

  it("bad argument values are a tool execution error the model can read", async () => {
    const { body, d } = await send(req(legacy("tools/call", { name: "earnings_report", arguments: { ticker: "<script>" } })));
    expect(body.result.isError).toBe(true);
    expect(body.result.content[0].text).toMatch(/Not a ticker/);
    expect(body.result.structuredContent).toBeUndefined();
    expect(d.lines).toEqual([expect.stringMatching(/^mcp tool=earnings_report error=arguments ms=\d+$/)]);
  });

  it("a database failure is a tool error with a generic message", async () => {
    const broken: D1Like = {
      prepare() {
        throw new Error("D1_ERROR: database is down");
      },
    };
    const spy = vi.spyOn(console, "error").mockImplementation(() => {});
    const { body, d } = await send(req(legacy("tools/call", { name: "latest_brief", arguments: {} })), deps({ db: broken }));
    spy.mockRestore();
    expect(body.result.isError).toBe(true);
    expect(body.result.content[0].text).not.toMatch(/D1/);
    expect(d.lines[0]).toMatch(/^mcp tool=latest_brief cache=miss error=internal ms=\d+$/);
  });
});

describe("cache and logs", () => {
  it("answers an identical call from the cache without reading D1 or R2 again", async () => {
    const c = counted(memoryD1(seed()));
    const d = deps({ db: c.db, cache: memoryCache() });
    const call = () => req(legacy("tools/call", { name: "earnings_report", arguments: { ticker: "aapl", lang: "zh" } }), { "CF-Connecting-IP": "203.0.113.9" });
    const first = await send(call(), d);
    const reads = c.sql.length;
    const gets = (d.bucket as unknown as { gets: string[] }).gets.length;
    expect(reads).toBeGreaterThan(0);
    const second = await send(call(), d);
    expect(second.body.result).toEqual(first.body.result);
    expect(c.sql.length).toBe(reads);
    expect((d.bucket as unknown as { gets: string[] }).gets.length).toBe(gets);
    expect(d.lines).toEqual([
      expect.stringMatching(/^mcp tool=earnings_report cache=miss ms=\d+$/),
      expect.stringMatching(/^mcp tool=earnings_report cache=hit ms=\d+$/),
    ]);
    // Normalized arguments: "aapl" and "AAPL" share one entry.
    await send(req(legacy("tools/call", { name: "earnings_report", arguments: { ticker: "AAPL", lang: "zh" } })), d);
    expect(d.lines[2]).toMatch(/cache=hit/);
    expect(d.lines.join("\n")).not.toContain("203.0.113.9");
  });

  it("works without a cache", async () => {
    const { body } = await send(req(legacy("tools/call", { name: "latest_brief", arguments: {} })), deps({ cache: null }));
    expect(body.result.isError).toBe(false);
  });

  it("logs one line per request, with the method", async () => {
    const { d } = await send(req(legacy("tools/list")));
    expect(d.lines).toEqual([expect.stringMatching(/^mcp method=tools\/list ms=\d+$/)]);
  });
});

describe("Origin", () => {
  it("refuses a browser origin other than the site's", async () => {
    const { status, body } = await send(req(legacy("tools/list"), { Origin: "https://evil.example" }));
    expect(status).toBe(403);
    expect(body.id).toBeNull();
    expect((await send(req(legacy("tools/list"), { Origin: "https://prebell.site" }))).status).toBe(200);
  });

  it("allows only localhost when SITE_URL is unset (local dev)", () => {
    expect(originAllowed("http://localhost:3000", undefined)).toBe(true);
    expect(originAllowed("http://127.0.0.1:8787", undefined)).toBe(true);
    expect(originAllowed("http://localhost.evil.example", undefined)).toBe(false);
    expect(originAllowed("http://localhost:8787", "https://prebell.site")).toBe(false);
  });
});

describe("off switch and GET", () => {
  it("answers 404 to everything when disabled", async () => {
    const res = await handleMcpPost(req(legacy("tools/list")), deps({ enabled: false }));
    expect(res.status).toBe(404);
    expect(handleMcpGet(new Request(URL_), { enabled: false, siteUrl: "https://prebell.site" }).status).toBe(404);
  });

  it("GET describes the server for a person, and refuses an event stream", async () => {
    const page = handleMcpGet(new Request(URL_, { headers: { Accept: "text/html" } }), { enabled: true, siteUrl: "https://prebell.site" });
    expect(page.status).toBe(200);
    expect(page.headers.get("content-type")).toBe("text/plain; charset=utf-8");
    const text = await page.text();
    expect(text).toContain("claude mcp add --transport http prebell https://prebell.site/mcp");
    expect(text).toMatch(/not investment advice/);
    const sse = handleMcpGet(new Request(URL_, { headers: { Accept: "text/event-stream" } }), { enabled: true, siteUrl: "https://prebell.site" });
    expect(sse.status).toBe(405);
    expect(sse.headers.get("allow")).toBe("POST");
  });
});

describe("review fixes", () => {
  it("B1: the reviewer's 9999-12-31 calendar request answers at once with an argument error", async () => {
    const started = Date.now();
    const { status, body } = await send(req(legacy("tools/call", { name: "earnings_calendar", arguments: { from: "9999-12-31", to: "9999-12-31" } })));
    expect(Date.now() - started).toBeLessThan(1000);
    expect(status).toBe(200);
    expect(body.result.isError).toBe(true);
    expect(body.result.content[0].text).toBe("from must be a date written YYYY-MM-DD.");
  });

  it("S3: only the canonical host is served; workers.dev and other hosts get 404", async () => {
    for (const host of ["https://prebell-web.example.workers.dev/mcp", "https://www.prebell.site/mcp", "https://evil.example/mcp"]) {
      const post = new Request(host, { method: "POST", body: JSON.stringify(legacy("tools/list")) });
      expect((await handleMcpPost(post, deps())).status).toBe(404);
      expect(handleMcpGet(new Request(host), { enabled: true, siteUrl: "https://prebell.site" }).status).toBe(404);
    }
    // Local preview: SITE_URL is prebell.site but the address is loopback.
    for (const host of ["http://localhost:8871/mcp", "http://127.0.0.1:8787/mcp"]) {
      const post = new Request(host, { method: "POST", body: JSON.stringify(legacy("ping")) });
      expect((await handleMcpPost(post, deps())).status).toBe(200);
    }
    // Without SITE_URL (next dev) any host is served.
    const any = new Request("https://whatever.example/mcp", { method: "POST", body: JSON.stringify(legacy("ping")) });
    expect((await handleMcpPost(any, deps({ siteUrl: undefined }))).status).toBe(200);
  });

  it("S5: a deeply nested tool name is a JSON-RPC error, not a crash", async () => {
    const depth = 7000;
    const body = `{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":${"[".repeat(depth)}${"]".repeat(depth)}}}`;
    expect(body.length).toBeLessThan(MAX_BODY_BYTES);
    const res = await send(req(body));
    expect(res.status).toBe(200);
    expect(res.body.error.code).toBe(-32602);
    expect(res.body.error.message).toBe("Unknown tool: object");
  });

  it("S5: any unexpected failure is a JSON-RPC internal error with no-store, never an HTML 500", async () => {
    const spy = vi.spyOn(console, "error").mockImplementation(() => {});
    const d = deps({
      log: () => {
        throw new Error("logger broke");
      },
    });
    const res = await handleMcpPost(req(legacy("tools/list")), d);
    spy.mockRestore();
    expect(res.status).toBe(500);
    expect(res.headers.get("content-type")).toBe("application/json");
    expect(res.headers.get("cache-control")).toBe("no-store");
    expect(await res.json()).toEqual({ jsonrpc: "2.0", id: null, error: { code: -32603, message: "Internal error" } });
  });

  it("N1: degraded answers are not cached", async () => {
    const errSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    const failing = { async get(): Promise<null> { throw new Error("r2 down"); } };
    const d = deps({ bucket: failing, cache: memoryCache(), onError: () => {} });
    const call = () => req(legacy("tools/call", { name: "latest_brief", arguments: {} }));
    expect((await send(call(), d)).body.result.structuredContent.market_data_available).toBe(false);
    await send(call(), d);
    errSpy.mockRestore();
    expect(d.lines).toEqual([
      expect.stringMatching(/^mcp tool=latest_brief cache=miss degraded ms=\d+$/),
      expect.stringMatching(/^mcp tool=latest_brief cache=miss degraded ms=\d+$/),
    ]);
    // A healthy answer is cached as before.
    const ok = deps({ cache: memoryCache() });
    await send(call(), ok);
    await send(call(), ok);
    expect(ok.lines[1]).toMatch(/cache=hit/);
  });
});

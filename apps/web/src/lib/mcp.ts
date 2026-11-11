/**
 * PreBell's read-only MCP server at `/mcp`: JSON-RPC 2.0 over Streamable HTTP, stateless, one JSON response per
 * POST (no sessions, no event streams, no server-initiated messages).
 *
 * It is dual-era (MCP 2026-07-28, "Versioning: Backward Compatibility"): a request carrying the per-request
 * `_meta` protocol version is served as revision 2026-07-28 (`server/discover`, header checks); anything else is
 * served as the handshake revisions 2025-11-25, 2025-06-18 and 2025-03-26 (`initialize`, `ping`). Both eras get the
 * same three tools.
 *
 * Public and unauthenticated, so: it answers only on the site's own host (not workers.dev), a browser `Origin`
 * other than the site's is refused (DNS rebinding and cross-site use), batches are refused, arguments are
 * validated before any read, healthy answers are cached for a minute, any unexpected failure is a JSON-RPC error
 * rather than a 500 page, and each request logs one line without the caller's address.
 *
 * The 16 KB body cap protects parsing only: OpenNext reads the whole request body into memory before the route
 * runs, so it cannot stop a large upload from being buffered. A site-wide guard in a custom Worker entry would.
 */

import { CACHE_TTL, INSTRUCTIONS, isToolName, prepareCall, TOOLS, ArgumentError, type ToolContext } from "./mcptools";
import type { EdgeCache } from "./sitemap";

export const MODERN_VERSIONS = ["2026-07-28"];
export const LEGACY_VERSIONS = ["2025-11-25", "2025-06-18", "2025-03-26"];
const SUPPORTED = [...MODERN_VERSIONS, ...LEGACY_VERSIONS];
/** The 2025-11-25 transport: a request without the version header is taken as 2025-03-26. */
const ASSUMED_LEGACY = "2025-03-26";

/** Larger bodies are refused before parsing (OpenNext has already buffered them; see above). */
export const MAX_BODY_BYTES = 16 * 1024;
export { CACHE_TTL };
const TOOLS_TTL_MS = 3_600_000;

const SERVER_INFO = {
  name: "prebell",
  title: "PreBell",
  version: "1.0.0",
  description: "US market briefs and earnings checks from prebell.site, read-only.",
  websiteUrl: "https://prebell.site",
};

export interface McpDeps extends ToolContext {
  /** `MCP_ENABLED === "true"`; otherwise the route answers 404. */
  enabled: boolean;
  /**
   * `SITE_URL` as configured: only its host (and loopback, for local dev) is served, and browser origins other
   * than its own are refused. Unset only in local dev, where any host is served.
   */
  siteUrl: string | undefined;
  /** Workers' `caches.default`; null where there is none (`next dev`, tests). */
  cache: EdgeCache | null;
  log: (line: string) => void;
  /** Keeps a cache write alive after the response (`ctx.waitUntil`); awaited when absent. */
  waitUntil?: (p: Promise<unknown>) => void;
}

type Id = string | number;
const PARSE_ERROR = -32700;
const INVALID_REQUEST = -32600;
const METHOD_NOT_FOUND = -32601;
const INVALID_PARAMS = -32602;
const INTERNAL_ERROR = -32603;
const HEADER_MISMATCH = -32020;
const UNSUPPORTED_VERSION = -32022;

const HEADERS = {
  "Content-Type": "application/json",
  "Cache-Control": "no-store",
  "X-Content-Type-Options": "nosniff",
};

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: HEADERS });
const failure = (id: Id | null, code: number, message: string, data?: unknown) => ({
  jsonrpc: "2.0",
  id,
  error: { code, message, ...(data === undefined ? {} : { data }) },
});
const notFound = () => new Response("Not found", { status: 404, headers: { "Content-Type": "text/plain; charset=utf-8" } });

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
/** A loggable, echoable form of a client value: a string's safe characters, else only its type (never `String()`, which a deeply nested array overflows). */
const short = (v: unknown) => (typeof v === "string" ? v.replace(/[^\w./-]/g, "?").slice(0, 60) : typeof v);

/**
 * Whether `/mcp` is served at this request's host: `SITE_URL`'s host only, so the workers.dev address (no WAF
 * rule, no Cache API) and any other host get 404. Loopback is served too: `next dev` reads the production
 * `SITE_URL` but answers on localhost (the wrangler preview already presents the route's host, prebell.site).
 * Without `SITE_URL` every host is served.
 */
export function hostAllowed(requestUrl: string, siteUrl: string | undefined): boolean {
  if (!siteUrl) return true;
  const { host, hostname } = new URL(requestUrl);
  return host === new URL(siteUrl).host || hostname === "localhost" || hostname === "127.0.0.1" || hostname === "[::1]";
}

/** A browser origin this server answers: the site's own (or, in local dev without `SITE_URL`, localhost). */
export function originAllowed(origin: string, siteUrl: string | undefined): boolean {
  if (siteUrl) return origin === new URL(siteUrl).origin;
  return /^http:\/\/(localhost|127\.0\.0\.1)(:\d+)?$/.test(origin);
}

/** The request body as text; "too_large" over `max` bytes (declared or as read); null when not UTF-8. */
async function readBody(req: Request, max: number): Promise<string | null | "too_large"> {
  const declared = Number(req.headers.get("content-length"));
  if (Number.isFinite(declared) && declared > max) return "too_large";
  if (!req.body) return "";
  const reader = req.body.getReader();
  const chunks: Uint8Array[] = [];
  let size = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    size += value.byteLength;
    if (size > max) {
      await reader.cancel().catch(() => {});
      return "too_large";
    }
    chunks.push(value);
  }
  const all = new Uint8Array(size);
  let at = 0;
  for (const c of chunks) {
    all.set(c, at);
    at += c.byteLength;
  }
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(all);
  } catch {
    return null;
  }
}

/** An `Mcp-Name` header value, decoding the `=?base64?...?=` form (2026-07-28 Streamable HTTP, Value Encoding). */
function headerValue(raw: string | null): string | null {
  if (raw === null) return null;
  const m = /^=\?base64\?([A-Za-z0-9+/=]*)\?=$/.exec(raw);
  if (!m) return raw;
  try {
    const bytes = Uint8Array.from(atob(m[1]), (c) => c.charCodeAt(0));
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    return null;
  }
}

const GET_TEXT = `PreBell MCP server (read-only)

This address is a Model Context Protocol (MCP) server for AI agents. It reads what
https://prebell.site has published: the earnings calendar, earnings checks with each
figure's source sentence and SEC filing link, and the latest premarket brief or
after-close recap as a time-stamped snapshot. No live quotes, no sign-in.

Connect (Streamable HTTP):
  Claude Code:  claude mcp add --transport http prebell https://prebell.site/mcp
  Other clients: {"mcpServers": {"prebell": {"type": "http", "url": "https://prebell.site/mcp"}}}

Tools: earnings_calendar, earnings_report, latest_brief (English and Chinese).
Summaries and reasons are machine-written from the linked sources.
Informational only; not investment advice; not real-time.
How to connect, for people: https://prebell.site/en/mcp
`;

/**
 * `GET /mcp`: a short description for a person who opens the address. A client asking for an event stream
 * (`Accept: text/event-stream`, the 2025 transports' optional GET stream) gets 405: this server has none.
 */
export function handleMcpGet(req: Request, deps: Pick<McpDeps, "enabled" | "siteUrl">): Response {
  if (!deps.enabled || !hostAllowed(req.url, deps.siteUrl)) return notFound();
  const accept = req.headers.get("accept") ?? "";
  if (accept.includes("text/event-stream") && !accept.includes("text/html"))
    return new Response(null, { status: 405, headers: { Allow: "POST" } });
  return new Response(GET_TEXT, {
    headers: {
      "Content-Type": "text/plain; charset=utf-8",
      "Cache-Control": "public, max-age=3600",
      "X-Content-Type-Options": "nosniff",
    },
  });
}

/**
 * `POST /mcp`: one JSON-RPC message in, one JSON response (or 202) out. Nothing a client sends can produce an
 * HTML 500: an unexpected failure is JSON-RPC -32603 with `Cache-Control: no-store`.
 */
export async function handleMcpPost(req: Request, deps: McpDeps): Promise<Response> {
  try {
    return await servePost(req, deps);
  } catch (e) {
    try {
      console.error("mcp internal error", e instanceof Error ? e.name : typeof e);
    } catch {}
    return json(failure(null, INTERNAL_ERROR, "Internal error"), 500);
  }
}

async function servePost(req: Request, deps: McpDeps): Promise<Response> {
  if (!deps.enabled || !hostAllowed(req.url, deps.siteUrl)) return notFound();
  const started = Date.now();
  const logLine = (fields: string) => deps.log(`mcp ${fields} ms=${Date.now() - started}`);

  const origin = req.headers.get("origin");
  if (origin !== null && !originAllowed(origin, deps.siteUrl)) {
    logLine("error=origin");
    return json(failure(null, INVALID_REQUEST, "Origin not allowed"), 403);
  }

  const raw = await readBody(req, MAX_BODY_BYTES);
  if (raw === "too_large") {
    logLine("error=too_large");
    return json(failure(null, INVALID_REQUEST, `Request body over ${MAX_BODY_BYTES} bytes`), 413);
  }
  let msg: unknown;
  try {
    msg = raw === null ? undefined : JSON.parse(raw);
  } catch {
    msg = undefined;
  }
  if (msg === undefined) {
    logLine("error=parse");
    return json(failure(null, PARSE_ERROR, "Parse error"), 400);
  }
  if (Array.isArray(msg)) {
    logLine("error=batch");
    return json(failure(null, INVALID_REQUEST, "Batch requests are not supported; send one message per POST"), 400);
  }
  if (!isObj(msg) || msg.jsonrpc !== "2.0") {
    logLine("error=invalid");
    return json(failure(null, INVALID_REQUEST, "Invalid Request"), 400);
  }

  // A response to a server request (this server sends none) or a notification: accepted, no body.
  if (msg.method === undefined && ("result" in msg || "error" in msg)) return new Response(null, { status: 202 });
  if (typeof msg.method !== "string") {
    logLine("error=invalid");
    return json(failure(null, INVALID_REQUEST, "Invalid Request"), 400);
  }
  const method = msg.method;
  if (!("id" in msg)) {
    logLine(`method=${short(method)} notification`);
    return new Response(null, { status: 202 });
  }
  const id = msg.id;
  if (!((typeof id === "number" && Number.isFinite(id)) || (typeof id === "string" && id.length <= 200))) {
    logLine("error=invalid_id");
    return json(failure(null, INVALID_REQUEST, "Invalid Request: id must be a string or a number"), 400);
  }
  if (msg.params !== undefined && !isObj(msg.params)) {
    logLine(`method=${short(method)} error=params`);
    return json(failure(id, INVALID_PARAMS, "Invalid params: params must be an object"), 400);
  }
  const params = (msg.params ?? {}) as Record<string, unknown>;
  const meta = isObj(params._meta) ? params._meta : {};
  const metaVersion = meta["io.modelcontextprotocol/protocolVersion"];
  const headerVersion = req.headers.get("mcp-protocol-version");

  // The legacy handshake. A dual-era server answers it with a legacy version (the requested one if supported).
  if (method === "initialize") {
    const requested = params.protocolVersion;
    const version = typeof requested === "string" && LEGACY_VERSIONS.includes(requested) ? requested : LEGACY_VERSIONS[0];
    logLine("method=initialize");
    return json({
      jsonrpc: "2.0",
      id,
      result: {
        protocolVersion: version,
        capabilities: { tools: { listChanged: false } },
        serverInfo: SERVER_INFO,
        instructions: INSTRUCTIONS,
      },
    });
  }

  let modern: boolean;
  if (metaVersion !== undefined) {
    // 2026-07-28: every request names its version in `_meta` and mirrors it, the method and the tool in headers.
    const mismatch = (what: string) => {
      logLine(`method=${short(method)} error=header`);
      return json(failure(id, HEADER_MISMATCH, `Header mismatch: ${what}`), 400);
    };
    if (headerVersion === null) return mismatch("MCP-Protocol-Version header is missing");
    if (headerVersion !== metaVersion) return mismatch("MCP-Protocol-Version does not match _meta");
    if (typeof metaVersion !== "string" || !MODERN_VERSIONS.includes(metaVersion)) {
      logLine(`method=${short(method)} error=version`);
      return json(
        failure(id, UNSUPPORTED_VERSION, "Unsupported protocol version", {
          supported: SUPPORTED,
          requested: String(metaVersion).slice(0, 40),
        }),
        400,
      );
    }
    if (req.headers.get("mcp-method") !== method) return mismatch("Mcp-Method header is missing or does not match");
    if (!isObj(meta["io.modelcontextprotocol/clientCapabilities"])) {
      logLine(`method=${short(method)} error=params`);
      return json(failure(id, INVALID_PARAMS, "Invalid params: _meta clientCapabilities is required"), 400);
    }
    if (method === "tools/call" && typeof params.name === "string" && headerValue(req.headers.get("mcp-name")) !== params.name)
      return mismatch("Mcp-Name header is missing or does not match params.name");
    modern = true;
  } else {
    const version = headerVersion ?? ASSUMED_LEGACY;
    if (!LEGACY_VERSIONS.includes(version)) {
      logLine(`method=${short(method)} error=version`);
      if (MODERN_VERSIONS.includes(version))
        return json(failure(id, INVALID_PARAMS, "Invalid params: _meta io.modelcontextprotocol/protocolVersion is required"), 400);
      return json(
        failure(id, UNSUPPORTED_VERSION, "Unsupported protocol version", {
          supported: SUPPORTED,
          requested: version.slice(0, 40),
        }),
        400,
      );
    }
    modern = false;
  }

  // Modern results carry `resultType` and the server's identity; legacy results are the bare object.
  const ok = (result: Record<string, unknown>) =>
    json({
      jsonrpc: "2.0",
      id,
      result: modern ? { resultType: "complete", ...result, _meta: { "io.modelcontextprotocol/serverInfo": SERVER_INFO } } : result,
    });

  switch (method) {
    case "ping":
      logLine("method=ping");
      return ok({});
    case "server/discover":
      if (!modern) break;
      logLine("method=server/discover");
      return ok({
        supportedVersions: SUPPORTED,
        capabilities: { tools: { listChanged: false } },
        instructions: INSTRUCTIONS,
        ttlMs: TOOLS_TTL_MS,
        cacheScope: "public",
      });
    case "tools/list":
      logLine("method=tools/list");
      return ok({ tools: TOOLS, ...(modern ? { ttlMs: TOOLS_TTL_MS, cacheScope: "public" } : {}) });
    case "tools/call":
      return callTool(id, params, deps, ok, logLine);
  }
  logLine(`method=${short(method)} error=method`);
  // 2026-07-28 answers an unknown method with 404; the 2025 revisions with an ordinary JSON-RPC error.
  return json(failure(id, METHOD_NOT_FOUND, "Method not found"), modern ? 404 : 200);
}

async function callTool(
  id: Id,
  params: Record<string, unknown>,
  deps: McpDeps,
  ok: (result: Record<string, unknown>) => Response,
  logLine: (fields: string) => void,
): Promise<Response> {
  const name = params.name;
  // An unknown tool or a malformed call is a protocol error; bad argument values are a tool error below.
  if (!isToolName(name)) {
    logLine("tool=unknown error=unknown_tool");
    return json(failure(id, INVALID_PARAMS, `Unknown tool: ${short(name)}`));
  }
  if (params.arguments !== undefined && !isObj(params.arguments)) {
    logLine(`tool=${name} error=params`);
    return json(failure(id, INVALID_PARAMS, "Invalid params: arguments must be an object"));
  }
  const toolError = (text: string) => ok({ content: [{ type: "text", text }], isError: true });

  // An answer built while `onError` fired (an R2 or D1 failure the tool answered around) is degraded: it is
  // returned but not cached, so the next call tries again.
  let degraded = false;
  const weeks = { hit: 0, miss: 0 };
  const ctx: ToolContext = {
    ...deps,
    onError: (e) => {
      degraded = true;
      deps.onError?.(e);
    },
    noteCache: (h) => void (h ? weeks.hit++ : weeks.miss++),
  };
  let call;
  try {
    call = prepareCall(name, (params.arguments ?? {}) as Record<string, unknown>, ctx);
  } catch (e) {
    if (!(e instanceof ArgumentError)) throw e;
    logLine(`tool=${name} error=arguments`);
    return toolError(e.message);
  }

  // The Cache API keeps GET responses only, so a POST's answer is stored under a synthetic GET of its
  // normalized arguments. Nothing serves this path; it is a cache key. A call without a key (the calendar)
  // caches its weeks itself.
  const key = call.key
    ? new Request(new URL(`/__mcp_cache/v1/${name}?${new URLSearchParams(Object.entries(call.key).sort())}`, deps.origin))
    : null;
  let data: Record<string, unknown> | null = null;
  let hit = false;
  try {
    const cached = key ? await deps.cache?.match(key) : undefined;
    if (cached) {
      data = (await cached.json()) as Record<string, unknown>;
      hit = true;
    }
  } catch {
    data = null;
  }
  if (!data) {
    try {
      data = await call.run();
    } catch (e) {
      console.error("mcp tool failed", name, e instanceof Error ? e.message.slice(0, 200) : "unknown");
      logLine(`tool=${name} cache=miss error=internal`);
      return toolError("PreBell could not read its data just now. Try again in a minute.");
    }
    if (degraded) {
      logLine(`tool=${name} cache=miss degraded`);
      return ok({ content: [{ type: "text", text: JSON.stringify(data) }], structuredContent: data, isError: false });
    }
    const store = key && deps.cache
      ?.put(
        key,
        new Response(JSON.stringify(data), {
          headers: { "Content-Type": "application/json", "Cache-Control": `public, max-age=${CACHE_TTL}` },
        }),
      )
      .catch(() => {});
    if (store) {
      if (deps.waitUntil) deps.waitUntil(store);
      else await store;
    }
  }
  // A call cached by parts (the calendar's weeks) is a hit when every part was.
  if (!key) hit = weeks.hit > 0 && weeks.miss === 0;
  logLine(`tool=${name} cache=${hit ? "hit" : "miss"}`);
  // The structured result, and the same JSON as text for clients that read only `content`.
  return ok({ content: [{ type: "text", text: JSON.stringify(data) }], structuredContent: data, isError: false });
}

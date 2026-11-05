import { afterEach, beforeEach, describe, expect, it, vi, type MockInstance } from "vitest";
import { POST } from "../src/app/api/subscribe/route";
import { subscribe, validEmail } from "../src/lib/subscribe";

const ctx = vi.hoisted(() => ({
  env: {} as Record<string, unknown>,
  throws: false,
}));
vi.mock("@opennextjs/cloudflare", () => ({
  getCloudflareContext: () => {
    if (ctx.throws) throw new Error("no Cloudflare context");
    return { env: ctx.env };
  },
  initOpenNextCloudflareForDev: () => {},
}));

const reply = (status: number, body: unknown = {}) =>
  Promise.resolve(new Response(typeof body === "string" ? body : JSON.stringify(body), { status }));

// The error paths log one line each; capture it so the run stays quiet and the content is checked.
let errors: MockInstance<typeof console.error>;
beforeEach(() => {
  errors = vi.spyOn(console, "error").mockImplementation(() => {});
});
afterEach(() => {
  errors.mockRestore();
});
const logged = () => errors.mock.calls.map((args) => args.join(" "));

describe("validEmail", () => {
  it("accepts ordinary addresses and rejects malformed ones", () => {
    for (const ok of ["a@b.co", "rick+tag@example.com", "first.last@sub.example.org"]) expect(validEmail(ok)).toBe(true);
    for (const bad of ["", "plain", "a@b", "@b.co", "a@.co", "a b@c.de", "a@b@c.de", "a@b.c o", "x".repeat(300) + "@a.co"])
      expect(validEmail(bad)).toBe(false);
  });
});

describe("subscribe", () => {
  const tags = ["prebell", "lang:en", "source:home"];

  it("maps 201 to ok, already-subscribed 400 to already, 500 to error", async () => {
    const ok = vi.fn(() => reply(201, { id: "x" }));
    expect(await subscribe("KEY", "a@b.co", tags, ok)).toBe("ok");
    const already = vi.fn(() => reply(400, { detail: "This email address is already subscribed." }));
    expect(await subscribe("KEY", "a@b.co", tags, already)).toBe("already");
    const other400 = vi.fn(() => reply(400, { detail: "Invalid email" }));
    expect(await subscribe("KEY", "a@b.co", tags, other400)).toBe("error");
    expect(await subscribe("KEY", "a@b.co", tags, vi.fn(() => reply(500, "boom")))).toBe("error");
    expect(await subscribe("KEY", "a@b.co", tags, vi.fn(() => Promise.reject(new Error("offline"))))).toBe("error");
    // Only the status (or "network error") is logged: never the upstream body, the error or the key.
    expect(logged()).toEqual([
      "buttondown subscribe failed, status 400",
      "buttondown subscribe failed, status 500",
      "buttondown subscribe failed, network error",
    ]);
  });

  it("posts the subscriber to Buttondown with the token and tags", async () => {
    const f = vi.fn<typeof fetch>(() => reply(201));
    await subscribe("KEY", "a@b.co", tags, f);
    const [url, init] = f.mock.calls[0];
    expect(url).toBe("https://api.buttondown.com/v1/subscribers");
    expect(init?.method).toBe("POST");
    expect((init?.headers as Record<string, string>).Authorization).toBe("Token KEY");
    expect(JSON.parse(init?.body as string)).toEqual({ email_address: "a@b.co", tags, type: "unactivated" });
  });
});

describe("POST /api/subscribe", () => {
  const post = (body: unknown) =>
    POST(
      new Request("https://prebell.example/api/subscribe", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: typeof body === "string" ? body : JSON.stringify(body),
      }),
    );
  const good = { email: "a@b.co", source: "home", lang: "en" };

  beforeEach(() => {
    ctx.env = { BUTTONDOWN_API_KEY: "SECRETKEY" };
    ctx.throws = false;
    vi.unstubAllGlobals();
  });

  it("rejects bad JSON, a bad email and a filled honeypot with 400", async () => {
    const f = vi.fn(() => reply(201));
    vi.stubGlobal("fetch", f);
    expect((await post("{not json")).status).toBe(400);
    expect((await post({ ...good, email: "nope" })).status).toBe(400);
    expect((await post({ ...good, website: "http://spam" })).status).toBe(400);
    expect((await post({ ...good, source: "elsewhere" })).status).toBe(400);
    expect((await post({ ...good, lang: "fr" })).status).toBe(400);
    expect((await post(null)).status).toBe(400);
    expect(f).not.toHaveBeenCalled();
  });

  it("returns 503 without a key, or without a Cloudflare context", async () => {
    ctx.env = {};
    const res = await post(good);
    expect(res.status).toBe(503);
    expect(res.headers.get("Cache-Control")).toBe("private, no-store");
    ctx.throws = true;
    expect((await post(good)).status).toBe(503);
  });

  it("returns 200 for ok and already, tagging the subscriber", async () => {
    const f = vi.fn<typeof fetch>(() => reply(201));
    vi.stubGlobal("fetch", f);
    const res = await post({ ...good, source: "post", lang: "zh", website: "" });
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({ ok: true });
    expect(res.headers.get("Cache-Control")).toBe("private, no-store");
    expect(JSON.parse(f.mock.calls[0][1]?.body as string).tags).toEqual(["prebell", "lang:zh", "source:post"]);
    vi.stubGlobal("fetch", vi.fn(() => reply(400, { detail: "already subscribed" })));
    expect(await (await post(good)).json()).toEqual({ ok: true });
    expect(errors).not.toHaveBeenCalled();
  });

  it("never echoes upstream error text or the key", async () => {
    vi.stubGlobal("fetch", vi.fn(() => reply(500, "UPSTREAM-LEAK Token SECRETKEY")));
    const res = await post(good);
    expect(res.status).toBe(502);
    expect(res.headers.get("Cache-Control")).toBe("private, no-store");
    const text = await res.text();
    expect(JSON.parse(text)).toEqual({ ok: false });
    expect(text).not.toContain("UPSTREAM-LEAK");
    expect(text).not.toContain("SECRETKEY");
    expect(logged()).toEqual(["buttondown subscribe failed, status 500"]);
  });
});

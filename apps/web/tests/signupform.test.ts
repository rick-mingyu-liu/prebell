import { describe, expect, it } from "vitest";
import { messageFor, statusOf } from "../src/components/SignupForm";
import { signup as en } from "../messages/en.json";
import zh from "../messages/zh.json";

const res = (status: number, body: unknown) =>
  new Response(typeof body === "string" ? body : JSON.stringify(body), { status });

describe("SignupForm result", () => {
  it("tells a new subscriber to confirm and an existing one that they are already on the list", async () => {
    expect(await statusOf(res(200, { ok: true }))).toBe("done");
    expect(await statusOf(res(200, { ok: true, already: true }))).toBe("already");
    expect(messageFor("done", en)).toBe("Check your inbox to confirm.");
    expect(messageFor("already", en)).toBe("You're already on the list.");
  });

  it("maps a 400 to invalid and any other failure to error", async () => {
    expect(await statusOf(res(400, { ok: false }))).toBe("invalid");
    expect(await statusOf(res(503, { ok: false }))).toBe("error");
    expect(await statusOf(res(502, { ok: false }))).toBe("error");
    // A 200 with an unreadable body is still a success: the subscriber was created.
    expect(await statusOf(res(200, "not json"))).toBe("done");
    expect(messageFor("invalid", en)).toBe("Enter a valid email address.");
    expect(messageFor("error", en)).toBe("Something went wrong. Please try again.");
    expect(messageFor("idle", en)).toBeNull();
    expect(messageFor("loading", en)).toBeNull();
  });

  it("describes what the digest holds: earnings checks and the week ahead, no briefs", () => {
    expect(en.copy).toBe("One email a week: three earnings checks and the week ahead. No spam, unsubscribe anytime.");
    expect(en.copy).not.toMatch(/brief/i);
  });

  it("is English only: the Chinese catalogue has no signup strings", () => {
    expect("signup" in zh).toBe(false);
  });
});

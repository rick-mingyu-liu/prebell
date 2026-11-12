import { describe, expect, it } from "vitest";
import { messageFor, statusOf } from "../src/components/SignupForm";
import { signup as en } from "../messages/en.json";
import { signup as zh } from "../messages/zh.json";

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
    expect(en.copy).toBe("One email a week: earnings checks from the past week, and the week ahead. No spam, unsubscribe anytime.");
    expect(en.copy).not.toMatch(/brief/i);
  });

  it("has every string in Chinese too, and the Chinese copy says the email is in English", () => {
    expect(Object.keys(zh).sort()).toEqual(Object.keys(en).sort());
    for (const [k, v] of Object.entries(zh)) expect(v, k).not.toBe("");
    expect(zh.copy).toContain("邮件目前只有英文版");
    expect(zh.success).toContain("英文");
    expect(zh.copy).toContain("财报速评");
    expect(zh.copy).not.toContain("简报");
    expect(messageFor("done", zh)).toBe(zh.success);
    expect(messageFor("already", zh)).toBe(zh.already);
    expect(messageFor("invalid", zh)).toBe(zh.invalid);
    expect(messageFor("error", zh)).toBe(zh.error);
  });
});

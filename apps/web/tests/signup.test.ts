import { describe, expect, it, vi } from "vitest";
import { buttondownKey, signupEnabled } from "../src/lib/signup";

const ctx = vi.hoisted(() => ({ env: {} as Record<string, unknown>, throws: false }));
vi.mock("@opennextjs/cloudflare", () => ({
  getCloudflareContext: () => {
    if (ctx.throws) throw new Error("no Cloudflare context");
    return { env: ctx.env };
  },
}));

describe("signupEnabled", () => {
  it("is on only when the Buttondown key is a non-empty string", () => {
    ctx.throws = false;
    ctx.env = { BUTTONDOWN_API_KEY: "KEY" };
    expect(signupEnabled()).toBe(true);
    expect(buttondownKey()).toBe("KEY");
    for (const bad of [undefined, "", 42, null]) {
      ctx.env = { BUTTONDOWN_API_KEY: bad };
      expect(signupEnabled(), String(bad)).toBe(false);
      expect(buttondownKey()).toBeNull();
    }
  });

  it("is off without a Cloudflare context", () => {
    ctx.throws = true;
    expect(signupEnabled()).toBe(false);
    expect(buttondownKey()).toBeNull();
  });
});

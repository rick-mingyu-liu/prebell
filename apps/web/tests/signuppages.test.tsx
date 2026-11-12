import { prerender } from "react-dom/static";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Locale } from "../src/i18n/routing";
import { translator } from "../src/lib/translator";
import { memoryD1 } from "./d1";

// The weekly digest is one English email; the signup is on pages in both languages, and the Chinese
// form says the email is in English. The pages read D1
// through getDb(), the Worker env (FACTS, BUTTONDOWN_API_KEY) through the Cloudflare context and
// messages through next-intl/server; each is swapped for an in-memory or synchronous stand-in.
const env = vi.hoisted(() => ({ db: null as unknown, locale: "en" as string, key: "KEY" as string | undefined }));
vi.mock("@/lib/db", () => ({ getDb: () => env.db }));
vi.mock("@opennextjs/cloudflare", () => ({
  getCloudflareContext: () => ({
    env: { FACTS: { get: async () => null, head: async () => null }, BUTTONDOWN_API_KEY: env.key },
  }),
}));
vi.mock("next/headers", () => ({ headers: async () => new Headers({ host: "prebell.site" }) }));
vi.mock("next-intl/server", () => ({
  setRequestLocale: (l: string) => {
    env.locale = l;
  },
  getTranslations: async (arg?: string | { locale: string; namespace?: string }) => {
    const ns = typeof arg === "string" ? arg : arg?.namespace;
    const t = translator((typeof arg === "object" ? arg.locale : env.locale) as Locale);
    return ns ? (k: string, v?: Record<string, unknown>) => t(`${ns}.${k}` as never, v as never) : t;
  },
}));

const post = await import("../src/app/[locale]/p/[slug]/page");
const home = await import("../src/app/[locale]/page");
const about = await import("../src/app/[locale]/about/page");

const seed = `
INSERT INTO runs VALUES ('premarket:2026-11-05','premarket','2026-11-05','published',1,NULL,'2026-11-05T12:00:00Z','2026-11-05T12:00:00Z');
INSERT INTO posts (id, run_key, content_type, template_id, trading_date, slug, facts_r2_key, published_at, cover_key, revised_at)
  VALUES (1,'premarket:2026-11-05','premarket','premarket:v1','2026-11-05','premarket-2026-11-05',NULL,'2026-11-05T12:01:00Z',NULL,NULL);
INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at)
  VALUES (1,'en','published','Futures slip as yields rise','Stocks set to open lower.','# x','2026-11-05T12:01:00Z');
INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at)
  VALUES (1,'zh','published','收益率上升，期货走低','美股料低开。','# x','2026-11-05T12:01:00Z');
`;

const props = (params: Record<string, string>) =>
  ({ params: Promise.resolve(params), searchParams: Promise.resolve({}) }) as never;
const html = async (el: React.ReactElement) => {
  const { prelude } = await prerender(el);
  return new Response(prelude).text();
};
const pages = {
  home: (locale: Locale) => home.default(props({ locale })),
  post: (locale: Locale) => post.default(props({ locale, slug: "premarket-2026-11-05" })),
  about: (locale: Locale) => about.default(props({ locale })),
};

describe("weekly-digest signup placement", () => {
  beforeEach(() => {
    env.db = memoryD1(seed);
    env.key = "KEY";
  });

  it("shows the form on the English home, post and About pages", async () => {
    for (const [name, page] of Object.entries(pages)) {
      const out = await html(await page("en"));
      expect(out, name).toContain('class="signup"');
      expect(out, name).toContain(`id="signup-email-${name}"`);
      expect(out, name).toContain("Get the weekly digest");
    }
  });

  it("shows it in Chinese on the Chinese home, post and About pages, saying the email is in English", async () => {
    for (const [name, page] of Object.entries(pages)) {
      const out = await html(await page("zh"));
      expect(out, name).toContain('class="signup"');
      expect(out, name).toContain(`id="signup-email-${name}"`);
      expect(out, name).toContain("订阅每周摘要");
      expect(out, name).toContain("邮件目前只有英文版");
      expect(out, name).not.toContain("Get the weekly digest");
    }
  });

  it("stays hidden in both languages until the web Worker has the key", async () => {
    env.key = undefined;
    for (const [name, page] of Object.entries(pages)) {
      for (const locale of ["en", "zh"] as const) {
        const out = await html(await page(locale));
        expect(out, `${name} ${locale}`).not.toContain('class="signup"');
      }
    }
  });
});

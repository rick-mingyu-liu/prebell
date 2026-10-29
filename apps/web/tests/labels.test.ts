import { describe, expect, it } from "vitest";
import { pendingTitle, typeLabel, type Translator } from "../src/lib/labels";

const messages: Record<string, string> = {
  "types.premarket": "Premarket brief",
  "types.recap": "After-close recap",
  "post.pendingTitle": "{type}, {date}",
};
const t = Object.assign(
  (key: string, values: Record<string, string> = {}) =>
    messages[key].replace(/\{(\w+)\}/g, (_, k: string) => values[k] ?? ""),
  { has: (key: string) => key in messages },
) as Translator;

describe("labels", () => {
  it("names known types and passes unknown ones through", () => {
    expect(typeLabel(t, "recap")).toBe("After-close recap");
    expect(typeLabel(t, "earnings")).toBe("earnings");
  });

  it("titles a pending post by its type and ET date", () => {
    expect(pendingTitle(t, { contentType: "recap", tradingDate: "2026-10-29" }, "en")).toMatch(
      /^After-close recap, Oct 29, 2026$/,
    );
  });
});

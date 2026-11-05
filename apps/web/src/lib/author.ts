import { existsSync } from "node:fs";
import { join } from "node:path";

/**
 * Whether `public/author.jpg` was shipped. `next.config.ts` checks the file once at build time and
 * inlines the answer as `HAS_AUTHOR_PHOTO`, because the deployed Worker has no file system to ask.
 * Without that flag (tests, scripts) the file is checked directly.
 */
function detect(): boolean {
  const flag = process.env.HAS_AUTHOR_PHOTO;
  if (flag !== undefined) return flag === "1";
  try {
    return existsSync(join(process.cwd(), "public", "author.jpg"));
  } catch {
    return false;
  }
}

export const hasAuthorPhoto: boolean = detect();

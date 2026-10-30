import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";

export default defineConfig({
  // Next resolves `server-only` itself (no npm package needed); tests run as server code, so give them
  // the package's server build, which is empty.
  // `@/` is the tsconfig path to src/, used by the page modules.
  resolve: {
    alias: {
      "server-only": "next/dist/compiled/server-only/empty.js",
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  test: {
    include: ["tests/**/*.test.{ts,tsx}"],
    environment: "node",
  },
});

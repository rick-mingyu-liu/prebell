import { defineConfig } from "vitest/config";

export default defineConfig({
  // Next resolves `server-only` itself (no npm package needed); tests run as server code, so give them
  // the package's server build, which is empty.
  resolve: { alias: { "server-only": "next/dist/compiled/server-only/empty.js" } },
  test: {
    include: ["tests/**/*.test.{ts,tsx}"],
    environment: "node",
  },
});

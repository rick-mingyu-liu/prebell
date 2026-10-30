import Database from "better-sqlite3";
import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import type { D1Like, D1StatementLike } from "../src/lib/posts";

const dir = fileURLToPath(new URL("../../../migrations/", import.meta.url));
/** Every numbered migration, in the order D1 applies them. */
export const migrations = readdirSync(dir)
  .filter((f) => /^\d+_.*\.sql$/.test(f))
  .sort();
const schema = migrations.map((f) => readFileSync(dir + f, "utf8")).join("\n");

/** An in-memory SQLite with the real D1 schema, behind the D1 calls posts.ts uses. */
export function memoryD1(seedSql: string): D1Like {
  const sqlite = new Database(":memory:");
  sqlite.exec(schema);
  sqlite.exec(seedSql);
  return {
    prepare(sql: string) {
      const st = sqlite.prepare(sql);
      let args: unknown[] = [];
      const stmt: D1StatementLike = {
        bind(...values: unknown[]) {
          args = values;
          return stmt;
        },
        async first<T>() {
          return (st.get(...args) as T | undefined) ?? null;
        },
        async all<T>() {
          return { results: st.all(...args) as T[] };
        },
      };
      return stmt;
    },
  };
}

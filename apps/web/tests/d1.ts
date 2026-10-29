import Database from "better-sqlite3";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import type { D1Like, D1StatementLike } from "../src/lib/posts";

const schema = readFileSync(fileURLToPath(new URL("../../../migrations/0001_init.sql", import.meta.url)), "utf8");

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

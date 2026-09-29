import { expect, test } from "bun:test";
import { Database } from "bun:sqlite";
import type { DashboardDb } from "../dashboard/types.ts";
import { runMigrations } from "./runner.ts";

test("TOTP migration resumes after interruption between column additions", async () => {
  const sqlite = new Database(":memory:");
  let interrupt = true;
  const db: DashboardDb = {
    driver: "sqlite",
    async query<T>(sql: string, params = []) {
      return sqlite.query(sql).all(...params) as T[];
    },
    async execute(sql, params = []) {
      if (
        interrupt &&
        sql.startsWith("ALTER TABLE dashboard_users ADD COLUMN auth_version")
      ) {
        interrupt = false;
        throw new Error("simulated disconnect");
      }
      return sqlite.query(sql).run(...params).changes;
    },
    async close() {},
  };
  try {
    await expect(runMigrations(db)).rejects.toThrow("simulated disconnect");
    expect(await runMigrations(db)).toEqual(["002_dashboard_totp"]);
    expect(await runMigrations(db)).toEqual([]);
    const columns = await db.query<{ name: string }>(
      "PRAGMA table_info(dashboard_users)",
    );
    expect(
      columns.filter((column) => column.name === "totp_state"),
    ).toHaveLength(1);
    expect(
      columns.filter((column) => column.name === "auth_version"),
    ).toHaveLength(1);
  } finally {
    sqlite.close();
  }
});

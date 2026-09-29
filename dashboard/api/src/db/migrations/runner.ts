import type { DashboardDb, DashboardDbDriver } from "../dashboard/types.ts";

export interface Migration {
  id: string;
  columns?: {
    table: string;
    name: string;
    definition: Record<DashboardDbDriver, string>;
  }[];
  up: Record<DashboardDbDriver, string[]>;
}

export const migrations: Migration[] = [
  {
    id: "001_dashboard_users",
    up: {
      pgsql: [
        `CREATE TABLE IF NOT EXISTS dashboard_migrations (
          id VARCHAR(255) PRIMARY KEY,
          applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )`,
        `CREATE TABLE IF NOT EXISTS dashboard_users (
          id VARCHAR(36) PRIMARY KEY,
          email VARCHAR(255) NOT NULL UNIQUE,
          password_hash VARCHAR(255) NOT NULL,
          name VARCHAR(255) NOT NULL DEFAULT '',
          role VARCHAR(50) NOT NULL DEFAULT 'operator',
          active BOOLEAN NOT NULL DEFAULT TRUE,
          created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
          updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )`,
        `CREATE INDEX IF NOT EXISTS idx_dashboard_users_email ON dashboard_users(email)`,
      ],
      mysql: [
        `CREATE TABLE IF NOT EXISTS dashboard_migrations (
          id VARCHAR(255) PRIMARY KEY,
          applied_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
        )`,
        `CREATE TABLE IF NOT EXISTS dashboard_users (
          id VARCHAR(36) PRIMARY KEY,
          email VARCHAR(255) NOT NULL UNIQUE,
          password_hash VARCHAR(255) NOT NULL,
          name VARCHAR(255) NOT NULL DEFAULT '',
          role VARCHAR(50) NOT NULL DEFAULT 'operator',
          active TINYINT(1) NOT NULL DEFAULT 1,
          created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
          updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP
        )`,
        `CREATE INDEX idx_dashboard_users_email ON dashboard_users(email)`,
      ],
      sqlite: [
        `CREATE TABLE IF NOT EXISTS dashboard_migrations (
          id TEXT PRIMARY KEY,
          applied_at TEXT NOT NULL DEFAULT (datetime('now'))
        )`,
        `CREATE TABLE IF NOT EXISTS dashboard_users (
          id TEXT PRIMARY KEY,
          email TEXT NOT NULL UNIQUE,
          password_hash TEXT NOT NULL,
          name TEXT NOT NULL DEFAULT '',
          role TEXT NOT NULL DEFAULT 'operator',
          active INTEGER NOT NULL DEFAULT 1,
          created_at TEXT NOT NULL DEFAULT (datetime('now')),
          updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )`,
        `CREATE INDEX IF NOT EXISTS idx_dashboard_users_email ON dashboard_users(email)`,
      ],
    },
  },
  {
    id: "002_dashboard_totp",
    columns: [
      {
        table: "dashboard_users",
        name: "totp_state",
        definition: {
          pgsql: "TEXT NOT NULL DEFAULT '{}'",
          mysql: "TEXT NULL",
          sqlite: "TEXT NOT NULL DEFAULT '{}'",
        },
      },
      {
        table: "dashboard_users",
        name: "auth_version",
        definition: {
          pgsql: "VARCHAR(36) NOT NULL DEFAULT ''",
          mysql: "VARCHAR(36) NOT NULL DEFAULT ''",
          sqlite: "TEXT NOT NULL DEFAULT ''",
        },
      },
    ],
    up: {
      pgsql: [],
      mysql: [
        "UPDATE dashboard_users SET totp_state = '{}' WHERE totp_state IS NULL",
      ],
      sqlite: [],
    },
  },
];

export async function runMigrations(db: DashboardDb): Promise<string[]> {
  const applied: string[] = [];

  for (const migration of migrations) {
    const existing = await db
      .query<{ id: string }>(
        "SELECT id FROM dashboard_migrations WHERE id = ?",
        [migration.id],
      )
      .catch(() => [] as { id: string }[]);

    if (existing.length > 0) continue;

    // MySQL DDL commits implicitly. Inspect columns so an interrupted migration can resume.
    for (const column of migration.columns ?? []) {
      let names: string[];
      if (db.driver === "sqlite") {
        const rows = await db.query<{ name: string }>(
          `PRAGMA table_info(${column.table})`,
        );
        names = rows.map((row) => row.name);
      } else {
        const schema =
          db.driver === "mysql" ? "DATABASE()" : "current_schema()";
        const rows = await db.query<{ column_name: string }>(
          `SELECT column_name AS column_name FROM information_schema.columns WHERE table_schema = ${schema} AND table_name = ?`,
          [column.table],
        );
        names = rows.map((row) => row.column_name);
      }
      if (!names.includes(column.name)) {
        await db.execute(
          `ALTER TABLE ${column.table} ADD COLUMN ${column.name} ${column.definition[db.driver]}`,
        );
      }
    }

    const statements = migration.up[db.driver];
    for (const sql of statements) {
      await db.execute(sql);
    }

    await db.execute("INSERT INTO dashboard_migrations (id) VALUES (?)", [
      migration.id,
    ]);
    applied.push(migration.id);
  }

  return applied;
}

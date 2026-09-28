import { afterEach, expect, test } from "bun:test";
import { Database } from "bun:sqlite";
import { Hono } from "hono";
import { createSession, sessionCookie } from "../auth/session.ts";
import { verifyPassword } from "../auth/password.ts";
import { runMigrations } from "../db/migrations/runner.ts";
import type { DashboardDb } from "../db/dashboard/types.ts";
import { UsersRepository } from "../db/users-repository.ts";
import { createUsersRoutes } from "./users.ts";
import type { DashboardUser } from "../types/user.ts";

const databases: Database[] = [];
afterEach(() => {
  for (const db of databases.splice(0)) db.close();
});

async function fixture() {
  const sqlite = new Database(":memory:");
  databases.push(sqlite);
  const db: DashboardDb = {
    driver: "sqlite",
    async query<T>(sql: string, params = []) {
      return sqlite.query(sql).all(...params) as T[];
    },
    async execute(sql, params = []) {
      return sqlite.query(sql).run(...params).changes;
    },
    async close() {},
  };
  await runMigrations(db);
  const users = new UsersRepository(db);
  const admin = await users.create({
    email: "admin@example.com",
    password: "original-password",
    role: "admin",
  });
  const operator = await users.create({
    email: "operator@example.com",
    password: "original-password",
    role: "operator",
  });
  const app = new Hono();
  app.route("/users", createUsersRoutes(users));
  async function cookie(user: DashboardUser) {
    return sessionCookie(
      await createSession({
        id: user.id,
        email: user.email,
        name: user.name,
        role: user.role,
        passwordHash: user.password_hash,
        authVersion: user.auth_version,
      }),
    ).split(";")[0]!;
  }
  async function request(
    user: DashboardUser,
    target: DashboardUser,
    body: unknown,
    path = "",
    method = "PUT",
  ) {
    return app.request(`/users/${target.id}${path}`, {
      method,
      headers: {
        "content-type": "application/json",
        cookie: await cookie(user),
      },
      body: JSON.stringify(body),
    });
  }
  return { users, admin, operator, app, cookie, request };
}

test("generic profile updates reject self-service password changes for operators and admins", async () => {
  const f = await fixture();
  for (const user of [f.admin, f.operator]) {
    const response = await f.request(user, user, {
      password: "attacker-chosen",
    });
    expect(response.status).toBe(400);
    expect((await response.json()).error).toContain("change-password");
    expect((await f.users.findById(user.id))!.password_hash).toBe(
      user.password_hash,
    );
    expect((await f.request(user, user, { name: "Updated" })).status).toBe(200);
  }
});

test("self password changes verify the existing password and invalidate prior sessions", async () => {
  const f = await fixture();
  for (const user of [f.admin, f.operator]) {
    expect(
      (
        await f.request(
          user,
          user,
          { new_password: "new-password" },
          "/change-password",
          "POST",
        )
      ).status,
    ).toBe(400);
    expect(
      (
        await f.request(
          user,
          user,
          { current_password: "wrong", new_password: "new-password" },
          "/change-password",
          "POST",
        )
      ).status,
    ).toBe(401);
    expect(
      (
        await f.request(
          user,
          user,
          {
            current_password: "original-password",
            new_password: "new-password",
          },
          "/change-password",
          "POST",
        )
      ).status,
    ).toBe(200);
    const updated = (await f.users.findById(user.id))!;
    expect(await verifyPassword("new-password", updated.password_hash)).toBe(
      true,
    );
    expect(
      (
        await f.app.request(`/users/${user.id}`, {
          headers: { cookie: await f.cookie(user) },
        })
      ).status,
    ).toBe(401);
  }
});

test("administrators can reset another user's password and preserve their second factor", async () => {
  const f = await fixture();
  const enrolled = await f.users.compareAndSetTotp(
    f.operator,
    JSON.stringify({
      secret: "encrypted-test-material",
      recoveryHashes: ["hash"],
    }),
    true,
  );
  const cookie = await f.cookie(enrolled!);
  const response = await f.request(
    f.admin,
    f.operator,
    { new_password: "reset-password" },
    "/change-password",
    "POST",
  );
  expect(response.status).toBe(200);
  const updated = (await f.users.findById(f.operator.id))!;
  expect(await verifyPassword("reset-password", updated.password_hash)).toBe(
    true,
  );
  expect(updated.totp_state).toBe(enrolled!.totp_state);
  expect(
    (await f.app.request(`/users/${f.operator.id}`, { headers: { cookie } }))
      .status,
  ).toBe(401);
  expect(
    (await f.request(f.admin, updated, { password: "another-reset" })).status,
  ).toBe(200);
});

test("operators cannot reset another user's password", async () => {
  const f = await fixture();
  expect(
    (await f.request(f.operator, f.admin, { password: "attacker-chosen" }))
      .status,
  ).toBe(403);
  expect(
    (
      await f.request(
        f.operator,
        f.admin,
        { new_password: "attacker-chosen" },
        "/change-password",
        "POST",
      )
    ).status,
  ).toBe(403);
  expect((await f.users.findById(f.admin.id))!.password_hash).toBe(
    f.admin.password_hash,
  );
});

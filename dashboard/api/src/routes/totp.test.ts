import { afterEach, describe, expect, test } from "bun:test";
import { Database } from "bun:sqlite";
import { Hono } from "hono";
import { Secret, TOTP } from "otpauth";
import { LoginRateLimiter } from "../auth/login-rate-limit.ts";
import { hashCode, totpState } from "../auth/totp.ts";
import type { DashboardDb } from "../db/dashboard/types.ts";
import { migrations, runMigrations } from "../db/migrations/runner.ts";
import { UsersRepository } from "../db/users-repository.ts";
import { createAuthRoutes } from "./auth.ts";

const databases: Database[] = [];
afterEach(() => {
  for (const db of databases.splice(0)) db.close();
});

async function fixture(
  key: Buffer | null = Buffer.alloc(32, 9),
  maxAttempts = 100,
) {
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
  expect(await runMigrations(db)).toEqual([]);
  const users = new UsersRepository(db);
  const user = await users.create({
    email: "operator@example.com",
    password: "test-password",
    role: "operator",
  });
  let time = 1_800_000;
  const makeLimiter = () =>
    new LoginRateLimiter({
      windowMs: 15 * 60_000,
      maxPerIp: maxAttempts,
      maxPerAccount: maxAttempts,
      maxTrackedIps: 100,
      maxTrackedAccounts: 100,
      now: () => time,
    });
  const app = new Hono();
  app.route(
    "/auth",
    createAuthRoutes(users, {
      totpKey: key,
      now: () => time,
      limiter: makeLimiter(),
      totpLimiter: makeLimiter(),
      resolveSourceIp: () => "192.0.2.1",
    }),
  );
  async function post(path: string, body: unknown, cookie = "") {
    return app.request(`/auth${path}`, {
      method: "POST",
      headers: { "content-type": "application/json", cookie },
      body: JSON.stringify(body),
    });
  }
  const login = () =>
    post("/login", { email: user.email, password: "test-password" });
  const cookieOf = (res: Response) =>
    res.headers.get("set-cookie")?.split(";")[0] ?? "";
  const cookie = cookieOf(await login());
  async function enroll() {
    const setup = await post(
      "/totp/setup",
      { password: "test-password" },
      cookie,
    );
    expect(setup.status).toBe(200);
    const { secret, otpauth_uri } = await setup.json();
    const code = (at = time) =>
      new TOTP({ secret: Secret.fromBase32(secret) }).generate({
        timestamp: at,
      });
    const enabled = await post(
      "/totp/enable",
      { password: "test-password", code: code() },
      cookie,
    );
    expect(enabled.status).toBe(200);
    return {
      secret,
      otpauth_uri,
      code,
      enabledCookie: cookieOf(enabled),
      ...(await enabled.json()),
    };
  }
  return {
    app,
    post,
    login,
    cookie,
    cookieOf,
    users,
    user,
    enroll,
    advance: (ms = 30_000) => {
      time += ms;
    },
  };
}

describe("dashboard two-factor endpoints", () => {
  test("migrations define all supported database dialects", () => {
    const migration = migrations.find((m) => m.id === "002_dashboard_totp")!;
    for (const driver of ["mysql", "pgsql", "sqlite"] as const) {
      expect(migration.columns!.map((column) => column.name)).toEqual([
        "totp_state",
        "auth_version",
      ]);
      expect(
        migration.columns!.every((column) => column.definition[driver]),
      ).toBe(true);
    }
  });

  test("enrollment requires password and confirmation; secrets never appear in public records", async () => {
    const f = await fixture();
    expect(
      (await f.post("/totp/setup", { password: "wrong" }, f.cookie)).status,
    ).toBe(401);
    expect(
      (await f.post("/totp/setup", { password: "test-password" })).status,
    ).toBe(401);
    const setup = await f.post(
      "/totp/setup",
      { password: "test-password" },
      f.cookie,
    );
    const material = await setup.json();
    const pending = await f.users.findById(f.user.id);
    expect(pending!.totp_state).not.toContain(material.secret);
    expect((await (await f.login()).json()).totp_enabled).toBe(false);
    expect(
      (
        await f.post(
          "/totp/enable",
          { password: "test-password", code: "invalid" },
          f.cookie,
        )
      ).status,
    ).toBe(401);
    const enrollment = await f.enroll();
    const me = await f.app.request("/auth/me", {
      headers: { cookie: enrollment.enabledCookie },
    });
    const publicUser = await me.json();
    expect(publicUser.totp_enabled).toBe(true);
    expect(publicUser).not.toHaveProperty("totp_state");
    expect(publicUser).not.toHaveProperty("password_hash");
    expect(
      (await f.app.request("/auth/me", { headers: { cookie: f.cookie } }))
        .status,
    ).toBe(401);
    expect(
      (
        await f.post(
          "/totp/setup",
          { password: "test-password" },
          enrollment.enabledCookie,
        )
      ).status,
    ).toBe(409);
  });

  test("password alone issues no session, challenges are single use and TOTP cannot replay", async () => {
    const f = await fixture();
    const enrollment = await f.enroll();
    const login = await f.login();
    expect(login.headers.get("set-cookie")).toBeNull();
    expect(login.headers.get("cache-control")).toBe("no-store");
    const challenge = (await login.json()).challenge;
    expect(
      (await f.post("/totp/verify", { challenge, code: enrollment.code() }))
        .status,
    ).toBe(401);
    f.advance();
    const verified = await f.post("/totp/verify", {
      challenge,
      code: enrollment.code(),
    });
    expect(verified.status).toBe(200);
    expect(f.cookieOf(verified)).not.toBe("");
    expect(
      (await f.post("/totp/verify", { challenge, code: enrollment.code() }))
        .status,
    ).toBe(401);
    const another = (await (await f.login()).json()).challenge;
    expect(
      (
        await f.post("/totp/verify", {
          challenge: another,
          code: enrollment.code(),
        })
      ).status,
    ).toBe(401);
  });

  test("recovery consumption is atomic across concurrent requests and remains one use", async () => {
    const f = await fixture();
    const enrollment = await f.enroll();
    const code = enrollment.recovery_codes[0];
    const stored = await f.users.findById(f.user.id);
    expect(stored!.totp_state).not.toContain(code);
    expect(stored!.totp_state).toContain(hashCode(code));
    const challenge = (await (await f.login()).json()).challenge;
    const responses = await Promise.all(
      Array.from({ length: 4 }, () =>
        f.post("/totp/verify", { challenge, code }),
      ),
    );
    expect(responses.filter((r) => r.status === 200)).toHaveLength(1);
    expect(responses.filter((r) => r.headers.has("set-cookie"))).toHaveLength(
      1,
    );
    const next = (await (await f.login()).json()).challenge;
    expect(
      (await f.post("/totp/verify", { challenge: next, code })).status,
    ).toBe(401);
    expect(
      totpState((await f.users.findById(f.user.id))!).recoveryHashes,
    ).toHaveLength(9);
  });

  test("challenges expire, cap guesses durably, and reject stale password versions", async () => {
    const f = await fixture();
    const enrollment = await f.enroll();
    let challenge = (await (await f.login()).json()).challenge;
    f.advance(5 * 60_000);
    expect(
      (await f.post("/totp/verify", { challenge, code: enrollment.code() }))
        .status,
    ).toBe(401);
    challenge = (await (await f.login()).json()).challenge;
    for (let i = 0; i < 5; i++)
      expect(
        (await f.post("/totp/verify", { challenge, code: "wrong" })).status,
      ).toBe(401);
    expect(
      totpState((await f.users.findById(f.user.id))!).challenge?.attempts,
    ).toBe(5);
    expect(
      (
        await f.post("/totp/verify", {
          challenge,
          code: enrollment.recovery_codes[0],
        })
      ).status,
    ).toBe(401);
    challenge = (await (await f.login()).json()).challenge;
    await f.users.update(f.user.id, { password: "new-password" });
    expect(
      (
        await f.post("/totp/verify", {
          challenge,
          code: enrollment.recovery_codes[0],
        })
      ).status,
    ).toBe(401);
  });

  test("replacing a challenge invalidates its predecessor and disabled users cannot finish", async () => {
    const f = await fixture();
    const enrollment = await f.enroll();
    const first = (await (await f.login()).json()).challenge;
    const second = (await (await f.login()).json()).challenge;
    expect(
      (
        await f.post("/totp/verify", {
          challenge: first,
          code: enrollment.recovery_codes[0],
        })
      ).status,
    ).toBe(401);
    await f.users.update(f.user.id, { active: false });
    expect(
      (
        await f.post("/totp/verify", {
          challenge: second,
          code: enrollment.recovery_codes[0],
        })
      ).status,
    ).toBe(401);
  });

  test("regeneration and disable require both factors and invalidate previous sessions", async () => {
    const f = await fixture();
    const enrollment = await f.enroll();
    expect(
      (
        await f.post(
          "/totp/disable",
          { password: "wrong", code: enrollment.recovery_codes[0] },
          enrollment.enabledCookie,
        )
      ).status,
    ).toBe(401);
    expect(
      (
        await f.post(
          "/totp/recovery-codes",
          { password: "test-password", code: "wrong" },
          enrollment.enabledCookie,
        )
      ).status,
    ).toBe(401);
    const regenerated = await f.post(
      "/totp/recovery-codes",
      { password: "test-password", code: enrollment.recovery_codes[0] },
      enrollment.enabledCookie,
    );
    expect(regenerated.status).toBe(200);
    const recovery = (await regenerated.json()).recovery_codes;
    expect(
      (
        await f.app.request("/auth/me", {
          headers: { cookie: enrollment.enabledCookie },
        })
      ).status,
    ).toBe(401);
    const current = f.cookieOf(regenerated);
    expect(
      (
        await f.post(
          "/totp/disable",
          { password: "test-password", code: enrollment.recovery_codes[1] },
          current,
        )
      ).status,
    ).toBe(401);
    const disabled = await f.post(
      "/totp/disable",
      { password: "test-password", code: recovery[0] },
      current,
    );
    expect(disabled.status).toBe(200);
    expect((await disabled.json()).totp_enabled).toBe(false);
    expect(
      (await f.app.request("/auth/me", { headers: { cookie: current } }))
        .status,
    ).toBe(401);
    expect((await (await f.login()).json()).totp_enabled).toBe(false);
  });

  test("pending setup expires and fails after a password change", async () => {
    const f = await fixture();
    const setup = await f.post(
      "/totp/setup",
      { password: "test-password" },
      f.cookie,
    );
    const { secret } = await setup.json();
    f.advance(10 * 60_000);
    expect(
      (
        await f.post(
          "/totp/enable",
          { password: "test-password", code: "123456" },
          f.cookie,
        )
      ).status,
    ).toBe(409);
    await f.users.update(f.user.id, { password: "new-password" });
    const login = await f.post("/login", {
      email: f.user.email,
      password: "new-password",
    });
    expect(
      (
        await f.post(
          "/totp/enable",
          {
            password: "new-password",
            code: new TOTP({ secret: Secret.fromBase32(secret) }).generate(),
          },
          f.cookieOf(login),
        )
      ).status,
    ).toBe(409);
  });

  test("optional key preserves password-only login but missing/wrong keys fail closed for MFA", async () => {
    const absent = await fixture(null);
    expect((await absent.login()).status).toBe(200);
    expect(
      (
        await absent.post(
          "/totp/setup",
          { password: "test-password" },
          absent.cookie,
        )
      ).status,
    ).toBe(503);
    const f = await fixture();
    const enrollment = await f.enroll();
    const row = (await absent.users.findById(absent.user.id))!;
    const otherState = (await f.users.findById(f.user.id))!.totp_state!;
    await absent.users.compareAndSetTotp(row, otherState, true);
    expect((await absent.login()).status).toBe(503);
    const current = (await f.users.findById(f.user.id))!;
    const state = totpState(current);
    state.secret = `${state.secret!.slice(0, -4)}AAAA`;
    await f.users.compareAndSetTotp(current, JSON.stringify(state));
    const challenge = (await (await f.login()).json()).challenge;
    f.advance();
    const response = await f.post("/totp/verify", {
      challenge,
      code: enrollment.code(),
    });
    expect(response.status).toBe(503);
    expect(response.headers.get("set-cookie")).toBeNull();
  });

  test("management and verification attempts are limited with Retry-After", async () => {
    const f = await fixture(Buffer.alloc(32, 9), 2);
    for (let i = 0; i < 2; i++)
      expect(
        (await f.post("/totp/setup", { password: "wrong" }, f.cookie)).status,
      ).toBe(401);
    const response = await f.post(
      "/totp/setup",
      { password: "test-password" },
      f.cookie,
    );
    expect(response.status).toBe(429);
    expect(response.headers.get("retry-after")).toBe("900");
  });
});

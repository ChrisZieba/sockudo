import { Hono, type Context } from "hono";
import { createRequireAuth } from "../auth/middleware.ts";
import { verifyPassword } from "../auth/password.ts";
import { createSession, sessionCookie } from "../auth/session.ts";
import {
  consumeFactor,
  decryptTotp,
  encryptTotp,
  hashCode,
  matchesHash,
  newEnrollment,
  newRecoveryCodes,
  totpState,
  userCredentialVersion,
  verifyTotp,
} from "../auth/totp.ts";
import type { LoginRateLimiter } from "../auth/login-rate-limit.ts";
import type { UsersRepository } from "../db/users-repository.ts";
import type { AppVariables } from "../types/hono.ts";
import { toPublicUser, type DashboardUser } from "../types/user.ts";

interface TotpOptions {
  key: Buffer | null;
  now: () => number;
  limiter: LoginRateLimiter;
  loginLimiter: LoginRateLimiter;
  resolveSourceIp: (c: Context) => string;
}

async function setSession(c: Context, user: DashboardUser): Promise<void> {
  const token = await createSession({
    id: user.id,
    email: user.email,
    name: user.name,
    role: user.role,
    passwordHash: user.password_hash,
    authVersion: user.auth_version,
  });
  c.header("Set-Cookie", sessionCookie(token));
}

export function createTotpRoutes(users: UsersRepository, options: TotpOptions) {
  const routes = new Hono<{ Variables: AppVariables }>();
  const requireAuth = createRequireAuth(users);
  const { now, key, limiter } = options;

  routes.use("*", async (c, next) => {
    c.header("Cache-Control", "no-store");
    if (!key)
      return c.json({ error: "Two-factor authentication is unavailable" }, 503);
    await next();
  });

  routes.post("/verify", async (c) => {
    const body = await c.req.json().catch(() => null);
    if (
      !body ||
      typeof body.challenge !== "string" ||
      body.challenge.length > 128 ||
      typeof body.code !== "string" ||
      body.code.length > 64
    ) {
      return c.json({ error: "challenge and code are required" }, 400);
    }
    const userId = body.challenge.split(".")[0];
    if (!/^[a-f0-9-]{36}$/.test(userId))
      return c.json({ error: "Invalid or expired challenge" }, 401);
    const rate = limiter.consume(options.resolveSourceIp(c), hashCode(userId));
    if (rate.limited) {
      c.header("Retry-After", String(rate.retryAfterSeconds));
      return c.json({ error: "Too many authentication attempts" }, 429);
    }
    const user = await users.findById(userId);
    if (!user?.active)
      return c.json({ error: "Invalid or expired challenge" }, 401);
    const state = totpState(user);
    const challenge = state.challenge;
    if (
      !state.secret ||
      !challenge ||
      challenge.expiresAt <= now() ||
      challenge.attempts >= 5 ||
      challenge.version !== userCredentialVersion(user) ||
      !matchesHash(body.challenge, challenge.hash)
    ) {
      return c.json({ error: "Invalid or expired challenge" }, 401);
    }
    challenge.attempts += 1;
    const attempted = await users.compareAndSetTotp(
      user,
      JSON.stringify(state),
    );
    if (!attempted)
      return c.json({ error: "Authentication changed; try again" }, 409);
    let valid = false;
    try {
      valid = consumeFactor(state, user.id, body.code.trim(), key!, now());
    } catch {
      return c.json({ error: "Two-factor authentication is unavailable" }, 503);
    }
    if (!valid) {
      return c.json({ error: "Invalid authentication code" }, 401);
    }
    delete state.challenge;
    const updated = await users.compareAndSetTotp(
      attempted,
      JSON.stringify(state),
    );
    if (!updated)
      return c.json({ error: "Authentication changed; sign in again" }, 409);
    limiter.resetAccount(hashCode(user.id));
    options.loginLimiter.resetAccount(
      Buffer.from(hashCode(user.email), "hex").toString("base64url"),
    );
    await setSession(c, updated);
    return c.json(toPublicUser(updated));
  });

  // Password reauthentication is required for every security-management operation.
  routes.use("/setup", requireAuth);
  routes.use("/enable", requireAuth);
  routes.use("/disable", requireAuth);
  routes.use("/recovery-codes", requireAuth);
  for (const path of ["/setup", "/enable", "/disable", "/recovery-codes"]) {
    routes.post(path, async (c) => {
      const user = c.get("currentUser");
      const rate = limiter.consume(
        options.resolveSourceIp(c),
        hashCode(user.id),
      );
      if (rate.limited) {
        c.header("Retry-After", String(rate.retryAfterSeconds));
        return c.json({ error: "Too many authentication attempts" }, 429);
      }
      const body = await c.req.json().catch(() => null);
      if (
        !body ||
        typeof body.password !== "string" ||
        body.password.length > 1024 ||
        (path !== "/setup" &&
          (typeof body.code !== "string" || body.code.length > 64))
      ) {
        return c.json(
          { error: "password and authentication code are required" },
          400,
        );
      }
      if (!(await verifyPassword(body.password, user.password_hash)))
        return c.json({ error: "Invalid current password" }, 401);
      const state = totpState(user);
      if (path === "/setup") {
        if (state.secret)
          return c.json(
            { error: "Two-factor authentication is already enabled" },
            409,
          );
        const enrollment = newEnrollment(user.email);
        state.pending = {
          secret: encryptTotp(enrollment.secret, user.id, key!),
          expiresAt: now() + 10 * 60_000,
          version: userCredentialVersion(user),
        };
        if (!(await users.compareAndSetTotp(user, JSON.stringify(state))))
          return c.json({ error: "Account changed; try again" }, 409);
        limiter.resetAccount(hashCode(user.id));
        return c.json(enrollment);
      }
      if (path === "/enable") {
        if (
          state.secret ||
          !state.pending ||
          state.pending.expiresAt <= now() ||
          state.pending.version !== userCredentialVersion(user)
        ) {
          return c.json({ error: "Start two-factor setup again" }, 409);
        }
        let step: number | null;
        try {
          step = verifyTotp(
            decryptTotp(state.pending.secret, user.id, key!),
            body.code.trim(),
            now(),
          );
        } catch {
          return c.json(
            { error: "Two-factor authentication is unavailable" },
            503,
          );
        }
        if (step === null)
          return c.json({ error: "Invalid authentication code" }, 401);
        const recovery = newRecoveryCodes();
        const updated = await users.compareAndSetTotp(
          user,
          JSON.stringify({
            secret: state.pending.secret,
            lastStep: step,
            recoveryHashes: recovery.hashes,
          }),
          true,
        );
        if (!updated)
          return c.json({ error: "Account changed; try again" }, 409);
        limiter.resetAccount(hashCode(user.id));
        await setSession(c, updated);
        return c.json({
          user: toPublicUser(updated),
          recovery_codes: recovery.codes,
        });
      }
      let valid = false;
      try {
        valid = consumeFactor(state, user.id, body.code.trim(), key!, now());
      } catch {
        return c.json(
          { error: "Two-factor authentication is unavailable" },
          503,
        );
      }
      if (!valid) return c.json({ error: "Invalid authentication code" }, 401);
      if (path === "/disable") {
        const updated = await users.compareAndSetTotp(user, "{}", true);
        if (!updated)
          return c.json({ error: "Account changed; try again" }, 409);
        limiter.resetAccount(hashCode(user.id));
        await setSession(c, updated);
        return c.json(toPublicUser(updated));
      }
      const recovery = newRecoveryCodes();
      state.recoveryHashes = recovery.hashes;
      delete state.challenge;
      delete state.pending;
      const updated = await users.compareAndSetTotp(
        user,
        JSON.stringify(state),
        true,
      );
      if (!updated) return c.json({ error: "Account changed; try again" }, 409);
      limiter.resetAccount(hashCode(user.id));
      await setSession(c, updated);
      return c.json({ recovery_codes: recovery.codes });
    });
  }
  return routes;
}

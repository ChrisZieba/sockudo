import { Hono, type Context } from "hono";
import { createHash, randomBytes } from "node:crypto";
import { isIP } from "node:net";
import {
  clearSessionCookie,
  createSession,
  sessionCookie,
} from "../auth/session.ts";
import { verifyPassword } from "../auth/password.ts";
import { createRequireAuth } from "../auth/middleware.ts";
import {
  createDefaultLoginRateLimiter,
  type LoginRateLimiter,
} from "../auth/login-rate-limit.ts";
import type { UsersRepository } from "../db/users-repository.ts";
import { toPublicUser } from "../types/user.ts";
import type { AppVariables } from "../types/hono.ts";
import { config } from "../config.ts";
import { totpState, hashCode, userCredentialVersion } from "../auth/totp.ts";
import { createTotpRoutes } from "./totp.ts";

interface AuthBindings {
  remoteAddress?: string;
}

interface AuthRouteOptions {
  limiter?: LoginRateLimiter;
  resolveSourceIp?: (c: Context) => string;
  totpLimiter?: LoginRateLimiter;
  totpKey?: Buffer | null;
  now?: () => number;
}

export function createAuthRoutes(
  usersRepo: UsersRepository,
  options: AuthRouteOptions = {},
) {
  const authRoutes = new Hono<{
    Bindings: AuthBindings;
    Variables: AppVariables;
  }>();
  const limiter = options.limiter ?? createDefaultLoginRateLimiter();
  const resolveSourceIp = options.resolveSourceIp ?? defaultSourceIp;
  const requireAuth = createRequireAuth(usersRepo);
  const now = options.now ?? Date.now;
  const totpKey =
    options.totpKey === undefined ? config.totpEncryptionKey : options.totpKey;
  authRoutes.use("*", async (c, next) => {
    c.header("Cache-Control", "no-store");
    await next();
  });

  authRoutes.post("/login", async (c) => {
    const body = await c.req.json().catch(() => null);
    if (
      !body ||
      typeof body.email !== "string" ||
      typeof body.password !== "string" ||
      body.email.length > 255 ||
      body.password.length > 1024
    ) {
      return c.json({ error: "email and password are required" }, 400);
    }
    const email = body.email.trim().toLowerCase();
    const password = body.password;
    const accountKey = accountFingerprint(email);

    const rate = limiter.consume(resolveSourceIp(c), accountKey);
    if (rate.limited) {
      c.header("Retry-After", String(rate.retryAfterSeconds));
      return c.json({ error: "Too many login attempts" }, 429);
    }

    const user = await usersRepo.findByEmail(email);
    if (!user || !user.active) {
      return c.json({ error: "Invalid credentials" }, 401);
    }

    const valid = await verifyPassword(password, user.password_hash);
    if (!valid) {
      return c.json({ error: "Invalid credentials" }, 401);
    }

    const state = totpState(user);
    if (state.secret) {
      if (!totpKey)
        return c.json(
          { error: "Two-factor authentication is unavailable" },
          503,
        );
      const challenge = `${user.id}.${randomBytes(32).toString("base64url")}`;
      state.challenge = {
        hash: hashCode(challenge),
        expiresAt: now() + 5 * 60_000,
        attempts: 0,
        version: userCredentialVersion(user),
      };
      if (!(await usersRepo.compareAndSetTotp(user, JSON.stringify(state)))) {
        return c.json({ error: "Authentication changed; sign in again" }, 409);
      }
      return c.json({ mfa_required: true, challenge });
    }

    const token = await createSession({
      id: user.id,
      email: user.email,
      name: user.name,
      role: user.role,
      passwordHash: user.password_hash,
      authVersion: user.auth_version,
    });
    limiter.resetAccount(accountKey);
    c.header("Set-Cookie", sessionCookie(token));
    return c.json(toPublicUser(user));
  });

  authRoutes.post("/logout", (c) => {
    c.header("Set-Cookie", clearSessionCookie());
    return c.json({ ok: true });
  });

  authRoutes.get("/me", requireAuth, (c) =>
    c.json(toPublicUser(c.get("currentUser"))),
  );

  authRoutes.route(
    "/totp",
    createTotpRoutes(usersRepo, {
      key: totpKey,
      now,
      limiter: options.totpLimiter ?? createDefaultLoginRateLimiter(),
      loginLimiter: limiter,
      resolveSourceIp,
    }),
  );

  authRoutes.use("/protected-check", requireAuth);
  authRoutes.get("/protected-check", (c) => c.json({ ok: true }));

  return authRoutes;
}

function accountFingerprint(email: string): string {
  return createHash("sha256").update(email, "utf8").digest("base64url");
}

function defaultSourceIp(c: Context): string {
  const direct = (c.env as { remoteAddress?: unknown } | undefined)
    ?.remoteAddress;
  if (typeof direct === "string" && direct.length <= 64 && isIP(direct)) {
    if (!config.trustProxy) return direct;
  }

  if (config.trustProxy) {
    const forwarded = c.req.header("x-forwarded-for");
    if (forwarded && forwarded.length <= 512) {
      const comma = forwarded.indexOf(",");
      const first = forwarded.slice(0, comma === -1 ? undefined : comma).trim();
      if (first.length <= 64 && isIP(first)) return first;
    }
  }

  return typeof direct === "string" && direct.length <= 64 ? direct : "unknown";
}

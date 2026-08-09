import type { AuthTokenData, AuthTokenRequest, AuthTokenResult, Options } from "./options";
import { decode as decodeBase64 } from "@stablelib/base64";
import { decode as decodeUtf8 } from "@stablelib/utf8";

export class TokenAuthError extends Error {
  readonly code?: number;

  constructor(message: string, code?: number) {
    super(message);
    this.name = "TokenAuthError";
    this.code = code;
    Object.setPrototypeOf(this, new.target.prototype);
  }
}

export class TokenExpiredError extends TokenAuthError {
  constructor(message = "Capability token expired") {
    super(message, 40142);
    this.name = "TokenExpiredError";
  }
}

export class TokenRevokedError extends TokenAuthError {
  constructor(message = "Capability token was revoked") {
    super(message, 40160);
    this.name = "TokenRevokedError";
  }
}

export interface NormalizedAuthToken {
  token: string;
  issuedAt?: number;
  expiresAt?: number;
}

export async function requestAuthToken(
  options: Pick<Options, "authCallback" | "authUrl">,
  request: AuthTokenRequest,
): Promise<NormalizedAuthToken> {
  let result: AuthTokenResult;
  if (options.authCallback) {
    result = await options.authCallback(request);
  } else if (options.authUrl) {
    const response = await fetch(options.authUrl, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ socket_id: request.socketId, reason: request.reason }),
    });
    if (!response.ok) {
      throw new TokenAuthError(`Capability-token request failed with status ${response.status}`);
    }
    result = (await response.json()) as AuthTokenResult;
  } else {
    throw new TokenAuthError("No capability-token provider is configured");
  }
  return normalizeAuthToken(result);
}

export function normalizeAuthToken(result: AuthTokenResult): NormalizedAuthToken {
  const data: AuthTokenData = typeof result === "string" ? { token: result } : result;
  if (!data || typeof data.token !== "string" || data.token.length === 0) {
    throw new TokenAuthError("Capability-token provider returned an empty token");
  }

  const claims = decodeJwtClaims(data.token);
  let issuedAt = firstFinite(data.issuedAtMs, data.issuedAt, data.iat, claims?.iat);
  let expiresAt = firstFinite(data.expiresAtMs, data.expiresAt, data.exp, claims?.exp);
  if (data.issuedAtMs !== undefined) issuedAt = data.issuedAtMs / 1000;
  if (data.expiresAtMs !== undefined) expiresAt = data.expiresAtMs / 1000;
  if (data.expiresIn !== undefined && Number.isFinite(data.expiresIn)) {
    issuedAt ??= Date.now() / 1000;
    expiresAt = issuedAt + data.expiresIn;
  }
  return { token: data.token, issuedAt, expiresAt };
}

export function authTokenRefreshDelay(
  token: Pick<NormalizedAuthToken, "issuedAt" | "expiresAt">,
  nowSeconds = Date.now() / 1000,
): number | undefined {
  if (token.expiresAt === undefined) return undefined;
  const issuedAt = token.issuedAt ?? nowSeconds;
  const lifetime = token.expiresAt - issuedAt;
  if (lifetime <= 0) return 1000;
  const refreshAt = issuedAt + lifetime * 0.8;
  return Math.max(1000, Math.round((refreshAt - nowSeconds) * 1000));
}

function firstFinite(...values: unknown[]): number | undefined {
  for (const value of values) {
    if (typeof value === "number" && Number.isFinite(value)) return value;
  }
  return undefined;
}

function decodeJwtClaims(token: string): Record<string, unknown> | undefined {
  const payload = token.split(".")[1];
  if (!payload) return undefined;
  try {
    const normalized = payload.replace(/-/g, "+").replace(/_/g, "/");
    const padded = normalized.padEnd(Math.ceil(normalized.length / 4) * 4, "=");
    const json = decodeUtf8(decodeBase64(padded));
    const value = JSON.parse(json);
    return value && typeof value === "object" ? value : undefined;
  } catch {
    return undefined;
  }
}

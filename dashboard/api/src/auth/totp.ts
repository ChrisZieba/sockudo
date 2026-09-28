import {
  createCipheriv,
  createDecipheriv,
  createHash,
  randomBytes,
  timingSafeEqual,
} from "node:crypto";
import { Secret, TOTP } from "otpauth";
import type { DashboardUser } from "../types/user.ts";
import { credentialVersion } from "./session.ts";

export interface TotpState {
  secret?: string;
  pending?: { secret: string; expiresAt: number; version: string };
  lastStep?: number;
  recoveryHashes?: string[];
  challenge?: {
    hash: string;
    expiresAt: number;
    attempts: number;
    version: string;
  };
}

export function totpState(user: DashboardUser): TotpState {
  return JSON.parse(user.totp_state ?? "{}");
}

export function userCredentialVersion(user: DashboardUser): string {
  return credentialVersion(user.password_hash, user.auth_version);
}

export function hashCode(value: string): string {
  return createHash("sha256").update(value, "utf8").digest("hex");
}

export function matchesHash(value: string, hash: string): boolean {
  const actual = Buffer.from(hashCode(value));
  const expected = Buffer.from(hash);
  return actual.length === expected.length && timingSafeEqual(actual, expected);
}

export function encryptTotp(
  secret: string,
  userId: string,
  key: Buffer,
): string {
  const iv = randomBytes(12);
  const cipher = createCipheriv("aes-256-gcm", key, iv);
  cipher.setAAD(Buffer.from(`sockudo-dashboard-totp-v1:${userId}`));
  const ciphertext = Buffer.concat([
    cipher.update(secret, "utf8"),
    cipher.final(),
  ]);
  return [
    "v1",
    iv.toString("base64url"),
    cipher.getAuthTag().toString("base64url"),
    ciphertext.toString("base64url"),
  ].join(".");
}

export function decryptTotp(
  value: string,
  userId: string,
  key: Buffer,
): string {
  const [version, iv, tag, ciphertext] = value.split(".");
  if (version !== "v1" || !iv || !tag || !ciphertext)
    throw new Error("Invalid TOTP secret format");
  const decipher = createDecipheriv(
    "aes-256-gcm",
    key,
    Buffer.from(iv, "base64url"),
  );
  decipher.setAAD(Buffer.from(`sockudo-dashboard-totp-v1:${userId}`));
  decipher.setAuthTag(Buffer.from(tag, "base64url"));
  return Buffer.concat([
    decipher.update(Buffer.from(ciphertext, "base64url")),
    decipher.final(),
  ]).toString("utf8");
}

export function newEnrollment(email: string): {
  secret: string;
  otpauth_uri: string;
} {
  const secret = new Secret({ size: 20 }).base32;
  return { secret, otpauth_uri: authenticator(secret, email).toString() };
}

function authenticator(secret: string, label = ""): TOTP {
  return new TOTP({
    issuer: "Sockudo Dashboard",
    label,
    algorithm: "SHA1",
    digits: 6,
    period: 30,
    secret: Secret.fromBase32(secret),
  });
}

export function verifyTotp(
  secret: string,
  code: string,
  now: number,
  lastStep = -1,
): number | null {
  if (!/^\d{6}$/.test(code)) return null;
  const delta = authenticator(secret).validate({
    token: code,
    timestamp: now,
    window: 1,
  });
  if (delta === null) return null;
  const step = Math.floor(now / 30_000) + delta;
  return step > lastStep ? step : null;
}

export function newRecoveryCodes(): { codes: string[]; hashes: string[] } {
  const codes = Array.from({ length: 10 }, () =>
    randomBytes(16).toString("hex"),
  );
  return { codes, hashes: codes.map(hashCode) };
}

// Mutate a snapshot only; callers must persist it with a compare-and-set before granting access.
export function consumeFactor(
  state: TotpState,
  userId: string,
  code: string,
  key: Buffer,
  now: number,
): boolean {
  if (!state.secret) return false;
  if (/^\d{6}$/.test(code)) {
    const step = verifyTotp(
      decryptTotp(state.secret, userId, key),
      code,
      now,
      state.lastStep,
    );
    if (step === null) return false;
    state.lastStep = step;
    return true;
  }
  const index =
    state.recoveryHashes?.findIndex((hash) => matchesHash(code, hash)) ?? -1;
  if (index < 0) return false;
  state.recoveryHashes!.splice(index, 1);
  return true;
}

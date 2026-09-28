import { describe, expect, test } from "bun:test";
import { TOTP, Secret } from "otpauth";
import { resolveTotpEncryptionKey } from "./configuration.ts";
import {
  consumeFactor,
  decryptTotp,
  encryptTotp,
  newEnrollment,
  newRecoveryCodes,
  verifyTotp,
} from "./totp.ts";

describe("TOTP cryptography", () => {
  const key = Buffer.alloc(32, 17);
  test("uses the RFC 6238 SHA-1 vector and rejects replay/malformed codes", () => {
    const secret = Secret.fromUTF8("12345678901234567890").base32;
    expect(verifyTotp(secret, "287082", 59_000)).toBe(1);
    expect(verifyTotp(secret, "287082", 59_000, 1)).toBeNull();
    expect(verifyTotp(secret, "287082", 150_000)).toBeNull();
    expect(verifyTotp(secret, "287082x", 59_000)).toBeNull();
  });

  test("encryption is randomized, authenticated and bound to one user", () => {
    const encrypted = encryptTotp("SECRET", "user1", key);
    expect(encrypted).not.toContain("SECRET");
    expect(encryptTotp("SECRET", "user1", key)).not.toBe(encrypted);
    expect(decryptTotp(encrypted, "user1", key)).toBe("SECRET");
    expect(() => decryptTotp(encrypted, "user2", key)).toThrow();
    expect(() =>
      decryptTotp(encrypted, "user1", Buffer.alloc(32, 18)),
    ).toThrow();
    expect(() =>
      decryptTotp(encrypted.replace("v1", "v2"), "user1", key),
    ).toThrow();
  });

  test("setup URIs are interoperable and recovery codes are one-use hashes", () => {
    const setup = newEnrollment("operator@example.com");
    expect(setup.otpauth_uri).toStartWith("otpauth://totp/");
    const code = new TOTP({ secret: Secret.fromBase32(setup.secret) }).generate(
      { timestamp: 60_000 },
    );
    expect(verifyTotp(setup.secret, code, 60_000)).toBe(2);
    const recovery = newRecoveryCodes();
    const state = {
      secret: encryptTotp(setup.secret, "user", key),
      recoveryHashes: recovery.hashes,
    };
    expect(recovery.codes).toHaveLength(10);
    expect(JSON.stringify(state)).not.toContain(recovery.codes[0]!);
    expect(consumeFactor(state, "user", recovery.codes[0]!, key, 60_000)).toBe(
      true,
    );
    expect(consumeFactor(state, "user", recovery.codes[0]!, key, 60_000)).toBe(
      false,
    );
  });

  test("a separate canonical 256-bit encryption key is optional but validated", () => {
    expect(resolveTotpEncryptionKey({})).toBeNull();
    expect(
      resolveTotpEncryptionKey({
        DASHBOARD_TOTP_ENCRYPTION_KEY: key.toString("base64"),
      }),
    ).toEqual(key);
    for (const value of [
      "secret",
      Buffer.alloc(31).toString("base64"),
      `${key.toString("base64")}garbage`,
    ]) {
      expect(() =>
        resolveTotpEncryptionKey({ DASHBOARD_TOTP_ENCRYPTION_KEY: value }),
      ).toThrow();
    }
  });
});

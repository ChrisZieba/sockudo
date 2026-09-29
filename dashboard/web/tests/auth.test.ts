import { afterEach, beforeEach, expect, test } from "bun:test";
import { createPinia, setActivePinia } from "pinia";
import { useAuthStore } from "../src/stores/auth";

const originalFetch = globalThis.fetch;
const user = {
  id: "operator-1", email: "operator@example.com", name: "Operator",
  role: "operator", active: true, totp_enabled: true,
  created_at: "", updated_at: "",
};

beforeEach(() => { setActivePinia(createPinia()); });
afterEach(() => { globalThis.fetch = originalFetch; });

test("a password-only login preserves the existing session flow", async () => {
  globalThis.fetch = (async () => Response.json(user)) as typeof fetch;
  const auth = useAuthStore();
  await auth.login(user.email, "password");
  expect(auth.user?.id).toBe(user.id);
  expect(auth.challenge).toBeNull();
  expect(auth.loading).toBe(false);
});

test("an MFA challenge cannot authenticate until verification succeeds", async () => {
  const calls: Array<{ url: string; body: unknown }> = [];
  globalThis.fetch = (async (url, init) => {
    calls.push({ url: String(url), body: JSON.parse(String(init?.body)) });
    return Response.json(calls.length === 1 ? { mfa_required: true, challenge: "challenge-token" } : user);
  }) as typeof fetch;
  const auth = useAuthStore();
  await auth.login(user.email, "password");
  expect(auth.user).toBeNull();
  expect(auth.email).toBeNull();
  expect(auth.challenge).toBe("challenge-token");
  await auth.verifyTotp(" 123456 ");
  expect(calls[1]).toEqual({ url: "/api/v1/auth/totp/verify", body: { challenge: "challenge-token", code: "123456" } });
  expect(auth.user?.id).toBe(user.id);
  expect(auth.challenge).toBeNull();
});

test("rejected verification keeps the account unauthenticated and can be retried", async () => {
  globalThis.fetch = (async () => Response.json({ error: "Invalid verification code" }, { status: 401 })) as typeof fetch;
  const auth = useAuthStore();
  auth.challenge = "challenge-token";
  await expect(auth.verifyTotp("000000")).rejects.toThrow("Invalid verification code");
  expect(auth.user).toBeNull();
  expect(auth.challenge).toBe("challenge-token");
  expect(auth.error).toBe("Invalid verification code");
  expect(auth.loading).toBe(false);
});

test("clearing the local session removes pending authentication state", () => {
  const auth = useAuthStore();
  auth.challenge = "challenge-token";
  auth.error = "Invalid verification code";
  auth.clearSession();
  expect(auth.user).toBeNull();
  expect(auth.challenge).toBeNull();
  expect(auth.error).toBeNull();
});

import { afterEach, describe, expect, test } from "bun:test";
import type { AppRecord } from "../types/app.ts";
import {
  createSignedSockudoPushRequest,
  requestSockudoPush,
} from "./sockudo-push.ts";

const originalFetch = globalThis.fetch;

const app: AppRecord = {
  id: "app-1",
  key: "public-key",
  secret: "top-secret",
  enabled: true,
  policy: {
    limits: {
      max_connections: 100,
      max_client_events_per_second: 10,
    },
    features: { enable_client_messages: false },
    channels: {},
  },
};

afterEach(() => {
  globalThis.fetch = originalFetch;
});

describe("Sockudo push request signing", () => {
  test("signs GET query parameters using the Pusher canonical form", () => {
    const signed = createSignedSockudoPushRequest(
      app,
      {
        method: "GET",
        path: "/apps/app-1/push/deviceRegistrations",
        query: { cursor: "next page", limit: 50 },
      },
      1_785_312_000,
      "http://sockudo.internal:6001",
    );

    const url = new URL(signed.url);
    expect(url.pathname).toBe("/apps/app-1/push/deviceRegistrations");
    expect(url.searchParams.get("cursor")).toBe("next page");
    expect(url.searchParams.get("limit")).toBe("50");
    expect(url.searchParams.get("auth_key")).toBe("public-key");
    expect(url.searchParams.get("auth_timestamp")).toBe("1785312000");
    expect(url.searchParams.get("auth_version")).toBe("1.0");
    expect(url.searchParams.get("body_md5")).toBeNull();
    expect(url.searchParams.get("auth_signature")).toBe(
      "fef85f045e082ed950e634bcab8da70f19fe7aba2342bd48c3c0d99f32e11f03",
    );
    expect(signed.url).not.toContain(app.secret);
    expect(signed.body).toBeUndefined();
  });

  test("hashes the exact POST JSON body before signing", () => {
    const body = {
      recipients: [{ type: "channel", channel: "news" }],
      payload: { title: "Hello", body: "World" },
    };
    const signed = createSignedSockudoPushRequest(
      app,
      {
        method: "POST",
        path: "/apps/app-1/push/publish",
        body,
      },
      1_785_312_000,
      "http://sockudo.internal:6001",
    );

    const url = new URL(signed.url);
    expect(signed.body).toBe(JSON.stringify(body));
    expect(url.searchParams.get("body_md5")).toBe(
      "7ecae25d7d55afefdc5253911cb128f3",
    );
    expect(url.searchParams.get("auth_signature")).toBe(
      "0445b34247211230d30e92e0b4a8dec976d079a367308702d76b413ad3a21cec",
    );
    expect(signed.headers.get("content-type")).toBe("application/json");
    expect(signed.headers.get("x-sockudo-push-capability")).toBe(
      "push-admin",
    );
  });

  test("rejects caller-supplied authentication parameters", () => {
    expect(() =>
      createSignedSockudoPushRequest(
        app,
        {
          method: "GET",
          path: "/apps/app-1/push/credentials",
          query: { auth_signature: "attacker-controlled" },
        },
        1_785_312_000,
      ),
    ).toThrow("Reserved Sockudo authentication query key");
  });

  test("preserves camelCase query keys while signing their lowercase form", () => {
    const signed = createSignedSockudoPushRequest(
      app,
      {
        method: "GET",
        path: "/apps/app-1/push/deadLetters",
        query: { deviceId: "device-1", sinceMs: 100 },
      },
      1_785_312_000,
      "http://sockudo.internal:6001",
    );
    const url = new URL(signed.url);

    expect(url.searchParams.get("deviceId")).toBe("device-1");
    expect(url.searchParams.get("sinceMs")).toBe("100");
    expect(url.searchParams.has("deviceid")).toBe(false);
    expect(url.searchParams.has("sincems")).toBe(false);
  });

  test("redacts credential and signing material from upstream JSON", async () => {
    globalThis.fetch = (async () =>
      Response.json({
        credential_id: "fcm",
        secret: app.secret,
        nested: {
          privateKey: "private-material",
          error: `failed at ?auth_signature=signed&body_md5=digest`,
        },
      })) as unknown as typeof fetch;

    const result = await requestSockudoPush(app, {
      method: "GET",
      path: "/apps/app-1/push/credentials",
    });

    expect(result.body).toEqual({
      credential_id: "fcm",
      secret: "[REDACTED]",
      nested: {
        privateKey: "[REDACTED]",
        error: "[REDACTED]",
      },
    });
    expect(JSON.stringify(result.body)).not.toContain(app.secret);
    expect(JSON.stringify(result.body)).not.toContain("private-material");
    expect(JSON.stringify(result.body)).not.toContain("auth_signature");
  });

  test("does not expose a signed URL when the upstream request fails", async () => {
    globalThis.fetch = (async (input: RequestInfo | URL) => {
      throw new Error(`network failure for ${String(input)}`);
    }) as unknown as typeof fetch;

    await expect(
      requestSockudoPush(app, {
        method: "GET",
        path: "/apps/app-1/push/credentials",
      }),
    ).rejects.toThrow("Sockudo push API is unavailable");
  });
});

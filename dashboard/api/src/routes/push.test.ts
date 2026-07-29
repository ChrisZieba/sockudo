import { describe, expect, test } from "bun:test";
import { Hono } from "hono";
import { createSession, sessionCookie } from "../auth/session.ts";
import type { UsersRepository } from "../db/users-repository.ts";
import type { AppsRepository } from "../db/types.ts";
import type {
  SockudoPushRequestOptions,
  SockudoPushRequester,
} from "../services/sockudo-push.ts";
import type { AppRecord } from "../types/app.ts";
import type { DashboardUser, UserRole } from "../types/user.ts";
import { createPushRoutes } from "./push.ts";

const managedApp: AppRecord = {
  id: "app-1",
  key: "public-key",
  secret: "never-return-this-app-secret",
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

function dashboardUser(role: UserRole): DashboardUser {
  return {
    id: `${role}-1`,
    email: `${role}@example.com`,
    password_hash: `$argon2id$${role}-password-hash`,
    name: role,
    role,
    active: true,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
  };
}

function usersRepository(user: DashboardUser): UsersRepository {
  return {
    findById: async (id: string) => (id === user.id ? user : null),
  } as unknown as UsersRepository;
}

function appsRepository(app: AppRecord | null = managedApp): AppsRepository {
  return {
    findById: async (id: string) => (app?.id === id ? app : null),
  } as unknown as AppsRepository;
}

async function cookieFor(user: DashboardUser): Promise<string> {
  const token = await createSession({
    id: user.id,
    email: user.email,
    name: user.name,
    role: user.role,
    passwordHash: user.password_hash,
  });
  return sessionCookie(token).split(";")[0]!;
}

interface CapturedCall {
  app: AppRecord;
  options: SockudoPushRequestOptions;
}

function requesterReturning(
  calls: CapturedCall[],
  body: unknown = { ok: true },
  status = 200,
): SockudoPushRequester {
  return async <T>(app: AppRecord, options: SockudoPushRequestOptions) => {
    calls.push({ app, options });
    return { status, body: body as T };
  };
}

function testApp(
  role: UserRole,
  requester: SockudoPushRequester,
  repo: AppsRepository = appsRepository(),
) {
  const user = dashboardUser(role);
  const api = new Hono();
  api.route(
    "/api/v1/apps",
    createPushRoutes(repo, usersRepository(user), requester),
  );
  return { api, user };
}

describe("dashboard push routes", () => {
  test("requires a current dashboard session for reads", async () => {
    const calls: CapturedCall[] = [];
    const { api } = testApp("operator", requesterReturning(calls));

    const response = await api.request(
      "/api/v1/apps/app-1/push/credentials",
    );

    expect(response.status).toBe(401);
    expect(calls).toHaveLength(0);
  });

  test("allows operators to read and only forwards allowlisted query keys", async () => {
    const calls: CapturedCall[] = [];
    const responseBody = {
      items: [],
      next_cursor: null,
      has_more: false,
    };
    const { api, user } = testApp(
      "operator",
      requesterReturning(calls, responseBody),
    );
    const cookie = await cookieFor(user);

    const response = await api.request(
      "/api/v1/apps/app-1/push/credentials?limit=25&cursor=opaque&auth_signature=attacker",
      { headers: { cookie } },
    );

    expect(response.status).toBe(200);
    expect(await response.json()).toEqual(responseBody);
    expect(response.headers.get("cache-control")).toBe("no-store");
    expect(calls).toHaveLength(1);
    expect(calls[0]!.app.secret).toBe(managedApp.secret);
    expect(calls[0]!.options).toEqual({
      method: "GET",
      path: "/apps/app-1/push/credentials",
      query: { limit: "25", cursor: "opaque" },
    });
    expect(JSON.stringify(await responseBody)).not.toContain(managedApp.secret);
  });

  test("prevents operators from publishing or uploading credentials", async () => {
    const calls: CapturedCall[] = [];
    const { api, user } = testApp("operator", requesterReturning(calls));
    const cookie = await cookieFor(user);

    const publish = await api.request("/api/v1/apps/app-1/push/publish", {
      method: "POST",
      headers: { cookie, "content-type": "application/json" },
      body: JSON.stringify({
        recipients: [{ type: "channel", channel: "news" }],
        payload: { title: "hello" },
      }),
    });
    const upload = await api.request(
      "/api/v1/apps/app-1/push/credentials/fcm",
      {
        method: "POST",
        headers: { cookie, "content-type": "application/json" },
        body: JSON.stringify({ serviceAccountJson: { project_id: "project" } }),
      },
    );

    expect(publish.status).toBe(403);
    expect(upload.status).toBe(403);
    expect(calls).toHaveLength(0);
  });

  test("maps the admin management routes to fixed native push paths", async () => {
    const calls: CapturedCall[] = [];
    const { api, user } = testApp("admin", requesterReturning(calls));
    const cookie = await cookieFor(user);
    const headers = { cookie, "content-type": "application/json" };

    expect(
      (
        await api.request("/api/v1/apps/app-1/push/credentials/apns", {
          method: "POST",
          headers,
          body: JSON.stringify({
            credentialId: "primary",
            teamId: "team",
            keyId: "key",
            privateKey: "private",
          }),
        })
      ).status,
    ).toBe(200);
    expect(
      (
        await api.request("/api/v1/apps/app-1/push/devices/device%2Fone", {
          method: "DELETE",
          headers: { cookie },
        })
      ).status,
    ).toBe(200);
    expect(
      (
        await api.request("/api/v1/apps/app-1/push/publish", {
          method: "POST",
          headers,
          body: JSON.stringify({
            recipients: [{ type: "channel", channel: "news" }],
            payload: { title: "Hello" },
          }),
        })
      ).status,
    ).toBe(200);
    expect(
      (
        await api.request(
          "/api/v1/apps/app-1/push/dead-letters/dead%2Fone/replay",
          {
            method: "POST",
            headers,
          },
        )
      ).status,
    ).toBe(200);

    expect(
      calls.map(({ options }) => ({
        method: options.method,
        path: options.path,
      })),
    ).toEqual([
      {
        method: "POST",
        path: "/apps/app-1/push/credentials/apns",
      },
      {
        method: "DELETE",
        path: "/apps/app-1/push/deviceRegistrations/device%2Fone",
      },
      {
        method: "POST",
        path: "/apps/app-1/push/publish",
      },
      {
        method: "POST",
        path: "/apps/app-1/push/deadLetters/dead%2Fone/replay",
      },
    ]);
    expect(calls[3]!.options.body).toEqual({});
  });

  test("maps subscription, status, and dead-letter reads", async () => {
    const calls: CapturedCall[] = [];
    const { api, user } = testApp("operator", requesterReturning(calls));
    const cookie = await cookieFor(user);

    await api.request(
      "/api/v1/apps/app-1/push/subscriptions?channel=private-news&deviceId=device-1&limit=10&ignored=true",
      { headers: { cookie } },
    );
    await api.request(
      "/api/v1/apps/app-1/push/publish/publish%2Fone/status",
      { headers: { cookie } },
    );
    await api.request(
      "/api/v1/apps/app-1/push/dead-letters?provider=apns&sinceMs=100&untilMs=200&limit=5",
      { headers: { cookie } },
    );

    expect(calls.map(({ options }) => options)).toEqual([
      {
        method: "GET",
        path: "/apps/app-1/push/channelSubscriptions",
        query: {
          channel: "private-news",
          deviceId: "device-1",
          limit: "10",
        },
      },
      {
        method: "GET",
        path: "/apps/app-1/push/publish/publish%2Fone/status",
      },
      {
        method: "GET",
        path: "/apps/app-1/push/deadLetters",
        query: {
          provider: "apns",
          sinceMs: "100",
          untilMs: "200",
          limit: "5",
        },
      },
    ]);
  });

  test("returns safe local errors without exposing app or signed secrets", async () => {
    const requester: SockudoPushRequester = async () => {
      throw new Error(
        `failed URL ?auth_signature=signed with ${managedApp.secret}`,
      );
    };
    const { api, user } = testApp("operator", requester);
    const cookie = await cookieFor(user);

    const response = await api.request(
      "/api/v1/apps/app-1/push/credentials",
      { headers: { cookie } },
    );
    const text = await response.text();

    expect(response.status).toBe(502);
    expect(text).toContain("Sockudo push request failed");
    expect(text).not.toContain("auth_signature");
    expect(text).not.toContain(managedApp.secret);
  });

  test("rejects unsupported providers and missing apps before proxying", async () => {
    const calls: CapturedCall[] = [];
    const requester = requesterReturning(calls);
    const { api, user } = testApp("admin", requester);
    const cookie = await cookieFor(user);

    const provider = await api.request(
      "/api/v1/apps/app-1/push/credentials/unknown",
      {
        method: "POST",
        headers: { cookie, "content-type": "application/json" },
        body: "{}",
      },
    );

    const missingApi = testApp(
      "admin",
      requester,
      appsRepository(null),
    ).api;
    const missing = await missingApi.request(
      "/api/v1/apps/app-1/push/devices",
      { headers: { cookie } },
    );

    expect(provider.status).toBe(400);
    expect(missing.status).toBe(404);
    expect(calls).toHaveLength(0);
  });
});

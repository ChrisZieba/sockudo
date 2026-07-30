import { Hono } from "hono";
import {
  createRequireAdmin,
  createRequireAuth,
} from "../auth/middleware.ts";
import type { UsersRepository } from "../db/users-repository.ts";
import type { AppsRepository } from "../db/types.ts";
import {
  requestSockudoPush,
  SockudoPushTransportError,
  type SockudoPushQuery,
  type SockudoPushRequester,
  type SockudoPushResponse,
} from "../services/sockudo-push.ts";
import type { AppRecord } from "../types/app.ts";
import type { AppVariables } from "../types/hono.ts";
import type {
  PushCredentialRouteProvider,
  PushPublishInput,
} from "../types/push.ts";

const MAX_JSON_BODY_BYTES = 1_048_576;
const CREDENTIAL_PROVIDERS = new Set<PushCredentialRouteProvider>([
  "fcm",
  "apns",
  "webpush",
  "hms",
  "wns",
]);

interface JsonObjectResult {
  value?: Record<string, unknown>;
  error?: string;
  status?: 400 | 413;
}

async function readJsonObject(request: Request): Promise<JsonObjectResult> {
  const declaredLength = Number(request.headers.get("content-length"));
  if (
    Number.isFinite(declaredLength) &&
    declaredLength > MAX_JSON_BODY_BYTES
  ) {
    return { error: "Request body is too large", status: 413 };
  }

  const text = await request.text();
  if (new TextEncoder().encode(text).byteLength > MAX_JSON_BODY_BYTES) {
    return { error: "Request body is too large", status: 413 };
  }
  if (!text) return { error: "JSON object body is required", status: 400 };

  try {
    const value: unknown = JSON.parse(text);
    if (!value || typeof value !== "object" || Array.isArray(value)) {
      return { error: "JSON object body is required", status: 400 };
    }
    return { value: value as Record<string, unknown> };
  } catch {
    return { error: "Request body must be valid JSON", status: 400 };
  }
}

function hasNonEmptyString(
  value: Record<string, unknown>,
  key: string,
): boolean {
  return typeof value[key] === "string" && value[key].trim().length > 0;
}

function credentialValidationError(
  provider: PushCredentialRouteProvider,
  value: Record<string, unknown>,
): string | null {
  if (
    value.credentialId !== undefined &&
    !hasNonEmptyString(value, "credentialId")
  ) {
    return "credentialId must be a non-empty string";
  }
  if (
    value.version !== undefined &&
    (!Number.isSafeInteger(value.version) || Number(value.version) < 1)
  ) {
    return "version must be a positive integer";
  }

  switch (provider) {
    case "fcm":
      return value.serviceAccountJson &&
        typeof value.serviceAccountJson === "object" &&
        !Array.isArray(value.serviceAccountJson)
        ? null
        : "serviceAccountJson must be an object";
    case "apns": {
      const hasCertificate =
        hasNonEmptyString(value, "p12") || hasNonEmptyString(value, "pem");
      const hasToken =
        hasNonEmptyString(value, "teamId") &&
        hasNonEmptyString(value, "keyId") &&
        hasNonEmptyString(value, "privateKey");
      return hasCertificate || hasToken
        ? null
        : "Provide p12, pem, or teamId/keyId/privateKey APNs credentials";
    }
    case "webpush":
      return hasNonEmptyString(value, "publicKey") &&
        hasNonEmptyString(value, "privateKey")
        ? null
        : "publicKey and privateKey are required";
    case "hms":
      return hasNonEmptyString(value, "hmsAppId") &&
        hasNonEmptyString(value, "clientSecret")
        ? null
        : "hmsAppId and clientSecret are required";
    case "wns":
      return hasNonEmptyString(value, "packageSid") &&
        hasNonEmptyString(value, "clientSecret")
        ? null
        : "packageSid and clientSecret are required";
  }
}

function queryFrom(
  request: { query(name: string): string | undefined },
  keys: readonly string[],
): SockudoPushQuery {
  const query: SockudoPushQuery = {};
  for (const key of keys) {
    const value = request.query(key);
    if (value !== undefined) query[key] = value;
  }
  return query;
}

function nativePath(app: AppRecord, suffix: string): string {
  return `/apps/${app.id}/push/${suffix}`;
}

function proxyResponse(result: SockudoPushResponse): Response {
  const headers = new Headers({ "cache-control": "no-store" });
  if (result.body !== null) {
    headers.set("content-type", "application/json; charset=UTF-8");
  }
  if (result.retryAfter) headers.set("retry-after", result.retryAfter);
  if (result.forcedAsync) {
    headers.set("x-sockudo-push-forced-async", result.forcedAsync);
  }
  return new Response(
    result.body === null ? null : JSON.stringify(result.body),
    { status: result.status, headers },
  );
}

async function withApp(
  repo: AppsRepository,
  appId: string,
  run: (app: AppRecord) => Promise<Response>,
): Promise<Response> {
  const app = await repo.findById(appId);
  if (!app) {
    return Response.json({ error: "App not found" }, { status: 404 });
  }
  if (!/^[A-Za-z0-9_-]{1,128}$/.test(app.id)) {
    return Response.json(
      { error: "App id is not valid for the Sockudo push API" },
      { status: 400 },
    );
  }
  try {
    return await run(app);
  } catch (error) {
    if (
      error instanceof SockudoPushTransportError ||
      error instanceof TypeError
    ) {
      return Response.json(
        { error: "Sockudo push API is unavailable" },
        { status: 502 },
      );
    }
    return Response.json(
      { error: "Sockudo push request failed" },
      { status: 502 },
    );
  }
}

export function createPushRoutes(
  repo: AppsRepository,
  usersRepo: UsersRepository,
  requester: SockudoPushRequester = requestSockudoPush,
) {
  const push = new Hono<{ Variables: AppVariables }>();
  const requireAuth = createRequireAuth(usersRepo);
  const requireAdmin = createRequireAdmin(usersRepo);

  push.use("*", requireAuth);

  push.get("/:appId/push/credentials", async (c) =>
    withApp(repo, c.req.param("appId"), async (app) =>
      proxyResponse(
        await requester(app, {
          method: "GET",
          path: nativePath(app, "credentials"),
          query: queryFrom(c.req, ["limit", "cursor"]),
        }),
      ),
    ),
  );

  push.post(
    "/:appId/push/credentials/:provider",
    requireAdmin,
    async (c) => {
      const provider = c.req.param("provider");
      if (!CREDENTIAL_PROVIDERS.has(provider as PushCredentialRouteProvider)) {
        return c.json({ error: "Unsupported push credential provider" }, 400);
      }
      const parsed = await readJsonObject(c.req.raw);
      if (!parsed.value) {
        return c.json(
          { error: parsed.error ?? "Invalid request body" },
          parsed.status ?? 400,
        );
      }
      const validationError = credentialValidationError(
        provider as PushCredentialRouteProvider,
        parsed.value,
      );
      if (validationError) return c.json({ error: validationError }, 400);

      return withApp(repo, c.req.param("appId"), async (app) =>
        proxyResponse(
          await requester(app, {
            method: "POST",
            path: nativePath(app, `credentials/${provider}`),
            body: parsed.value,
          }),
        ),
      );
    },
  );

  push.get("/:appId/push/devices", async (c) =>
    withApp(repo, c.req.param("appId"), async (app) =>
      proxyResponse(
        await requester(app, {
          method: "GET",
          path: nativePath(app, "deviceRegistrations"),
          query: queryFrom(c.req, ["limit", "cursor"]),
        }),
      ),
    ),
  );

  push.delete(
    "/:appId/push/devices/:deviceId",
    requireAdmin,
    async (c) =>
      withApp(repo, c.req.param("appId"), async (app) =>
        proxyResponse(
          await requester(app, {
            method: "DELETE",
            path: nativePath(
              app,
              `deviceRegistrations/${encodeURIComponent(c.req.param("deviceId"))}`,
            ),
          }),
        ),
      ),
  );

  push.get("/:appId/push/subscriptions", async (c) =>
    withApp(repo, c.req.param("appId"), async (app) =>
      proxyResponse(
        await requester(app, {
          method: "GET",
          path: nativePath(app, "channelSubscriptions"),
          query: queryFrom(c.req, [
            "channel",
            "deviceId",
            "limit",
            "cursor",
          ]),
        }),
      ),
    ),
  );

  push.post("/:appId/push/publish", requireAdmin, async (c) => {
    const parsed = await readJsonObject(c.req.raw);
    if (!parsed.value) {
      return c.json(
        { error: parsed.error ?? "Invalid request body" },
        parsed.status ?? 400,
      );
    }
    if (
      !Array.isArray(parsed.value.recipients) ||
      parsed.value.recipients.length === 0 ||
      !parsed.value.payload ||
      typeof parsed.value.payload !== "object" ||
      Array.isArray(parsed.value.payload)
    ) {
      return c.json(
        { error: "recipients and a payload object are required" },
        400,
      );
    }

    return withApp(repo, c.req.param("appId"), async (app) =>
      proxyResponse(
        await requester(app, {
          method: "POST",
          path: nativePath(app, "publish"),
          body: parsed.value as unknown as PushPublishInput,
        }),
      ),
    );
  });

  push.get("/:appId/push/publish/:publishId/status", async (c) =>
    withApp(repo, c.req.param("appId"), async (app) =>
      proxyResponse(
        await requester(app, {
          method: "GET",
          path: nativePath(
            app,
            `publish/${encodeURIComponent(c.req.param("publishId"))}/status`,
          ),
        }),
      ),
    ),
  );

  push.get("/:appId/push/dead-letters", async (c) =>
    withApp(repo, c.req.param("appId"), async (app) =>
      proxyResponse(
        await requester(app, {
          method: "GET",
          path: nativePath(app, "deadLetters"),
          query: queryFrom(c.req, [
            "limit",
            "cursor",
            "provider",
            "sinceMs",
            "untilMs",
          ]),
        }),
      ),
    ),
  );

  push.post(
    "/:appId/push/dead-letters/:deadLetterId/replay",
    requireAdmin,
    async (c) =>
      withApp(repo, c.req.param("appId"), async (app) =>
        proxyResponse(
          await requester(app, {
            method: "POST",
            path: nativePath(
              app,
              `deadLetters/${encodeURIComponent(
                c.req.param("deadLetterId"),
              )}/replay`,
            ),
            body: {},
          }),
        ),
      ),
  );

  return push;
}

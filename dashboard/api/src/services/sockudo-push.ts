import { createHash, createHmac } from "node:crypto";
import { config } from "../config.ts";
import type { AppRecord } from "../types/app.ts";

const RESERVED_QUERY_KEYS = new Set([
  "auth_key",
  "auth_signature",
  "auth_timestamp",
  "auth_version",
  "body_md5",
]);

const APP_ID_PATTERN = /^[A-Za-z0-9_-]{1,128}$/;

export type SockudoPushMethod = "GET" | "POST" | "DELETE";
export type SockudoPushQuery = Record<
  string,
  string | number | boolean | undefined
>;

export interface SockudoPushRequestOptions {
  method: SockudoPushMethod;
  path: string;
  query?: SockudoPushQuery;
  body?: unknown;
}

export interface SignedSockudoPushRequest {
  url: string;
  headers: Headers;
  body?: string;
}

export interface SockudoPushResponse<T = unknown> {
  status: number;
  body: T | null;
  retryAfter?: string;
  forcedAsync?: string;
}

export type SockudoPushRequester = <T = unknown>(
  app: AppRecord,
  options: SockudoPushRequestOptions,
) => Promise<SockudoPushResponse<T>>;

export class SockudoPushTransportError extends Error {
  constructor() {
    super("Sockudo push API is unavailable");
    this.name = "SockudoPushTransportError";
  }
}

function normalizeQuery(
  query: SockudoPushQuery | undefined,
): Record<string, string> {
  const normalized: Record<string, string> = {};
  for (const [rawKey, rawValue] of Object.entries(query ?? {})) {
    const lowercaseKey = rawKey.toLowerCase();
    if (RESERVED_QUERY_KEYS.has(lowercaseKey)) {
      throw new Error(
        `Reserved Sockudo authentication query key: ${lowercaseKey}`,
      );
    }
    if (
      Object.keys(normalized).some(
        (existing) => existing.toLowerCase() === lowercaseKey,
      )
    ) {
      throw new Error(`Duplicate Sockudo query key: ${lowercaseKey}`);
    }
    if (rawValue !== undefined) normalized[rawKey] = String(rawValue);
  }
  return normalized;
}

function canonicalQuery(params: Record<string, string>): string {
  return Object.entries(params)
    .map(([key, value]) => [key.toLowerCase(), value] as const)
    .sort(([left], [right]) => (left < right ? -1 : left > right ? 1 : 0))
    .map(([key, value]) => `${key}=${value}`)
    .join("&");
}

function validatePushPath(app: AppRecord, path: string): void {
  if (!APP_ID_PATTERN.test(app.id)) {
    throw new Error("App id is not valid for the Sockudo push API");
  }
  const prefix = `/apps/${app.id}/push/`;
  if (!path.startsWith(prefix) || path.includes("?") || path.includes("#")) {
    throw new Error("Sockudo push request path is outside the allowed app scope");
  }
}

export function createSignedSockudoPushRequest(
  app: AppRecord,
  options: SockudoPushRequestOptions,
  nowSeconds = Math.floor(Date.now() / 1_000),
  baseUrl = config.sockudoHttpUrl,
): SignedSockudoPushRequest {
  validatePushPath(app, options.path);

  const body =
    options.body === undefined ? undefined : JSON.stringify(options.body);
  const params = normalizeQuery(options.query);
  params.auth_key = app.key;
  params.auth_timestamp = String(nowSeconds);
  params.auth_version = "1.0";
  if (options.method === "POST") {
    params.body_md5 = createHash("md5")
      .update(body ?? "", "utf8")
      .digest("hex");
  } else if (body !== undefined) {
    throw new Error(`${options.method} push requests cannot include a body`);
  }

  const stringToSign = [
    options.method,
    options.path,
    canonicalQuery(params),
  ].join("\n");
  const signature = createHmac("sha256", app.secret)
    .update(stringToSign, "utf8")
    .digest("hex");

  const url = new URL(options.path, baseUrl);
  for (const key of Object.keys(params).sort()) {
    url.searchParams.append(key, params[key]!);
  }
  url.searchParams.append("auth_signature", signature);

  const headers = new Headers({
    "x-sockudo-push-capability": "push-admin",
  });
  if (body !== undefined) {
    headers.set("content-type", "application/json");
  }

  return { url: url.toString(), headers, body };
}

function sanitizeResponseValue(value: unknown, appSecret: string): unknown {
  if (typeof value === "string") {
    if (
      value.includes("auth_signature=") ||
      value.includes("body_md5=") ||
      value.includes(appSecret)
    ) {
      return "[REDACTED]";
    }
    return value;
  }
  if (Array.isArray(value)) {
    return value.map((item) => sanitizeResponseValue(item, appSecret));
  }
  if (!value || typeof value !== "object") return value;

  const redactedKeys = new Set([
    "secret",
    "privatekey",
    "private_key",
    "clientsecret",
    "client_secret",
    "serviceaccountjson",
    "service_account_json",
    "p12",
    "p12password",
    "p12_password",
    "pem",
    "auth_signature",
    "body_md5",
  ]);
  const result: Record<string, unknown> = {};
  for (const [key, item] of Object.entries(value)) {
    result[key] = redactedKeys.has(key.toLowerCase())
      ? "[REDACTED]"
      : sanitizeResponseValue(item, appSecret);
  }
  return result;
}

async function parseResponseBody(
  response: Response,
  appSecret: string,
): Promise<unknown | null> {
  if (response.status === 204) return null;
  const text = await response.text();
  if (!text) return null;
  try {
    return sanitizeResponseValue(JSON.parse(text), appSecret);
  } catch {
    return response.ok
      ? { error: "Sockudo push API returned an unexpected response" }
      : { error: "Sockudo push API rejected the request" };
  }
}

export const requestSockudoPush: SockudoPushRequester = async <T>(
  app: AppRecord,
  options: SockudoPushRequestOptions,
): Promise<SockudoPushResponse<T>> => {
  let signed: SignedSockudoPushRequest;
  try {
    signed = createSignedSockudoPushRequest(app, options);
  } catch (error) {
    if (error instanceof Error && error.message.startsWith("App id")) {
      throw error;
    }
    throw new SockudoPushTransportError();
  }

  try {
    const response = await fetch(signed.url, {
      method: options.method,
      headers: signed.headers,
      body: signed.body,
      signal: AbortSignal.timeout(10_000),
    });
    return {
      status: response.status,
      body: (await parseResponseBody(response, app.secret)) as T | null,
      retryAfter: response.headers.get("retry-after") ?? undefined,
      forcedAsync:
        response.headers.get("x-sockudo-push-forced-async") ?? undefined,
    };
  } catch {
    throw new SockudoPushTransportError();
  }
};

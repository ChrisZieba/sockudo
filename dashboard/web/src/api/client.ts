export interface Webhook {
  url?: string;
  lambda_function?: string;
  lambda?: { function_name: string; region: string };
  event_types: string[];
  filter?: {
    channel_prefix?: string;
    channel_suffix?: string;
    channel_pattern?: string;
    channel_namespace?: string;
    channel_namespaces?: string[];
  };
  headers?: Record<string, string>;
  retry?: {
    enabled?: boolean;
    max_attempts?: number;
    max_elapsed_time_ms?: number;
    initial_backoff_ms?: number;
    max_backoff_ms?: number;
  };
  request_timeout_ms?: number;
}

export interface MessageRateLimit {
  enabled: boolean;
  max_attempts: number;
  decay_seconds: number;
  terminate_on_limit: boolean;
}

export interface HistoryPolicy {
  enabled?: boolean;
  rewind_enabled?: boolean;
  retention_window_seconds?: number;
  max_messages_per_channel?: number;
  max_bytes_per_channel?: number;
}

export interface PresenceHistoryPolicy {
  enabled?: boolean;
  retention_window_seconds?: number;
  max_events_per_channel?: number;
  max_bytes_per_channel?: number;
}

export interface ChannelNamespace {
  name: string;
  channel_name_pattern?: string;
  max_channel_name_length?: number;
  annotations_enabled?: boolean;
  allow_user_limited_channels?: boolean;
  allow_subscribe_for_client?: boolean;
  allow_publish_for_client?: boolean;
  allow_presence_for_client?: boolean;
  history?: Omit<HistoryPolicy, "enabled">;
  presence_history?: PresenceHistoryPolicy;
}

export interface AppPolicy {
  limits: {
    max_connections: number;
    max_client_events_per_second: number;
    max_backend_events_per_second?: number;
    max_read_requests_per_second?: number;
    max_presence_members_per_channel?: number;
    max_presence_member_size_in_kb?: number;
    max_channel_name_length?: number;
    max_event_channels_at_once?: number;
    max_event_name_length?: number;
    max_event_payload_in_kb?: number;
    max_event_batch_size?: number;
    decay_seconds?: number;
    terminate_on_limit?: boolean;
    message_rate_limit?: MessageRateLimit;
  };
  features: {
    enable_client_messages: boolean;
    enable_user_authentication?: boolean;
    enable_watchlist_events?: boolean;
  };
  channels: {
    allowed_origins?: string[];
    annotations_enabled?: boolean;
    channel_delta_compression?: Record<
      string,
      | "inherit"
      | "disabled"
      | "fossil"
      | "xdelta3"
      | {
          enabled?: boolean;
          algorithm?: "Fossil" | "Xdelta3";
          conflation_key?: string;
          max_messages_per_key?: number;
          max_conflation_keys?: number;
          enable_tags?: boolean;
        }
    >;
    channel_namespaces?: ChannelNamespace[];
  };
  webhooks?: Webhook[];
  idempotency?: { enabled?: boolean; ttl_seconds?: number };
  connection_recovery?: {
    enabled?: boolean;
    buffer_ttl_seconds?: number;
    max_buffer_size?: number;
  };
  history?: HistoryPolicy;
  presence_history?: PresenceHistoryPolicy;
}

export interface App {
  id: string;
  key: string;
  secret: string;
  enabled: boolean;
  policy: AppPolicy;
  webhook_count?: number;
}

export interface MetricsSummary {
  connected_sockets: number;
  new_connections_total: number;
  new_disconnections_total: number;
  ws_messages_received_total: number;
  ws_messages_sent_total: number;
  http_calls_received_total: number;
  channel_subscriptions_total: number;
  rate_limit_triggered_total: number;
  connection_errors_total: number;
  by_app: Array<{ app_id: string; connected_sockets: number }>;
}

export interface MetricSample {
  name: string;
  labels: Record<string, string>;
  value: number;
  timestamp?: number;
}

export interface MetricFamily {
  name: string;
  help?: string;
  type?: "counter" | "gauge" | "histogram" | "summary" | "untyped";
  samples: MetricSample[];
}

export interface MetricsResponse {
  samples: MetricSample[];
  families?: MetricFamily[];
  scraped_at?: string;
}

export interface StatsResponse {
  totals: {
    apps: number;
    connections: number;
    users: number;
  };
  apps: Array<{
    app_id: string;
    connections: number;
    users: number;
    occupancy: {
      channels: number;
      subscriptions: number;
    };
  }>;
  memory: { used: number; total: number; percent: number };
}

export type PushProvider =
  | "fcm"
  | "apns"
  | "webPush"
  | "hms"
  | "wns"
  | "realtime";

export type PushCredentialProvider = Exclude<PushProvider, "realtime">;
export type PushCredentialRouteProvider =
  | "fcm"
  | "apns"
  | "webpush"
  | "hms"
  | "wns";

export interface PushListResponse<T> {
  items: T[];
  next_cursor: string | null;
  has_more: boolean;
}

export interface PushCredential {
  app_id: string;
  credential_id: string;
  provider: PushCredentialProvider;
  version: number;
}

interface VersionedPushCredentialInput {
  credentialId?: string;
  version?: number;
}

export interface FcmPushCredentialInput
  extends VersionedPushCredentialInput {
  serviceAccountJson: Record<string, unknown>;
}

export interface ApnsPushCredentialInput
  extends VersionedPushCredentialInput {
  p12?: string;
  p12Password?: string;
  pem?: string;
  teamId?: string;
  keyId?: string;
  privateKey?: string;
}

export interface WebPushCredentialInput
  extends VersionedPushCredentialInput {
  publicKey: string;
  privateKey: string;
}

export interface HmsPushCredentialInput
  extends VersionedPushCredentialInput {
  hmsAppId: string;
  clientSecret: string;
}

export interface WnsPushCredentialInput
  extends VersionedPushCredentialInput {
  packageSid: string;
  clientSecret: string;
}

export interface PushCredentialInputByProvider {
  fcm: FcmPushCredentialInput;
  apns: ApnsPushCredentialInput;
  webpush: WebPushCredentialInput;
  hms: HmsPushCredentialInput;
  wns: WnsPushCredentialInput;
}

export interface PushDeviceRecipient {
  transportType: "gcm" | "apns" | "web" | "hms" | "wns" | "realtime";
  provider: PushProvider;
  tokenHash: string;
}

export interface PushDevice {
  appId: string;
  id: string;
  clientId?: string;
  formFactor: string;
  platform: string;
  timezone: string;
  locale: string;
  lastActiveAtMs: number;
  pushState: "ACTIVE" | "FAILING" | "FAILED";
  pushFailureCount: number;
  recipient: PushDeviceRecipient;
}

export interface PushChannelSubscription {
  appId: string;
  channel: string;
  deviceId: string;
  clientId?: string;
  provider: PushProvider;
  tokenHash: string;
  credentialVersion?: number;
}

export type PushTarget =
  | { type: "device"; device_id: string }
  | { type: "client"; client_id: string }
  | { type: "channel"; channel: string }
  | { type: "registeredTopic"; topic: string }
  | { type: "userTopic"; topic: string }
  | { type: "providerTopic"; provider: PushProvider; topic: string }
  | {
      type: "providerCondition";
      provider: PushProvider;
      condition: string;
    }
  | { type: "indexedFilter"; filter: Record<string, unknown> }
  | { type: "recipient"; recipient: Record<string, unknown> };

export interface PushPayload {
  templateId?: string;
  templateData?: unknown;
  title?: string;
  body?: string;
  icon?: string;
  sound?: string;
  collapseKey?: string;
}

export interface PushProviderOverride {
  provider: PushProvider;
  payload: unknown;
}

export interface PushPublishInput {
  publishId?: string;
  recipients: PushTarget[];
  payload: PushPayload;
  providerOverrides?: PushProviderOverride[];
  sync?: boolean;
  notBeforeMs?: number;
  expiresAtMs?: number;
}

export interface PushPublishAccepted {
  publishId: string;
  status: string;
  expectedRecipients: number;
  fanoutRegime: string;
  renderedPayloads: Array<{
    provider: PushProvider;
    payload: unknown;
    usedOverride: boolean;
  }>;
}

export interface PushPublishStatus {
  appId: string;
  publishId: string;
  state: string;
  counters: Record<string, number>;
  fanoutRegime?: string;
  retryAfterMs?: number;
  errorReason?: string;
}

export interface PushDeadLetter {
  deadLetterId: string;
  appId: string;
  publishId: string;
  provider?: PushProvider;
  stage: string;
  key: string;
  reason: string;
  occurredAtMs: number;
  replayable: boolean;
}

export interface PushPageQuery {
  limit?: number;
  cursor?: string;
}

export interface PushSubscriptionQuery extends PushPageQuery {
  channel?: string;
  deviceId?: string;
}

export interface PushDeadLetterQuery extends PushPageQuery {
  provider?: PushCredentialRouteProvider;
  sinceMs?: number;
  untilMs?: number;
}

import type { DashboardUser, UserRole } from "@/types/user";

class ApiError extends Error {
  constructor(
    message: string,
    public status: number,
  ) {
    super(message);
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, {
    credentials: "include",
    headers: { "Content-Type": "application/json", ...init?.headers },
    ...init,
  });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) {
    throw new ApiError(data.error ?? res.statusText, res.status);
  }
  return data as T;
}

function withQuery(
  path: string,
  query?: object,
): string {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(query ?? {})) {
    if (value !== undefined && value !== "") {
      search.set(key, String(value));
    }
  }
  const encoded = search.toString();
  return encoded ? `${path}?${encoded}` : path;
}

export const api = {
  login: (email: string, password: string) =>
    request<DashboardUser>("/api/v1/auth/login", {
      method: "POST",
      body: JSON.stringify({ email, password }),
    }),
  logout: () =>
    request<{ ok: boolean }>("/api/v1/auth/logout", { method: "POST" }),
  me: () => request<DashboardUser>("/api/v1/auth/me"),
  listUsers: () => request<DashboardUser[]>("/api/v1/users"),
  createUser: (body: {
    email: string;
    password: string;
    name?: string;
    role?: UserRole;
    active?: boolean;
  }) =>
    request<DashboardUser>("/api/v1/users", {
      method: "POST",
      body: JSON.stringify(body),
    }),
  updateUser: (
    id: string,
    body: Partial<{
      email: string;
      name: string;
      role: UserRole;
      active: boolean;
      password: string;
    }>,
  ) =>
    request<DashboardUser>(`/api/v1/users/${id}`, {
      method: "PUT",
      body: JSON.stringify(body),
    }),
  deleteUser: (id: string) =>
    request<{ ok: boolean }>(`/api/v1/users/${id}`, { method: "DELETE" }),
  changePassword: (
    id: string,
    body: { current_password?: string; new_password: string },
  ) =>
    request<DashboardUser>(`/api/v1/users/${id}/change-password`, {
      method: "POST",
      body: JSON.stringify(body),
    }),
  listApps: () => request<App[]>("/api/v1/apps"),
  getApp: (id: string, revealSecret = false) =>
    request<App>(
      `/api/v1/apps/${id}${revealSecret ? "?reveal_secret=true" : ""}`,
    ),
  createApp: (body: {
    id: string;
    key: string;
    secret?: string;
    enabled?: boolean;
    policy?: Partial<AppPolicy>;
  }) =>
    request<App>("/api/v1/apps", {
      method: "POST",
      body: JSON.stringify(body),
    }),
  updateApp: (
    id: string,
    body: Partial<App> & { replace_policy?: boolean },
  ) =>
    request<App>(`/api/v1/apps/${id}`, {
      method: "PUT",
      body: JSON.stringify(body),
    }),
  deleteApp: (id: string) =>
    request<{ ok: boolean }>(`/api/v1/apps/${id}`, { method: "DELETE" }),
  rotateSecret: (id: string) =>
    request<App>(`/api/v1/apps/${id}/rotate-secret`, { method: "POST" }),
  listWebhooks: (appId: string) =>
    request<Webhook[]>(`/api/v1/apps/${appId}/webhooks`),
  createWebhook: (appId: string, webhook: Webhook) =>
    request<App>(`/api/v1/apps/${appId}/webhooks`, {
      method: "POST",
      body: JSON.stringify(webhook),
    }),
  updateWebhook: (appId: string, index: number, webhook: Webhook) =>
    request<App>(`/api/v1/apps/${appId}/webhooks/${index}`, {
      method: "PUT",
      body: JSON.stringify(webhook),
    }),
  deleteWebhook: (appId: string, index: number) =>
    request<App>(`/api/v1/apps/${appId}/webhooks/${index}`, {
      method: "DELETE",
    }),
  listPushCredentials: (appId: string, query?: PushPageQuery) =>
    request<PushListResponse<PushCredential>>(
      withQuery(`/api/v1/apps/${appId}/push/credentials`, query),
    ),
  storePushCredential: <P extends PushCredentialRouteProvider>(
    appId: string,
    provider: P,
    body: PushCredentialInputByProvider[P],
  ) =>
    request<PushCredential>(
      `/api/v1/apps/${appId}/push/credentials/${provider}`,
      {
        method: "POST",
        body: JSON.stringify(body),
      },
    ),
  listPushDevices: (appId: string, query?: PushPageQuery) =>
    request<PushListResponse<PushDevice>>(
      withQuery(`/api/v1/apps/${appId}/push/devices`, query),
    ),
  deletePushDevice: (appId: string, deviceId: string) =>
    request<unknown>(
      `/api/v1/apps/${appId}/push/devices/${encodeURIComponent(deviceId)}`,
      { method: "DELETE" },
    ),
  listPushSubscriptions: (
    appId: string,
    query?: PushSubscriptionQuery,
  ) =>
    request<PushListResponse<PushChannelSubscription>>(
      withQuery(`/api/v1/apps/${appId}/push/subscriptions`, query),
    ),
  publishPush: (appId: string, body: PushPublishInput) =>
    request<PushPublishAccepted>(`/api/v1/apps/${appId}/push/publish`, {
      method: "POST",
      body: JSON.stringify(body),
    }),
  getPushPublishStatus: (appId: string, publishId: string) =>
    request<PushPublishStatus>(
      `/api/v1/apps/${appId}/push/publish/${encodeURIComponent(publishId)}/status`,
    ),
  listPushDeadLetters: (appId: string, query?: PushDeadLetterQuery) =>
    request<PushListResponse<PushDeadLetter>>(
      withQuery(`/api/v1/apps/${appId}/push/dead-letters`, query),
    ),
  replayPushDeadLetter: (appId: string, deadLetterId: string) =>
    request<unknown>(
      `/api/v1/apps/${appId}/push/dead-letters/${encodeURIComponent(deadLetterId)}/replay`,
      { method: "POST", body: JSON.stringify({}) },
    ),
  metricsSummary: () =>
    request<MetricsSummary>("/api/v1/ops/metrics/summary"),
  metrics: () => request<MetricsResponse>("/api/v1/ops/metrics"),
  stats: () => request<StatsResponse>("/api/v1/ops/stats"),
  opsConfig: () =>
    request<{
      app_manager_driver: string;
      sockudo_http_url: string;
      sockudo_metrics_url: string;
    }>("/api/v1/ops/config"),
};

export { ApiError };

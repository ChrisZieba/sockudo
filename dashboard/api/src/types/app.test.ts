import { describe, expect, test } from "bun:test";
import { rowToApp } from "../db/types.ts";
import { mergePolicy, type AppPolicy } from "./app.ts";

function exhaustivePolicy(): AppPolicy {
  return {
    limits: {
      max_connections: 250,
      max_backend_events_per_second: 80,
      max_client_events_per_second: 40,
      max_read_requests_per_second: 20,
      max_presence_members_per_channel: 200,
      max_presence_member_size_in_kb: 4,
      max_channel_name_length: 180,
      max_event_channels_at_once: 12,
      max_event_name_length: 96,
      max_event_payload_in_kb: 64,
      max_event_batch_size: 15,
      decay_seconds: 3,
      terminate_on_limit: true,
      message_rate_limit: {
        enabled: true,
        max_attempts: 90,
        decay_seconds: 30,
        terminate_on_limit: false,
      },
    },
    features: {
      enable_client_messages: true,
      enable_user_authentication: true,
      enable_watchlist_events: true,
    },
    channels: {
      allowed_origins: ["https://app.example.com"],
      annotations_enabled: true,
      channel_delta_compression: {
        "prices-*": {
          enabled: true,
          algorithm: "Fossil",
          conflation_key: "data.symbol",
          max_messages_per_key: 8,
          max_conflation_keys: 100,
          enable_tags: true,
        },
      },
      channel_namespaces: [
        {
          name: "trading",
          channel_name_pattern: "^trading:",
          max_channel_name_length: 120,
          annotations_enabled: true,
          allow_user_limited_channels: true,
          allow_subscribe_for_client: true,
          allow_publish_for_client: false,
          allow_presence_for_client: true,
          history: {
            rewind_enabled: true,
            retention_window_seconds: 3_600,
            max_messages_per_channel: 5_000,
            max_bytes_per_channel: 10_000_000,
          },
          presence_history: {
            enabled: true,
            retention_window_seconds: 600,
            max_events_per_channel: 1_000,
            max_bytes_per_channel: 2_000_000,
          },
        },
      ],
    },
    webhooks: [
      {
        url: "https://hooks.example.com/sockudo",
        event_types: ["message_version_created"],
        filter: { channel_namespace: "trading" },
        retry: { enabled: true, max_attempts: 5 },
      },
    ],
    idempotency: { enabled: true, ttl_seconds: 600 },
    connection_recovery: {
      enabled: true,
      buffer_ttl_seconds: 120,
      max_buffer_size: 2_000,
    },
    history: {
      enabled: true,
      rewind_enabled: true,
      retention_window_seconds: 86_400,
      max_messages_per_channel: 50_000,
      max_bytes_per_channel: 100_000_000,
    },
    presence_history: {
      enabled: true,
      retention_window_seconds: 3_600,
      max_events_per_channel: 20_000,
      max_bytes_per_channel: 50_000_000,
    },
  };
}

describe("app policy merging", () => {
  test("preserves every untouched advanced field during a narrow update", () => {
    const base = exhaustivePolicy();
    const merged = mergePolicy(
      {
        limits: { max_connections: 500 },
        history: { enabled: false },
      },
      base,
    );

    expect(merged.limits.max_connections).toBe(500);
    expect(merged.history).toEqual({
      ...base.history,
      enabled: false,
    });
    expect(merged.limits.message_rate_limit).toEqual(
      base.limits.message_rate_limit,
    );
    expect(merged.channels).toEqual(base.channels);
    expect(merged.connection_recovery).toEqual(base.connection_recovery);
    expect(merged.presence_history).toEqual(base.presence_history);
    expect(merged.webhooks).toEqual(base.webhooks);
  });

  test("deep-merges a partial all-message rate-limit update", () => {
    const base = exhaustivePolicy();
    const merged = mergePolicy(
      {
        limits: {
          message_rate_limit: {
            ...base.limits.message_rate_limit!,
            max_attempts: 120,
          },
        },
      },
      base,
    );

    expect(merged.limits.message_rate_limit).toEqual({
      enabled: true,
      max_attempts: 120,
      decay_seconds: 30,
      terminate_on_limit: false,
    });
  });

  test("returns a clone when no update is supplied", () => {
    const base = exhaustivePolicy();
    const merged = mergePolicy(undefined, base);
    merged.channels.allowed_origins?.push("https://other.example.com");

    expect(merged).not.toBe(base);
    expect(base.channels.allowed_origins).toEqual([
      "https://app.example.com",
    ]);
  });
});

describe("legacy app rows", () => {
  test("reconstructs advanced flat and JSON policy fields without data loss", () => {
    const app = rowToApp({
      id: "legacy-app",
      key: "legacy-key",
      secret: "legacy-secret",
      enabled: true,
      policy: null,
      max_connections: 400,
      max_client_events_per_second: 50,
      max_presence_members_per_channel: 120,
      max_presence_member_size_in_kb: 8,
      max_channel_name_length: 140,
      max_event_channels_at_once: 16,
      max_event_name_length: 80,
      max_event_payload_in_kb: 96,
      max_event_batch_size: 20,
      channel_delta_compression: { "prices-*": "fossil" },
      idempotency: { enabled: true, ttl_seconds: 300 },
      connection_recovery: {
        enabled: true,
        buffer_ttl_seconds: 90,
        max_buffer_size: 500,
      },
    });

    expect(app.policy.limits).toMatchObject({
      max_connections: 400,
      max_client_events_per_second: 50,
      max_presence_members_per_channel: 120,
      max_presence_member_size_in_kb: 8,
      max_channel_name_length: 140,
      max_event_channels_at_once: 16,
      max_event_name_length: 80,
      max_event_payload_in_kb: 96,
      max_event_batch_size: 20,
    });
    expect(app.policy.channels.channel_delta_compression).toEqual({
      "prices-*": "fossil",
    });
    expect(app.policy.idempotency).toEqual({
      enabled: true,
      ttl_seconds: 300,
    });
    expect(app.policy.connection_recovery).toEqual({
      enabled: true,
      buffer_ttl_seconds: 90,
      max_buffer_size: 500,
    });
  });
});

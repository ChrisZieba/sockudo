import mysql from "mysql2/promise";
import { config } from "../config.ts";
import {
  mergePolicy,
  type AppCreateInput,
  type AppPolicy,
  type AppRecord,
  type AppUpdateInput,
} from "../types/app.ts";
import {
  type AppsRepository,
  policyToFlat,
  rowToApp,
} from "./types.ts";

export class MysqlAppsRepository implements AppsRepository {
  private pool: mysql.Pool;
  private table: string;

  constructor() {
    const my = config.database.mysql;
    this.table = my.table;
    this.pool = mysql.createPool({
      host: my.host,
      port: my.port,
      user: my.user,
      password: my.password,
      database: my.database,
      connectionLimit: 10,
    });
  }

  private async query<T>(sql: string, params: unknown[] = []): Promise<T[]> {
    const [rows] = await this.pool.query(sql, params);
    return rows as T[];
  }

  async list(): Promise<AppRecord[]> {
    const rows = await this.query<Record<string, unknown>>(`
      SELECT id, \`key\`, secret, enabled, policy, webhooks, allowed_origins,
             max_connections, enable_client_messages,
             max_backend_events_per_second, max_client_events_per_second,
             max_read_requests_per_second, max_presence_members_per_channel,
             max_presence_member_size_in_kb, max_channel_name_length,
             max_event_channels_at_once, max_event_name_length,
             max_event_payload_in_kb, max_event_batch_size,
             enable_user_authentication, enable_watchlist_events,
             channel_delta_compression, idempotency, connection_recovery
      FROM \`${this.table}\`
      ORDER BY id ASC
    `);
    return rows.map((row) => this.mapRow(row));
  }

  async findById(id: string): Promise<AppRecord | null> {
    const rows = await this.query<Record<string, unknown>>(
      `
      SELECT id, \`key\`, secret, enabled, policy, webhooks, allowed_origins,
             max_connections, enable_client_messages,
             max_backend_events_per_second, max_client_events_per_second,
             max_read_requests_per_second, max_presence_members_per_channel,
             max_presence_member_size_in_kb, max_channel_name_length,
             max_event_channels_at_once, max_event_name_length,
             max_event_payload_in_kb, max_event_batch_size,
             enable_user_authentication, enable_watchlist_events,
             channel_delta_compression, idempotency, connection_recovery
      FROM \`${this.table}\`
      WHERE id = ?
    `,
      [id],
    );
    if (rows.length === 0) return null;
    return this.mapRow(rows[0]);
  }

  async create(input: AppCreateInput): Promise<AppRecord> {
    const policy = mergePolicy(input.policy);
    const flat = policyToFlat(policy);
    const enabled = input.enabled ?? true;

    await this.query(
      `
      INSERT INTO \`${this.table}\` (
        id, \`key\`, secret, max_connections, enable_client_messages, enabled,
        max_backend_events_per_second, max_client_events_per_second,
        max_read_requests_per_second, max_presence_members_per_channel,
        max_presence_member_size_in_kb, max_channel_name_length,
        max_event_channels_at_once, max_event_name_length,
        max_event_payload_in_kb, max_event_batch_size,
        enable_user_authentication, enable_watchlist_events,
        policy, webhooks, allowed_origins, channel_delta_compression,
        idempotency, connection_recovery
      ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    `,
      [
        input.id,
        input.key,
        input.secret,
        flat.max_connections,
        flat.enable_client_messages,
        enabled,
        flat.max_backend_events_per_second,
        flat.max_client_events_per_second,
        flat.max_read_requests_per_second,
        flat.max_presence_members_per_channel,
        flat.max_presence_member_size_in_kb,
        flat.max_channel_name_length,
        flat.max_event_channels_at_once,
        flat.max_event_name_length,
        flat.max_event_payload_in_kb,
        flat.max_event_batch_size,
        flat.enable_user_authentication,
        flat.enable_watchlist_events,
        JSON.stringify(policy),
        JSON.stringify(policy.webhooks ?? []),
        JSON.stringify(policy.channels.allowed_origins ?? ["*"]),
        JSON.stringify(policy.channels.channel_delta_compression ?? null),
        JSON.stringify(policy.idempotency ?? null),
        JSON.stringify(policy.connection_recovery ?? null),
      ],
    );

    const created = await this.findById(input.id);
    if (!created) throw new Error("Failed to create app");
    return created;
  }

  async update(id: string, input: AppUpdateInput): Promise<AppRecord> {
    const existing = await this.findById(id);
    if (!existing) throw new Error("App not found");

    const policy = input.replace_policy
      ? mergePolicy(input.policy)
      : mergePolicy(input.policy, existing.policy);
    const flat = policyToFlat(policy);
    const key = input.key ?? existing.key;
    const secret = input.secret ?? existing.secret;
    const enabled = input.enabled ?? existing.enabled;

    const result = await this.pool.query(
      `
      UPDATE \`${this.table}\` SET
        \`key\` = ?, secret = ?, max_connections = ?, enable_client_messages = ?,
        enabled = ?, max_backend_events_per_second = ?,
        max_client_events_per_second = ?, max_read_requests_per_second = ?,
        max_presence_members_per_channel = ?, max_presence_member_size_in_kb = ?,
        max_channel_name_length = ?, max_event_channels_at_once = ?,
        max_event_name_length = ?, max_event_payload_in_kb = ?,
        max_event_batch_size = ?, enable_user_authentication = ?,
        enable_watchlist_events = ?, policy = ?, webhooks = ?,
        allowed_origins = ?, channel_delta_compression = ?,
        idempotency = ?, connection_recovery = ?
      WHERE id = ?
    `,
      [
        key,
        secret,
        flat.max_connections,
        flat.enable_client_messages,
        enabled,
        flat.max_backend_events_per_second,
        flat.max_client_events_per_second,
        flat.max_read_requests_per_second,
        flat.max_presence_members_per_channel,
        flat.max_presence_member_size_in_kb,
        flat.max_channel_name_length,
        flat.max_event_channels_at_once,
        flat.max_event_name_length,
        flat.max_event_payload_in_kb,
        flat.max_event_batch_size,
        flat.enable_user_authentication,
        flat.enable_watchlist_events,
        JSON.stringify(policy),
        JSON.stringify(policy.webhooks ?? []),
        JSON.stringify(policy.channels.allowed_origins ?? ["*"]),
        JSON.stringify(policy.channels.channel_delta_compression ?? null),
        JSON.stringify(policy.idempotency ?? null),
        JSON.stringify(policy.connection_recovery ?? null),
        id,
      ],
    );

    const header = result[0] as { affectedRows?: number };
    if (!header.affectedRows) throw new Error("App not found");
    const updated = await this.findById(id);
    if (!updated) throw new Error("Failed to update app");
    return updated;
  }

  async delete(id: string): Promise<void> {
    const result = await this.pool.query(
      `DELETE FROM \`${this.table}\` WHERE id = ?`,
      [id],
    );
    const header = result[0] as { affectedRows?: number };
    if (!header.affectedRows) throw new Error("App not found");
  }

  async close(): Promise<void> {
    await this.pool.end();
  }

  private mapRow(row: Record<string, unknown>): AppRecord {
    const parseJson = <T>(value: unknown): T | null => {
      if (value == null) return null;
      if (typeof value === "object") return value as T;
      if (typeof value === "string") return JSON.parse(value) as T;
      return null;
    };

    return rowToApp({
      id: row.id as string,
      key: row.key as string,
      secret: row.secret as string,
      enabled: Boolean(row.enabled),
      policy: parseJson<AppPolicy>(row.policy),
      webhooks: parseJson<AppPolicy["webhooks"]>(row.webhooks),
      allowed_origins: parseJson<string[]>(row.allowed_origins),
      max_connections: Number(row.max_connections),
      enable_client_messages: Boolean(row.enable_client_messages),
      max_client_events_per_second: Number(row.max_client_events_per_second),
      max_backend_events_per_second:
        row.max_backend_events_per_second != null
          ? Number(row.max_backend_events_per_second)
          : null,
      max_read_requests_per_second:
        row.max_read_requests_per_second != null
          ? Number(row.max_read_requests_per_second)
          : null,
      max_presence_members_per_channel:
        row.max_presence_members_per_channel != null
          ? Number(row.max_presence_members_per_channel)
          : null,
      max_presence_member_size_in_kb:
        row.max_presence_member_size_in_kb != null
          ? Number(row.max_presence_member_size_in_kb)
          : null,
      max_channel_name_length:
        row.max_channel_name_length != null
          ? Number(row.max_channel_name_length)
          : null,
      max_event_channels_at_once:
        row.max_event_channels_at_once != null
          ? Number(row.max_event_channels_at_once)
          : null,
      max_event_name_length:
        row.max_event_name_length != null
          ? Number(row.max_event_name_length)
          : null,
      max_event_payload_in_kb:
        row.max_event_payload_in_kb != null
          ? Number(row.max_event_payload_in_kb)
          : null,
      max_event_batch_size:
        row.max_event_batch_size != null
          ? Number(row.max_event_batch_size)
          : null,
      enable_user_authentication:
        row.enable_user_authentication != null
          ? Boolean(row.enable_user_authentication)
          : null,
      enable_watchlist_events:
        row.enable_watchlist_events != null
          ? Boolean(row.enable_watchlist_events)
          : null,
      channel_delta_compression: parseJson<
        AppPolicy["channels"]["channel_delta_compression"]
      >(row.channel_delta_compression),
      idempotency: parseJson<AppPolicy["idempotency"]>(row.idempotency),
      connection_recovery: parseJson<AppPolicy["connection_recovery"]>(
        row.connection_recovery,
      ),
    });
  }
}

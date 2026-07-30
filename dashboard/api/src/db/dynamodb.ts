import {
  DynamoDBClient,
  ScanCommand,
  GetItemCommand,
  PutItemCommand,
  DeleteItemCommand,
} from "@aws-sdk/client-dynamodb";
import { marshall, unmarshall } from "@aws-sdk/util-dynamodb";
import { config } from "../config.ts";
import {
  mergePolicy,
  type AppCreateInput,
  type AppPolicy,
  type AppRecord,
  type AppUpdateInput,
} from "../types/app.ts";
import { type AppsRepository, rowToApp } from "./types.ts";

export class DynamodbAppsRepository implements AppsRepository {
  private client: DynamoDBClient;
  private table: string;

  constructor() {
    const ddb = config.database.dynamodb;
    this.table = ddb.table;
    this.client = new DynamoDBClient({
      region: ddb.region,
      ...(ddb.endpoint ? { endpoint: ddb.endpoint } : {}),
      ...(ddb.accessKeyId && ddb.secretAccessKey
        ? {
            credentials: {
              accessKeyId: ddb.accessKeyId,
              secretAccessKey: ddb.secretAccessKey,
            },
          }
        : {}),
    });
  }

  async list(): Promise<AppRecord[]> {
    const result = await this.client.send(
      new ScanCommand({ TableName: this.table }),
    );
    return (result.Items ?? [])
      .map((item) => this.itemToApp(unmarshall(item)))
      .sort((a, b) => a.id.localeCompare(b.id));
  }

  async findById(id: string): Promise<AppRecord | null> {
    const result = await this.client.send(
      new GetItemCommand({
        TableName: this.table,
        Key: marshall({ id }),
      }),
    );
    if (!result.Item) return null;
    return this.itemToApp(unmarshall(result.Item));
  }

  async create(input: AppCreateInput): Promise<AppRecord> {
    const existing = await this.findById(input.id);
    if (existing) throw new Error("App already exists");

    const record: AppRecord = {
      id: input.id,
      key: input.key,
      secret: input.secret,
      enabled: input.enabled ?? true,
      policy: mergePolicy(input.policy),
    };

    await this.client.send(
      new PutItemCommand({
        TableName: this.table,
        Item: marshall(this.appToItem(record), { removeUndefinedValues: true }),
        ConditionExpression: "attribute_not_exists(id)",
      }),
    );

    return record;
  }

  async update(id: string, input: AppUpdateInput): Promise<AppRecord> {
    const existing = await this.findById(id);
    if (!existing) throw new Error("App not found");

    const record: AppRecord = {
      id,
      key: input.key ?? existing.key,
      secret: input.secret ?? existing.secret,
      enabled: input.enabled ?? existing.enabled,
      policy: input.replace_policy
        ? mergePolicy(input.policy)
        : mergePolicy(input.policy, existing.policy),
    };

    await this.client.send(
      new PutItemCommand({
        TableName: this.table,
        Item: marshall(this.appToItem(record), { removeUndefinedValues: true }),
      }),
    );

    return record;
  }

  async delete(id: string): Promise<void> {
    await this.client.send(
      new DeleteItemCommand({
        TableName: this.table,
        Key: marshall({ id }),
        ConditionExpression: "attribute_exists(id)",
      }),
    );
  }

  async close(): Promise<void> {
    this.client.destroy();
  }

  private appToItem(app: AppRecord): Record<string, unknown> {
    return {
      id: app.id,
      key: app.key,
      secret: app.secret,
      enabled: app.enabled,
      policy: JSON.stringify(app.policy),
    };
  }

  private itemToApp(item: Record<string, unknown>): AppRecord {
    const policyRaw = item.policy;
    if (typeof policyRaw === "string") {
      return {
        id: String(item.id),
        key: String(item.key),
        secret: String(item.secret),
        enabled: item.enabled !== false,
        policy: JSON.parse(policyRaw) as AppPolicy,
      };
    }
    if (policyRaw && typeof policyRaw === "object") {
      return {
        id: String(item.id),
        key: String(item.key),
        secret: String(item.secret),
        enabled: item.enabled !== false,
        policy: policyRaw as AppPolicy,
      };
    }

    const optionalNumber = (key: string): number | null => {
      const value = item[key];
      return value == null || !Number.isFinite(Number(value))
        ? null
        : Number(value);
    };
    const optionalBoolean = (key: string): boolean | null => {
      const value = item[key];
      return typeof value === "boolean" ? value : null;
    };
    const optionalJson = <T>(key: string): T | null => {
      const value = item[key];
      if (value == null) return null;
      if (typeof value === "object") return value as T;
      if (typeof value !== "string") return null;
      try {
        return JSON.parse(value) as T;
      } catch {
        return null;
      }
    };

    return rowToApp({
      id: String(item.id),
      key: String(item.key),
      secret: String(item.secret),
      enabled: item.enabled !== false,
      policy: null,
      webhooks: optionalJson<AppPolicy["webhooks"]>("webhooks"),
      allowed_origins: Array.isArray(item.allowed_origins)
        ? item.allowed_origins.map(String)
        : optionalJson<string[]>("allowed_origins"),
      max_connections: optionalNumber("max_connections") ?? 10_000,
      enable_client_messages: item.enable_client_messages === true,
      max_backend_events_per_second: optionalNumber(
        "max_backend_events_per_second",
      ),
      max_client_events_per_second:
        optionalNumber("max_client_events_per_second") ?? 1_000,
      max_read_requests_per_second: optionalNumber(
        "max_read_requests_per_second",
      ),
      max_presence_members_per_channel: optionalNumber(
        "max_presence_members_per_channel",
      ),
      max_presence_member_size_in_kb: optionalNumber(
        "max_presence_member_size_in_kb",
      ),
      max_channel_name_length: optionalNumber("max_channel_name_length"),
      max_event_channels_at_once: optionalNumber(
        "max_event_channels_at_once",
      ),
      max_event_name_length: optionalNumber("max_event_name_length"),
      max_event_payload_in_kb: optionalNumber("max_event_payload_in_kb"),
      max_event_batch_size: optionalNumber("max_event_batch_size"),
      enable_user_authentication: optionalBoolean(
        "enable_user_authentication",
      ),
      enable_watchlist_events: optionalBoolean("enable_watchlist_events"),
      channel_delta_compression: optionalJson<
        AppPolicy["channels"]["channel_delta_compression"]
      >("channel_delta_compression"),
      idempotency: optionalJson<AppPolicy["idempotency"]>("idempotency"),
      connection_recovery: optionalJson<AppPolicy["connection_recovery"]>(
        "connection_recovery",
      ),
    });
  }
}

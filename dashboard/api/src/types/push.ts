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

interface VersionedCredentialInput {
  credentialId?: string;
  version?: number;
}

export interface FcmCredentialInput extends VersionedCredentialInput {
  serviceAccountJson: Record<string, unknown>;
}

export interface ApnsCredentialInput extends VersionedCredentialInput {
  p12?: string;
  p12Password?: string;
  pem?: string;
  teamId?: string;
  keyId?: string;
  privateKey?: string;
}

export interface WebPushCredentialInput extends VersionedCredentialInput {
  publicKey: string;
  privateKey: string;
}

export interface HmsCredentialInput extends VersionedCredentialInput {
  hmsAppId: string;
  clientSecret: string;
}

export interface WnsCredentialInput extends VersionedCredentialInput {
  packageSid: string;
  clientSecret: string;
}

export interface PushCredentialInputByProvider {
  fcm: FcmCredentialInput;
  apns: ApnsCredentialInput;
  webpush: WebPushCredentialInput;
  hms: HmsCredentialInput;
  wns: WnsCredentialInput;
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
  | { type: "device"; device_id?: string; deviceId?: string }
  | { type: "client"; client_id?: string; clientId?: string }
  | { type: "channel"; channel: string }
  | { type: "registeredTopic"; topic: string }
  | { type: "userTopic"; topic: string }
  | {
      type: "providerTopic";
      provider: PushProvider;
      topic: string;
    }
  | {
      type: "providerCondition";
      provider: PushProvider;
      condition: string;
    }
  | { type: "indexedFilter"; filter: Record<string, unknown> }
  | {
      type: "recipient";
      recipient: Record<string, unknown>;
    };

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

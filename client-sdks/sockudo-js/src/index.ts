// Main entry point for @sockudo/client

import SockudoClass from "./core/sockudo";
export * from "./core/versioned_messages";
export * from "./core/push";
export * from "./core/token_auth";
export type {
  AuthTokenCallback,
  AuthTokenData,
  AuthTokenReason,
  AuthTokenRequest,
  AuthTokenResult,
  CapabilityTokenAuthData,
  CapabilityTokenExpiredData,
  Options,
} from "./core/options";
export type { WireSerial } from "./core/connection/protocol/message-types";
export type {
  ChannelHistoryPage,
  ChannelHistoryParams,
  GetMessageResponse,
  ListMessageVersionsResponse,
  MessageVersionsParams,
} from "./core/channels/channel";

export default SockudoClass;

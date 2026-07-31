export { version } from "../version.js";
import {
  createClientSession as createCoreClientSession,
  createAgentSession as createCoreAgentSession,
  type ClientSession,
  type ClientSessionOptions,
  type AgentSession,
  type AgentSessionOptions,
} from "../core/transport/index.js";
export * from "./codec/index.js";
import {
  UIMessageCodec,
  type AI,
  type VercelInput,
  type VercelOutput,
  type VercelProjection,
} from "./codec/index.js";

/**
 * Client transport options for Vercel UI messages.
 *
 * @defaultValue `api` defaults to `"/api/chat"`.
 */
export type VercelClientSessionOptions = Omit<
  ClientSessionOptions<VercelInput, VercelOutput, VercelProjection, AI.UIMessage>,
  "api" | "codec"
> & {
  /** Server endpoint URL for the HTTP poke.
   *
   * @defaultValue `"/api/chat"`.
   */
  api?: string;
};

/**
 * Server transport options for Vercel UI messages.
 */
export type VercelAgentSessionOptions = Omit<
  AgentSessionOptions<VercelInput, VercelOutput, VercelProjection, AI.UIMessage>,
  "codec"
>;

/**
 * Creates a Sockudo client transport pre-bound to {@link UIMessageCodec}.
 *
 * Async methods reject with `ErrorInfo`; synchronous misuse throws `ErrorInfo`
 * with `InvalidArgument`.
 */
export function createClientSession(
  options: VercelClientSessionOptions,
): ClientSession<VercelInput, VercelOutput, VercelProjection, AI.UIMessage> {
  return createCoreClientSession({
    ...options,
    api: options.api ?? "/api/chat",
    codec: UIMessageCodec,
  });
}

/**
 * Creates a Sockudo server transport pre-bound to {@link UIMessageCodec}.
 *
 * Public methods reject with `ErrorInfo`; synchronous misuse throws
 * `ErrorInfo` with `InvalidArgument`.
 */
export function createAgentSession(
  options: VercelAgentSessionOptions,
): AgentSession<VercelOutput, VercelProjection, AI.UIMessage> {
  return createCoreAgentSession({
    ...options,
    codec: UIMessageCodec,
  });
}

export * from "./transport/index.js";

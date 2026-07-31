export { version } from "./version.js";
// The INBOUND_LEGACY_* constants are deliberately not re-exported: they exist
// only so history hydration can read pre-3.0 channel content, and are not part
// of the public API.
export {
  EVENT_AI_CANCEL,
  EVENT_AI_INPUT,
  EVENT_AI_OUTPUT,
  EVENT_AI_RUN_END,
  EVENT_AI_RUN_RESUME,
  EVENT_AI_RUN_START,
  EVENT_AI_RUN_SUSPEND,
  EVENT_AI_STEP_END,
  EVENT_AI_STEP_START,
  HEADER_CODEC_MESSAGE_ID,
  HEADER_DISCRETE,
  HEADER_ERROR_CODE,
  HEADER_ERROR_MESSAGE,
  HEADER_EVENT_ID,
  HEADER_FORK_OF,
  HEADER_INPUT_CLIENT_ID,
  HEADER_INPUT_CODEC_MESSAGE_ID,
  HEADER_INVOCATION_ID,
  HEADER_MSG_REGENERATE,
  HEADER_PARENT,
  HEADER_ROLE,
  HEADER_RUN_CLIENT_ID,
  HEADER_RUN_ID,
  HEADER_RUN_REASON,
  HEADER_STATUS,
  HEADER_STEER_CODEC_MESSAGE_IDS,
  HEADER_STEP_CLIENT_ID,
  HEADER_STEP_ID,
  HEADER_STEP_REASON,
  HEADER_STEP_START_SERIAL,
  HEADER_STREAM,
  HEADER_STREAM_ID,
} from "./constants.js";
export {
  ErrorCode,
  ErrorInfo,
  errorInfoIs,
  formatErrorMessage,
  statusCodeForErrorCode,
  toErrorInfo,
  type ErrorInfoOptions,
} from "./errors.js";
export {
  EventEmitter,
  type EventEmitterOptions,
  type EventUnsubscribe,
  type EventsMap,
} from "./event-emitter.js";
export {
  LogLevel,
  consoleLogger,
  makeLogger,
  redactValue,
  type LogContext,
  type LogHandler,
  type Logger,
  type MakeLoggerOptions,
} from "./logger.js";
export {
  buildTransportHeaders,
  getCodecHeaders,
  getTransportHeaders,
  headerReader,
  headerWriter,
  mergeHeaders,
  stripUndefined,
  type AiExtras,
  type BuildTransportHeadersOptions,
  type HeaderMap,
} from "./utils.js";
export * from "./realtime/index.js";
export * from "./core/codec/index.js";
export * from "./core/transport/index.js";

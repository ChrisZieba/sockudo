/** Client-to-agent AI input event name. */
export const EVENT_AI_INPUT = "ai-input";
/** Agent-to-client AI output event name. */
export const EVENT_AI_OUTPUT = "ai-output";
/** AI run lifecycle start event name. */
export const EVENT_AI_RUN_START = "ai-run-start";
/** AI run lifecycle suspend event name. */
export const EVENT_AI_RUN_SUSPEND = "ai-run-suspend";
/** AI run lifecycle resume event name. */
export const EVENT_AI_RUN_RESUME = "ai-run-resume";
/** AI run lifecycle end event name. */
export const EVENT_AI_RUN_END = "ai-run-end";
/** AI cancellation event name. */
export const EVENT_AI_CANCEL = "ai-cancel";

/**
 * AI step lifecycle start event name. A step is a re-attemptable unit of
 * execution inside a run; a retry publishes a fresh `ai-step-start` under the
 * same `step-id`, and the highest `step-start-serial` is the canonical attempt.
 */
export const EVENT_AI_STEP_START = "ai-step-start";
/** AI step lifecycle end event name. Closes one attempt of a step. */
export const EVENT_AI_STEP_END = "ai-step-end";

/**
 * Legacy inbound-only AI turn lifecycle event names.
 *
 * Retained for wire tolerance, not naming: channels written before 3.0 may
 * still contain these, and history hydration must not silently drop them. The
 * SDK never publishes them. Deliberately not re-exported from the package
 * barrel — they are not part of the public API.
 */
export const INBOUND_LEGACY_EVENT_TURN_START = "ai-turn-start";
export const INBOUND_LEGACY_EVENT_TURN_END = "ai-turn-end";

/** Transport header key for run identity. */
export const HEADER_RUN_ID = "run-id";
/** Transport header key for verified run client identity. */
export const HEADER_RUN_CLIENT_ID = "run-client-id";
/** Transport header key for run end reason. */
export const HEADER_RUN_REASON = "run-reason";
/**
 * Legacy inbound-only transport header keys.
 *
 * Wire tolerance for pre-3.0 channel history, as with the legacy event names
 * above. Never written. Not part of the public API.
 *
 * `turn-continue` is here rather than deleted because the *data* path still
 * needs the flag; only its name was legacy. See {@link HEADER_RUN_CONTINUE}.
 */
export const INBOUND_LEGACY_HEADER_TURN_ID = "turn-id";
export const INBOUND_LEGACY_HEADER_TURN_CLIENT_ID = "turn-client-id";
export const INBOUND_LEGACY_HEADER_TURN_REASON = "turn-reason";
export const INBOUND_LEGACY_HEADER_TURN_CONTINUE = "turn-continue";

/**
 * Marks a client input as re-entering an existing run.
 *
 * Not redundant with the `ai-run-resume` event name, which covers the lifecycle
 * path only. This SDK mints the run id client-side so optimistic state has an
 * id before the agent replies, which means `run-id` is present on every input
 * and cannot discriminate a continuation on its own. The flag also gates
 * whether `parent`/`fork-of` are re-read, since re-reading them on a
 * continuation would re-anchor the run in the tree.
 */
export const HEADER_RUN_CONTINUE = "run-continue";

/** Transport header key for step identity, stable across retry attempts. */
export const HEADER_STEP_ID = "step-id";
/**
 * Transport header key back-referencing the serial of the `ai-step-start` that
 * opened the attempt. Carried on `ai-output` and `ai-step-end` only — never on
 * `ai-step-start`, whose own channel serial *is* the value.
 */
export const HEADER_STEP_START_SERIAL = "step-start-serial";
/** Transport header key for step end reason: `complete`, `failed`, `cancelled`. */
export const HEADER_STEP_REASON = "step-reason";
/** Transport header key for the verified client identity that owns a step. */
export const HEADER_STEP_CLIENT_ID = "step-client-id";
/**
 * Transport header key stamping which steers the agent had drained when the
 * step attempt that produced this output opened. JSON-stringified array;
 * omitted when empty.
 */
export const HEADER_STEER_CODEC_MESSAGE_IDS = "steer-codec-message-ids";

/** Transport header key for invocation identity. */
export const HEADER_INVOCATION_ID = "invocation-id";
/** Transport header key for input event identity. */
export const HEADER_EVENT_ID = "event-id";
/** Transport header key for codec message identity. */
export const HEADER_CODEC_MESSAGE_ID = "codec-message-id";
/** Transport header key for the input codec message targeted by a cancel signal. */
export const HEADER_INPUT_CODEC_MESSAGE_ID = "input-codec-message-id";
/** Transport header key indicating streaming content. */
export const HEADER_STREAM = "stream";
/** Transport header key for stream identity. */
export const HEADER_STREAM_ID = "stream-id";
/** Transport header key for stream status. */
export const HEADER_STATUS = "status";
/** Transport header key indicating discrete content. */
export const HEADER_DISCRETE = "discrete";
/** Transport header key for message role. */
export const HEADER_ROLE = "role";
/** Transport header key for parent codec message identity. */
export const HEADER_PARENT = "parent";
/** Transport header key for fork source codec message identity. */
export const HEADER_FORK_OF = "fork-of";
/** Transport header key indicating regeneration. */
export const HEADER_MSG_REGENERATE = "msg-regenerate";
/**
 * Transport header key for the suspended run replaced by a client tool-result
 * fork. Superseded runs remain addressable in history but are hidden from
 * branch selection.
 */
export const HEADER_SUPERSEDES = "supersedes";
/** Transport header key for stream error code. */
export const HEADER_ERROR_CODE = "error-code";
/** Transport header key for stream error message. */
export const HEADER_ERROR_MESSAGE = "error-message";
/** Transport header key for verified input client identity. */
export const HEADER_INPUT_CLIENT_ID = "input-client-id";

import type { SendOptions } from "../../core/transport/index.js";
import { ErrorCode, ErrorInfo } from "../../errors.js";
import type { AI, ForkSeed, ToolResult, ToolResultError, VercelInput } from "../codec/index.js";

/** Successful or failed client-side tool resolution. */
export type ToolCallResolution = { output: unknown } | { errorMessage: string };

/** Inputs required to construct a client tool-result fork. */
export interface CreateToolResultForkOptions {
  /** Full message projection of the suspended run. */
  runMessages: readonly AI.UIMessage[];
  /** Codec message id of the suspended run's structural parent. */
  parentCodecMessageId: string;
  /** Tool call being resolved. */
  toolCallId: string;
  /** Successful output or failure message. */
  result: ToolCallResolution;
  /** Suspended run replaced by the fork. */
  supersedesRunId: string;
}

/**
 * Creates a tool resolution and send options for a new assistant reply fork.
 *
 * The fork carries the suspended run's complete projection, allowing both the
 * client and agent reducers to reconstruct prior tool context before applying
 * the new result. The returned send options intentionally omit `runId`, so a
 * new run is created and the suspended trunk can be superseded.
 */
export function createToolResultFork(options: CreateToolResultForkOptions): {
  input: VercelInput;
  sendOptions: SendOptions;
} {
  const target = options.runMessages.find((message) =>
    message.parts.some(
      (part) => part.type === "dynamic-tool" && part.toolCallId === options.toolCallId,
    ),
  );
  if (target === undefined) {
    throw new ErrorInfo({
      code: ErrorCode.InvalidArgument,
      statusCode: 400,
      message: `unable to fork tool result; no run message carries toolCallId ${options.toolCallId}`,
    });
  }

  const forkSeed: ForkSeed = { messages: options.runMessages };
  const input: ToolResult | ToolResultError =
    "errorMessage" in options.result
      ? {
          type: "tool-result-error",
          toolCallId: options.toolCallId,
          message: options.result.errorMessage,
          forkSeed,
        }
      : {
          type: "tool-result",
          toolCallId: options.toolCallId,
          output: options.result.output,
          forkSeed,
        };
  return {
    input,
    sendOptions: {
      role: "assistant",
      parent: options.parentCodecMessageId,
      forkOf: target.id,
      messageId: target.id,
      supersedes: options.supersedesRunId,
    },
  };
}

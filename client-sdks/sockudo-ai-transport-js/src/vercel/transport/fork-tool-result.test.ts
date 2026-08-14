import { describe, expect, it } from "vitest";

import { ErrorCode } from "../../errors.js";
import type { AI } from "../codec/index.js";
import { createToolResultFork } from "./fork-tool-result.js";

describe("createToolResultFork", () => {
  const suspended = {
    id: "assistant-1",
    role: "assistant",
    parts: [
      {
        type: "dynamic-tool",
        toolName: "lookup",
        toolCallId: "tool-1",
        state: "input-available",
        input: { q: "sockudo" },
      },
    ],
  } satisfies AI.UIMessage;

  it("builds a run-less assistant fork with the full suspended projection", () => {
    const fork = createToolResultFork({
      runMessages: [suspended],
      parentCodecMessageId: "user-1",
      toolCallId: "tool-1",
      result: { output: { ok: true } },
      supersedesRunId: "run-trunk",
    });

    expect(fork.input).toEqual({
      type: "tool-result",
      toolCallId: "tool-1",
      output: { ok: true },
      forkSeed: { messages: [suspended] },
    });
    expect(fork.sendOptions).toEqual({
      role: "assistant",
      parent: "user-1",
      forkOf: "assistant-1",
      messageId: "assistant-1",
      supersedes: "run-trunk",
    });
    expect(fork.sendOptions.runId).toBeUndefined();
  });

  it("rejects a tool call that is not owned by the suspended run", () => {
    expect(() =>
      createToolResultFork({
        runMessages: [suspended],
        parentCodecMessageId: "user-1",
        toolCallId: "missing",
        result: { errorMessage: "failed" },
        supersedesRunId: "run-trunk",
      }),
    ).toThrow(expect.objectContaining({ code: ErrorCode.InvalidArgument, statusCode: 400 }));
  });
});

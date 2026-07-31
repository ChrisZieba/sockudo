import { describe, expect, it, vi } from "vitest";

import {
  EVENT_AI_CANCEL,
  EVENT_AI_INPUT,
  EVENT_AI_OUTPUT,
  EVENT_AI_RUN_END,
  EVENT_AI_RUN_START,
  HEADER_CODEC_MESSAGE_ID,
  HEADER_INVOCATION_ID,
  HEADER_STATUS,
  HEADER_STREAM,
  HEADER_RUN_CONTINUE,
  HEADER_RUN_ID,
  HEADER_RUN_REASON,
  HEADER_STEER_CODEC_MESSAGE_IDS,
} from "../../constants.js";
import { ErrorCode } from "../../errors.js";
import type { ErrorInfo } from "../../errors.js";
import { createMockClient, MockChannel } from "../../realtime/mocks.js";
import { getTransportHeaders } from "../../utils.js";
import type { SockudoRawMessage } from "../../realtime/adapter.js";
import type { PublishMessage } from "../../realtime/types.js";
import type { Codec, DecodedBatch, Decoder, ReducerMeta, UserMessage } from "../codec/index.js";
import { createClientSession } from "./client-session.js";
import type { ClientRun } from "./client-session.js";
import type { InvocationIdProvider } from "./invocation.js";

describe("client transport", () => {
  it("publishes input before poking and sends locked invocation fields", async () => {
    const calls: string[] = [];
    const { channel, fetch, ids } = setupCalls(calls);
    const session = createClientSession({
      channel,
      codec: testCodec(),
      api: "https://agent.test/run",
      clientId: "client-1",
      idProvider: ids,
      runStartDeadlineMs: 0,
      fetch,
      body: () => ({ runId: "bad", custom: "ok" }),
      headers: () => ({ Authorization: "secret" }),
    });

    const active = (await session.view.send({
      id: "user-1",
      text: "hello",
    })) as ClientRun<Message, Message>;

    expect(active).toMatchObject({
      runId: "turn-1",
      invocationId: "inv-1",
      inputEventId: "evt-1",
      optimisticMsgIds: ["user-1"],
    });
    expect(calls).toEqual(["publish:ai-input", "fetch"]);
    const request = fetch.mock.calls[0]?.[1] as RequestInit;
    expect(requestBodyJson(request)).toMatchObject({
      custom: "ok",
      runId: "turn-1",
      invocationId: "inv-1",
      inputEventId: "evt-1",
      sessionName: "chat",
      clientId: "client-1",
      messages: [{ id: "user-1", text: "hello" }],
    });
    expect(request.headers).toMatchObject({
      "Content-Type": "application/json",
      Authorization: "secret",
    });
  });

  it("steers a live run and settles the outcome from the agent's stamp", async () => {
    const client = createMockClient({ clientId: "client-1" });
    const channel = client.getMockChannel("chat");
    const publishes: PublishMessage[] = [];
    const inner = MockChannel.prototype.publish.bind(channel);
    vi.spyOn(channel, "publish").mockImplementation((message) => {
      publishes.push(message);
      return inner(message);
    });
    const fetch = okFetch();
    const ids = fixedIds();
    const session = createClientSession({
      channel,
      codec: testCodec(),
      api: "https://agent.test/run",
      clientId: "client-1",
      idProvider: ids,
      runStartDeadlineMs: 0,
      fetch,
    });

    const run = (await session.view.send({
      id: "user-1",
      text: "hello",
    })) as ClientRun<Message, Message>;

    const steer = run.steer({ id: "user-2", text: "actually, in French" });
    const ack = await steer.published;
    expect(ack.serial).toBeDefined();

    // The steer rides ai-input against the existing run, not a new run.
    const steerPublish = publishes.find(
      (message) => message.name === EVENT_AI_INPUT && message.messageId !== "user-1",
    );
    expect(getTransportHeaders(steerPublish?.extras)).toMatchObject({
      [HEADER_RUN_ID]: run.runId,
      [HEADER_RUN_CONTINUE]: "true",
    });

    const steerId = steerPublish?.messageId ?? "";

    // The agent stamps it onto an output, then ends the run.
    channel.inject(stampedOutput("turn-1", "inv-1", "assistant-1", 3, [steerId]));
    channel.inject(lifecycle(EVENT_AI_RUN_END, "turn-1", "inv-1", 4, "complete"));

    await expect(steer.outcome).resolves.toEqual({
      consumed: true,
      runTerminalReason: "complete",
    });
  });

  it("reports a steer the agent never drained as not consumed", async () => {
    const calls: string[] = [];
    const { channel, fetch, ids } = setupCalls(calls);
    const session = createClientSession({
      channel,
      codec: testCodec(),
      api: "https://agent.test/run",
      clientId: "client-1",
      idProvider: ids,
      runStartDeadlineMs: 0,
      fetch,
    });

    const run = (await session.view.send({
      id: "user-1",
      text: "hello",
    })) as ClientRun<Message, Message>;
    const steer = run.steer({ id: "user-2", text: "too late" });
    await steer.published;

    // Output carrying no stamp at all, then a cancel.
    channel.inject(output("turn-1", "inv-1", "assistant-1", "token", 3));
    channel.inject(lifecycle(EVENT_AI_RUN_END, "turn-1", "inv-1", 4, "cancelled"));

    await expect(steer.outcome).resolves.toEqual({
      consumed: false,
      runTerminalReason: "cancelled",
    });
  });

  it("rolls back optimistic messages on publish failure", async () => {
    const client = createMockClient({ clientId: "client-1" });
    const channel = client.getMockChannel("chat");
    vi.spyOn(channel, "publish").mockRejectedValueOnce({
      status: 403,
      data: { code: 403, error: "forbidden" },
    });
    const session = createClientSession({
      channel,
      codec: testCodec(),
      api: "https://agent.test/run",
      idProvider: fixedIds(),
      runStartDeadlineMs: 0,
      fetch: okFetch(),
    });

    await expect(session.view.send({ id: "user-1", text: "hello" })).rejects.toMatchObject({
      code: ErrorCode.InsufficientCapability,
    });
    expect(session.view.getMessages()).toEqual([]);
  });

  it("rejects with a clear error when server feature flags omit ai-transport", () => {
    const logs: string[] = [];
    const client = createMockClient({ clientId: "client-1" });
    Object.assign(client.connection, {
      features: ["history", "versioned-messages"],
    });

    expect(() =>
      createClientSession({
        client,
        channelName: "chat",
        codec: testCodec(),
        api: "https://agent.test/run",
        idProvider: fixedIds(),
        fetch: okFetch(),
        logger: {
          trace: () => undefined,
          debug: () => undefined,
          info: () => undefined,
          warn: () => undefined,
          error: (message) => logs.push(message),
          withContext() {
            return this;
          },
        },
      }),
    ).toThrow(
      expect.objectContaining({
        code: ErrorCode.ChannelNotReady,
        statusCode: 501,
        message:
          "unable to create client transport; Sockudo server does not advertise the ai-transport feature",
      }),
    );
    expect(logs).toEqual([
      "unable to create client transport; Sockudo server does not advertise the ai-transport feature",
    ]);
  });

  it("accepts clients that advertise ai-transport or cannot expose feature flags yet", () => {
    expect(() =>
      createClientSession({
        client: withConnectionFeatures(createMockClient({ clientId: "client-1" }), [
          "ai-transport",
        ]),
        channelName: "chat",
        codec: testCodec(),
        api: "https://agent.test/run",
        idProvider: fixedIds(),
        fetch: okFetch(),
      }),
    ).not.toThrow();
    expect(() =>
      createClientSession({
        client: createMockClient({ clientId: "client-1" }),
        channelName: "chat",
        codec: testCodec(),
        api: "https://agent.test/run",
        idProvider: fixedIds(),
        fetch: okFetch(),
      }),
    ).not.toThrow();
  });

  it("leaves optimistic state and errors the stream when POST fails", async () => {
    const client = createMockClient({ clientId: "client-1" });
    const channel = client.getMockChannel("chat");
    const errors: ErrorInfo[] = [];
    const session = createClientSession({
      channel,
      codec: testCodec(),
      api: "https://agent.test/run",
      idProvider: fixedIds(),
      runStartDeadlineMs: 0,
      fetch: vi.fn(() => Promise.resolve(new Response(null, { status: 500 }))),
    });
    session.on("error", (error) => errors.push(error));

    const active = (await session.view.send({
      id: "user-1",
      text: "hello",
    })) as ClientRun<Message, Message>;
    await Promise.resolve();

    expect(session.view.getMessages()).toEqual([{ id: "user-1", text: "hello" }]);
    expect(errors).toHaveLength(1);
    await expect(active.stream.getReader().read()).rejects.toMatchObject({
      code: ErrorCode.SessionSendFailed,
    });
  });

  it("waits for matching turn-start and routes output chunks", async () => {
    const client = createMockClient({ clientId: "client-1" });
    const channel = client.getMockChannel("chat");
    const session = createClientSession({
      channel,
      codec: testCodec(),
      api: "https://agent.test/run",
      clientId: "client-1",
      idProvider: fixedIds(),
      fetch: okFetch(),
      runStartDeadlineMs: 100,
    });

    const pending = session.view.send({ id: "user-1", text: "hello" });
    queueMicrotask(() => {
      channel.inject(lifecycle(EVENT_AI_RUN_START, "turn-1", "inv-1", 2));
    });
    const active = (await pending) as ClientRun<Message, Message>;
    channel.inject(output("turn-1", "inv-1", "assistant-1", "token", 3));

    await expect(active.stream.getReader().read()).resolves.toEqual({
      done: false,
      value: { id: "assistant-1", text: "token" },
    });
  });

  it("does not fold future summary actions into reduced state", () => {
    const client = createMockClient({ clientId: "client-1" });
    const channel = client.getMockChannel("chat");
    const session = createClientSession({
      channel,
      codec: testCodec(),
      api: "https://agent.test/run",
      clientId: "client-1",
      idProvider: fixedIds(),
      fetch: okFetch(),
      runStartDeadlineMs: 0,
    });
    const rawMessages: string[] = [];
    session.on("message", (message) => {
      rawMessages.push(`${message.name}:${message.action}`);
    });

    channel.inject(output("turn-1", "inv-1", "assistant-1", "before", 3));
    expect(session.view.getMessages()).toEqual([{ id: "assistant-1", text: "before" }]);

    channel.inject({
      event: "sockudo:message.future",
      name: EVENT_AI_OUTPUT,
      channel: "chat",
      data: { id: "assistant-1", text: "corrupt" },
      action: "future",
      message_serial: "assistant-1",
      history_serial: "9007199254740993",
      delivery_serial: "9007199254740994",
      extras: {
        ai: {
          transport: {
            [HEADER_RUN_ID]: "turn-1",
            [HEADER_INVOCATION_ID]: "inv-1",
            [HEADER_CODEC_MESSAGE_ID]: "assistant-1",
          },
        },
      },
    });
    expect(rawMessages).toContain(`${EVENT_AI_OUTPUT}:summary`);
    expect(session.view.getMessages()).toEqual([{ id: "assistant-1", text: "before" }]);

    channel.inject(output("turn-1", "inv-1", "assistant-2", "after", 4));
    expect(session.view.getMessages()).toEqual([
      { id: "assistant-1", text: "before" },
      { id: "assistant-2", text: "after" },
    ]);
  });

  it("rejects when turn-start misses the configured deadline", async () => {
    const client = createMockClient({ clientId: "client-1" });
    const session = createClientSession({
      channel: client.getMockChannel("chat"),
      codec: testCodec(),
      api: "https://agent.test/run",
      idProvider: fixedIds(),
      fetch: okFetch(),
      runStartDeadlineMs: 1,
    });

    await expect(session.view.send({ id: "user-1", text: "hello" })).rejects.toMatchObject({
      code: ErrorCode.RunStartDeadlineExceeded,
      statusCode: 504,
    });
  });

  it("publishes cancel filters and waitForRun resolves on terminal turn-end", async () => {
    const client = createMockClient({ clientId: "client-1" });
    const channel = client.getMockChannel("chat");
    const session = createClientSession({
      channel,
      codec: testCodec(),
      api: "https://agent.test/run",
      clientId: "client-1",
      idProvider: fixedIds(),
      fetch: okFetch(),
      runStartDeadlineMs: 0,
    });
    const published: unknown[] = [];
    const originalPublish = channel.publish.bind(channel);
    vi.spyOn(channel, "publish").mockImplementation((message) => {
      published.push(message);
      return originalPublish(message);
    });

    const active = (await session.view.send({
      id: "user-1",
      text: "hello",
    })) as ClientRun<Message, Message>;
    const waiting = session.waitForRun({ own: true });
    await active.cancel();
    channel.inject(lifecycle(EVENT_AI_RUN_END, "turn-1", "inv-1", 3, "cancelled"));

    await expect(waiting).resolves.toBeUndefined();
    expect(published).toContainEqual(
      expect.objectContaining({
        name: EVENT_AI_CANCEL,
        data: { runId: "turn-1" },
      }),
    );
  });

  it("stages events locally and drains them into the next send body", async () => {
    const client = createMockClient({ clientId: "client-1" });
    const fetch = okFetch();
    const session = createClientSession({
      channel: client.getMockChannel("chat"),
      codec: testCodec(),
      api: "https://agent.test/run",
      idProvider: fixedIds(),
      fetch,
      runStartDeadlineMs: 0,
      messages: [{ id: "assistant-1", text: "old" }],
    });

    session.stageEvents("assistant-1", [{ id: "assistant-1", text: "new" }]);
    expect(session.view.getMessages()).toEqual([{ id: "assistant-1", text: "new" }]);

    await session.view.send({ id: "user-1", text: "next" });

    const request = fetch.mock.calls[0]?.[1] as RequestInit;
    expect(requestBodyJson(request)).toMatchObject({
      events: [
        {
          msgId: "assistant-1",
          events: [{ id: "assistant-1", text: "new" }],
        },
      ],
    });
  });
});

function setupCalls(calls: string[]): {
  channel: MockChannel;
  fetch: ReturnType<typeof okFetch>;
  ids: InvocationIdProvider;
} {
  const client = createMockClient({ clientId: "client-1" });
  const channel = client.getMockChannel("chat");
  const originalPublish = channel.publish.bind(channel);
  vi.spyOn(channel, "publish").mockImplementation((message) => {
    calls.push(`publish:${message.name ?? ""}`);
    return originalPublish(message);
  });
  const fetch = vi.fn(() => {
    calls.push("fetch");
    return Promise.resolve(new Response(null, { status: 200 }));
  }) as ReturnType<typeof okFetch>;
  return { channel, fetch, ids: fixedIds() };
}

function okFetch(): ReturnType<typeof vi.fn> & typeof fetch {
  return vi.fn(() => Promise.resolve(new Response(null, { status: 200 }))) as ReturnType<
    typeof vi.fn
  > &
    typeof fetch;
}

function requestBodyJson(request: RequestInit): unknown {
  if (typeof request.body !== "string") {
    throw new TypeError("expected string request body");
  }
  return JSON.parse(request.body) as unknown;
}

function fixedIds(): InvocationIdProvider {
  let turn = 0;
  let invocation = 0;
  let event = 0;
  let message = 0;
  return {
    runId: () => `turn-${String(++turn)}`,
    invocationId: () => `inv-${String(++invocation)}`,
    inputEventId: () => `evt-${String(++event)}`,
    messageId: () => `msg-${String(++message)}`,
  };
}

function lifecycle(
  name: typeof EVENT_AI_RUN_START | typeof EVENT_AI_RUN_END,
  runId: string,
  invocationId: string,
  serial: number,
  reason?: "complete" | "cancelled" | "error" | "suspended",
): SockudoRawMessage {
  return {
    event: "sockudo:message.create",
    name,
    channel: "chat",
    data: {},
    message_serial: `${name}-${String(serial)}`,
    history_serial: serial,
    delivery_serial: serial,
    extras: {
      ai: {
        transport: {
          [HEADER_RUN_ID]: runId,
          [HEADER_INVOCATION_ID]: invocationId,
          ...(reason !== undefined ? { [HEADER_RUN_REASON]: reason } : {}),
        },
      },
    },
  };
}

function output(
  runId: string,
  invocationId: string,
  messageId: string,
  text: string,
  serial: number,
): SockudoRawMessage {
  return {
    event: "sockudo:message.create",
    name: EVENT_AI_OUTPUT,
    channel: "chat",
    data: { id: messageId, text },
    message_serial: messageId,
    history_serial: serial,
    delivery_serial: serial,
    extras: {
      ai: {
        transport: {
          [HEADER_RUN_ID]: runId,
          [HEADER_INVOCATION_ID]: invocationId,
          [HEADER_CODEC_MESSAGE_ID]: messageId,
          [HEADER_STREAM]: "true",
          [HEADER_STATUS]: "streaming",
        },
      },
    },
  };
}

function withConnectionFeatures(
  client: ReturnType<typeof createMockClient>,
  features: readonly string[],
): ReturnType<typeof createMockClient> {
  Object.assign(client.connection, { features });
  return client;
}

function testCodec(): Codec<Message, Message, Projection, Message> {
  return {
    init: () => ({ messages: [] }),
    fold(projection, event) {
      const index = projection.messages.findIndex((message) => message.id === event.id);
      if (index === -1) {
        projection.messages.push(event);
      } else {
        projection.messages[index] = event;
      }
      return projection;
    },
    getMessages: (projection) => projection.messages,
    createUserMessage(message): UserMessage<Message> {
      return { message };
    },
    createRegenerate(target, parent) {
      return { target, parent };
    },
    resolveToolTarget: () => undefined,
    isTerminal: (output) => output.done === true,
    createEncoder() {
      throw new Error("not used");
    },
    createDecoder(): Decoder<Message, Message> {
      return {
        decode(message): DecodedBatch<Message, Message> {
          const data = message.data as Message;
          const decoded = {
            event: data,
            messageId:
              message.getTransportHeaders()[HEADER_CODEC_MESSAGE_ID] ?? message.messageSerial,
            meta: {
              serial: message.deliverySerial ?? message.historySerial,
              messageId:
                message.getTransportHeaders()[HEADER_CODEC_MESSAGE_ID] ?? message.messageSerial,
            } satisfies ReducerMeta,
          };
          if (message.name === EVENT_AI_OUTPUT) {
            return { inputs: [], outputs: [decoded] };
          }
          return { inputs: [decoded], outputs: [] };
        },
      };
    },
  };
}

interface Message {
  id: string;
  text: string;
  done?: boolean;
}

interface Projection {
  messages: Message[];
}

function stampedOutput(
  runId: string,
  invocationId: string,
  messageId: string,
  serial: number,
  steerIds: readonly string[],
): SockudoRawMessage {
  const base = output(runId, invocationId, messageId, "token", serial);
  const transport = getTransportHeaders(base.extras);
  return {
    ...base,
    extras: {
      ai: {
        transport: {
          ...transport,
          [HEADER_STEER_CODEC_MESSAGE_IDS]: JSON.stringify(steerIds),
        },
      },
    },
  };
}

import {
  EVENT_AI_CANCEL,
  EVENT_AI_INPUT,
  INBOUND_LEGACY_EVENT_TURN_END,
  INBOUND_LEGACY_EVENT_TURN_START,
  EVENT_AI_RUN_END,
  EVENT_AI_RUN_RESUME,
  EVENT_AI_RUN_START,
  EVENT_AI_RUN_SUSPEND,
  EVENT_AI_STEP_END,
  EVENT_AI_STEP_START,
  HEADER_INVOCATION_ID,
  HEADER_RUN_ID,
  HEADER_RUN_REASON,
} from "../../constants.js";
import { ErrorCode, ErrorInfo, toErrorInfo } from "../../errors.js";
import { EventEmitter, type EventUnsubscribe } from "../../event-emitter.js";
import { LogLevel, makeLogger, type Logger } from "../../logger.js";
import type { ChannelLike, ClientLike, InboundMessage } from "../../realtime/index.js";
import {
  buildTransportHeaders,
  mergeHeaders,
  stripUndefined,
  type HeaderMap,
} from "../../utils.js";
import type { Codec, DecodedBatch, DecodedEvent } from "../codec/index.js";
import { createTree, type Tree } from "./tree.js";
import { createView, type View, type ViewSendExecutor } from "./view.js";
import { createDefaultInvocationIdProvider, type InvocationIdProvider } from "./invocation.js";
import { createStreamRouter, type StreamRouter } from "./stream-router.js";
import { SteerCoordinator, type SteerResult } from "./steer.js";

/**
 * Cancellation scope for client-side turn cancellation.
 *
 * @defaultValue `cancel()` and `waitForRun()` default to `{ own: true }`.
 */
export type CancelFilter =
  | { runId: string; own?: never; clientId?: never; all?: never }
  | { own: boolean; runId?: never; clientId?: never; all?: never }
  | { clientId: string; runId?: never; own?: never; all?: never }
  | { all: boolean; runId?: never; own?: never; clientId?: never };

/**
 * Per-send options for HTTP body/header merging and branch metadata.
 */
export interface SendOptions {
  /** Additional POST body fields. Invocation fields always win. */
  body?: Record<string, unknown>;
  /** Additional POST headers. */
  headers?: Record<string, string>;
  /** Return the active stream immediately instead of waiting for `ai-run-start`.
   *
   * @defaultValue `false`.
   */
  waitForRunStart?: boolean;
  /** Existing run id for suspended-run continuation. */
  runId?: string;
  /** Role stamped on the input. Tool-result forks use `assistant`. */
  role?: "user" | "assistant";
  /** Suspended run replaced by this client tool-result fork. */
  supersedes?: string;
  /** Message id this send replaces. */
  forkOf?: string;
  /** Parent message id. Defaults to the selected branch tail. */
  parent?: string;
  /** Trigger label for the default POST body.
   *
   * @defaultValue `"message"`
   */
  trigger?: string;
  /** Message id associated with edit/regenerate requests. */
  messageId?: string;
}

/**
 * A handle to an active client-side turn.
 */
export interface ClientRun<TInput, TOutput> {
  /** Decoded output stream for this run. */
  stream: ReadableStream<TOutput>;
  /** Run identity. */
  runId: string;
  /** Invocation identity for this send or continuation. */
  invocationId: string;
  /** Primary input event id. */
  inputEventId: string;
  /** Cancels this run and closes the local stream. */
  cancel(): Promise<void>;
  /**
   * Sends additional input into this already-running run.
   *
   * Unlike a fresh send this neither cancels the run nor starts a new one: the
   * agent's loop picks the input up at its next step boundary. Returns two
   * promises — `published` for the publish itself, `outcome` for whether the
   * agent's loop had the steer visible before the run reached a terminal event.
   */
  steer(input: TInput): SteerResult;
  /** Optimistically inserted codec message ids. */
  optimisticMsgIds: readonly string[];
}

/**
 * Options for closing a client session.
 */
export interface CloseOptions {
  /** Optional cancel filter to publish before local teardown. */
  cancel?: CancelFilter;
}

/**
 * Client session event map.
 */
export interface ClientSessionEvents {
  /** Non-fatal transport error. */
  error: ErrorInfo;
  /** Raw normalized inbound channel message before codec folding. */
  message: InboundMessage;
}

/**
 * Options for creating a client session.
 */
export interface ClientSessionOptions<TInput, TOutput, TProjection, TMessage> {
  /** Realtime client used with `channelName`. */
  client?: ClientLike;
  /** Realtime channel. */
  channel?: ChannelLike;
  /** Channel name used when `client` is supplied.
   *
   * @defaultValue `channel.name` when `channel` is supplied.
   */
  channelName?: string;
  /** Domain codec. */
  codec: Codec<TInput, TOutput, TProjection, TMessage>;
  /** Server endpoint URL for the HTTP poke. */
  api: string;
  /** Verified client identity.
   *
   * @defaultValue `client.connection.clientId` when available.
   */
  clientId?: string;
  /** Static or per-send POST headers. */
  headers?: Record<string, string> | (() => Record<string, string>);
  /** Static or per-send POST body fields. */
  body?: Record<string, unknown> | (() => Record<string, unknown>);
  /** Fetch credentials mode. */
  credentials?: RequestCredentials;
  /** Fetch implementation.
   *
   * @defaultValue `globalThis.fetch`.
   */
  fetch?: typeof globalThis.fetch;
  /** Initial messages seeded as a linear chain. */
  messages?: readonly TMessage[];
  /** Logger.
   *
   * @defaultValue Silent SDK logger.
   */
  logger?: Logger;
  /** Run-start wait deadline in milliseconds.
   *
   * @defaultValue `30000`.
   */
  runStartDeadlineMs?: number;
  /** Deterministic id provider for tests. */
  idProvider?: InvocationIdProvider;
  /** Maximum queued stream chunks before local stream error.
   *
   * @defaultValue `1024`.
   */
  streamQueueLimit?: number;
}

/**
 * Client-side transport that owns tree, views, sends, streams, and cancellation.
 */
export interface ClientSession<TInput, TOutput, TProjection, TMessage> {
  /** Complete conversation tree. */
  readonly tree: Tree<TInput | TOutput, TProjection>;
  /** Default branch-aware view. */
  readonly view: View<TInput, TMessage>;
  /** Creates an additional branch-aware view. */
  createView(): View<TInput, TMessage>;
  /** Cancels turns matching a filter. Rejects with {@link ErrorInfo} on publish failure. */
  cancel(filter?: CancelFilter): Promise<void>;
  /** Resolves when matching active turns complete. */
  waitForRun(filter?: CancelFilter): Promise<void>;
  /** Locally applies events and queues them for the next send POST body. */
  stageEvents(msgId: string, events: readonly TOutput[]): void;
  /** Locally replaces a message projection while preserving known headers/serial. */
  stageMessage(msgId: string, message: TMessage): void;
  /** Subscribes to transport events. */
  on<K extends keyof ClientSessionEvents>(
    event: K,
    handler: (payload: ClientSessionEvents[K]) => void,
  ): EventUnsubscribe;
  /** Closes local resources, optionally publishing cancel first. */
  close(options?: CloseOptions): Promise<void>;
}

/**
 * Creates a Sockudo client session.
 *
 * Async public methods reject with {@link ErrorInfo}. Synchronous misuse throws
 * {@link ErrorInfo} with {@link ErrorCode.InvalidArgument} or
 * {@link ErrorCode.SessionClosed}.
 */
export function createClientSession<TInput, TOutput, TProjection, TMessage>(
  options: ClientSessionOptions<TInput, TOutput, TProjection, TMessage>,
): ClientSession<TInput, TOutput, TProjection, TMessage> {
  return new DefaultClientSession(options);
}

interface PendingRunStart<TInput, TOutput> {
  invocationId: string;
  resolve(turn: ClientRun<TInput, TOutput>): void;
  reject(error: ErrorInfo): void;
  timer: ReturnType<typeof setTimeout> | undefined;
  turn: ClientRun<TInput, TOutput>;
}

interface OwnRun {
  clientId: string;
  invocationId: string;
}

interface StagedEvents<TOutput> {
  msgId: string;
  events: readonly TOutput[];
}

class DefaultClientSession<TInput, TOutput, TProjection, TMessage> implements ClientSession<
  TInput,
  TOutput,
  TProjection,
  TMessage
> {
  private readonly channel: ChannelLike;
  private readonly clientId: string | undefined;
  private readonly logger: Logger;
  private readonly idProvider: InvocationIdProvider;
  private readonly decoder;
  private readonly router: StreamRouter<TOutput>;
  private readonly steer = new SteerCoordinator();
  private readonly emitter = new EventEmitter<ClientSessionEvents>();
  private readonly views = new Set<View<TInput, TMessage>>();
  private readonly ownRuns = new Map<string, OwnRun>();
  private readonly pendingRunStarts = new Map<string, PendingRunStart<TInput, TOutput>>();
  private readonly closeResolvers = new Set<() => void>();
  private readonly unsubscribes: EventUnsubscribe[] = [];
  private readonly runStartDeadlineMs: number;
  private readonly fetchFn: typeof globalThis.fetch;
  private readonly headerProvider: () => Record<string, string>;
  private readonly bodyProvider: () => Record<string, unknown>;
  private stagedEvents: StagedEvents<TOutput>[] = [];
  private closed = false;
  private connected = false;

  public readonly tree: Tree<TInput | TOutput, TProjection>;
  public readonly view: View<TInput, TMessage>;

  public constructor(
    private readonly options: ClientSessionOptions<TInput, TOutput, TProjection, TMessage>,
  ) {
    if (!options.channel && !options.client) {
      throw new ErrorInfo({
        code: ErrorCode.InvalidArgument,
        message: "unable to create client transport; channel or client is required",
      });
    }
    if (options.client && !options.channel && !options.channelName) {
      throw new ErrorInfo({
        code: ErrorCode.InvalidArgument,
        message: "unable to create client transport; channelName is required with client",
      });
    }
    this.channel =
      options.channel ??
      options.client?.channels.get(options.channelName ?? "") ??
      missingChannel();
    this.clientId = options.clientId ?? options.client?.connection.clientId;
    this.logger = (options.logger ?? makeLogger({ logLevel: LogLevel.Silent })).withContext({
      component: "ClientSession",
    });
    assertAiTransportFeature(readConnectionFeatures(options.client), this.logger);
    this.idProvider = options.idProvider ?? createDefaultInvocationIdProvider();
    this.decoder = options.codec.createDecoder();
    this.fetchFn = options.fetch ?? globalThis.fetch.bind(globalThis);
    this.runStartDeadlineMs = options.runStartDeadlineMs ?? 30_000;
    this.headerProvider = normalizeProvider(options.headers);
    this.bodyProvider = normalizeProvider(options.body);
    this.tree = createTree<TInput | TOutput, TProjection>(options.codec);
    this.router = createStreamRouter<TOutput>({
      isTerminal: (output) => options.codec.isTerminal(output),
      ...(options.streamQueueLimit !== undefined
        ? { maxQueuedChunks: options.streamQueueLimit }
        : {}),
    });
    const executor = this.createSendExecutor();
    this.view = createView({
      tree: this.tree,
      codec: options.codec,
      decoder: this.decoder,
      history: this.channel,
      sendExecutor: executor,
    });
    this.views.add(this.view);
    this.seedMessages(options.messages ?? []);
  }

  public createView(): View<TInput, TMessage> {
    this.assertOpen("create view");
    const view = createView({
      tree: this.tree,
      codec: this.options.codec,
      decoder: this.decoder,
      history: this.channel,
      sendExecutor: this.createSendExecutor(),
    });
    this.views.add(view);
    return view;
  }

  public async cancel(filter: CancelFilter = { own: true }): Promise<void> {
    if (this.closed) {
      return;
    }
    try {
      await this.channel.publish({
        name: EVENT_AI_CANCEL,
        data: filter,
        extras: {
          ai: {
            transport: cancelHeaders(filter, this.clientId),
          },
        },
      });
      this.closeMatchingRunStreams(filter);
    } catch (error) {
      throw toErrorInfo(error, {
        code: ErrorCode.SessionSendFailed,
        message: "unable to cancel turns; cancel publish failed",
      });
    }
  }

  public waitForRun(filter: CancelFilter = { own: true }): Promise<void> {
    if (this.closed) {
      return Promise.resolve();
    }
    const remaining = this.getMatchingRunIds(filter);
    if (remaining.size === 0) {
      return Promise.resolve();
    }
    return new Promise((resolve) => {
      const done = (): void => {
        unsubscribe();
        this.closeResolvers.delete(done);
        resolve();
      };
      const unsubscribe = this.tree.on("turn", (node) => {
        if (node.status === "active" || node.status === "suspended") {
          return;
        }
        remaining.delete(node.runId);
        if (remaining.size === 0) {
          done();
        }
      });
      this.closeResolvers.add(done);
    });
  }

  public stageEvents(msgId: string, events: readonly TOutput[]): void {
    if (this.closed || events.length === 0) {
      return;
    }
    const headers = this.tree.getHeaders(msgId);
    const node = this.tree.getNodeByCodecMessageId(msgId);
    if (!headers || !node) {
      this.logger.warn("stageEvents ignored unknown message id", { msgId });
      return;
    }
    const decoded = events.map((event, index) =>
      decodedEvent<TInput | TOutput>(
        event,
        msgId,
        `${String(node.startSerial ?? "local")}:stage:${String(index)}`,
      ),
    );
    this.tree.applyMessage(decoded, headers, node.startSerial ?? "local");
    this.stagedEvents.push({ msgId, events: [...events] });
  }

  public stageMessage(msgId: string, message: TMessage): void {
    if (this.closed) {
      return;
    }
    const headers = this.tree.getHeaders(msgId);
    const node = this.tree.getNodeByCodecMessageId(msgId);
    if (!headers || !node) {
      this.logger.warn("stageMessage ignored unknown message id", { msgId });
      return;
    }
    this.tree.applyMessage(
      [decodedEvent(message as unknown as TInput | TOutput, msgId, node.startSerial ?? "local")],
      headers,
      node.startSerial ?? "local",
    );
  }

  public on<K extends keyof ClientSessionEvents>(
    event: K,
    handler: (payload: ClientSessionEvents[K]) => void,
  ): EventUnsubscribe {
    if (this.closed) {
      return () => undefined;
    }
    if (event === "message") {
      this.connect();
    }
    return this.emitter.on(event, handler);
  }

  public async close(options: CloseOptions = {}): Promise<void> {
    if (this.closed) {
      return;
    }
    if (options.cancel) {
      try {
        await this.cancel(options.cancel);
      } catch {
        // Best-effort shutdown path.
      }
    }
    this.closed = true;
    for (const pending of this.pendingRunStarts.values()) {
      if (pending.timer) {
        clearTimeout(pending.timer);
      }
      pending.reject(
        new ErrorInfo({
          code: ErrorCode.SessionClosed,
          statusCode: 400,
          message: "unable to wait for turn start; transport is closed",
        }),
      );
    }
    this.pendingRunStarts.clear();
    this.router.closeAll();
    for (const view of this.views) {
      view.close();
    }
    this.views.clear();
    for (const unsubscribe of this.unsubscribes) {
      unsubscribe();
    }
    this.unsubscribes.length = 0;
    for (const resolve of Array.from(this.closeResolvers)) {
      resolve();
    }
  }

  private createSendExecutor(): ViewSendExecutor<TInput, TMessage> {
    return {
      send: (message, options = {}) => this.internalSend([message], options),
      sendInput: (input, options = {}) => this.internalSendInput(normalizeInputs(input), options),
      regenerate: (target, parent, options = {}) =>
        this.internalSendInput([this.options.codec.createRegenerate(target, parent) as TInput], {
          ...options,
          forkOf: target,
          parent,
          messageId: target,
          trigger: "regenerate",
        }),
      edit: (messageId, message, options = {}) =>
        this.internalSend([message], {
          ...options,
          forkOf: messageId,
          messageId,
          trigger: "edit",
        }),
      update: (messageId, patch, options = {}) => {
        const events = Array.isArray(patch)
          ? (patch as readonly TOutput[])
          : ([patch] as readonly unknown[] as readonly TOutput[]);
        this.stageEvents(messageId, events);
        return this.internalSend([], {
          ...options,
          messageId,
          trigger: "update",
        });
      },
    };
  }

  private async internalSend(
    messages: readonly TMessage[],
    sendOptions: SendOptions,
  ): Promise<ClientRun<TInput, TOutput>> {
    const inputs = messages.map((message) => this.options.codec.createUserMessage(message).message);
    return this.sendPipeline(
      inputs.map((message) => message as unknown as TInput),
      messages,
      sendOptions,
    );
  }

  private internalSendInput(
    inputs: readonly TInput[],
    sendOptions: SendOptions,
  ): Promise<ClientRun<TInput, TOutput>> {
    return this.sendPipeline(inputs, [], sendOptions);
  }

  private async sendPipeline(
    inputs: readonly TInput[],
    messages: readonly TMessage[],
    sendOptions: SendOptions,
  ): Promise<ClientRun<TInput, TOutput>> {
    this.assertOpen("send");
    this.connect();

    const runId = sendOptions.runId ?? this.idProvider.runId();
    const invocationId = this.idProvider.invocationId();
    const eventIds = inputs.map(() => this.idProvider.inputEventId());
    const inputEventId = eventIds[eventIds.length - 1] ?? this.idProvider.inputEventId();
    const parent = sendOptions.parent ?? this.currentParent();
    const isContinuation = sendOptions.runId !== undefined;
    const regenerateOf =
      sendOptions.trigger === "regenerate"
        ? (sendOptions.messageId ?? sendOptions.forkOf)
        : undefined;
    const wireForkOf = sendOptions.trigger === "regenerate" ? undefined : sendOptions.forkOf;
    const optimisticMsgIds: string[] = [];
    const staged = this.stagedEvents;
    this.stagedEvents = [];

    for (let index = 0; index < messages.length; index += 1) {
      const message = messages[index];
      if (message === undefined) {
        continue;
      }
      const messageId = messageIdOf(message) ?? this.idProvider.messageId();
      optimisticMsgIds.push(messageId);
      const headers = this.inputHeaders({
        runId,
        ...(sendOptions.role !== undefined ? { role: sendOptions.role } : {}),
        ...(sendOptions.supersedes !== undefined ? { supersedes: sendOptions.supersedes } : {}),
        invocationId,
        inputEventId: eventIds[index] ?? inputEventId,
        codecMessageId: messageId,
        ...(index === 0
          ? parent !== undefined
            ? { parent }
            : {}
          : optimisticMsgIds[index - 1] !== undefined
            ? { parent: optimisticMsgIds[index - 1] }
            : {}),
        ...(wireForkOf !== undefined ? { forkOf: wireForkOf } : {}),
        runContinue: isContinuation,
        ...(regenerateOf !== undefined ? { regenerates: regenerateOf } : {}),
      });
      this.tree.applyMessage(
        [
          decodedEvent<TInput | TOutput>(
            message as unknown as TInput | TOutput,
            messageId,
            "optimistic",
          ),
        ],
        headers,
        "optimistic",
      );
    }

    try {
      for (let index = 0; index < inputs.length; index += 1) {
        const input = inputs[index];
        if (input === undefined) {
          continue;
        }
        const inputCodecMessageId = optimisticMsgIds[index] ?? sendOptions.messageId;
        await this.channel.publish({
          name: EVENT_AI_INPUT,
          data: input,
          ...(inputCodecMessageId !== undefined
            ? {
                messageSerial: inputCodecMessageId,
                messageId: inputCodecMessageId,
              }
            : {}),
          extras: {
            ai: {
              transport: this.inputHeaders({
                runId,
                ...(sendOptions.role !== undefined ? { role: sendOptions.role } : {}),
                ...(sendOptions.supersedes !== undefined
                  ? { supersedes: sendOptions.supersedes }
                  : {}),
                invocationId,
                inputEventId: eventIds[index] ?? inputEventId,
                ...(inputCodecMessageId !== undefined
                  ? { codecMessageId: inputCodecMessageId }
                  : {}),
                ...(index === 0
                  ? parent !== undefined
                    ? { parent }
                    : {}
                  : optimisticMsgIds[index - 1] !== undefined
                    ? { parent: optimisticMsgIds[index - 1] }
                    : {}),
                ...(wireForkOf !== undefined ? { forkOf: wireForkOf } : {}),
                runContinue: isContinuation,
                ...(regenerateOf !== undefined ? { regenerates: regenerateOf } : {}),
              }),
            },
          },
        });
      }
    } catch (error) {
      for (const msgId of optimisticMsgIds) {
        this.tree.delete(msgId);
      }
      throw mapPublishFailure(error, {
        code: ErrorCode.SessionSendFailed,
        message: "unable to send; channel publish failed",
      });
    }

    const stream = this.router.has(runId)
      ? this.rebindContinuation(runId, invocationId)
      : this.router.createStream(runId, invocationId);
    this.ownRuns.set(runId, {
      invocationId,
      clientId: this.clientId ?? "",
    });
    const activeRun: ClientRun<TInput, TOutput> = {
      stream,
      runId,
      invocationId,
      inputEventId,
      cancel: () => this.cancel({ runId }),
      steer: (input: TInput) => this.publishSteer(runId, input),
      optimisticMsgIds,
    };

    const waiter =
      sendOptions.waitForRunStart === false ? undefined : this.waitForRunStart(activeRun);
    this.poke(
      {
        runId,
        invocationId,
        inputEventId,
        parent,
        forkOf: sendOptions.forkOf,
        trigger: sendOptions.trigger ?? "message",
        messageId: sendOptions.messageId ?? optimisticMsgIds.at(-1),
        messages,
        inputs,
        staged,
      },
      sendOptions,
    );
    return waiter ?? activeRun;
  }

  private connect(): void {
    if (this.connected) {
      return;
    }
    this.connected = true;
    this.unsubscribes.push(
      this.channel.subscribe((message) => {
        this.handleInbound(message);
      }),
      this.channel.on("continuity_lost", (error) => {
        this.handleContinuityLost(error);
      }),
    );
  }

  private handleInbound(message: InboundMessage): void {
    if (this.closed) {
      return;
    }
    try {
      this.emitter.emit("message", message);
      const transportHeaders = message.getTransportHeaders();
      if (isRunStartMessage(message.name)) {
        this.handleRunStart(message, transportHeaders, "start");
        return;
      }
      if (message.name === EVENT_AI_RUN_RESUME) {
        this.handleRunStart(message, transportHeaders, "resume");
        return;
      }
      if (message.name === EVENT_AI_RUN_SUSPEND) {
        this.handleRunEnd(
          message,
          mergeHeaders(transportHeaders, { [HEADER_RUN_REASON]: "suspended" }),
          "suspend",
        );
        return;
      }
      if (isRunEndMessage(message.name)) {
        this.handleRunEnd(message, transportHeaders, "end");
        return;
      }
      if (message.name === EVENT_AI_STEP_START || message.name === EVENT_AI_STEP_END) {
        this.tree.applyStepLifecycle({
          type: message.name === EVENT_AI_STEP_START ? "step-start" : "step-end",
          headers: transportHeaders,
          serial: message.deliverySerial ?? message.historySerial,
        });
        return;
      }
      if (message.action === "summary") {
        return;
      }
      // Observed before folding: a steer stamp settles outcomes even when the
      // message carries no decodable events.
      this.steer.observe(transportHeaders);
      const batch = this.decoder.decode(message);
      const folded = decodedForFold<TInput, TOutput>(batch);
      if (folded.length > 0) {
        this.tree.applyMessage(
          folded,
          transportHeaders,
          message.deliverySerial ?? message.historySerial,
        );
      }
      const runId = transportHeaders[HEADER_RUN_ID];
      if (!runId) {
        return;
      }
      const invocationId = transportHeaders[HEADER_INVOCATION_ID];
      for (const output of batch.outputs) {
        this.router.route(runId, invocationId, output.event);
      }
    } catch (error) {
      this.emitError(
        toErrorInfo(error, {
          code: ErrorCode.SessionSubscriptionError,
          message: "unable to process channel message; subscription failed",
        }),
      );
    }
  }

  private handleRunStart(
    message: InboundMessage,
    headers: HeaderMap,
    type: "start" | "resume",
  ): void {
    const node = this.tree.applyRunLifecycle({
      type,
      headers,
      serial: message.deliverySerial ?? message.historySerial,
    });
    const runId = headers[HEADER_RUN_ID];
    const invocationId = headers[HEADER_INVOCATION_ID];
    if (runId && invocationId) {
      const own = this.ownRuns.get(runId);
      if (own && own.invocationId !== invocationId) {
        own.invocationId = invocationId;
        this.router.rebindStream(runId, invocationId);
      }
      const pending = this.pendingRunStarts.get(invocationId);
      if (pending) {
        this.resolvePendingRunStart(invocationId);
      }
    }
    if (node?.status === "suspended" && runId && invocationId) {
      this.router.rebindStream(runId, invocationId);
    }
  }

  /**
   * Publishes a steer into a live run.
   *
   * `runContinue` is set because this is by definition re-entry into an
   * existing run, which also stops the tree re-reading `parent`/`fork-of` and
   * re-anchoring the run.
   *
   * The outcome promise is registered before the publish so a stamp that
   * arrives while the publish is still in flight is not missed.
   */
  private publishSteer(runId: string, input: TInput): SteerResult {
    this.assertOpen("steer");
    this.connect();

    const codecMessageId = this.idProvider.messageId();
    const outcome = this.steer.track(runId, codecMessageId);

    const published = (async () => {
      const ack = await this.channel.publish({
        name: EVENT_AI_INPUT,
        data: input,
        messageSerial: codecMessageId,
        messageId: codecMessageId,
        extras: {
          ai: {
            transport: this.inputHeaders({
              runId,
              invocationId: this.idProvider.invocationId(),
              inputEventId: this.idProvider.inputEventId(),
              codecMessageId,
              runContinue: true,
            }),
          },
        },
      });
      // Sockudo has four serial spaces; the durable history serial is the one that
      // confirms the publish landed.
      return { serial: ack.historySerial };
    })().catch((error: unknown) => {
      throw mapPublishFailure(error, {
        code: ErrorCode.SessionSendFailed,
        message: "unable to steer; channel publish failed",
      });
    });

    return { published, outcome };
  }

  private handleRunEnd(message: InboundMessage, headers: HeaderMap, type: "suspend" | "end"): void {
    const runId = headers[HEADER_RUN_ID];
    const invocationId = headers[HEADER_INVOCATION_ID];
    if (
      runId &&
      invocationId &&
      this.router.activeInvocation(runId) !== undefined &&
      this.router.activeInvocation(runId) !== invocationId
    ) {
      return;
    }
    const node = this.tree.applyRunLifecycle({
      type,
      headers,
      serial: message.deliverySerial ?? message.historySerial,
    });
    if (runId) {
      // A suspend settles only steers it can prove consumed; a resume may still
      // claim the rest, so they stay pending until the run truly ends.
      this.steer.settle(
        runId,
        node?.status === "active" ? undefined : node?.status,
        type === "end",
      );
    }
    if (!runId) {
      return;
    }
    const reason = headers[HEADER_RUN_REASON];
    if (reason !== "suspended") {
      this.router.closeStream(runId);
      this.ownRuns.delete(runId);
      for (const pending of Array.from(this.pendingRunStarts.values())) {
        if (pending.turn.runId === runId) {
          this.resolvePendingRunStart(pending.invocationId);
        }
      }
    }
    if (node && reason === "suspended" && invocationId) {
      this.router.rebindStream(runId, invocationId);
    }
  }

  private handleContinuityLost(error: ErrorInfo): void {
    for (const runId of this.ownRuns.keys()) {
      this.router.errorStream(runId, error);
    }
    this.emitError(error);
  }

  private waitForRunStart(turn: ClientRun<TInput, TOutput>): Promise<ClientRun<TInput, TOutput>> {
    if (this.runStartDeadlineMs === 0) {
      return Promise.resolve(turn);
    }
    return new Promise((resolve, reject) => {
      const pending: PendingRunStart<TInput, TOutput> = {
        invocationId: turn.invocationId,
        resolve,
        reject,
        turn,
        timer: setTimeout(() => {
          this.pendingRunStarts.delete(turn.invocationId);
          reject(
            new ErrorInfo({
              code: ErrorCode.RunStartDeadlineExceeded,
              statusCode: 504,
              message: "unable to send; turn start deadline exceeded",
            }),
          );
        }, this.runStartDeadlineMs),
      };
      this.pendingRunStarts.set(turn.invocationId, pending);
    });
  }

  private resolvePendingRunStart(invocationId: string): void {
    const pending = this.pendingRunStarts.get(invocationId);
    if (!pending) {
      return;
    }
    this.pendingRunStarts.delete(invocationId);
    if (pending.timer) {
      clearTimeout(pending.timer);
    }
    pending.resolve(pending.turn);
  }

  private poke(context: PokeContext<TInput, TMessage, TOutput>, sendOptions: SendOptions): void {
    const body = {
      id: context.invocationId,
      messages: context.messages,
      inputs: context.inputs,
      history: this.view.getMessages(),
      clientId: this.clientId,
      parent: context.parent,
      forkOf: context.forkOf,
      trigger: context.trigger,
      messageId: context.messageId,
      events: context.staged,
      ...this.bodyProvider(),
      ...sendOptions.body,
      sessionName: this.channel.name,
      runId: context.runId,
      invocationId: context.invocationId,
      inputEventId: context.inputEventId,
    };
    const headers = {
      "Content-Type": "application/json",
      ...this.headerProvider(),
      ...sendOptions.headers,
    };
    void this.fetchFn(this.options.api, {
      method: "POST",
      headers,
      body: JSON.stringify(stripUndefined(body)),
      ...(this.options.credentials ? { credentials: this.options.credentials } : {}),
    })
      .then((response) => {
        if (!response.ok) {
          const error = new ErrorInfo({
            code: ErrorCode.SessionSendFailed,
            statusCode: response.status,
            message: `unable to send; HTTP POST returned ${String(response.status)}`,
          });
          this.rejectRunStart(context.invocationId, error);
          this.router.errorStream(context.runId, error);
          this.emitError(error);
        }
      })
      .catch((error: unknown) => {
        const info = toErrorInfo(error, {
          code: ErrorCode.SessionSendFailed,
          message: "unable to send; HTTP POST failed",
        });
        this.rejectRunStart(context.invocationId, info);
        this.router.errorStream(context.runId, info);
        this.emitError(info);
      });
  }

  private rejectRunStart(invocationId: string, error: ErrorInfo): void {
    const pending = this.pendingRunStarts.get(invocationId);
    if (!pending) {
      return;
    }
    this.pendingRunStarts.delete(invocationId);
    if (pending.timer) {
      clearTimeout(pending.timer);
    }
    pending.reject(error);
  }

  private inputHeaders(options: {
    runId: string;
    role?: "user" | "assistant";
    supersedes?: string;
    invocationId: string;
    inputEventId: string;
    codecMessageId?: string;
    parent?: string;
    forkOf?: string;
    runContinue?: boolean;
    regenerates?: string | boolean;
  }): HeaderMap {
    const headers = buildTransportHeaders({
      role: options.role ?? "user",
      runId: options.runId,
      invocationId: options.invocationId,
      inputEventId: options.inputEventId,
      ...(options.codecMessageId !== undefined ? { codecMessageId: options.codecMessageId } : {}),
      ...(this.clientId !== undefined
        ? { runClientId: this.clientId, inputClientId: this.clientId }
        : {}),
      ...(options.parent !== undefined ? { parent: options.parent } : {}),
      ...(options.forkOf !== undefined ? { forkOf: options.forkOf } : {}),
      ...(options.runContinue !== undefined ? { runContinue: options.runContinue } : {}),
      ...(options.regenerates !== undefined ? { regenerates: options.regenerates } : {}),
      ...(options.supersedes !== undefined ? { supersedes: options.supersedes } : {}),
    });
    return headers;
  }

  private currentParent(): string | undefined {
    const messages = this.view.getMessages();
    for (let index = messages.length - 1; index >= 0; index -= 1) {
      const id = messageIdOf(messages[index]);
      if (id !== undefined) {
        return id;
      }
    }
    return undefined;
  }

  private seedMessages(messages: readonly TMessage[]): void {
    let parent: string | undefined;
    for (const message of messages) {
      const msgId = messageIdOf(message) ?? this.idProvider.messageId();
      const runId = this.idProvider.runId();
      const headers = buildTransportHeaders({
        role: "user",
        runId,
        codecMessageId: msgId,
        ...(parent !== undefined ? { parent } : {}),
        ...(this.clientId !== undefined ? { runClientId: this.clientId } : {}),
      });
      this.tree.applyMessage(
        [decodedEvent<TInput | TOutput>(message as unknown as TInput | TOutput, msgId, 0)],
        headers,
        0,
      );
      parent = msgId;
    }
  }

  private closeMatchingRunStreams(filter: CancelFilter): void {
    for (const runId of this.getMatchingRunIds(filter)) {
      this.router.closeStream(runId);
    }
  }

  private getMatchingRunIds(filter: CancelFilter): Set<string> {
    const matched = new Set<string>();
    const active = this.tree.getActiveRunIds();
    if ("all" in filter && filter.all) {
      for (const turns of active.values()) {
        for (const runId of turns) {
          matched.add(runId);
        }
      }
    } else if ("runId" in filter && filter.runId) {
      matched.add(filter.runId);
    } else if ("clientId" in filter && filter.clientId) {
      for (const runId of active.get(filter.clientId) ?? []) {
        matched.add(runId);
      }
    } else if ("own" in filter && filter.own) {
      for (const runId of active.get(this.clientId ?? "") ?? []) {
        matched.add(runId);
      }
    }
    return matched;
  }

  private rebindContinuation(runId: string, invocationId: string): ReadableStream<TOutput> {
    this.router.rebindStream(runId, invocationId);
    return this.router.getStream(runId) ?? this.router.createStream(runId, invocationId);
  }

  private emitError(error: ErrorInfo): void {
    this.emitter.emit("error", error);
  }

  private assertOpen(operation: string): void {
    if (this.closed) {
      throw new ErrorInfo({
        code: ErrorCode.SessionClosed,
        statusCode: 400,
        message: `unable to ${operation}; transport is closed`,
      });
    }
  }
}

interface PokeContext<TInput, TMessage, TOutput> {
  runId: string;
  invocationId: string;
  inputEventId: string;
  parent: string | undefined;
  forkOf: string | undefined;
  trigger: string;
  messageId: string | undefined;
  messages: readonly TMessage[];
  inputs: readonly TInput[];
  staged: readonly StagedEvents<TOutput>[];
}

function normalizeProvider<T extends Record<string, unknown>>(
  value: T | (() => T) | undefined,
): () => T {
  if (typeof value === "function") {
    return value;
  }
  return () => value ?? ({} as T);
}

function assertAiTransportFeature(features: readonly string[] | undefined, logger: Logger): void {
  if (features === undefined) {
    return;
  }
  if (features.includes("ai-transport")) {
    return;
  }
  const error = new ErrorInfo({
    code: ErrorCode.ChannelNotReady,
    statusCode: 501,
    message:
      "unable to create client transport; Sockudo server does not advertise the ai-transport feature",
    detail: { requiredFeature: "ai-transport", features },
  });
  logger.error(error.message, {
    code: error.code,
    requiredFeature: "ai-transport",
    advertisedFeatures: features,
  });
  throw error;
}

function readConnectionFeatures(client: ClientLike | undefined): readonly string[] | undefined {
  const connection = client?.connection as
    | (ClientLike["connection"] & { features?: unknown })
    | undefined;
  if (!Array.isArray(connection?.features)) {
    return undefined;
  }
  return connection.features.filter((feature) => typeof feature === "string");
}

function decodedEvent<TEvent>(
  event: TEvent,
  messageId: string,
  serial: string | number,
): DecodedEvent<TEvent> {
  return { event, messageId, meta: { serial, messageId } };
}

function decodedForFold<TInput, TOutput>(
  batch: DecodedBatch<TInput, TOutput>,
): readonly DecodedEvent<TInput | TOutput>[] {
  if (batch.inputs.length === 0) {
    return batch.outputs;
  }
  if (batch.outputs.length === 0) {
    return batch.inputs;
  }
  return [...batch.inputs, ...batch.outputs] as DecodedEvent<TInput | TOutput>[];
}

function messageIdOf(message: unknown): string | undefined {
  if (message !== null && typeof message === "object" && "id" in message) {
    const id = (message as { id?: unknown }).id;
    return typeof id === "string" ? id : undefined;
  }
  return undefined;
}

function normalizeInputs<TInput>(input: TInput | readonly TInput[]): readonly TInput[] {
  return Array.isArray(input) ? (input as readonly TInput[]) : [input as TInput];
}

function cancelHeaders(filter: CancelFilter, clientId: string | undefined): HeaderMap {
  if ("runId" in filter && filter.runId) {
    return buildTransportHeaders({ runId: filter.runId });
  }
  if ("clientId" in filter && filter.clientId) {
    return buildTransportHeaders({ runClientId: filter.clientId });
  }
  if ("own" in filter && filter.own) {
    return buildTransportHeaders(clientId === undefined ? {} : { inputClientId: clientId });
  }
  return buildTransportHeaders({});
}

function isRunStartMessage(name: string): boolean {
  return name === EVENT_AI_RUN_START || name === INBOUND_LEGACY_EVENT_TURN_START;
}

function isRunEndMessage(name: string): boolean {
  return name === EVENT_AI_RUN_END || name === INBOUND_LEGACY_EVENT_TURN_END;
}

function missingChannel(): ChannelLike {
  throw new ErrorInfo({
    code: ErrorCode.InvalidArgument,
    message: "unable to create client transport; channel could not be resolved",
  });
}

function mapPublishFailure(
  error: unknown,
  fallback: { code: ErrorCode; message: string },
): ErrorInfo {
  const mapped = toErrorInfo(error, fallback);
  const status = statusLike(error) ?? mapped.statusCode;
  if (status === 401 || status === 403 || mapped.code === 401 || mapped.code === 403) {
    return new ErrorInfo({
      code: ErrorCode.InsufficientCapability,
      statusCode: status,
      message: mapped.message,
      cause: error,
      detail: mapped.detail,
    });
  }
  return mapped;
}

function statusLike(value: unknown): number | undefined {
  const record =
    value !== null && typeof value === "object" ? (value as Record<string, unknown>) : undefined;
  const status = record?.status ?? record?.statusCode;
  return typeof status === "number" ? status : undefined;
}

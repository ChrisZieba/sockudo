import { HEADER_INPUT_CLIENT_ID, HEADER_RUN_CLIENT_ID, HEADER_RUN_ID } from "../../constants.js";
import { ErrorCode, ErrorInfo } from "../../errors.js";
import type { InboundMessage } from "../../realtime/index.js";
import type { HeaderMap } from "../../utils.js";
import type { CancelFilter } from "./client-session.js";

/** Passed to turn cancel authorization hooks. */
export interface CancelRequest {
  /** Raw cancel message. */
  message: InboundMessage;
  /** Parsed cancel filter. */
  filter: CancelFilter;
  /** Matched turn ids. */
  matchedRunIds: string[];
  /** Matched turn owners by turn id. */
  runOwners: Map<string, string>;
}

/** Registered server-side turn state. */
export interface ManagedRun {
  /** AgentRun identity. */
  runId: string;
  /** Owning client id. */
  clientId?: string;
  /** Aborts this turn. */
  abort(reason?: unknown): void;
  /** Optional cancel authorization hook. */
  onCancel?(request: CancelRequest): Promise<boolean> | boolean;
  /** Optional turn-scoped error hook. */
  onError?(error: ErrorInfo): void;
  /**
   * Notified when a client input arrives naming this already-running run —
   * a steer. Distinct from the run's originating input, which is matched by
   * invocation id instead.
   */
  onSteer?(codecMessageId: string): void;
  /** Whether this turn has already observed cancellation. */
  cancelled: boolean;
}

/** Input event buffered by invocation id. */
export interface BufferedInputEvent {
  /** Raw inbound message. */
  message: InboundMessage;
  /** Transport headers. */
  headers: HeaderMap;
}

/** AgentRun manager options. */
export interface RunManagerOptions {
  /** Maximum buffered input events.
   *
   * @defaultValue `200`
   */
  inputEventBufferLimit?: number;
}

/**
 * O(1) registry for server turns, early input lookup, and cancel routing.
 */
export class RunManager {
  private readonly turns = new Map<string, ManagedRun>();
  private readonly inputBuffer = new Map<string, BufferedInputEvent[]>();
  private readonly inputWaiters = new Map<string, Set<(event: BufferedInputEvent) => void>>();
  private readonly bufferOrder: string[] = [];
  private readonly stragglerSeen = new Set<string>();
  private readonly bufferLimit: number;

  /** Creates a manager. */
  public constructor(options: RunManagerOptions = {}) {
    this.bufferLimit = options.inputEventBufferLimit ?? 200;
  }

  /** Registers a turn for input lookup and cancellation. */
  public register(turn: ManagedRun): void {
    this.turns.set(turn.runId, turn);
  }

  /** Deregisters a turn. */
  public deregister(runId: string): void {
    this.turns.delete(runId);
  }

  /** Returns current turn owners. */
  public runOwners(): Map<string, string> {
    const owners = new Map<string, string>();
    for (const [runId, turn] of this.turns) {
      if (turn.clientId !== undefined) {
        owners.set(runId, turn.clientId);
      }
    }
    return owners;
  }

  /** Buffers or delivers an inbound input event. */
  public observeInput(message: InboundMessage): void {
    const headers = message.getTransportHeaders();
    // An input naming a run we are already executing is a steer, not this
    // run's originating input: route it to the run before invocation matching,
    // which would otherwise never pair it with anything.
    const steerRunId = headers["run-id"];
    const steerCodecMessageId = headers["codec-message-id"];
    if (steerRunId !== undefined && steerCodecMessageId !== undefined) {
      const running = this.turns.get(steerRunId);
      if (running?.onSteer !== undefined) {
        running.onSteer(steerCodecMessageId);
      }
    }
    const invocationId = headers["invocation-id"];
    if (!invocationId) {
      return;
    }
    const event: BufferedInputEvent = { message, headers };
    const waiters = this.inputWaiters.get(invocationId);
    if (waiters && waiters.size > 0) {
      for (const waiter of waiters) {
        waiter(event);
      }
      this.inputWaiters.delete(invocationId);
      this.stragglerSeen.add(invocationId);
      return;
    }
    if (this.stragglerSeen.has(invocationId)) {
      return;
    }
    let events = this.inputBuffer.get(invocationId);
    if (!events) {
      events = [];
      this.inputBuffer.set(invocationId, events);
    }
    events.push(event);
    this.bufferOrder.push(invocationId);
    this.evictInputBuffer();
  }

  /** Looks up the first input event for an invocation id. */
  public lookupInput(invocationId: string, timeoutMs: number): Promise<BufferedInputEvent> {
    const buffered = this.inputBuffer.get(invocationId)?.shift();
    if (buffered) {
      this.stragglerSeen.add(invocationId);
      return Promise.resolve(buffered);
    }
    return new Promise((resolve, reject) => {
      const waiter = (event: BufferedInputEvent): void => {
        clearTimeout(timer);
        this.inputWaiters.get(invocationId)?.delete(waiter);
        this.stragglerSeen.add(invocationId);
        resolve(event);
      };
      const timer = setTimeout(() => {
        this.inputWaiters.get(invocationId)?.delete(waiter);
        reject(
          new ErrorInfo({
            code: ErrorCode.InputEventNotFound,
            statusCode: 504,
            message: "unable to start turn; input event was not found",
          }),
        );
      }, timeoutMs);
      let waiters = this.inputWaiters.get(invocationId);
      if (!waiters) {
        waiters = new Set();
        this.inputWaiters.set(invocationId, waiters);
      }
      waiters.add(waiter);
    });
  }

  /** Routes a cancel message to matching turns. */
  public routeCancel(message: InboundMessage, fallbackFilter?: CancelFilter): void {
    const headers = message.getTransportHeaders();
    const filter = fallbackFilter ?? parseCancelFilter(headers, message.data);
    const matched = this.matchRuns(filter, headers);
    const owners = this.runOwners();
    const request: CancelRequest = {
      message,
      filter,
      matchedRunIds: [...matched],
      runOwners: owners,
    };
    for (const runId of matched) {
      const turn = this.turns.get(runId);
      if (!turn || turn.cancelled) {
        continue;
      }
      void this.authorizeAndAbort(turn, request);
    }
  }

  /** Aborts and clears every registered turn. */
  public close(): void {
    for (const turn of this.turns.values()) {
      turn.abort();
    }
    this.turns.clear();
    this.inputBuffer.clear();
    this.inputWaiters.clear();
  }

  private async authorizeAndAbort(turn: ManagedRun, request: CancelRequest): Promise<void> {
    try {
      if (turn.onCancel && !(await turn.onCancel(request))) {
        return;
      }
      turn.cancelled = true;
      turn.abort();
    } catch (error) {
      turn.onError?.(
        new ErrorInfo({
          code: ErrorCode.CancelListenerError,
          message: "unable to cancel turn; cancel listener failed",
          cause: error,
        }),
      );
    }
  }

  private matchRuns(filter: CancelFilter, headers: HeaderMap): Set<string> {
    const matched = new Set<string>();
    if ("all" in filter && filter.all) {
      for (const runId of this.turns.keys()) {
        matched.add(runId);
      }
      return matched;
    }
    if ("runId" in filter && filter.runId) {
      if (this.turns.has(filter.runId)) {
        matched.add(filter.runId);
      }
      return matched;
    }
    const targetClientId =
      "clientId" in filter && filter.clientId
        ? filter.clientId
        : "own" in filter && filter.own
          ? (headers[HEADER_INPUT_CLIENT_ID] ?? headers[HEADER_RUN_CLIENT_ID])
          : undefined;
    if (!targetClientId) {
      return matched;
    }
    for (const [runId, turn] of this.turns) {
      if (turn.clientId === targetClientId) {
        matched.add(runId);
      }
    }
    return matched;
  }

  private evictInputBuffer(): void {
    while (this.bufferOrder.length > this.bufferLimit) {
      const invocationId = this.bufferOrder.shift();
      if (!invocationId) {
        return;
      }
      const events = this.inputBuffer.get(invocationId);
      events?.shift();
      if (!events || events.length === 0) {
        this.inputBuffer.delete(invocationId);
      }
    }
  }
}

function parseCancelFilter(headers: HeaderMap, data: unknown): CancelFilter {
  if (headers[HEADER_RUN_ID]) {
    return { runId: headers[HEADER_RUN_ID] };
  }
  const record =
    data !== null && typeof data === "object" ? (data as Record<string, unknown>) : undefined;
  if (typeof record?.runId === "string") {
    return { runId: record.runId };
  }
  if (typeof record?.clientId === "string") {
    return { clientId: record.clientId };
  }
  if (record?.all === true || headers.filter === "all") {
    return { all: true };
  }
  return { own: true };
}

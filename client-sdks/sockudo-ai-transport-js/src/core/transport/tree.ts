import {
  HEADER_CODEC_MESSAGE_ID,
  HEADER_FORK_OF,
  HEADER_INPUT_CLIENT_ID,
  HEADER_INVOCATION_ID,
  HEADER_MSG_REGENERATE,
  HEADER_PARENT,
  HEADER_RUN_CLIENT_ID,
  HEADER_RUN_CONTINUE,
  INBOUND_LEGACY_HEADER_TURN_CONTINUE,
  HEADER_RUN_ID,
  HEADER_RUN_REASON,
  HEADER_STEP_CLIENT_ID,
  HEADER_STEP_ID,
  HEADER_STEP_REASON,
  HEADER_STEP_START_SERIAL,
} from "../../constants.js";
import { EventEmitter, type EventUnsubscribe } from "../../event-emitter.js";
import type { HeaderMap } from "../../utils.js";
import type { DecodedEvent, Reducer, ReducerMeta } from "../codec/index.js";

/**
 * Sockudo serial values preserve unsafe integers as strings.
 */
export type TreeSerial = number | string;

/**
 * Wire turn-end reasons.
 */
export type RunEndReason = "complete" | "cancelled" | "error" | "suspended";

/**
 * AgentRun status tracked by the conversation tree.
 */
export type RunStatus = "active" | RunEndReason;

/**
 * Wire step-end reasons.
 *
 * Narrower than {@link RunEndReason}: a step has no `error` arm, because a
 * stream, model, or tool failure ends the attempt `failed` and is retryable.
 * `cancelled` is run-level bleed-through — cancelling a run closes its open
 * step so the bracket stays balanced.
 */
export type StepEndReason = "complete" | "failed" | "cancelled";

/**
 * State of one logical step within a run.
 *
 * A step is re-attemptable: a retry publishes a fresh `ai-step-start` under the
 * same `step-id`, and the attempt with the highest `step-start-serial` is
 * canonical. Output stamped with a superseded attempt's serial is ignored, so a
 * failed attempt's partial output never reaches the projection.
 */
export interface StepInfo {
  /** Stable step identity across attempts. */
  stepId: string;
  /** Status of the canonical attempt. */
  status: "active" | StepEndReason;
  /** Number of distinct attempts observed. */
  attemptCount: number;
  /** Verified client id that owns the step, when known. */
  stepClientId?: string;
  /** Serial of the canonical attempt's `ai-step-start`. */
  startSerial?: TreeSerial;
}

/**
 * Step lifecycle event accepted by {@link Tree.applyStepLifecycle}.
 */
export interface StepLifecycleEvent {
  /** Lifecycle kind. */
  type: "step-start" | "step-end";
  /** Transport headers carried by the lifecycle message. */
  headers: HeaderMap;
  /** Serial assigned by Sockudo delivery or history. */
  serial: TreeSerial;
}

/**
 * AgentRun-keyed node in the conversation tree.
 */
export interface RunNode<TProjection> {
  /** Stable turn id. */
  runId: string;
  /** Parent turn id, resolved from parent or fork metadata. */
  parentRunId?: string;
  /** AgentRun id this node forked from. */
  forkOf?: string;
  /** Anchored codec message id this turn regenerates. */
  regeneratesCodecMessageId?: string;
  /** Verified client id. */
  clientId?: string;
  /** Current turn status. */
  status: RunStatus;
  /** Codec projection. May be mutated in place by the reducer. */
  projection: TProjection;
  /** Latest invocation id for the turn or continuation. */
  invocationId?: string;
  /** First known serial for sorting; serial-less optimistic turns sort last. */
  startSerial?: TreeSerial;
  /** Terminal lifecycle serial. */
  endSerial?: TreeSerial;
  /** Steps observed for this run, in first-seen order. */
  steps: StepInfo[];
}

/**
 * AgentRun lifecycle event accepted by {@link Tree.applyRunLifecycle}.
 */
export interface RunLifecycleEvent {
  /**
   * Lifecycle kind, mirroring the four wire events.
   *
   * `resume` is a distinct arm rather than a `start` carrying a continuation
   * header: re-entry is already identified on the wire by the `ai-run-resume`
   * event name, so the header carried no information the name did not.
   */
  type: "start" | "suspend" | "resume" | "end";
  /** Transport headers carried by the lifecycle message. */
  headers: HeaderMap;
  /** Serial assigned by Sockudo delivery or history. */
  serial: TreeSerial;
  /** Optional decoded turn-end reason override. */
  reason?: RunEndReason;
}

/**
 * Folded message event emitted by the tree.
 */
export interface TreeMessageEvent<TEvent, TProjection> {
  /** Owning turn id. */
  runId: string;
  /** Owning turn node. */
  node: RunNode<TProjection>;
  /** Folded codec event. */
  event: TEvent;
  /** Codec message id. */
  messageId?: string;
  /** Fold serial. */
  serial: TreeSerial;
}

/**
 * Conversation tree event map.
 */
export interface TreeEvents<TEvent, TProjection> {
  /** Structural tree state changed. */
  update: { structuralVersion: number };
  /** Preferred message-fold event. */
  message: TreeMessageEvent<TEvent, TProjection>;
  /** Deprecated Ably-compatible message-fold alias. */
  "ably-message": TreeMessageEvent<TEvent, TProjection>;
  /** AgentRun metadata or status changed. */
  turn: RunNode<TProjection>;
  /** AgentRun projection was folded, possibly mutating in place. */
  "turn-projection-updated": TreeMessageEvent<TEvent, TProjection>;
}

/**
 * Conversation tree construction options.
 */
export interface TreeOptions<TProjection> {
  /** Optional optimistic projection seed for tests and docs adapters. */
  createProjection?(): TProjection;
}

/**
 * Public conversation tree API.
 */
export interface Tree<TEvent, TProjection> {
  /** Current structural cache version. */
  readonly structuralVersion: number;
  /** Applies decoded message events using the shared live/history upsert path. */
  applyMessage(
    decodedEvents: readonly DecodedEvent<TEvent>[],
    transportHeaders: HeaderMap,
    serial: TreeSerial,
  ): RunNode<TProjection> | undefined;
  /** Applies turn lifecycle metadata. */
  applyRunLifecycle(event: RunLifecycleEvent): RunNode<TProjection> | undefined;
  /** Applies step lifecycle metadata. */
  applyStepLifecycle(event: StepLifecycleEvent): RunNode<TProjection> | undefined;
  /** Whether output stamped with this step attempt is from the canonical attempt. */
  isCanonicalStepAttempt(stepId: string, startSerial: string): boolean;
  /** Deletes a codec message id and removes unreachable turns. */
  delete(codecMessageId: string): void;
  /** Docs-compatible upsert alias for {@link applyMessage}. */
  upsert(
    decodedEvents: readonly DecodedEvent<TEvent>[],
    transportHeaders: HeaderMap,
    serial: TreeSerial,
  ): RunNode<TProjection> | undefined;
  /** Gets a node by turn id. */
  getRunNode(runId: string): RunNode<TProjection> | undefined;
  /** Gets a node by codec message id. */
  getNodeByCodecMessageId(codecMessageId: string): RunNode<TProjection> | undefined;
  /** Docs-compatible message-granularity node lookup. */
  getNode(codecMessageId: string): RunNode<TProjection> | undefined;
  /** Gets edit siblings for a turn id or codec message id. */
  getSiblingNodes(id: string): readonly RunNode<TProjection>[];
  /** Docs-compatible sibling lookup alias. */
  getSiblings(id: string): readonly RunNode<TProjection>[];
  /** Returns whether a turn or codec message id has edit siblings. */
  hasSiblingNodes(id: string): boolean;
  /** Docs-compatible sibling predicate alias. */
  hasSiblings(id: string): boolean;
  /** Gets the regenerate group for an anchored codec message id. */
  getRegenerateGroup(codecMessageId: string): readonly RunNode<TProjection>[];
  /** Gets the latest continuation invocation for a turn. */
  getLatestContinuationInvocation(runId: string): string | undefined;
  /** Gets active and suspended turns grouped by verified client id. */
  getActiveRunIds(): Map<string, Set<string>>;
  /** Gets defensive transport headers by codec message id. */
  getHeaders(codecMessageId: string): HeaderMap | undefined;
  /** Gets all turn nodes in start-serial order, with optimistic turns last. */
  getRunNodes(): readonly RunNode<TProjection>[];
  /** Subscribes to conversation tree events. */
  on<K extends keyof TreeEvents<TEvent, TProjection>>(
    event: K,
    handler: (payload: TreeEvents<TEvent, TProjection>[K]) => void,
  ): EventUnsubscribe;
}

/**
 * Creates a turn-keyed conversation tree.
 *
 * Projection ref-equality is not a change signal: reducers may mutate the
 * projection in place, and streaming folds emit `turn-projection-updated`.
 */
export function createTree<TEvent, TProjection>(
  reducer: Reducer<TEvent, TProjection>,
  options: TreeOptions<TProjection> = {},
): Tree<TEvent, TProjection> {
  return new TreeImpl(reducer, options);
}

class TreeImpl<TEvent, TProjection> implements Tree<TEvent, TProjection> {
  private readonly emitter = new EventEmitter<TreeEvents<TEvent, TProjection>>();
  private readonly runIndex = new Map<string, RunNode<TProjection>>();
  private readonly codecMessageIdToRunId = new Map<string, string>();
  private readonly runCodecMessageIds = new Map<string, Set<string>>();
  private readonly headersByCodecMessageId = new Map<string, HeaderMap>();
  private readonly sortedRuns: string[] = [];
  private readonly parentIndex = new Map<string, Set<string>>();
  private readonly rootRuns = new Set<string>();
  private readonly regenerateByMsgId = new Map<string, Set<string>>();
  private readonly pendingParentRefByRun = new Map<string, string>();
  private readonly pendingForkRefByRun = new Map<string, string>();
  private readonly siblingCache = new Map<string, CachedNodes<TProjection>>();
  private readonly regenerateCache = new Map<string, CachedNodes<TProjection>>();
  private readonly latestContinuationInvocation = new Map<string, string>();
  private version = 0;

  public constructor(
    private readonly reducer: Reducer<TEvent, TProjection>,
    private readonly options: TreeOptions<TProjection>,
  ) {}

  public get structuralVersion(): number {
    return this.version;
  }

  public applyMessage(
    decodedEvents: readonly DecodedEvent<TEvent>[],
    transportHeaders: HeaderMap,
    serial: TreeSerial,
  ): RunNode<TProjection> | undefined {
    // Output from a superseded step attempt is not materialised: a retry
    // republishes under the same step-id, and only the highest
    // step-start-serial is canonical, so a failed attempt's partial output must
    // not reach the projection.
    const stampedStepId = transportHeaders[HEADER_STEP_ID];
    const stampedStepStart = transportHeaders[HEADER_STEP_START_SERIAL];
    if (
      stampedStepId !== undefined &&
      stampedStepStart !== undefined &&
      !this.isCanonicalStepAttempt(stampedStepId, stampedStepStart)
    ) {
      return this.runIndex.get(transportHeaders[HEADER_RUN_ID] ?? "");
    }
    if (decodedEvents.length === 0) {
      return undefined;
    }
    const route = this.resolveMessageRoute(decodedEvents, transportHeaders);
    if (!route.runId) {
      return undefined;
    }
    const metadata = this.metadataFromHeaders(
      route.runId,
      transportHeaders,
      !isContinuationHeaders(transportHeaders),
    );
    const node = this.ensureRun(route.runId, metadata, serial);
    if (this.promoteSerial(node, serial)) {
      this.bump();
    }
    for (const decoded of decodedEvents) {
      const messageId =
        decoded.messageId ?? decoded.meta.messageId ?? transportHeaders[HEADER_CODEC_MESSAGE_ID];
      if (messageId !== undefined) {
        this.attachCodecMessage(node.runId, messageId, transportHeaders);
      }
      const meta: ReducerMeta = messageId === undefined ? { serial } : { serial, messageId };
      node.projection = this.reducer.fold(node.projection, decoded.event, meta);
      const event: TreeMessageEvent<TEvent, TProjection> = {
        runId: node.runId,
        node,
        event: decoded.event,
        serial,
      };
      setOptional(event, "messageId", messageId);
      this.emitter.emit("message", event);
      this.emitter.emit("ably-message", event);
      this.emitter.emit("turn-projection-updated", event);
    }
    return node;
  }

  public applyStepLifecycle(event: StepLifecycleEvent): RunNode<TProjection> | undefined {
    const runId = event.headers[HEADER_RUN_ID];
    const stepId = event.headers[HEADER_STEP_ID];
    if (runId === undefined || stepId === undefined) {
      return undefined;
    }
    const node = this.ensureRun(runId, {}, undefined);
    const existing = node.steps.find((step) => step.stepId === stepId);

    if (event.type === "step-start") {
      // A step-start carries no back-reference: its own serial IS the attempt's
      // identity. A repeat under the same step-id is a retry.
      const stepClientId = event.headers[HEADER_STEP_CLIENT_ID];
      if (existing === undefined) {
        const step: StepInfo = { stepId, status: "active", attemptCount: 1 };
        setOptional(step, "startSerial", event.serial);
        setOptional(step, "stepClientId", stepClientId);
        node.steps.push(step);
      } else {
        existing.attemptCount += 1;
        // Only a higher serial supersedes, so out-of-order redelivery of an
        // older attempt cannot un-promote the canonical one.
        if (
          existing.startSerial === undefined ||
          compareSerial(event.serial, existing.startSerial) > 0
        ) {
          existing.startSerial = event.serial;
          existing.status = "active";
        }
        if (stepClientId !== undefined) {
          existing.stepClientId = stepClientId;
        }
      }
      this.bump();
      this.emitter.emit("turn", node);
      return node;
    }

    if (existing === undefined) {
      return node;
    }
    // A step-end for a superseded attempt must not close the canonical one.
    const endsCanonical =
      existing.startSerial === undefined ||
      this.stepAttemptIsCanonical(existing, event.headers[HEADER_STEP_START_SERIAL]);
    if (!endsCanonical) {
      return node;
    }
    const reason = event.headers[HEADER_STEP_REASON];
    existing.status =
      reason === "complete" || reason === "failed" || reason === "cancelled" ? reason : "complete";
    this.bump();
    this.emitter.emit("turn", node);
    return node;
  }

  public isCanonicalStepAttempt(stepId: string, startSerial: string): boolean {
    for (const node of this.runIndex.values()) {
      const step = node.steps.find((candidate) => candidate.stepId === stepId);
      if (step !== undefined) {
        return this.stepAttemptIsCanonical(step, startSerial);
      }
    }
    // An unseen step cannot be judged superseded; treat its output as live so a
    // reordered step-start does not silently drop content.
    return true;
  }

  private stepAttemptIsCanonical(step: StepInfo, startSerial: string | undefined): boolean {
    if (startSerial === undefined || step.startSerial === undefined) {
      return true;
    }
    return compareSerial(startSerial, step.startSerial) >= 0;
  }

  public applyRunLifecycle(event: RunLifecycleEvent): RunNode<TProjection> | undefined {
    const runId = event.headers[HEADER_RUN_ID];
    if (!runId) {
      return undefined;
    }
    if (event.type === "start" || event.type === "resume") {
      // The `resume` arm is the current signal. The header check keeps pre-3.0
      // channel history readable, where re-entry was a `start` carrying
      // `turn-continue`/`run-continue` rather than its own event name.
      const isContinuation = event.type === "resume" || isContinuationHeaders(event.headers);
      const metadata = this.metadataFromHeaders(runId, event.headers, !isContinuation);
      const node = this.ensureRun(runId, metadata, event.serial);
      const wasTerminal = isTerminalStatus(node.status);
      let changed = this.promoteSerial(node, event.serial);
      if (
        !wasTerminal &&
        node.status !== "active" &&
        (node.status !== "suspended" || isContinuation)
      ) {
        node.status = "active";
        changed = true;
      }
      const invocationId = event.headers[HEADER_INVOCATION_ID];
      if (invocationId !== undefined) {
        node.invocationId = invocationId;
        if (isContinuation) {
          this.latestContinuationInvocation.set(runId, invocationId);
        }
      }
      if (!isContinuation) {
        changed = this.backfillMetadata(node, metadata) || changed;
      }
      if (changed) {
        this.bump();
      }
      this.emitter.emit("turn", node);
      return node;
    }
    const node = this.ensureRun(
      runId,
      this.metadataFromHeaders(runId, event.headers, true),
      undefined,
    );
    const reason = event.reason ?? readRunReason(event.headers[HEADER_RUN_REASON]);
    let changed = false;
    if (reason !== undefined && node.status !== reason) {
      node.status = reason;
      changed = true;
    }
    if (node.endSerial !== event.serial) {
      node.endSerial = event.serial;
      changed = true;
    }
    if (changed) {
      this.bump();
    }
    this.emitter.emit("turn", node);
    return node;
  }

  public delete(codecMessageId: string): void {
    const runId = this.codecMessageIdToRunId.get(codecMessageId);
    if (!runId) {
      return;
    }
    this.codecMessageIdToRunId.delete(codecMessageId);
    this.headersByCodecMessageId.delete(codecMessageId);
    const ids = this.runCodecMessageIds.get(runId);
    ids?.delete(codecMessageId);
    if (ids && ids.size > 0) {
      this.bump();
      return;
    }
    this.removeRunAndDescendants(runId);
    this.bump();
  }

  public upsert(
    decodedEvents: readonly DecodedEvent<TEvent>[],
    transportHeaders: HeaderMap,
    serial: TreeSerial,
  ): RunNode<TProjection> | undefined {
    return this.applyMessage(decodedEvents, transportHeaders, serial);
  }

  public getRunNode(runId: string): RunNode<TProjection> | undefined {
    return this.runIndex.get(runId);
  }

  public getNodeByCodecMessageId(codecMessageId: string): RunNode<TProjection> | undefined {
    const runId = this.codecMessageIdToRunId.get(codecMessageId);
    return runId === undefined ? undefined : this.runIndex.get(runId);
  }

  public getNode(codecMessageId: string): RunNode<TProjection> | undefined {
    return this.getNodeByCodecMessageId(codecMessageId);
  }

  public getSiblingNodes(id: string): readonly RunNode<TProjection>[] {
    const node = this.resolveNode(id);
    if (!node) {
      return [];
    }
    const cached = this.siblingCache.get(node.runId);
    if (cached?.version === this.version) {
      return cached.nodes;
    }
    const nodes = this.computeSiblingRuns(node);
    this.siblingCache.set(node.runId, {
      version: this.version,
      nodes,
    });
    return nodes;
  }

  public getSiblings(id: string): readonly RunNode<TProjection>[] {
    return this.getSiblingNodes(id);
  }

  public hasSiblingNodes(id: string): boolean {
    return this.getSiblingNodes(id).length > 1;
  }

  public hasSiblings(id: string): boolean {
    return this.hasSiblingNodes(id);
  }

  public getRegenerateGroup(codecMessageId: string): readonly RunNode<TProjection>[] {
    const cached = this.regenerateCache.get(codecMessageId);
    if (cached?.version === this.version) {
      return cached.nodes;
    }
    const nodes = this.computeRegenerateGroup(codecMessageId);
    this.regenerateCache.set(codecMessageId, {
      version: this.version,
      nodes,
    });
    return nodes;
  }

  public getLatestContinuationInvocation(runId: string): string | undefined {
    return this.latestContinuationInvocation.get(runId) ?? this.runIndex.get(runId)?.invocationId;
  }

  public getActiveRunIds(): Map<string, Set<string>> {
    const active = new Map<string, Set<string>>();
    for (const runId of this.sortedRuns) {
      const node = this.runIndex.get(runId);
      if (!node?.clientId || !isLiveStatus(node.status)) {
        continue;
      }
      let turns = active.get(node.clientId);
      if (!turns) {
        turns = new Set();
        active.set(node.clientId, turns);
      }
      turns.add(runId);
    }
    return active;
  }

  public getHeaders(codecMessageId: string): HeaderMap | undefined {
    return this.headersByCodecMessageId.get(codecMessageId);
  }

  public getRunNodes(): readonly RunNode<TProjection>[] {
    return this.sortedRuns.flatMap((runId) => {
      const node = this.runIndex.get(runId);
      return node === undefined ? [] : [node];
    });
  }

  public on<K extends keyof TreeEvents<TEvent, TProjection>>(
    event: K,
    handler: (payload: TreeEvents<TEvent, TProjection>[K]) => void,
  ): EventUnsubscribe {
    return this.emitter.on(event, handler);
  }

  private resolveMessageRoute(
    decodedEvents: readonly DecodedEvent<TEvent>[],
    headers: HeaderMap,
  ): { runId?: string } {
    const codecMessageId =
      headers[HEADER_CODEC_MESSAGE_ID] ??
      decodedEvents[0]?.messageId ??
      decodedEvents[0]?.meta.messageId;
    if (isContinuationHeaders(headers) && codecMessageId) {
      const existing = this.codecMessageIdToRunId.get(codecMessageId);
      if (existing) {
        return { runId: existing };
      }
    }
    const runId = headers[HEADER_RUN_ID];
    if (runId) {
      return { runId };
    }
    if (codecMessageId) {
      const existing = this.codecMessageIdToRunId.get(codecMessageId);
      if (existing) {
        return { runId: existing };
      }
    }
    return {};
  }

  private metadataFromHeaders(
    runId: string,
    headers: HeaderMap,
    allowGraphMetadata: boolean,
  ): RunMetadata {
    const metadata: RunMetadata = {};
    if (allowGraphMetadata) {
      const forkRef = headers[HEADER_FORK_OF];
      const forkedRunId = this.resolveReference(forkRef);
      const parentRef = headers[HEADER_PARENT];
      const parentFromHeader = this.resolveReference(parentRef);
      const parentRunId =
        forkedRunId !== undefined ? this.runIndex.get(forkedRunId)?.parentRunId : parentFromHeader;
      if (parentRunId !== undefined && parentRunId !== runId) {
        metadata.parentRunId = parentRunId;
      } else if (parentRef !== undefined) {
        metadata.parentRef = parentRef;
      }
      if (forkedRunId !== undefined && forkedRunId !== runId) {
        metadata.forkOf = forkedRunId;
      } else if (forkRef !== undefined) {
        metadata.forkRef = forkRef;
      }
      const regenerateRef = headers[HEADER_MSG_REGENERATE];
      if (regenerateRef !== undefined && regenerateRef !== "false") {
        const anchor =
          regenerateRef === "true"
            ? (headers[HEADER_FORK_OF] ?? headers[HEADER_CODEC_MESSAGE_ID])
            : regenerateRef;
        if (anchor !== undefined) {
          metadata.regeneratesCodecMessageId = anchor;
        }
      }
    }
    const clientId = headers[HEADER_RUN_CLIENT_ID] ?? headers[HEADER_INPUT_CLIENT_ID];
    if (clientId !== undefined) {
      metadata.clientId = clientId;
    }
    const invocationId = headers[HEADER_INVOCATION_ID];
    if (invocationId !== undefined) {
      metadata.invocationId = invocationId;
    }
    return metadata;
  }

  private ensureRun(
    runId: string,
    metadata: RunMetadata,
    serial: TreeSerial | undefined,
  ): RunNode<TProjection> {
    const existing = this.runIndex.get(runId);
    if (existing) {
      if (this.backfillMetadata(existing, metadata)) {
        this.bump();
      }
      return existing;
    }
    const node: RunNode<TProjection> = {
      runId,
      status: "active",
      steps: [],
      projection: this.options.createProjection?.() ?? this.reducer.init(),
    };
    setOptional(node, "startSerial", serial);
    this.runIndex.set(runId, node);
    this.sortedRuns.push(runId);
    this.sortRuns();
    this.backfillMetadata(node, metadata);
    this.resolvePendingReferences(runId, runId);
    this.bump();
    this.emitter.emit("turn", node);
    return node;
  }

  private backfillMetadata(node: RunNode<TProjection>, metadata: RunMetadata): boolean {
    let changed = false;
    if (
      metadata.parentRunId !== undefined &&
      metadata.parentRunId !== node.runId &&
      node.parentRunId !== metadata.parentRunId
    ) {
      this.moveParent(node, metadata.parentRunId);
      changed = true;
    } else if (metadata.parentRef !== undefined) {
      this.pendingParentRefByRun.set(node.runId, metadata.parentRef);
    } else if (node.parentRunId === undefined) {
      this.rootRuns.add(node.runId);
    }
    if (
      metadata.forkOf !== undefined &&
      metadata.forkOf !== node.runId &&
      node.forkOf !== metadata.forkOf
    ) {
      node.forkOf = metadata.forkOf;
      changed = true;
    } else if (metadata.forkRef !== undefined) {
      this.pendingForkRefByRun.set(node.runId, metadata.forkRef);
    }
    if (
      metadata.regeneratesCodecMessageId !== undefined &&
      node.regeneratesCodecMessageId !== metadata.regeneratesCodecMessageId
    ) {
      this.moveRegenerateAnchor(node, metadata.regeneratesCodecMessageId);
      changed = true;
    }
    if (metadata.clientId !== undefined && node.clientId !== metadata.clientId) {
      node.clientId = metadata.clientId;
      changed = true;
    }
    if (metadata.invocationId !== undefined && node.invocationId !== metadata.invocationId) {
      node.invocationId = metadata.invocationId;
      changed = true;
    }
    return changed;
  }

  private promoteSerial(node: RunNode<TProjection>, serial: TreeSerial): boolean {
    if (node.startSerial !== undefined && compareSerial(serial, node.startSerial) >= 0) {
      return false;
    }
    node.startSerial = serial;
    this.sortRuns();
    return true;
  }

  private attachCodecMessage(runId: string, codecMessageId: string, headers: HeaderMap): void {
    const previous = this.codecMessageIdToRunId.get(codecMessageId);
    if (previous === runId) {
      this.headersByCodecMessageId.set(codecMessageId, headers);
      return;
    }
    if (previous !== undefined) {
      this.runCodecMessageIds.get(previous)?.delete(codecMessageId);
    }
    this.codecMessageIdToRunId.set(codecMessageId, runId);
    this.headersByCodecMessageId.set(codecMessageId, headers);
    let ids = this.runCodecMessageIds.get(runId);
    if (!ids) {
      ids = new Set();
      this.runCodecMessageIds.set(runId, ids);
    }
    ids.add(codecMessageId);
    this.resolvePendingReferences(codecMessageId, runId);
    this.bump();
  }

  private resolvePendingReferences(reference: string, resolvedRunId: string): void {
    for (const [runId, pendingRef] of this.pendingParentRefByRun) {
      if (pendingRef !== reference || runId === resolvedRunId) {
        continue;
      }
      const node = this.runIndex.get(runId);
      if (node) {
        this.moveParent(node, resolvedRunId);
      }
      this.pendingParentRefByRun.delete(runId);
    }
    for (const [runId, pendingRef] of this.pendingForkRefByRun) {
      if (pendingRef !== reference || runId === resolvedRunId) {
        continue;
      }
      const node = this.runIndex.get(runId);
      const forked = this.runIndex.get(resolvedRunId);
      if (node && forked) {
        node.forkOf = resolvedRunId;
        this.moveParent(node, forked.parentRunId);
      }
      this.pendingForkRefByRun.delete(runId);
    }
  }

  private moveParent(
    node: RunNode<TProjection>,
    parentRunId: string | undefined,
    propagateForkChildren = true,
  ): void {
    if (node.parentRunId !== undefined) {
      this.parentIndex.get(node.parentRunId)?.delete(node.runId);
    } else {
      this.rootRuns.delete(node.runId);
    }
    if (parentRunId === undefined) {
      this.rootRuns.add(node.runId);
      delete node.parentRunId;
      if (propagateForkChildren) {
        this.moveForkChildrenToParent(node, new Set());
      }
      return;
    }
    node.parentRunId = parentRunId;
    let children = this.parentIndex.get(parentRunId);
    if (!children) {
      children = new Set();
      this.parentIndex.set(parentRunId, children);
    }
    children.add(node.runId);
    if (propagateForkChildren) {
      this.moveForkChildrenToParent(node, new Set());
    }
  }

  private moveRegenerateAnchor(node: RunNode<TProjection>, anchor: string): void {
    if (node.regeneratesCodecMessageId !== undefined) {
      this.regenerateByMsgId.get(node.regeneratesCodecMessageId)?.delete(node.runId);
    }
    node.regeneratesCodecMessageId = anchor;
    let turns = this.regenerateByMsgId.get(anchor);
    if (!turns) {
      turns = new Set();
      this.regenerateByMsgId.set(anchor, turns);
    }
    turns.add(node.runId);
  }

  private moveForkChildrenToParent(node: RunNode<TProjection>, seen: Set<string>): void {
    if (seen.has(node.runId)) {
      return;
    }
    seen.add(node.runId);
    for (const candidate of this.runIndex.values()) {
      if (candidate.runId !== node.runId && candidate.forkOf === node.runId) {
        this.moveParent(candidate, node.parentRunId, false);
        this.moveForkChildrenToParent(candidate, seen);
      }
    }
  }

  private removeRunAndDescendants(runId: string): void {
    const children = this.parentIndex.get(runId);
    if (children) {
      for (const child of Array.from(children)) {
        this.removeRunAndDescendants(child);
      }
    }
    const node = this.runIndex.get(runId);
    if (!node) {
      return;
    }
    if (node.parentRunId !== undefined) {
      this.parentIndex.get(node.parentRunId)?.delete(runId);
    } else {
      this.rootRuns.delete(runId);
    }
    if (node.regeneratesCodecMessageId !== undefined) {
      this.regenerateByMsgId.get(node.regeneratesCodecMessageId)?.delete(runId);
    }
    for (const codecMessageId of this.runCodecMessageIds.get(runId) ?? []) {
      this.codecMessageIdToRunId.delete(codecMessageId);
      this.headersByCodecMessageId.delete(codecMessageId);
    }
    this.runCodecMessageIds.delete(runId);
    this.runIndex.delete(runId);
    this.pendingParentRefByRun.delete(runId);
    this.pendingForkRefByRun.delete(runId);
    this.parentIndex.delete(runId);
    const index = this.sortedRuns.indexOf(runId);
    if (index !== -1) {
      this.sortedRuns.splice(index, 1);
    }
    this.latestContinuationInvocation.delete(runId);
  }

  private computeSiblingRuns(node: RunNode<TProjection>): RunNode<TProjection>[] {
    const candidateIds =
      node.parentRunId === undefined ? this.rootRuns : this.parentIndex.get(node.parentRunId);
    if (!candidateIds) {
      return [node];
    }
    const candidateSet = new Set(candidateIds);
    const group = new Set<string>([node.runId]);
    let changed = true;
    while (changed) {
      changed = false;
      for (const candidateId of candidateSet) {
        if (group.has(candidateId) || this.isDescendant(candidateId, node.runId)) {
          continue;
        }
        const candidate = this.runIndex.get(candidateId);
        if (!candidate) {
          continue;
        }
        if (
          (candidate.forkOf !== undefined && group.has(candidate.forkOf)) ||
          (node.forkOf !== undefined && candidateId === node.forkOf) ||
          this.forkChainTouches(candidate, group)
        ) {
          group.add(candidateId);
          changed = true;
        }
      }
    }
    return this.nodesInSortOrder(group);
  }

  private computeRegenerateGroup(codecMessageId: string): RunNode<TProjection>[] {
    const nodes: RunNode<TProjection>[] = [];
    const owner = this.getNodeByCodecMessageId(codecMessageId);
    if (owner) {
      nodes.push(owner);
    }
    const regenerates = this.regenerateByMsgId.get(codecMessageId);
    if (regenerates) {
      for (const node of this.nodesInSortOrder(regenerates)) {
        if (node.runId !== owner?.runId) {
          nodes.push(node);
        }
      }
    }
    return nodes;
  }

  private forkChainTouches(node: RunNode<TProjection>, group: Set<string>): boolean {
    const seen = new Set<string>();
    let current: string | undefined = node.forkOf;
    while (current !== undefined && !seen.has(current)) {
      if (group.has(current)) {
        return true;
      }
      seen.add(current);
      current = this.runIndex.get(current)?.forkOf;
    }
    return false;
  }

  private isDescendant(candidateId: string, ancestorId: string): boolean {
    const seen = new Set<string>();
    let current = this.runIndex.get(candidateId)?.parentRunId;
    while (current !== undefined && !seen.has(current)) {
      if (current === ancestorId) {
        return true;
      }
      seen.add(current);
      current = this.runIndex.get(current)?.parentRunId;
    }
    return false;
  }

  private nodesInSortOrder(ids: Set<string>): RunNode<TProjection>[] {
    const nodes: RunNode<TProjection>[] = [];
    for (const runId of this.sortedRuns) {
      if (ids.has(runId)) {
        const node = this.runIndex.get(runId);
        if (node) {
          nodes.push(node);
        }
      }
    }
    return nodes;
  }

  private resolveNode(id: string): RunNode<TProjection> | undefined {
    return this.runIndex.get(id) ?? this.getNodeByCodecMessageId(id);
  }

  private resolveReference(reference: string | undefined): string | undefined {
    if (reference === undefined) {
      return undefined;
    }
    return this.codecMessageIdToRunId.get(reference) ?? this.runIndex.get(reference)?.runId;
  }

  private sortRuns(): void {
    this.sortedRuns.sort((left, right) => {
      const leftNode = this.runIndex.get(left);
      const rightNode = this.runIndex.get(right);
      return compareOptionalSerial(leftNode?.startSerial, rightNode?.startSerial);
    });
  }

  private bump(): void {
    this.version += 1;
    this.siblingCache.clear();
    this.regenerateCache.clear();
    this.emitter.emit("update", { structuralVersion: this.version });
  }
}

interface RunMetadata {
  parentRunId?: string;
  parentRef?: string;
  forkOf?: string;
  forkRef?: string;
  regeneratesCodecMessageId?: string;
  clientId?: string;
  invocationId?: string;
}

interface CachedNodes<TProjection> {
  version: number;
  nodes: readonly RunNode<TProjection>[];
}

function readRunReason(value: string | undefined): RunEndReason | undefined {
  switch (value) {
    case "complete":
    case "cancelled":
    case "error":
    case "suspended":
      return value;
    default:
      return undefined;
  }
}

function isTerminalStatus(status: RunStatus): boolean {
  return status === "complete" || status === "cancelled" || status === "error";
}

function isLiveStatus(status: RunStatus): boolean {
  return status === "active" || status === "suspended";
}

function compareOptionalSerial(
  left: TreeSerial | undefined,
  right: TreeSerial | undefined,
): -1 | 0 | 1 {
  if (left === undefined && right === undefined) {
    return 0;
  }
  if (left === undefined) {
    return 1;
  }
  if (right === undefined) {
    return -1;
  }
  return compareSerial(left, right);
}

function compareSerial(left: TreeSerial, right: TreeSerial): -1 | 0 | 1 {
  const leftBigInt = serialBigInt(left);
  const rightBigInt = serialBigInt(right);
  if (leftBigInt !== undefined && rightBigInt !== undefined) {
    return leftBigInt < rightBigInt ? -1 : leftBigInt > rightBigInt ? 1 : 0;
  }
  return String(left) < String(right) ? -1 : String(left) > String(right) ? 1 : 0;
}

function serialBigInt(value: TreeSerial): bigint | undefined {
  if (typeof value === "number") {
    return Number.isSafeInteger(value) ? BigInt(value) : undefined;
  }
  return /^\d+$/.test(value) ? BigInt(value) : undefined;
}

function setOptional<T extends object, K extends keyof T>(
  target: T,
  key: K,
  value: T[K] | undefined,
): void {
  if (value !== undefined) {
    target[key] = value;
  }
}

/**
 * Transport roles used by tree routing.
 */
export const treeRoutingRoles = {
  /** User input role. */
  user: "user",
  /** Assistant output role. */
  assistant: "assistant",
  /** Tool output role. */
  tool: "tool",
} as const satisfies Record<string, string>;

/**
 * Whether headers mark a run continuation.
 *
 * `run-continue` is the current key. `turn-continue` is accepted read-only so
 * history written before 3.0 still promotes a suspended run back to active
 * instead of silently stalling.
 */
function isContinuationHeaders(headers: HeaderMap): boolean {
  return (
    headers[HEADER_RUN_CONTINUE] === "true" ||
    headers[INBOUND_LEGACY_HEADER_TURN_CONTINUE] === "true"
  );
}

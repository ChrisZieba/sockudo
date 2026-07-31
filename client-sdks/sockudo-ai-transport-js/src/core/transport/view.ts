import { ErrorCode, ErrorInfo, toErrorInfo } from "../../errors.js";
import { EventEmitter, type EventUnsubscribe } from "../../event-emitter.js";
import type { Codec, Decoder } from "../codec/index.js";
import type { InboundMessage, PaginatedResult } from "../../realtime/types.js";
import { decodeHistoryPage, type HistoryReader } from "./decode-history.js";
import type { SendOptions } from "./client-transport.js";
import type { Tree, TreeMessageEvent, RunEndReason, RunNode } from "./tree.js";

/**
 * View-visible message metadata.
 */
export interface MessageMetadata {
  /** Codec message id. */
  codecMessageId: string;
  /** Owning turn id. */
  runId: string;
  /** Verified client id when known. */
  clientId?: string;
  /** Streaming state or terminal turn status. */
  status: "streaming" | RunEndReason;
}

/**
 * Branch selection intent.
 */
export type BranchSelectionIntent = "user" | "auto" | "pinned" | "pending";

/**
 * Send executor injected by P7 transport.
 */
export interface ViewSendExecutor<TInput, TMessage> {
  /** Sends a user message. */
  send(message: TMessage, options?: SendOptions): Promise<unknown>;
  /** Sends a codec input event. */
  sendInput(input: TInput | readonly TInput[], options?: SendOptions): Promise<unknown>;
  /** Requests regeneration. */
  regenerate(target: string, parent: string, options?: SendOptions): Promise<unknown>;
  /** Edits a message. */
  edit(messageId: string, message: TMessage, options?: SendOptions): Promise<unknown>;
  /** Updates a message. */
  update(messageId: string, patch: unknown, options?: SendOptions): Promise<unknown>;
}

/**
 * View event map.
 */
export interface ViewEvents<TMessage> {
  /** Visible output changed. */
  update: readonly TMessage[];
  /** Visible message turn projection changed. */
  message: TMessage;
  /** Visible turn changed. */
  turn: RunNode<unknown>;
}

/**
 * View construction options.
 */
export interface ViewOptions<TInput, TOutput, TProjection, TMessage> {
  /** Backing conversation tree. */
  tree: Tree<TInput | TOutput, TProjection>;
  /** Domain codec. */
  codec: Codec<TInput, TOutput, TProjection, TMessage>;
  /** Decoder used for history pages. */
  decoder?: Decoder<TInput, TOutput>;
  /** Optional paginated history source. */
  history?: HistoryReader;
  /** Optional P7 send executor. */
  sendExecutor?: ViewSendExecutor<TInput, TMessage>;
  /** Returns a stable message id. */
  getMessageId?(message: TMessage): string | undefined;
  /** Initially withheld turn ids for pagination tests and restored views. */
  withheldRunIds?: readonly string[];
}

/**
 * Public branch-aware view API.
 */
export interface View<TInput, TMessage> {
  /** Returns currently visible messages. */
  getMessages(): readonly TMessage[];
  /** Returns currently visible turn nodes. */
  flattenNodes(): readonly RunNode<unknown>[];
  /** Returns whether older turns may be loaded. */
  hasOlder(): boolean;
  /** Loads older turns or history. */
  loadOlder(limit?: number): Promise<void>;
  /** Selects a turn sibling by id and clamped index. */
  select(id: string, index: number, intent?: BranchSelectionIntent): void;
  /** Gets the selected turn sibling index. */
  getSelectedIndex(id: string): number;
  /** Gets sibling turns in serial-chronological order. */
  getSiblings(id: string): readonly RunNode<unknown>[];
  /** Returns whether a turn or message has turn siblings. */
  hasSiblings(id: string): boolean;
  /** Gets a turn node by turn id or codec message id. */
  getNode(id: string): RunNode<unknown> | undefined;
  /** Gets metadata for a visible or known message. */
  getMessageMetadata(msgId: string): MessageMetadata | undefined;
  /** Returns whether a message has branch siblings. */
  hasMessageSiblings(msgId: string): boolean;
  /** Gets message siblings anchored at an edit or regeneration slot. */
  getMessageSiblings(msgId: string): readonly TMessage[];
  /** Gets selected message sibling index. */
  getSelectedMessageSiblingIndex(msgId: string): number;
  /** Selects a message sibling. */
  selectMessageSibling(msgId: string, index: number, intent?: BranchSelectionIntent): void;
  /** Sends a user message. */
  send(message: TMessage, options?: SendOptions): Promise<unknown>;
  /** Sends a codec input. */
  sendInput(input: TInput | readonly TInput[], options?: SendOptions): Promise<unknown>;
  /** Requests regeneration. */
  regenerate(target: string, parent: string, options?: SendOptions): Promise<unknown>;
  /** Edits a message. */
  edit(messageId: string, message: TMessage, options?: SendOptions): Promise<unknown>;
  /** Updates a message. */
  update(messageId: string, patch: unknown, options?: SendOptions): Promise<unknown>;
  /** Subscribes to scoped view events. */
  on<K extends keyof ViewEvents<TMessage>>(
    event: K,
    handler: (payload: ViewEvents<TMessage>[K]) => void,
  ): EventUnsubscribe;
  /** Closes this view and unsubscribes from the tree. */
  close(): void;
  /** Whether a loadOlder operation is active. */
  readonly loading: boolean;
  /** Latest load error, if any. */
  readonly loadError: ErrorInfo | undefined;
}

/**
 * Creates a paginated branch-aware view over a conversation tree.
 */
export function createView<TInput, TOutput, TProjection, TMessage>(
  options: ViewOptions<TInput, TOutput, TProjection, TMessage>,
): View<TInput, TMessage> {
  return new ViewImpl(options);
}

class ViewImpl<TInput, TOutput, TProjection, TMessage> implements View<TInput, TMessage> {
  private readonly emitter = new EventEmitter<ViewEvents<TMessage>>();
  private readonly getMessageId: (message: TMessage) => string | undefined;
  private readonly siblingSelections = new Map<string, string>();
  private readonly siblingSelectionIntents = new Map<string, BranchSelectionIntent>();
  private readonly messageSelections = new Map<string, string>();
  private readonly messageSelectionIntents = new Map<string, BranchSelectionIntent>();
  private readonly withheldRunIds: string[];
  private readonly unsubscribes: EventUnsubscribe[];
  private cachedVersion = -1;
  private cachedNodes: RunNode<TProjection>[] = [];
  private cachedMessages: TMessage[] = [];
  private cachedMessageMetadata = new Map<string, MessageMetadata>();
  private cachedMessageObjects = new Map<string, TMessage>();
  private cachedRanges = new Map<string, MessageRange>();
  private historyPage: PaginatedResult<InboundMessage> | undefined;
  private historyExhausted = false;
  private firstHistoryLoad = true;
  private closed = false;
  private loadingValue = false;
  private loadErrorValue: ErrorInfo | undefined;

  public constructor(
    private readonly options: ViewOptions<TInput, TOutput, TProjection, TMessage>,
  ) {
    this.getMessageId = (message) => options.getMessageId?.(message) ?? defaultMessageId(message);
    this.withheldRunIds = [...(options.withheldRunIds ?? [])];
    this.unsubscribes = [
      options.tree.on("update", () => {
        this.handleStructuralUpdate();
      }),
      options.tree.on("turn-projection-updated", (event) => {
        this.handleProjectionUpdate(event);
      }),
      options.tree.on("turn", (node) => {
        this.handleRun(node);
      }),
    ];
    this.rebuild();
  }

  public get loading(): boolean {
    return this.loadingValue;
  }

  public get loadError(): ErrorInfo | undefined {
    return this.loadErrorValue;
  }

  public getMessages(): readonly TMessage[] {
    this.ensureFresh();
    return this.cachedMessages;
  }

  public flattenNodes(): readonly RunNode<unknown>[] {
    this.ensureFresh();
    return this.cachedNodes;
  }

  public hasOlder(): boolean {
    return (
      this.withheldRunIds.length > 0 ||
      (!this.historyExhausted && (this.historyPage === undefined || this.historyPage.hasNext()))
    );
  }

  public async loadOlder(limit = 100): Promise<void> {
    if (this.loadingValue || this.closed) {
      return;
    }
    this.loadingValue = true;
    this.loadErrorValue = undefined;
    this.emitUpdate();
    try {
      const target = Math.max(0, limit);
      let revealed = this.drainWithheld(target);
      while (
        revealed < target &&
        !this.historyExhausted &&
        this.options.history &&
        this.options.decoder
      ) {
        const page =
          this.historyPage === undefined
            ? await this.options.history.history({
                direction: "newest_first",
                limit: target * 10,
                untilAttach: this.firstHistoryLoad,
              })
            : this.historyPage.hasNext()
              ? await this.historyPage.next()
              : undefined;
        this.firstHistoryLoad = false;
        if (!page) {
          this.historyExhausted = true;
          break;
        }
        const before = this.options.tree.getRunNodes().length;
        decodeHistoryPage(page, this.options.decoder, this.options.tree);
        this.historyPage = page;
        if (!page.hasNext()) {
          this.historyExhausted = true;
        }
        const after = this.options.tree.getRunNodes().length;
        revealed += Math.max(0, after - before);
      }
      this.rebuild();
    } catch (error) {
      this.loadErrorValue = toErrorInfo(error, {
        code: ErrorCode.SessionSendFailed,
        message: "unable to load older messages; history read failed",
      });
    } finally {
      this.loadingValue = false;
      this.emitUpdate();
    }
  }

  public select(id: string, index: number, intent: BranchSelectionIntent = "user"): void {
    const siblings = this.getSiblings(id);
    if (siblings.length === 0) {
      return;
    }
    const selected = siblings[clamp(index, 0, siblings.length - 1)];
    if (!selected) {
      return;
    }
    const key = siblingGroupKey(siblings);
    this.siblingSelections.set(key, selected.runId);
    this.siblingSelectionIntents.set(key, intent);
    this.rebuild();
    this.emitUpdate();
  }

  public getSelectedIndex(id: string): number {
    const siblings = this.getSiblings(id);
    if (siblings.length === 0) {
      return 0;
    }
    const key = siblingGroupKey(siblings);
    const selected = this.selectedRunId(key, siblings);
    return Math.max(
      0,
      siblings.findIndex((node) => node.runId === selected),
    );
  }

  public getSiblings(id: string): readonly RunNode<unknown>[] {
    return this.options.tree.getSiblingNodes(id);
  }

  public hasSiblings(id: string): boolean {
    return this.getSiblings(id).length > 1;
  }

  public getNode(id: string): RunNode<unknown> | undefined {
    return this.options.tree.getRunNode(id) ?? this.options.tree.getNodeByCodecMessageId(id);
  }

  public getMessageMetadata(msgId: string): MessageMetadata | undefined {
    this.ensureFresh();
    return this.cachedMessageMetadata.get(msgId);
  }

  public hasMessageSiblings(msgId: string): boolean {
    return this.getMessageSiblings(msgId).length > 1;
  }

  public getMessageSiblings(msgId: string): readonly TMessage[] {
    this.ensureFresh();
    const regen = this.options.tree.getRegenerateGroup(msgId);
    if (regen.length > 1) {
      return regen.flatMap((node, index) =>
        index === 0 ? this.messageById(node, msgId) : this.firstMessage(node),
      );
    }
    const node = this.options.tree.getNodeByCodecMessageId(msgId);
    if (!node) {
      return [];
    }
    return this.options.tree
      .getSiblingNodes(node.runId)
      .flatMap((sibling) => this.firstMessage(sibling));
  }

  public getSelectedMessageSiblingIndex(msgId: string): number {
    const siblings = this.getMessageSiblings(msgId);
    if (siblings.length === 0) {
      return 0;
    }
    const selected = this.messageSelections.get(msgId);
    if (!selected) {
      return siblings.length - 1;
    }
    const index = siblings.findIndex((message) => this.getMessageId(message) === selected);
    return Math.max(0, index);
  }

  public selectMessageSibling(
    msgId: string,
    index: number,
    intent: BranchSelectionIntent = "user",
  ): void {
    const siblings = this.getMessageSiblings(msgId);
    if (siblings.length === 0) {
      return;
    }
    const selected = siblings[clamp(index, 0, siblings.length - 1)];
    const selectedId = selected === undefined ? undefined : this.getMessageId(selected);
    if (selectedId === undefined) {
      return;
    }
    this.messageSelections.set(msgId, selectedId);
    this.messageSelectionIntents.set(msgId, intent);
    this.rebuild();
    this.emitUpdate();
  }

  public send(message: TMessage, options?: SendOptions): Promise<unknown> {
    return this.callExecutor("send", () => this.requireExecutor().send(message, options));
  }

  public sendInput(input: TInput | readonly TInput[], options?: SendOptions): Promise<unknown> {
    return this.callExecutor("send input", () => this.requireExecutor().sendInput(input, options));
  }

  public regenerate(target: string, parent: string, options?: SendOptions): Promise<unknown> {
    this.messageSelectionIntents.set(target, "pending");
    return this.callExecutor("regenerate", () =>
      this.requireExecutor().regenerate(target, parent, options),
    );
  }

  public edit(messageId: string, message: TMessage, options?: SendOptions): Promise<unknown> {
    this.messageSelectionIntents.set(messageId, "pending");
    return this.callExecutor("edit message", () =>
      this.requireExecutor().edit(messageId, message, options),
    );
  }

  public update(messageId: string, patch: unknown, options?: SendOptions): Promise<unknown> {
    return this.callExecutor("update message", () =>
      this.requireExecutor().update(messageId, patch, options),
    );
  }

  public on<K extends keyof ViewEvents<TMessage>>(
    event: K,
    handler: (payload: ViewEvents<TMessage>[K]) => void,
  ): EventUnsubscribe {
    return this.emitter.on(event, handler);
  }

  public close(): void {
    if (this.closed) {
      return;
    }
    this.closed = true;
    for (const unsubscribe of this.unsubscribes) {
      unsubscribe();
    }
  }

  private ensureFresh(): void {
    if (this.cachedVersion !== this.options.tree.structuralVersion) {
      this.rebuild();
    }
  }

  private rebuild(): void {
    const nodes = this.flatten();
    const extraction = this.extractMessages(nodes);
    this.cachedNodes = nodes;
    this.cachedMessages = extraction.messages;
    this.cachedMessageMetadata = extraction.metadata;
    this.cachedMessageObjects = extraction.objects;
    this.cachedRanges = extraction.ranges;
    this.cachedVersion = this.options.tree.structuralVersion;
    this.repinSelections(nodes);
  }

  private flatten(): RunNode<TProjection>[] {
    const visible: RunNode<TProjection>[] = [];
    const reachable = new Set<string>();
    const withheld = new Set(this.withheldRunIds);
    for (const node of this.options.tree.getRunNodes()) {
      if (withheld.has(node.runId) || node.regeneratesCodecMessageId) {
        continue;
      }
      if (node.parentRunId !== undefined && !reachable.has(node.parentRunId)) {
        continue;
      }
      const siblings = this.options.tree
        .getSiblingNodes(node.runId)
        .filter((sibling) => !sibling.regeneratesCodecMessageId);
      if (siblings.length > 1) {
        const key = siblingGroupKey(siblings);
        const selected = this.selectedRunId(key, siblings);
        if (node.runId !== selected) {
          continue;
        }
      }
      visible.push(node);
      reachable.add(node.runId);
    }
    return visible;
  }

  private extractMessages(nodes: readonly RunNode<TProjection>[]): {
    messages: TMessage[];
    metadata: Map<string, MessageMetadata>;
    objects: Map<string, TMessage>;
    ranges: Map<string, MessageRange>;
  } {
    const messages: TMessage[] = [];
    const metadata = new Map<string, MessageMetadata>();
    const objects = new Map<string, TMessage>();
    const ranges = new Map<string, MessageRange>();
    for (const node of nodes) {
      const start = messages.length;
      const extracted = this.extractRunMessages(node, new Set());
      for (const message of extracted) {
        const id = this.getMessageId(message);
        if (id !== undefined) {
          metadata.set(id, this.metadataForMessage(id, node));
          objects.set(id, message);
        }
        messages.push(message);
      }
      ranges.set(node.runId, { start, end: messages.length });
    }
    return { messages, metadata, objects, ranges };
  }

  private extractRunMessages(node: RunNode<TProjection>, seenAnchors: Set<string>): TMessage[] {
    const source = this.options.codec.getMessages(node.projection);
    const output: TMessage[] = [];
    for (const message of source) {
      const id = this.getMessageId(message);
      if (id === undefined || seenAnchors.has(id)) {
        output.push(message);
        continue;
      }
      const selectedRegenerator = this.selectedRegenerator(id);
      if (!selectedRegenerator) {
        output.push(message);
        continue;
      }
      seenAnchors.add(id);
      output.push(...this.extractRunMessages(selectedRegenerator, seenAnchors));
      return output;
    }
    return output;
  }

  private selectedRegenerator(codecMessageId: string): RunNode<TProjection> | undefined {
    const group = this.options.tree.getRegenerateGroup(codecMessageId);
    if (group.length <= 1) {
      return undefined;
    }
    const selectedMessageId = this.messageSelections.get(codecMessageId);
    if (selectedMessageId !== undefined) {
      const selected = group.find((node) =>
        this.options.codec
          .getMessages(node.projection)
          .some((message) => this.getMessageId(message) === selectedMessageId),
      );
      return selected?.runId === group[0]?.runId ? undefined : selected;
    }
    return group[group.length - 1];
  }

  private metadataForMessage(
    codecMessageId: string,
    fallbackNode: RunNode<TProjection>,
  ): MessageMetadata {
    const owner = this.options.tree.getNodeByCodecMessageId(codecMessageId) ?? fallbackNode;
    const metadata: MessageMetadata = {
      codecMessageId,
      runId: owner.runId,
      status: owner.status === "active" ? "streaming" : owner.status,
    };
    if (owner.clientId !== undefined) {
      metadata.clientId = owner.clientId;
    }
    return metadata;
  }

  private firstMessage(node: RunNode<unknown>): TMessage[] {
    const messages = this.options.codec.getMessages(node.projection as TProjection);
    const first = messages[0];
    return first === undefined ? [] : [first];
  }

  private messageById(node: RunNode<unknown>, messageId: string): TMessage[] {
    return this.options.codec
      .getMessages(node.projection as TProjection)
      .filter((message) => this.getMessageId(message) === messageId)
      .slice(0, 1);
  }

  private selectedRunId(key: string, siblings: readonly RunNode<unknown>[]): string | undefined {
    const selected = this.siblingSelections.get(key);
    if (selected !== undefined && siblings.some((node) => node.runId === selected)) {
      return selected;
    }
    return siblings[siblings.length - 1]?.runId;
  }

  private repinSelections(nodes: readonly RunNode<TProjection>[]): void {
    for (const node of nodes) {
      const siblings = this.options.tree.getSiblingNodes(node.runId);
      if (siblings.length <= 1) {
        continue;
      }
      const key = siblingGroupKey(siblings);
      if (!this.siblingSelections.has(key)) {
        this.siblingSelections.set(key, node.runId);
        this.siblingSelectionIntents.set(key, "pinned");
      }
    }
  }

  private drainWithheld(limit: number): number {
    const count = Math.min(limit, this.withheldRunIds.length);
    this.withheldRunIds.splice(0, count);
    return count;
  }

  private handleStructuralUpdate(): void {
    const previous = this.cachedMessages;
    this.rebuild();
    if (!sameMessageRefs(previous, this.cachedMessages)) {
      this.emitUpdate();
    }
  }

  private handleProjectionUpdate(event: TreeMessageEvent<TInput | TOutput, TProjection>): void {
    const visibleNode = this.cachedNodes.find((node) => node.runId === event.runId);
    if (
      visibleNode &&
      !visibleNode.regeneratesCodecMessageId &&
      !this.runHasVisibleRegeneration(visibleNode)
    ) {
      this.patchRunMessages(visibleNode);
      const message =
        event.messageId === undefined ? undefined : this.cachedMessageObjects.get(event.messageId);
      if (message !== undefined) {
        this.emitter.emit("message", message);
      }
      return;
    }
    const previous = this.cachedMessages;
    this.rebuild();
    if (!sameMessageRefs(previous, this.cachedMessages)) {
      this.emitUpdate();
    }
    const message =
      event.messageId === undefined ? undefined : this.cachedMessageObjects.get(event.messageId);
    if (message !== undefined) {
      this.emitter.emit("message", message);
    }
  }

  private handleRun(node: RunNode<TProjection>): void {
    if (this.cachedNodes.some((visible) => visible.runId === node.runId)) {
      this.emitter.emit("turn", node);
    }
  }

  private emitUpdate(): void {
    this.emitter.emit("update", this.cachedMessages);
  }

  private patchRunMessages(node: RunNode<TProjection>): void {
    const range = this.cachedRanges.get(node.runId);
    if (!range) {
      this.rebuild();
      this.emitUpdate();
      return;
    }
    const nextMessages = this.extractRunMessages(node, new Set());
    const previous = this.cachedMessages.slice(range.start, range.end);
    if (sameMessageRefs(previous, nextMessages)) {
      return;
    }
    for (const message of previous) {
      const id = this.getMessageId(message);
      if (id !== undefined) {
        this.cachedMessageMetadata.delete(id);
        this.cachedMessageObjects.delete(id);
      }
    }
    this.cachedMessages.splice(range.start, range.end - range.start, ...nextMessages);
    const delta = nextMessages.length - (range.end - range.start);
    range.end = range.start + nextMessages.length;
    for (const [runId, otherRange] of this.cachedRanges) {
      if (runId !== node.runId && otherRange.start > range.start) {
        otherRange.start += delta;
        otherRange.end += delta;
      }
    }
    for (const message of nextMessages) {
      const id = this.getMessageId(message);
      if (id !== undefined) {
        this.cachedMessageMetadata.set(id, this.metadataForMessage(id, node));
        this.cachedMessageObjects.set(id, message);
      }
    }
    this.emitUpdate();
  }

  private runHasVisibleRegeneration(node: RunNode<TProjection>): boolean {
    return this.options.codec.getMessages(node.projection).some((message) => {
      const id = this.getMessageId(message);
      return id !== undefined && this.options.tree.getRegenerateGroup(id).length > 1;
    });
  }

  private requireExecutor(): ViewSendExecutor<TInput, TMessage> {
    if (!this.options.sendExecutor) {
      throw new ErrorInfo({
        code: ErrorCode.InvalidArgument,
        message: "unable to send from view; no send executor is configured",
      });
    }
    return this.options.sendExecutor;
  }

  private async callExecutor(operation: string, run: () => Promise<unknown>): Promise<unknown> {
    try {
      return await run();
    } catch (error) {
      throw toErrorInfo(error, {
        code: ErrorCode.SessionSendFailed,
        message: `unable to ${operation}; send executor failed`,
      });
    }
  }
}

function defaultMessageId(message: unknown): string | undefined {
  if (message !== null && typeof message === "object" && "id" in message) {
    const id = (message as { id?: unknown }).id;
    return typeof id === "string" ? id : undefined;
  }
  return undefined;
}

function siblingGroupKey(nodes: readonly RunNode<unknown>[]): string {
  return nodes
    .map((node) => node.runId)
    .sort()
    .join("\u0000");
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, Math.trunc(value)));
}

function sameMessageRefs<TMessage>(left: readonly TMessage[], right: readonly TMessage[]): boolean {
  if (left.length !== right.length) {
    return false;
  }
  for (let index = 0; index < left.length; index += 1) {
    if (left[index] !== right[index]) {
      return false;
    }
  }
  return true;
}

interface MessageRange {
  start: number;
  end: number;
}

// core
export { version } from "./version.js";
export { EVENT_AI_CANCEL, EVENT_AI_INPUT, EVENT_AI_OUTPUT, EVENT_AI_RUN_END, EVENT_AI_RUN_RESUME, EVENT_AI_RUN_START, EVENT_AI_RUN_SUSPEND, EVENT_AI_STEP_END, EVENT_AI_STEP_START, HEADER_CODEC_MESSAGE_ID, HEADER_DISCRETE, HEADER_ERROR_CODE, HEADER_ERROR_MESSAGE, HEADER_EVENT_ID, HEADER_FORK_OF, HEADER_INPUT_CLIENT_ID, HEADER_INPUT_CODEC_MESSAGE_ID, HEADER_INVOCATION_ID, HEADER_MSG_REGENERATE, HEADER_PARENT, HEADER_ROLE, HEADER_RUN_CLIENT_ID, HEADER_RUN_ID, HEADER_RUN_REASON, HEADER_STATUS, HEADER_STEER_CODEC_MESSAGE_IDS, HEADER_SUPERSEDES, HEADER_STEP_CLIENT_ID, HEADER_STEP_ID, HEADER_STEP_REASON, HEADER_STEP_START_SERIAL, HEADER_STREAM, HEADER_STREAM_ID, } from "./constants.js";
export { ErrorCode, ErrorInfo, errorInfoIs, formatErrorMessage, statusCodeForErrorCode, toErrorInfo, type ErrorInfoOptions, } from "./errors.js";
export { EventEmitter, type EventEmitterOptions, type EventUnsubscribe, type EventsMap, } from "./event-emitter.js";
export { LogLevel, consoleLogger, makeLogger, redactValue, type LogContext, type LogHandler, type Logger, type MakeLoggerOptions, } from "./logger.js";
export { buildTransportHeaders, getCodecHeaders, getTransportHeaders, headerReader, headerWriter, mergeHeaders, stripUndefined, type AiExtras, type BuildTransportHeadersOptions, type HeaderMap, } from "./utils.js";
export * from "./realtime/index.js";
export * from "./core/codec/index.js";
export * from "./core/transport/index.js";
//# sourceMappingURL=index.d.ts.map

// react
export { version } from "../version.js";
import { createElement, type ReactNode } from "react";
import { ErrorInfo } from "../errors.js";
import type { InboundMessage } from "../realtime/index.js";
import { type BranchSelectionIntent, type ClientSession, type ClientSessionOptions, type RunNode, type View } from "../core/transport/index.js";
/**
 * Provider props for the generic AI Transport React layer.
 *
 * Final provider stack: use `@sockudo/client/react`'s `SockudoProvider` as the
 * outer realtime client owner, then place this provider inside it with the
 * AI channel name and transport options. This package imports the peer only
 * through `src/realtime/react`, where the Sockudo client is adapted into the
 * realtime seam.
 *
 * Construction errors are caught and exposed by {@link useClientSession} as
 * `transportError`; children still render under the provider registry.
 */
export type ClientSessionProviderProps<TInput, TOutput, TProjection, TMessage> = Omit<ClientSessionOptions<TInput, TOutput, TProjection, TMessage>, "client" | "channel" | "channelName"> & {
    /**
     * Registry key and Sockudo channel name for unnamed hook lookups.
     *
     * @defaultValue No default; a channel name is required.
     */
    channelName: string;
    /** Child React tree. */
    children?: ReactNode;
};
/**
 * Options for {@link useClientSession}.
 *
 * Lookup failures return a throwing `InvalidArgument` stub and set
 * `transportError`; when `skip` is true, `transportError` is omitted.
 */
export interface UseClientSessionOptions {
    /**
     * Provider channel name.
     *
     * @defaultValue Nearest provider in the React tree.
     */
    channelName?: string;
    /**
     * Suppresses lookup and returns an inert throwing stub with no error.
     *
     * @defaultValue `false`.
     */
    skip?: boolean;
    /**
     * Subscribes to resolved transport errors.
     *
     * @defaultValue No callback.
     */
    onError?(error: ErrorInfo): void;
}
/**
 * Result returned by {@link useClientSession}.
 *
 * Missing, skipped, and failed providers expose a transport proxy that throws
 * {@link ErrorInfo} with {@link ErrorCode.InvalidArgument} on property access.
 */
export interface UseClientSessionResult<TInput, TOutput, TProjection, TMessage> {
    /** Resolved transport or a throwing stub. */
    session: ClientSession<TInput, TOutput, TProjection, TMessage>;
    /**
     * Provider construction or lookup error.
     *
     * @defaultValue `undefined` when a transport resolves or lookup is skipped.
     */
    sessionError?: ErrorInfo;
}
/**
 * Options for view hooks.
 *
 * Without a resolved view, methods that require one throw {@link ErrorInfo}
 * with {@link ErrorCode.InvalidArgument}; snapshot fields remain stable
 * empties for SSR and pre-mount renders.
 */
export interface UseViewOptions<TInput, TMessage> {
    /**
     * Explicit transport.
     *
     * @defaultValue Context transport.
     */
    session?: ClientSession<TInput, unknown, unknown, TMessage>;
    /**
     * Explicit view; wins over `transport`.
     *
     * @defaultValue Transport default view.
     */
    view?: View<TInput, TMessage>;
    /**
     * Auto-load page size once per view instance.
     *
     * @defaultValue No automatic load.
     */
    limit?: number;
    /**
     * Suppresses lookup and returns a stable empty handle.
     *
     * @defaultValue `false`.
     */
    skip?: boolean;
}
/**
 * Branch-aware view hook handle.
 *
 * Methods that need a live view throw {@link ErrorInfo} with
 * {@link ErrorCode.InvalidArgument} when no view is resolved.
 */
export interface ViewHandle<TMessage> {
    /** Current visible messages. */
    messages: readonly TMessage[];
    /** Current visible turn nodes. */
    nodes: readonly RunNode<unknown>[];
    /** Whether older messages can be loaded. */
    hasOlder: boolean;
    /** Whether a load operation is active. */
    loading: boolean;
    /** Latest load error. */
    loadError?: ErrorInfo;
    /** Loads older messages unless already loading. */
    loadOlder(limit?: number): Promise<void>;
    /** Selects a sibling branch. */
    select(id: string, index: number, intent?: BranchSelectionIntent): void;
    /** Gets selected sibling index. */
    getSelectedIndex(id: string): number;
    /** Gets sibling turn nodes. */
    getSiblings(id: string): readonly RunNode<unknown>[];
    /** Returns whether siblings exist. */
    hasSiblings(id: string): boolean;
    /** Gets a turn node by turn id or codec message id. */
    getNode(id: string): RunNode<unknown> | undefined;
    /** Sends a user message. */
    send(message: TMessage): Promise<unknown>;
    /** Requests regeneration. */
    regenerate(target: string, parent: string): Promise<unknown>;
    /** Edits a message. */
    edit(messageId: string, message: TMessage): Promise<unknown>;
    /** Updates a message. */
    update(messageId: string, patch: unknown): Promise<unknown>;
}
/**
 * Options for {@link useCreateView}.
 *
 * Without a resolved transport the hook returns the same stable empty handle as
 * {@link useView}; methods that need a live view throw `InvalidArgument`.
 */
export interface UseCreateViewOptions<TInput, TMessage> {
    /**
     * Explicit transport.
     *
     * @defaultValue Context transport.
     */
    session?: ClientSession<TInput, unknown, unknown, TMessage>;
    /**
     * Auto-load page size once per owned view instance.
     *
     * @defaultValue No automatic load.
     */
    limit?: number;
    /**
     * Suppresses view creation.
     *
     * @defaultValue `false`.
     */
    skip?: boolean;
}
/**
 * Options for tree hooks.
 *
 * Tree callbacks do not subscribe to tree changes and throw `InvalidArgument`
 * only when called without a resolved transport.
 */
export interface UseTreeOptions<TInput, TMessage> {
    /**
     * Explicit transport.
     *
     * @defaultValue Context transport.
     */
    session?: ClientSession<TInput, unknown, unknown, TMessage>;
}
/**
 * Stable tree helper handle.
 *
 * This handle intentionally does not re-render on tree changes; use
 * {@link useView} for reactive branch snapshots.
 */
export interface TreeHandle {
    /** Gets sibling turn nodes without subscribing to tree changes. */
    getSiblings(id: string): readonly RunNode<unknown>[];
    /** Returns whether siblings exist without subscribing to tree changes. */
    hasSiblings(id: string): boolean;
    /** Gets a turn node without subscribing to tree changes. */
    getNode(id: string): RunNode<unknown> | undefined;
}
/**
 * Options for {@link useActiveRuns}.
 *
 * Without a resolved transport, the hook returns a stable empty map.
 */
export interface UseActiveRunsOptions<TInput, TMessage> {
    /**
     * Explicit transport.
     *
     * @defaultValue Context transport.
     */
    session?: ClientSession<TInput, unknown, unknown, TMessage>;
}
/**
 * Options for {@link useSockudoMessages}.
 *
 * Without a resolved transport or when skipped, the hook returns a stable empty
 * list.
 */
export interface UseSockudoMessagesOptions<TInput, TMessage> {
    /**
     * Explicit transport.
     *
     * @defaultValue Context transport.
     */
    session?: ClientSession<TInput, unknown, unknown, TMessage>;
    /**
     * Suppresses subscription and returns a stable empty list.
     *
     * @defaultValue `false`.
     */
    skip?: boolean;
}
/**
 * Generic hook bundle returned by {@link createSessionHooks}.
 *
 * The bundled hooks share the same baked type parameters and error behavior as
 * the default exports.
 */
export interface SessionHooks<TInput, TOutput, TProjection, TMessage> {
    /** Provider for a channel-keyed transport registry. */
    ClientSessionProvider(props: ClientSessionProviderProps<TInput, TOutput, TProjection, TMessage>): ReturnType<typeof createElement>;
    /** Reads the nearest or named client transport. */
    useClientSession(options?: UseClientSessionOptions): UseClientSessionResult<TInput, TOutput, TProjection, TMessage>;
    /** Subscribes to a branch-aware view. */
    useView(options?: UseViewOptions<TInput, TMessage>): ViewHandle<TMessage>;
    /** Creates, owns, and subscribes to an additional view. */
    useCreateView(options?: UseCreateViewOptions<TInput, TMessage>): ViewHandle<TMessage>;
    /** Returns stable tree callbacks without re-rendering on tree changes. */
    useTree(options?: UseTreeOptions<TInput, TMessage>): TreeHandle;
    /** Subscribes to active/suspended turn ownership. */
    useActiveRuns(options?: UseActiveRunsOptions<TInput, TMessage>): Map<string, Set<string>>;
    /** Subscribes to raw normalized inbound messages. */
    useSockudoMessages(options?: UseSockudoMessagesOptions<TInput, TMessage>): readonly InboundMessage[];
}
/**
 * Creates generic React hooks for a specific codec/message type family.
 *
 * @defaultValue Type parameters default to `unknown`.
 *
 * Returned hooks throw only via synchronous handle/stub access for invalid
 * usage; async transport/view methods reject with {@link ErrorInfo}.
 */
export declare function createSessionHooks<TInput = unknown, TOutput = unknown, TProjection = unknown, TMessage = unknown>(): SessionHooks<TInput, TOutput, TProjection, TMessage>;
/**
 * Provides a channel-keyed AI client session using the outer
 * `@sockudo/client/react` `SockudoProvider`.
 *
 * @defaultValue No default `channelName`; the prop is required.
 *
 * Construction failures are caught and exposed through
 * {@link useClientSession}; children continue to render.
 */
export declare function ClientSessionProvider(props: ClientSessionProviderProps<unknown, unknown, unknown, unknown>): ReturnType<typeof createElement>;
/**
 * Reads the nearest or named AI client session.
 *
 * @defaultValue Uses the nearest provider when `channelName` is omitted.
 *
 * Missing, skipped, and failed providers return a throwing `InvalidArgument`
 * stub; `transportError` is set except when `skip` is true.
 */
export declare function useClientSession(options?: UseClientSessionOptions): UseClientSessionResult<unknown, unknown, unknown, unknown>;
/**
 * Subscribes to a branch-aware view and returns a reactive snapshot handle.
 *
 * @defaultValue Uses the context transport's default view.
 *
 * Snapshot fields are stable empties before mount; methods that require a
 * resolved view throw {@link ErrorInfo} with {@link ErrorCode.InvalidArgument}.
 */
export declare function useView(options?: UseViewOptions<unknown, unknown>): ViewHandle<unknown>;
/**
 * Creates and owns an additional view for the resolved transport.
 *
 * @defaultValue Uses the context transport and performs no automatic history
 * load unless `limit` is provided.
 *
 * The owned view is closed on unmount or transport change; unresolved transports
 * return the stable empty view handle.
 */
export declare function useCreateView(options?: UseCreateViewOptions<unknown, unknown>): ViewHandle<unknown>;
/**
 * Returns stable tree callbacks without subscribing to tree updates.
 *
 * @defaultValue Uses the context transport.
 *
 * Callback access throws `InvalidArgument` only when invoked without a resolved
 * transport.
 */
export declare function useTree(options?: UseTreeOptions<unknown, unknown>): TreeHandle;
/**
 * Subscribes to active/suspended turn ownership.
 *
 * @defaultValue Uses the context transport.
 *
 * Returns a new `Map<clientId, Set<runId>>` reference for each turn event and a
 * stable empty map without a resolved transport.
 */
export declare function useActiveRuns(options?: UseActiveRunsOptions<unknown, unknown>): Map<string, Set<string>>;
/**
 * Subscribes to the raw normalized Sockudo inbound message firehose.
 *
 * @defaultValue Uses the context transport and does not subscribe when `skip` is
 * true.
 *
 * The returned array is append-only for a transport instance and resets on
 * transport changes.
 */
export declare function useSockudoMessages(options?: UseSockudoMessagesOptions<unknown, unknown>): readonly InboundMessage[];
//# sourceMappingURL=index.d.ts.map

// vue
export { version } from "../version.js";
import { type ComputedRef, type InjectionKey, type Ref, type ShallowRef } from "vue";
import { ErrorInfo } from "../errors.js";
import type { ClientLike, InboundMessage } from "../realtime/index.js";
import { type BranchSelectionIntent, type ClientSession, type ClientSessionOptions, type RunNode, type View } from "../core/transport/index.js";
/**
 * Provider options for the generic AI Transport Vue layer.
 */
export type ClientSessionProviderOptions<TInput, TOutput, TProjection, TMessage> = Omit<ClientSessionOptions<TInput, TOutput, TProjection, TMessage>, "client" | "channel" | "channelName"> & {
    /** Registry key and Sockudo channel name. */
    channelName: string;
    /** Explicit realtime client. Defaults to `@sockudo/client/vue` context. */
    client?: ClientLike;
    /** Closes the transport when the current Vue effect scope is disposed.
     *
     * @defaultValue `true`.
     */
    closeOnScopeDispose?: boolean;
};
/**
 * Options for {@link useClientSession}.
 */
export interface UseClientSessionOptions {
    /** Provider channel name. Defaults to nearest provided transport. */
    channelName?: string;
    /** Suppresses lookup and returns empty refs. */
    skip?: boolean;
    /** Subscribes to resolved transport errors. */
    onError?(error: ErrorInfo): void;
}
/**
 * Result returned by {@link useClientSession}.
 */
export interface UseClientSessionResult<TInput, TOutput, TProjection, TMessage> {
    /** Resolved transport ref. */
    session: ShallowRef<ClientSession<TInput, TOutput, TProjection, TMessage> | undefined>;
    /** Provider construction or lookup error ref. */
    sessionError: ShallowRef<ErrorInfo | undefined>;
}
/**
 * Options for Vue view composables.
 */
export interface UseViewOptions<TInput, TMessage> {
    /** Explicit transport. */
    session?: ClientSession<TInput, unknown, unknown, TMessage>;
    /** Explicit view; wins over `transport`. */
    view?: View<TInput, TMessage>;
    /** Auto-load page size once per view instance. */
    limit?: number;
    /** Suppresses lookup and returns stable empty refs. */
    skip?: boolean;
}
/**
 * Vue branch-aware view handle.
 */
export interface ViewHandle<TMessage> {
    /** Current visible messages. */
    messages: Ref<readonly TMessage[]>;
    /** Current visible turn nodes. */
    nodes: Ref<readonly RunNode<unknown>[]>;
    /** Whether older messages can be loaded. */
    hasOlder: Ref<boolean>;
    /** Whether a load operation is active. */
    loading: Ref<boolean>;
    /** Latest load error. */
    loadError: Ref<ErrorInfo | undefined>;
    /** Loads older messages unless already loading. */
    loadOlder(limit?: number): Promise<void>;
    /** Selects a sibling branch. */
    select(id: string, index: number, intent?: BranchSelectionIntent): void;
    /** Gets selected sibling index. */
    getSelectedIndex(id: string): number;
    /** Gets sibling turn nodes. */
    getSiblings(id: string): readonly RunNode<unknown>[];
    /** Returns whether siblings exist. */
    hasSiblings(id: string): boolean;
    /** Gets a turn node by turn id or codec message id. */
    getNode(id: string): RunNode<unknown> | undefined;
    /** Sends a user message. */
    send(message: TMessage): Promise<unknown>;
    /** Requests regeneration. */
    regenerate(target: string, parent: string): Promise<unknown>;
    /** Edits a message. */
    edit(messageId: string, message: TMessage): Promise<unknown>;
    /** Updates a message. */
    update(messageId: string, patch: unknown): Promise<unknown>;
}
/**
 * Options for {@link useCreateView}.
 */
export interface UseCreateViewOptions<TInput, TMessage> {
    /** Explicit transport. Defaults to context transport. */
    session?: ClientSession<TInput, unknown, unknown, TMessage>;
    /** Auto-load page size once per owned view instance. */
    limit?: number;
    /** Suppresses view creation. */
    skip?: boolean;
}
/**
 * Options for tree composables.
 */
export interface UseTreeOptions<TInput, TMessage> {
    /** Explicit transport. Defaults to context transport. */
    session?: ClientSession<TInput, unknown, unknown, TMessage>;
}
/**
 * Stable tree helper handle.
 */
export interface TreeHandle {
    /** Gets sibling turn nodes without subscribing to tree changes. */
    getSiblings(id: string): readonly RunNode<unknown>[];
    /** Returns whether siblings exist without subscribing to tree changes. */
    hasSiblings(id: string): boolean;
    /** Gets a turn node without subscribing to tree changes. */
    getNode(id: string): RunNode<unknown> | undefined;
}
/**
 * Options for active-turn subscriptions.
 */
export interface UseActiveRunsOptions<TInput, TMessage> {
    /** Explicit transport. Defaults to context transport. */
    session?: ClientSession<TInput, unknown, unknown, TMessage>;
}
/**
 * Options for raw Sockudo message subscriptions.
 */
export interface UseSockudoMessagesOptions<TInput, TMessage> {
    /** Explicit transport. Defaults to context transport. */
    session?: ClientSession<TInput, unknown, unknown, TMessage>;
    /** Suppresses subscription and returns a stable empty list. */
    skip?: boolean;
}
/**
 * Vue session scope returned by {@link createSessionScope}.
 */
export interface SessionScope<TInput, TOutput, TProjection, TMessage> {
    /** Provides a channel-keyed transport registry. */
    provideSession(options: ClientSessionProviderOptions<TInput, TOutput, TProjection, TMessage>): UseClientSessionResult<TInput, TOutput, TProjection, TMessage>;
    /** Reads the nearest or named client transport. */
    useClientSession(options?: UseClientSessionOptions): UseClientSessionResult<TInput, TOutput, TProjection, TMessage>;
    /** Subscribes to a branch-aware view. */
    useView(options?: UseViewOptions<TInput, TMessage>): ViewHandle<TMessage>;
    /** Creates, owns, and subscribes to an additional view. */
    useCreateView(options?: UseCreateViewOptions<TInput, TMessage>): ViewHandle<TMessage>;
    /** Returns stable tree callbacks without re-rendering on tree changes. */
    useTree(options?: UseTreeOptions<TInput, TMessage>): TreeHandle;
    /** Subscribes to active/suspended turn ownership. */
    useActiveRuns(options?: UseActiveRunsOptions<TInput, TMessage>): ComputedRef<Map<string, Set<string>>>;
    /** Subscribes to raw normalized inbound messages. */
    useSockudoMessages(options?: UseSockudoMessagesOptions<TInput, TMessage>): Ref<readonly InboundMessage[]>;
}
interface SessionSlot {
    session: ShallowRef<ClientSession<unknown, unknown, unknown, unknown> | undefined>;
    sessionError: ShallowRef<ErrorInfo | undefined>;
}
interface SessionRegistry {
    defaultChannelName?: string;
    slots: Map<string, SessionSlot>;
}
/**
 * Creates generic Vue composables for a specific codec/message type family.
 */
export declare function createSessionScope<TInput = unknown, TOutput = unknown, TProjection = unknown, TMessage = unknown>(key?: InjectionKey<SessionRegistry>): SessionScope<TInput, TOutput, TProjection, TMessage>;
/**
 * Provides a channel-keyed AI client session using `@sockudo/client/vue`.
 */
export declare function provideSession(options: ClientSessionProviderOptions<unknown, unknown, unknown, unknown>): UseClientSessionResult<unknown, unknown, unknown, unknown>;
/**
 * Reads the nearest or named Vue client session.
 */
export declare function useClientSession(options?: UseClientSessionOptions): UseClientSessionResult<unknown, unknown, unknown, unknown>;
/**
 * Subscribes to a branch-aware view and returns reactive refs.
 */
export declare function useView(options?: UseViewOptions<unknown, unknown>): ViewHandle<unknown>;
/**
 * Creates and owns an additional view for the resolved transport.
 */
export declare function useCreateView(options?: UseCreateViewOptions<unknown, unknown>): ViewHandle<unknown>;
/**
 * Returns stable tree callbacks without subscribing to tree updates.
 */
export declare function useTree(options?: UseTreeOptions<unknown, unknown>): TreeHandle;
/**
 * Subscribes to active/suspended turn ownership.
 */
export declare function useActiveRuns(options?: UseActiveRunsOptions<unknown, unknown>): ComputedRef<Map<string, Set<string>>>;
/**
 * Subscribes to the raw normalized Sockudo inbound message firehose.
 */
export declare function useSockudoMessages(options?: UseSockudoMessagesOptions<unknown, unknown>): Ref<readonly InboundMessage[]>;
//# sourceMappingURL=index.d.ts.map

// svelte
export { version } from "../version.js";
import { type Readable } from "svelte/store";
import { ErrorInfo } from "../errors.js";
import type { InboundMessage } from "../realtime/index.js";
import { type BranchSelectionIntent, type ClientSession, type ClientSessionOptions, type RunNode, type View } from "../core/transport/index.js";
/**
 * Svelte session store options.
 */
export type SessionStoreOptions<TInput, TOutput, TProjection, TMessage> = ClientSessionOptions<TInput, TOutput, TProjection, TMessage> & {
    /** Closes the transport when the current Svelte component is destroyed.
     *
     * @defaultValue `true`.
     */
    closeOnDestroy?: boolean;
};
/**
 * Options for {@link getClientSession}.
 */
export interface GetClientSessionOptions {
    /** Provider channel name. Defaults to nearest context transport. */
    channelName?: string;
    /** Suppresses lookup and returns empty state. */
    skip?: boolean;
    /** Subscribes to resolved transport errors. */
    onError?(error: ErrorInfo): void;
}
/**
 * Svelte session state.
 */
export interface ClientSessionState<TInput, TOutput, TProjection, TMessage> {
    /** Resolved transport. */
    session?: ClientSession<TInput, TOutput, TProjection, TMessage>;
    /** Provider construction or lookup error. */
    sessionError?: ErrorInfo;
}
/**
 * Svelte session store.
 */
export interface ClientSessionStore<TInput, TOutput, TProjection, TMessage> extends Readable<ClientSessionState<TInput, TOutput, TProjection, TMessage>> {
    /** Channel registry key. */
    readonly channelName?: string;
    /** Closes the transport if it exists. */
    close(): Promise<void>;
}
/**
 * Options for Svelte view stores.
 */
export interface ViewStoreOptions<TInput, TMessage> {
    /** Explicit transport. */
    session?: ClientSession<TInput, unknown, unknown, TMessage> | Readable<ClientSessionState<TInput, unknown, unknown, TMessage>>;
    /** Explicit view; wins over `transport`. */
    view?: View<TInput, TMessage>;
    /** Auto-load page size once per view instance. */
    limit?: number;
    /** Suppresses lookup and returns stable empty state. */
    skip?: boolean;
}
/**
 * Svelte view state.
 */
export interface ViewState<TMessage> {
    /** Current visible messages. */
    messages: readonly TMessage[];
    /** Current visible turn nodes. */
    nodes: readonly RunNode<unknown>[];
    /** Whether older messages can be loaded. */
    hasOlder: boolean;
    /** Whether a load operation is active. */
    loading: boolean;
    /** Latest load error. */
    loadError?: ErrorInfo;
}
/**
 * Svelte branch-aware view store.
 */
export interface ViewStore<TMessage> extends Readable<ViewState<TMessage>> {
    /** Loads older messages unless already loading. */
    loadOlder(limit?: number): Promise<void>;
    /** Selects a sibling branch. */
    select(id: string, index: number, intent?: BranchSelectionIntent): void;
    /** Gets selected sibling index. */
    getSelectedIndex(id: string): number;
    /** Gets sibling turn nodes. */
    getSiblings(id: string): readonly RunNode<unknown>[];
    /** Returns whether siblings exist. */
    hasSiblings(id: string): boolean;
    /** Gets a turn node by turn id or codec message id. */
    getNode(id: string): RunNode<unknown> | undefined;
    /** Sends a user message. */
    send(message: TMessage): Promise<unknown>;
    /** Requests regeneration. */
    regenerate(target: string, parent: string): Promise<unknown>;
    /** Edits a message. */
    edit(messageId: string, message: TMessage): Promise<unknown>;
    /** Updates a message. */
    update(messageId: string, patch: unknown): Promise<unknown>;
    /** Closes an owned view. */
    close(): void;
}
/**
 * Options for active-turn subscriptions.
 */
export interface ActiveRunsStoreOptions<TInput, TMessage> {
    /** Explicit transport. Defaults to context transport. */
    session?: ClientSession<TInput, unknown, unknown, TMessage> | Readable<ClientSessionState<TInput, unknown, unknown, TMessage>>;
}
/**
 * Options for raw Sockudo message subscriptions.
 */
export interface SockudoMessagesStoreOptions<TInput, TMessage> {
    /** Explicit transport. Defaults to context transport. */
    session?: ClientSession<TInput, unknown, unknown, TMessage> | Readable<ClientSessionState<TInput, unknown, unknown, TMessage>>;
    /** Suppresses subscription and returns a stable empty list. */
    skip?: boolean;
}
/**
 * Creates a Svelte readable store that owns one client session.
 */
export declare function createSessionStore<TInput = unknown, TOutput = unknown, TProjection = unknown, TMessage = unknown>(options: SessionStoreOptions<TInput, TOutput, TProjection, TMessage>): ClientSessionStore<TInput, TOutput, TProjection, TMessage>;
/**
 * Sets the Svelte session context for child components.
 */
export declare function setSessionContext<TInput, TOutput, TProjection, TMessage>(store: ClientSessionStore<TInput, TOutput, TProjection, TMessage>): ClientSessionStore<TInput, TOutput, TProjection, TMessage>;
/**
 * Creates, stores, and provides a Svelte session in one call.
 */
export declare function provideSession<TInput = unknown, TOutput = unknown, TProjection = unknown, TMessage = unknown>(options: SessionStoreOptions<TInput, TOutput, TProjection, TMessage>): ClientSessionStore<TInput, TOutput, TProjection, TMessage>;
/**
 * Reads the nearest or named Svelte client session store.
 */
export declare function getClientSession<TInput = unknown, TOutput = unknown, TProjection = unknown, TMessage = unknown>(options?: GetClientSessionOptions): Readable<ClientSessionState<TInput, TOutput, TProjection, TMessage>>;
/**
 * Creates a branch-aware Svelte view store.
 */
export declare function createViewStore<TInput = unknown, TMessage = unknown>(options?: ViewStoreOptions<TInput, TMessage>): ViewStore<TMessage>;
/**
 * Creates and owns an additional branch-aware Svelte view store.
 */
export declare function createOwnedViewStore<TInput = unknown, TMessage = unknown>(options?: Omit<ViewStoreOptions<TInput, TMessage>, "view">): ViewStore<TMessage>;
/**
 * Creates stable tree callbacks for a Svelte session.
 */
export declare function createTreeHandle<TInput = unknown, TMessage = unknown>(options?: ActiveRunsStoreOptions<TInput, TMessage>): {
    /** Gets sibling turn nodes without subscribing to tree changes. */
    getSiblings(id: string): readonly RunNode<unknown>[];
    /** Returns whether siblings exist without subscribing to tree changes. */
    hasSiblings(id: string): boolean;
    /** Gets a turn node without subscribing to tree changes. */
    getNode(id: string): RunNode<unknown> | undefined;
};
/**
 * Subscribes to active/suspended turn ownership.
 */
export declare function createActiveRunsStore<TInput = unknown, TMessage = unknown>(options?: ActiveRunsStoreOptions<TInput, TMessage>): Readable<Map<string, Set<string>>>;
/**
 * Subscribes to the raw normalized Sockudo inbound message firehose.
 */
export declare function createSockudoMessagesStore<TInput = unknown, TMessage = unknown>(options?: SockudoMessagesStoreOptions<TInput, TMessage>): Readable<readonly InboundMessage[]>;
//# sourceMappingURL=index.d.ts.map

// vercel
export { version } from "../version.js";
import { type ClientSession, type ClientSessionOptions, type AgentSession, type AgentSessionOptions } from "../core/transport/index.js";
export * from "./codec/index.js";
import { type AI, type VercelInput, type VercelOutput, type VercelProjection } from "./codec/index.js";
/**
 * Client session options for Vercel UI messages.
 *
 * @defaultValue `api` defaults to `"/api/chat"`.
 */
export type VercelClientSessionOptions = Omit<ClientSessionOptions<VercelInput, VercelOutput, VercelProjection, AI.UIMessage>, "api" | "codec"> & {
    /** Server endpoint URL for the HTTP poke.
     *
     * @defaultValue `"/api/chat"`.
     */
    api?: string;
};
/**
 * Agent session options for Vercel UI messages.
 */
export type VercelAgentSessionOptions = Omit<AgentSessionOptions<VercelInput, VercelOutput, VercelProjection, AI.UIMessage>, "codec">;
/**
 * Creates a Sockudo client session pre-bound to {@link UIMessageCodec}.
 *
 * Async methods reject with `ErrorInfo`; synchronous misuse throws `ErrorInfo`
 * with `InvalidArgument`.
 */
export declare function createClientSession(options: VercelClientSessionOptions): ClientSession<VercelInput, VercelOutput, VercelProjection, AI.UIMessage>;
/**
 * Creates a Sockudo agent session pre-bound to {@link UIMessageCodec}.
 *
 * Public methods reject with `ErrorInfo`; synchronous misuse throws
 * `ErrorInfo` with `InvalidArgument`.
 */
export declare function createAgentSession(options: VercelAgentSessionOptions): AgentSession<VercelOutput, VercelProjection, AI.UIMessage>;
export * from "./transport/index.js";
//# sourceMappingURL=index.d.ts.map

// vercel/react
export { version } from "../../version.js";
import { createElement, type ReactNode } from "react";
import { ErrorInfo } from "../../errors.js";
import { type SessionHooks, type ClientSessionProviderProps, type TreeHandle, type UseActiveRunsOptions, type UseClientSessionOptions, type UseClientSessionResult, type UseCreateViewOptions, type UseSockudoMessagesOptions, type UseTreeOptions, type UseViewOptions, type ViewHandle } from "../../react/index.js";
import type { InboundMessage } from "../../realtime/index.js";
import type { ClientSession } from "../../core/transport/index.js";
import { type ChatTransport, type ChatTransportOptions } from "../transport/index.js";
import { type AI, type VercelInput, type VercelOutput, type VercelProjection } from "../codec/index.js";
type VercelSession = ClientSession<VercelInput, VercelOutput, VercelProjection, AI.UIMessage>;
type MessageSetter = (value: readonly AI.UIMessage[] | ((messages: readonly AI.UIMessage[]) => readonly AI.UIMessage[])) => void;
/**
 * Provider props for the Vercel `useChat` transport layer.
 *
 * `chatOptions` is captured by `useMemo([transport, chatOptions])`; callers
 * should pass a referentially stable object to avoid replacing the chat
 * transport between renders.
 *
 * @defaultValue `api` defaults to `"/api/chat"`.
 */
export type ChatTransportProviderProps = Omit<ClientSessionProviderProps<VercelInput, VercelOutput, VercelProjection, AI.UIMessage>, "api" | "codec"> & {
    /** Server endpoint URL for the route handler.
     *
     * @defaultValue `"/api/chat"`.
     */
    api?: string;
    /** Optional Vercel chat adapter hooks; keep this object stable. */
    chatOptions?: ChatTransportOptions;
    /** Child React tree. */
    children?: ReactNode;
};
/**
 * Options for {@link useChatTransport}.
 *
 * Missing, skipped, and failed providers return throwing stubs. Error fields
 * are set except when `skip` is true.
 */
export interface UseChatTransportOptions {
    /**
     * Provider channel name.
     *
     * @defaultValue Nearest chat transport provider.
     */
    channelName?: string;
    /**
     * Suppresses lookup and returns throwing stubs with no error fields.
     *
     * @defaultValue `false`.
     */
    skip?: boolean;
}
/**
 * Result returned by {@link useChatTransport}.
 *
 * Stub access throws {@link ErrorInfo} with
 * {@link ErrorCode.InvalidArgument}; async methods on real transports reject
 * with {@link ErrorInfo}.
 */
export interface UseChatTransportResult {
    /** Resolved Vercel chat transport or a throwing stub. */
    chatTransport: ChatTransport;
    /** Resolved underlying client transport or a throwing stub. */
    session: VercelSession;
    /**
     * Chat transport lookup or construction error.
     *
     * @defaultValue `undefined` when resolved or skipped.
     */
    chatTransportError?: ErrorInfo;
    /**
     * Underlying client session lookup or construction error.
     *
     * @defaultValue `undefined` when resolved or skipped.
     */
    sessionError?: ErrorInfo;
}
/**
 * Options for {@link useMessageSync}.
 */
export interface UseMessageSyncOptions {
    /** Vercel `useChat` `setMessages` function. */
    setMessages: MessageSetter;
    /**
     * Provider channel name.
     *
     * @defaultValue Nearest chat transport provider.
     */
    channelName?: string;
    /**
     * Suppresses subscriptions.
     *
     * @defaultValue `false`.
     */
    skip?: boolean;
}
/**
 * Creates a Vercel-typed generic transport hook bundle.
 *
 * @defaultValue Type parameters are fixed to the Vercel UIMessage codec.
 */
export declare function createSessionHooks(): SessionHooks<VercelInput, VercelOutput, VercelProjection, AI.UIMessage>;
/**
 * Provides a Vercel `ChatTransport` and the underlying Vercel-typed client
 * transport for one Sockudo channel.
 *
 * This component wraps the generic {@link ClientSessionProvider} with
 * {@link UIMessageCodec}. It does not close the chat transport on unmount; the
 * underlying generic transport provider owns lifecycle and strict-mode cleanup.
 */
export declare function ChatTransportProvider(props: ChatTransportProviderProps): ReturnType<typeof createElement>;
/**
 * Reads the nearest or named Vercel chat transport.
 *
 * @defaultValue Uses the nearest provider when `channelName` is omitted.
 *
 * Missing, skipped, and failed providers return throwing stubs; error fields are
 * omitted only when skipped.
 */
export declare function useChatTransport(options?: UseChatTransportOptions): UseChatTransportResult;
/**
 * Synchronizes Sockudo view updates into Vercel `useChat` state.
 *
 * While this client owns an active stream, synchronization is suppressed to
 * avoid Vercel optimistic id replacement flicker. When streaming transitions to
 * `false`, a sync runs immediately.
 */
export declare function useMessageSync(options: UseMessageSyncOptions): void;
/**
 * Merges Sockudo tree messages with Vercel's local optimistic overlay.
 *
 * Tree message order is preserved. Assistant tool-resolution parts from the
 * overlay win only over matching unresolved tree tool parts, preserving the
 * tree part's `type`. Overlay messages unknown to the tree are appended.
 */
export declare function mergeMessages(treeMessages: readonly AI.UIMessage[], overlayMessages: readonly AI.UIMessage[]): readonly AI.UIMessage[];
/**
 * Reads the nearest or named Vercel client session.
 *
 * @defaultValue Uses the nearest provider when `channelName` is omitted.
 */
export declare function useClientSession(options?: UseClientSessionOptions): UseClientSessionResult<VercelInput, VercelOutput, VercelProjection, AI.UIMessage>;
/**
 * Subscribes to a Vercel UIMessage view.
 *
 * @defaultValue Uses the context transport's default view.
 */
export declare function useView(options?: UseViewOptions<VercelInput, AI.UIMessage>): ViewHandle<AI.UIMessage>;
/**
 * Creates and owns an additional Vercel UIMessage view.
 *
 * @defaultValue Uses the context transport.
 */
export declare function useCreateView(options?: UseCreateViewOptions<VercelInput, AI.UIMessage>): ViewHandle<AI.UIMessage>;
/**
 * Returns stable tree callbacks for the Vercel transport.
 *
 * @defaultValue Uses the context transport.
 */
export declare function useTree(options?: UseTreeOptions<VercelInput, AI.UIMessage>): TreeHandle;
/**
 * Subscribes to active/suspended Vercel turn ownership.
 *
 * @defaultValue Uses the context transport.
 */
export declare function useActiveRuns(options?: UseActiveRunsOptions<VercelInput, AI.UIMessage>): Map<string, Set<string>>;
/**
 * Subscribes to raw normalized Sockudo inbound messages for the Vercel
 * transport.
 *
 * @defaultValue Uses the context transport.
 */
export declare function useSockudoMessages(options?: UseSockudoMessagesOptions<VercelInput, AI.UIMessage>): readonly InboundMessage[];
//# sourceMappingURL=index.d.ts.map

// vercel/vue
export { version } from "../../version.js";
import { type ComputedRef, type Ref, type ShallowRef } from "vue";
import { ErrorInfo } from "../../errors.js";
import { type ClientSessionProviderOptions, type UseActiveRunsOptions, type UseClientSessionOptions, type UseClientSessionResult, type UseCreateViewOptions, type UseSockudoMessagesOptions, type UseTreeOptions, type UseViewOptions, type ViewHandle, type TreeHandle } from "../../vue/index.js";
import type { InboundMessage } from "../../realtime/index.js";
import type { ClientSession } from "../../core/transport/index.js";
import { type ChatTransport, type ChatTransportOptions } from "../transport/index.js";
import { type AI, type VercelInput, type VercelOutput, type VercelProjection } from "../codec/index.js";
type VercelSession = ClientSession<VercelInput, VercelOutput, VercelProjection, AI.UIMessage>;
/**
 * Provider options for the Vercel AI SDK Vue session layer.
 */
export type ChatTransportProviderOptions = Omit<ClientSessionProviderOptions<VercelInput, VercelOutput, VercelProjection, AI.UIMessage>, "api" | "codec"> & {
    /** Server endpoint URL for the route handler.
     *
     * @defaultValue `"/api/chat"`.
     */
    api?: string;
    /** Optional Vercel chat adapter hooks. */
    chatOptions?: ChatTransportOptions;
};
/**
 * Result returned by {@link provideChatTransport} and {@link useChatTransport}.
 */
export interface UseChatTransportResult {
    /** Resolved Vercel chat transport ref. */
    chatTransport: ShallowRef<ChatTransport | undefined>;
    /** Resolved underlying client transport ref. */
    session: ShallowRef<VercelSession | undefined>;
    /** Chat transport lookup or construction error ref. */
    chatTransportError: ShallowRef<ErrorInfo | undefined>;
    /** Underlying client transport lookup or construction error ref. */
    sessionError: ShallowRef<ErrorInfo | undefined>;
}
/**
 * Provides a Vercel `ChatTransport` and underlying Vercel-typed client
 * transport for one Sockudo channel.
 */
export declare function provideChatTransport(options: ChatTransportProviderOptions): UseChatTransportResult;
/**
 * Reads the nearest or named Vercel chat transport.
 */
export declare function useChatTransport(options?: UseClientSessionOptions): UseChatTransportResult;
/**
 * Creates a Vercel-typed generic transport scope.
 */
export declare function createSessionScope(): import("../../vue/index.js").SessionScope<VercelInput, AI.UIMessageChunk, VercelProjection, AI.UIMessage>;
/**
 * Reads the nearest or named Vercel client session.
 */
export declare function useClientSession(options?: UseClientSessionOptions): UseClientSessionResult<VercelInput, VercelOutput, VercelProjection, AI.UIMessage>;
/**
 * Subscribes to a Vercel UIMessage view.
 */
export declare function useView(options?: UseViewOptions<VercelInput, AI.UIMessage>): ViewHandle<AI.UIMessage>;
/**
 * Creates and owns an additional Vercel UIMessage view.
 */
export declare function useCreateView(options?: UseCreateViewOptions<VercelInput, AI.UIMessage>): ViewHandle<AI.UIMessage>;
/**
 * Returns stable tree callbacks for the Vercel transport.
 */
export declare function useTree(options?: UseTreeOptions<VercelInput, AI.UIMessage>): TreeHandle;
/**
 * Subscribes to active/suspended Vercel turn ownership.
 */
export declare function useActiveRuns(options?: UseActiveRunsOptions<VercelInput, AI.UIMessage>): ComputedRef<Map<string, Set<string>>>;
/**
 * Subscribes to raw normalized Sockudo inbound messages for the Vercel
 * transport.
 */
export declare function useSockudoMessages(options?: UseSockudoMessagesOptions<VercelInput, AI.UIMessage>): Ref<readonly InboundMessage[]>;
//# sourceMappingURL=index.d.ts.map

// vercel/svelte
export { version } from "../../version.js";
import { type Readable } from "svelte/store";
import { ErrorInfo } from "../../errors.js";
import { type ActiveRunsStoreOptions, type ClientSessionState, type ClientSessionStore, type GetClientSessionOptions, type SockudoMessagesStoreOptions, type SessionStoreOptions, type ViewStore, type ViewStoreOptions } from "../../svelte/index.js";
import { type ChatTransport, type ChatTransportOptions } from "../transport/index.js";
import { type AI, type VercelInput, type VercelOutput, type VercelProjection } from "../codec/index.js";
/**
 * Svelte store options for the Vercel AI SDK transport layer.
 */
export type ChatTransportStoreOptions = Omit<SessionStoreOptions<VercelInput, VercelOutput, VercelProjection, AI.UIMessage>, "api" | "codec"> & {
    /** Server endpoint URL for the route handler.
     *
     * @defaultValue `"/api/chat"`.
     */
    api?: string;
    /** Optional Vercel chat adapter hooks. */
    chatOptions?: ChatTransportOptions;
};
/**
 * Svelte Vercel chat transport state.
 */
export interface ChatTransportState {
    /** Resolved Vercel chat transport. */
    chatTransport?: ChatTransport;
    /** Resolved underlying client session. */
    session?: ClientSessionState<VercelInput, VercelOutput, VercelProjection, AI.UIMessage>["session"];
    /** Chat transport lookup or construction error. */
    chatTransportError?: ErrorInfo;
    /** Underlying client transport lookup or construction error. */
    sessionError?: ErrorInfo;
}
/**
 * Svelte Vercel chat transport store.
 */
export interface ChatTransportStore extends Readable<ChatTransportState> {
    /** Channel registry key. */
    readonly channelName?: string;
    /** Closes the chat and client transport. */
    close(): Promise<void>;
}
/**
 * Creates a Sockudo-backed Vercel `ChatTransport` Svelte store.
 */
export declare function createChatTransportStore(options: ChatTransportStoreOptions): ChatTransportStore;
/**
 * Creates, stores, and provides a Vercel chat transport in one call.
 */
export declare function provideChatTransport(options: ChatTransportStoreOptions): ChatTransportStore;
/**
 * Reads the nearest or named Vercel chat transport store.
 */
export declare function getChatTransport(options?: GetClientSessionOptions): Readable<ChatTransportState>;
/**
 * Creates a Vercel-typed client session store.
 */
export declare function createClientSessionStore(options: Omit<SessionStoreOptions<VercelInput, VercelOutput, VercelProjection, AI.UIMessage>, "codec">): ClientSessionStore<VercelInput, VercelOutput, VercelProjection, AI.UIMessage>;
/**
 * Creates a Vercel UIMessage view store.
 */
export declare function createViewStore(options?: ViewStoreOptions<VercelInput, AI.UIMessage>): ViewStore<AI.UIMessage>;
/**
 * Creates and owns an additional Vercel UIMessage view store.
 */
export declare function createOwnedViewStore(options?: Omit<ViewStoreOptions<VercelInput, AI.UIMessage>, "view">): ViewStore<AI.UIMessage>;
/**
 * Creates stable tree callbacks for the Vercel transport.
 */
export declare function createTreeHandle(options?: ActiveRunsStoreOptions<VercelInput, AI.UIMessage>): {
    getSiblings(id: string): readonly import("../../index.js").RunNode<unknown>[];
    hasSiblings(id: string): boolean;
    getNode(id: string): import("../../index.js").RunNode<unknown> | undefined;
};
/**
 * Subscribes to active/suspended Vercel turn ownership.
 */
export declare function createActiveRunsStore(options?: ActiveRunsStoreOptions<VercelInput, AI.UIMessage>): Readable<Map<string, Set<string>>>;
/**
 * Subscribes to raw normalized Sockudo inbound messages for the Vercel
 * transport.
 */
export declare function createSockudoMessagesStore(options?: SockudoMessagesStoreOptions<VercelInput, AI.UIMessage>): Readable<readonly import("../../index.js").InboundMessage[]>;
//# sourceMappingURL=index.d.ts.map

// providers
export { version } from "../version.js";
import type { StreamResult, AgentRun, RunEndReason } from "../core/transport/index.js";
import type { AI, VercelOutput, VercelProjection } from "../vercel/codec/index.js";
/**
 * Well-known OpenAI-compatible provider identifiers.
 *
 * These providers expose a Chat Completions-compatible streaming endpoint and
 * can be used through {@link streamOpenAICompatibleText} without a provider SDK.
 */
export type OpenAICompatibleProviderName = "openai" | "openrouter" | "groq" | "togetherai" | "fireworks" | "deepseek" | "perplexity" | "mistral" | "xai" | "ollama" | "lmstudio";
/**
 * Default base URLs for high-traffic OpenAI-compatible providers.
 */
export declare const OPENAI_COMPATIBLE_PROVIDER_BASE_URLS: {
    readonly openai: "https://api.openai.com/v1";
    readonly openrouter: "https://openrouter.ai/api/v1";
    readonly groq: "https://api.groq.com/openai/v1";
    readonly togetherai: "https://api.together.xyz/v1";
    readonly fireworks: "https://api.fireworks.ai/inference/v1";
    readonly deepseek: "https://api.deepseek.com";
    readonly perplexity: "https://api.perplexity.ai";
    readonly mistral: "https://api.mistral.ai/v1";
    readonly xai: "https://api.x.ai/v1";
    readonly ollama: "http://127.0.0.1:11434/v1";
    readonly lmstudio: "http://127.0.0.1:1234/v1";
};
/**
 * Chat message shape accepted by OpenAI-compatible chat completion endpoints.
 */
export interface OpenAICompatibleChatMessage {
    /** Message role. */
    role: "system" | "user" | "assistant" | "tool";
    /** Provider-specific message content. */
    content?: unknown;
    /** Optional tool call id for tool result messages. */
    tool_call_id?: string;
    /** Provider-specific extra fields. */
    [key: string]: unknown;
}
/**
 * Shared text-generation request accepted by direct provider adapters.
 */
export interface ProviderTextRequest {
    /** Provider model id. */
    model: string;
    /** Simple user prompt. Ignored when `messages` is supplied. */
    prompt?: string;
    /** OpenAI-compatible chat history. */
    messages?: readonly OpenAICompatibleChatMessage[];
    /** Maximum generated tokens. */
    maxOutputTokens?: number;
    /** Sampling temperature. */
    temperature?: number;
    /** Nucleus sampling value. */
    topP?: number;
    /** Provider-specific request fields. */
    body?: Record<string, unknown>;
    /** Provider-specific headers. */
    headers?: Record<string, string | undefined>;
    /** Abort signal. */
    signal?: AbortSignal;
    /** Stable assistant message id for emitted UI chunks. */
    messageId?: string;
}
/**
 * OpenAI-compatible HTTP streaming options.
 */
export interface OpenAICompatibleStreamOptions extends ProviderTextRequest {
    /** Provider preset. Ignored when `baseURL` is supplied. */
    provider?: OpenAICompatibleProviderName;
    /** Base URL ending before `/chat/completions`. */
    baseURL?: string;
    /** Bearer token. Optional for local providers such as Ollama and LM Studio. */
    apiKey?: string;
    /** Endpoint path.
     *
     * @defaultValue `"/chat/completions"`.
     */
    path?: string;
    /** Fetch implementation.
     *
     * @defaultValue `globalThis.fetch`.
     */
    fetch?: typeof globalThis.fetch;
}
/**
 * Structural subset of the official OpenAI SDK used for Chat Completions.
 */
export interface OpenAIChatCompletionsClient {
    /** Chat completions namespace. */
    chat: {
        completions: {
            /** Creates a streaming chat completion. */
            create(request: Record<string, unknown> & {
                stream: true;
            }): Promise<AsyncIterable<unknown>>;
        };
    };
}
/**
 * Structural subset of the official OpenAI SDK used for Responses.
 */
export interface OpenAIResponsesClient {
    /** Responses namespace. */
    responses: {
        /** Creates a streaming response. */
        create(request: Record<string, unknown> & {
            stream: true;
        }): Promise<AsyncIterable<unknown>>;
    };
}
/**
 * OpenAI SDK Chat Completions stream options.
 */
export interface OpenAIChatCompletionStreamOptions extends ProviderTextRequest {
    /** Official OpenAI SDK client or structural equivalent. */
    client: OpenAIChatCompletionsClient;
}
/**
 * OpenAI SDK Responses stream options.
 */
export interface OpenAIResponseStreamOptions extends ProviderTextRequest {
    /** Official OpenAI SDK client or structural equivalent. */
    client: OpenAIResponsesClient;
    /** Raw Responses API `input` override. */
    input?: unknown;
}
/**
 * Structural subset of the official Anthropic SDK messages client.
 */
export interface AnthropicMessagesClient {
    /** Messages namespace. */
    messages: {
        /** Creates a streaming Anthropic messages response. */
        create(request: Record<string, unknown> & {
            stream: true;
        }): Promise<AsyncIterable<unknown>>;
    };
}
/**
 * Anthropic SDK message stream options.
 */
export interface AnthropicMessageStreamOptions extends ProviderTextRequest {
    /** Official Anthropic SDK client or structural equivalent. */
    client: AnthropicMessagesClient;
    /** Anthropic system prompt. */
    system?: string;
}
/**
 * Minimal direct LLM provider contract.
 */
export interface DirectLlmProvider {
    /** Streams a provider response as Vercel UI message chunks for Sockudo. */
    streamText(request: ProviderTextRequest): Promise<ReadableStream<VercelOutput>>;
}
/**
 * Provider registry returned by {@link createDirectLlmProviderRegistry}.
 */
export interface DirectLlmProviderRegistry {
    /** Resolves a provider by name. */
    get(name: string): DirectLlmProvider | undefined;
    /** Streams with a named provider. */
    streamText(name: string, request: ProviderTextRequest): Promise<ReadableStream<VercelOutput>>;
}
/**
 * Result returned by {@link runDirectLlm}.
 */
export interface RunDirectLlmResult {
    /** Pipe result from `turn.streamResponse`. */
    pipeResult: StreamResult;
    /** Published turn end reason. */
    runEndReason: RunEndReason;
}
/**
 * Streams text through a Chat Completions-compatible HTTP endpoint.
 */
export declare function streamOpenAICompatibleText(options: OpenAICompatibleStreamOptions): Promise<ReadableStream<VercelOutput>>;
/**
 * Streams with the official OpenAI SDK Chat Completions API.
 */
export declare function streamOpenAIChatCompletion(options: OpenAIChatCompletionStreamOptions): Promise<ReadableStream<VercelOutput>>;
/**
 * Streams with the official OpenAI SDK Responses API.
 */
export declare function streamOpenAIResponse(options: OpenAIResponseStreamOptions): Promise<ReadableStream<VercelOutput>>;
/**
 * Streams with the official Anthropic SDK Messages API.
 */
export declare function streamAnthropicMessage(options: AnthropicMessageStreamOptions): Promise<ReadableStream<VercelOutput>>;
/**
 * Creates a reusable OpenAI-compatible HTTP provider.
 */
export declare function createOpenAICompatibleProvider(defaults: Omit<OpenAICompatibleStreamOptions, "model"> & {
    model?: string;
}): DirectLlmProvider;
/**
 * Creates a reusable OpenAI SDK provider.
 */
export declare function createOpenAISdkProvider(defaults: (Omit<OpenAIChatCompletionStreamOptions, "model"> & {
    mode?: "chat";
    model?: string;
}) | (Omit<OpenAIResponseStreamOptions, "model"> & {
    mode: "responses";
    model?: string;
})): DirectLlmProvider;
/**
 * Creates a reusable Anthropic SDK provider.
 */
export declare function createAnthropicSdkProvider(defaults: Omit<AnthropicMessageStreamOptions, "model"> & {
    model?: string;
}): DirectLlmProvider;
/**
 * Creates a named direct-provider registry.
 */
export declare function createDirectLlmProviderRegistry(providers: Record<string, DirectLlmProvider>): DirectLlmProviderRegistry;
/**
 * Runs a Sockudo server run from a direct provider stream.
 *
 * This helper starts the run, streams provider chunks through
 * `turn.streamResponse`, maps completion to a run end reason, publishes
 * `ai-run-end`, and returns the evidence.
 */
export declare function runDirectLlm(turn: AgentRun<VercelOutput, VercelProjection, AI.UIMessage>, provider: DirectLlmProvider, request: ProviderTextRequest): Promise<RunDirectLlmResult>;
/**
 * Maps OpenAI Chat Completions stream events into UI message chunks.
 */
export declare function openAIChatCompletionEventsToUIMessageStream(events: AsyncIterable<unknown>, options?: {
    messageId?: string;
}): ReadableStream<VercelOutput>;
/**
 * Maps OpenAI Responses stream events into UI message chunks.
 */
export declare function openAIResponseEventsToUIMessageStream(events: AsyncIterable<unknown>, options?: {
    messageId?: string;
}): ReadableStream<VercelOutput>;
/**
 * Maps Anthropic Messages stream events into UI message chunks.
 */
export declare function anthropicMessageEventsToUIMessageStream(events: AsyncIterable<unknown>, options?: {
    messageId?: string;
}): ReadableStream<VercelOutput>;
//# sourceMappingURL=index.d.ts.map

// via re-export: constants.d.ts
/** Client-to-agent AI input event name. */
export declare const EVENT_AI_INPUT = "ai-input";
/** Agent-to-client AI output event name. */
export declare const EVENT_AI_OUTPUT = "ai-output";
/** AI run lifecycle start event name. */
export declare const EVENT_AI_RUN_START = "ai-run-start";
/** AI run lifecycle suspend event name. */
export declare const EVENT_AI_RUN_SUSPEND = "ai-run-suspend";
/** AI run lifecycle resume event name. */
export declare const EVENT_AI_RUN_RESUME = "ai-run-resume";
/** AI run lifecycle end event name. */
export declare const EVENT_AI_RUN_END = "ai-run-end";
/** AI cancellation event name. */
export declare const EVENT_AI_CANCEL = "ai-cancel";
/**
 * AI step lifecycle start event name. A step is a re-attemptable unit of
 * execution inside a run; a retry publishes a fresh `ai-step-start` under the
 * same `step-id`, and the highest `step-start-serial` is the canonical attempt.
 */
export declare const EVENT_AI_STEP_START = "ai-step-start";
/** AI step lifecycle end event name. Closes one attempt of a step. */
export declare const EVENT_AI_STEP_END = "ai-step-end";
/**
 * Legacy inbound-only AI turn lifecycle event names.
 *
 * Retained for wire tolerance, not naming: channels written before 3.0 may
 * still contain these, and history hydration must not silently drop them. The
 * SDK never publishes them. Deliberately not re-exported from the package
 * barrel — they are not part of the public API.
 */
export declare const INBOUND_LEGACY_EVENT_TURN_START = "ai-turn-start";
export declare const INBOUND_LEGACY_EVENT_TURN_END = "ai-turn-end";
/** Transport header key for run identity. */
export declare const HEADER_RUN_ID = "run-id";
/** Transport header key for verified run client identity. */
export declare const HEADER_RUN_CLIENT_ID = "run-client-id";
/** Transport header key for run end reason. */
export declare const HEADER_RUN_REASON = "run-reason";
/**
 * Legacy inbound-only transport header keys.
 *
 * Wire tolerance for pre-3.0 channel history, as with the legacy event names
 * above. Never written. Not part of the public API.
 *
 * `turn-continue` is here rather than deleted because the *data* path still
 * needs the flag; only its name was legacy. See {@link HEADER_RUN_CONTINUE}.
 */
export declare const INBOUND_LEGACY_HEADER_TURN_ID = "turn-id";
export declare const INBOUND_LEGACY_HEADER_TURN_CLIENT_ID = "turn-client-id";
export declare const INBOUND_LEGACY_HEADER_TURN_REASON = "turn-reason";
export declare const INBOUND_LEGACY_HEADER_TURN_CONTINUE = "turn-continue";
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
export declare const HEADER_RUN_CONTINUE = "run-continue";
/** Transport header key for step identity, stable across retry attempts. */
export declare const HEADER_STEP_ID = "step-id";
/**
 * Transport header key back-referencing the serial of the `ai-step-start` that
 * opened the attempt. Carried on `ai-output` and `ai-step-end` only — never on
 * `ai-step-start`, whose own channel serial *is* the value.
 */
export declare const HEADER_STEP_START_SERIAL = "step-start-serial";
/** Transport header key for step end reason: `complete`, `failed`, `cancelled`. */
export declare const HEADER_STEP_REASON = "step-reason";
/** Transport header key for the verified client identity that owns a step. */
export declare const HEADER_STEP_CLIENT_ID = "step-client-id";
/**
 * Transport header key stamping which steers the agent had drained when the
 * step attempt that produced this output opened. JSON-stringified array;
 * omitted when empty.
 */
export declare const HEADER_STEER_CODEC_MESSAGE_IDS = "steer-codec-message-ids";
/** Transport header key for invocation identity. */
export declare const HEADER_INVOCATION_ID = "invocation-id";
/** Transport header key for input event identity. */
export declare const HEADER_EVENT_ID = "event-id";
/** Transport header key for codec message identity. */
export declare const HEADER_CODEC_MESSAGE_ID = "codec-message-id";
/** Transport header key for the input codec message targeted by a cancel signal. */
export declare const HEADER_INPUT_CODEC_MESSAGE_ID = "input-codec-message-id";
/** Transport header key indicating streaming content. */
export declare const HEADER_STREAM = "stream";
/** Transport header key for stream identity. */
export declare const HEADER_STREAM_ID = "stream-id";
/** Transport header key for stream status. */
export declare const HEADER_STATUS = "status";
/** Transport header key indicating discrete content. */
export declare const HEADER_DISCRETE = "discrete";
/** Transport header key for message role. */
export declare const HEADER_ROLE = "role";
/** Transport header key for parent codec message identity. */
export declare const HEADER_PARENT = "parent";
/** Transport header key for fork source codec message identity. */
export declare const HEADER_FORK_OF = "fork-of";
/** Transport header key indicating regeneration. */
export declare const HEADER_MSG_REGENERATE = "msg-regenerate";
/**
 * Transport header key for the suspended run replaced by a client tool-result
 * fork. Superseded runs remain addressable in history but are hidden from
 * branch selection.
 */
export declare const HEADER_SUPERSEDES = "supersedes";
/** Transport header key for stream error code. */
export declare const HEADER_ERROR_CODE = "error-code";
/** Transport header key for stream error message. */
export declare const HEADER_ERROR_MESSAGE = "error-message";
/** Transport header key for verified input client identity. */
export declare const HEADER_INPUT_CLIENT_ID = "input-client-id";
//# sourceMappingURL=constants.d.ts.map

// via re-export: core/codec/decoder.d.ts
import type { HeaderMap } from "../../utils.js";
import type { InboundMessage } from "../../realtime/types.js";
import type { DecodedEvent } from "./types.js";
/**
 * Stream tracker passed to decoder hooks.
 */
export interface DecoderStreamTracker {
    /** Codec message id, equal to Sockudo `message_serial`. */
    messageId: string;
    /** Latest accumulated stream content. */
    accumulated: string;
    /** Whether this tracker was synthesized from an append/update first contact. */
    firstContact: boolean;
    /** Last decoded inbound message. */
    message: InboundMessage;
}
/**
 * Hooks used by the generic decoder core.
 */
export interface DecoderCoreHooks<TEvent> {
    /** Builds stream-start events. */
    buildStartEvents(tracker: DecoderStreamTracker): DecodedEvent<TEvent>[];
    /** Builds stream-delta events. */
    buildDeltaEvents(tracker: DecoderStreamTracker, delta: string): DecodedEvent<TEvent>[];
    /** Builds stream-end events. */
    buildEndEvents(tracker: DecoderStreamTracker, closingCodecHeaders: HeaderMap): DecodedEvent<TEvent>[];
    /** Decodes a non-stream discrete payload. */
    decodeDiscrete(message: InboundMessage): DecodedEvent<TEvent>[];
    /** Builds stream-delete events. */
    buildDeleteEvents?(tracker: DecoderStreamTracker): DecodedEvent<TEvent>[];
}
/**
 * Decoder-core observability hooks.
 */
export interface DecoderCoreMetrics {
    /** Called when an old stream tracker is evicted. */
    onTrackerEvicted?(messageId: string): void;
    /** Called when an append/update is first seen without its create. */
    onFirstContact?(messageId: string): void;
    /** Called when an update replaces non-prefix accumulated content. */
    onReplacement?(messageId: string): void;
    /** Called when a tracker is closed or deleted. */
    onTrackerClosed?(messageId: string): void;
}
/**
 * Decoder-core options.
 *
 * @defaultValue `maxStreams` is `1024`.
 */
export interface DecoderCoreOptions extends DecoderCoreMetrics {
    /** Maximum open stream trackers retained by the decoder. */
    maxStreams?: number;
}
/**
 * Generic decoder core.
 */
export interface DecoderCore<TEvent> {
    /** Decodes one normalized inbound message. */
    decode(message: InboundMessage): DecodedEvent<TEvent>[];
    /** Clears all tracked streams. */
    clear(): void;
}
/**
 * Creates a decoder core that understands Sockudo mutable-message stream frames.
 */
export declare function createDecoderCore<TEvent>(hooks: DecoderCoreHooks<TEvent>, options?: DecoderCoreOptions): DecoderCore<TEvent>;
//# sourceMappingURL=decoder.d.ts.map

// via re-export: core/codec/encoder.d.ts
import type { MessageAck } from "../../realtime/types.js";
import type { ChannelWriter, CodecHeaderSet, EncoderOptions, WriteOptions } from "./types.js";
/**
 * Options accepted by encoder-core write operations.
 */
export interface EncoderCoreWriteOptions extends WriteOptions, CodecHeaderSet {
    /** Event name for the create message. */
    name?: string;
    /** Operation id for mutation idempotency. */
    opId?: string;
}
/**
 * Core streaming encoder with Sockudo mutable-message recovery semantics.
 */
export interface EncoderCore {
    /** Publishes one discrete create. */
    publishDiscrete(payload: unknown, options?: EncoderCoreWriteOptions): Promise<MessageAck>;
    /** Publishes discrete creates sequentially. */
    publishDiscreteBatch(payloads: readonly unknown[], options?: EncoderCoreWriteOptions): Promise<MessageAck[]>;
    /** Starts a tracked stream and returns the codec message id. */
    startStream(streamId: string, payload: string, options?: EncoderCoreWriteOptions): Promise<string>;
    /** Appends stream data without awaiting the mutation. */
    appendStream(streamId: string, delta: string, options?: EncoderCoreWriteOptions): void;
    /** Completes a stream, flushing and recovering failed appends. */
    closeStream(streamId: string, options?: EncoderCoreWriteOptions): Promise<void>;
    /** Cancels a stream, flushing and recovering failed appends. */
    cancelStream(streamId: string, reason?: string, options?: EncoderCoreWriteOptions): Promise<void>;
    /** Cancels every active stream. */
    cancelAllStreams(reason?: string): Promise<void>;
    /** Closes all active streams as complete. */
    close(): Promise<void>;
}
/**
 * Creates the framework-agnostic encoder core.
 *
 * @defaultValue Stream append writes are fire-and-forget until close/cancel.
 */
export declare function createEncoderCore(writer: ChannelWriter, options?: EncoderOptions): EncoderCore;
//# sourceMappingURL=encoder.d.ts.map

// via re-export: core/codec/index.d.ts
export { createDecoderCore, type DecoderCore, type DecoderCoreHooks, type DecoderCoreMetrics, type DecoderCoreOptions, type DecoderStreamTracker, } from "./decoder.js";
export { createEncoderCore, type EncoderCore, type EncoderCoreWriteOptions } from "./encoder.js";
export { createLifecycleTracker, type LifecycleTracker, type PhaseConfig, } from "./lifecycle-tracker.js";
export { createAccumulator, type AssertChannelWriter, type ChannelWriter, type Codec, type Codec2, type CodecHeaderSet, type CreateAccumulatorOptions, type DecodedBatch, type DecodedEvent, type Decoder, type Encoder, type EncoderOptions, type EncoderOutboundMessage, type MessageAccumulator, type MessageProjection, type Reducer, type ReducerMeta, type Regenerate, type UserMessage, type WriteOptions, } from "./types.js";
//# sourceMappingURL=index.d.ts.map

// via re-export: core/codec/lifecycle-tracker.d.ts
/**
 * Lifecycle phase configuration.
 */
export interface PhaseConfig<TContext, TEvent> {
    /** Stable phase name. */
    name: string;
    /** Builds the synthetic event for this phase. */
    buildEvent(scopeId: string, context: TContext): TEvent;
}
/**
 * Per-scope phase synthesis helper.
 */
export interface LifecycleTracker<TContext, TEvent> {
    /** Emits missing phase events in configured order. */
    ensurePhases(scopeId: string, context: TContext): TEvent[];
    /** Marks a phase as already emitted. */
    markEmitted(scopeId: string, phase: string): void;
    /** Resets a phase and every phase after it for a scope. */
    resetPhase(scopeId: string, phase: string): void;
    /** Clears all phase state for a scope. */
    clearScope(scopeId: string): void;
}
/**
 * Creates a per-scope lifecycle tracker.
 */
export declare function createLifecycleTracker<TContext, TEvent>(phases: readonly PhaseConfig<TContext, TEvent>[]): LifecycleTracker<TContext, TEvent>;
//# sourceMappingURL=lifecycle-tracker.d.ts.map

// via re-export: core/codec/types.d.ts
import type { HeaderMap } from "../../utils.js";
import type { InboundMessage, MessageAck, MessageMutation, PublishMessage, Serial } from "../../realtime/types.js";
/**
 * Metadata supplied to codec reducers.
 */
export interface ReducerMeta {
    /** Serial used for deterministic idempotency. */
    serial: Serial;
    /** Optional codec message id, equal to Sockudo `message_serial` for streams. */
    messageId?: string;
}
/**
 * Deterministic event reducer used by codec projections.
 *
 * Implementations may mutate `state` and return the same object. Callers must
 * treat the returned value as authoritative.
 */
export interface Reducer<TEvent, TProjection> {
    /** Creates an empty projection. */
    init(): TProjection;
    /** Folds one event into `state`. */
    fold(state: TProjection, event: TEvent, meta: ReducerMeta): TProjection;
}
/**
 * Public codec contract for framework-agnostic AI Transport event handling.
 *
 * @defaultValue Reducers may mutate their projection for hot-path efficiency.
 */
export interface Codec<TInput, TOutput, TProjection, TMessage> extends Reducer<TInput | TOutput, TProjection> {
    /** Creates an encoder bound to a channel writer. */
    createEncoder(channel: ChannelWriter, options?: EncoderOptions): Encoder<TInput, TOutput>;
    /** Creates a decoder for inbound Sockudo messages. */
    createDecoder(): Decoder<TInput, TOutput>;
    /** Returns the user-visible messages from a projection. */
    getMessages(projection: TProjection): TMessage[];
    /** Wraps a user-authored message for transport submission. */
    createUserMessage(message: TMessage): UserMessage<TMessage>;
    /** Creates a regenerate command for a target and parent message. */
    createRegenerate(target: string, parent: string): Regenerate;
    /** Resolves a tool-call target from an output and current projection. */
    resolveToolTarget(output: TOutput, projection: TProjection): string | undefined;
    /** Returns whether an output terminates its stream or message. */
    isTerminal(output: TOutput): boolean;
}
/**
 * Docs-compatible two-generic codec alias.
 */
export type Codec2<TEvent, TMessage> = Codec<TEvent, TEvent, MessageProjection<TMessage>, TMessage>;
/**
 * Projection shape used by the docs-compatible codec alias.
 */
export interface MessageProjection<TMessage> {
    /** Mutable message list owned by the codec reducer. */
    messages: TMessage[];
}
/**
 * Encoded user message command.
 */
export interface UserMessage<TMessage> {
    /** Message payload. */
    message: TMessage;
}
/**
 * Encoded regenerate command.
 */
export interface Regenerate {
    /** Target message id. */
    target: string;
    /** Parent message id. */
    parent: string;
}
/**
 * Channel writer consumed by codec encoders.
 *
 * `ChannelLike` satisfies this interface structurally.
 */
export interface ChannelWriter {
    /** Publishes a create/discrete message. */
    publish(message: PublishMessage): Promise<MessageAck>;
    /** Appends string data to a mutable message. */
    appendMessage(messageSerial: string, data: string, options?: Omit<MessageMutation, "data">): Promise<MessageAck>;
    /** Updates a mutable message aggregate. */
    updateMessage(messageSerial: string, options?: MessageMutation): Promise<MessageAck>;
}
/**
 * Per-write options accepted by public encoders.
 */
export interface WriteOptions {
    /** Verified client id to pass through privileged write paths. */
    clientId?: string;
    /** Extras merged with AI transport and codec tiers. */
    extras?: unknown;
    /** Idempotent message id. */
    messageId?: string;
}
/**
 * Encoder construction options.
 *
 * @defaultValue `extras` defaults to no additional metadata.
 */
export interface EncoderOptions extends WriteOptions {
    /** Hook invoked before each write; exceptions are isolated. */
    onMessage?(message: EncoderOutboundMessage): void;
}
/**
 * Public encoder contract.
 */
export interface Encoder<TInput, TOutput> {
    /** Publishes an input event. */
    publishInput(input: TInput, options?: WriteOptions): Promise<MessageAck>;
    /** Publishes an output event. */
    publishOutput(output: TOutput, options?: WriteOptions): Promise<MessageAck>;
    /** Cancels active streams owned by this encoder. */
    cancel(reason?: string): Promise<void>;
    /** Closes the encoder and flushes active streams. */
    close(): Promise<void>;
}
/**
 * Public decoder contract.
 */
export interface Decoder<TInput, TOutput> {
    /** Decodes one inbound message into typed input and output events. */
    decode(message: InboundMessage): DecodedBatch<TInput, TOutput>;
}
/**
 * Decoded event plus deterministic fold metadata.
 */
export interface DecodedEvent<TEvent> {
    /** Decoded payload. */
    event: TEvent;
    /** Message serial used as the codec message id. */
    messageId?: string;
    /** Fold metadata. */
    meta: ReducerMeta;
}
/**
 * Decoder result split by event direction.
 */
export interface DecodedBatch<TInput, TOutput> {
    /** Decoded input events. */
    inputs: DecodedEvent<TInput>[];
    /** Decoded output events. */
    outputs: DecodedEvent<TOutput>[];
}
/**
 * Mutable outbound write envelope passed to encoder hooks.
 */
export interface EncoderOutboundMessage {
    /** Write kind. */
    kind: "publish" | "append" | "update";
    /** Mutable publish payload for create/discrete writes. */
    publish?: PublishMessage;
    /** Target mutable message serial for append/update writes. */
    messageSerial?: string;
    /** Append data. */
    data?: string;
    /** Mutable mutation options for append/update writes. */
    mutation?: MessageMutation | Omit<MessageMutation, "data">;
}
/**
 * Message accumulator compatible with the documented Ably codec shape.
 */
export interface MessageAccumulator<TOutput, TMessage> {
    /** Current visible messages. */
    readonly messages: TMessage[];
    /** Messages that have received a terminal output. */
    readonly completedMessages: TMessage[];
    /** Whether at least one stream is active. */
    readonly hasActiveStream: boolean;
    /** Processes decoded output events through the underlying codec reducer. */
    processOutputs(outputs: readonly DecodedEvent<TOutput>[]): void;
    /** Updates a message idempotently. */
    updateMessage(message: TMessage): void;
    /** Initializes a message when it is not already present. */
    initMessage(message: TMessage): void;
    /** Marks a message complete when it is currently active. */
    completeMessage(message: TMessage): void;
}
/**
 * Options for {@link createAccumulator}.
 */
export interface CreateAccumulatorOptions<TMessage> {
    /** Returns a stable message id. */
    getMessageId?(message: TMessage): string | undefined;
}
/**
 * Creates the documented accumulator adapter over `fold` and `getMessages`.
 */
export declare function createAccumulator<TInput, TOutput, TProjection, TMessage>(codec: Codec<TInput, TOutput, TProjection, TMessage>, options?: CreateAccumulatorOptions<TMessage>): MessageAccumulator<TOutput, TMessage>;
/**
 * Compile-time structural assertion that a type satisfies {@link ChannelWriter}.
 */
export type AssertChannelWriter<T extends ChannelWriter> = T;
/**
 * AI codec transport headers carried with stream writes.
 */
export interface CodecHeaderSet {
    /** Transport-tier headers. */
    transport?: HeaderMap;
    /** Codec-tier headers. */
    codec?: HeaderMap;
}
//# sourceMappingURL=types.d.ts.map

// via re-export: core/transport/agent-session.d.ts
import { ErrorInfo } from "../../errors.js";
import { type Logger } from "../../logger.js";
import type { ChannelLike, ClientLike } from "../../realtime/index.js";
import { type HeaderMap } from "../../utils.js";
import type { Codec, EncoderOptions } from "../codec/index.js";
import type { RunEndReason, StepEndReason } from "./tree.js";
import { type InvocationIdProvider } from "./invocation.js";
import { type ResolveWriteOptions, type StreamResult } from "./pipe-stream.js";
import { type CancelRequest } from "./run-manager.js";
import type { CancelFilter } from "./client-session.js";
/** Message node accepted by server-side `addMessages`. */
export interface MessageNode<TMessage> {
    /** Node discriminator. */
    kind?: "message";
    /** Domain message. */
    message: TMessage;
    /** Codec message id override for optimistic reconciliation. */
    msgId?: string;
    /** Parent codec message id. */
    parentId?: string;
    /** Fork source codec message id. */
    forkOf?: string;
    /** Header overrides. */
    headers?: HeaderMap;
}
/** Events targeting an existing codec message. */
export interface EventsNode<TOutput> {
    /** Node discriminator. */
    kind?: "event";
    /** Target codec message id. */
    msgId: string;
    /** Events to apply. */
    events: readonly TOutput[];
}
/** Options for `addMessages`. */
export interface AddMessageOptions {
    /** Verified client id for attribution. */
    clientId?: string;
}
/** Result of `addMessages`. */
export interface AddMessagesResult {
    /** Published codec message ids in order. */
    msgIds: string[];
}
/** Options for `streamResponse`. */
export interface StreamResponseOptions<TOutput> {
    /** Parent codec message id. */
    parent?: string;
    /** Fork source codec message id. */
    forkOf?: string;
    /** Per-output write option resolver. */
    resolveWriteOptions?: ResolveWriteOptions<TOutput>;
    /**
     * Step stamp applied to every output of this stream. Set by
     * {@link RunStep.streamResponse}; not intended for direct use.
     */
    stepHeaders?: HeaderMap;
}
/** Options for `loadConversation`. */
export interface LoadConversationOptions {
    /** History page size.
     *
     * @defaultValue `200`
     */
    pageLimit?: number;
    /** Maximum materialized messages.
     *
     * @defaultValue `2000`
     */
    maxMessages?: number;
}
/** Options for {@link AgentRun.createStep}. */
export interface StepOptions {
    /**
     * Stable step identity. Reuse it to publish a retry: the attempt with the
     * highest `step-start-serial` becomes canonical and supersedes the rest.
     *
     * @defaultValue A generated id.
     */
    stepId?: string;
    /** Verified client id owning this step. */
    stepClientId?: string;
}
/**
 * A re-attemptable unit of execution inside a run.
 *
 * Outputs published while the step is open are stamped with its `step-id` and
 * `step-start-serial`, so a client can discard output from a superseded attempt
 * instead of folding a failed attempt's partial content.
 */
export interface RunStep<TOutput> {
    /** Stable step identity across attempts. */
    readonly stepId: string;
    /** Serial of this attempt's `ai-step-start`, once started. */
    readonly startSerial: string | undefined;
    /** Publishes `ai-step-start`, opening this attempt. */
    start(): Promise<void>;
    /** Streams outputs stamped with this attempt. */
    streamResponse(stream: ReadableStream<TOutput>, options?: StreamResponseOptions<TOutput>): Promise<StreamResult>;
    /** Publishes `ai-step-end`, closing this attempt. */
    end(reason?: StepEndReason): Promise<void>;
}
/** Server-side run construction options. */
export interface CreateRunOptions<TOutput> {
    /** Run identity. */
    runId?: string;
    /** Owner client id. */
    clientId?: string;
    /** Parent codec message id. */
    parent?: string;
    /** Fork source codec message id. */
    forkOf?: string;
    /** Hook invoked before encoder writes. */
    onMessage?: EncoderOptions["onMessage"];
    /** Hook invoked when a stream aborts. */
    onAbort?(write: (event: TOutput) => Promise<void>): void | Promise<void>;
    /** Cancel authorization hook. */
    onCancel?(request: CancelRequest): Promise<boolean> | boolean;
    /** AgentRun-scoped non-fatal error hook. */
    onError?(error: ErrorInfo): void;
    /**
     * Notified when a steer arrives for this run. Optional: the run tracks
     * steers regardless, so an agent that only calls `hasInput()` needs no hook.
     */
    onSteer?(codecMessageId: string): void;
    /** External abort signal. */
    signal?: AbortSignal;
    /** Invocation id for input-event lookup. */
    invocationId?: string;
    /** Input event id for input-event lookup. */
    inputEventId?: string;
}
/** Server-side run. The legacy type name is kept for source compatibility. */
export interface AgentRun<TOutput, TProjection, TMessage> {
    /** Run identity. */
    readonly runId: string;
    /** Abort signal scoped to this turn. */
    readonly abortSignal: AbortSignal;
    /** Lightweight view over loaded messages. */
    readonly view: {
        readonly messages: readonly TMessage[];
    };
    /** Loaded messages alias. */
    readonly messages: readonly TMessage[];
    /** Publishes run start after optional input lookup. */
    start(): Promise<void>;
    /** Publishes discrete messages. */
    addMessages(nodes: readonly MessageNode<TMessage>[], options?: AddMessageOptions): Promise<AddMessagesResult>;
    /** Streams response outputs. */
    streamResponse(stream: ReadableStream<TOutput>, options?: StreamResponseOptions<TOutput>): Promise<StreamResult>;
    /** Publishes cross-turn events. */
    addEvents(nodes: readonly EventsNode<TOutput>[]): Promise<void>;
    /** Loads this turn projection from history and observed input. */
    loadProjection(): Promise<TProjection>;
    /** Loads conversation messages. */
    loadConversation(options?: LoadConversationOptions): Promise<TMessage[]>;
    /** Publishes run end or run suspend. */
    end(reason: RunEndReason): Promise<void>;
    /**
     * Opens a re-attemptable step within this run.
     *
     * Pass the same `stepId` again to publish a retry; the newer attempt
     * supersedes the older one on the client.
     */
    createStep(options?: StepOptions): RunStep<TOutput>;
    /**
     * Claims any steer input that arrived since the last call.
     *
     * Call this at a step boundary, before running the next inference: whatever
     * it drains is stamped onto that step attempt's outputs as
     * `steer-codec-message-ids`, which is how the client settles steer outcomes.
     * Returns true when input was claimed, so an agent loop can decide to take
     * another iteration instead of ending the run.
     */
    hasInput(): boolean;
}
/** Server transport options. */
export interface AgentSessionOptions<TInput, TOutput, TProjection, TMessage> {
    /** Realtime client used with `channelName`. */
    client?: ClientLike;
    /** Realtime channel. */
    channel?: ChannelLike;
    /** Channel name used when `client` is supplied. */
    channelName?: string;
    /** Domain codec. */
    codec: Codec<TInput, TOutput, TProjection, TMessage>;
    /** Logger.
     *
     * @defaultValue Silent SDK logger.
     */
    logger?: Logger;
    /** Transport-level error hook. */
    onError?(error: ErrorInfo): void;
    /** Input event lookup timeout in milliseconds.
     *
     * @defaultValue `30000`
     */
    inputEventLookupTimeoutMs?: number;
    /** Input event buffer cap.
     *
     * @defaultValue `200`
     */
    inputEventBufferLimit?: number;
    /** Subscribe rewind window.
     *
     * @defaultValue `"2m"`
     */
    rewindWindow?: string;
    /** Deterministic id provider for generated assistant message ids.
     *
     * @defaultValue Uses `crypto.randomUUID()` through the default invocation id provider.
     */
    idProvider?: InvocationIdProvider;
}
/** Server-side transport. */
export interface AgentSession<TOutput, TProjection, TMessage> {
    /** Creates and registers a turn synchronously. */
    createRun(options: CreateRunOptions<TOutput>): AgentRun<TOutput, TProjection, TMessage>;
    /** Unsubscribes, aborts turns, and clears state. */
    close(): void;
}
/** Creates a server/agent transport. */
export declare function createAgentSession<TInput, TOutput, TProjection, TMessage>(options: AgentSessionOptions<TInput, TOutput, TProjection, TMessage>): AgentSession<TOutput, TProjection, TMessage>;
export type { CancelFilter, CancelRequest, StreamResult };
//# sourceMappingURL=agent-session.d.ts.map

// via re-export: core/transport/client-session.d.ts
import { ErrorInfo } from "../../errors.js";
import { type EventUnsubscribe } from "../../event-emitter.js";
import { type Logger } from "../../logger.js";
import type { ChannelLike, ClientLike, InboundMessage } from "../../realtime/index.js";
import type { Codec } from "../codec/index.js";
import { type Tree } from "./tree.js";
import { type View } from "./view.js";
import { type InvocationIdProvider } from "./invocation.js";
import { type SteerResult } from "./steer.js";
/**
 * Cancellation scope for client-side turn cancellation.
 *
 * @defaultValue `cancel()` and `waitForRun()` default to `{ own: true }`.
 */
export type CancelFilter = {
    runId: string;
    own?: never;
    clientId?: never;
    all?: never;
} | {
    own: boolean;
    runId?: never;
    clientId?: never;
    all?: never;
} | {
    clientId: string;
    runId?: never;
    own?: never;
    all?: never;
} | {
    all: boolean;
    runId?: never;
    own?: never;
    clientId?: never;
};
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
    on<K extends keyof ClientSessionEvents>(event: K, handler: (payload: ClientSessionEvents[K]) => void): EventUnsubscribe;
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
export declare function createClientSession<TInput, TOutput, TProjection, TMessage>(options: ClientSessionOptions<TInput, TOutput, TProjection, TMessage>): ClientSession<TInput, TOutput, TProjection, TMessage>;
//# sourceMappingURL=client-session.d.ts.map

// via re-export: core/transport/decode-history.d.ts
import type { Decoder } from "../codec/index.js";
import type { InboundMessage, PaginatedResult, Serial } from "../../realtime/types.js";
import type { Tree } from "./tree.js";
/**
 * Result of decoding one history page into a conversation tree.
 */
export interface DecodeHistoryResult {
    /** Number of history messages processed. */
    processedMessages: number;
    /** Number of decoded codec events folded. */
    decodedEvents: number;
    /** Number of lifecycle messages applied. */
    lifecycleEvents: number;
}
/**
 * Decodes a page of Sockudo history into the shared conversation tree upsert path.
 */
export declare function decodeHistoryPage<TInput, TOutput, TProjection>(page: PaginatedResult<InboundMessage>, decoder: Decoder<TInput, TOutput>, tree: Tree<TInput | TOutput, TProjection>): DecodeHistoryResult;
/**
 * History source used by {@link loadHistoryIntoTree}.
 */
export interface HistoryReader {
    /** Reads one page of normalized history. */
    history(options?: {
        limit?: number;
        direction?: "newest_first" | "oldest_first" | "backwards" | "reverse";
        untilAttach?: boolean;
        end?: Serial;
    }): Promise<PaginatedResult<InboundMessage>>;
}
/**
 * Options for paginated history decoding.
 */
export interface LoadHistoryOptions {
    /** Target number of newly visible turns. */
    limit?: number;
    /** Whether to request `untilAttach` on the first history page. */
    untilAttach?: boolean;
    /** Wire page size. */
    wireLimit?: number;
}
/**
 * Result of a paginated history load.
 */
export interface LoadHistoryResult extends DecodeHistoryResult {
    /** Last page returned by the history source. */
    page: PaginatedResult<InboundMessage>;
}
/**
 * Loads and decodes one backward history page.
 */
export declare function loadHistoryIntoTree<TInput, TOutput, TProjection>(source: HistoryReader, decoder: Decoder<TInput, TOutput>, tree: Tree<TInput | TOutput, TProjection>, options?: LoadHistoryOptions): Promise<LoadHistoryResult>;
//# sourceMappingURL=decode-history.d.ts.map

// via re-export: core/transport/index.d.ts
export { createAgentSession, type AddMessageOptions, type AddMessagesResult, type CancelRequest, type EventsNode, type LoadConversationOptions, type MessageNode, type CreateRunOptions, type RunStep, type StepOptions, type AgentSession, type AgentSessionOptions, type StreamResponseOptions, type StreamResult, type AgentRun, } from "./agent-session.js";
export { createClientSession, type ClientRun, type CancelFilter, type ClientSession, type ClientSessionEvents, type ClientSessionOptions, type CloseOptions, type SendOptions, } from "./client-session.js";
export { decodeHistoryPage, loadHistoryIntoTree, type DecodeHistoryResult, type HistoryReader, type LoadHistoryOptions, type LoadHistoryResult, } from "./decode-history.js";
export { createDefaultInvocationIdProvider, type InvocationIdProvider } from "./invocation.js";
export { createStreamRouter, type StreamRouter, type StreamRouterOptions, } from "./stream-router.js";
export { createTree, treeRoutingRoles, type Tree, type TreeEvents, type TreeOptions, type TreeMessageEvent, type TreeSerial, type RunEndReason, type StepEndReason, type StepInfo, type StepLifecycleEvent, type RunLifecycleEvent, type RunNode, type RunStatus, } from "./tree.js";
export { createView, type BranchSelectionIntent, type MessageMetadata, type View, type ViewEvents, type ViewOptions, type ViewSendExecutor, } from "./view.js";
export { readSteerStamp, SteerCoordinator, type SteerOutcome, type SteerResult } from "./steer.js";
export { MAX_STEER_IDS_PER_STAMP, RunSteerTracker } from "./run-steer-tracker.js";
export { reorderUnrespondedSteers, unrespondedSteerIds, type OrderableMessage, type SteerOrderingOptions, } from "./steer-ordering.js";
//# sourceMappingURL=index.d.ts.map

// via re-export: core/transport/invocation.d.ts
/**
 * Deterministic identity providers for client session invocations.
 */
export interface InvocationIdProvider {
    /** Returns the next turn id. */
    runId(): string;
    /** Returns the next invocation id. */
    invocationId(): string;
    /** Returns the next input event id. */
    inputEventId(): string;
    /** Returns the next codec message id. */
    messageId(): string;
}
/**
 * Creates the default invocation id provider.
 *
 * @defaultValue Uses `crypto.randomUUID()` and falls back to a monotonic
 * process-local id if the runtime throws.
 */
export declare function createDefaultInvocationIdProvider(): InvocationIdProvider;
//# sourceMappingURL=invocation.d.ts.map

// via re-export: core/transport/run-steer-tracker.d.ts
/**
 * Maximum steer ids one stamp may carry.
 *
 * The Sockudo server bounds `steer-codec-message-ids` at 2 KiB, and a
 * UUID-shaped codec-message-id costs roughly 42 bytes inside the JSON array.
 * Overflow rejects the whole assistant output publish rather than dropping the
 * stamp, so the cap is deliberately well under the ceiling.
 *
 * Consequence worth knowing: past the cap the oldest drained ids are omitted,
 * so their `steer()` outcome reports `consumed: false` even though the agent
 * did see them. Losing outcome precision beats losing the message.
 */
export declare const MAX_STEER_IDS_PER_STAMP = 32;
/**
 * Per-run steer bookkeeping on the agent side.
 *
 * Steers arrive as client inputs while a run is already executing. The agent's
 * loop drains them at a step boundary via {@link drain}; whatever was drained
 * is then stamped onto that step attempt's outputs so the client can settle
 * outcomes.
 */
export declare class RunSteerTracker {
    private pending;
    private drained;
    /** Records a steer input that arrived for this run. */
    add(codecMessageId: string): void;
    /** Whether unclaimed steer input is waiting. */
    get hasPending(): boolean;
    /**
     * Moves pending steers into the drained set and returns them.
     *
     * Called when the agent's loop is about to run an inference, so "drained"
     * means "visible to the iteration that produced the next output".
     */
    drain(): readonly string[];
    /** Every steer drained so far, newest last, capped for the wire. */
    get stampIds(): readonly string[];
    /**
     * Transport headers stamping the drained steers, or an empty map when none.
     *
     * Omitted rather than empty when nothing was drained: the server rejects an
     * empty value for this key.
     */
    stampHeaders(): Record<string, string>;
    /** Forgets all state for this run. */
    clear(): void;
}
//# sourceMappingURL=run-steer-tracker.d.ts.map

// via re-export: core/transport/steer-ordering.d.ts
/**
 * Prompt ordering repair for steers.
 *
 * Messages arrive in serial order, and a steer lands mid-run, so by serial it
 * sits *between* assistant outputs:
 *
 *   user "explain X" → assistant "X is…" → user (steer) "in French" → assistant …
 *
 * Feeding that straight to a model has two problems. The prompt ends on an
 * assistant message, which several providers handle poorly when asked for a
 * completion, and the steer is buried behind assistant text that came after it,
 * so it loses salience exactly when it should carry most.
 *
 * Moving unresponded steers to the tail fixes both: the prompt ends on the
 * user's latest instruction.
 */
/**
 * Identifies a message for ordering purposes.
 *
 * Deliberately structural rather than tied to a codec: callers hold
 * `AI.UIMessage`, OpenAI items, or their own shape, and only the id and role
 * matter here.
 */
export interface OrderableMessage {
    /** Codec message id. */
    id: string;
    /** Message role. */
    role: string;
}
/** Options for {@link reorderUnrespondedSteers}. */
export interface SteerOrderingOptions {
    /**
     * Codec message ids of steers the agent has not yet responded to.
     *
     * "Responded to" is not the same as drained: a steer the agent drained but
     * whose step produced no assistant output afterwards is still unresponded, so
     * callers usually derive this from the steers drained since the last assistant
     * message rather than from every steer seen.
     */
    unrespondedSteerIds: readonly string[];
}
/**
 * Moves unresponded steers to the tail, preserving their relative order.
 *
 * Returns the input array unchanged (same reference) when there is nothing to
 * move, so callers can use it on every prompt build without allocating.
 *
 * Only trailing assistant messages are jumped: a steer stays put if it is
 * already at the tail, and messages that are not named as unresponded are never
 * reordered relative to each other.
 */
export declare function reorderUnrespondedSteers<TMessage extends OrderableMessage>(messages: readonly TMessage[], options: SteerOrderingOptions): readonly TMessage[];
/**
 * Derives which steers are still unresponded.
 *
 * A steer counts as responded once an assistant message appears after it in
 * serial order — that is the only evidence on the wire that the agent's output
 * came *after* seeing it. Steers drained by a step that then failed, or that
 * produced no output, therefore stay unresponded and get moved to the tail on
 * the next prompt build.
 */
export declare function unrespondedSteerIds(messages: readonly OrderableMessage[], steerIds: readonly string[]): readonly string[];
//# sourceMappingURL=steer-ordering.d.ts.map

// via re-export: core/transport/steer.d.ts
import type { Serial } from "../../realtime/types.js";
import type { HeaderMap } from "../../utils.js";
import type { RunEndReason } from "./tree.js";
/**
 * Outcome of a steering publish, derived from the agent's responses.
 *
 * Each agent response carries a `steer-codec-message-ids` header listing the
 * steers its loop had drained when the stamping step attempt opened. The union
 * of those lists per run is what decides membership.
 */
export interface SteerOutcome {
    /**
     * Whether the steer landed inside the run's consumed-input window.
     *
     * `true` means the agent's loop had it visible at the iteration it drained
     * on — not that the agent's prompt actually used it. The wire cannot report
     * more than visibility, so neither does this.
     */
    consumed: boolean;
    /**
     * Terminal reason of the run, present when the outcome was settled by a run
     * end. Absent when settled by a suspend, since a later resume may still
     * consume the steer.
     */
    runTerminalReason?: RunEndReason;
}
/**
 * Result of {@link ClientRun.steer}.
 */
export interface SteerResult {
    /** Resolves when the steer publish lands, carrying the assigned serial. */
    published: Promise<{
        serial: Serial | undefined;
    }>;
    /** Resolves with consumed/not-consumed at the run's next terminal event. */
    outcome: Promise<SteerOutcome>;
}
/**
 * Reads the steer stamp off a message's transport headers.
 *
 * The wire value is a JSON array. A malformed value is treated as absent
 * rather than thrown: a stamp is an observation used to settle outcomes, and
 * one unreadable stamp must not break folding of the message it rode on.
 */
export declare function readSteerStamp(headers: HeaderMap): readonly string[];
/**
 * Tracks in-flight steers per run and settles their outcomes.
 *
 * One coordinator per client session. Outcomes settle when a run reaches a
 * terminal lifecycle event; anything still pending when the session closes is
 * rejected rather than left hanging.
 */
export declare class SteerCoordinator {
    private readonly pendingByRun;
    private readonly stampsByRun;
    /** Registers a steer awaiting its outcome. */
    track(runId: string, codecMessageId: string): Promise<SteerOutcome>;
    /**
     * Folds a stamp observed on an agent response into the run's union.
     *
     * Accumulating rather than replacing matters: a run's responses each report
     * only what was drained by the step attempt that produced them, so the
     * consumed window is the union across the run, not the latest stamp.
     */
    observe(headers: HeaderMap): void;
    /**
     * Settles every steer pending on a run.
     *
     * A suspend settles outcomes without a terminal reason and keeps the stamp
     * union, because a resume can still consume steers that have not landed yet.
     * A run end drops the union.
     */
    settle(runId: string, reason: RunEndReason | undefined, terminal: boolean): void;
    /** Rejects everything still pending. Called on close and continuity loss. */
    drain(message: string): void;
}
//# sourceMappingURL=steer.d.ts.map

// via re-export: core/transport/stream-router.d.ts
import { ErrorInfo } from "../../errors.js";
/**
 * Client-side per-turn stream router.
 */
export interface StreamRouter<TOutput> {
    /** Registers or replaces a stream for a turn invocation. */
    createStream(runId: string, invocationId: string): ReadableStream<TOutput>;
    /** Rebinds a suspended turn stream to a continuation invocation. */
    rebindStream(runId: string, invocationId: string): boolean;
    /** Routes one decoded output to an active own-turn stream. */
    route(runId: string, invocationId: string | undefined, output: TOutput): boolean;
    /** Closes an active stream. */
    closeStream(runId: string): boolean;
    /** Errors an active stream. */
    errorStream(runId: string, error: ErrorInfo): boolean;
    /** Returns true when a turn has an active stream. */
    has(runId: string): boolean;
    /** Returns the currently bound invocation id for a turn. */
    activeInvocation(runId: string): string | undefined;
    /** Returns the currently bound stream for a turn. */
    getStream(runId: string): ReadableStream<TOutput> | undefined;
    /** Closes all streams. */
    closeAll(): void;
}
/**
 * Options for {@link createStreamRouter}.
 */
export interface StreamRouterOptions<TOutput> {
    /** Returns whether an output terminates the stream. */
    isTerminal(output: TOutput): boolean;
    /** Maximum queued chunks per stream.
     *
     * @defaultValue `1024`
     */
    maxQueuedChunks?: number;
}
/**
 * Creates a bounded O(1) client-side stream router.
 */
export declare function createStreamRouter<TOutput>(options: StreamRouterOptions<TOutput>): StreamRouter<TOutput>;
//# sourceMappingURL=stream-router.d.ts.map

// via re-export: core/transport/tree.d.ts
import { type EventUnsubscribe } from "../../event-emitter.js";
import type { HeaderMap } from "../../utils.js";
import type { DecodedEvent, Reducer } from "../codec/index.js";
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
    update: {
        structuralVersion: number;
    };
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
    applyMessage(decodedEvents: readonly DecodedEvent<TEvent>[], transportHeaders: HeaderMap, serial: TreeSerial): RunNode<TProjection> | undefined;
    /** Applies turn lifecycle metadata. */
    applyRunLifecycle(event: RunLifecycleEvent): RunNode<TProjection> | undefined;
    /** Applies step lifecycle metadata. */
    applyStepLifecycle(event: StepLifecycleEvent): RunNode<TProjection> | undefined;
    /** Whether output stamped with this step attempt is from the canonical attempt. */
    isCanonicalStepAttempt(stepId: string, startSerial: string): boolean;
    /** Deletes a codec message id and removes unreachable turns. */
    delete(codecMessageId: string): void;
    /** Docs-compatible upsert alias for {@link applyMessage}. */
    upsert(decodedEvents: readonly DecodedEvent<TEvent>[], transportHeaders: HeaderMap, serial: TreeSerial): RunNode<TProjection> | undefined;
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
    on<K extends keyof TreeEvents<TEvent, TProjection>>(event: K, handler: (payload: TreeEvents<TEvent, TProjection>[K]) => void): EventUnsubscribe;
}
/**
 * Creates a turn-keyed conversation tree.
 *
 * Projection ref-equality is not a change signal: reducers may mutate the
 * projection in place, and streaming folds emit `turn-projection-updated`.
 */
export declare function createTree<TEvent, TProjection>(reducer: Reducer<TEvent, TProjection>, options?: TreeOptions<TProjection>): Tree<TEvent, TProjection>;
/**
 * Transport roles used by tree routing.
 */
export declare const treeRoutingRoles: {
    /** User input role. */
    readonly user: "user";
    /** Assistant output role. */
    readonly assistant: "assistant";
    /** Tool output role. */
    readonly tool: "tool";
};
//# sourceMappingURL=tree.d.ts.map

// via re-export: core/transport/view.d.ts
import { ErrorInfo } from "../../errors.js";
import { type EventUnsubscribe } from "../../event-emitter.js";
import type { Codec, Decoder } from "../codec/index.js";
import { type HistoryReader } from "./decode-history.js";
import type { SendOptions } from "./client-session.js";
import type { Tree, RunEndReason, RunNode } from "./tree.js";
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
    on<K extends keyof ViewEvents<TMessage>>(event: K, handler: (payload: ViewEvents<TMessage>[K]) => void): EventUnsubscribe;
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
export declare function createView<TInput, TOutput, TProjection, TMessage>(options: ViewOptions<TInput, TOutput, TProjection, TMessage>): View<TInput, TMessage>;
//# sourceMappingURL=view.d.ts.map

// via re-export: errors.d.ts
/**
 * Stable SDK error codes used by `@sockudo/ai-transport`.
 */
export declare enum ErrorCode {
    /** Malformed request or invalid wire data. */
    BadRequest = 40000,
    /** Invalid local API argument. */
    InvalidArgument = 104012,
    /** Capability token expired. */
    TokenExpired = 40142,
    /** Authentication or capability check failed. */
    InsufficientCapability = 40003,
    /** Capability token was revoked. */
    TokenRevoked = 40160,
    /** Encoder recovery failed after a stream append failure. */
    EncoderRecoveryFailed = 104000,
    /** Channel subscription failed. */
    SessionSubscriptionError = 104001,
    /** Cancellation listener failed. */
    CancelListenerError = 104002,
    /** Invalid run lifecycle operation. */
    RunLifecycleError = 104003,
    /** Session was used after close. */
    SessionClosed = 104004,
    /** Send or acknowledgement failed. */
    SessionSendFailed = 104005,
    /** Channel continuity was lost and history backfill is required. */
    ChannelContinuityLost = 104006,
    /** Channel is not ready. */
    ChannelNotReady = 104007,
    /** Stream failed. */
    StreamError = 104008,
    /** Run start did not arrive before the configured deadline. */
    RunStartDeadlineExceeded = 104009,
    /** Input event was not found. */
    InputEventNotFound = 104010,
    /** Channel history could not be fetched after bounded retries. */
    HistoryFetchFailed = 104011
}
/**
 * Constructor options for {@link ErrorInfo}.
 */
export interface ErrorInfoOptions {
    /** Numeric SDK or Sockudo platform code. */
    code: ErrorCode | number;
    /** Human-readable error message. */
    message: string;
    /** HTTP-like status code; derived from `code` by default. */
    statusCode?: number;
    /** Original error or thrown value. */
    cause?: unknown;
    /** Structured diagnostic detail. */
    detail?: unknown;
}
/**
 * Formats SDK error messages in the Ably-compatible form.
 */
export declare function formatErrorMessage(operation: string, reason: string): string;
/**
 * Error shape thrown or rejected by public SDK APIs.
 *
 * @defaultValue `statusCode` is derived from the numeric code.
 */
export declare class ErrorInfo extends Error {
    /** Numeric SDK or Sockudo platform code. */
    readonly code: ErrorCode | number;
    /** HTTP-like status code. */
    readonly statusCode: number;
    /** Original error or thrown value. */
    readonly cause?: unknown;
    /** Structured diagnostic detail. */
    readonly detail?: unknown;
    /** Creates a new SDK error. */
    constructor(options: ErrorInfoOptions);
}
/**
 * Returns whether a value is an {@link ErrorInfo} with `code`.
 */
export declare function errorInfoIs(value: unknown, code: ErrorCode | number): value is ErrorInfo;
/**
 * Derives an HTTP-like status code from an SDK or platform error code.
 */
export declare function statusCodeForErrorCode(code: ErrorCode | number): number;
/**
 * Converts unknown thrown values into {@link ErrorInfo}.
 */
export declare function toErrorInfo(value: unknown, fallback: ErrorInfoOptions): ErrorInfo;
//# sourceMappingURL=errors.d.ts.map

// via re-export: event-emitter.d.ts
import { type Logger } from "./logger.js";
/**
 * Generic event map used by {@link EventEmitter}.
 */
export type EventsMap = object;
/**
 * Callback returned by {@link EventEmitter.on}.
 */
export type EventUnsubscribe = () => void;
/**
 * Options for {@link EventEmitter}.
 */
export interface EventEmitterOptions {
    /** Logger used for listener exceptions.
     *
     * @defaultValue A silent SDK logger.
     */
    logger?: Logger;
}
/**
 * Tiny typed synchronous emitter.
 *
 * Listener exceptions are caught, logged, and never propagated to the emit
 * site.
 */
export declare class EventEmitter<Events extends EventsMap> {
    private readonly listeners;
    private readonly logger;
    /** Creates an emitter. */
    constructor(options?: EventEmitterOptions);
    /** Subscribes to an event and returns an unsubscribe callback. */
    on<K extends keyof Events>(event: K, listener: (payload: Events[K]) => void): EventUnsubscribe;
    /** Emits an event synchronously to current listeners. */
    emit<K extends keyof Events>(event: K, payload: Events[K]): void;
}
//# sourceMappingURL=event-emitter.d.ts.map

// via re-export: logger.d.ts
/**
 * Logging levels in ascending verbosity.
 */
export declare enum LogLevel {
    /** Disable all logging. */
    Silent = 0,
    /** Log errors only. */
    Error = 1,
    /** Log warnings and errors. */
    Warn = 2,
    /** Log info, warnings, and errors. */
    Info = 3,
    /** Log debug and higher severity messages. */
    Debug = 4,
    /** Log every message. */
    Trace = 5
}
/**
 * Context object attached to SDK log messages.
 */
export type LogContext = Readonly<Record<string, unknown>>;
/**
 * Sink used by {@link makeLogger}.
 */
export type LogHandler = (line: string) => void;
/**
 * Logger interface used throughout the SDK.
 */
export interface Logger {
    /** Emits a trace message when enabled. */
    trace(message: string, context?: LogContext): void;
    /** Emits a debug message when enabled. */
    debug(message: string, context?: LogContext): void;
    /** Emits an info message when enabled. */
    info(message: string, context?: LogContext): void;
    /** Emits a warning message when enabled. */
    warn(message: string, context?: LogContext): void;
    /** Emits an error message when enabled. */
    error(message: string, context?: LogContext): void;
    /** Returns a logger with context merged into every subsequent call. */
    withContext(context: LogContext): Logger;
}
/**
 * Options for {@link makeLogger}.
 */
export interface MakeLoggerOptions {
    /** Log sink.
     *
     * @defaultValue A no-op handler.
     */
    logHandler?: LogHandler;
    /** Maximum verbosity to emit.
     *
     * @defaultValue {@link LogLevel.Silent}
     */
    logLevel?: LogLevel;
    /** Context merged into every log call.
     *
     * @defaultValue `{}`
     */
    context?: LogContext;
    /** Clock provider for deterministic tests.
     *
     * @defaultValue `new Date().toISOString()`
     */
    now?: () => string;
}
/**
 * Console-backed log sink.
 */
export declare const consoleLogger: LogHandler;
/**
 * Creates a redacting SDK logger.
 */
export declare function makeLogger(options?: MakeLoggerOptions): Logger;
/**
 * Returns a JSON-safe copy with sensitive keys masked.
 */
export declare function redactValue(value: unknown): unknown;
//# sourceMappingURL=logger.d.ts.map

// via re-export: realtime/adapter.d.ts
import type { AppendRollupWindow, ChannelLike, ClientLike, InboundMessage, Serial } from "./types.js";
/** Mutable-message information returned by `@sockudo/client`. */
export interface SockudoMutableMessageInfo {
    /** Sockudo action such as `message.append`. */
    action: string;
    /** Delivered event name. */
    event: string;
    /** Stable logical message serial. */
    messageSerial: string;
    /** Version serial. */
    versionSerial?: string;
    /** Durable history serial. */
    historySerial?: Serial;
    /** Version timestamp in milliseconds. */
    versionTimestampMs?: number;
}
/** Compatible signature for `@sockudo/client` `getMutableMessageInfo`. */
export type MutableMessageInfoReader = (event: Pick<SockudoRawMessage, "event" | "extras">) => SockudoMutableMessageInfo | null;
/** Raw Sockudo message shape consumed by the adapter. */
export interface SockudoRawMessage {
    /** Delivered event name. */
    event: string;
    /** Delivered channel name. */
    channel?: string;
    /** Opaque payload. */
    data?: unknown;
    /** Logical message name. */
    name?: string;
    /** Verified user/client identity. */
    user_id?: string;
    /** Recovery stream identity. */
    stream_id?: string;
    /** Idempotent message id. */
    message_id?: string;
    /** Delivery serial. */
    serial?: Serial;
    /** Sockudo extras. */
    extras?: unknown;
    /** Additive future fields. */
    [key: string]: unknown;
}
/** Structural subset required from a Sockudo channel object. */
export interface SockudoChannelPeer {
    /** Channel name. */
    name: string;
    /** Attach serial captured by Sockudo on subscribe. */
    attachSerial?: Serial;
    /** Publish-create helper. */
    publishCreate?(message: Record<string, unknown>): Promise<unknown>;
    /** Append helper. */
    appendMessage?(messageSerial: string, data: string, options?: Record<string, unknown>): Promise<unknown>;
    /** Update helper. */
    updateMessage?(messageSerial: string, options?: Record<string, unknown>): Promise<unknown>;
    /** Delete helper. */
    deleteMessage?(messageSerial: string, options?: Record<string, unknown>): Promise<unknown>;
    /** Channel history helper. */
    channelHistory?(params?: Record<string, unknown>): Promise<unknown>;
    /** Event binding helper. */
    bind?(event: string, listener: (...args: readonly unknown[]) => void): unknown;
    /** Event unbinding helper. */
    unbind?(event?: string, listener?: (...args: readonly unknown[]) => void): unknown;
    /** Global channel event binding helper. */
    bind_global?(listener: (...args: readonly unknown[]) => void): unknown;
    /** Raw event handler. */
    handleEvent?(event: SockudoRawMessage): unknown;
    /** Presence member store when this is a presence channel. */
    members?: SockudoMembersPeer;
    /** Presence update helper. */
    update?(data?: unknown): Promise<void>;
    /** Presence enter helper. */
    enter?(data?: unknown): Promise<void>;
    /** Presence leave helper. */
    leave?(data?: unknown): Promise<void>;
}
/** Structural subset required from a Sockudo client object. */
export interface SockudoClientPeer {
    /** Channel registry. */
    channels?: {
        /** Adds or returns a channel. */
        add?(name: string, client: SockudoClientPeer): SockudoChannelPeer;
        /** Finds a channel. */
        find?(name: string): SockudoChannelPeer | undefined;
    };
    /** Connection state. */
    connection?: {
        /** Current state. */
        state?: string;
        /** Verified server identity. */
        clientId?: string;
    };
    /** Subscribes to a channel. */
    subscribe?(name: string, options?: Record<string, unknown>): SockudoChannelPeer;
    /** Finds a channel. */
    channel?(name: string): SockudoChannelPeer | undefined;
    /** Binds a global client event. */
    bind?(event: string, listener: (payload: unknown) => void): unknown;
    /** Unbinds a global client event. */
    unbind?(event?: string, listener?: (payload: unknown) => void): unknown;
    /** Returns verified identity. */
    verifiedClientId?(): string | undefined;
    /** Disconnects the client. */
    disconnect?(): void;
}
/** Options for adapting a Sockudo client. */
export interface AdaptSockudoClientOptions {
    /** Mutable-message helper from `@sockudo/client`. */
    mutableMessageInfoReader?: MutableMessageInfoReader;
}
/** Options for adapting a Sockudo channel. */
export interface AdaptSockudoChannelOptions extends AdaptSockudoClientOptions {
    /** Optional parent client for subscribe/recovery passthrough. */
    client?: SockudoClientPeer;
}
/** Options for creating a Sockudo realtime client through the peer dependency. */
export interface CreateSockudoRealtimeClientOptions extends AdaptSockudoClientOptions {
    /** Options passed to `new Sockudo(appKey, options)`, including token/authUrl/authCallback. */
    clientOptions?: Record<string, unknown>;
    /** Append delivery mode passed to `@sockudo/client` for V2 sockets. */
    appendMode?: "delta" | "full";
    /** Append rollup window passed as `transportParams.append_rollup_window`. */
    appendRollupWindow?: AppendRollupWindow;
}
/** Creates a `ClientLike` by dynamically loading the `@sockudo/client` peer. */
export declare function createSockudoRealtimeClient(appKey: string, options?: CreateSockudoRealtimeClientOptions): Promise<ClientLike>;
/** Adapts an existing Sockudo client into the realtime seam. */
export declare function adaptSockudoClient(client: SockudoClientPeer, options?: AdaptSockudoClientOptions): ClientLike;
/** Adapts an existing Sockudo channel into the realtime seam. */
export declare function adaptSockudoChannel(channel: SockudoChannelPeer, options?: AdaptSockudoChannelOptions): ChannelLike;
/** Normalizes one raw Sockudo message into the seam message shape. */
export declare function normalizeInboundMessage(raw: SockudoRawMessage, mutableMessageInfoReader?: MutableMessageInfoReader): InboundMessage;
/** Compares serials without truncating unsafe integer strings. */
export declare function compareSerial(a: Serial, b: Serial): -1 | 0 | 1;
/** Returns whether serial `a` is less than or equal to serial `b`. */
export declare function serialLessThanOrEqual(a: Serial, b: Serial): boolean;
/** Validates an append rollup window. */
export declare function validateAppendRollupWindow(value: unknown): asserts value is AppendRollupWindow;
/** Validates an append delivery mode. */
export declare function validateAppendMode(value: unknown): asserts value is "delta" | "full";
interface SockudoMembersPeer {
    members?: unknown;
    each?(listener: (member: unknown) => void): void;
}
export {};
//# sourceMappingURL=adapter.d.ts.map

// via re-export: realtime/index.d.ts
export { adaptSockudoChannel, adaptSockudoClient, compareSerial, createSockudoRealtimeClient, normalizeInboundMessage, serialLessThanOrEqual, validateAppendRollupWindow, type AdaptSockudoChannelOptions, type AdaptSockudoClientOptions, type CreateSockudoRealtimeClientOptions, type MutableMessageInfoReader, type SockudoChannelPeer, type SockudoClientPeer, type SockudoMutableMessageInfo, type SockudoRawMessage, } from "./adapter.js";
export { createMockClient, MockChannel, MockClient, type MockClientOptions, type MockRealtimeProviders, } from "./mocks.js";
export type { AppendRollupWindow, ChannelEvents, ChannelLike, ClientLike, ConnectionState, GetChannelOptions, HistoryOptions, InboundMessage, InboundMessageAction, InboundMessageVersion, MessageAck, MessageListener, MessageMutation, PaginatedResult, PresenceEventName, PresenceLike, PresenceMember, PublishMessage, RewindOption, Serial, SubscribeOptions, Unsubscribe, } from "./types.js";
//# sourceMappingURL=index.d.ts.map

// via re-export: realtime/mocks.d.ts
import type { ChannelEvents, ChannelLike, ClientLike, HistoryOptions, InboundMessage, MessageAck, MessageListener, MessageMutation, PaginatedResult, PresenceLike, PublishMessage, Serial, SubscribeOptions, Unsubscribe } from "./types.js";
import type { SockudoRawMessage } from "./adapter.js";
/** Deterministic providers used by the in-memory realtime mock. */
export interface MockRealtimeProviders {
    /** Returns the next message serial. */
    messageSerial?(): string;
    /** Returns the next history serial. */
    historySerial?(): Serial;
    /** Returns the next delivery serial. */
    deliverySerial?(): Serial;
    /** Returns the next version serial. */
    versionSerial?(): string;
    /** Returns the current timestamp in milliseconds. */
    now?(): number;
}
/** Options for {@link createMockClient}. */
export interface MockClientOptions {
    /** Verified client identity. */
    clientId?: string;
    /** Deterministic providers. */
    providers?: MockRealtimeProviders;
}
/** In-memory `ClientLike` test double with scripted serials and recovery injection. */
export declare class MockClient implements ClientLike {
    private readonly channelMap;
    private readonly providers;
    private closed;
    /** Creates a mock client. */
    constructor(options?: MockClientOptions);
    /** Channel registry. */
    readonly channels: ClientLike["channels"];
    /** Mock connection state. */
    readonly connection: {
        state: string;
        clientId: string | undefined;
    };
    /** Returns the concrete mock channel by name. */
    getMockChannel(name: string): MockChannel;
    /** Injects a continuity loss event for `channelName`. */
    injectContinuityLost(channelName: string, code?: "stream_reset" | "position_expired"): void;
    /** Closes the mock client. */
    close(): void;
    /** Returns whether this client has been closed. */
    isClosed(): boolean;
}
/** In-memory `ChannelLike` test double. */
export declare class MockChannel implements ChannelLike {
    /** Channel name. */
    readonly name: string;
    private readonly providers;
    private readonly messages;
    private readonly listeners;
    private readonly events;
    private readonly presenceState;
    /** Attach serial captured by the mock subscription. */
    attachSerial: Serial | undefined;
    /** Presence API for the channel. */
    readonly presence: PresenceLike;
    /** Creates a mock channel. */
    constructor(
    /** Channel name. */
    name: string, providers: Required<MockRealtimeProviders>);
    /** Publishes a mock mutable create and dispatches it to subscribers. */
    publish(message: PublishMessage): Promise<MessageAck>;
    /** Appends to a mock mutable message. */
    appendMessage(messageSerial: string, data: string, options?: Omit<MessageMutation, "data">): Promise<MessageAck>;
    /** Updates a mock mutable message. */
    updateMessage(messageSerial: string, options?: MessageMutation): Promise<MessageAck>;
    /** Deletes a mock mutable message. */
    deleteMessage(messageSerial: string, options?: MessageMutation): Promise<MessageAck>;
    /** Subscribes to mock message delivery. */
    subscribe(listener: MessageListener, options?: SubscribeOptions): Unsubscribe;
    /** Reads mock channel history. */
    history(options?: HistoryOptions): Promise<PaginatedResult<InboundMessage>>;
    /** Subscribes to mock channel events. */
    on<K extends keyof ChannelEvents>(event: K, listener: (payload: ChannelEvents[K]) => void): Unsubscribe;
    /** Injects an arbitrary raw message. */
    inject(raw: SockudoRawMessage): void;
    /** Injects continuity loss. */
    injectContinuityLost(code: "stream_reset" | "position_expired"): void;
    private mutate;
    private nextAck;
    private rawMessage;
    private dispatch;
}
/** Creates an in-memory realtime mock client. */
export declare function createMockClient(options?: MockClientOptions): MockClient;
//# sourceMappingURL=mocks.d.ts.map

// via re-export: utils.d.ts
/** Immutable string header map. */
export type HeaderMap = Readonly<Record<string, string>>;
/** Extras shape carrying Sockudo AI metadata tiers. */
export interface AiExtras {
    /** AI metadata tiers. */
    ai?: {
        /** SDK-interpreted transport headers. */
        transport?: Record<string, unknown>;
        /** Opaque codec headers. */
        codec?: Record<string, unknown>;
    };
}
/** Inputs for canonical transport header construction. */
export interface BuildTransportHeadersOptions {
    /** Message role. */
    role?: string;
    /** Run identity. */
    runId?: string;
    /** Codec message identity. */
    codecMessageId?: string;
    /** Verified run client identity. */
    runClientId?: string;
    /** Parent codec message identity. */
    parent?: string;
    /** Fork source codec message identity. */
    forkOf?: string;
    /** Codec message id of the assistant message this message regenerates. */
    regenerates?: string | boolean;
    /** Run id of the suspended reply replaced by this fork. */
    supersedes?: string;
    /** Invocation identity. */
    invocationId?: string;
    /** Verified input client identity. */
    inputClientId?: string;
    /** Input event identity. */
    inputEventId?: string;
    /** Whether this legacy turn continues a suspended turn. Native runs use `ai-run-resume`. */
    runContinue?: boolean;
}
/** Reads AI transport headers into a null-prototype string map. */
export declare function getTransportHeaders(extras: unknown): HeaderMap;
/** Reads AI codec headers into a null-prototype string map. */
export declare function getCodecHeaders(extras: unknown): HeaderMap;
/** Creates a mutable null-prototype writer for string headers. */
export declare function headerWriter(): {
    readonly headers: Record<string, string>;
    str(key: string, value: string | undefined): void;
    bool(key: string, value: boolean | undefined): void;
    json(key: string, value: unknown): void;
    set(key: string, value: string | number | boolean | undefined): void;
    setJson(key: string, value?: unknown): void;
};
/** Creates typed readers over string headers. */
export declare function headerReader(headers: HeaderMap): {
    str(key: string): string | undefined;
    bool(key: string): boolean | undefined;
    string(key: string): string | undefined;
    boolean(key: string): boolean | undefined;
    json(key: string): unknown;
};
/** Merges header maps into a new null-prototype map. */
export declare function mergeHeaders(...sources: readonly HeaderMap[]): HeaderMap;
/** Returns a shallow copy with `undefined` properties omitted. */
export declare function stripUndefined<T extends Record<string, unknown>>(value: T): Partial<T>;
/** Builds canonical Sockudo AI transport headers. */
export declare function buildTransportHeaders(options: BuildTransportHeadersOptions): HeaderMap;
//# sourceMappingURL=utils.d.ts.map

// via re-export: vercel/codec/decoder.d.ts
import type { Decoder } from "../../core/codec/index.js";
import type { VercelInput, VercelOutput } from "./events.js";
/** Creates the inverse Vercel wire decoder. */
export declare function createVercelDecoder(): Decoder<VercelInput, VercelOutput>;
//# sourceMappingURL=decoder.d.ts.map

// via re-export: vercel/codec/encoder.d.ts
import type { ChannelWriter, Encoder, EncoderOptions } from "../../core/codec/index.js";
import type { VercelInput, VercelOutput } from "./events.js";
/** Creates a Vercel UIMessage encoder over Sockudo mutable-message writes. */
export declare function createVercelEncoder(channel: ChannelWriter, options?: EncoderOptions): Encoder<VercelInput, VercelOutput>;
//# sourceMappingURL=encoder.d.ts.map

// via re-export: vercel/codec/index.d.ts
import type { Codec } from "../../core/codec/index.js";
import type { AI, VercelInput, VercelOutput, VercelProjection } from "./events.js";
/**
 * Vercel AI SDK v6/v7 UI message codec.
 *
 * Chunk mapping:
 * `text-start|text-delta|text-end`, `reasoning-start|reasoning-delta|reasoning-end`,
 * and `tool-input-start|tool-input-delta|tool-input-available` use Sockudo
 * mutable stream create/append/terminal writes. All other UIMessageChunk
 * families are discrete `ai-output` events with codec headers carrying the
 * Vercel chunk type and stable ids. User UI messages may be published either as
 * raw `{ message }` inputs by `ClientSession` or as `ai-input` user-part
 * discretes by the codec encoder; observers decode both forms. Tool results,
 * tool errors, approval responses, and regenerate requests are `ai-input`
 * discretes with their tool/branch ids in codec or transport headers.
 */
export declare const UIMessageCodec: Codec<VercelInput, VercelOutput, VercelProjection, AI.UIMessage>;
export { createVercelDecoder } from "./decoder.js";
export { createVercelEncoder } from "./encoder.js";
export { createVercelProjection, foldVercelEvent } from "./reducer.js";
export { toolBase, transitionToolPart } from "./tool-transitions.js";
export type { AI, ForkSeed, MessageTrackers, ToolApprovalResponse, ToolResult, ToolResultError, VercelInput, VercelOutput, VercelProjection, } from "./events.js";
//# sourceMappingURL=index.d.ts.map

// via re-export: vercel/codec/reducer.d.ts
import type { ReducerMeta } from "../../core/codec/index.js";
import type { VercelInput, VercelOutput, VercelProjection } from "./events.js";
/** Creates an empty Vercel projection. */
export declare function createVercelProjection(): VercelProjection;
/** Folds one Vercel input or output event into the projection. */
export declare function foldVercelEvent(projection: VercelProjection, event: VercelInput | VercelOutput, meta: ReducerMeta): VercelProjection;
//# sourceMappingURL=reducer.d.ts.map

// via re-export: vercel/codec/tool-transitions.d.ts
import type { AI } from "./events.js";
/** Base dynamic tool part fields. */
export interface ToolBase {
    /** Tool name. */
    toolName: string;
    /** Tool call id. */
    toolCallId: string;
}
/** Creates a base dynamic tool part. */
export declare function toolBase(base: ToolBase): AI.UIMessagePart;
/** Applies a legal Vercel dynamic-tool transition. */
export declare function transitionToolPart(part: AI.UIMessagePart | undefined, next: AI.DynamicToolState, patch?: {
    toolName?: string;
    title?: string;
    toolMetadata?: Record<string, unknown>;
    providerExecuted?: boolean;
    input?: unknown;
    output?: unknown;
    errorText?: string;
    approval?: AI.ToolApproval;
    preliminary?: boolean;
    callProviderMetadata?: unknown;
    resultProviderMetadata?: unknown;
}): AI.UIMessagePart;
//# sourceMappingURL=tool-transitions.d.ts.map

// via re-export: vercel/transport/chat-transport.d.ts
import type { ClientSession, CloseOptions } from "../../core/transport/index.js";
import type { AI, VercelInput, VercelOutput, VercelProjection } from "../codec/index.js";
/**
 * Context passed to `prepareSendMessagesRequest`.
 */
export interface SendMessagesRequestContext {
    /** Chat id from Vercel `useChat`, when supplied. */
    chatId?: string;
    /** Vercel send trigger. */
    trigger: "submit-message" | "regenerate-message";
    /** Message id for edit or regenerate requests. */
    messageId?: string;
    /** Historical messages sent in the default POST body. */
    history: readonly AI.UIMessage[];
    /** New messages sent in the default POST body. */
    messages: readonly AI.UIMessage[];
    /** Codec message id that the new branch forks from. */
    forkOf?: string;
    /** Parent codec message id for the new branch. */
    parent?: string;
}
/**
 * Request override returned by `prepareSendMessagesRequest`.
 */
export interface PreparedSendMessagesRequest {
    /** POST body fields merged over the default Vercel/Sockudo body. */
    body?: Record<string, unknown>;
    /** POST headers merged over Vercel request headers. */
    headers?: Record<string, string> | HeadersInit;
}
/**
 * Optional Vercel chat adapter hooks.
 */
export interface ChatTransportOptions {
    /** Customizes the POST body/headers after Sockudo derives branch metadata. */
    prepareSendMessagesRequest?(ctx: SendMessagesRequestContext): PreparedSendMessagesRequest;
}
/**
 * Vercel `useChat` send options consumed structurally by this adapter.
 */
export interface ChatTransportSendMessagesOptions {
    /** Vercel send trigger. */
    trigger: "submit-message" | "regenerate-message";
    /** Chat id supplied by `useChat`. */
    chatId?: string;
    /** Message id for edit or regenerate requests. */
    messageId?: string;
    /** Current Vercel UI message overlay. */
    messages: AI.UIMessage[];
    /** Abort signal owned by `useChat.stop()`. */
    abortSignal?: AbortSignal;
    /** Additional POST headers from `useChat`. */
    headers?: Record<string, string> | HeadersInit;
    /** Additional POST body fields from `useChat`. */
    body?: Record<string, unknown>;
    /** Request metadata passed through by Vercel; not sent by default. */
    metadata?: unknown;
}
/**
 * Vercel `reconnectToStream` options consumed structurally by this adapter.
 */
export interface ChatTransportReconnectOptions {
    /** Chat id supplied by `useChat`. */
    chatId?: string;
    /** Additional POST headers from `useChat`. */
    headers?: Record<string, string> | HeadersInit;
    /** Additional POST body fields from `useChat`. */
    body?: Record<string, unknown>;
    /** Request metadata passed through by Vercel; not sent by default. */
    metadata?: unknown;
}
/**
 * Chat transport that structurally satisfies Vercel AI SDK `ChatTransport` and
 * exposes Ably-compatible Sockudo streaming lifecycle helpers.
 */
export interface ChatTransport {
    /** Whether this transport currently owns an active stream. */
    readonly streaming: boolean;
    /** Sends a Vercel chat request and returns Sockudo-routed UI chunks. */
    sendMessages(options: ChatTransportSendMessagesOptions): Promise<ReadableStream<AI.UIMessageChunk>>;
    /** Returns `null`; Sockudo channel observation handles in-progress streams. */
    reconnectToStream(options: ChatTransportReconnectOptions): Promise<ReadableStream<AI.UIMessageChunk> | null>;
    /** Closes the underlying client transport. */
    close(options?: CloseOptions): Promise<void>;
    /** Subscribes to owned-streaming flag changes. */
    onStreamingChange(cb: (streaming: boolean) => void): () => void;
}
type VercelClientSession = ClientSession<VercelInput, VercelOutput, VercelProjection, AI.UIMessage>;
/**
 * Creates a Vercel `useChat` transport over a Sockudo client session.
 */
export declare function createChatTransport(session: VercelClientSession, chatOptions?: ChatTransportOptions): ChatTransport;
/**
 * Derives continuation inputs by diffing Vercel's optimistic overlay against
 * the assistant message currently materialized by Sockudo's conversation tree.
 */
export declare function deriveContinuationInputs(overlay: AI.UIMessage, treeMessage: AI.UIMessage): VercelInput[];
export {};
//# sourceMappingURL=chat-transport.d.ts.map

// via re-export: vercel/transport/fork-tool-result.d.ts
import type { SendOptions } from "../../core/transport/index.js";
import type { AI, VercelInput } from "../codec/index.js";
/** Successful or failed client-side tool resolution. */
export type ToolCallResolution = {
    output: unknown;
} | {
    errorMessage: string;
};
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
export declare function createToolResultFork(options: CreateToolResultForkOptions): {
    input: VercelInput;
    sendOptions: SendOptions;
};
//# sourceMappingURL=fork-tool-result.d.ts.map

// via re-export: vercel/transport/index.d.ts
export { createChatTransport, deriveContinuationInputs, type ChatTransport, type ChatTransportOptions, type ChatTransportReconnectOptions, type ChatTransportSendMessagesOptions, type PreparedSendMessagesRequest, type SendMessagesRequestContext, } from "./chat-transport.js";
export { createToolResultFork, type CreateToolResultForkOptions, type ToolCallResolution, } from "./fork-tool-result.js";
export { vercelRunEndReason, type VercelFinishReason } from "./run-end-reason.js";
//# sourceMappingURL=index.d.ts.map

// via re-export: vercel/transport/run-end-reason.d.ts
import type { StreamResult, RunEndReason } from "../../core/transport/index.js";
/**
 * Finish reason values returned by Vercel AI SDK stream helpers.
 */
export type VercelFinishReason = string | undefined;
/**
 * Maps a Sockudo pipe result and Vercel finish reason to an AI Transport turn
 * end reason.
 *
 * Non-complete pipe results win immediately and the finish-reason promise is
 * observed in the background to avoid unhandled rejections. Complete streams
 * suspend on Vercel `tool-calls`, complete on all other finish reasons, map
 * abort-shaped rejections to `cancelled`, and map all other rejections to
 * `error`.
 */
export declare function vercelRunEndReason(pipeResult: StreamResult, finishReason: Promise<VercelFinishReason>): Promise<RunEndReason>;
//# sourceMappingURL=run-end-reason.d.ts.map

// via re-export: version.d.ts
/**
 * Current `@sockudo/ai-transport` package version injected by the build.
 *
 * This value is read-only and does not perform I/O.
 *
 * @defaultValue `"0.0.0-dev"` when no build-time version is injected.
 */
export declare const version: string;
//# sourceMappingURL=version.d.ts.map

export { version } from "../version.js";

import {
  createContext,
  createElement,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { ErrorCode, ErrorInfo } from "../errors.js";
import type { InboundMessage } from "../realtime/index.js";
import { useSockudoRealtimeClient } from "../realtime/react.js";
import {
  createClientSession,
  type BranchSelectionIntent,
  type ClientSession,
  type ClientSessionOptions,
  type RunNode,
  type View,
} from "../core/transport/index.js";

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
export type ClientSessionProviderProps<TInput, TOutput, TProjection, TMessage> = Omit<
  ClientSessionOptions<TInput, TOutput, TProjection, TMessage>,
  "client" | "channel" | "channelName"
> & {
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
  transport: ClientSession<TInput, TOutput, TProjection, TMessage>;
  /**
   * Provider construction or lookup error.
   *
   * @defaultValue `undefined` when a transport resolves or lookup is skipped.
   */
  transportError?: ErrorInfo;
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
  transport?: ClientSession<TInput, unknown, unknown, TMessage>;
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
  transport?: ClientSession<TInput, unknown, unknown, TMessage>;
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
  transport?: ClientSession<TInput, unknown, unknown, TMessage>;
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
  transport?: ClientSession<TInput, unknown, unknown, TMessage>;
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
  transport?: ClientSession<TInput, unknown, unknown, TMessage>;
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
  ClientSessionProvider(
    props: ClientSessionProviderProps<TInput, TOutput, TProjection, TMessage>,
  ): ReturnType<typeof createElement>;
  /** Reads the nearest or named client transport. */
  useClientSession(
    options?: UseClientSessionOptions,
  ): UseClientSessionResult<TInput, TOutput, TProjection, TMessage>;
  /** Subscribes to a branch-aware view. */
  useView(options?: UseViewOptions<TInput, TMessage>): ViewHandle<TMessage>;
  /** Creates, owns, and subscribes to an additional view. */
  useCreateView(options?: UseCreateViewOptions<TInput, TMessage>): ViewHandle<TMessage>;
  /** Returns stable tree callbacks without re-rendering on tree changes. */
  useTree(options?: UseTreeOptions<TInput, TMessage>): TreeHandle;
  /** Subscribes to active/suspended turn ownership. */
  useActiveRuns(options?: UseActiveRunsOptions<TInput, TMessage>): Map<string, Set<string>>;
  /** Subscribes to raw normalized inbound messages. */
  useSockudoMessages(
    options?: UseSockudoMessagesOptions<TInput, TMessage>,
  ): readonly InboundMessage[];
}

interface SessionSlot {
  transport?: ClientSession<unknown, unknown, unknown, unknown>;
  error?: ErrorInfo;
}

interface SessionRegistry {
  defaultChannelName?: string;
  slots: Map<string, SessionSlot>;
}

const SessionContext = createContext<SessionRegistry | undefined>(undefined);
const autoLoadedViews = new WeakSet<View<unknown, unknown>>();
const pendingSessionCloses = new WeakMap<
  ClientSession<unknown, unknown, unknown, unknown>,
  { cancelled: boolean }
>();
const stableEmptyMessages: readonly unknown[] = [];
const stableEmptyNodes: readonly RunNode<unknown>[] = [];
const stableEmptyRawMessages: readonly InboundMessage[] = [];
const stableEmptyActiveRuns = new Map<string, Set<string>>();

/**
 * Creates generic React hooks for a specific codec/message type family.
 *
 * @defaultValue Type parameters default to `unknown`.
 *
 * Returned hooks throw only via synchronous handle/stub access for invalid
 * usage; async transport/view methods reject with {@link ErrorInfo}.
 */
export function createSessionHooks<
  TInput = unknown,
  TOutput = unknown,
  TProjection = unknown,
  TMessage = unknown,
>(): SessionHooks<TInput, TOutput, TProjection, TMessage> {
  function ClientSessionProvider(
    props: ClientSessionProviderProps<TInput, TOutput, TProjection, TMessage>,
  ): ReturnType<typeof createElement> {
    const parent = useContext(SessionContext);
    const client = useSockudoRealtimeClient();
    const { children, channelName, ...transportOptions } = props;
    const key = channelName;
    const slot = useMemo<SessionSlot>(() => {
      try {
        return {
          transport: createClientSession({
            ...transportOptions,
            client,
            channelName,
          }),
        };
      } catch (error) {
        return { error: toConstructionError(error) };
      }
    }, [channelName, client]);
    useDeferredSessionClose(slot.transport);
    const registry = useMemo<SessionRegistry>(() => {
      const slots = new Map(parent?.slots);
      if (key !== "") {
        slots.set(key, slot);
      }
      const defaultChannelName = key || parent?.defaultChannelName;
      return defaultChannelName === undefined ? { slots } : { slots, defaultChannelName };
    }, [key, parent, slot]);
    return createElement(SessionContext.Provider, { value: registry }, children);
  }

  function useClientSession(
    options: UseClientSessionOptions = {},
  ): UseClientSessionResult<TInput, TOutput, TProjection, TMessage> {
    const registry = useContext(SessionContext);
    const callbackRef = useRef<((error: ErrorInfo) => void) | undefined>(undefined);
    callbackRef.current =
      options.onError === undefined
        ? undefined
        : (error: ErrorInfo) => {
            options.onError?.(error);
          };
    const slot = resolveSlot(registry, options.channelName);
    const skipped = options.skip === true;
    const transport = skipped ? undefined : slot?.transport;
    const error = skipped ? undefined : (slot?.error ?? missingProviderError(options.channelName));
    useEffect(() => {
      if (!transport || !callbackRef.current) {
        return;
      }
      return transport.on("error", (payload) => {
        callbackRef.current?.(payload);
      });
    }, [transport]);
    if (transport) {
      return {
        transport: transport as ClientSession<TInput, TOutput, TProjection, TMessage>,
      };
    }
    const result: UseClientSessionResult<TInput, TOutput, TProjection, TMessage> = {
      transport: throwingSessionStub(),
    };
    if (error !== undefined) {
      result.transportError = error;
    }
    return result;
  }

  function useView(options: UseViewOptions<TInput, TMessage> = {}): ViewHandle<TMessage> {
    const context = useClientSession(options.skip === undefined ? {} : { skip: options.skip });
    const contextTransport = context.transportError === undefined ? context.transport : undefined;
    const sourceView =
      options.skip === true
        ? undefined
        : (options.view ?? options.transport?.view ?? contextTransport?.view);
    return useViewHandle(sourceView, options.limit);
  }

  function useCreateView(
    options: UseCreateViewOptions<TInput, TMessage> = {},
  ): ViewHandle<TMessage> {
    const context = useClientSession(options.skip === undefined ? {} : { skip: options.skip });
    const contextTransport = context.transportError === undefined ? context.transport : undefined;
    const transport = options.skip === true ? undefined : (options.transport ?? contextTransport);
    const [view, setView] = useState<View<TInput, TMessage> | undefined>();
    useEffect(() => {
      if (options.skip === true || !transport) {
        setView(undefined);
        return;
      }
      const created = transport.createView();
      setView(created);
      return () => {
        created.close();
      };
    }, [options.skip, transport]);
    return useViewHandle(view, options.limit);
  }

  function useTree(options: UseTreeOptions<TInput, TMessage> = {}): TreeHandle {
    const context = useClientSession({
      skip: options.transport !== undefined,
    });
    const contextTransport = context.transportError === undefined ? context.transport : undefined;
    const transport = options.transport ?? contextTransport;
    return useMemo<TreeHandle>(
      () => ({
        getSiblings(id) {
          return requireSession(transport).tree.getSiblings(id);
        },
        hasSiblings(id) {
          return requireSession(transport).tree.hasSiblings(id);
        },
        getNode(id) {
          return requireSession(transport).tree.getNode(id);
        },
      }),
      [transport],
    );
  }

  function useActiveRuns(
    options: UseActiveRunsOptions<TInput, TMessage> = {},
  ): Map<string, Set<string>> {
    const context = useClientSession({
      skip: options.transport !== undefined,
    });
    const contextTransport = context.transportError === undefined ? context.transport : undefined;
    const transport = options.transport ?? contextTransport;
    const [active, setActive] = useState(() => cloneActiveRuns(transport));
    useEffect(() => {
      if (!transport) {
        setActive(stableEmptyActiveRuns);
        return;
      }
      setActive(cloneActiveRuns(transport));
      return transport.tree.on("turn", () => {
        setActive(cloneActiveRuns(transport));
      });
    }, [transport]);
    return active;
  }

  function useSockudoMessages(
    options: UseSockudoMessagesOptions<TInput, TMessage> = {},
  ): readonly InboundMessage[] {
    const context = useClientSession({
      skip: options.skip === true || options.transport !== undefined,
    });
    const contextTransport = context.transportError === undefined ? context.transport : undefined;
    const transport = options.skip === true ? undefined : (options.transport ?? contextTransport);
    const [messages, setMessages] = useState<readonly InboundMessage[]>(stableEmptyRawMessages);
    useEffect(() => {
      if (!transport) {
        setMessages(stableEmptyRawMessages);
        return;
      }
      setMessages(stableEmptyRawMessages);
      return transport.on("message", (message) => {
        setMessages((current) =>
          current === stableEmptyRawMessages ? [message] : [...current, message],
        );
      });
    }, [transport]);
    return options.skip === true ? stableEmptyRawMessages : messages;
  }

  return {
    ClientSessionProvider,
    useClientSession,
    useView,
    useCreateView,
    useTree,
    useActiveRuns,
    useSockudoMessages,
  };
}

const defaultHooks = createSessionHooks();

/**
 * Provides a channel-keyed AI client transport using the outer
 * `@sockudo/client/react` `SockudoProvider`.
 *
 * @defaultValue No default `channelName`; the prop is required.
 *
 * Construction failures are caught and exposed through
 * {@link useClientSession}; children continue to render.
 */
export function ClientSessionProvider(
  props: ClientSessionProviderProps<unknown, unknown, unknown, unknown>,
): ReturnType<typeof createElement> {
  return defaultHooks.ClientSessionProvider(props);
}

/**
 * Reads the nearest or named AI client transport.
 *
 * @defaultValue Uses the nearest provider when `channelName` is omitted.
 *
 * Missing, skipped, and failed providers return a throwing `InvalidArgument`
 * stub; `transportError` is set except when `skip` is true.
 */
export function useClientSession(
  options?: UseClientSessionOptions,
): UseClientSessionResult<unknown, unknown, unknown, unknown> {
  return defaultHooks.useClientSession(options);
}

/**
 * Subscribes to a branch-aware view and returns a reactive snapshot handle.
 *
 * @defaultValue Uses the context transport's default view.
 *
 * Snapshot fields are stable empties before mount; methods that require a
 * resolved view throw {@link ErrorInfo} with {@link ErrorCode.InvalidArgument}.
 */
export function useView(options?: UseViewOptions<unknown, unknown>): ViewHandle<unknown> {
  return defaultHooks.useView(options);
}

/**
 * Creates and owns an additional view for the resolved transport.
 *
 * @defaultValue Uses the context transport and performs no automatic history
 * load unless `limit` is provided.
 *
 * The owned view is closed on unmount or transport change; unresolved transports
 * return the stable empty view handle.
 */
export function useCreateView(
  options?: UseCreateViewOptions<unknown, unknown>,
): ViewHandle<unknown> {
  return defaultHooks.useCreateView(options);
}

/**
 * Returns stable tree callbacks without subscribing to tree updates.
 *
 * @defaultValue Uses the context transport.
 *
 * Callback access throws `InvalidArgument` only when invoked without a resolved
 * transport.
 */
export function useTree(options?: UseTreeOptions<unknown, unknown>): TreeHandle {
  return defaultHooks.useTree(options);
}

/**
 * Subscribes to active/suspended turn ownership.
 *
 * @defaultValue Uses the context transport.
 *
 * Returns a new `Map<clientId, Set<runId>>` reference for each turn event and a
 * stable empty map without a resolved transport.
 */
export function useActiveRuns(
  options?: UseActiveRunsOptions<unknown, unknown>,
): Map<string, Set<string>> {
  return defaultHooks.useActiveRuns(options);
}

/**
 * Subscribes to the raw normalized Sockudo inbound message firehose.
 *
 * @defaultValue Uses the context transport and does not subscribe when `skip` is
 * true.
 *
 * The returned array is append-only for a transport instance and resets on
 * transport changes.
 */
export function useSockudoMessages(
  options?: UseSockudoMessagesOptions<unknown, unknown>,
): readonly InboundMessage[] {
  return defaultHooks.useSockudoMessages(options);
}

function useViewHandle<TInput, TMessage>(
  view: View<TInput, TMessage> | undefined,
  limit: number | undefined,
): ViewHandle<TMessage> {
  const [snapshot, setSnapshot] = useState(() => viewSnapshot(view));
  const viewRef = useRef(view);
  viewRef.current = view;
  useEffect(() => {
    setSnapshot(viewSnapshot(view));
    if (!view) {
      return;
    }
    const unsubscribe = view.on("update", () => {
      setSnapshot(viewSnapshot(view));
    });
    return unsubscribe;
  }, [view]);
  useEffect(() => {
    if (!view || limit === undefined || autoLoadedViews.has(view)) {
      return;
    }
    autoLoadedViews.add(view);
    if (view.hasOlder() && !view.loading) {
      void view.loadOlder(limit);
    }
  }, [limit, view]);
  const loadOlder = useCallback(async (requestedLimit?: number): Promise<void> => {
    const current = viewRef.current;
    if (!current || current.loading) {
      return;
    }
    await current.loadOlder(requestedLimit);
    setSnapshot(viewSnapshot(current));
  }, []);
  const select = useCallback((id: string, index: number, intent?: BranchSelectionIntent): void => {
    viewRef.current?.select(id, index, intent);
    setSnapshot(viewSnapshot(viewRef.current));
  }, []);
  const getSelectedIndex = useCallback((id: string): number => {
    return viewRef.current?.getSelectedIndex(id) ?? 0;
  }, []);
  const getSiblings = useCallback((id: string): readonly RunNode<unknown>[] => {
    return viewRef.current?.getSiblings(id) ?? stableEmptyNodes;
  }, []);
  const hasSiblings = useCallback((id: string): boolean => {
    return viewRef.current?.hasSiblings(id) ?? false;
  }, []);
  const getNode = useCallback((id: string): RunNode<unknown> | undefined => {
    return viewRef.current?.getNode(id);
  }, []);
  const send = useCallback((message: TMessage): Promise<unknown> => {
    return requireView(viewRef.current).send(message);
  }, []);
  const regenerate = useCallback((target: string, parent: string): Promise<unknown> => {
    return requireView(viewRef.current).regenerate(target, parent);
  }, []);
  const edit = useCallback((messageId: string, message: TMessage): Promise<unknown> => {
    return requireView(viewRef.current).edit(messageId, message);
  }, []);
  const update = useCallback((messageId: string, patch: unknown): Promise<unknown> => {
    return requireView(viewRef.current).update(messageId, patch);
  }, []);
  return useMemo(
    () => ({
      messages: snapshot.messages,
      nodes: snapshot.nodes,
      hasOlder: snapshot.hasOlder,
      loading: snapshot.loading,
      ...(snapshot.loadError !== undefined ? { loadError: snapshot.loadError } : {}),
      loadOlder,
      select,
      getSelectedIndex,
      getSiblings,
      hasSiblings,
      getNode,
      send,
      regenerate,
      edit,
      update,
    }),
    [
      edit,
      getNode,
      getSelectedIndex,
      getSiblings,
      hasSiblings,
      loadOlder,
      regenerate,
      select,
      send,
      snapshot,
      update,
    ],
  );
}

function useDeferredSessionClose(
  transport: ClientSession<unknown, unknown, unknown, unknown> | undefined,
): void {
  useEffect(() => {
    if (transport) {
      const pending = pendingSessionCloses.get(transport);
      if (pending) {
        pending.cancelled = true;
        pendingSessionCloses.delete(transport);
      }
    }
    return () => {
      if (!transport) {
        return;
      }
      const pending = { cancelled: false };
      pendingSessionCloses.set(transport, pending);
      queueMicrotask(() => {
        if (!pending.cancelled) {
          pendingSessionCloses.delete(transport);
          void transport.close();
        }
      });
    };
  }, [transport]);
}

function viewSnapshot<TInput, TMessage>(
  view: View<TInput, TMessage> | undefined,
): {
  messages: readonly TMessage[];
  nodes: readonly RunNode<unknown>[];
  hasOlder: boolean;
  loading: boolean;
  loadError?: ErrorInfo;
} {
  if (!view) {
    return {
      messages: stableEmptyMessages as readonly TMessage[],
      nodes: stableEmptyNodes,
      hasOlder: false,
      loading: false,
    };
  }
  return {
    messages: view.getMessages(),
    nodes: view.flattenNodes(),
    hasOlder: view.hasOlder(),
    loading: view.loading,
    ...(view.loadError !== undefined ? { loadError: view.loadError } : {}),
  };
}

function resolveSlot(
  registry: SessionRegistry | undefined,
  channelName: string | undefined,
): SessionSlot | undefined {
  const key = channelName ?? registry?.defaultChannelName;
  return key === undefined ? undefined : registry?.slots.get(key);
}

function cloneActiveRuns(
  transport: ClientSession<unknown, unknown, unknown, unknown> | undefined,
): Map<string, Set<string>> {
  if (!transport) {
    return stableEmptyActiveRuns;
  }
  const clone = new Map<string, Set<string>>();
  for (const [clientId, turns] of transport.tree.getActiveRunIds()) {
    clone.set(clientId, new Set(turns));
  }
  return clone;
}

function missingProviderError(channelName: string | undefined): ErrorInfo {
  return new ErrorInfo({
    code: ErrorCode.InvalidArgument,
    statusCode: 400,
    message:
      channelName === undefined
        ? "unable to resolve client transport; no ClientSessionProvider is available"
        : `unable to resolve client transport; no ClientSessionProvider for channel ${channelName}`,
  });
}

function toConstructionError(error: unknown): ErrorInfo {
  if (error instanceof ErrorInfo) {
    return error;
  }
  return new ErrorInfo({
    code: ErrorCode.InvalidArgument,
    statusCode: 400,
    message: "unable to create client transport in ClientSessionProvider",
    cause: error,
  });
}

function throwingSessionStub<TInput, TOutput, TProjection, TMessage>(): ClientSession<
  TInput,
  TOutput,
  TProjection,
  TMessage
> {
  return new Proxy(
    {},
    {
      get(): never {
        throw missingProviderError(undefined);
      },
    },
  ) as ClientSession<TInput, TOutput, TProjection, TMessage>;
}

function requireView<TInput, TMessage>(
  view: View<TInput, TMessage> | undefined,
): View<TInput, TMessage> {
  if (!view) {
    throw missingProviderError(undefined);
  }
  return view;
}

function requireSession(
  transport: ClientSession<unknown, unknown, unknown, unknown> | undefined,
): ClientSession<unknown, unknown, unknown, unknown> {
  if (!transport) {
    throw missingProviderError(undefined);
  }
  return transport;
}

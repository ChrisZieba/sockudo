# @sockudo/ai-transport

## 3.0.0 - 2026-08-17

### Breaking changes

- Renamed the public transport/session API to the Session/Run vocabulary: `createClientTransport` to
  `createClientSession`, `createServerTransport` to `createAgentSession`, Turn types to Run types,
  `ConversationTree` to `Tree`, and `runDirectLlmTurn` to `runDirectLlm`.
- Renamed the public `ErrorCode` members to Session/Run names while preserving their numeric wire
  values.
- Write `run-continue` for new continuations. Existing `turn-continue` history remains readable, but
  it is no longer emitted.
- Require `@sockudo/client` 2.x; the coordinated release uses 2.2.0.

### Added

- Four-arm run lifecycle events (`start`, `suspend`, `resume`, and `end`), step lifecycle support,
  steering, and configurable AI header ceilings.
- Recovery-aware branching, supersession, complete tree projection seeds, and `createToolResultFork`
  for direct view-driven tool resolution.
- Ably AI Transport compatibility coverage and Vercel codec/transport parity for recovery,
  branching, steering, steps, and tool results.

### Fixed

- Correctly distinguish capability-denied, token-expired, and token-revoked failures while keeping
  SDK-local invalid arguments in the SDK error namespace.
- Preserve recovery metadata, full-width serials, future frames, and `extras.ai` across the realtime
  adapter.
- Refresh the demo and build dependency graph to patched releases and consume `protobufjs` 7.6.5
  from the coordinated `@sockudo/client` workspace package.

## 2.1.0 - 2026-06-27

- Hardened adapter-level forward-compatibility over `@sockudo/client` by replaying the shared E1
  fixtures, preserving `extras.ai`, keeping unsafe serials as `number | string`, skipping event-less
  frames, and treating future internal mutable actions as no-op summary frames.
- Force the realtime adapter onto Protocol V2, reject explicit V1 options, and distinguish canonical
  insufficient-capability (`40003`), token-expired (`40142`), and token-revoked (`40160`) errors.
  Local `InvalidArgument` now uses the SDK-local `104012` code so it cannot alias `40003`.

## 0.1.0

Initial GA release candidate for Sockudo AI Transport wire protocol v1.

- Core client/server transport APIs with Ably AI Transport parity.
- React, Vue, Svelte, Vercel framework, and direct provider subpath exports.
- Direct OpenAI SDK, OpenAI-compatible HTTP/SSE, Anthropic SDK, and common compatible provider
  presets.
- Versioned-message streaming, cancellation, history/recovery, branching, and view helpers.
- Public API snapshot, bundle budgets, benchmark guard, and dry-run release checks.

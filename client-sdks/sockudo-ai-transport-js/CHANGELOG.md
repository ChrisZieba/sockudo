# @sockudo/ai-transport

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

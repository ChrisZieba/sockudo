# Transport

The transport layer owns runs, cancellation, history, and tree synchronization. It does not own
WebSocket protocol logic; `@sockudo/client` supplies connection, recovery, channel auth, history,
presence, and mutable-message helpers.

Client sends publish `ai-input`, then poke the application API. Agents publish `ai-output` and the
`ai-run-*` lifecycle events with trusted credentials.

Client tool-result forks stamp `supersedes` with the suspended run id they replace. The tree keeps
that run available for direct lookup and history replay, while excluding it from visible branch and
active-run results.

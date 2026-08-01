# 0.1 Public API Parity Diff

Source of truth: `plans/ai-transport/02-sdk-prompts.md` section 1 and
`docs/specs/ai-transport-wire-protocol.md`.

| Surface                 | Sockudo 0.1 status | Notes                                                          |
| ----------------------- | ------------------ | -------------------------------------------------------------- |
| Entry points            | ✅                 | `.`, `/react`, `/vue`, `/svelte`, `/vercel/*`, `/providers`    |
| Realtime substrate      | ✅                 | `@sockudo/client` peer dependency only through `src/realtime/` |
| Client factory          | ✅                 | `createClientSession`                                          |
| Server factory          | ✅                 | `createAgentSession`                                           |
| Server run API          | ✅                 | `createRun`, `start`, `addMessages`, `streamResponse`, `end`   |
| Run reasons             | ✅                 | `complete`, `cancelled`, `error`, `suspended`                  |
| Client session surface  | ✅                 | tree/view/cancel/wait/stage/close/error                        |
| View surface            | ✅                 | send/edit/regenerate/branch navigation/history                 |
| ClientRun               | ✅                 | stream, ids, optimistic ids, cancel, steer                     |
| CancelFilter            | ✅                 | `runId`, `own`, `clientId`, `all`; default own                 |
| Codec core              | ✅                 | encoder/decoder/accumulator/lifecycle helpers                  |
| Vercel helpers          | ✅                 | UIMessage codec, chat transport, run-end reason                |
| React helpers           | ✅                 | providers, view hooks, active runs, message sync               |
| Vue helpers             | ✅                 | composables, view refs, active runs, Vercel chat transport     |
| Svelte helpers          | ✅                 | stores, view stores, active runs, Vercel chat transport        |
| Direct provider helpers | ✅                 | OpenAI SDK, OpenAI-compatible HTTP/SSE, Anthropic SDK          |
| Error surface           | ✅                 | `ErrorInfo`, `ErrorCode`, `errorInfoIs`                        |
| Steering                | ✅                 | `ClientRun.steer`, `hasInput`, `onSteer`, stamp outcomes       |
| Step lifecycle          | ✅                 | `createStep`, attempts, retry supersession                     |

Public API freeze is enforced by `pnpm api:snapshot`.

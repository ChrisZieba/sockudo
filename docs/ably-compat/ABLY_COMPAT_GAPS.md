# Ably Compatibility Gaps

This file is the structured list of intentional exceptions and remaining release work for
Sockudo's opt-in `ably-compat` facade. The selected scope is the pinned Ably REST/WebSocket
manifest, excluding Live Objects and non-WebSocket realtime transports.

## Intentional Exceptions

| Exception | Classification | Structured definition |
| --- | --- | --- |
| Full Ably platform API parity | Not claimed | `scope/advertised-surface.md` limits the public claim to the selected REST/WebSocket surface. |
| LiveObjects/object modes | Out of scope | `test/rest/liveobjects.test.js` and `test/realtime/liveobjects.test.js` are excluded in both manifests. |
| Multiple and non-WebSocket realtime transports | Out of scope | `test/realtime/transports.test.js` is excluded; all active realtime cases are forced to `web_socket`. |
| Behavior outside the selected manifests | Not claimed | A new or renamed upstream file makes the runner fail classification rather than silently reducing coverage. |

There are no per-test exclusions in the pinned Node or browser manifests. In latest-upstream CI,
only three exact SDK/harness assertions are filtered: the Comet transport inventory and two SDK
default TLS/port checks replaced by loopback sandbox routing. The official ably-go and AI Transport
suites have no test-body or assertion exclusions.

## Remaining Release Work

| Item | Status |
| --- | --- |
| Published artifact containing the current compatibility changes | Pending a new version/tag. Existing published assets predate this evidence and must not inherit it. |
| Released-binary verification | Implemented as `SOCKUDO_RELEASE_TAG=vX.Y.Z make release-verify` and the dispatchable `Released Sockudo compatibility` workflow; it becomes green only after it runs against the new tag. |
| Independent release load/soak budget | Still red in the retained run: publish and delivery p99 exceed their separate budgets, and the full three-run topology matrix is incomplete. |

## Closed Compatibility Findings

| Finding | Resolution and evidence |
| --- | --- |
| Go pending-ACK queue panicked after reconnect | A low-latency reconnect can write the same queued `ProtocolMessage` twice on its replacement transport. Sockudo now tracks the inbound serial frontier per transport, processes and answers the first copy once, suppresses later copies on that transport, and starts a fresh frontier for a replacement transport so a genuinely lost ACK can be recovered. The upstream empty-queue panic remains enabled as a regression guard. |
| Four `resume_lost_continuity` failures | The pinned upstream fixture accidentally called the return value of `recordPrivateApi(...)` because of a parenthesis/comma typo. That helper returns `undefined`, so the body stopped before its protocol assertion. Patch `0006` corrects only those setup statements; no assertion or expected value changes. All four JSON/MsgPack expansions now pass, and strict completeness is 250/250. |
| Browser diagnostics made otherwise-passing assertions red | The local browser harness now distinguishes expected failed connections, uses browser-supported message decoding, and settles each upstream result once. Chromium defaults and strict completeness finish with zero assertion, page, console, context, or external-request errors. |
| Second core SDK absent | ably-go is pinned and runs its official unit suite plus the official integration suite with `-race` in JSON and MsgPack. Latest-upstream CI also follows current `ably-go` `main`; every SDK patch hash is retained with the evidence. |
| Go REST pagination stopped after page one | Sockudo now emits each `first`/`next` relation as a separate `Link` header value, matching the official Go SDK's iterator while preserving HTTP Link semantics for ably-js. |
| Go token capability/JWT/realtime publish differences | Capability intersection accepts Ably's `[*]*` wildcard, invalid JWT signatures return Ably code `40144`, and a realtime message may carry only its own server-assigned `connectionId`, which is stripped before canonical publish. Focused Rust and official Go assertions cover each rule. |
| Go connection fixture timed out before `CONNECTED` | Sockudo's heartbeat interval used Tokio's immediate first tick, so `heartbeats=true` could emit `HEARTBEAT` before `CONNECTED`. The interval now begins after its first full period, preserving `CONNECTED` as the first successful protocol frame. |
| Go reconnect dropped or reordered messages | Abnormal disconnects now retain a bounded per-subscriber recovery window, transfer it only to the authenticated resumed session, send `ATTACHED` before replay, preserve message order, and fail closed with `90003` on overflow. The official RTN15 reconnect/recovery cases pass in JSON and MsgPack. |
| Go explicit reauthorization entered `DISCONNECTED` | A failed explicit `AUTH` update now emits the required connection `ERROR` with the authentication reason, causing the SDK to enter `FAILED`. |
| Go multi-message idempotent IDs accepted invalid batches | REST publish now validates one common base ID with consecutive `:0..n` suffixes and rejects malformed batches with protocol code `40031`. |
| Go stats fixtures and typed responses diverged | Fixture ingestion accepts both JSON and MsgPack, the bounded field limit admits the canonical legacy SDK shape, transport and aggregate counters no longer double-count, and responses expose both legacy nested fields and the modern flattened `entries` map. Pagination, bounds, direction, and rollups pass in both formats. |
| Go presence-history pagination lost its route | The `first`/`next` relations now resolve relative to the existing `/presence/history` endpoint instead of duplicating path segments. Multi-page and `First()` navigation pass. |
| Go channel status reported a retained channel inactive | Channel status now considers both live occupancy and retained history, so a successfully published persistent channel reports active. |
| Go local fallback tests depended on public hosts | The structured portability patches preserve the original SDK-visible hostnames while routing only an explicit allowlist to loopback, retain the official fallback/internet assertions, and refuse unlisted hosts. Fixture ingestion and local child proxy destinations are configurable without changing expected values. |
| Executable release runner absent | The harness can now download a tagged Linux asset, verify its detached SHA-256, reject an unexpected archive layout, and execute every required lane against that binary with provenance retained in reports. |
| Public ownership/support wording absent | Public docs now state that Sockudo and the compatibility layer are community-built and community-maintained, not Ably products, and not supported by Ably. |

## Required Before a Broader Claim

- Publish a new release containing these changes and retain a green released-binary workflow artifact.
- Keep the manifest-defined exceptions visible beside any compatibility statement.
- Keep Pusher V1 and Sockudo V2 conformance green with and without `ably-compat`.
- Satisfy the independent performance/security/chaos release policy before product promotion.
- Expand the claim only when the pinned manifests and retained evidence expand with it.

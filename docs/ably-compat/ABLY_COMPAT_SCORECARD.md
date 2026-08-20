# Ably REST, WebSocket, and AI Transport Compatibility Scorecard

Evidence refreshed: 2026-08-13

Sockudo's opt-in `ably-compat` feature exposes the selected Ably REST and WebSocket surface while
the native Sockudo/Pusher `/app/{appKey}` route remains unchanged. The pinned Node, Chromium,
strict-completeness, and AI Transport lanes are green. The second-SDK ably-go lane is now part of
both pinned release evidence and latest-upstream CI.

This is source-build evidence. A published release containing these changes must still pass the
released-binary workflow before that tag is promoted as verified.

## Target Versions

| Surface | Target | Evidence |
| --- | --- | --- |
| Ably AI Transport package | `@ably/ai-transport@0.4.0`, source `e817858c3e66931c7847cd08bab88384288c9bea` | Complete pinned unit and integration manifests. |
| Ably JS upstream suite | `ably@2.21.0`, source `400c6a42ff4903cd3bba3c556f75dfbea1b74448` | 41-file REST/WebSocket manifest under `sockudo-compatibility/`. |
| Ably Go upstream suite | source `26cb17198f4fb62a54aa2ca7c7ba1127ebdaaccc` | Official unit suite and official integration suite in JSON and MsgPack. |
| Minimum browser runtime | Playwright `1.61.1`, Chromium `149.0.7827.55` | Exact revision and every selected file are pinned in `scope/browser-manifest.json`. |
| Sockudo server workspace | `5.0.0` source tree | The evidence below uses a locally built binary; released-binary evidence is required before promoting the `v5.0.0` asset. |

## Compatibility Summary

| Area | Status | Notes |
| --- | --- | --- |
| Pusher Protocol V1 compatibility | Supported | Ably compatibility is additive and must not alter V1 wire behavior. |
| Sockudo-native AI Transport | Supported | Uses Cargo feature `ai-transport` plus runtime `[ai_transport]`; it does not require `ably-compat`. |
| Ably facade feature gate | Supported | Root Ably WebSocket and REST routes are compiled only with `ably-compat`. |
| Ably Realtime JSON/MsgPack over WebSocket | Pinned Node, Chromium, and Go evidence green | No fallback realtime transport is implemented or claimed. |
| Ably REST selected surface | Pinned Node, Chromium, and Go evidence green | Auth, time, publish, history, pagination, stats, status, batch, presence, mutable messages, annotations, and the selected push-recipient surface are covered. |
| History, rewind, recovery, presence, and SYNC | Default and strict lanes green | Every audited upstream-default pending body runs in the separate strict lane. |
| Mutable messages and annotations | Green for the selected suites | They use Sockudo's native version and annotation services. |
| LiveObjects/object modes | Intentionally out of scope | Both Live Objects files are structurally excluded by the reviewed manifest. |
| Non-WebSocket Ably realtime transports | Intentionally out of scope | Comet, XHR polling/streaming, SSE, long polling, and transport fallback are not implemented. |
| Full Ably platform API parity | Not claimed | Evidence applies only to the selected REST and WebSocket surface. |

## Stable Commands

```bash
cd sockudo-compatibility
make conformance
make strict-completeness
make browser-conformance
make browser-strict
make go-conformance
make ait-conformance
```

To verify a published Linux release and its detached checksum instead of compiling source:

```bash
SOCKUDO_RELEASE_TAG=vX.Y.Z make release-verify
```

The released-binary command downloads the matching musl asset and `.sha256` file, validates the
manifest and digest, verifies the archive contains only the `sockudo` binary, and runs every lane
above against that exact binary. The dispatchable `Released Sockudo compatibility` workflow
retains the reports, pins, scopes, and patches as artifacts.

## Fresh Pinned Results

| Lane | Result | Status |
| --- | --- | --- |
| Node upstream defaults | 41 files; 575 passed; 0 failed; 27 upstream-default pending; 0 runner errors | Green |
| Node strict completeness | 6 files; 250 passed; 0 failed; 0 pending | Green |
| Chromium defaults | 41 files; 574 passed; 0 failed; 27 upstream-default pending; 0 page, console, context, or external-request errors | Green |
| Chromium strict | 6 files; 250 passed; 0 failed; 0 pending; 0 browser-boundary errors | Green |
| Ably Go unit | 318 passed; 0 failed; 0 skipped | Green |
| Ably Go integration, JSON | 457 passed; 0 failed; 6 upstream-declared skips | Green |
| Ably Go integration, MsgPack | 457 passed; 0 failed; 6 upstream-declared skips | Green |
| AI Transport | 4 files; 50 passed; 0 failed; 0 pending | Green |

The 27 pending results in each default ably-js lane are not exceptions or exclusions. They are
unchanged upstream defaults, audited in `scope/pending-audit.json`; their 250 expanded assertions
all pass in the separate strict lane.

## Structured Scope and Exceptions

- `scope/scope-manifest.json` and `scope/browser-manifest.json` classify every upstream test file.
- Exactly two Live Objects files and the multi-transport file are excluded; `testExcludes` is empty.
- `scope/pending-audit.json` identifies every upstream-pending declaration and its expanded result
  count.
- `scope/known-failures.json` is the reviewed in-scope failure ledger; the runner fails on new
  failures and on stale entries.
- The ably-go assertions and expected values are unchanged. Its retained patches provide the
  static-app, loopback transport, fixture URL, and local proxy-destination contracts; its
  pending-ACK validation remains intact, and every applied patch hash is reported.
- Every report records source revisions, applied-patch hashes, binary provenance, and binary SHA-256.

## Independent Product Release Gates

Compatibility evidence does not override independent security, chaos, fuzz, or performance gates.
The retained two-node burst is correctness-clean (10,000 publishes and 1,000,000 deliveries with
zero loss, duplicates, reordering, unexpected messages, or gaps), but its recorded latency exceeds
the separate release-load budgets. That performance gate remains tracked independently of SDK
protocol compatibility.

## Compatibility Claim

For a tag that has passed `release-verify`, the evidence-backed wording is:

- **Ably REST and WebSocket compatibility, excluding Live Objects**, with WebSocket-only transport
  scope and the pinned report attached.
- **AI Transport compatibility**, supported by its separate complete suite and unmodified browser
  demo/recovery evidence.

Do not use “complete Ably support” or “full Ably platform compatibility.” Sockudo and this layer are
community-built and community-maintained, are not Ably products, and are not supported by Ably.

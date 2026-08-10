# Ably AI Transport Compatibility Test Plan

This plan separates Sockudo-native AI Transport verification from the optional Ably compatibility
facade. Native AI Transport uses Cargo feature `ai-transport`; the Ably facade additionally requires
Cargo feature `ably-compat`.

## Required Local Service

```bash
cargo run -p sockudo --features "v2,ai-transport,ably-compat,redis,postgres,push" -- \
  --config config/config.toml
```

The default repository config uses app key `app-key`, app secret `app-secret`, port `6001`, and AI
channel prefix `private-ai-`.

## Stable Commands

```bash
make ably-compat-test
make ably-protocol-discovery
make ably-ai-transport-test
make ably-ai-demo
```

## Pull Request Latest-Upstream Gate

Every pull request runs `.github/workflows/ably-upstream-compat.yml`. The workflow checks out the
current `main` heads of `ably/ably-js` and `ably/ably-ai-transport-js` at job start, records the
resolved commit IDs in the GitHub step summary and diagnostic artifact, and tests the pull request's
Sockudo merge commit. No Ably credentials or long-lived test app are required. Sockudo's public
`tests/ably-compat/upstream-sandbox.mjs` provisioner creates isolated, randomly keyed Sockudo
children locally from the repository's own config template. This keeps the pull request workflow
usable from forks without a private-repository token.

The `ably-js` job runs every Node `test/unit`, `test/rest`, and `test/realtime` file discovered on
upstream `main`, except:

- `test/rest/liveobjects.test.js` and `test/realtime/liveobjects.test.js`, because Live Objects is
  outside Sockudo's advertised compatibility surface;
- `test/realtime/transports.test.js`, because Sockudo realtime is WebSocket-only;
- the exact `node_transports` assertion, which requires the SDK's Comet inventory; and
- two SDK-default TLS/port assertions whose expected defaults are necessarily replaced by the
  local sandbox routing. The remaining connectivity, REST fallback, and init assertions still run.

`ABLY_TEST_TRANSPORTS=web_socket` forces all parameterized realtime cases onto WebSocket. New files
are included automatically; the file list is printed so an upstream rename cannot silently shrink
coverage. Upstream-default pending tests remain pending, matching Ably's own Node command.

The `ably-ai-transport-js` job has no test exclusions. It runs the complete unit suite followed by
the complete integration suite against the same local Sockudo sandbox. The two jobs are deliberately
separate from pinned release evidence: latest-upstream CI detects drift, while a release claim must
still cite immutable source revisions and retained reports.

Release evidence uses the pinned harness directly and keeps each lane separate:

```bash
cd sockudo-compatibility
make conformance
make strict-completeness
make browser-conformance
make browser-matrix
make browser-strict
make ait-conformance

cd ..
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p sockudo-ably-compat --all-features
```

Browser reports fail on assertion failures, runner errors, page errors, console errors, leaked
contexts, and unexpected external requests. Failure screenshots and Playwright traces are retained
under `sockudo-compatibility/reports/browser-artifacts/`; engine diagnostics are not filtered to
make a lane green.

The strict lane enables all 27 in-scope Mocha results represented by the 11 recorded upstream
pending declarations. The pinned source expands to 27 results rather than the 25 estimated in the
original work request; `scope/pending-audit.json` records every stable title. The lane does not
rewrite bodies or expectations. Pinned upstream defects remain reported as release blockers.

The legacy smoke targets use `tests/ably-compat`, stock `ably@2.21.0`, and
`@ably/ai-transport@0.4.0`. The Ably facade is expected at the Sockudo root WebSocket/REST endpoint;
existing Sockudo/Pusher clients continue to use `/app/{appKey}` and `/apps/{appId}`.
`make ably-protocol-discovery` also installs Playwright Chromium and runs one stock browser bundle
Pub/Sub/history lane from an allowed localhost origin.

## Required Test Cases

| Case | Command |
| --- | --- |
| Ably Realtime connect, attach, publish, receive, history, detach | `make ably-compat-test` |
| Broader stock `ably` SDK discovery across JSON, REST publish/history, presence, token, token capability enforcement, MsgPack, and browser lanes | `make ably-protocol-discovery` |
| Ably mutable message create/append/update/delete/version reads | `make ably-compat-test` |
| Ably recovery/history replay during streamed output | `make ably-compat-test` |
| Stock `@ably/ai-transport` chat smoke | `make ably-ai-transport-test` |
| Headless chat and recovery demos | `make ably-ai-demo` |

## Claim Gate

Do not claim full Ably support from these tests. The passing claim is:

> Sockudo implements an opt-in Ably REST and WebSocket compatibility surface, excluding Live
> Objects; publish a compatibility claim only for lanes with attached passing evidence.

The pinned Node, strict-completeness, browser, and AIT lanes are independent gates. Broader or
unqualified claims require every selected lane to pass and updated scorecard evidence.

Security, fuzz, chaos, and performance evidence are also independent gates. The release load guard
requires three independent runs across both topologies and every scenario in
`tests/load/ably-compat/profiles/release.json`; a correctness-clean run that exceeds a latency or
memory budget is not a performance pass.

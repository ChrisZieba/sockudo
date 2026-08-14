# Ably AI Transport Compatibility Test Plan

This plan separates Sockudo-native AI Transport verification from the optional Ably compatibility
facade. Native AI Transport uses Cargo feature `ai-transport`; the facade additionally requires
`ably-compat`.

## Pull Request Latest-Upstream Gate

Every pull request runs `.github/workflows/ably-upstream-compat.yml`. It builds the pull request's
Sockudo binary once, records all resolved revisions, and tests the current `main` branches of:

- `ably/ably-js` in Node over WebSocket;
- `ably/ably-go` using its official unit suite and its official integration suite in JSON and
  MsgPack with the race detector; and
- `ably/ably-ai-transport-js` using its complete unit and integration suites.

No Ably credentials or long-lived test app are required. The local provisioner creates isolated,
randomly keyed Sockudo children from repository configuration, so the workflow remains usable from
fork pull requests.

The discovered ably-js files exclude exactly:

- `test/rest/liveobjects.test.js` and `test/realtime/liveobjects.test.js`, because Live Objects is
  outside the advertised surface; and
- `test/realtime/transports.test.js`, because Sockudo realtime is WebSocket-only.

Three exact ably-js assertions are filtered because the local topology necessarily replaces their
SDK defaults: the Comet transport inventory and two default TLS/port assertions. Every other
discovered unit, REST, and realtime test runs. `ABLY_TEST_TRANSPORTS=web_socket` forces parameterized
realtime cases onto the advertised transport.

The ably-go patches load the generated static app, point the JWT authURL and stats-fixture helpers
at bounded loopback services, and adapt HTTPS/WSS only when the explicit test environment flag is
set. The exact primary, fallback, and internet-probe hostnames used by the official tests are
tunnelled through an allowlist while remaining unchanged to the SDK; unlisted non-loopback hosts
are refused. The idempotent-retry fixtures resolve the provisioned local child's endpoint and port.
A separate one-line SDK fix ignores a duplicate ACK after the pending queue has already drained,
matching the SDK's own surrounding contract instead of panicking in the low-latency reconnect
fixture. Assertions and expected values are unchanged. The AI Transport job has no exclusions.

## Pinned Source-Evidence Gate

Release claims use immutable pins and separate reports:

```bash
cd sockudo-compatibility
npm ci
make conformance
make strict-completeness
make browser-install
make browser-conformance
make browser-strict
make go-conformance
make ait-conformance
```

The default, strict-completeness, browser, Go, and AI Transport reports remain independent. A pass
in one lane is never reported as a pass in another. Reports record SDK and server revisions,
patch hashes, binary path and SHA-256, selected scope, and exact results.

Browser reports fail on assertion failures, runner errors, page errors, console errors, leaked
contexts, unexpected external requests, missing definitions, and unexpected pending results.
Failure screenshots and Playwright traces are retained under
`sockudo-compatibility/reports/browser-artifacts/`.

The strict lane executes every result expanded from the 11 upstream-pending declarations. The
pinned source expands those declarations to 27 default results and 250 strict assertions. Patch
`0006` fixes a setup-only parenthesis/comma typo in four resume expansions; it does not change an
assertion or expected value. `scope/pending-audit.json` records every stable identity and rationale.

## Published-Release Gate

After publishing a release that includes the compatibility changes, verify the exact artifact:

```bash
cd sockudo-compatibility
SOCKUDO_RELEASE_TAG=vX.Y.Z make release-verify
```

The runner:

1. selects the Linux asset matching the runner architecture;
2. downloads the archive and detached `.sha256` file from that explicit tag;
3. validates the checksum manifest, SHA-256 digest, and one-binary archive layout;
4. runs Node default and strict, Chromium default and strict, Go unit/JSON/MsgPack, and AI
   Transport against the downloaded binary; and
5. writes the tag, target, binary hash, pins, patches, scopes, and results into retained evidence.

The standalone harness also exposes a manual `Released Sockudo compatibility` workflow with a
required `release_tag` input. A tag is not called verified until that workflow completes green.

## Legacy Smoke Commands

The repository-level smoke targets remain useful for narrow iteration:

```bash
make ably-compat-test
make ably-protocol-discovery
make ably-ai-transport-test
make ably-ai-demo
```

They use stock `ably@2.21.0` and `@ably/ai-transport@0.4.0` against the root Ably endpoint. Existing
Sockudo/Pusher clients continue to use `/app/{appKey}` and `/apps/{appId}`.

## Claim Gate

Do not claim full Ably platform support. For a tag with attached passing released-binary evidence,
the accurate claim is:

> Sockudo implements a community-maintained Ably REST and WebSocket compatibility surface,
> excluding Live Objects and non-WebSocket realtime transports. Sockudo is not an Ably product and
> is not supported by Ably.

Security, fuzz, chaos, and performance evidence remain independent product release gates.

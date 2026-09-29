# Sockudo AI Transport Conformance

Raw protocol conformance harness for the AIT-S SDK executable spec.

This suite intentionally has no Sockudo SDK dependency. It uses Node `fetch` plus the built-in
WebSocket client to exercise the server wire surface directly.

## Run

Start a Sockudo server with AI Transport, Protocol V2, connection recovery, history, and versioned
messages enabled. Disable AI rollup for this original-mutation transcript lane, then:

```bash
cd tests/ai-conformance
node src/run.mjs
```

Defaults:

- `SOCKUDO_BASE_URL=http://127.0.0.1:6001`
- `SOCKUDO_WS_URL=ws://127.0.0.1:6001/app/app-key?protocol=2&client=ait-conformance&version=0`
- `SOCKUDO_APP_ID=app-id`
- `SOCKUDO_APP_KEY=app-key`
- `SOCKUDO_APP_SECRET=app-secret`

Live runs report every scenario failure and exit nonzero if any scenario fails.
The suite sends object data directly to the documented HTTP publish endpoint; it does not
stringify objects to bypass input parsing errors.

Identity-check unit tests:

```bash
node --test src/protocol-client.test.mjs
```

Offline fixture validation:

```bash
AIT_CONFORMANCE_OFFLINE=1 node src/run.mjs
```

## Golden Transcripts

Golden files live in `fixtures/golden/`. Runtime serials, timestamps, UUID-like IDs, and socket IDs
are normalized before comparison. The golden transcripts are the SDK-facing executable spec for
canonical AI Transport sequences. The full frame shape is compared; metadata is not discarded.
Before normalization, each live frame and HTTP latest record must match the exact message, version,
history and delivery identities in its HTTP acknowledgement. Message identity and history position
remain stable across mutations, while version and delivery positions strictly increase.

WebSocket identities are carried in `extras.headers.sockudo_*` and top-level `serial`/`stream_id`.
HTTP latest/history responses use flattened version metadata; see
[`ai-transport-wire-protocol.md`](../../docs/specs/ai-transport-wire-protocol.md).
The `recoverySmoke` scenario checks recovery metadata on delivery; it does not exercise reconnect.

Set `AIT_CONFORMANCE_RUN_ID` to a unique suffix for independent live runs. To inspect normalized
actual transcripts without changing fixtures, set `AIT_CONFORMANCE_ACTUAL_DIR` to an output
directory. Mismatches still fail and never update goldens automatically.

Forward-compat fixtures live in `fixtures/forward-compat/` and are consumed by SDK tolerance lanes.

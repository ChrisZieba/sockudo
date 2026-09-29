# Sockudo AI Transport Conformance

Raw protocol conformance harness for the AIT-S SDK executable spec.

This suite intentionally has no Sockudo SDK dependency. It uses Node `fetch` plus the built-in
WebSocket client to exercise the server wire surface directly.

## Run

Start an AI-enabled Sockudo server, then:

```bash
scripts/ai-conformance-node.sh
```

The script gives each run a fresh `AIT_CONFORMANCE_RUN_ID` so channels, history, and idempotency
state never collide with earlier runs against the same server. Private AI channel subscriptions are
signed with the app secret, as a server-side auth endpoint would.

Defaults:

- `SOCKUDO_BASE_URL=http://127.0.0.1:6001`
- `SOCKUDO_WS_URL=ws://127.0.0.1:6001/app/app-key?protocol=2&client=ait-conformance&version=0`
- `SOCKUDO_APP_ID=app-id`
- `SOCKUDO_APP_KEY=app-key`
- `SOCKUDO_APP_SECRET=app-secret`

Offline fixture validation:

```bash
AIT_CONFORMANCE_OFFLINE=1 node src/run.mjs
```

## Golden Transcripts

Golden files live in `fixtures/golden/`. Runtime serials, timestamps, stream and message IDs, and
socket IDs are normalized before comparison. The golden transcripts are the SDK-facing executable
spec for canonical AI Transport sequences, recorded with the V2 default `append_mode=delta`.

After an intentional wire change, regenerate them from a live server and review the diff:

```bash
AIT_CONFORMANCE_UPDATE_GOLDEN=1 scripts/ai-conformance-node.sh
```

Forward-compat fixtures live in `fixtures/forward-compat/` and are consumed by SDK tolerance lanes.

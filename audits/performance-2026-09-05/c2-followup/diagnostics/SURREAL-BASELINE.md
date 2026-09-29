# SurrealDB baseline timeout diagnosis

**Decoder diagnosis only:** 3.2.4 resolves this timeout but still has the
independent memory-engine concurrency defect described in `CONCURRENCY.md`.
The supported version-store minimum and final fixture pin are stable 3.3.0.
The 3.2.4 numbers below remain isolated single-writer diagnostic evidence.

The historical 512 × 256 append timeout is reproducible on a fresh SurrealDB
3.0.4 fixture. It is a protocol decoder limit, not an append calculation or
quiet-host issue. The unchanged saved baseline executable completes the same
workload after upgrading only the isolated database to 3.2.4.

## Evidence

- Saved baseline executable: `target/c2/sockudo-durable-baseline-write-diag`;
  its SHA-256 is recorded in each run manifest. Storage statements and harness
  are unchanged between the two database releases.
- `results/pr461-surreal-baseline-diagnostic-2`: the original 3.0.4 server
  committed delivery position 486, then stopped advancing with the next position
  at 487 through the 300-second timeout. Both database and client CPU became
  nearly idle; memory settled around 1.93 GiB of 7.75 GiB. Independent HTTP
  progress queries continued succeeding. The container was not OOM-killed.
- The last stored legacy byte-array payload had 249,521 integer elements.
  The next append adds 512 serialized bytes (the fragment appears in both
  message and envelope), producing 250,033 elements. The transaction binds the
  payload twice, as latest state and entry content.
- Each integer value consumes two FlatBuffers tables. The next request crosses
  the old verifier's 1,000,000-table ceiling. `flatbuffer_boundary.rs` reproduces
  this boundary with the actual SDK encoder: two arrays of 249,521 elements
  pass; two arrays of 250,033 elements fail with `TooManyTables`. The latter
  encoded request is approximately 14 MB, below the WebSocket message limit.
  Native byte values pass the same verifier at both sizes. The current decoder
  also accepts both array sizes.
- The official [3.0.4 decoder source](https://github.com/surrealdb/surrealdb/blob/v3.0.4/surrealdb/types/src/flatbuffers/mod.rs)
  uses the default verifier. The [3.2.4 decoder](https://github.com/surrealdb/surrealdb/blob/v3.2.4/surrealdb/types/src/flatbuffers/mod.rs)
  sizes the table budget to the transport-bounded input while keeping recursion
  bounded; its comment references [upstream issue 7037](https://github.com/surrealdb/surrealdb/issues/7037).
- `results/pr461-surreal-fixed-baseline`: the identical executable completes
  all 512 appends without a proxy on 3.2.4, exit 0.
- `results/pr461-surreal-fixed-baseline-wire`: it also completes with the
  original wire proxy, exit 0. Client bytes total 3,802,179,766, averaging
  7,426,132 bytes per append (integer output); maximum 14,751,846 bytes.

These are diagnostics and write-volume measurements, not quiet latency results.
Earlier timed-out cases remain excluded; they have not been converted to passes.
The decoder fix is present from 3.2.4, but the final benchmark server must be
pinned consistently to 3.3.0 for every source variant to include the independent
transaction-engine correction. Preserve the old 3.0.4 failure separately.
No historical application storage implementation was altered.

Decoder-diagnostic fixture image (not the final supported fixture):
`surrealdb/surrealdb:v3.2.4@sha256:51baed8709f57f67dcf04b30e3177db846803fa9342dae2be58c6fa5f8d59843`.

## Reproduction

`diagnose_surreal_baseline.py` samples only the isolated C2 fixture and numeric
progress/CPU/memory data, with a hard timeout. `--wire` enables the existing
harness proxy; `--chunked` enables the current format when testing the final
binary. Restart only the isolated SurrealDB fixture before each run to avoid
mixing memory retained by earlier cases. The original runs used 3.0.4; the active Compose fixture now uses 3.3.0. Use an
explicit image override only when reproducing historical failures.

## Same-server completed write-volume comparison

All three preserved binaries completed 512 × 256 on freshly restarted 3.2.4.
The preserved final binary also passed complete history, replay and restart
digest checks. These rows supersede no old measurements; they are a separate
comparison under the fixed server.

| Build | Client bytes per append | Maximum client bytes per append |
| --- | ---: | ---: |
| baseline | 7,426,132.36 | 14,751,846 |
| c2 | 3,752,545.55 | 7,415,438 |
| followup | 6,553.64 | 8,480 |

Machine-readable totals and exact executable hashes: `results/pr461-surreal-fixed-comparison.json`.
The final row uses the preserved PR binary before the additional concurrency
constraint in this follow-up; the new constraint is validated separately by
strict live correctness tests.

## Final supported-server comparison (3.3.0)

The unchanged baseline and C2 executables and the current PR executable all
completed the same 512 × 256 workload on freshly restarted 3.3.0 fixtures.
The current executable includes the server-version gate, checked transaction
errors, bounded retries and unique delivery index. Its full historical reads,
replay and restart digests also passed. Build profiles differ (historical
release versus current debug); only serialized wire volume is compared, with
no latency claim.

| Build | Client bytes per append | Maximum client bytes per append |
| --- | ---: | ---: |
| baseline | 7,426,132.36 | 14,751,846 |
| c2 | 3,752,545.55 | 7,415,438 |
| current | 6,553.64 | 8,480 |

Raw totals, exact executable hashes and the common pinned image are recorded
in `results/pr461-surreal-330-comparison.json`; corresponding run directories
retain workload logs, host/container samples and completion manifests.

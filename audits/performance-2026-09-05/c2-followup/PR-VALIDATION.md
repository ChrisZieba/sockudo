# Bounded append storage: PR validation

PR #461 was initially submitted with deferred quiet-host latency measurements.
The subsequent follow-up addresses the correctness limitations and adds an
isolated Linux measurement workflow. The new matrix must complete before a
within-10% latency claim can be made. Rejected/noisy runs remain excluded.

## Change

- Preserve every append operation, receipt, four serial identities and public
  full-state reads while storing accumulated data in versioned bounded chunks.
- SQL uses 1,024-byte chunks; DynamoDB, ScyllaDB and SurrealDB use 4,096-byte
  chunks. NoSQL prefix caches are bounded to 8 MiB and keyed by run generation.
- Persisted activation defaults off. Stop writers, upgrade every node, then
  explicitly enable compact writes. For rollback, stop writers, disable writes,
  and materialize before returning older readers. Maintenance commands are
  documented in the mutable-message guide.
- Preserve legacy full rows and C2 format 1. Format 2 accepts its earlier full
  field names while using compact names for new internal records.
- Correct HTTP object-data deserialization and stale AI conformance fixtures;
  validate raw acknowledgement/fanout identities without changing fanout wire
  behavior.

## Validation of initial PR commit 5063eb34

| Check | Result |
|---|---|
| Workspace tests | 1,630 passed, 0 failed, 1 ignored |
| Formatting, workspace and all-backend strict Clippy | Passed |
| Documentation type check and production build | Passed |
| Final live storage checks across five backends | 7 passed |
| SQL abort/rollout/purge regressions | 2 passed |
| Surreal stored-function transaction-abort regression | Passed |
| Actual master/C2/final mixed-release matrix | All five backends passed |
| Raw AI conformance with PostgreSQL durable storage enabled | 10/10 passed |
| Unmodified Ably smoke, mutation and recovery suites | Passed |
| Baseline and final AI performance budget guards | Both passed unchanged budgets |
| Per-append socket bounds | All 45 backend/size/fragment cases passed |
| Version, replay and restart digests | All 45 match the saved master oracle |

Validation predates PR submission; these are local results, not a claim about
GitHub CI. Saved executables and raw logs remain in the local audit workspace;
large archives, binaries, application/process snapshots and raw logs are not
included in this PR. The committed runners and harnesses support reproduction.

## Accepted socket bounds

The user accepted protocol-adjusted limits replacing the initial approximate
1 KiB overhead allowance. Each measured append is asserted independently.
Here `f` is fragment bytes and `b64(n) = 4 × ceil(n / 3)`.

| Backend | Maximum client socket payload bytes |
|---|---|
| PostgreSQL/MySQL | f + 1,024 + 3,072; first two appends allow f + 1,024 + 6,144 |
| DynamoDB | b64(f) + b64(4,096) + 10,240 |
| ScyllaDB | f + 4,096 + 8,192 |
| SurrealDB | f + 4,096 + 5,120 |

These bounds cover fixed benchmark metadata, requests and driver framing;
TCP/IP headers are excluded. They are not universal bounds for arbitrary
application metadata or receipt-key sizes.

## Measured storage effects

At 2,000 × 256 bytes, PostgreSQL relation size is 6,930,432 bytes before the
existing diagnostic VACUUM FULL and 5,726,208 afterward: **1.2103×**, below the
2× target without requiring a manual vacuum to meet it.

| Bytes written per append at 2,000 × 256 | Master | C2 | Final |
|---|---:|---:|---:|
| PostgreSQL WAL | 374,815.85 | 194,331.74 | 4,217.61 |
| MySQL redo | 567,952.90 | 292,924.42 | 7,913.98 |

The historical comparison completed 62 diagnostic cases. Original master
SurrealDB 512 × 256 timed out with and without the socket proxy; neither timeout
counts as a pass or a measurement. Final code passes that case and 2,000 × 256.
WAL/redo are global counter deltas and may include database background activity.
DynamoDB item/WCU and Scylla bound-value counters are application estimates,
not physical disk or replication measurements. Volume diagnostics are not
quiet latency evidence; local emulators are not managed-service benchmarks.

## Follow-up changes and verification

- DynamoDB version history, replay and latest enumeration follow service pages,
  including all-expired pages. Large appends use bounded, fenced staging and an
  atomic publication transaction instead of rejecting more than 94 chunks.
  Live regressions cover takeover, old-writer races, marker fencing, item-size
  rejection, pagination and rollback. The final boundary case exercises 99
  changed chunks in two staging transactions.
- DynamoDB validates the complete legacy item size before rollback rewrites.
  `--check-append-storage-rollback` does not rewrite records or the marker;
  normal store initialization may create missing schema. `--rollback-append-storage`
  checks feasibility before disabling compact writes and materializing.
- SurrealDB enforces unique app/channel delivery positions, rejects corrupt
  upgrades and retries only definite transaction conflicts with fresh state.
  Its immutable v2 function preserves the v1 ABI for existing nodes. Startup
  requires stable SurrealDB 3.3.0 or newer because older in-memory engines can
  violate transaction isolation even with unique indexes (upstream #7473). Tests
  verify all receipts and positions through concurrent writes and restart.
- The original SurrealDB 3.0.4 timeout is a FlatBuffers verifier limit. The
  identical baseline binary completes on 3.2.4. See
  [the diagnosis](diagnostics/SURREAL-BASELINE.md); original failed attempts
  remain failed. All three remote builds use the same pinned 3.3.0 fixture; 3.2.4 fixed the
  decoder but still exhibited the independent in-memory concurrency defect.
- A separate build/measurement workflow runs three repetitions on a fresh Linux
  VM with the unchanged load gate and additional CPU-steal rejection. Portable
  runner support has 27 passing tests. Remote results remain pending until the
  workflow produces its complete validation artifact.

Current workspace tests pass (1,631 passed, zero failures, one ignored) with
`REDIS_URL` pointing at the isolated fixture. Workspace strict Clippy, all-backend
compilation, four focused DynamoDB tests, four shared live storage tests and the
four-backend rollout/materialization test pass. All 36 refreshed PostgreSQL,
MySQL, DynamoDB and ScyllaDB wire cases pass, with full-state digests matching
the saved baseline memory oracle. The initial strict SurrealDB 3.3.0 suite
passes; additional repetitions are in progress. The 3.2.4 startup rejection
leaves database tables unchanged. Documentation types pass; production build
passes using `npm run build -- --webpack` because the sandbox blocks Turbopack's
helper port. Final repetitions, SurrealDB volume and CI results are recorded
after they finish; the earlier table is not evidence for these new changes.

## Measurement status and physical constraints

Only one accepted memory repetition exists for master/C2/final at 128 × 16 and
2,000 × 256. The repeated durable latency matrix is incomplete because host
load remained above the requested limit of 2. The user needs the background
applications and declined closing them. The small-history 10% target therefore
remains unverified. SQL has 7/8/10 sequential awaited database operations per
ordinary append in master/C2/final; final adds marker fencing and chunk writes.
SQL reads remain one query, with additional chunk assembly. These source facts
are not substitutes for measured latency or attribution of a particular slowdown.

DynamoDB's service-level 400 KiB item limit still applies to original operations,
receipts and materialized legacy rows. Oversized rollback is rejected without
truncation or receipt deletion; retain a chunk-capable release in that case.
Stopped-writer activation and rollback remain required, especially for older
writers that do not observe format fences. These constraints cannot safely be
removed merely by changing the maintenance command. Application write counters
cover publication transactions and exclude staging/lease/maintenance writes;
the socket proxy counts all requests. Existing 16/64/256-byte fragment volume
cases do not use multi-batch staging. Emulator evidence is not a claim about
managed-service performance.

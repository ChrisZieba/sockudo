# Bounded append storage: PR validation

The user requested submission to `master` on 2026-09-29 after accepting the
completed checks and explicitly deferring the remaining repeated quiet-host
latency matrix. No within-10% latency claim is made. Rejected/noisy runs remain
excluded; the deferral does not turn an incomplete benchmark into a pass.

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

## Completed validation

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

## Deferred measurement and constraints

Only one accepted memory repetition exists for master/C2/final at 128 × 16 and
2,000 × 256. The repeated durable latency matrix is incomplete because host
load remained above the requested limit of 2. The user needs the background
applications and declined closing them. The small-history 10% target therefore
remains unverified. SQL has 7/8/10 sequential awaited database operations per
ordinary append in master/C2/final; final adds marker fencing and chunk writes.
SQL reads remain one query, with additional chunk assembly. These source facts
are not substitutes for measured latency or attribution of a particular slowdown.

DynamoDB limits a transaction to 94 changed chunks, and rollback must respect
the legacy 400 KiB item limit. Existing SurrealDB concurrent lost updates and
DynamoDB history-page truncation at 1 MiB remain explicitly out of scope.

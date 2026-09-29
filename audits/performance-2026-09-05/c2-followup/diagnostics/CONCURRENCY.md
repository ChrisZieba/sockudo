# SurrealDB concurrent-write correction

Durable version storage now requires a **stable SurrealDB server >= 3.3.0**.
The SDK parses the server version; initialization checks it before any schema
writes. Prereleases and affected older releases fail with a configuration
error. The remote API does not reliably identify its storage engine, so the
requirement applies to all remote version-store deployments, including older
RocksDB servers which might otherwise have sound isolation. Plain history is
not subject to this new version-store gate.

## Why an application CAS alone was insufficient

The historical memory engine could commit two writes from one predecessor.
Adding a unique delivery index exposed the deeper problem: a repeated strict
3.2.4 run still committed version 43 and version 44 of the same message at
channel delivery position 30. Database inspection confirmed the UNIQUE
`app_id, channel, delivery_serial` index existed. Subsequent reconstruction
correctly rejected the corrupt aggregate. The raw failing test is retained at
`results/pr461-surreal-concurrency/unsupported-324-fork.log`; these failures
are not converted to passing measurements.

This matches upstream [issue 7473](https://github.com/surrealdb/surrealdb/issues/7473):
the old memory engine could miss a concurrent commit during its conflict scan.
Neither an application condition nor another uniqueness constraint repairs
that isolation failure. The official
[3.3.0 lockfile](https://github.com/surrealdb/surrealdb/blob/v3.3.0/Cargo.lock)
resolves `surrealmx 0.27.0`, after the corrected commit-watermark implementation.
The [stable release](https://github.com/surrealdb/surrealdb/releases/tag/v3.3.0)
is used for the supported fixture.

No process-local mutex or new distributed lock was substituted for database
transaction guarantees. The new delivery uniqueness index remains defense in
depth and rejects preexisting corrupt data during initialization. Its name
contains `version_conflict`, preserving older clients' error classification
when the database reports a collision. However, an old application binary
cannot acquire this new server-version safety gate; operators must upgrade
all writers and follow the existing rollout procedure.

## Transaction retries and error handling

The complete read/plan/transaction operation retries up to eight times after a
definite conflict. Every attempt retains the caller's predecessor precondition,
version identity, and operation receipt identity. A changed predecessor returns
a conflict instead of silently rebasing the requested mutation.

SurrealDB can return a `NotExecuted` placeholder before the actual transaction
failure. The adapter now inspects all statement errors, preserving the real
cause rather than blindly retrying generic errors. Ambiguous network failures
and timeouts are not classified as safe retries. Stored mutation function v2
has a new immutable name; v1 is unchanged.

## Validation on the supported engine

Pinned fixture:
`surrealdb/surrealdb:v3.3.0@sha256:681c6c22c287421b5c7d99e0fde79b6e0d32c36c1ddeaab2762a1661cb04cd20`.

- Strict independent-connection receipt/concurrency test passed 21 repetitions:
  legacy and chunked formats, same and different messages, 320 mutations per
  repetition. Every acknowledged fragment, version identity, delivery position,
  history identity, persisted receipt, full replay and restarted read is checked.
- Existing race probe passed 50 trials, 40 mutations per trial.
- Existing concurrent-append test passed 40 mutations with its former
  corruption escape hatch removed.
- Total concurrent mutations across these supported-server runs: **8,760**.
- Forced stored-function abort preserves stream position, entry, and latest
  state, including UTF-8 identities; this test passed.
- A deliberate duplicate delivery position is rejected. A synthetic corrupt
  legacy store fails initialization twice, proving a failed index build does
  not leave a definition that bypasses validation on the next start.
- Version boundary and narrow retry-classifier unit tests passed.
- A live 3.2.4 initialization was rejected by the safety gate; database table
  names were identical before and after the attempt.

Writer barriers are bounded to 10 seconds and task joins to 30 seconds; the
external repeated-run harness also limits each test to 60 seconds. A failure
cannot leave this regression indefinitely waiting for its other writer.
Logs are under `results/pr461-surreal-concurrency/`.

The separate `SURREAL-BASELINE.md` explains the old FlatBuffers timeout. Version
3.2.4 fixed that decoder problem but did **not** fix the independent memory
engine defect. Single-writer volume measurements on that intermediate release
remain diagnostic evidence only. The final benchmark matrix uses 3.3.0 for all
three application variants.

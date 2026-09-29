# Dedicated Linux quiet-latency run

This workflow supplies the missing three-repetition comparison without closing
or changing any personal application. It creates no paid cloud resources. The
normal GitHub-hosted `ubuntu-24.04` jobs use the repository's existing Actions
entitlement. A hosted VM is dedicated to the job; the underlying physical host
is shared, so every sample still has to pass the quiet gate. Linux CPU steal
also invalidates a sample. A successful workflow is evidence completeness,
not automatic approval of any observed performance regression.

## Source and build provenance

`.github/workflows/c2-quiet-latency.yml` runs manually or on changes to this
benchmark support on `perf/append-storage-bounded-writes`. It needs only
`contents: read`, no repository token, cloud credential, or personal machine.
A branch push can execute the workflow before it exists on the default branch.
Only the coordinating agent/user should publish or trigger it.

Three independent build jobs use Rust **1.98.1**, identical Ubuntu image labels,
locked Cargo dependencies and explicit release profiles. The official release
manifest checksum endpoint was verified before pinning. No compiled-target
cache is restored. Measurement starts in a fourth, fresh VM only after all
three builds finish; all three builds are measured on that same VM.

- **Baseline:** commit `3b57039d4565a01a1bdae66fd891872e7e49debd`, plus the
  existing `c2/prerequisite-read-fixes.patch`. These are the original read
  compatibility fixes used by the earlier baseline, not the new C2 storage.
- **C2:** the same baseline plus `source-snapshots/c2-final-from-baseline.patch`.
  Before adding benchmark overlays, `git write-tree` must equal recovered tree
  `574ca6a1895453ad97ec555abc1f7a424f4418b8`. The patch SHA-256 is pinned to
  `588119b5b3dd415b0a2a381c5ddda20632054f676491dc4a33727d77b6ac87bf`.
  `source-snapshots/c2-final-manifest.json` records original snapshot provenance.
- **Follow-up:** the exact workflow commit, never a moving branch ref.

All three receive the same current deterministic memory fixture and durable
append/read fixture. The historical harness rejects `C2_CHUNKED`; only the
follow-up enables its persisted format marker. Mixed-release and concurrency
tests are omitted from this benchmark executable. Write-volume hooks are not
invoked. The Linux memory diagnostic converts `ru_maxrss` from KiB to bytes;
macOS output and the measured operation loops remain unchanged.

Memory builds retain LTO and one codegen unit. Durable test builds retain the
previous diagnostic settings: no LTO, 16 codegen units, build-override stripping
disabled. Each source gets its own target directory. Build manifests capture
source trees, patches, harness hashes, Cargo.lock, compiler, feature commands,
profiles and final binary hashes. The measurement job checks matching compiler,
lockfile, profiles and memory fixtures, and verifies downloaded binary hashes.

## Measurement and acceptance

Only one database fixture runs at a time. `compose.pinned.yaml` pins the original
audited multi-architecture image digests for PostgreSQL, MySQL, DynamoDB and
ScyllaDB. SurrealDB uses **3.3.0** for every variant. The original fixture at
commit `5063eb34` used 3.0.4 and hit its FlatBuffers table-verifier limit. The
identical saved baseline binary completed 512 × 256 on 3.2.4, but separate
concurrent-writer tests reproduced that release's memory-engine lost-update
defect (upstream #7473). Version 3.3.0 fixes that engine defect as well as the
decoder limit; it is also the follow-up store's minimum supported server.
Both the active local fixture and remote fixture now use the same pinned 3.3.0
image. Old timeout and decoder evidence remains unchanged. The benchmark
artifacts identify the exact image used for each new comparison.
Protocol readiness is checked before sampling. Each container stays running
across all three variants and repetitions; none is restarted during a case. Every repetition runs baseline → C2
→ follow-up. Cases remain **128 × 16** and **2000 × 256**, with the historical
DynamoDB/SurrealDB 2000 × 256 exclusions preserved. This requires **90 accepted
processes**: 18 memory and 72 durable. The smaller case covers all five durable
backends; the larger covers PostgreSQL, MySQL and ScyllaDB.

The existing start gate is unchanged: one-minute load must be below 2, no
process may report at least 50% CPU, and process inspection must succeed. After
the workload, only explicitly captured fixture-container host PIDs may be
heavy. Linux uses `ps --sort=-pcpu` instead of macOS `ps -r`; both inspect all
processes. GNU/Linux timing uses `getrusage` with peak RSS converted to bytes,
retaining the existing `.time` keys. On Linux, any increase or reset of the
CPU steal counter between before/after snapshots rejects the sample. Rejected
attempts are retained and never promoted to canonical results. Up to 60 quiet
attempts are allowed per process, without relaxing a threshold.

Wire proxies, WCU/value counters, volume-only shortcuts and progress diagnostics
are disabled during latency sampling. The append subprocess has a 900-second
Linux timeout; a failed or timed-out case fails the job. Database CPU counters
bracket each actual process, excluding quiet waits. Existing digest checks
verify the complete version history, replay and fresh-store reconstruction.
The final validator also requires three accepted runs for every build/case,
matching digests across all builds/backends, and every original latency field.

Results are a new **Linux comparison**. Do not pool their timing distributions
with existing Apple Silicon/macOS samples. `latency-validation.json` includes
per-repetition values, medians, ranges, and final-versus-baseline/C2 changes.
`summary.csv`/`summary.md` preserve the existing summary format. Review any
small-history increase above 10%; completeness alone does not waive that gate.

## Retained artifacts and failure recovery

`c2-build-<phase>-<commit>` contains executables, build logs and provenance.
`c2-quiet-latency-<commit>` contains accepted CSV/time/CPU files, all rejected
attempts and host snapshots, container configuration/logs, machine metadata,
and the final validation report. Uploads run even after a failure. Results are
retained for 30 days by the workflow; download accepted evidence into the audit
before that retention period ends. A missing final validation report means the
matrix did not complete. A failed workflow must not be described as quiet
latency passing.

Support checks (no database or compiler work):

```sh
python3 -m unittest discover -s audits/performance-2026-09-05/c2 -p 'test_*.py'
python3 -m unittest discover -s audits/performance-2026-09-05/c2-followup/remote -p 'test_*.py'
```

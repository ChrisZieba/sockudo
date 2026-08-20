# Sockudo 5.0.0 Release Checklist

This checklist prepares one reviewed commit for the Sockudo 5.0.0 server release and every SDK
release built from it. Do not create any release tag until that exact commit is green in CI.

## Version Matrix

| Package | Version | Release tag |
| --- | --- | --- |
| Sockudo server and publishable workspace crates | 5.0.0 | `v5.0.0` |
| `@sockudo/client` | 2.2.0 | `client-js-v2.2.0` |
| `@sockudo/ai-transport` | 3.0.0 | `client-ai-transport-js-v3.0.0` |
| `Sockudo.Client` | 2.2.0 | `client-dotnet-v2.2.0` |
| `sockudo_flutter` | 2.2.0 | `client-flutter-v2.2.0` |
| `io.sockudo:sockudo-kotlin` | 2.2.0 | `client-kotlin-v2.2.0` |
| `sockudo-python` | 2.2.0 | `client-python-v2.2.0` |
| `SockudoSwift` | 3.0.0 | `client-swift-v3.0.0` |
| Node server SDK `sockudo` | 2.2.0 | `server-node-v2.2.0` |
| `sockudo-http-python` | 2.2.0 | `server-python-v2.2.0` |
| `sockudo/sockudo-php-server` | 2.2.0 | `server-php-v2.2.0` |
| Ruby server SDK `sockudo` | 2.2.0 | `server-ruby-v2.2.0` |
| Go server SDK v2 | 2.2.0 | `server-sdks/sockudo-http-go/v2.2.0` |
| `sockudo-http` Rust server SDK | 2.2.0 | `server-rust-v2.2.0` |
| `io.sockudo:sockudo-http-java` | 2.2.0 | `server-java-v2.2.0` |
| `SockudoServer` | 2.2.0 | `server-dotnet-v2.2.0` |
| Swift server SDK `Sockudo` | 2.2.0 | `server-swift-v2.2.0` |

AI Transport 3.0.0 and the Swift client 3.0.0 carry real breaking changes documented in their
package changelogs. The other SDK changes are additive and remain on the 2.x line. The Go module
stays on `/v2`; moving it to 3.0.0 would require a breaking `/v3` import path.

## Required Gates

- [ ] `cargo fmt --all -- --check`
- [ ] focused tests for protocol, core, adapter, Ably compatibility, AI Transport, push, and server
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] package and dry-run every publishable Rust crate, including `sockudo-ably-compat`
- [ ] run the complete SDK CI matrix
- [ ] run `SDK Release` with `package=all` and `dry_run=true`
- [ ] run the pinned Ably Node, Chromium, Go, strict-completeness, and AI Transport lanes
- [ ] satisfy the independent compatibility performance/load budgets and complete the required
  topology runs
- [ ] review the 5.0.0 and package changelogs, migration notes, install commands, and version matrix

The retained compatibility scorecard currently records the independent load/soak budget as red.
Release preparation may continue, but 5.0.0 must not be promoted until fresh evidence satisfies
that gate or the release policy is explicitly changed and documented.

Before the dependency crates exist on crates.io, local preflight can fully package only the leaf
crates. Verify `cargo package --list` for all publishable crates locally, then let the release
workflow package and publish the full set in dependency order.

## Tag And Publish Order

1. Merge the reviewed release commit and record its full commit SHA.
2. Create `v5.0.0` from that SHA. Let the server workflow publish workspace crates, binaries, and
   container images. Confirm `sockudo-ably-compat` is published before `sockudo`.
3. Run released-binary compatibility verification against `v5.0.0` and retain the reports.
4. Publish `@sockudo/client` 2.2.0 before `@sockudo/ai-transport` 3.0.0.
5. Create the remaining client tags from the same SHA.
6. Create the nine server SDK tags from the same SHA. The PHP and Swift mirror workflows must
   finish before checking Packagist or SwiftPM availability.
7. Verify registry metadata, provenance, install snippets, mirrored tags, and package contents for
   every entry in the matrix.

Create annotated tags so the release intent and source commit remain clear:

```bash
git tag -a v5.0.0 -m "Sockudo 5.0.0"
git tag -a client-js-v2.2.0 -m "@sockudo/client 2.2.0"
git tag -a client-ai-transport-js-v3.0.0 -m "@sockudo/ai-transport 3.0.0"
```

Use the exact remaining tag names from the version matrix. Push tags individually after the
preceding release has completed; do not publish the whole train with an unreviewed wildcard push.

## Post-release Verification

- [ ] `cargo install sockudo --version 5.0.0 --locked`
- [ ] pull and smoke-test both `ghcr.io/sockudo/sockudo:5.0.0` and `sockudo/sockudo:5.0.0`
- [ ] verify Linux GNU/musl archives and detached SHA-256 files for x86_64 and ARM64
- [ ] install each SDK from its public registry or SwiftPM mirror at the matrix version
- [ ] confirm the PHP mirror exposes `v2.2.0` and both Swift mirrors expose their expected tags
- [ ] mark the published-artifact and released-binary items green in the compatibility scorecard
- [ ] announce breaking AI Transport and Swift actor-isolation migrations beside the release notes

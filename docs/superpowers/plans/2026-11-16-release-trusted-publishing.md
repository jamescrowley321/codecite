# Release with crates.io Trusted Publishing, an SBOM and build provenance

Issue: #110.

**Goal:** a tag-triggered release workflow that publishes the five crates to
crates.io with Trusted Publishing (no registry token in the repository),
fails fast on a tag that does not match the crates or a stale `Cargo.lock`,
and ships prebuilt binaries with an SBOM and a build provenance attestation.

**Architecture:** one workflow, `.github/workflows/release.yml`, on `v*` tags,
in five jobs, each with only the permissions it needs:

1. `verify`: the tag is `v<semver>`, points at a commit on `main`, equals the
   version of every crate whose `publish` is not `false`, and
   `cargo metadata --locked` (with dependencies, since `--no-deps` skips
   resolution and would miss a stale lock) succeeds.
2. `build`: per target (x86_64 and aarch64 Linux, aarch64 macOS, each on a
   native runner), `cargo auditable build --release --locked -p codecite`,
   a `.tar.gz` with the binary, `README.md`, `LICENSE` and `NOTICE`, and an
   SPDX SBOM from syft's Rust binary cataloger. An SBOM listing no crates
   fails the job.
3. `package`: `cargo package --workspace --locked`, with no `id-token`, so
   crate build scripts never run beside a registry token.
4. `publish`: in the `release` environment with `id-token: write`; skips
   crates already at the version on crates.io (sparse index), then
   `rust-lang/crates-io-auth-action` trades the OIDC token for a crates.io
   token scoped to the job, then `cargo publish --workspace --locked
   --no-verify` with an `--exclude` per skipped crate. On a fork it runs the
   same with `--dry-run` instead.
5. `release`: in the `release` environment; `SHA256SUMS`, `actions/attest-build-provenance` over the
   archives and SBOMs, and `gh release create --verify-tag` (a draft on a
   fork, a prerelease for a `-` version).

No caches in the workflow, so nothing a PR run writes can reach a released
binary. Actions pinned by commit SHA; `persist-credentials: false` on every
checkout; every `${{ }}` reaches a script through `env`.

## Steps

- [ ] Write `release.yml` as above.
- [ ] Check the verify script locally: `v0.0.1` passes; `v0.0.2`, `v01.0` and
  `0.0.1` fail with their reason; a bumped `workspace.package.version` without
  `cargo update` makes `cargo metadata --locked` fail.
- [ ] `cargo publish --workspace --locked --dry-run` passes locally (the fork
  path).
- [ ] `cargo auditable build` and `syft scan file:target/release/codecite`
  locally produce an SBOM that lists the linked crates.
- [ ] `zizmor` (1.31.1, regular persona, min severity low, online) and
  `actionlint` report nothing.
- [ ] `docs/design.md`: the release process (Architecture) and supply chain
  (Security and privacy). `docs/usage.md`: verifying a release binary.

## Left to the owner

- Register the trusted publisher on crates.io for each crate (repository,
  `release.yml`, environment `release`). crates.io configures a trusted
  publisher on a crate that exists, so the first publish of each crate may
  need a short-lived, scoped token by hand, revoked afterwards.
- oss-admin `terraform/codecite`: the `release` environment, limited to `v*`
  tags, and a tag ruleset so only the owner creates, moves or deletes `v*`
  tags.
- The fork dry run: tag a fork, confirm crates.io is untouched, and
  `gh attestation verify` an archive against the fork.

# Releasing CodeLore

This document captures the versioning policy + the mechanical release procedure. The release pipeline (multi-platform `cargo build` matrix + SLSA L3 build provenance + distroless container + Homebrew formula regeneration) is a hand-rolled GitHub Actions workflow at `.github/workflows/release.yml` and triggers on git tag push.

## Versioning Policy

CodeLore follows **Semantic Versioning 2.0.0** (SemVer) — see https://semver.org/.

### Version format

`MAJOR.MINOR.PATCH[-PRERELEASE][+BUILD]`

The current workspace version lives at `[workspace.package].version` in the root `Cargo.toml` and is inherited by every crate (`version.workspace = true`). All three crates always share the same version — they ship together.

### Pre-1.0 (the current state)

`0.MINOR.PATCH[-alpha.N | -beta.N | -rc.N]`

While the project is pre-1.0:

- The MINOR version acts as the breaking-change axis (`0.1.x` → `0.2.0` for breaking changes).
- PATCH covers non-breaking changes (bug fixes + non-breaking features).
- Pre-release suffixes follow the alpha → beta → rc → release ladder:
  - `0.1.0-alpha.N` — internal milestones, unstable surface, expect daily breakage.
  - `0.1.0-beta.N` — feature-complete for the release scope, public preview, schema + CLI surface stabilizing.
  - `0.1.0-rc.N` — release candidate, no planned changes besides bug fixes. CI green, docs current.
  - `0.1.0` — first stable. After this, SemVer rules apply. **Today: this is what ships.**

### Post-1.0

After `1.0.0`:

- **MAJOR** — any backwards-incompatible change to:
  - CLI flag names or values
  - Default values that change output meaningfully
  - Output schema (CSV columns, JSON field shape, SARIF rule IDs, provenance manifest layout)
  - Cache file format (the `cache.rs` `CACHE_EPOCH` constant)
  - Public Rust API (codelore-lib's exported items)
- **MINOR** — new flags, new analyses, new output formats, new SARIF rules. No removals.
- **PATCH** — bug fixes, perf improvements, internal refactors. No new flags or analyses.

### Version-axis examples

| Change | Bump |
|---|---|
| Fix `clone_coupling` p_value=0 bug | PATCH (`0.1.0` → `0.1.1`) |
| Add new analysis `soc` | MINOR (`0.1.1` → `0.2.0` pre-1.0; `1.0.0` → `1.1.0` post-1.0) |
| Rename `--max-coupling` to `--max-coupling-pct` | MAJOR (`0.2.0` → `0.3.0` pre-1.0; `1.0.0` → `2.0.0` post-1.0) |
| Drop the always-empty `name` column from hotspots CSV | MAJOR (CSV schema change) |
| Add `--code-maat-compat` flag | MINOR (additive flag, no behavior change without setting it) |
| Add new SARIF rule | MINOR |
| Change Manifest JSON schema layout | MAJOR |
| Add hot-path index in DuckDB schema | PATCH (transparent to users) |

### `0.1.0` ⇒ how we got here, and the path from here

The workspace ships `0.1.0` as the first stable tag. Getting here meant collapsing the planned `alpha → beta → rc → stable` ladder: the alpha phase ran with a small set of real users (primarily one), the three-sprint bugfix + modernization + code-maat-parity work landed under heavy use, and the externally-visible surface stabilized through that real-world exercise rather than through a separate beta/rc gate. We promoted directly from `0.1.0-alpha.1` to `0.1.0` once: PAR-1 through PAR-10 (every code-maat-parity analysis) had shipped; CI was green on Linux, macOS, and Windows; all 21 analyses had test coverage; the `<owner>` placeholders had been resolved to a real repo URL; and the SemVer policy in this document was in place.

From `0.1.0` the path forward is:

1. **`0.1.x` patch releases** — bug fixes + transparent perf wins (no flag changes, no schema changes).
2. **`0.2.0` / `0.3.0` etc.** — any breaking change to the CLI flags, output schema, cache format, or public Rust API.
3. **`1.0.0`** — the strong-stability commitment. We pull this trigger when we feel confident the surface won't need a breaking change for the foreseeable future. Probably 6-12 months of real-world `0.x` usage.

For future major-version cuts (`1.0`, `2.0`, …), revive the `alpha → beta → rc → stable` ladder — the rigor matters more once external users depend on the previous major.

## MSRV (Minimum Supported Rust Version) Policy

**Pre-1.0 stance: MSRV tracks the toolchain channel verbatim. No N-2 buffer.**

The workspace's `Cargo.toml` `rust-version` field and `rust-toolchain.toml`'s `channel` are pinned to the same stable release (currently `1.96` / `1.96.0`). A Rust toolchain bump and an MSRV bump are the same operation — every pin site moves together in the release-cut commit. There are six of them: `rust-toolchain.toml`'s `channel`, the workspace `rust-version` in `Cargo.toml`, `clippy.toml`'s `msrv`, `Containerfile`'s `ARG RUST_VERSION` (and the `rust:<ver>-bookworm` build-stage comment above it), the `dtolnay/rust-toolchain` action invocations, and the `CHANGELOG.md` entry. `scripts/cut-release.sh` performs the package-version bump and CHANGELOG flip; the toolchain/MSRV pins are edited by hand as part of the same commit.

### Why no buffer

The conventional N-2-stable MSRV buffer (Rust convention for library crates published to `crates.io`) trades currency for compatibility. CodeLore is a CLI binary, not a library consumed by other crates' build graphs — there is no downstream `cargo build` chain we destabilize by requiring fresh stable. The binary ships as prebuilt artifacts (five targets per release) plus a `cargo binstall`-compatible asset layout plus a distroless container. End users do not compile from source as a primary install path.

The cost we accept: a contributor building from a Rust ≤ 1.95 toolchain gets a build error pointing at the `rust-version` mismatch. The benefit: we get to use every stable Rust feature the moment it ships (the SQL-driven analyses surface benefits from `let-else` / `let chains` / `if let` chaining as they land), and the MSRV ratchet has no surprises — every MSRV bump is a deliberate `scripts/cut-release.sh` invocation, never a silently-broken downstream.

### Post-1.0 reconsideration

Once we hit `1.0`, the binary will likely grow a stable library surface (`codelore-lib` consumed by IDE extensions, lints crates, custom dashboards). At that point this section gets revisited and the conventional N-2 buffer applies. The trigger is "external Rust consumers depend on `codelore-lib`'s API", not the version number itself.

### How to bump MSRV

Move every one of the six pin sites listed above together in the release-cut commit — never bump one in isolation (a lone `rust-toolchain.toml` bump is silently overridden by `rust-version`, and a lone `rust-version` bump breaks the build). `scripts/cut-release.sh` handles the package-version bump and CHANGELOG flip; the toolchain/MSRV pins (`rust-toolchain.toml`, workspace `rust-version`, `clippy.toml`, `Containerfile`, the `dtolnay/rust-toolchain` action invocations) are updated by hand alongside it in the same commit.

## Release Procedure

### Pre-flight checklist

Before bumping the version, every item must be true:

- [ ] `cargo test --workspace --features test-support,spa` is green (CI's non-browser test scope; `--all-features` silently skips browser-tests without Chrome)
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` is green
- [ ] `cargo fmt --all --check` is green
- [ ] `cargo deny check` is green
- [ ] `CHANGELOG.md` has an entry for the version being cut, with all user-facing changes listed
- [ ] `README.md` status line reflects the release scope
- [ ] `docs/advanced-usage.md` matches the shipping CLI surface

### Cut a release

**Recommended: use the `scripts/cut-release.sh` helper.** It codifies the full procedure (pre-flight checks, version bump, CHANGELOG flip, lockfile sync, sanity build, CI gate, the `disable-ruleset → tag → restore` dance with `trap EXIT` cleanup) into one idempotent script:

```bash
./scripts/cut-release.sh X.Y.Z              # full cut
./scripts/cut-release.sh X.Y.Z --dry-run    # preview — refuses to take any state-changing action
./scripts/cut-release.sh X.Y.Z --skip-ci-wait   # re-attempt after CI already green
```

The script refuses to proceed unless all of: working tree clean, on `main`, in sync with `origin/main`, target tag doesn't exist yet, `X.Y.Z` parses as digit-only semver, `CHANGELOG.md [Unreleased]` has content, `gh` CLI authenticated. The `trap EXIT` registered ruleset-restore runs on ANY exit path (success, error, ^C) so the `protect-release-tags` ruleset is never left disabled.

### Manual procedure (if the script isn't available — emergency fallback only)

```bash
# 1. Bump the workspace version
#    Edit Cargo.toml: [workspace.package].version = "X.Y.Z"
$EDITOR Cargo.toml

# 2. Refresh the lockfile (each-crate versions inherit automatically)
cargo update --workspace

# 3. Run the full gate one more time
cargo test --workspace --features test-support,spa
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all --check

# 4. Commit the version bump
git add Cargo.toml Cargo.lock CHANGELOG.md
git commit -m "release: vX.Y.Z"

# 5. Push commit + wait for CI green on it (required by the ruleset)
git push origin main
gh run watch $(gh run list --limit 1 --branch main --workflow CI --json databaseId --jq '.[0].databaseId') --exit-status

# 6. Tag the release (annotated; release-pipeline triggers on any `v*` tag)
git tag -a vX.Y.Z -m "vX.Y.Z"

# 7. Push tag — see "Tag push ruleset dance" below for why this can fail
git push origin vX.Y.Z
```

### Tag push ruleset dance

The `protect-release-tags` ruleset requires green status checks on the target commit before a tag-creation push is accepted. GitHub's Check Runs API (what GitHub Actions writes to) and the older Commit Status API don't always cross-talk reliably even with the `integration_id: 15368` (GitHub Actions app) hint set on the ruleset's `required_status_checks` parameters. So tag pushes sometimes fail with `remote: 6 of 6 required status checks are expected.` even when all 6 checks are visibly green on the target commit.

**Fix when this happens:** temporarily disable the ruleset, push the tag, restore the ruleset. `scripts/cut-release.sh` does this automatically via `trap EXIT`. Manual:

```bash
gh api -X PUT repos/emrecdr/codelore/rulesets/17437461 --input - <<'JSON'
{ "name": "protect-release-tags", "target": "tag", "enforcement": "disabled",
  "conditions": { "ref_name": { "include": ["refs/tags/v*"], "exclude": [] } },
  "rules": [ { "type": "deletion" }, { "type": "non_fast_forward" } ] }
JSON

git push origin vX.Y.Z

# Restore — see scripts/cut-release.sh's restore_ruleset() function for the
# canonical body to re-PUT. Always run this before exiting the shell.
```

### What the tag push triggers

The `v*` tag-creation ref is also gated by the `protect-release-tags` repo ruleset, which requires all six CI contexts (`rustfmt`, `clippy`, `cargo-deny`, `test (ubuntu/macos/windows-latest)`) green on the target commit before the tag is accepted. If you tag a red commit, the push is rejected — `git tag` succeeds locally, `git push origin vX.Y.Z` fails with a rules-violation message.

Once accepted, three workflows fire in parallel on `vX.Y.Z`:

1. **`.github/workflows/release.yml`** (the release pipeline):
   - `plan` — resolves the tag string
   - `build` — matrix of 5 targets (aarch64-darwin, x86_64-darwin, x86_64-linux-gnu, aarch64-linux-gnu, x86_64-windows-msvc), each running `cargo build --release --locked --target $TARGET` and packaging the artifact into a versioned tarball/zip. This job holds **no** signing permissions: it runs `build.rs`, so under SLSA Build L3 the sigstore token must be unreachable from it
   - `attest` — matrix over the same 5 targets, calling the reusable trusted signer at `.github/workflows/attest-artifact.yml`. Each instance downloads one artifact, hashes it, and signs via `actions/attest`. This is the only job permitted to hold `id-token`/`attestations`, and `release` depends on it so a failed attestation blocks publication
   - `release` — downloads all 5 build artifacts and publishes the GitHub Release via `softprops/action-gh-release@v3`, with `generate_release_notes: true` pulling the CHANGELOG section automatically
   - `homebrew-publish` — downloads the same build artifacts (bit-identical to what end-users `brew install`), computes SHA256 of each, renders `Formula/codelore.rb`, checks out `emrecdr/homebrew-codelore` via the `HOMEBREW_TAP_DEPLOY_KEY` SSH deploy key, and pushes the regenerated formula if it changed
   - `crates-publish` — publishes `codelore-rca` → `codelore-lib` → `codelore` to crates.io in that dependency order; skipped (the publish step, not the job) unless the `CRATES_IO_TOKEN` repository secret is configured, so forks and unconfigured checkouts still get a green release

2. **`.github/workflows/container.yml`** publishes a distroless container image to `ghcr.io/emrecdr/codelore:vX.Y.Z` (and `:latest` for non-pre-release tags).

3. **`.github/workflows/bench.yml`** (weekly) is unaffected — it tracks main, not tags.

`cargo binstall codelore` works automatically once the GitHub Release exists — `cargo-binstall` scans the Release asset list for the expected `codelore-<tag>-<target>.tar.gz` / `.zip` pattern.

### Pre-release vs stable

Use SemVer suffix conventions on the tag (`v0.2.0-alpha.1`, `v0.2.0-beta.2`, `v0.2.0-rc.1`). `softprops/action-gh-release@v3` does NOT automatically mark these as pre-release — if you want the GitHub Release flagged as "Pre-release", either add `prerelease: true` to the `release` step temporarily for that tag, or edit the Release on the GitHub UI after publish. The Homebrew tap formula is unconditionally overwritten on every tag push, so a pre-release tag will move the tap to the pre-release version — if you want the tap to stay on the last stable, either skip the homebrew-publish job for pre-release tags (add an `if: !contains(needs.plan.outputs.tag, '-')` guard) or roll back the formula manually.

### Publishing to crates.io

The `crates-publish` job (see the tag-push job list above for the order and the secret gate) runs only after `plan`, `build`, and `release` have all succeeded, leaning on `cargo publish`'s own wait for each crate to propagate through the index before the next one starts. GitHub Actions doesn't expose the `secrets` context to *any* `if` expression (job- or step-level), so the job maps `CRATES_IO_TOKEN` through job-level `env` and the publish step's guard tests `env.CRATES_IO_TOKEN` instead.

If the job fails partway through, finish the remaining crates by hand, in the same order:

```bash
cargo publish -p codelore-rca   # hard error if this version is already published — expected, skip to the next line
cargo publish -p codelore-lib
cargo publish -p codelore
```

`cargo publish` hard-errors (does not skip) on a version that's already live, and the step's `bash -e` shell exits at that first failing command — so re-triggering the `crates-publish` job after a partial failure never reaches the later crates. Finishing by hand with the commands above, skipping whichever crate(s) already succeeded, is the only recovery.

### Yanking a release

If a release ships with a critical bug:

```bash
# Yanks the crates.io upload — existing installs keep working, new
# installs of that exact version refuse.
cargo yank --version X.Y.Z

# Cut a fix release immediately (bump PATCH).
# Do NOT delete the git tag — yanking on crates.io is the right surface.
```

## Changelog Discipline

Every release entry in `CHANGELOG.md` follows the [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) format:

```markdown
## [X.Y.Z] - YYYY-MM-DD

### Added
- New analyses, flags, output formats

### Changed
- Behavior changes (these are MAJOR bumps post-1.0)

### Fixed
- Bug fixes

### Removed
- Removed features (MAJOR bumps)

### Deprecated
- Features still working but flagged for future removal

### Security
- Vulnerabilities patched
```

The PR template asks "what CHANGELOG entry does this PR justify?" — every user-facing change MUST land in the changelog before merge. Internal refactors, test-only changes, doc-only changes can skip the changelog.

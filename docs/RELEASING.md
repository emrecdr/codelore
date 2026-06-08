# Releasing CodeLore

This document captures the versioning policy + the mechanical release procedure. The release pipeline (`cargo-dist` multi-platform binaries + SLSA L3 provenance + distroless container) is configured under `[workspace.metadata.dist]` in the root `Cargo.toml` and triggers on git tag push.

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
  - `0.1.0-alpha.N` — internal milestones, unstable surface, expect daily breakage. Today: `0.1.0-alpha.1`.
  - `0.1.0-beta.N` — feature-complete for the release scope, public preview, schema + CLI surface stabilizing.
  - `0.1.0-rc.N` — release candidate, no planned changes besides bug fixes. CI green, docs current.
  - `0.1.0` — first stable. After this, SemVer rules apply.

### Post-1.0

After `1.0.0`:

- **MAJOR** — any backwards-incompatible change to:
  - CLI flag names or values
  - Default values that change output meaningfully
  - Output schema (CSV columns, JSON field shape, SARIF rule IDs, provenance manifest layout)
  - Cache file format (the `cache.rs` `SCHEMA_VERSION` constant)
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

### `0.1.0-alpha.1` ⇒ how we got here, and the next steps

Today the workspace ships `0.1.0-alpha.1`. The 16-commit bugfix + modernization + parity sprint has put us at "feature-complete for v0.1 scope, surface still mid-stabilization". The honest pre-release ladder from here:

1. **Bump to `0.1.0-alpha.2`** for any internal milestone.
2. **Bump to `0.1.0-beta.1`** once we ship the parity plan end-to-end (PAR-1 through PAR-10) AND the breaking-change docs in advanced-usage are settled.
3. **Bump to `0.1.0-rc.1`** when CI green, docs current, performance benches stable.
4. **`0.1.0`** = first published stable release. Tag, push, release pipeline runs.

The skip from `0.1.0` to `1.0.0` is when we feel confident enough to make the strong stability promise on the CLI surface + output schemas. Probably 6-12 months of real-world usage past `0.1.0`.

## Release Procedure

### Pre-flight checklist

Before bumping the version, every item must be true:

- [ ] `cargo test --workspace --all-features` is green
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` is green
- [ ] `cargo fmt --all --check` is green
- [ ] `cargo deny check` is green
- [ ] `CHANGELOG.md` has an entry for the version being cut, with all user-facing changes listed
- [ ] `README.md` status line reflects the release scope
- [ ] `docs/advanced-usage.md` matches the shipping CLI surface

### Cut a release

```bash
# 1. Bump the workspace version
#    Edit Cargo.toml: [workspace.package].version = "X.Y.Z"
$EDITOR Cargo.toml

# 2. Refresh the lockfile (each-crate versions inherit automatically)
cargo update --workspace

# 3. Run the full gate one more time
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all --check

# 4. Commit the version bump
git add Cargo.toml Cargo.lock CHANGELOG.md
git commit -m "release: vX.Y.Z"

# 5. Tag the release (signed; matches v1.0.0 release-pipeline trigger)
git tag -s vX.Y.Z -m "vX.Y.Z"

# 6. Push commit + tag together
git push --follow-tags
```

### What the tag push triggers

When a `vX.Y.Z` tag lands on `main`:

1. **`.github/workflows/release.yml`** runs the `cargo-dist` pipeline:
   - Builds release binaries for 6 platforms (aarch64-darwin, x86_64-darwin, x86_64-linux-gnu, x86_64-linux-musl, aarch64-linux-gnu, x86_64-windows-msvc)
   - Signs with SLSA L3 provenance
   - Uploads as a GitHub Release with the changelog excerpt for that version
   - Updates the `<owner>/homebrew-codelore` Homebrew tap
   - Generates a `cargo binstall` manifest so `cargo binstall codelore` works from day one of the release

2. **`.github/workflows/container.yml`** publishes a distroless container image to `ghcr.io/<owner>/codelore:vX.Y.Z` (and `:latest` for non-pre-release tags).

3. **`.github/workflows/bench.yml`** (weekly) is unaffected — it tracks main, not tags.

### Pre-release vs stable

`cargo-dist` recognizes pre-release tags by the SemVer suffix (`-alpha`, `-beta`, `-rc`):

- Tags like `v0.1.0-alpha.2` produce a GitHub Release marked "Pre-release".
- The Homebrew tap publishes pre-release versions under a separate stable channel.
- `cargo binstall codelore` defaults to the latest **stable** tag — users must pass `--version 0.1.0-alpha.2` explicitly for pre-releases.

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

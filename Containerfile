# syntax=docker/dockerfile:1.7
# Distroless container image for CodeLore.
#
# Build stage: rust:1.96-bookworm (matches workspace MSRV).
#   We use cargo-chef to cache the dependency build layer so source-only
#   changes don't trigger a full DuckDB rebuild (~5 min cold).
#
# Runtime stage: gcr.io/distroless/cc-debian12:nonroot
#   - cc variant: provides libgcc + libstdc++ that DuckDB's bundled build needs
#   - nonroot: UID 65532, no shell, no package manager
#   - Target image size: ~25-30 MB compressed (most of it is the bundled
#     DuckDB binary blob)
#
# Usage:
#   docker build -t codelore .
#   docker run --rm -v /path/to/repo:/repo:ro codelore analyze --repo /repo --analysis hotspots

ARG RUST_VERSION=1.96
ARG DEBIAN_RELEASE=bookworm
# Base-image digests are pinned IN-LINE on the FROM instructions
# below (not via ARG) so Dependabot's docker ecosystem can auto-PR
# digest bumps — Dependabot reads the literal `image:tag@sha256:...`
# shape and won't resolve ARG substitutions. Tag refs without a
# digest are MUTABLE on both Docker Hub and gcr.io (both registries
# rebuild floating tags on a regular schedule), so an unpinned build
# today is NOT guaranteed to be byte-identical to the same
# Containerfile built yesterday. The digests below tie each base to
# a single immutable manifest so reproducibility, provenance
# attestation (cosign / SLSA), and CVE diffing work as intended.
#
# Updating: bump via the Dependabot docker stanza in
# `.github/dependabot.yml`, or capture a new digest manually with
# `docker buildx imagetools inspect <ref> | grep Digest` and edit
# both FROM lines. The bump is intentional, not automatic.
# REPO is the GitHub `owner/repo` slug. Used only by the
# `org.opencontainers.image.source` OCI label so `docker inspect`
# / `cosign verify` / Snyk / Grype / Trivy etc. dereference back
# to the canonical repository. The default `emrecdr/codelore`
# matches what container.yml ships from CI on tag push. Forks
# should override with `--build-arg REPO=<owner>/<fork>`.
ARG REPO=emrecdr/codelore

#─── chef stage ─────────────────────────────────────────────────────────────
FROM rust:${RUST_VERSION}-${DEBIAN_RELEASE}@sha256:19817ead3289c8c631c73df281e18b59b172f6a31f4f563290f69cddd06c30e9 AS chef
RUN cargo install cargo-chef --locked
WORKDIR /src

#─── planner stage: capture deps ────────────────────────────────────────────
# Workspace Cargo.toml carries a `[patch.crates-io]` entry pointing at
# `vendor/duckdb-rs/crates/libduckdb-sys/` (workaround for upstream
# `duckdb-rs#786` — MSVC 19.40 build break). Cargo errors when the patch
# path is missing, so vendor/ must be present BEFORE `cargo chef prepare`
# evaluates the manifest. Source-of-truth for that directory is the
# `scripts/vendor-duckdb-rs.sh` script — `container.yml` runs it on the
# host before `docker/build-push-action`, and the resulting tree lands in
# the build context via `COPY . .` below. Carried into the builder stage
# explicitly so `cargo chef cook` can resolve the same patch entry without
# the full source tree.
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

#─── builder stage ──────────────────────────────────────────────────────────
FROM chef AS builder
COPY --from=planner /src/recipe.json recipe.json
COPY --from=planner /src/vendor vendor
# `--features spa` activates the opt-in dashboard emitter
# (`--format spa`). Container images ship with this enabled so
# users running `docker run codelore … --format spa` get the
# dashboard out of the box.
RUN cargo chef cook --release --features spa --recipe-path recipe.json
COPY . .
RUN cargo build --release --features spa -p codelore-cli && \
    strip target/release/codelore

#─── runtime stage ──────────────────────────────────────────────────────────
FROM gcr.io/distroless/cc-debian12:nonroot@sha256:b0ae8e989418b458e0f25489bc3be523718938a2b70864cc0f6a00af1ddbd985 AS runtime

# Re-declare the build arg inside this stage so `${REPO}` expands in
# the label below — ARG values declared in earlier stages don't
# automatically carry into later stages in a multi-stage build.
ARG REPO=emrecdr/codelore

LABEL org.opencontainers.image.title="CodeLore"
LABEL org.opencontainers.image.description="Behavioral code analyzer — read the lore of your codebase"
LABEL org.opencontainers.image.licenses="GPL-3.0-only"
LABEL org.opencontainers.image.source="https://github.com/${REPO}"

COPY --from=builder /src/target/release/codelore /usr/local/bin/codelore
USER nonroot
WORKDIR /repo
ENTRYPOINT ["/usr/local/bin/codelore"]
CMD ["--help"]

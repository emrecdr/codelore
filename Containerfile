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

#─── chef stage ─────────────────────────────────────────────────────────────
FROM rust:${RUST_VERSION}-${DEBIAN_RELEASE} AS chef
RUN cargo install cargo-chef --locked
WORKDIR /src

#─── planner stage: capture deps ────────────────────────────────────────────
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

#─── builder stage ──────────────────────────────────────────────────────────
FROM chef AS builder
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release -p codelore-cli && \
    strip target/release/codelore

#─── runtime stage ──────────────────────────────────────────────────────────
FROM gcr.io/distroless/cc-debian12:nonroot AS runtime

LABEL org.opencontainers.image.title="CodeLore"
LABEL org.opencontainers.image.description="Behavioral code analyzer — read the lore of your codebase"
LABEL org.opencontainers.image.licenses="GPL-3.0-only"
LABEL org.opencontainers.image.source="https://github.com/<owner>/codelore"

COPY --from=builder /src/target/release/codelore /usr/local/bin/codelore
USER nonroot
WORKDIR /repo
ENTRYPOINT ["/usr/local/bin/codelore"]
CMD ["--help"]

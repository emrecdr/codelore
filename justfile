# CodeLore task runner
# Install just: cargo install just

set positional-arguments

default:
    @just --list

# Build everything
build:
    cargo build --workspace

# Build release
release:
    cargo build --workspace --release

# Run all tests (matches CI's non-browser test scope). `--all-features` would
# pull in `browser-tests`, which SILENTLY SKIPS without Chrome — giving false
# "just ci == CI" confidence. Browser tests are isolated in `test-browser`,
# mirroring CI's separate spa-browser job.
test:
    cargo test --workspace --features test-support,spa

# Run the SPA headless-browser smoke test (mirrors CI's spa-browser job).
# Requires a Chrome/Chromium binary on PATH; skips gracefully without one.
test-browser:
    cargo test -p codelore-lib --features browser-tests,spa,test-support --test spa_browser_test

# Run clippy with our hard standards
lint:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

# Format check
fmt-check:
    cargo fmt --all --check

# Format
fmt:
    cargo fmt --all

# License + advisory check
deny:
    cargo deny check

# Coverage report — requires: cargo install cargo-llvm-cov
coverage:
    cargo llvm-cov --workspace --html

# All CI checks
ci: fmt-check lint deny test

# Run the binary
codelore *ARGS:
    cargo run --release -p codelore -- "$@"

# Recompile the SPA's Tailwind v4 + DaisyUI 5 CSS asset.
# Requires the Tailwind v4 standalone CLI on $PATH (see
# crates/codelore-lib/src/output/spa/tailwind-src/README.md for the
# install workflow). The compiled output is checked in so build.rs can
# inline it the same way it inlines ECharts/d3.
spa-css-rebuild:
    tailwindcss \
        -i crates/codelore-lib/src/output/spa/tailwind-src/input.css \
        -o crates/codelore-lib/src/output/spa/tailwind.daisyui.min.css \
        --minify

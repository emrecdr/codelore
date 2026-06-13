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

# Run all tests
test:
    cargo test --workspace --all-features

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
    cargo run --release -p codelore-cli -- "$@"

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

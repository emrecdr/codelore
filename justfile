# bca task runner
# Install just: cargo install just

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

# Coverage report
coverage:
    cargo llvm-cov --workspace --html

# All CI checks
ci: fmt-check lint deny test

# Run the binary
bca *ARGS:
    cargo run --release -p bca-cli -- {{ARGS}}

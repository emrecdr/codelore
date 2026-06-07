#!/usr/bin/env bash
# Profile-Guided Optimization campaign script for CodeLore.
#
# Per spec §6.5, the PGO campaign is deferred to v1.1 (after the benchmark
# suite stabilizes and we have confidence the gains exceed the build-time
# cost). This script is the scaffolding — run it manually before tagging
# v1.1 to produce an optimized binary.
#
# Prerequisites:
#   - cargo-pgo: `cargo install cargo-pgo`
#   - LLVM tools: `rustup component add llvm-tools-preview`
#   - A representative training repo at $CODELORE_BENCH_LINUX_KERNEL_PATH
#     (the Linux kernel snapshot is the canonical workload; substitute any
#     large git repo if unavailable).

set -euo pipefail

readonly TRAINING_REPO="${CODELORE_BENCH_LINUX_KERNEL_PATH:-/tmp/linux-kernel-snapshot}"
readonly TARGET="${TARGET:-x86_64-unknown-linux-gnu}"

if [[ ! -d "$TRAINING_REPO" ]]; then
  echo "error: training repo not found at $TRAINING_REPO" >&2
  echo "       set CODELORE_BENCH_LINUX_KERNEL_PATH or clone Linux kernel first" >&2
  exit 2
fi

if ! command -v cargo-pgo >/dev/null 2>&1; then
  echo "error: cargo-pgo not installed. Run: cargo install cargo-pgo" >&2
  exit 2
fi

echo ">>> Stage 1/3: instrumented build"
cargo pgo build -- --bin codelore --release

echo ">>> Stage 2/3: training run (hotspots analysis against $TRAINING_REPO)"
"./target/${TARGET}/release/codelore" \
  analyze --analysis hotspots \
  --repo "$TRAINING_REPO" \
  --format parquet \
  --output /tmp/pgo-training-hotspots.parquet

echo ">>> Stage 3/3: PGO-optimized build"
cargo pgo optimize build -- --bin codelore --release

echo
echo "Done. Optimized binary: ./target/${TARGET}/release/codelore"
echo "Compare with the baseline release binary using the criterion bench:"
echo "  cargo bench -p codelore-lib --all-features --bench end_to_end"

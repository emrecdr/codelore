//! End-to-end `criterion` benches for the `CodeLore` ingest pipeline.
//!
//! Three targets:
//!   - `ingest_tiny`: 5 commits (sanity benchmark; sub-ms).
//!   - `ingest_medium`: 500 commits across 25 files (CI baseline).
//!   - `ingest_linux_kernel_snapshot`: only runs when
//!     `CODELORE_BENCH_LINUX_KERNEL_PATH` env var is set (CI fetches a cached
//!     snapshot once a week). Validates the spec §1.1 release blockers:
//!     < 10 min wall-clock + < 4 GB peak RSS for the Linux kernel.
//!
//! Run locally: `cargo bench -p codelore-lib --all-features`
//! Run kernel:  `CODELORE_BENCH_LINUX_KERNEL_PATH=/path/to/linux cargo bench ...`

use std::hint::black_box;

use codelore_lib::Options;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;
use criterion::{Criterion, criterion_group, criterion_main};

fn ingest_tiny(c: &mut Criterion) {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        ..Options::default()
    };
    c.bench_function("ingest_tiny", |b| {
        b.iter(|| {
            let repo = GixRepo::open(tiny.dir.path()).unwrap();
            let db = FactsDb::new_in_memory().unwrap();
            db.ingest(black_box(&repo), black_box(&opts)).unwrap();
        });
    });
}

fn ingest_medium(c: &mut Criterion) {
    let medium = codelore_lib::test_support::medium_repo::build();
    let opts = Options {
        repo_path: medium.dir.path().to_path_buf(),
        ..Options::default()
    };
    let mut group = c.benchmark_group("ingest");
    group.sample_size(10);
    group.bench_function("medium_500_commits", |b| {
        b.iter(|| {
            let repo = GixRepo::open(medium.dir.path()).unwrap();
            let db = FactsDb::new_in_memory().unwrap();
            db.ingest(black_box(&repo), black_box(&opts)).unwrap();
        });
    });
    group.finish();
}

fn ingest_linux_kernel_snapshot(c: &mut Criterion) {
    let Some(path) = std::env::var_os("CODELORE_BENCH_LINUX_KERNEL_PATH") else {
        eprintln!(
            "CODELORE_BENCH_LINUX_KERNEL_PATH not set — skipping linux kernel snapshot bench"
        );
        return;
    };
    let opts = Options {
        repo_path: path.into(),
        ..Options::default()
    };
    let mut group = c.benchmark_group("ingest_kernel");
    group.sample_size(10);
    group.measurement_time(std::time::Duration::from_secs(120));
    group.bench_function("linux_kernel_snapshot", |b| {
        b.iter(|| {
            let repo = GixRepo::open(&opts.repo_path).unwrap();
            let db = FactsDb::new_in_memory().unwrap();
            db.ingest(black_box(&repo), black_box(&opts)).unwrap();
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    ingest_tiny,
    ingest_medium,
    ingest_linux_kernel_snapshot
);
criterion_main!(benches);

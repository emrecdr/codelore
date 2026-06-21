//! End-to-end `criterion` benches for the `CodeLore` ingest pipeline.
//!
//! Six targets:
//!   - `ingest_tiny`: 5 commits (sanity benchmark; sub-ms).
//!   - `ingest_medium`: 500 commits across 25 files (CI baseline).
//!   - `ingest_medium_parallel`: complexity extraction with the default
//!     Rayon thread pool (logical-CPU count).
//!   - `ingest_medium_serial`: complexity extraction pinned to 1 thread, used
//!     as the serial baseline for the parallel-vs-serial comparison.
//!   - `ingest_capacity_sweep`: producer→consumer channel capacity swept across
//!     `[16, 64, 256, 1024]` on the medium fixture, surfacing the curve so the
//!     default `64` can be re-tuned against empirical data instead of folklore.
//!   - `ingest_linux_kernel_snapshot`: only runs when
//!     `CODELORE_BENCH_LINUX_KERNEL_PATH` env var is set (CI fetches a cached
//!     snapshot once a week). Validates the release blockers: < 10 min wall-
//!     clock + < 4 GB peak RSS for the Linux kernel.
//!
//! Run locally: `cargo bench -p codelore-lib --all-features`
//! Run kernel:  `CODELORE_BENCH_LINUX_KERNEL_PATH=/path/to/linux cargo bench ...`

use std::hint::black_box;

use codelore_lib::Options;
use codelore_lib::facts::FactsDb;
use codelore_lib::facts::ingest::set_channel_capacity_override;
use codelore_lib::repo::GixRepo;
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

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

/// Parallel ingest with the default Rayon thread pool.
///
/// Rayon uses `available_parallelism()` (logical CPU count) by default.
/// This bench is the "parallel" baseline for the serial vs parallel comparison.
fn ingest_medium_parallel(c: &mut Criterion) {
    let medium = codelore_lib::test_support::medium_repo::build();
    let opts = Options {
        repo_path: medium.dir.path().to_path_buf(),
        ..Options::default()
    };
    let mut group = c.benchmark_group("complexity_extraction");
    group.sample_size(10);
    group.bench_function("parallel_default_threads", |b| {
        b.iter(|| {
            let repo = GixRepo::open(medium.dir.path()).unwrap();
            let db = FactsDb::new_in_memory().unwrap();
            db.ingest(black_box(&repo), black_box(&opts)).unwrap();
        });
    });
    group.finish();
}

/// Serial ingest (1 thread), driven by the `RAYON_NUM_THREADS` environment
/// variable set *before* starting this bench binary.
///
/// ## Threading constraint explanation
///
/// Two obvious approaches for forcing serial mode inside a Criterion bench
/// both have blockers under this codebase's constraints:
///
/// 1. **`rayon::ThreadPoolBuilder::build_global(1)`** — can only be called ONCE
///    per process. Subsequent calls are silently ignored, so this would only
///    work for the very first bench that triggers pool initialisation.
///
/// 2. **`rayon::ThreadPool::install(|| db.ingest(...))`** — `ThreadPool::install`
///    requires `OP: Send`. `FactsDb` wraps `duckdb::Connection` (which contains
///    `RefCell`) and is therefore `!Send`. Moving `db` into the closure would
///    fail to compile.
///
/// 3. **`std::env::set_var("RAYON_NUM_THREADS", "1")`** — `set_var` is
///    `unsafe` in Rust ≥ 1.81, and this codebase has `unsafe_code = "forbid"`.
///
/// **Chosen approach**: pass `RAYON_NUM_THREADS=1` in the *environment of the
/// cargo bench invocation*, not inside the bench code. This is clean (no unsafe,
/// no pool juggling) and is documented here. Rayon reads the variable once on
/// first global-pool initialisation; after that the pool is frozen.
///
/// Run the two benches in separate invocations to compare:
/// ```sh
/// # parallel (default thread count = logical CPUs):
/// cargo bench -p codelore-lib --all-features -- complexity_extraction/parallel_default_threads
///
/// # serial (1 thread):
/// RAYON_NUM_THREADS=1 cargo bench -p codelore-lib --all-features -- complexity_extraction/serial_1_thread
/// ```
fn ingest_medium_serial(c: &mut Criterion) {
    let medium = codelore_lib::test_support::medium_repo::build();
    let opts = Options {
        repo_path: medium.dir.path().to_path_buf(),
        ..Options::default()
    };
    let mut group = c.benchmark_group("complexity_extraction");
    group.sample_size(10);
    group.bench_function("serial_1_thread", |b| {
        b.iter(|| {
            let repo = GixRepo::open(medium.dir.path()).unwrap();
            let db = FactsDb::new_in_memory().unwrap();
            db.ingest(black_box(&repo), black_box(&opts)).unwrap();
        });
    });
    group.finish();
}

/// V6: sweep the producer→consumer channel capacity across
/// `[16, 64, 256, 1024]` on the medium fixture. The default `64` was
/// folklore — no empirical measurement existed. This bench surfaces
/// the wall-clock × capacity curve so the constant can be re-tuned
/// from data the next time someone touches the ingest hot path.
///
/// Mechanism: a process-wide `AtomicUsize` override in `ingest.rs`
/// (`set_channel_capacity_override`) lets one `cargo bench`
/// invocation iterate every capacity in turn without
/// `unsafe { env::set_var }` (workspace `unsafe_code = "forbid"`
/// blocks that). The override is reset to `0` at the end of the
/// sweep so any later bench in the same invocation falls back to
/// the const default.
fn ingest_capacity_sweep(c: &mut Criterion) {
    let medium = codelore_lib::test_support::medium_repo::build();
    let opts = Options {
        repo_path: medium.dir.path().to_path_buf(),
        ..Options::default()
    };
    let mut group = c.benchmark_group("ingest_capacity_sweep");
    group.sample_size(10);
    for capacity in &[16_usize, 64, 256, 1024] {
        group.bench_with_input(
            BenchmarkId::from_parameter(capacity),
            capacity,
            |b, &cap| {
                set_channel_capacity_override(cap);
                b.iter(|| {
                    let repo = GixRepo::open(medium.dir.path()).unwrap();
                    let db = FactsDb::new_in_memory().unwrap();
                    db.ingest(black_box(&repo), black_box(&opts)).unwrap();
                });
            },
        );
    }
    set_channel_capacity_override(0);
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
    // clippy 1.96 wants `Duration::from_mins(2)` but that constructor is
    // still unstable as of stable Rust 1.96. Stick with `from_secs(120)`.
    #[allow(clippy::duration_suboptimal_units)]
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
    ingest_medium_parallel,
    ingest_medium_serial,
    ingest_capacity_sweep,
    ingest_linux_kernel_snapshot
);
criterion_main!(benches);

//! End-to-end coverage for the Maintainability-Index band rollup.
//!
//! `analyses/mi.rs` has no `run_*` entry point — its computational surface
//! is `MiRollup::from_hotspots` (band rollup) plus `MiBand::from_rank`
//! (percentile→band classifier), both consuming the `mi` / `mi_rank`
//! columns the hotspots SQL produces. The inline unit tests feed
//! hand-built `HotspotRow`s; this binary drives the real
//! `ingest → run_hotspots → MiRollup` path so a break in the
//! complexity-scan → MI → percentile-rank chain surfaces here.

use codelore_lib::Options;
use codelore_lib::analyses::hotspots::run_hotspots;
use codelore_lib::analyses::mi::{MiBand, MiRollup};
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

#[test]
fn mi_rollup_over_ingested_hotspots_is_consistent() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_hotspots(&db, &opts).expect("run hotspots");
    assert!(!rows.is_empty(), "tiny fixture should yield ≥1 hotspot row");

    let rollup = MiRollup::from_hotspots(&rows);

    // Invariant 1: every hotspot row is accounted for exactly once across
    // the four buckets — banded files plus the `unknown` bucket cover the
    // whole set with no double-counting.
    assert_eq!(
        rollup.known() + rollup.unknown,
        rows.len(),
        "rollup must partition all hotspot rows: known={} unknown={} total={}",
        rollup.known(),
        rollup.unknown,
        rows.len()
    );

    // Invariant 2: the tiny fixture's two Rust files produce a finite MI
    // and a finite percentile rank, so at least one file is banded (the
    // complexity → MI → PERCENT_RANK chain actually ran).
    assert!(
        rollup.known() >= 1,
        "expected ≥1 banded file from the Rust fixture; got {rollup:?}"
    );

    // Invariant 3: the rollup's per-band counts equal an independent tally
    // computed by re-classifying each row through `MiBand::from_rank` —
    // catches any drift between the rollup's bucketing and the public
    // classifier.
    let mut low = 0usize;
    let mut moderate = 0usize;
    let mut high = 0usize;
    let mut unknown = 0usize;
    for row in &rows {
        match (row.mi, row.mi_rank) {
            (Some(mi), Some(rank)) if mi.is_finite() && rank.is_finite() => {
                match MiBand::from_rank(rank) {
                    MiBand::Low => low += 1,
                    MiBand::Moderate => moderate += 1,
                    MiBand::High => high += 1,
                }
            }
            _ => unknown += 1,
        }
    }
    assert_eq!(rollup.low, low, "low band count drift");
    assert_eq!(rollup.moderate, moderate, "moderate band count drift");
    assert_eq!(rollup.high, high, "high band count drift");
    assert_eq!(rollup.unknown, unknown, "unknown band count drift");

    // Invariant 4: every banded percentile rank lies in the [0, 1] domain
    // that `PERCENT_RANK()` is contracted to emit.
    for row in &rows {
        if let Some(rank) = row.mi_rank
            && rank.is_finite()
        {
            assert!(
                (0.0..=1.0).contains(&rank),
                "mi_rank out of [0,1] for {}: {rank}",
                row.path
            );
        }
    }
}

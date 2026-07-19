//! Integration tests for `change_context::build_change_context` over a real
//! ingested fixture.
//!
//! `coupling_repo` guarantees `src/alpha/svc.rs` ↔ `src/beta/svc.rs` co-change
//! in ≥5 commits, so a briefing for `src/alpha/svc.rs` must name `src/beta/svc.rs`
//! as a co-change partner. The fixture is single-author, so ownership is a
//! clean sole-owner line. The suite also pins the determinism contract
//! (byte-identical across two builds) and the 1–20 path-count guard.

use codelore_lib::change_context::{MAX_BRIEFING_PATHS, build_change_context};
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;
use codelore_lib::test_support::coupling_repo::{self, CouplingRepo};
use codelore_lib::{CodeLoreError, Options};

/// Ingest `coupling_repo` into an in-memory fact store. Returns the fixture
/// alongside the repo / db / opts so the caller keeps the tempdir (and its git
/// dir, which `merge_or_rebase_in_progress` probes) alive for the whole test.
fn ingested() -> (CouplingRepo, GixRepo, FactsDb, Options) {
    let fixture = coupling_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("GixRepo::open");
    let db = FactsDb::new_in_memory().expect("new_in_memory");
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");
    (fixture, repo, db, opts)
}

#[test]
fn briefing_names_the_cochange_partner_and_owner() {
    let (_fixture, repo, db, opts) = ingested();
    let paths = vec!["src/alpha/svc.rs".to_string()];
    let out = build_change_context(&db, &repo, &opts, &paths).expect("briefing");

    assert!(
        out.contains("src/alpha/svc.rs"),
        "briefing must name the requested path: {out}"
    );
    assert!(
        out.contains("co-change:"),
        "briefing must carry a co-change line: {out}"
    );
    assert!(
        out.contains("src/beta/svc.rs"),
        "the guaranteed co-change partner must appear: {out}"
    );
    assert!(
        out.contains("health "),
        "briefing must carry a health line: {out}"
    );
    assert!(
        out.contains("owner:"),
        "briefing must carry an owner line: {out}"
    );
    assert!(
        !out.contains("no history at HEAD"),
        "a file with real history is not the no-history block: {out}"
    );
}

#[test]
fn briefing_is_byte_identical_across_two_builds() {
    let (_fixture, repo, db, opts) = ingested();
    let paths = vec![
        "src/alpha/svc.rs".to_string(),
        "src/beta/svc.rs".to_string(),
    ];
    let first = build_change_context(&db, &repo, &opts, &paths).expect("first build");
    let second = build_change_context(&db, &repo, &opts, &paths).expect("second build");
    assert_eq!(first, second, "the rendered briefing must be deterministic");
}

#[test]
fn empty_path_list_errors_naming_the_limit() {
    let (_fixture, repo, db, opts) = ingested();
    let err = build_change_context(&db, &repo, &opts, &[]).expect_err("empty must error");
    assert!(matches!(err, CodeLoreError::InvalidOptions(_)), "{err:?}");
    assert!(
        err.to_string().contains(&MAX_BRIEFING_PATHS.to_string()),
        "error must name the limit: {err}"
    );
}

#[test]
fn oversized_path_list_errors_naming_the_limit() {
    let (_fixture, repo, db, opts) = ingested();
    let paths: Vec<String> = (0..=MAX_BRIEFING_PATHS)
        .map(|i| format!("src/f{i}.rs"))
        .collect();
    assert_eq!(paths.len(), MAX_BRIEFING_PATHS + 1);
    let err = build_change_context(&db, &repo, &opts, &paths).expect_err("oversized must error");
    assert!(matches!(err, CodeLoreError::InvalidOptions(_)), "{err:?}");
    assert!(
        err.to_string().contains(&MAX_BRIEFING_PATHS.to_string()),
        "error must name the limit: {err}"
    );
}

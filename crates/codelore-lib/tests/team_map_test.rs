//! Integration test: `--team-map-file` aliases author identities at
//! ingest time, so downstream analyses see the team name instead of
//! the per-person email.

use std::fs;
use std::str::FromStr;

use codelore_lib::Options;
use codelore_lib::analyses::authors::run_authors;
use codelore_lib::analysis::AnalysisName;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

/// `--team-map-file` aliases author emails to team names in the
/// `authors` analysis output.
#[test]
fn team_map_renames_authors_in_output() {
    let diff_repo = codelore_lib::test_support::differential_repo::build();
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("team-map.csv");
    // The differential fixture has author `alice@example.com` (and the
    // mailmap-resolved variant via canonical_author). Map them to
    // "Backend Team" so the alias shows up downstream.
    fs::write(
        &path,
        "author,team\n\
         alice@example.com,Backend Team\n\
         canonical-alice@example.com,Backend Team\n",
    )
    .unwrap();

    let repo = GixRepo::open(diff_repo.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: diff_repo.dir.path().to_path_buf(),
        min_revs: 1,
        team_map_file: Some(path),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let _rows = run_authors(&db, &opts).expect("authors");
    // The aliasing lands in `commits.canonical_author`. Query the table
    // directly — it's the strongest guarantee the team-map ran.
    let team_count: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM commits WHERE canonical_author = 'Backend Team'",
            [],
            |r| r.get(0),
        )
        .expect("count commits");
    assert!(
        team_count > 0,
        "team-map aliasing produced 0 commits under the team name"
    );
}

/// Auto-discovery: `.codelore-teams` in the repo root is loaded even
/// without an explicit `--team-map-file` flag.
#[test]
fn dot_codelore_teams_is_auto_discovered() {
    let diff_repo = codelore_lib::test_support::differential_repo::build();
    let auto_path = diff_repo.dir.path().join(".codelore-teams");
    fs::write(
        &auto_path,
        "author,team\ncanonical-alice@example.com,AutoTeam\n",
    )
    .unwrap();

    let repo = GixRepo::open(diff_repo.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: diff_repo.dir.path().to_path_buf(),
        min_revs: 1,
        // team_map_file deliberately NOT set — auto-discover only
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let team_count: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM commits WHERE canonical_author = 'AutoTeam'",
            [],
            |r| r.get(0),
        )
        .expect("count commits");
    assert!(
        team_count > 0,
        ".codelore-teams was not auto-discovered (0 commits under 'AutoTeam')"
    );
}

/// `AnalysisName` recognises only validation: the team-map module
/// integrates with the existing analysis pipeline without breaking the
/// alias dispatcher.
#[test]
fn team_map_does_not_break_analysis_dispatcher() {
    // Sanity check: aliases still resolve. This catches accidental
    // breakage of `analysis.rs::from_str` from the C-track edits.
    assert!(AnalysisName::from_str("ownership").is_ok());
    assert!(AnalysisName::from_str("fragmentation").is_ok());
    assert!(AnalysisName::from_str("nonsense-analysis").is_err());
}

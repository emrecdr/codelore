use codelore_lib::AnalysisName;
use codelore_lib::CodeLoreError;
use codelore_lib::types::{ChangeType, CommitEvent, FileChange, Hunk, SCHEMA_VERSION};
use time::macros::datetime;

#[test]
fn schema_version_is_six() {
    // Schema v6: drops `imports.rev`'s `REFERENCES commits(rev)` foreign
    // key so head-only ingest can populate `imports` without a walked
    // `commits` table. Cache key includes this sentinel so v5 caches are
    // naturally invalidated on next open.
    assert_eq!(SCHEMA_VERSION, 6);
}

#[test]
fn commit_event_construction() {
    let event = CommitEvent {
        rev: "abcdef1".into(),
        author_email: "a@b.com".into(),
        author_name: "A B".into(),
        committer_email: "a@b.com".into(),
        date: datetime!(2026-06-06 12:30:45 UTC),
        committer_date: datetime!(2026-06-06 12:30:45 UTC),
        message: "test".into(),
        parents: vec![],
        changes: vec![FileChange {
            path: "src/main.rs".into(),
            change_type: ChangeType::Modified,
            loc_added: 10,
            loc_deleted: 3,
            hunks: vec![Hunk {
                old_start: 1,
                old_lines: 3,
                new_start: 1,
                new_lines: 10,
            }],
        }],
        canonical_author: None,
        ai_attribution: None,
        kamei: None,
    };
    assert_eq!(event.rev, "abcdef1");
    assert_eq!(event.changes.len(), 1);
}

#[test]
fn bca_error_exit_codes_match_spec() {
    assert_eq!(CodeLoreError::InvalidOptions("x".into()).exit_code(), 2);
    assert_eq!(CodeLoreError::Repo("x".into()).exit_code(), 3);
    assert_eq!(CodeLoreError::Analysis("x".into()).exit_code(), 4);
    assert_eq!(CodeLoreError::Output("x".into()).exit_code(), 5);
    // Io variant (write-side, generic): exit 5 — broken pipe writing
    // CSV/JSON/markdown, etc.
    let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "x");
    assert_eq!(CodeLoreError::Io(io_err).exit_code(), 5);
    // RepoIo variant (read-side input): exit 3 — `--team-map FILE`
    // failed to open, `--arch-rules-file` unreadable, etc. The user
    // pointed CodeLore at unreadable input; same exit bucket as "the
    // repo path didn't exist". Distinct from write-side `Io` because
    // CI orchestrators dispatch differently: exit 3 = "fix the input
    // you pointed me at", exit 5 = "something went wrong on output".
    let repo_io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "team-map");
    assert_eq!(CodeLoreError::RepoIo(repo_io_err).exit_code(), 3);
}

#[test]
fn analysis_name_roundtrip() {
    for &name in AnalysisName::all() {
        let s = name.as_str();
        let parsed: AnalysisName = s.parse().unwrap();
        assert_eq!(parsed, name, "roundtrip for {s}");
    }
}

#[test]
fn analysis_name_rejects_unknown() {
    let r: Result<AnalysisName, _> = "not-a-real-analysis".parse();
    assert!(r.is_err());
}

/// Every variant of `AnalysisName` must (a) appear in `all()` so the
/// CLI's `--help` / supported-names list is complete, AND (b)
/// round-trip through `as_str` / `from_str` cleanly.
///
/// The compile-time guard inside `all()` (a `match name { ... }` over
/// every variant) catches "added a variant but forgot to register it"
/// at build time. This runtime test is the **belt** to that
/// compile-time **brace**: a future contributor who edits the guard
/// match (e.g. accidentally widening an arm via `Self::A | Self::B`
/// when they meant to add a new line) might still compile but ship
/// a regression in `all()`. The duplicate-detection and non-empty
/// assertions below catch that second-order failure.
#[test]
fn analysis_name_all_is_unique_and_non_empty() {
    let all = AnalysisName::all();
    let mut unique = all.to_vec();
    unique.sort_by_key(|n| n.as_str());
    unique.dedup();
    assert_eq!(
        unique.len(),
        all.len(),
        "AnalysisName::all() contains duplicates: {:?} -> deduped {:?}",
        all.iter().map(|n| n.as_str()).collect::<Vec<_>>(),
        unique.iter().map(|n| n.as_str()).collect::<Vec<_>>(),
    );
    assert!(!all.is_empty(), "AnalysisName::all() must not be empty");
}

#[test]
fn analysis_name_display_matches_as_str() {
    for &name in AnalysisName::all() {
        assert_eq!(format!("{name}"), name.as_str());
    }
}

#[test]
fn default_options_match_code_maat_thresholds() {
    use codelore_lib::Options;
    let opts = Options::default();
    assert_eq!(opts.min_revs, 5);
    assert_eq!(opts.min_shared_revs, 5);
    assert_eq!(opts.min_coupling_pct, 30);
    assert_eq!(opts.max_coupling_pct, 100);
    assert_eq!(opts.max_changeset_size, 30);
    #[allow(clippy::float_cmp)]
    {
        assert_eq!(opts.fisher_significance, 0.05);
    }
}

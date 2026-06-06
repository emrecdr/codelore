use bca_lib::AnalysisName;
use bca_lib::BcaError;
use bca_lib::types::{ChangeType, CommitEvent, FileChange, Hunk, SCHEMA_VERSION};
use time::macros::date;

#[test]
fn schema_version_is_one() {
    assert_eq!(SCHEMA_VERSION, 1);
}

#[test]
fn commit_event_construction() {
    let event = CommitEvent {
        rev: "abcdef1".into(),
        author_email: "a@b.com".into(),
        author_name: "A B".into(),
        committer_email: "a@b.com".into(),
        date: date!(2026 - 06 - 06),
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
        kamei: None,
    };
    assert_eq!(event.rev, "abcdef1");
    assert_eq!(event.changes.len(), 1);
}

#[test]
fn bca_error_exit_codes_match_spec() {
    assert_eq!(BcaError::Provenance("x".into()).exit_code(), 2);
    assert_eq!(BcaError::Repo("x".into()).exit_code(), 3);
    assert_eq!(BcaError::Analysis("x".into()).exit_code(), 4);
    assert_eq!(BcaError::Output("x".into()).exit_code(), 5);
    // Io variant: construct via a real io::Error
    let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "x");
    assert_eq!(BcaError::Io(io_err).exit_code(), 5);
}

#[test]
fn analysis_name_roundtrip() {
    for name in &[
        "hotspots",
        "coupling",
        "ownership",
        "code-age",
        "abs-churn",
        "author-churn",
        "entity-churn",
        "communication",
        "code-health",
        "summary",
        "revisions",
        "authors", // standalone code-maat parity
    ] {
        let parsed: AnalysisName = name.parse().unwrap();
        assert_eq!(parsed.as_str(), *name, "roundtrip for {name}");
    }
}

#[test]
fn analysis_name_rejects_unknown() {
    let r: Result<AnalysisName, _> = "not-a-real-analysis".parse();
    assert!(r.is_err());
}

#[test]
fn default_options_match_code_maat_thresholds() {
    use bca_lib::Options;
    let opts = Options::default();
    assert_eq!(opts.min_revs, 5);
    assert_eq!(opts.min_shared_revs, 5);
    assert_eq!(opts.min_coupling_pct, 30);
    assert_eq!(opts.max_coupling_pct, 100);
    assert_eq!(opts.max_changeset_size, 30);
    assert_eq!(opts.fisher_significance, 0.05);
}

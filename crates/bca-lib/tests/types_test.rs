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

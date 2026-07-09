//! Evidence chain for gate findings — the top-N commits that most recently
//! and heavily touched a path, used to populate `codeFlows` and
//! `relatedLocations` in SARIF output.

use crate::{CodeLoreError, Options, Result, facts::FactsDb};

/// One commit that touched the path being evidenced.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EvidenceCommit {
    /// Full commit SHA.
    pub rev: String,
    /// Commit author-date, ISO 8601 string from the `commits` table.
    pub date: String,
    /// Canonical author (mailmap-resolved).
    pub author: String,
    /// LOC added + LOC deleted for this path in this revision.
    pub churn: i64,
    /// First line of the commit message, capped at 80 characters.
    ///
    /// Truncation is performed in SQL via `substr(split_part(..., chr(10), 1), 1, 80)`.
    /// `DuckDB`'s `substr` counts Unicode code points, not bytes, so multibyte
    /// (e.g. UTF-8 emoji or CJK characters) are never split at a byte boundary.
    /// No additional Rust slicing is needed.
    pub message_head: String,
}

/// Return the top-N commits that most recently touched `path`, lineage-aware.
///
/// Results are ordered newest-first (by `commits.date DESC, rev DESC` for
/// determinism when two commits share the same timestamp).  `n` is capped at
/// 5 by the caller contract — GitHub renders `codeFlows` in full but chains
/// longer than 5 entries add noise without improving actionability.
///
/// Returns an empty `Vec` when `path` has no history (nonexistent path or a
/// path that predates the ingest window).
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on SQL preparation or execution
/// failure.
pub fn evidence_for_path(
    db: &FactsDb,
    opts: &Options,
    path: &str,
    n: u32,
) -> Result<Vec<EvidenceCommit>> {
    crate::analyses::lineage::materialize_if_needed(db, opts)?;
    let src = crate::analyses::lineage::source_table(opts);

    let sql = format!(
        "SELECT co.rev,
                CAST(co.date AS TEXT),
                co.canonical_author,
                c.loc_added + c.loc_deleted AS churn,
                substr(split_part(co.message, chr(10), 1), 1, 80)
         FROM {src} c
         JOIN commits co USING (rev)
         WHERE c.path = ?
         ORDER BY co.date DESC, co.rev DESC
         LIMIT ?"
    );

    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| CodeLoreError::Analysis(format!("evidence_for_path prepare: {e}")))?;

    let rows = stmt
        .query_map(duckdb::params![path, n], |r| {
            Ok(EvidenceCommit {
                rev: r.get(0)?,
                date: r.get(1)?,
                author: r.get(2)?,
                churn: r.get(3)?,
                message_head: r.get(4)?,
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("evidence_for_path query: {e}")))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("evidence_for_path row: {e}")))?;

    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Options;
    use crate::facts::FactsDb;
    use crate::repo::GixRepo;

    #[cfg(feature = "test-support")]
    use crate::test_support::biomarker_repo;

    /// Ingest `biomarker_repo` and return `(db, opts)`.
    #[cfg(feature = "test-support")]
    fn ingest_biomarker() -> (FactsDb, Options, biomarker_repo::BiomarkerRepo) {
        let repo = biomarker_repo::build();
        let opts = Options {
            repo_path: repo.dir.path().to_path_buf(),
            min_revs: 1,
            ..Options::default()
        };
        let gix = GixRepo::open(&opts.repo_path).expect("open gix repo");
        let db = FactsDb::open_or_ingest(&opts, &gix).expect("ingest");
        (db, opts, repo)
    }

    /// `src/complex.rs` is touched in commits at dates[1], [2], [5] (the
    /// three edit commits after the seed).  Evidence should return those
    /// three rows, newest-first, with churn > 0.
    #[test]
    #[cfg(feature = "test-support")]
    fn evidence_newest_first_and_churn_positive() {
        let (db, opts, _repo) = ingest_biomarker();
        let rows = evidence_for_path(&db, &opts, "src/complex.rs", 5).expect("evidence_for_path");

        // At least the 3 edit commits (seed may or may not appear depending on
        // whether the initial add registers as churn; the SQL uses loc_added +
        // loc_deleted, so the seed commit IS included as churn > 0).
        assert!(
            !rows.is_empty(),
            "expected evidence rows for src/complex.rs, got none"
        );

        // All rows must have churn > 0 (loc_added + loc_deleted for that path).
        for r in &rows {
            assert!(
                r.churn > 0,
                "expected churn > 0 for every evidence row, got churn={} at {}",
                r.churn,
                r.date,
            );
        }

        // Rows must be ordered newest-first.
        for w in rows.windows(2) {
            assert!(
                w[0].date >= w[1].date,
                "expected newest-first ordering: {} < {}",
                w[0].date,
                w[1].date,
            );
        }

        // Message heads must be non-empty.
        for r in &rows {
            assert!(
                !r.message_head.is_empty(),
                "expected non-empty message_head, got empty at {}",
                r.date,
            );
        }
    }

    /// A nonexistent path returns an empty vec (not an error).
    #[test]
    #[cfg(feature = "test-support")]
    fn evidence_nonexistent_path_returns_empty() {
        let (db, opts, _repo) = ingest_biomarker();
        let rows = evidence_for_path(&db, &opts, "src/does_not_exist.rs", 5)
            .expect("evidence_for_path for nonexistent path");
        assert!(
            rows.is_empty(),
            "expected empty vec for nonexistent path, got {} rows",
            rows.len(),
        );
    }

    /// `n` is respected: requesting 2 returns at most 2 rows even when more
    /// exist.
    #[test]
    #[cfg(feature = "test-support")]
    fn evidence_n_limits_results() {
        let (db, opts, _repo) = ingest_biomarker();
        // complex.rs has at least 3 edit commits.
        let rows =
            evidence_for_path(&db, &opts, "src/complex.rs", 2).expect("evidence_for_path with n=2");
        assert!(
            rows.len() <= 2,
            "expected at most 2 rows, got {}",
            rows.len(),
        );
    }

    /// Same-second commits are ordered deterministically by `(date DESC, rev DESC)`.
    /// This test verifies that the ORDER BY clause compiles and runs without
    /// error — full same-second determinism is an ordering guarantee, not
    /// a value guarantee on a fixture where timestamps are distinct.
    #[test]
    #[cfg(feature = "test-support")]
    fn evidence_ordering_is_deterministic() {
        let (db, opts, _repo) = ingest_biomarker();
        let rows1 = evidence_for_path(&db, &opts, "src/complex.rs", 5).expect("first call");
        let rows2 = evidence_for_path(&db, &opts, "src/complex.rs", 5).expect("second call");
        assert_eq!(
            rows1.iter().map(|r| &r.rev).collect::<Vec<_>>(),
            rows2.iter().map(|r| &r.rev).collect::<Vec<_>>(),
            "evidence ordering must be deterministic across identical calls"
        );
    }

    /// Build a minimal one-commit repo with the given message, ingest it, and
    /// return `(db, opts, _dir)`.  `_dir` must be kept alive while db is used.
    #[cfg(feature = "test-support")]
    fn repo_with_message(msg: &str) -> (FactsDb, Options, tempfile::TempDir) {
        use std::process::Command;
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path();
        let git = |args: &[&str]| {
            let ok = Command::new("git")
                .arg("-C")
                .arg(p)
                .args(args)
                .status()
                .expect("git")
                .success();
            assert!(ok, "git {args:?} failed");
        };
        git(&["init", "-b", "main", "--quiet"]);
        git(&["config", "user.email", "t@test.com"]);
        git(&["config", "user.name", "T"]);
        std::fs::write(p.join("f.rs"), "fn x() {}").unwrap();
        git(&["add", "f.rs"]);
        // Pass the message via GIT_EDITOR=-less + --allow-empty-message workaround:
        // use -m with the raw message string (git handles arbitrary content via -m).
        let ok = Command::new("git")
            .arg("-C")
            .arg(p)
            .args(["commit", "--quiet", "-m"])
            .arg(msg)
            .status()
            .expect("git commit")
            .success();
        assert!(ok, "git commit with long message failed");

        let opts = Options {
            repo_path: p.to_path_buf(),
            min_revs: 1,
            ..Options::default()
        };
        let gix = GixRepo::open(p).expect("open gix repo");
        let db = FactsDb::open_or_ingest(&opts, &gix).expect("ingest");
        (db, opts, dir)
    }

    /// A commit message longer than 90 ASCII characters must be truncated to
    /// exactly 80 characters in `message_head`.
    ///
    /// `DuckDB`'s `substr(split_part(message, chr(10), 1), 1, 80)` counts Unicode
    /// code points, not bytes.  For ASCII this equals the character count.
    #[test]
    #[cfg(feature = "test-support")]
    fn evidence_message_head_truncated_to_80_ascii() {
        // 95-char ASCII first line (no newline), so split_part returns the whole
        // thing and substr caps it at 80.
        let long_msg = "a".repeat(95);
        let (db, opts, _dir) = repo_with_message(&long_msg);
        let rows = evidence_for_path(&db, &opts, "f.rs", 5).expect("evidence_for_path");
        assert!(
            !rows.is_empty(),
            "expected at least one evidence row for f.rs"
        );
        let head = &rows[0].message_head;
        assert_eq!(
            head.chars().count(),
            80,
            "message_head must be exactly 80 chars for a 95-char input, got {:?} (len {})",
            head,
            head.chars().count(),
        );
    }

    /// A commit message whose 80th code-point boundary falls inside a multibyte
    /// character (e.g. a CJK character spanning 3 UTF-8 bytes) must not panic
    /// and must return ≤ 80 Unicode code points — never a partial byte sequence.
    ///
    /// `DuckDB`'s `substr` is code-point-aware (UTF-8 safe), so this is enforced
    /// in SQL without any Rust slicing.
    #[test]
    #[cfg(feature = "test-support")]
    fn evidence_message_head_multibyte_safe() {
        // 79 ASCII chars + 5 CJK ideographs (each 3 UTF-8 bytes).
        // The boundary at position 80 falls mid-ideograph in a byte-naive slicer
        // but DuckDB substr counts code points, so position 80 = the 80th
        // ideograph character — no partial byte sequence is possible.
        let msg = format!("{}{}", "x".repeat(79), "字".repeat(5));
        assert!(msg.chars().count() == 84, "sanity: 84 code points total");

        let (db, opts, _dir) = repo_with_message(&msg);
        let rows = evidence_for_path(&db, &opts, "f.rs", 5).expect("evidence_for_path");
        assert!(
            !rows.is_empty(),
            "expected at least one evidence row for f.rs"
        );
        let head = &rows[0].message_head;
        // Must be at most 80 code points and must be valid UTF-8 (no panic on
        // chars().count() means no partial byte sequence).
        let char_count = head.chars().count();
        assert!(
            char_count <= 80,
            "message_head must be ≤ 80 code points, got {char_count}: {head:?}"
        );
        // Must contain the first 79 ASCII chars.
        assert!(
            head.starts_with(&"x".repeat(79)),
            "first 79 chars must be ASCII 'x', got: {head:?}"
        );
    }
}

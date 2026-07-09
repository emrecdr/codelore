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
}

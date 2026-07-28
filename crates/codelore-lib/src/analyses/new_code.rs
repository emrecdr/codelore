//! `new-code` gate scope — the born/touched partition of the active working set
//! that the `[new_code]` two-band gate evaluates.
//!
//! This module is the impure half (fact-store + repo access) of the gate; the
//! pure comparison against the configured thresholds lives in
//! [`crate::quality_gates::evaluate_new_code_rows`], mirroring the
//! analysis→rows→evaluator split every other gate uses.
//!
//! ## The partition
//!
//! Over a rolling window of `window_days` anchored to the repo's most recent
//! commit date (reproducible on archived repos, matching every other windowed
//! analysis):
//!
//! - **Born in window** — a live-at-HEAD source file whose first commit lands
//!   inside the window. Its HEAD code-health score is carried for the born band.
//! - **Touched (not born) in window** — a live-at-HEAD source file whose most
//!   recent commit lands inside the window but whose first commit predates it.
//!   Its net health movement over the window is computed by the *shared*
//!   window-start scan ([`effort_exposure::window_net_movement`]), the same
//!   machinery the improving-churn effort exemption uses — no second scan.
//! - Everything else (untouched legacy, non-source, files deleted before HEAD)
//!   is out of scope.
//!
//! ## Shallow history
//!
//! When no commit predates the window ([`effort_exposure::window_start_rev`]
//! returns `None`), the whole repository fits inside the window: there is no
//! legacy tail to contrast the working set against, so the partition is
//! meaningless. [`run_new_code_scope`] reports this via
//! [`NewCodeScope::window_start_present`] `= false`, and the gate surfaces skip
//! with a disclosure rather than flag every file as born.

use std::collections::{HashMap, HashSet};

use crate::analyses::code_health::CodeHealthRow;
use crate::analyses::{effort_exposure, lineage, query};
use crate::facts::FactsDb;
use crate::repo::Repo;
use crate::{Options, Result};

/// The born/touched partition of the working set for one `[new_code]` window.
///
/// `born` and `touched` are sorted by path so the downstream violations render
/// deterministically. Empty vectors with `window_start_present = false` signal
/// the shallow-history skip.
#[derive(Debug, Clone, Default)]
pub struct NewCodeScope {
    /// `false` when history is shallower than the window (no pre-window commit);
    /// the gate skips with a disclosure in that case.
    pub window_start_present: bool,
    /// `(path, HEAD code-health score)` for each live-at-HEAD source file born
    /// inside the window — the born band's inputs.
    pub born: Vec<(String, f64)>,
    /// `(path, net health movement over the window)` for each live-at-HEAD
    /// source file touched but not born inside the window — the touched band's
    /// inputs. Positive is improvement, negative is degradation, zero is no
    /// risk-band movement.
    pub touched: Vec<(String, f64)>,
}

/// Compute the [`NewCodeScope`] for `window_days` against the fact store.
///
/// `health` must be the HEAD-scope, unlimited-row code-health rows (as the gate
/// path already computes for `code_health_min`): its scores feed the born band,
/// and its path set is the live-at-HEAD source universe that scopes both bands.
/// Reusing it means no second health scan — the born band is a pure lookup and
/// the touched band adds only the scoped window-start parse shared with the
/// effort decomposition.
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on SQL or row-mapping failure, or
/// any error from the shared window-start scan.
pub fn run_new_code_scope<R: Repo>(
    db: &FactsDb,
    repo: &R,
    opts: &Options,
    window_days: u32,
    health: &[CodeHealthRow],
) -> Result<NewCodeScope> {
    // Shallow-history skip: no commit strictly before the window ⇒ no legacy
    // baseline. Resolve it first so an empty-commits fact store (head-only
    // ingest) or a too-young repo short-circuits before any partition query.
    let Some(window_start) = effort_exposure::window_start_rev(db, window_days)? else {
        return Ok(NewCodeScope::default());
    };

    // Live-at-HEAD source files carry the born band's HEAD scores; a path absent
    // from the health rows (non-source, or deleted before HEAD) is out of scope.
    let scores: HashMap<&str, f64> = health.iter().map(|r| (r.path.as_str(), r.score)).collect();

    lineage::materialize_if_needed(db, opts)?;
    let src = lineage::source_table(opts);

    let mut born = Vec::new();
    let mut touched_paths: HashSet<String> = HashSet::new();
    for (path, born_in_window, touched_in_window) in born_touched_flags(db, src, window_days)? {
        let Some(&score) = scores.get(path.as_str()) else {
            continue; // out of scope: non-source or not live at HEAD
        };
        if born_in_window {
            born.push((path, score));
        } else if touched_in_window {
            // Born ⊂ touched, so the born branch above already excludes born
            // files from the touched band (the two-band split).
            touched_paths.insert(path);
        }
    }

    // Touched-but-not-born: the shared window-start net-movement scan, scoped to
    // exactly this set (never a full-tree health scan).
    let net = effort_exposure::window_net_movement(
        db,
        repo,
        &touched_paths,
        Some(window_start.as_str()),
    )?;
    let mut touched: Vec<(String, f64)> = net.into_iter().collect();

    born.sort_by(|a, b| a.0.cmp(&b.0));
    touched.sort_by(|a, b| a.0.cmp(&b.0));

    Ok(NewCodeScope {
        window_start_present: true,
        born,
        touched,
    })
}

/// Per-path `(born_in_window, touched_in_window)` flags over the trailing
/// window. A path is *born* in the window when its earliest commit lands inside
/// it (`MIN(date) >= boundary`) and *touched* when its latest does
/// (`MAX(date) >= boundary`); born ⊂ touched follows from `MIN <= MAX`. The
/// boundary is the repo's last commit date minus `wd` days — identical to the
/// effort-exposure window anchor.
fn born_touched_flags(db: &FactsDb, src: &str, wd: u32) -> Result<Vec<(String, bool, bool)>> {
    let sql = format!(
        "SELECT c.path,
                (MIN(ci.date) >= (SELECT MAX(date) FROM commits) - INTERVAL '{wd} days') AS born,
                (MAX(ci.date) >= (SELECT MAX(date) FROM commits) - INTERVAL '{wd} days') AS touched
         FROM {src} c
         JOIN commits ci ON ci.rev = c.rev
         GROUP BY c.path"
    );
    query::query_map_collect(db, &sql, [], "new-code born/touched partition", |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, bool>(1)?,
            r.get::<_, bool>(2)?,
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::born_touched_flags;
    use crate::analyses::effort_exposure::window_start_rev;
    use crate::facts::FactsDb;
    use std::collections::HashMap;

    /// Seed a commit with a fixed author date; the other columns are inert for
    /// the born/touched partition, which reads only `date`.
    fn seed_commit(db: &FactsDb, rev: &str, date: &str) {
        db.conn()
            .execute(
                &format!(
                    "INSERT INTO commits (rev, author_email, author_name, committer_email, \
                     canonical_author, date, committer_date, message, is_merge, parent_count) \
                     VALUES ('{rev}', 'a@b.com', 'A', 'a@b.com', 'A', \
                     TIMESTAMP '{date}', TIMESTAMP '{date}', 'm', false, 1)"
                ),
                [],
            )
            .expect("insert commit");
    }

    fn seed_change(db: &FactsDb, rev: &str, path: &str, kind: &str) {
        db.conn()
            .execute(
                &format!(
                    "INSERT INTO changes (rev, path, change_type, loc_added, loc_deleted) \
                     VALUES ('{rev}', '{path}', '{kind}', 10, 0)"
                ),
                [],
            )
            .expect("insert change");
    }

    #[test]
    fn born_touched_partition_splits_by_first_and_last_touch() {
        // MAX(date) = 2026-06-01; a 90-day window opens ~2026-03-03.
        //   legacy.rs  first Jan01 (< open), last May01 (>= open) → touched, not born
        //   fresh.rs   first Jun01 (>= open)                       → born (⊂ touched)
        //   ancient.rs first Jan01, last Feb01 (both < open)       → out of scope
        let db = FactsDb::new_in_memory().expect("db");
        seed_commit(&db, "c1", "2026-01-01");
        seed_commit(&db, "c2", "2026-02-01");
        seed_commit(&db, "c3", "2026-05-01");
        seed_commit(&db, "c4", "2026-06-01");
        seed_change(&db, "c1", "legacy.rs", "added");
        seed_change(&db, "c3", "legacy.rs", "modified");
        seed_change(&db, "c4", "fresh.rs", "added");
        seed_change(&db, "c1", "ancient.rs", "added");
        seed_change(&db, "c2", "ancient.rs", "modified");

        let flags: HashMap<String, (bool, bool)> = born_touched_flags(&db, "changes", 90)
            .expect("partition")
            .into_iter()
            .map(|(p, born, touched)| (p, (born, touched)))
            .collect();

        assert_eq!(flags["legacy.rs"], (false, true), "touched but not born");
        assert_eq!(flags["fresh.rs"], (true, true), "born ⊂ touched");
        assert_eq!(flags["ancient.rs"], (false, false), "untouched legacy");
    }

    #[test]
    fn shallow_history_has_no_window_start() {
        // Every commit sits inside the window ⇒ no pre-window baseline ⇒
        // window_start_rev is None, which run_new_code_scope treats as the
        // shallow-history skip.
        let db = FactsDb::new_in_memory().expect("db");
        seed_commit(&db, "c1", "2026-05-20");
        seed_commit(&db, "c2", "2026-06-01");
        assert!(
            window_start_rev(&db, 90).expect("query").is_none(),
            "history shallower than the window has no pre-window rev"
        );

        // With a commit older than the window, a baseline exists.
        seed_commit(&db, "c0", "2026-01-01");
        assert_eq!(
            window_start_rev(&db, 90).expect("query").as_deref(),
            Some("c0"),
            "the newest pre-window commit anchors the window start"
        );
    }
}

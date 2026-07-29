//! Betterer-style committed quality snapshot for `codelore check --ratchet`.
//!
//! The snapshot file `.codelore-ratchet.toml` lives in the target repo and is
//! committed by the user. Three semantics:
//!
//! 1. **No snapshot** → measure current values, write the file, exit 0.
//!    Print "ratchet initialized".
//! 2. **Snapshot exists, any metric WORSE** → exit 1 listing regressions.
//!    The file is NOT rewritten on failure (preserve the committed baseline).
//! 3. **Snapshot exists, all same-or-better** → rewrite the file with the
//!    improved values (tighten the ratchet), exit 0, print which keys tightened.
//!
//! The design follows the Betterer pattern: the ratchet file is the contract
//! and git history is the audit trail — commit the file; your gate history is
//! then minable from git.
//!
//! The file also carries a `ratchet_schema` key (see [`RATCHET_SCHEMA`]) so a
//! baseline written under one definition of a gated metric is never silently
//! compared against a later, redefined one — a schema mismatch (including a
//! pre-fix file with no key at all) is treated as case 1, with a one-time
//! warning explaining the reset.

use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{CodeLoreError, Result};

/// Filename of the ratchet snapshot, written to the repo root.
pub const RATCHET_FILENAME: &str = ".codelore-ratchet.toml";

/// Ratchet-file schema version — a manual cache-buster for the committed
/// `.codelore-ratchet.toml`, written into the file as the `ratchet_schema`
/// key on every save. Bump this when a gated metric's scale or meaning
/// changes (e.g. the hotspot/code-health scoring formula is re-anchored),
/// so a baseline written under the old definition is discarded rather than
/// silently compared against the redefined metric. Mirrors
/// `cache.rs::CACHE_EPOCH`'s manual-bump pattern, but scoped to ratchet
/// gate-metric redefinitions rather than fact-store correctness fixes.
///
/// Files written before this key existed deserialize `ratchet_schema` via
/// `#[serde(default)]` to `0`, which never equals a real schema version
/// (starting at `1`) — so pre-fix files take the same discard-and-reset
/// path as a genuine mismatch, exactly once.
const RATCHET_SCHEMA: u32 = 1;

/// Per-metric direction: which direction is "better"?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// A higher observed value is better (e.g. code-health score).
    HigherBetter,
    /// A lower observed value is better (e.g. red-effort %, cycle count).
    LowerBetter,
}

/// Canonical ratchet metrics with their improvement directions.
///
/// Keys match the TOML `[ratchet]` field names. The order is stable and
/// used when writing the snapshot.
pub const RATCHET_METRICS: &[(&str, Direction)] = &[
    ("code_health_min_observed", Direction::HigherBetter),
    ("red_effort_pct_observed", Direction::LowerBetter),
    ("dependency_cycles_observed", Direction::LowerBetter),
];

/// Observed values for the three ratchet metrics, extracted from gate outputs.
///
/// `None` means the corresponding analysis produced no data (e.g. the repo
/// has no scorable files for `code_health_min_observed`). A `None` metric is
/// skipped in both comparison and snapshot writes.
///
/// `code_familiarity_min` deliberately has no ratchet slot: familiarity moves
/// with team activity rather than code changes, so ratcheting it would flag
/// regressions on quiet weeks. Configure it as a plain gate instead.
#[derive(Debug, Clone, Default)]
pub struct RatchetMetrics {
    /// Worst per-file composite code-health score observed (0–100, higher=better).
    pub code_health_min_observed: Option<f64>,
    /// Red-band share of window LOC churn (percent, 0–100, lower=better).
    pub red_effort_pct_observed: Option<f64>,
    /// Count of non-trivial import-graph dependency cycles (lower=better).
    pub dependency_cycles_observed: Option<f64>,
}

/// Persisted snapshot TOML structure.
///
/// Only fields present in the file are compared; absent fields skip
/// the corresponding metric check so users can opt individual metrics in/out.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RatchetSnapshot {
    /// Schema version this snapshot was written under. See [`RATCHET_SCHEMA`].
    #[serde(default)]
    pub ratchet_schema: u32,
    #[serde(default)]
    pub ratchet: RatchetTable,
}

/// The `[ratchet]` TOML table.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RatchetTable {
    pub code_health_min_observed: Option<f64>,
    pub red_effort_pct_observed: Option<f64>,
    pub dependency_cycles_observed: Option<f64>,
}

/// Outcome of `evaluate_ratchet`.
#[derive(Debug)]
pub enum RatchetOutcome {
    /// All metrics same-or-better. Field: keys that actually improved.
    Improved { tightened: Vec<String> },
    /// One or more metrics worse than the snapshot.
    Regressed { regressions: Vec<RatchetRegression> },
}

/// A single metric that regressed.
#[derive(Debug)]
pub struct RatchetRegression {
    pub key: String,
    pub snapshot_value: f64,
    pub observed_value: f64,
    pub direction: Direction,
}

/// Derive a `RatchetSnapshot` from the current observed metrics.
#[must_use]
pub fn snapshot_from_metrics(metrics: &RatchetMetrics) -> RatchetSnapshot {
    RatchetSnapshot {
        ratchet_schema: RATCHET_SCHEMA,
        ratchet: RatchetTable {
            code_health_min_observed: metrics.code_health_min_observed,
            red_effort_pct_observed: metrics.red_effort_pct_observed,
            dependency_cycles_observed: metrics.dependency_cycles_observed,
        },
    }
}

/// Read the ratchet snapshot from disk.
///
/// Returns `Ok(None)` when the file does not exist.
/// Returns `Err` (typed as `CodeLoreError::Analysis`) when the file exists but
/// cannot be parsed — this is the refuse-and-error case.
///
/// # Errors
///
/// [`CodeLoreError::Analysis`] on I/O or TOML parse failure when the file exists.
pub fn read_snapshot(repo_root: &Path) -> Result<Option<RatchetSnapshot>> {
    let path = ratchet_path(repo_root);
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path).map_err(|e| {
        CodeLoreError::Analysis(format!("read ratchet file {}: {e}", path.display()))
    })?;
    // A present-but-empty file is corrupt, not absent. An empty string parses
    // cleanly to an all-`None` snapshot (every field is `#[serde(default)]`),
    // which `evaluate_ratchet` reads as "everything improved" and then silently
    // rebaselines the committed floor from the current — possibly regressed —
    // run. Three states must stay distinct: NO file (`Ok(None)`, initialize); a
    // DESTROYED file (0-byte or whitespace — a torn write, an empty merge
    // resolution, manual truncation — the loud error here); and a well-formed
    // file (parsed below, where an empty `[ratchet]` table legitimately means
    // "no floors configured"). "The floors were destroyed" must never look like
    // "no floors were configured".
    if raw.trim().is_empty() {
        return Err(CodeLoreError::Analysis(format!(
            "ratchet file {} is present but empty — the committed baseline was destroyed \
             (a truncated write, an empty merge resolution, or manual truncation). An empty \
             ratchet must not silently rebaseline the gate: restore the file from version \
             control, or delete it to re-initialize the baseline from this run.",
            path.display()
        )));
    }
    // The typed parse rejects truncated/corrupt TOML alike — no separate
    // untyped pre-parse is needed.
    let snap = toml::from_str::<RatchetSnapshot>(&raw).map_err(|e| {
        CodeLoreError::Analysis(format!(
            "ratchet file is not valid TOML ({}): {e}",
            path.display()
        ))
    })?;
    // A well-formed file written under a different (or absent, i.e. pre-fix)
    // `ratchet_schema` compared a gated metric against a baseline that no
    // longer means the same thing — e.g. a re-anchored hotspot/code-health
    // formula. Unlike the destroyed-file case above, this is not corrupt: it
    // is a legitimate file that is simply stale relative to the current
    // metric definitions. Discard it (return as if absent) so the caller's
    // existing "no snapshot" path re-initializes the baseline from this run,
    // and say so once so the reset isn't mysterious.
    if snap.ratchet_schema != RATCHET_SCHEMA {
        tracing::warn!(
            "ratchet reset: {} was written under gate-metric schema {} (current {}); gate \
             metric definitions changed since that baseline was recorded, so it is discarded \
             and baselines re-establish on this run.",
            path.display(),
            snap.ratchet_schema,
            RATCHET_SCHEMA
        );
        return Ok(None);
    }
    Ok(Some(snap))
}

/// Write the ratchet snapshot to disk, overwriting any existing file.
///
/// # Errors
///
/// [`CodeLoreError::Analysis`] on I/O or serialization failure.
pub fn write_snapshot(repo_root: &Path, snap: &RatchetSnapshot) -> Result<()> {
    let path = ratchet_path(repo_root);
    let header = "# Generated by `codelore check --ratchet`. Commit this file.\n\
                  # Ratchet tightens automatically on improvement; edit manually to relax.\n\n";
    let body = toml::to_string_pretty(snap)
        .map_err(|e| CodeLoreError::Analysis(format!("serialize ratchet snapshot: {e}")))?;
    let payload = format!("{header}{body}");
    // A committed baseline is a shared floor for the whole team, so the write
    // must be all-or-nothing. Plain `fs::write` truncates then writes; a crash,
    // OOM kill, or ENOSPC between the two would leave a 0-byte file that parses
    // back to "everything improved" and silently rebaselines the gate. Route
    // through the same write-to-temp + fsync + atomic-rename helper every
    // analyze output uses, so an interrupted write leaves the previous good file
    // untouched rather than a torn one.
    crate::output::atomic_publish(&path, |tmp| {
        fs::write(tmp, payload.as_bytes()).map_err(|e| {
            CodeLoreError::Analysis(format!("write ratchet file {}: {e}", path.display()))
        })
    })
}

/// Compare current metrics against the snapshot and determine the outcome.
///
/// Metrics absent from both the snapshot and the current run are skipped.
/// A metric present in the snapshot but absent from the current run is a
/// regression (analysis degraded).
#[must_use]
pub fn evaluate_ratchet(snap: &RatchetSnapshot, metrics: &RatchetMetrics) -> RatchetOutcome {
    let mut regressions: Vec<RatchetRegression> = Vec::new();
    let mut tightened: Vec<String> = Vec::new();

    for (key, direction) in RATCHET_METRICS {
        let snap_val = match *key {
            "code_health_min_observed" => snap.ratchet.code_health_min_observed,
            "red_effort_pct_observed" => snap.ratchet.red_effort_pct_observed,
            "dependency_cycles_observed" => snap.ratchet.dependency_cycles_observed,
            _ => None,
        };
        let obs_val = match *key {
            "code_health_min_observed" => metrics.code_health_min_observed,
            "red_effort_pct_observed" => metrics.red_effort_pct_observed,
            "dependency_cycles_observed" => metrics.dependency_cycles_observed,
            _ => None,
        };

        // Metric absent from snapshot: not yet ratcheted, skip.
        let Some(sv) = snap_val else { continue };
        // Metric present in snapshot but absent now: degraded → regression.
        let Some(ov) = obs_val else {
            regressions.push(RatchetRegression {
                key: (*key).to_owned(),
                snapshot_value: sv,
                observed_value: f64::NAN,
                direction: *direction,
            });
            continue;
        };

        // TOML's ryu float serialization is lossless, so a value written and
        // re-read compares bit-identical. f64::EPSILON guards only true
        // recomputation noise (e.g. slightly different code-health averages
        // across runs on the same commit), not serialization round-trip drift.
        let worse = match direction {
            Direction::HigherBetter => ov < sv - f64::EPSILON,
            Direction::LowerBetter => ov > sv + f64::EPSILON,
        };
        let strictly_better = match direction {
            Direction::HigherBetter => ov > sv + f64::EPSILON,
            Direction::LowerBetter => ov < sv - f64::EPSILON,
        };

        if worse {
            regressions.push(RatchetRegression {
                key: (*key).to_owned(),
                snapshot_value: sv,
                observed_value: ov,
                direction: *direction,
            });
        } else if strictly_better {
            tightened.push((*key).to_owned());
        }
    }

    if regressions.is_empty() {
        RatchetOutcome::Improved { tightened }
    } else {
        RatchetOutcome::Regressed { regressions }
    }
}

/// Format the ratchet outcome as a human-readable message for stdout/stderr.
#[must_use]
pub fn format_ratchet_outcome(outcome: &RatchetOutcome) -> String {
    let mut out = String::new();
    match outcome {
        RatchetOutcome::Improved { tightened } => {
            if tightened.is_empty() {
                writeln!(out, "✅ ratchet: no regression (all metrics held)").unwrap();
            } else {
                writeln!(out, "✅ ratchet: tightened {}", tightened.join(", ")).unwrap();
            }
        }
        RatchetOutcome::Regressed { regressions } => {
            writeln!(out, "❌ ratchet: {} regression(s):", regressions.len()).unwrap();
            for r in regressions {
                if r.observed_value.is_nan() {
                    writeln!(
                        out,
                        "  - {}: was {:.2} → now unavailable (analysis degraded)",
                        r.key, r.snapshot_value
                    )
                    .unwrap();
                } else {
                    let dir = match r.direction {
                        Direction::HigherBetter => "↓ (worse)",
                        Direction::LowerBetter => "↑ (worse)",
                    };
                    writeln!(
                        out,
                        "  - {}: was {:.2} → now {:.2} {dir}",
                        r.key, r.snapshot_value, r.observed_value
                    )
                    .unwrap();
                }
            }
        }
    }
    out
}

/// Path of the ratchet snapshot file.
#[must_use]
pub fn ratchet_path(repo_root: &Path) -> PathBuf {
    repo_root.join(RATCHET_FILENAME)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tmp() -> TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    fn metrics_good() -> RatchetMetrics {
        RatchetMetrics {
            code_health_min_observed: Some(65.0),
            red_effort_pct_observed: Some(10.0),
            dependency_cycles_observed: Some(2.0),
        }
    }

    // ── init ──────────────────────────────────────────────────────────────────

    #[test]
    fn init_writes_snapshot_when_missing() {
        let dir = tmp();
        assert!(read_snapshot(dir.path()).unwrap().is_none());
        let snap = snapshot_from_metrics(&metrics_good());
        write_snapshot(dir.path(), &snap).unwrap();
        let read_back = read_snapshot(dir.path()).unwrap().unwrap();
        assert!((read_back.ratchet.code_health_min_observed.unwrap() - 65.0).abs() < 0.01);
        assert!((read_back.ratchet.red_effort_pct_observed.unwrap() - 10.0).abs() < 0.01);
        assert!((read_back.ratchet.dependency_cycles_observed.unwrap() - 2.0).abs() < 0.01);
    }

    // ── corrupt TOML → typed error ────────────────────────────────────────────

    #[test]
    fn corrupt_toml_returns_typed_error() {
        let dir = tmp();
        fs::write(dir.path().join(RATCHET_FILENAME), b"not valid toml ][[[").unwrap();
        let err = read_snapshot(dir.path()).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("not valid TOML"),
            "expected TOML error, got: {msg}"
        );
    }

    // ── destroyed baseline: 0-byte / whitespace-only → corrupt, not improved ──

    #[test]
    fn empty_ratchet_file_is_corrupt_not_improved() {
        // The torn-write signature: a 0-byte file. It parses to an all-`None`
        // snapshot that would read as "everything improved" and silently
        // rebaseline the committed floor. read_snapshot must instead fail loudly
        // — distinct from a missing file (which legitimately initializes) and a
        // well-formed no-floors file.
        let dir = tmp();
        fs::write(dir.path().join(RATCHET_FILENAME), b"").unwrap();
        let err = read_snapshot(dir.path()).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("empty") && msg.contains("destroyed"),
            "expected a loud destroyed-baseline error, got: {msg}"
        );
    }

    #[test]
    fn whitespace_only_ratchet_file_is_corrupt() {
        // Whitespace-only content trims to empty and parses to all-`None` just
        // like a 0-byte file — same corruption, same loud error.
        let dir = tmp();
        fs::write(dir.path().join(RATCHET_FILENAME), b"\n  \n").unwrap();
        assert!(
            read_snapshot(dir.path()).is_err(),
            "a whitespace-only ratchet file must be rejected as corrupt"
        );
    }

    #[test]
    fn well_formed_empty_table_is_no_floors_not_corrupt() {
        // A serialized snapshot with no metrics set (and the current schema
        // key, so it isn't treated as a pre-fix legacy file) is the legitimate
        // "no floors configured" state — distinct from destroyed — and must
        // round-trip as an all-`None` snapshot rather than erroring.
        let dir = tmp();
        fs::write(
            dir.path().join(RATCHET_FILENAME),
            format!("ratchet_schema = {RATCHET_SCHEMA}\n\n[ratchet]\n"),
        )
        .unwrap();
        let snap = read_snapshot(dir.path())
            .expect("a well-formed empty table must not be corrupt")
            .expect("a present file at the current schema returns Some");
        assert!(snap.ratchet.code_health_min_observed.is_none());
        assert!(snap.ratchet.red_effort_pct_observed.is_none());
        assert!(snap.ratchet.dependency_cycles_observed.is_none());
    }

    // ── ratchet_schema: reset on missing/stale key, accept on current key ────

    #[test]
    fn ratchet_schema_round_trips_through_write_and_read() {
        let dir = tmp();
        let snap = snapshot_from_metrics(&metrics_good());
        write_snapshot(dir.path(), &snap).unwrap();
        let read_back = read_snapshot(dir.path()).unwrap().unwrap();
        assert_eq!(read_back.ratchet_schema, RATCHET_SCHEMA);
    }

    #[test]
    fn missing_ratchet_schema_key_discards_snapshot() {
        // A file written before the `ratchet_schema` key existed — the exact
        // shape `well_formed_empty_table_is_no_floors_not_corrupt` used to
        // write. It must now be discarded (treated as absent) rather than
        // silently compared against redefined metrics.
        let dir = tmp();
        fs::write(
            dir.path().join(RATCHET_FILENAME),
            b"[ratchet]\ncode_health_min_observed = 65.0\n",
        )
        .unwrap();
        assert!(
            read_snapshot(dir.path()).unwrap().is_none(),
            "a file with no ratchet_schema key must be discarded, not served"
        );
    }

    #[test]
    fn stale_ratchet_schema_key_discards_snapshot() {
        let dir = tmp();
        fs::write(
            dir.path().join(RATCHET_FILENAME),
            b"ratchet_schema = 999\n\n[ratchet]\ncode_health_min_observed = 65.0\n",
        )
        .unwrap();
        assert!(
            read_snapshot(dir.path()).unwrap().is_none(),
            "a file with a stale ratchet_schema key must be discarded, not served"
        );
    }

    #[test]
    fn current_ratchet_schema_key_is_accepted() {
        let dir = tmp();
        fs::write(
            dir.path().join(RATCHET_FILENAME),
            format!(
                "ratchet_schema = {RATCHET_SCHEMA}\n\n[ratchet]\ncode_health_min_observed = 65.0\n"
            ),
        )
        .unwrap();
        let snap = read_snapshot(dir.path())
            .unwrap()
            .expect("a file at the current schema must be served");
        assert!((snap.ratchet.code_health_min_observed.unwrap() - 65.0).abs() < 0.01);
    }

    #[test]
    fn write_snapshot_leaves_no_temp_stray() {
        // Proves the write routes through atomic_publish: its write-to-temp +
        // rename contract removes the process-unique `.tmp.<pid>` sibling on
        // success, so a completed write leaves only the destination.
        let dir = tmp();
        write_snapshot(dir.path(), &snapshot_from_metrics(&metrics_good())).unwrap();
        let strays: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".tmp."))
            .collect();
        assert!(
            strays.is_empty(),
            "an atomic write must not orphan a temp file: {strays:?}"
        );
        // And the published file is the real snapshot, readable back.
        assert!(read_snapshot(dir.path()).unwrap().is_some());
    }

    // ── regression: snapshot has BETTER values than current → exit-1 scenario ─

    #[test]
    fn better_snapshot_than_current_is_regression() {
        // Snapshot says health was 80 (better); current is 65 → regression.
        let snap = RatchetSnapshot {
            ratchet: RatchetTable {
                code_health_min_observed: Some(80.0),
                red_effort_pct_observed: Some(5.0),
                dependency_cycles_observed: Some(0.0),
            },
            ..Default::default()
        };
        let outcome = evaluate_ratchet(&snap, &metrics_good());
        let RatchetOutcome::Regressed { regressions } = outcome else {
            panic!("expected regression");
        };
        assert_eq!(regressions.len(), 3);
        // code_health: 65 < 80 → worse (HigherBetter)
        assert_eq!(regressions[0].key, "code_health_min_observed");
        // red_effort: 10 > 5 → worse (LowerBetter)
        assert_eq!(regressions[1].key, "red_effort_pct_observed");
        // cycles: 2 > 0 → worse (LowerBetter)
        assert_eq!(regressions[2].key, "dependency_cycles_observed");
    }

    // ── tightening: snapshot has WORSE values than current → pass + rewrite ───

    #[test]
    fn worse_snapshot_than_current_is_improvement() {
        // Snapshot says health was 50 (worse); current is 65 → tighten.
        let snap = RatchetSnapshot {
            ratchet: RatchetTable {
                code_health_min_observed: Some(50.0),
                red_effort_pct_observed: Some(20.0),
                dependency_cycles_observed: Some(5.0),
            },
            ..Default::default()
        };
        let outcome = evaluate_ratchet(&snap, &metrics_good());
        let RatchetOutcome::Improved { tightened } = outcome else {
            panic!("expected improvement");
        };
        assert_eq!(tightened.len(), 3);
    }

    // ── partial snapshot (only one metric) ───────────────────────────────────

    #[test]
    fn partial_snapshot_only_checks_present_keys() {
        let snap = RatchetSnapshot {
            ratchet: RatchetTable {
                code_health_min_observed: Some(80.0), // regression
                red_effort_pct_observed: None,        // not ratcheted
                dependency_cycles_observed: None,     // not ratcheted
            },
            ..Default::default()
        };
        let outcome = evaluate_ratchet(&snap, &metrics_good());
        let RatchetOutcome::Regressed { regressions } = outcome else {
            panic!("expected regression");
        };
        assert_eq!(regressions.len(), 1);
        assert_eq!(regressions[0].key, "code_health_min_observed");
    }

    // ── direction sanity ─────────────────────────────────────────────────────

    #[test]
    fn higher_better_direction_is_correct() {
        // HigherBetter: current > snapshot → improvement (tightened)
        let snap = RatchetSnapshot {
            ratchet: RatchetTable {
                code_health_min_observed: Some(60.0),
                ..Default::default()
            },
            ..Default::default()
        };
        let metrics = RatchetMetrics {
            code_health_min_observed: Some(70.0),
            ..Default::default()
        };
        let outcome = evaluate_ratchet(&snap, &metrics);
        let RatchetOutcome::Improved { tightened } = outcome else {
            panic!("expected improvement");
        };
        assert!(tightened.contains(&"code_health_min_observed".to_owned()));
    }

    #[test]
    fn lower_better_direction_is_correct() {
        // LowerBetter: current < snapshot → improvement (tightened)
        let snap = RatchetSnapshot {
            ratchet: RatchetTable {
                dependency_cycles_observed: Some(3.0),
                ..Default::default()
            },
            ..Default::default()
        };
        let metrics = RatchetMetrics {
            dependency_cycles_observed: Some(1.0),
            ..Default::default()
        };
        let outcome = evaluate_ratchet(&snap, &metrics);
        let RatchetOutcome::Improved { tightened } = outcome else {
            panic!("expected improvement");
        };
        assert!(tightened.contains(&"dependency_cycles_observed".to_owned()));
    }

    // ── degraded metric in current → regression ───────────────────────────────

    #[test]
    fn absent_current_metric_with_snapshot_is_regression() {
        let snap = RatchetSnapshot {
            ratchet: RatchetTable {
                code_health_min_observed: Some(65.0),
                ..Default::default()
            },
            ..Default::default()
        };
        let metrics = RatchetMetrics {
            code_health_min_observed: None, // degraded
            ..Default::default()
        };
        let outcome = evaluate_ratchet(&snap, &metrics);
        let RatchetOutcome::Regressed { regressions } = outcome else {
            panic!("expected regression");
        };
        assert_eq!(regressions.len(), 1);
        assert!(regressions[0].observed_value.is_nan());
    }
}

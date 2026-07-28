//! Historical band scan, validation report, and constrained weight tuning —
//! Units C and D of the own-repo defect-calibration design.
//!
//! # Unit C — the historical band scan + validation report
//!
//! [`band_history`] reuses `health_trend`'s at-rev machinery
//! (`sampled_commits` → `live_paths_at` → `ingest_complexity_at_rev` +
//! `materialize_imports_at_rev` → a `history_cutoff`-scoped
//! `run_code_health_scoped`) to recompute code-health bands at ≤12 evenly
//! spaced historical revisions — but, unlike `health_trend::file_series`,
//! with NO top-50 path cap: the validation report needs every scored file,
//! since any of them might be the one a mined defect touched.
//!
//! [`validate`] matches each `SzzLink`'s defect-introducing commit to the
//! nearest band sample at-or-before its date, tallies the headline band
//! table, and scores HEAD's `structural_risk` against the defect-implicated
//! file labels via `stats::{auc, precision_at_k}`.
//!
//! # Unit D — constrained weight tuning
//!
//! [`tune_weights`] runs a deterministic coordinate-descent search over the
//! eight `SMELL_WEIGHTS` and only adopts a tuned set when the evidence
//! clears an honesty floor: fewer than 30 linked defect-changes, fewer than 10
//! implicated files, a tuned validation AUC below random (0.5), or a
//! validation-AUC improvement short of the acceptance margin, each keep the
//! defaults instead of the tuned weights.
//!
//! ## Design decision: re-scoring without re-running the SQL pass
//!
//! Evaluating a candidate weight set's training/validation AUC needs a
//! `structural_risk` for every file under THAT candidate — but re-running
//! `run_code_health_scoped` (the full SQL pass, including the
//! coupling/god-class/clone sub-scans) once per candidate is far too
//! expensive across a coordinate-descent search. Instead:
//!
//! 1. [`capture_intensities`] reads the `code_health_biomarkers_v1` session
//!    temp table's rows ONCE, immediately after a single HEAD
//!    `run_code_health_scoped` call, in the SAME connection/session (that
//!    table is deliberately left queryable after the scan returns — see its
//!    own doc comment in `code_health.rs`), into a
//!    `HashMap<path, [f64; 8]>` keyed in `SMELL_WEIGHTS` order.
//! 2. Each candidate's risk is then computed Rust-side by
//!    [`structural_risk_from_intensities`]: `Σ wᵢ·intensityᵢ` clamped to
//!    1.0 — mirroring the SQL `LEAST(1.0, SUM(intensity * CASE …))` formula
//!    exactly (the HEAD, `include_clones = true` path, which carries no
//!    renormalization divisor).
//!
//! The two are unit-tested for parity (tolerance `1e-9`) against a real
//! run's `structural_risk` on the biomarker fixture in
//! `tests/defect_calibration_test.rs`.

use std::collections::{HashMap, HashSet};

use crate::analyses::architecture_trend::{
    import_graph_from_live_paths, live_paths_at, sampled_commits,
};
use crate::analyses::code_health::{
    CloneSource, CodeHealthRow, HealthScanCtx, SMELL_WEIGHTS, run_code_health_scoped,
};
use crate::facts::FactsDb;
use crate::facts::ingest::at_rev::{ingest_complexity_at_rev, materialize_imports_at_rev};
use crate::repo::Repo;
use crate::{Options, Result};

use super::szz::SzzLink;
use super::{TuningDecision, ValidationMetrics};

// ─── Unit C: historical band scan ────────────────────────────────────────────

/// Session-scoped temp-table names for [`band_history`]'s rev-scoped scan.
/// Distinct from `health_trend`'s own `cm_at_rev`/`imports_at_rev` names
/// (the same at-rev machinery, `CREATE OR REPLACE` per sample) purely so the
/// two scans can never collide if ever run against the same connection in
/// one session.
const CM_AT_REV: &str = "defect_cm_at_rev";
const IMPORTS_AT_REV: &str = "defect_imports_at_rev";

/// Uncapped per-sample band maps: for each of ≤12 sampled revs (the same
/// evenly-spaced history sampler `health_trend` uses, oldest-first, newest
/// always included), the file→band map at that point in history.
///
/// Unlike `health_trend::run_health_trend_detail`'s `file_series` (capped to
/// the top-50 hotspot paths, to keep the SPA JSON payload small), this scan
/// is UNCAPPED: [`validate`] needs a band for whichever file a mined defect
/// happened to touch, not just the busiest ones.
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on any query / ingest failure.
pub fn band_history<R: Repo>(
    db: &FactsDb,
    repo: &R,
    opts: &Options,
) -> Result<Vec<(String, HashMap<String, String>)>> {
    let samples = sampled_commits(db)?;
    // ALL files must feed the scan — never the user's `--rows` cut.
    let scan_opts = opts.with_no_row_limit();

    let mut out = Vec::with_capacity(samples.len());
    for (rev, ts) in &samples {
        let date = ts.get(..10).unwrap_or(ts).to_string();

        let live = live_paths_at(db, ts)?;
        let graph = import_graph_from_live_paths(repo, rev, &live);
        ingest_complexity_at_rev(db, repo, rev, &live, CM_AT_REV)?;
        materialize_imports_at_rev(db, &graph.resolved_edges(), IMPORTS_AT_REV)?;

        let cx = HealthScanCtx {
            complexity_source: CM_AT_REV.to_string(),
            imports_source: IMPORTS_AT_REV.to_string(),
            history_cutoff: Some(ts.clone()),
            include_clones: false,
            clone_source: CloneSource::WorkingTree,
        };
        let code_rows = run_code_health_scoped(db, &scan_opts, &cx)?;
        let bands: HashMap<String, String> =
            code_rows.into_iter().map(|r| (r.path, r.band)).collect();
        out.push((date, bands));
    }
    Ok(out)
}

// ─── Unit C: validation report ───────────────────────────────────────────────

/// Canonical band-table row order — always emitted in this order, even at
/// zero count, so consumers can rely on exactly three rows.
const BAND_ORDER: [&str; 3] = ["red", "yellow", "green"];

/// The nearest band sample at-or-before `defect_date` (dates compared
/// LEXICOGRAPHICALLY — the same zero-padded, UTC-normalized date contract
/// `szz::link_defects` relies on). `bands` is assumed oldest-first — exactly
/// [`band_history`]'s own output order. Falls back to the EARLIEST sample
/// when `defect_date` predates every sample (the spec's "else earliest
/// sample"); `None` only when `bands` itself is empty.
fn band_at_defect<'a, H: std::hash::BuildHasher>(
    bands: &'a [(String, HashMap<String, String, H>)],
    defect_date: &str,
) -> Option<&'a HashMap<String, String, H>> {
    let mut nearest = None;
    for (date, map) in bands {
        if date.as_str() <= defect_date {
            nearest = Some(map);
        } else {
            break; // `bands` ascending: every later sample is too new too.
        }
    }
    nearest.or_else(|| bands.first().map(|(_, map)| map))
}

/// The band `link`'s defect-introducing commit occupied, at `link.path`, at
/// the nearest historical sample. `None` — counted in
/// [`ValidationMetrics::excluded_no_data`] by [`validate`] — when the defect
/// commit's date is unknown, no sample exists at all, or the file has no
/// health data at the chosen sample (spec: "files without complexity data at
/// the sample are excluded and counted").
fn band_for_link<'a>(
    link: &SzzLink,
    commit_dates: &HashMap<String, String, impl std::hash::BuildHasher>,
    bands: &'a [(String, HashMap<String, String, impl std::hash::BuildHasher>)],
) -> Option<&'a str> {
    let defect_date = commit_dates.get(&link.defect_rev)?;
    let band_map = band_at_defect(bands, defect_date)?;
    band_map.get(&link.path).map(String::as_str)
}

/// Band table + AUC + precision@k against defect labels.
///
/// The label set is every file touched by at least one link (`link.path`,
/// deduplicated) — "defect-implicated" in the mined window. The band table
/// tallies each link individually against the nearest historical band
/// sample at-or-before its defect commit's date (see [`band_for_link`]);
/// AUC and precision@k score HEAD's `structural_risk` against that same
/// label set directly — independent of band-sample resolution, since
/// `head_health` always carries current data for every scored file.
#[must_use]
pub fn validate(
    links: &[SzzLink],
    commit_dates: &HashMap<String, String, impl std::hash::BuildHasher>,
    bands: &[(String, HashMap<String, String, impl std::hash::BuildHasher>)],
    head_health: &[CodeHealthRow],
) -> ValidationMetrics {
    let labels: HashSet<&str> = links.iter().map(|l| l.path.as_str()).collect();
    let linked_defects: HashSet<&str> = links.iter().map(|l| l.defect_rev.as_str()).collect();

    let mut band_counts: HashMap<&str, u32> = HashMap::new();
    let mut excluded_no_data = 0u32;
    for link in links {
        match band_for_link(link, commit_dates, bands) {
            Some(band) => *band_counts.entry(band).or_insert(0) += 1,
            None => excluded_no_data += 1,
        }
    }
    let total: u32 = band_counts.values().sum();
    let band_table = BAND_ORDER
        .iter()
        .map(|&band| {
            let count = band_counts.get(band).copied().unwrap_or(0);
            let share = if total > 0 {
                f64::from(count) / f64::from(total)
            } else {
                0.0
            };
            (band.to_string(), count, share)
        })
        .collect();

    let scored: Vec<(f64, bool)> = head_health
        .iter()
        .map(|r| (r.structural_risk, labels.contains(r.path.as_str())))
        .collect();
    let red_count = head_health.iter().filter(|r| r.band == "red").count();

    ValidationMetrics {
        band_table,
        auc_default: crate::stats::auc(&scored),
        precision_at_10: crate::stats::precision_at_k(&scored, 10),
        precision_at_red: crate::stats::precision_at_k(&scored, red_count),
        implicated_files: u32::try_from(labels.len()).unwrap_or(u32::MAX),
        linked_defects: u32::try_from(linked_defects.len()).unwrap_or(u32::MAX),
        sample_dates: bands.iter().map(|(date, _)| date.clone()).collect(),
        excluded_no_data,
    }
}

// ─── Unit D: intensity capture + the Rust-side risk formula ─────────────────

/// The [`SMELL_WEIGHTS`] defaults converted to owned `(name, weight)`
/// tuples, in `SMELL_WEIGHTS` order — the canonical `defaults` argument for
/// [`tune_weights`] and the reference weight vector
/// [`structural_risk_from_intensities`] is parity-tested against.
#[must_use]
pub fn default_weights() -> Vec<(String, f64)> {
    crate::analyses::code_health::default_smell_weights()
}

/// Capture the per-file, per-smell biomarker intensities [`tune_weights`]
/// needs to re-score candidate weight sets, without re-running the whole
/// code-health SQL pass for every candidate — see this module's design-
/// decision doc comment above.
///
/// Callers MUST call this immediately after a HEAD
/// [`run_code_health_scoped`] (or [`crate::analyses::code_health::run_code_health`])
/// call on the SAME `db` connection: `code_health_biomarkers_v1` is a
/// `CREATE OR REPLACE TEMPORARY TABLE` refreshed by every code-health scan,
/// so an intervening scoped/historical scan (e.g. [`band_history`]) would
/// silently replace it with a different revision's intensities.
///
/// Rows naming a smell absent from [`SMELL_WEIGHTS`] are defensively
/// skipped — this should never happen since both sides share the same
/// `SMELL_WEIGHTS` source of truth, but a stray row must never panic or
/// silently corrupt a different smell's slot.
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on a `FactsDb` query failure.
pub fn capture_intensities(db: &FactsDb) -> Result<HashMap<String, [f64; 8]>> {
    let rows: Vec<(String, String, f64)> = crate::analyses::query::query_map_collect(
        db,
        "SELECT path, smell, intensity FROM code_health_biomarkers_v1",
        [],
        "defect-calibration:capture-intensities",
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, f64>(2)?,
            ))
        },
    )?;

    let mut out: HashMap<String, [f64; 8]> = HashMap::new();
    for (path, smell, intensity) in rows {
        let Some(idx) = SMELL_WEIGHTS.iter().position(|&(name, _)| name == smell) else {
            continue; // Unknown smell name — defensive; should never happen.
        };
        out.entry(path).or_insert([0.0; 8])[idx] = intensity;
    }
    Ok(out)
}

/// Rust-side mirror of `code_health`'s SQL formula
/// `LEAST(1.0, SUM(intensity * weight))` — specifically the HEAD
/// (`include_clones = true`) path, which carries no renormalization divisor
/// (`STRUCTURAL_SCALE_NO_DRY` only applies when DRY is excluded, which never
/// happens at HEAD). `weights` must be in the same fixed order as
/// `intensities`'s 8 slots — [`default_weights`]'s order (`SMELL_WEIGHTS`
/// order) — for the parity to hold; see the parity test in
/// `tests/defect_calibration_test.rs`.
#[must_use]
pub fn structural_risk_from_intensities(weights: &[(String, f64)], intensities: &[f64; 8]) -> f64 {
    let raw: Vec<f64> = weights.iter().map(|(_, w)| *w).collect();
    risk_raw(&raw, intensities)
}

/// The allocation-light core of [`structural_risk_from_intensities`], used
/// directly by the coordinate-descent search below where weights are
/// already plain `f64`s (no names needed mid-search).
fn risk_raw(weights: &[f64], intensities: &[f64; 8]) -> f64 {
    let sum: f64 = weights
        .iter()
        .zip(intensities.iter())
        .map(|(w, i)| w * i)
        .sum();
    sum.min(1.0)
}

// ─── Unit D: constrained weight tuning ───────────────────────────────────────

/// Honesty floor: with fewer than this many linked defect-changes, the mined
/// evidence is too thin to trust a tuned weight set over the defaults, so
/// the defaults are kept regardless of what the search would find.
const MIN_LINKED_DEFECTS: usize = 30;
/// Honesty floor: with fewer than this many distinct implicated files, the
/// mined defects are too concentrated in a handful of paths to trust a
/// tuned weight set over the defaults, so the defaults are kept regardless
/// of what the search would find.
const MIN_IMPLICATED_FILES: usize = 10;
/// The validation-split AUC improvement a tuned weight set must clear over
/// the defaults' own validation AUC to be adopted — wide enough that a
/// modest AUC bump from noise or an unlucky split is never mistaken for a
/// genuine improvement.
const ACCEPTANCE_MARGIN: f64 = 0.02;

/// Absolute discrimination floor: tuned weights must rank defect-implicated
/// files at least as well as random (AUC 0.5) on the validation split, no
/// matter how large their margin over the defaults. Beating a
/// worse-than-random baseline is not evidence of predictive value, and
/// `tuned (applied)` reads as an endorsement — so a below-random tuning keeps
/// the defaults and records both AUCs for the reader to judge.
const DISCRIMINATION_FLOOR: f64 = 0.5;
/// Relative steps tried for each weight during coordinate descent —
/// `default × step`, bounded to ±50% of the default so a single step can
/// never swing a weight to an implausible extreme.
const STEPS: [f64; 5] = [0.5, 0.75, 1.0, 1.25, 1.5];
/// Coordinate-descent passes over all eight weights (spec: "2 passes").
const PASSES: usize = 2;

/// Project `weights` back onto the sum-to-1 simplex by dividing every entry
/// by their sum. A no-op when the sum is non-positive — cannot happen with
/// the non-negative `SMELL_WEIGHTS` defaults and non-negative relative
/// steps this search uses, but guarded rather than dividing by zero.
///
/// This renormalization is naive: it rescales every entry uniformly, so it
/// does NOT re-clamp any weight back inside [`STEPS`]'s ±50%-of-default
/// step band. The band only bounds the STEP taken at the moment of
/// acceptance, not the vector's final resting value — after several
/// accepted steps across several coordinates, repeated projection can push
/// an individual weight outside that band (each acceptance divides every
/// entry by a running sum that itself drifts as other coordinates move).
/// Readers relying on "every weight stays within ±50% of its default"
/// should not: only the per-step candidate does.
fn project_sum_to_one(weights: &mut [f64]) {
    let sum: f64 = weights.iter().sum();
    if sum > 0.0 {
        for w in weights.iter_mut() {
            *w /= sum;
        }
    }
}

/// AUC of `weights` scored against `labels`, via [`risk_raw`]. A `labels`
/// entry whose path has no captured intensity is skipped — no biomarker
/// data to score, and defaulting to some placeholder risk would silently
/// bias the ranking. `None` when the (skip-filtered) scored set ends up with
/// an empty positive or negative class — see [`crate::stats::auc`].
fn auc_for(
    weights: &[f64],
    labels: &[(String, bool)],
    intensities: &HashMap<String, [f64; 8], impl std::hash::BuildHasher>,
) -> Option<f64> {
    let scored: Vec<(f64, bool)> = labels
        .iter()
        .filter_map(|(path, label)| {
            intensities
                .get(path)
                .map(|ints| (risk_raw(weights, ints), *label))
        })
        .collect();
    crate::stats::auc(&scored)
}

/// Deterministic coordinate descent over the eight weights: [`PASSES`]
/// passes, each trying [`STEPS`] `× defaults[i]` for one weight at a time
/// (every other weight held at its CURRENT value), in `SMELL_WEIGHTS` order.
/// A step is accepted only when it strictly improves the training AUC over
/// the current weights (ties keep the earlier, lower-index step); an
/// acceptance immediately projects the whole vector back to sum-to-1 before
/// the search continues to the next weight. Returns `defaults` unchanged
/// (by value, not merely by AUC) if no step ever improves on it.
fn coordinate_descent(
    defaults: &[f64],
    train: &[(String, bool)],
    intensities: &HashMap<String, [f64; 8], impl std::hash::BuildHasher>,
) -> Vec<f64> {
    let mut current = defaults.to_vec();
    let mut current_objective = auc_for(&current, train, intensities).unwrap_or(0.0);

    for _pass in 0..PASSES {
        for i in 0..defaults.len() {
            let mut best: Option<(f64, f64)> = None; // (objective, trial weight)
            for &step in &STEPS {
                let mut trial = current.clone();
                trial[i] = defaults[i] * step;
                let objective = auc_for(&trial, train, intensities).unwrap_or(0.0);
                let improves_on_best = match best {
                    None => true,
                    Some((best_objective, _)) => objective > best_objective,
                };
                if improves_on_best {
                    best = Some((objective, trial[i]));
                }
            }
            if let Some((objective, weight)) = best
                && objective > current_objective
            {
                current[i] = weight;
                project_sum_to_one(&mut current);
                current_objective =
                    auc_for(&current, train, intensities).unwrap_or(current_objective);
            }
        }
    }
    current
}

/// Total positively-labeled rows across `train` and `validation` combined —
/// the honesty floor's "linked defect-changes" count. Rows are NOT deduplicated by
/// path here: `train`/`validation` are expected to carry one row per
/// (defect, file) incidence (the temporal split's caller-side construction),
/// so a file touched by several distinct defects contributes several rows —
/// see [`implicated_file_count`] for the distinct-file count.
fn linked_defect_count(train: &[(String, bool)], validation: &[(String, bool)]) -> usize {
    train
        .iter()
        .chain(validation)
        .filter(|(_, label)| *label)
        .count()
}

/// Distinct positively-labeled paths across `train` and `validation`
/// combined — the honesty floor's "implicated files" count.
fn implicated_file_count(train: &[(String, bool)], validation: &[(String, bool)]) -> usize {
    train
        .iter()
        .chain(validation)
        .filter(|(_, label)| *label)
        .map(|(path, _)| path.as_str())
        .collect::<HashSet<_>>()
        .len()
}

/// Constrained deterministic coordinate search over the eight
/// `SMELL_WEIGHTS` (Unit D). `intensities` are per-file 8-smell intensity
/// vectors in `SMELL_WEIGHTS` order (see [`capture_intensities`]);
/// `train`/`validation` are `(path, label)` splits already partitioned by
/// fix date (60/40, older/newer — a leakage guard against a random split,
/// prepared by the caller); `defaults` are the weights to fall back to and
/// the search's starting point (see [`default_weights`]).
///
/// Honesty floor first: fewer than [`MIN_LINKED_DEFECTS`] total
/// positively-labeled rows, or fewer than [`MIN_IMPLICATED_FILES`] distinct
/// positively-labeled paths, across `train` ∪ `validation`, keeps the
/// defaults without running any search — the mined evidence is too thin to
/// trust a tuned weight set.
/// Past the floor, [`coordinate_descent`] searches `train` for the
/// training-AUC-maximizing weights; the result is adopted only if its
/// `validation` AUC clears [`DISCRIMINATION_FLOOR`] (at least random-level
/// ranking on unseen recent defects) AND beats the defaults' own
/// `validation` AUC by at least [`ACCEPTANCE_MARGIN`] — otherwise the
/// defaults are kept, with the reason recorded and both AUCs shown.
///
/// Returns the chosen weights (tuned when adopted, else `defaults`
/// unchanged) plus the [`TuningDecision`] recording which branch fired.
#[must_use]
pub fn tune_weights(
    intensities: &HashMap<String, [f64; 8], impl std::hash::BuildHasher>,
    train: &[(String, bool)],
    validation: &[(String, bool)],
    defaults: &[(String, f64)],
) -> (Vec<(String, f64)>, TuningDecision) {
    let defaults_vec = defaults.to_vec();
    let kept = |reason: &str,
                auc_validation_default: Option<f64>,
                auc_validation_tuned: Option<f64>|
     -> (Vec<(String, f64)>, TuningDecision) {
        (
            defaults_vec.clone(),
            TuningDecision::DefaultsKept {
                reason: reason.to_string(),
                auc_validation_default,
                auc_validation_tuned,
            },
        )
    };

    if linked_defect_count(train, validation) < MIN_LINKED_DEFECTS {
        return kept("fewer than 30 linked defect-changes", None, None);
    }
    if implicated_file_count(train, validation) < MIN_IMPLICATED_FILES {
        return kept("fewer than 10 implicated files", None, None);
    }

    let default_raw: Vec<f64> = defaults.iter().map(|(_, w)| *w).collect();
    let Some(auc_validation_default) = auc_for(&default_raw, validation, intensities) else {
        return kept(
            "validation split has no positive/negative class to score",
            None,
            None,
        );
    };

    let tuned_raw = coordinate_descent(&default_raw, train, intensities);
    let auc_train = auc_for(&tuned_raw, train, intensities).unwrap_or(auc_validation_default);
    let auc_validation_tuned =
        auc_for(&tuned_raw, validation, intensities).unwrap_or(auc_validation_default);

    if auc_validation_tuned < DISCRIMINATION_FLOOR {
        return kept(
            "tuned weights rank below random on the validation split",
            Some(auc_validation_default),
            Some(auc_validation_tuned),
        );
    }
    if auc_validation_tuned >= auc_validation_default + ACCEPTANCE_MARGIN {
        let weights: Vec<(String, f64)> = defaults
            .iter()
            .zip(&tuned_raw)
            .map(|((name, _), &w)| (name.clone(), w))
            .collect();
        (
            weights,
            TuningDecision::Applied {
                auc_train,
                auc_validation_default,
                auc_validation_tuned,
            },
        )
    } else {
        kept(
            "tuned weights did not beat the default validation AUC by the required margin",
            Some(auc_validation_default),
            Some(auc_validation_tuned),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn health_row(path: &str, structural_risk: f64, band: &str) -> CodeHealthRow {
        CodeHealthRow {
            path: path.to_string(),
            cognitive: 0.0,
            score: 100.0 * (1.0 - structural_risk),
            structural_risk,
            percentile: 0.0,
            band: band.to_string(),
            corpus_percentile: None,
            beyond_corpus: false,
            corpus_percentile_ci_low: None,
            corpus_percentile_ci_high: None,
        }
    }

    // ─── validate(): hand-computed band table + AUC ─────────────────────────

    #[test]
    fn validate_hand_computed_band_table_and_auc() {
        let links = vec![
            SzzLink {
                defect_rev: "d1".to_string(),
                fix_rev: "f1".to_string(),
                path: "a.rs".to_string(),
            },
            SzzLink {
                defect_rev: "d2".to_string(),
                fix_rev: "f2".to_string(),
                path: "b.rs".to_string(),
            },
        ];
        let commit_dates = HashMap::from([
            ("d1".to_string(), "2026-01-05".to_string()),
            ("d2".to_string(), "2026-03-01".to_string()),
        ]);
        // Oldest-first, as band_history emits. d1 (2026-01-05) falls between
        // the two samples -> nearest at-or-before is 2026-01-01, where a.rs
        // is red. d2 (2026-03-01) is at-or-after both samples -> nearest is
        // the latest, 2026-02-01, where b.rs is green.
        let bands = vec![
            (
                "2026-01-01".to_string(),
                HashMap::from([
                    ("a.rs".to_string(), "red".to_string()),
                    ("b.rs".to_string(), "green".to_string()),
                ]),
            ),
            (
                "2026-02-01".to_string(),
                HashMap::from([
                    ("a.rs".to_string(), "yellow".to_string()),
                    ("b.rs".to_string(), "green".to_string()),
                ]),
            ),
        ];
        let head_health = vec![
            health_row("a.rs", 0.9, "red"),
            health_row("b.rs", 0.2, "green"),
            health_row("c.rs", 0.5, "yellow"),
        ];

        let metrics = validate(&links, &commit_dates, &bands, &head_health);

        assert_eq!(
            metrics.band_table,
            vec![
                ("red".to_string(), 1, 0.5),
                ("yellow".to_string(), 0, 0.0),
                ("green".to_string(), 1, 0.5),
            ]
        );
        assert_eq!(metrics.implicated_files, 2);
        assert_eq!(metrics.linked_defects, 2);
        assert_eq!(metrics.excluded_no_data, 0);
        assert_eq!(
            metrics.sample_dates,
            vec!["2026-01-01".to_string(), "2026-02-01".to_string()]
        );
        // scored = [(0.9,true), (0.2,true), (0.5,false)]; hand-derived AUC:
        // ranks ascending 0.2(1),0.5(2),0.9(3); rank_sum_pos = 1+3=4;
        // u = 4 - 2*3/2 = 1; auc = 1/(2*1) = 0.5.
        assert_eq!(metrics.auc_default, Some(0.5));
        // k=10 > 3 scored rows -> None.
        assert_eq!(metrics.precision_at_10, None);
        // One red file (a.rs); top-1 by score is a.rs (true) -> precision 1.0.
        assert_eq!(metrics.precision_at_red, Some(1.0));
    }

    #[test]
    fn validate_falls_back_to_earliest_sample_when_defect_predates_all_samples() {
        let links = vec![SzzLink {
            defect_rev: "d1".to_string(),
            fix_rev: "f1".to_string(),
            path: "a.rs".to_string(),
        }];
        let commit_dates = HashMap::from([("d1".to_string(), "2020-01-01".to_string())]);
        let bands = vec![(
            "2026-01-01".to_string(),
            HashMap::from([("a.rs".to_string(), "red".to_string())]),
        )];
        let head_health = vec![health_row("a.rs", 0.9, "red")];

        let metrics = validate(&links, &commit_dates, &bands, &head_health);
        assert_eq!(metrics.excluded_no_data, 0);
        assert_eq!(
            metrics.band_table,
            vec![
                ("red".to_string(), 1, 1.0),
                ("yellow".to_string(), 0, 0.0),
                ("green".to_string(), 0, 0.0),
            ]
        );
    }

    #[test]
    fn validate_excludes_links_with_no_band_data_at_the_chosen_sample() {
        let links = vec![SzzLink {
            defect_rev: "d1".to_string(),
            fix_rev: "f1".to_string(),
            path: "missing.rs".to_string(),
        }];
        let commit_dates = HashMap::from([("d1".to_string(), "2026-01-05".to_string())]);
        // Sample exists, but does not carry a band for "missing.rs".
        let bands = vec![(
            "2026-01-01".to_string(),
            HashMap::from([("a.rs".to_string(), "red".to_string())]),
        )];
        let head_health = vec![health_row("a.rs", 0.9, "red")];

        let metrics = validate(&links, &commit_dates, &bands, &head_health);
        assert_eq!(metrics.excluded_no_data, 1);
        assert_eq!(metrics.band_table.iter().map(|(_, n, _)| n).sum::<u32>(), 0);
    }

    #[test]
    fn validate_excludes_links_with_unknown_commit_date() {
        let links = vec![SzzLink {
            defect_rev: "unknown-rev".to_string(),
            fix_rev: "f1".to_string(),
            path: "a.rs".to_string(),
        }];
        let commit_dates: HashMap<String, String> = HashMap::new();
        let bands = vec![(
            "2026-01-01".to_string(),
            HashMap::from([("a.rs".to_string(), "red".to_string())]),
        )];
        let head_health = vec![health_row("a.rs", 0.9, "red")];

        let metrics = validate(&links, &commit_dates, &bands, &head_health);
        assert_eq!(metrics.excluded_no_data, 1);
        // The label set is still every linked path, regardless of whether
        // the band lookup for the band table succeeded.
        assert_eq!(metrics.implicated_files, 1);
        assert_eq!(metrics.auc_default, None); // single-class scored set.
    }

    // ─── default_weights / structural_risk_from_intensities sanity ─────────

    #[test]
    fn default_weights_matches_smell_weights_order_and_sums_to_one() {
        let weights = default_weights();
        assert_eq!(weights.len(), 8);
        for (got, &(name, weight)) in weights.iter().zip(SMELL_WEIGHTS.iter()) {
            assert_eq!(got.0, name);
            assert!((got.1 - weight).abs() < 1e-12);
        }
        let sum: f64 = weights.iter().map(|(_, w)| w).sum();
        assert!((sum - 1.0).abs() < 1e-9);
    }

    #[test]
    fn structural_risk_from_intensities_clamps_at_one() {
        let weights: Vec<(String, f64)> = vec![
            ("a".to_string(), 0.6),
            ("b".to_string(), 0.6),
            ("c".to_string(), 0.0),
            ("d".to_string(), 0.0),
            ("e".to_string(), 0.0),
            ("f".to_string(), 0.0),
            ("g".to_string(), 0.0),
            ("h".to_string(), 0.0),
        ];
        let intensities = [1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        // Raw sum 1.2 must clamp to 1.0.
        let risk = structural_risk_from_intensities(&weights, &intensities);
        assert!(
            (risk - 1.0).abs() < 1e-12,
            "expected clamp to 1.0, got {risk}"
        );
    }

    // ─── tune_weights(): honesty floor + acceptance branches ────────────────

    /// Build `n` distinct files (`{prefix}{i}.rs`) each with `intensity`,
    /// labeled `label`, appended to `dest`.
    fn seed_files(
        dest: &mut Vec<(String, bool)>,
        intensities: &mut HashMap<String, [f64; 8]>,
        prefix: &str,
        n: usize,
        label: bool,
        intensity: [f64; 8],
    ) {
        for i in 0..n {
            let path = format!("{prefix}{i}.rs");
            intensities.insert(path.clone(), intensity);
            dest.push((path, label));
        }
    }

    #[test]
    fn tune_weights_keeps_defaults_below_the_linked_defect_floor() {
        let mut intensities = HashMap::new();
        let mut train = Vec::new();
        let mut validation = Vec::new();
        // 20 linked defect-changes total (< 30), but >= 10 implicated files, so
        // only the defect-count floor should fire.
        seed_files(
            &mut train,
            &mut intensities,
            "pos",
            15,
            true,
            [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        );
        seed_files(
            &mut validation,
            &mut intensities,
            "posv",
            5,
            true,
            [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        );
        seed_files(&mut train, &mut intensities, "neg", 15, false, [0.0; 8]);

        let defaults = default_weights();
        let (weights, decision) = tune_weights(&intensities, &train, &validation, &defaults);
        assert_eq!(weights, defaults);
        match decision {
            TuningDecision::DefaultsKept {
                reason,
                auc_validation_default,
                auc_validation_tuned,
            } => {
                assert_eq!(reason, "fewer than 30 linked defect-changes");
                assert_eq!(auc_validation_default, None);
                assert_eq!(auc_validation_tuned, None);
            }
            TuningDecision::Applied { .. } => {
                panic!("expected DefaultsKept(floor), got Applied")
            }
        }
    }

    #[test]
    fn tune_weights_keeps_defaults_below_the_implicated_file_floor() {
        let mut intensities = HashMap::new();
        let mut train = Vec::new();
        let mut validation = Vec::new();
        // 30 linked-defect ROWS (>= 30) over only 5 distinct files (< 10):
        // each of 5 files repeats 6 times, as if hit by 6 separate defects.
        for rep in 0..6 {
            seed_files(
                &mut train,
                &mut intensities,
                &format!("dup{rep}_"),
                5,
                true,
                [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            );
        }
        // Re-key onto only 5 distinct paths (seed_files above made 30
        // distinct names; collapse by reusing 5 shared paths directly).
        train.clear();
        intensities.clear();
        let shared_paths = ["dup0.rs", "dup1.rs", "dup2.rs", "dup3.rs", "dup4.rs"];
        for &path in &shared_paths {
            intensities.insert(path.to_string(), [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        }
        for i in 0..30 {
            let path = shared_paths[i % shared_paths.len()];
            train.push((path.to_string(), true));
        }
        seed_files(
            &mut validation,
            &mut intensities,
            "neg",
            10,
            false,
            [0.0; 8],
        );

        let defaults = default_weights();
        let (weights, decision) = tune_weights(&intensities, &train, &validation, &defaults);
        assert_eq!(weights, defaults);
        match decision {
            TuningDecision::DefaultsKept {
                reason,
                auc_validation_default,
                auc_validation_tuned,
            } => {
                assert_eq!(reason, "fewer than 10 implicated files");
                assert_eq!(auc_validation_default, None);
                assert_eq!(auc_validation_tuned, None);
            }
            TuningDecision::Applied { .. } => {
                panic!("expected DefaultsKept(floor), got Applied")
            }
        }
    }

    #[test]
    fn tune_weights_keeps_defaults_when_margin_is_not_met() {
        let mut intensities = HashMap::new();
        let mut train = Vec::new();
        let mut validation = Vec::new();
        // Every file — positive or negative — carries the IDENTICAL uniform
        // intensity vector, so every weight vector ties every file's risk:
        // no candidate can ever beat the current objective, and the search
        // never accepts a step. Tuned == defaults, so validation AUC cannot
        // improve at all, let alone by the margin.
        let uniform = [0.5; 8];
        seed_files(&mut train, &mut intensities, "pos", 18, true, uniform);
        seed_files(&mut train, &mut intensities, "neg", 18, false, uniform);
        seed_files(&mut validation, &mut intensities, "posv", 12, true, uniform);
        seed_files(
            &mut validation,
            &mut intensities,
            "negv",
            12,
            false,
            uniform,
        );

        let defaults = default_weights();
        let (weights, decision) = tune_weights(&intensities, &train, &validation, &defaults);
        assert_eq!(weights, defaults);
        match decision {
            TuningDecision::DefaultsKept {
                reason,
                auc_validation_default,
                auc_validation_tuned,
            } => {
                assert_eq!(
                    reason,
                    "tuned weights did not beat the default validation AUC by the required margin"
                );
                // All-tied scores -> AUC 0.5 for both, so the "improvement"
                // is exactly zero — well short of the 0.02 margin.
                assert_eq!(auc_validation_default, Some(0.5));
                assert_eq!(auc_validation_tuned, Some(0.5));
            }
            TuningDecision::Applied { .. } => {
                panic!("expected DefaultsKept(margin), got Applied")
            }
        }
    }

    #[test]
    fn tune_weights_applies_when_a_smell_perfectly_separates_and_clears_the_margin() {
        let mut intensities = HashMap::new();
        let mut train = Vec::new();
        let mut validation = Vec::new();

        // Plain positives: only complex-method (index 0) fires.
        let positive = [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        // Confuser negatives: god-class + large-method (indices 1, 2) fire,
        // whose combined DEFAULT weight (0.18 + 0.12 = 0.30) outranks a
        // plain positive's default risk (0.22) — an inversion that default
        // weights get wrong.
        let confuser_negative = [0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let plain_negative = [0.0; 8];

        seed_files(&mut train, &mut intensities, "pos", 18, true, positive);
        seed_files(
            &mut validation,
            &mut intensities,
            "posv",
            12,
            true,
            positive,
        );
        seed_files(
            &mut train,
            &mut intensities,
            "confuser",
            9,
            false,
            confuser_negative,
        );
        seed_files(
            &mut validation,
            &mut intensities,
            "confuserv",
            6,
            false,
            confuser_negative,
        );
        seed_files(
            &mut train,
            &mut intensities,
            "neg",
            9,
            false,
            plain_negative,
        );
        seed_files(
            &mut validation,
            &mut intensities,
            "negv",
            6,
            false,
            plain_negative,
        );

        let defaults = default_weights();
        let (weights, decision) = tune_weights(&intensities, &train, &validation, &defaults);

        match decision {
            TuningDecision::Applied {
                auc_validation_default,
                auc_validation_tuned,
                ..
            } => {
                assert!(
                    auc_validation_tuned >= auc_validation_default + ACCEPTANCE_MARGIN,
                    "tuned {auc_validation_tuned} must clear default {auc_validation_default} \
                     by at least {ACCEPTANCE_MARGIN}"
                );
                // Confirm the mediocre default baseline this fixture is
                // built to produce (inversion against the confuser class).
                assert!(auc_validation_default < 0.9, "expected a real inversion");
            }
            TuningDecision::DefaultsKept { reason, .. } => {
                panic!("expected Applied, got DefaultsKept({reason})")
            }
        }

        // The search shifted weight toward complex-method (index 0), the
        // perfectly-separating smell.
        assert_ne!(weights, defaults);
        let tuned_complex_method = weights[0].1;
        let default_complex_method = defaults[0].1;
        assert!(
            tuned_complex_method > default_complex_method,
            "expected complex-method weight to increase: tuned={tuned_complex_method} \
             default={default_complex_method}"
        );
        // Every Applied outcome must still land on the sum-to-1 simplex —
        // a missing (or broken) `project_sum_to_one` call would silently
        // drift the weights off it.
        let sum: f64 = weights.iter().map(|(_, w)| w).sum();
        assert!(
            (sum - 1.0).abs() < 1e-9,
            "tuned weights must sum to 1.0, got {sum}"
        );
    }

    // ─── coordinate_descent(): stepping-base + train-vs-validation regressions ──

    /// Regression for a bug class the `Applied` test above cannot catch:
    /// that fixture's training AUC saturates at its maximum (1.0) after a
    /// single accepted step, so no later step can ever be accepted again —
    /// and with only one acceptance, `current` still equals `defaults` at
    /// the moment of stepping, making `defaults[i] * step` and a buggy
    /// `current[i] * step` produce the IDENTICAL output there.
    ///
    /// This fixture forces a SECOND acceptance after a projection has
    /// already rescaled `current` away from `defaults`, where the two bases
    /// genuinely diverge. Six files, `defaults = [0.5, 0.3, 0.2]`:
    /// one negative at `[0.5, 0.4, 0.0]`, two positives at `[0.0, 0.1,
    /// 1.0]`, three negatives at `[0.3, 0.5, 0.0]`. Hand-traced:
    ///
    /// - Default risks: 0.37 / 0.23 / 0.30 — both positives rank below
    ///   every negative, AUC 0.
    /// - Weight 0's step ×0.5 (= 0.25) lifts AUC to 0.75 (positives at
    ///   0.23 rise above the 0.225 negatives) and is accepted; projection
    ///   rescales to `[1/3, 0.4, 4/15]` (sum was 0.75).
    /// - Weight 1 must now step from its DEFAULT 0.3: ×0.5 = 0.15 reaches
    ///   AUC 1.0 → accepted → final projection lands on
    ///   `[4/9, 0.2, 16/45]`. A mutant stepping from the CURRENT 0.4
    ///   would accept 0.2 instead and land on `[5/12, 1/4, 1/3]` — off by
    ///   0.05 in the second slot, far beyond this test's 1e-9 tolerance.
    ///
    /// Every decisive AUC comparison above has a margin ≥ 1e-3 and every
    /// cross-class risk gap ≥ 5e-3, so the expectations are robust — not
    /// artifacts of floating-point tie-breaking.
    #[test]
    fn coordinate_descent_steps_from_the_default_weight_not_the_current_one() {
        let defaults = [0.5, 0.3, 0.2];
        let mut train = vec![("neg_a0".to_string(), false)];
        let mut intensities = HashMap::from([(
            "neg_a0".to_string(),
            [0.5, 0.4, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        )]);
        for i in 0..2 {
            let path = format!("pos{i}");
            intensities.insert(path.clone(), [0.0, 0.1, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
            train.push((path, true));
        }
        for i in 0..3 {
            let path = format!("neg_b{i}");
            intensities.insert(path.clone(), [0.3, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
            train.push((path, false));
        }

        let tuned = coordinate_descent(&defaults, &train, &intensities);

        let expected = [4.0 / 9.0, 0.2, 16.0 / 45.0];
        for (got, want) in tuned.iter().zip(expected.iter()) {
            assert!(
                (got - want).abs() < 1e-9,
                "coordinate_descent must step from the DEFAULT weight, not the current one \
                 — expected {expected:?}, got {tuned:?}"
            );
        }
    }

    /// A tuned weight set that only improves the TRAINING split's AUC — and
    /// not the VALIDATION split's — must never be adopted: that would be
    /// fitting noise in the split used to search, not evidence the tuned
    /// weights generalize. `train` reuses the perfectly-separating-plus-
    /// confuser structure from the `Applied` test above (so training AUC
    /// genuinely improves, verified independently below); `validation`
    /// gives every file — positive or negative — the SAME intensity vector,
    /// so its AUC is exactly 0.5 for ANY weight vector: the tuned weights
    /// that fit `train` so well cannot possibly show a validation-AUC gain
    /// here, by construction.
    #[test]
    fn tune_weights_rejects_a_tuned_set_that_only_improves_training_auc() {
        let mut intensities = HashMap::new();
        let mut train = Vec::new();
        let mut validation = Vec::new();

        let positive = [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let confuser_negative = [0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let plain_negative = [0.0; 8];
        seed_files(&mut train, &mut intensities, "pos", 18, true, positive);
        seed_files(
            &mut train,
            &mut intensities,
            "confuser",
            9,
            false,
            confuser_negative,
        );
        seed_files(
            &mut train,
            &mut intensities,
            "neg",
            9,
            false,
            plain_negative,
        );

        let uniform_validation = [0.0; 8];
        seed_files(
            &mut validation,
            &mut intensities,
            "posv",
            15,
            true,
            uniform_validation,
        );
        seed_files(
            &mut validation,
            &mut intensities,
            "negv",
            15,
            false,
            uniform_validation,
        );

        let defaults = default_weights();

        // Independently confirm this fixture really does give the search
        // training-AUC headroom past the acceptance margin — otherwise this
        // test would not be exercising the branch it claims to.
        let default_raw: Vec<f64> = defaults.iter().map(|(_, w)| *w).collect();
        let tuned_raw = coordinate_descent(&default_raw, &train, &intensities);
        let auc_train_default =
            auc_for(&default_raw, &train, &intensities).expect("train auc (default)");
        let auc_train_tuned = auc_for(&tuned_raw, &train, &intensities).expect("train auc (tuned)");
        assert!(
            auc_train_tuned >= auc_train_default + ACCEPTANCE_MARGIN,
            "fixture must give the search real training-AUC headroom to justify this test: \
             default={auc_train_default} tuned={auc_train_tuned}"
        );

        let (weights, decision) = tune_weights(&intensities, &train, &validation, &defaults);
        assert_eq!(
            weights, defaults,
            "a training-only AUC improvement must never be adopted over the defaults"
        );
        match decision {
            TuningDecision::DefaultsKept {
                reason,
                auc_validation_default,
                auc_validation_tuned,
            } => {
                assert_eq!(
                    reason,
                    "tuned weights did not beat the default validation AUC by the required margin"
                );
                // Both Some(0.5): the uniform validation vectors tie every
                // file's risk regardless of which weights score them.
                assert_eq!(auc_validation_default, Some(0.5));
                assert_eq!(auc_validation_tuned, Some(0.5));
            }
            TuningDecision::Applied { .. } => panic!(
                "expected DefaultsKept(margin): a training-only AUC gain must not be adopted"
            ),
        }
    }
    #[test]
    fn tune_weights_keeps_defaults_when_tuned_validation_auc_is_below_random() {
        let mut intensities = HashMap::new();
        let mut train = Vec::new();
        let mut validation = Vec::new();

        // Training: positives score a constant w7 (complex-conditional);
        // negatives score 0.5·w0 (complex-method). At the defaults the
        // negatives outrank the positives (0.11 > 0.07) and the ONLY
        // improving move is halving w0 (0.055 < 0.07) — the search tunes
        // exactly that weight and nothing else.
        seed_files(
            &mut train,
            &mut intensities,
            "tpos",
            30,
            true,
            [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0],
        );
        seed_files(
            &mut train,
            &mut intensities,
            "tneg",
            10,
            false,
            [0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        );

        // Validation: the positive scores w1 (god-class, untouched by
        // training). Six negatives sit on validation-only slots above it and
        // three below it at ANY reachable weight set (rank-invariant under
        // the sum-to-one projection); one negative keys on w0 and crosses
        // below the positive exactly when w0 drops. Default AUC = 3/10;
        // tuned AUC = 4/10 — clears the +0.02 margin yet ranks below random,
        // so the discrimination floor (not the margin rule) must fire.
        seed_files(
            &mut validation,
            &mut intensities,
            "vpos",
            1,
            true,
            [0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        );
        seed_files(
            &mut validation,
            &mut intensities,
            "vnegw0",
            1,
            false,
            [0.87, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        );
        seed_files(
            &mut validation,
            &mut intensities,
            "vneghi",
            6,
            false,
            [0.0, 0.0, 0.61, 0.0, 0.61, 0.61, 0.0, 0.0],
        );
        seed_files(
            &mut validation,
            &mut intensities,
            "vneglo",
            3,
            false,
            [0.0, 0.0, 0.2, 0.0, 0.2, 0.2, 0.0, 0.0],
        );

        let defaults = default_weights();
        let (weights, decision) = tune_weights(&intensities, &train, &validation, &defaults);
        assert_eq!(
            weights, defaults,
            "a below-random tuning must never replace the defaults"
        );
        match decision {
            TuningDecision::DefaultsKept {
                reason,
                auc_validation_default,
                auc_validation_tuned,
            } => {
                assert!(
                    reason.contains("below random"),
                    "the discrimination floor, not another branch, must fire: {reason}"
                );
                let d = auc_validation_default.expect("default validation AUC recorded");
                let t = auc_validation_tuned.expect("tuned validation AUC recorded");
                assert!((d - 0.30).abs() < 1e-9, "default validation AUC, got {d}");
                assert!((t - 0.40).abs() < 1e-9, "tuned validation AUC, got {t}");
            }
            other @ TuningDecision::Applied { .. } => {
                panic!("expected DefaultsKept via the discrimination floor, got {other:?}")
            }
        }
    }
}

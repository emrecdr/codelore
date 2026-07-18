//! `defect-validation` analysis — surfaces a defect-calibration artifact's
//! own evidence as flat, human-scannable `(metric, value)` rows.
//!
//! Unlike every other registered analysis, this one reads NOTHING from the
//! fact store: all of its numbers were already computed at mining time and
//! frozen into a `defects.calib.json` artifact (see
//! [`crate::defect_calibration`]). It resolves that artifact from
//! `opts.defect_calibration`, applies the SAME repo-identity guard the weight
//! application uses (`check_repo_identity`, honouring
//! `--allow-foreign-calibration`), and flattens its
//! [`ValidationMetrics`](crate::defect_calibration::ValidationMetrics) +
//! [`TuningDecision`](crate::defect_calibration::TuningDecision) +
//! [`MiningStats`](crate::defect_calibration::MiningStats) into rows.
//!
//! Presentation follows the project's honesty framing: the band table shows
//! where mined defects landed relative to code-health bands, but this is an
//! **association, not causation** — a red file that a defect-introducing
//! commit touched is evidence the score ranks it high, not proof the score
//! caused the defect. Every count carries its `n`; every absent
//! `Option` metric renders as an explicit `n/a (<why>)` rather than being
//! silently dropped. The tuning-decision AUCs are always surfaced — including
//! the case where tuning was *applied* yet the validation AUCs sit below 0.5,
//! so a reader can judge the outcome for themselves.
//!
//! Without a configured artifact the analysis returns zero rows and prints a
//! one-line hint on stderr pointing at `codelore calibrate-defects` — an
//! honest absence, not an error.

use crate::defect_calibration::{self, DefectArtifact, TuningDecision};
use crate::{Options, Result};

/// One flattened evidence row from a defect-calibration artifact:
/// `(metric, value)`. The value is a string so counts, ratios, honest
/// `n/a (<why>)` placeholders, and textual verdicts (e.g. the weights
/// source) share one row shape — mirroring
/// [`ArchitectureMetricRow`](crate::analyses::architecture_metrics::ArchitectureMetricRow).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DefectValidationRow {
    pub metric: String,
    pub value: String,
}

/// Run the `defect-validation` analysis.
///
/// Reads the `--defect-calibration` artifact configured in `opts` and
/// flattens its evidence into rows. Returns zero rows (and a stderr hint)
/// when no artifact is configured — the honest-absence path.
///
/// Takes only `opts`, not a `FactsDb`: every number reported here was frozen
/// into the artifact at mining time, so the analysis never touches the fact
/// store.
///
/// # Errors
///
/// Propagates [`defect_calibration::load`] failures (missing / malformed /
/// version-mismatched artifact) and the [`defect_calibration::check_repo_identity`]
/// foreign-repo guard — an explicitly configured artifact that cannot be read
/// or does not belong to this repo is a configuration mistake, not a
/// degradable state.
#[tracing::instrument(name = "defect-validation", skip_all)]
pub fn run_defect_validation(opts: &Options) -> Result<Vec<DefectValidationRow>> {
    let Some(path) = opts.defect_calibration.as_deref() else {
        eprintln!(
            "defect-validation: no defect-calibration artifact configured. Run \
             `codelore calibrate-defects --repo . --output defects.calib.json` first, \
             then re-run with `--defect-calibration defects.calib.json`."
        );
        return Ok(Vec::new());
    };
    let art = defect_calibration::load(path)?;
    defect_calibration::check_repo_identity(&art, &opts.repo_path, opts.allow_foreign_calibration)?;
    Ok(rows_from_artifact(&art))
}

/// Flatten an artifact's evidence into presentation rows, in a fixed order:
/// provenance → mining tallies → validation counts → band table → ranking
/// metrics → tuning decision. Pure (no I/O), so the row shape is unit-tested
/// directly against synthetic artifacts.
fn rows_from_artifact(art: &DefectArtifact) -> Vec<DefectValidationRow> {
    let v = &art.validation;
    let m = &art.mining;

    // The straight-line rows, in presentation order: provenance, mining
    // tallies, then validation counts. The band-table and tuning-decision rows
    // are appended below since they branch on the artifact's contents.
    let mut rows = vec![
        // provenance
        row("vintage", art.vintage.clone()),
        row("generated_at", art.generated_at.clone()),
        row("head_at_mining", art.head_at_mining.clone()),
        // mining tallies (full tally set)
        row("fixes_found", m.fixes_found.to_string()),
        row("links_found", m.links_found.to_string()),
        row("files_blamed", m.files_blamed.to_string()),
        row("lines_considered", m.lines_considered.to_string()),
        row(
            "lines_dropped_cosmetic",
            m.lines_dropped_cosmetic.to_string(),
        ),
        row("pure_addition_fixes", m.pure_addition_fixes.to_string()),
        row("blame_failures", m.blame_failures.to_string()),
        // validation counts
        row("implicated_files", v.implicated_files.to_string()),
        row("linked_defects", v.linked_defects.to_string()),
        row("excluded_no_data", v.excluded_no_data.to_string()),
        row("band_samples", samples_summary(&v.sample_dates)),
    ];

    // ── headline band table: where mined defects landed, by band-at-the-time ─
    let total: u32 = v.band_table.iter().map(|(_, count, _)| count).sum();
    for (band, count, share) in &v.band_table {
        let value = if total == 0 {
            format!("{count} defect-changes (no defects had band data)")
        } else {
            format!("{count}/{total} defect-changes ({:.1}%)", share * 100.0)
        };
        rows.push(row(&format!("band:{band}"), value));
    }

    // ── ranking quality of HEAD structural_risk vs the defect labels ────────
    rows.push(row(
        "auc_default",
        opt_ratio(
            v.auc_default,
            "single class: every scored file shares one defect label",
        ),
    ));
    rows.push(row(
        "precision_at_10",
        match v.precision_at_10 {
            Some(p) => format!("{p:.3} (k=10)"),
            None => "n/a (fewer than 10 files scored at HEAD)".to_owned(),
        },
    ));
    rows.push(row(
        "precision_at_red",
        match v.precision_at_red {
            Some(p) => format!("{p:.3} (k = files red at HEAD)"),
            None => "n/a (no files red at HEAD)".to_owned(),
        },
    ));

    // ── tuning decision: source + both validation AUCs, always shown ────────
    let (source, auc_train, auc_val_default, auc_val_tuned, kept_reason) = match &art.tuning {
        TuningDecision::Applied {
            auc_train,
            auc_validation_default,
            auc_validation_tuned,
        } => (
            "tuned (applied)".to_owned(),
            Some(*auc_train),
            Some(*auc_validation_default),
            Some(*auc_validation_tuned),
            None,
        ),
        TuningDecision::DefaultsKept {
            reason,
            auc_validation_default,
            auc_validation_tuned,
        } => (
            format!("defaults kept: {reason}"),
            None,
            *auc_validation_default,
            *auc_validation_tuned,
            Some(reason.clone()),
        ),
    };
    rows.push(row("weights_source", source));
    rows.push(row(
        "tuning_auc_train",
        match auc_train {
            Some(x) => format!("{x:.3}"),
            None => "n/a (no training AUC recorded when defaults kept)".to_owned(),
        },
    ));
    rows.push(row(
        "tuning_auc_validation_default",
        opt_auc_kept(auc_val_default, kept_reason.as_deref()),
    ));
    rows.push(row(
        "tuning_auc_validation_tuned",
        opt_auc_kept(auc_val_tuned, kept_reason.as_deref()),
    ));

    rows
}

/// `Some(x)` → `x` to three decimals; `None` → an honest `n/a (<why>)`.
fn opt_ratio(value: Option<f64>, why: &str) -> String {
    match value {
        Some(x) => format!("{x:.3}"),
        None => format!("n/a ({why})"),
    }
}

/// A tuning-split AUC that is absent only when a sample-size honesty floor
/// short-circuited before any AUC was computed. Renders the floor reason so
/// the absence is self-explanatory.
fn opt_auc_kept(value: Option<f64>, kept_reason: Option<&str>) -> String {
    match value {
        Some(x) => format!("{x:.3}"),
        None => match kept_reason {
            Some(reason) => format!("n/a (defaults kept before scoring: {reason})"),
            None => "n/a".to_owned(),
        },
    }
}

/// `sample_dates` summarised as a count plus its span. Empty → an explicit
/// no-samples note so the row is never a bare `0`.
fn samples_summary(dates: &[String]) -> String {
    match (dates.first(), dates.last()) {
        (Some(first), Some(last)) if dates.len() > 1 => {
            format!("{} (from {first} to {last})", dates.len())
        }
        (Some(only), _) => format!("1 ({only})"),
        _ => "0 (no historical band samples)".to_owned(),
    }
}

fn row(metric: &str, value: String) -> DefectValidationRow {
    DefectValidationRow {
        metric: metric.to_owned(),
        value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defect_calibration::{
        DEFECT_FORMAT_VERSION, MiningStats, OracleConfig, ValidationMetrics,
    };

    /// Look up the single row for `metric`, panicking if absent — keeps the
    /// assertions below focused on the value while still pinning presence.
    fn value_of<'a>(rows: &'a [DefectValidationRow], metric: &str) -> &'a str {
        let row = rows
            .iter()
            .find(|r| r.metric == metric)
            .unwrap_or_else(|| panic!("row {metric:?} must be present; got {rows:?}"));
        row.value.as_str()
    }

    fn base_artifact(tuning: TuningDecision, validation: ValidationMetrics) -> DefectArtifact {
        DefectArtifact {
            format_version: DEFECT_FORMAT_VERSION,
            repo_identity: "abc".to_owned(),
            head_at_mining: "cafebabe".to_owned(),
            vintage: "defects-2026-07-16".to_owned(),
            generated_at: "2026-07-16T00:00:00Z".to_owned(),
            oracle: OracleConfig::default(),
            mining: MiningStats {
                fixes_found: 295,
                links_found: 812,
                files_blamed: 640,
                lines_considered: 9001,
                lines_dropped_cosmetic: 120,
                blame_failures: 7,
                pure_addition_fixes: 33,
            },
            validation,
            weights: crate::defect_calibration::validate::default_weights(),
            tuning,
        }
    }

    fn rich_validation() -> ValidationMetrics {
        ValidationMetrics {
            band_table: vec![
                ("red".to_owned(), 18, 0.6),
                ("yellow".to_owned(), 9, 0.3),
                ("green".to_owned(), 3, 0.1),
            ],
            auc_default: Some(0.712),
            precision_at_10: Some(0.5),
            precision_at_red: Some(0.65),
            implicated_files: 25,
            linked_defects: 30,
            sample_dates: vec!["2026-01-01".to_owned(), "2026-04-01".to_owned()],
            excluded_no_data: 2,
        }
    }

    #[test]
    fn applied_artifact_flattens_to_the_expected_rows() {
        let art = base_artifact(
            TuningDecision::Applied {
                auc_train: 0.80,
                auc_validation_default: 0.71,
                auc_validation_tuned: 0.74,
            },
            rich_validation(),
        );
        let rows = rows_from_artifact(&art);

        assert_eq!(value_of(&rows, "vintage"), "defects-2026-07-16");
        assert_eq!(value_of(&rows, "head_at_mining"), "cafebabe");
        assert_eq!(value_of(&rows, "fixes_found"), "295");
        assert_eq!(value_of(&rows, "links_found"), "812");
        assert_eq!(value_of(&rows, "files_blamed"), "640");
        assert_eq!(value_of(&rows, "lines_considered"), "9001");
        assert_eq!(value_of(&rows, "lines_dropped_cosmetic"), "120");
        assert_eq!(value_of(&rows, "pure_addition_fixes"), "33");
        assert_eq!(value_of(&rows, "blame_failures"), "7");
        assert_eq!(value_of(&rows, "implicated_files"), "25");
        assert_eq!(value_of(&rows, "linked_defects"), "30");
        assert_eq!(value_of(&rows, "excluded_no_data"), "2");
        assert_eq!(
            value_of(&rows, "band_samples"),
            "2 (from 2026-01-01 to 2026-04-01)"
        );
        // Band table: count/total and the share as a percentage.
        assert_eq!(value_of(&rows, "band:red"), "18/30 defect-changes (60.0%)");
        assert_eq!(
            value_of(&rows, "band:yellow"),
            "9/30 defect-changes (30.0%)"
        );
        assert_eq!(value_of(&rows, "band:green"), "3/30 defect-changes (10.0%)");
        assert_eq!(value_of(&rows, "auc_default"), "0.712");
        assert_eq!(value_of(&rows, "precision_at_10"), "0.500 (k=10)");
        assert_eq!(
            value_of(&rows, "precision_at_red"),
            "0.650 (k = files red at HEAD)"
        );
        assert_eq!(value_of(&rows, "weights_source"), "tuned (applied)");
        assert_eq!(value_of(&rows, "tuning_auc_train"), "0.800");
        assert_eq!(value_of(&rows, "tuning_auc_validation_default"), "0.710");
        assert_eq!(value_of(&rows, "tuning_auc_validation_tuned"), "0.740");
    }

    #[test]
    fn applied_with_sub_half_validation_aucs_surfaces_both_plainly() {
        // The product fact this analysis must never bury: tuning can be
        // *applied* while both validation AUCs sit below 0.5. Both are shown
        // as plain numbers so the reader can judge.
        let art = base_artifact(
            TuningDecision::Applied {
                auc_train: 0.55,
                auc_validation_default: 0.41,
                auc_validation_tuned: 0.44,
            },
            rich_validation(),
        );
        let rows = rows_from_artifact(&art);
        assert_eq!(value_of(&rows, "weights_source"), "tuned (applied)");
        assert_eq!(value_of(&rows, "tuning_auc_validation_default"), "0.410");
        assert_eq!(value_of(&rows, "tuning_auc_validation_tuned"), "0.440");
    }

    #[test]
    fn defaults_kept_margin_case_shows_both_validation_aucs() {
        // Margin-unmet DefaultsKept: both validation AUCs are present and must
        // be shown; only the training AUC is honestly n/a.
        let art = base_artifact(
            TuningDecision::DefaultsKept {
                reason:
                    "tuned weights did not beat the default validation AUC by the required margin"
                        .to_owned(),
                auc_validation_default: Some(0.60),
                auc_validation_tuned: Some(0.58),
            },
            rich_validation(),
        );
        let rows = rows_from_artifact(&art);
        assert_eq!(
            value_of(&rows, "weights_source"),
            "defaults kept: tuned weights did not beat the default validation AUC by the required margin"
        );
        assert_eq!(
            value_of(&rows, "tuning_auc_train"),
            "n/a (no training AUC recorded when defaults kept)"
        );
        assert_eq!(value_of(&rows, "tuning_auc_validation_default"), "0.600");
        assert_eq!(value_of(&rows, "tuning_auc_validation_tuned"), "0.580");
    }

    #[test]
    fn defaults_kept_floor_case_renders_honest_na_with_reason() {
        // Sample-size floor short-circuited before any AUC existed: every
        // tuning AUC is n/a and the floor reason is carried into the value.
        let mut validation = rich_validation();
        validation.auc_default = None; // single-class thin evidence
        validation.precision_at_10 = None;
        validation.precision_at_red = None;
        validation.band_table = vec![
            ("red".to_owned(), 0, 0.0),
            ("yellow".to_owned(), 0, 0.0),
            ("green".to_owned(), 0, 0.0),
        ];
        validation.sample_dates = vec![];
        let art = base_artifact(
            TuningDecision::DefaultsKept {
                reason: "fewer than 30 linked defect-changes".to_owned(),
                auc_validation_default: None,
                auc_validation_tuned: None,
            },
            validation,
        );
        let rows = rows_from_artifact(&art);
        assert_eq!(
            value_of(&rows, "weights_source"),
            "defaults kept: fewer than 30 linked defect-changes"
        );
        assert_eq!(
            value_of(&rows, "auc_default"),
            "n/a (single class: every scored file shares one defect label)"
        );
        assert_eq!(
            value_of(&rows, "precision_at_10"),
            "n/a (fewer than 10 files scored at HEAD)"
        );
        assert_eq!(
            value_of(&rows, "precision_at_red"),
            "n/a (no files red at HEAD)"
        );
        assert_eq!(
            value_of(&rows, "band:red"),
            "0 defect-changes (no defects had band data)"
        );
        assert_eq!(
            value_of(&rows, "band_samples"),
            "0 (no historical band samples)"
        );
        assert_eq!(
            value_of(&rows, "tuning_auc_validation_default"),
            "n/a (defaults kept before scoring: fewer than 30 linked defect-changes)"
        );
        assert_eq!(
            value_of(&rows, "tuning_auc_validation_tuned"),
            "n/a (defaults kept before scoring: fewer than 30 linked defect-changes)"
        );
    }
}

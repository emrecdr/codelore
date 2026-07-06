//! `delta-health` — change-level health verdict for `codelore diff`.
//!
//! Judges the CHANGE, not the snapshot: each function added, removed, or
//! modified between base and head is classified low/medium/high risk from
//! absolute thresholds, given an outcome (good/neutral/bad) from its
//! before→after direction, and aggregated into a 0–100 ratio with an
//! explicit low-signal middle verdict. Snapshot scores are provably
//! insensitive to individual commits; this is the per-change complement.
//!
//! Thresholds are FIXED constants, not TOML-configurable: the gate cannot
//! be quietly loosened, and verdicts stay stable across PRs.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::facts::FactsDb;
use crate::{CodeLoreError, Result};

/// Function LOC at or above this is medium risk (SIG unit-size bands).
pub const LOC_MEDIUM_FROM: u32 = 31;
/// Function LOC at or above this is high risk (SIG bands / Large Method > 70).
pub const LOC_HIGH_FROM: u32 = 71;
/// Cyclomatic complexity at or above this is medium risk (SIG bands).
pub const CYCLOMATIC_MEDIUM_FROM: f64 = 6.0;
/// Cyclomatic complexity at or above this is high risk (SIG bands / CC > 10).
pub const CYCLOMATIC_HIGH_FROM: f64 = 11.0;
/// Ratio strictly below this ⇒ `degrading` verdict.
pub const RATIO_DEGRADING_BELOW: f64 = 40.0;
/// Ratio strictly above this ⇒ `improving` verdict.
pub const RATIO_IMPROVING_ABOVE: f64 = 70.0;
/// Good/bad weight multiplier for functions in base-red-band files.
pub const RED_FILE_WEIGHT_MULTIPLIER: f64 = 1.5;

/// Risk class of a single function. Derive order matters: `Low < Medium
/// < High` powers the improved/degraded direction comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskClass {
    Low,
    Medium,
    High,
}

impl RiskClass {
    /// Lowercase display form, matching the serde encoding.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// Outcome of one changed function within the scored change set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Good,
    Neutral,
    Bad,
}

impl Outcome {
    /// Lowercase display form, matching the serde encoding.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Good => "good",
            Self::Neutral => "neutral",
            Self::Bad => "bad",
        }
    }
}

/// Classify one function from its persisted metrics. Worst triggered
/// property wins; clone membership forces High (the copy/paste penalty —
/// AI-pasted duplicates cannot score low-risk).
#[must_use]
pub fn classify(loc: u32, cyclomatic: f64, clone_member: bool) -> RiskClass {
    if clone_member || loc >= LOC_HIGH_FROM || cyclomatic >= CYCLOMATIC_HIGH_FROM {
        return RiskClass::High;
    }
    if loc >= LOC_MEDIUM_FROM || cyclomatic >= CYCLOMATIC_MEDIUM_FROM {
        return RiskClass::Medium;
    }
    RiskClass::Low
}

/// Outcome matrix per the design: added = ∅→class, removed = class→∅.
/// Good — ends Low, strictly improves, or removes a High-risk function.
/// Bad — ends High or strictly degrades. Neutral — everything else.
#[must_use]
pub fn outcome_for(before: Option<RiskClass>, after: Option<RiskClass>) -> Outcome {
    match (before, after) {
        (None, Some(a)) => match a {
            RiskClass::Low => Outcome::Good,
            RiskClass::Medium => Outcome::Neutral,
            RiskClass::High => Outcome::Bad,
        },
        (Some(b), None) => {
            if b == RiskClass::High {
                Outcome::Good
            } else {
                Outcome::Neutral
            }
        }
        (Some(b), Some(a)) => {
            if a == RiskClass::Low || a < b {
                Outcome::Good
            } else if a == RiskClass::High || a > b {
                Outcome::Bad
            } else {
                Outcome::Neutral
            }
        }
        // A function neither present at base nor head is never a change
        // candidate; keep the match total without panicking.
        (None, None) => Outcome::Neutral,
    }
}

/// Verdict from the ratio. The middle band is deliberately labeled
/// `indeterminate` — the design's honest replacement for a binary cut.
#[must_use]
pub fn verdict_for(ratio: f64) -> &'static str {
    if ratio < RATIO_DEGRADING_BELOW {
        "degrading"
    } else if ratio > RATIO_IMPROVING_ABOVE {
        "improving"
    } else {
        "indeterminate"
    }
}

/// One function's persisted metrics at a single rev. Serialized into the
/// CLI's `--base-cache` JSON, so field changes need the same
/// back-compat care as the cache struct itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionMetricRow {
    pub path: String,
    pub name: String,
    pub loc: u32,
    pub cyclomatic: f64,
}

/// Extract per-function metric rows from an ingested fact store.
///
/// `complexity_metrics` also stores class- and file-level rows; the join
/// on `entities.kind` keeps only real functions/methods so nothing
/// file-shaped is ever classified as a changed function.
///
/// Persisted entity names embed the line span (`{fn}@{start}-{end}`), which
/// is not stable across revisions — editing a function, or any function
/// above it, shifts the span and would make an in-place edit read as
/// remove+add instead of modified. The name is stripped to its bare form so
/// base↔head pairing keys on a stable identity. Functions that share a bare
/// name within one file (e.g. same-named methods on different types)
/// collapse to a single worst-case row — a documented limitation, since the
/// line span is the only thing that told them apart and it is not stable.
///
/// # Errors
///
/// [`CodeLoreError::Analysis`] on SQL failures.
pub fn run_function_metrics(db: &FactsDb) -> Result<Vec<FunctionMetricRow>> {
    const SQL: &str = "
        SELECT
            cm.path,
            regexp_replace(cm.name, '@[0-9]+-[0-9]+$', '') AS fn_name,
            MAX(cm.loc) AS loc,
            MAX(CAST(cm.cyclomatic AS DOUBLE)) AS cyclomatic
        FROM complexity_metrics cm
        JOIN entities e ON e.path = cm.path AND e.name = cm.name
        WHERE e.kind IN ('function', 'method')
        GROUP BY cm.path, fn_name
        ORDER BY cm.path, fn_name";
    let mut stmt = db
        .conn()
        .prepare(SQL)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare delta-health metrics: {e}")))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(FunctionMetricRow {
                path: r.get::<_, String>(0)?,
                name: r.get::<_, String>(1)?,
                loc: r.get::<_, u32>(2)?,
                cyclomatic: r.get::<_, f64>(3)?,
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query delta-health metrics: {e}")))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("read delta-health metrics: {e}")))?;
    Ok(rows)
}

/// One changed function in the scored set. `weight` is the RAW LOC
/// weight; the red-file multiplier applies only inside the ratio so the
/// reported numbers stay physical (`in_red_file` tells the story).
#[derive(Debug, Clone, Serialize)]
pub struct DeltaFunctionRow {
    pub path: String,
    pub function: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<RiskClass>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<RiskClass>,
    pub outcome: Outcome,
    pub weight: f64,
    pub in_red_file: bool,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct DeltaHealthCounts {
    pub added: u32,
    pub modified: u32,
    pub removed: u32,
    /// Changed files with no analyzable functions at either rev
    /// (unsupported languages, config/docs). Surfaced so coverage gaps
    /// are visible instead of silently omitted.
    pub skipped: u32,
}

/// The `delta_health` section of a diff run. `ratio == None` ⟺
/// `verdict == "no-code-change"`.
#[derive(Debug, Clone, Serialize)]
pub struct DeltaHealthSection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ratio: Option<f64>,
    pub verdict: String,
    pub counts: DeltaHealthCounts,
    pub functions: Vec<DeltaFunctionRow>,
}

fn reasons_for(loc: u32, cyclomatic: f64, clone_member: bool) -> Vec<String> {
    let mut out = Vec::new();
    if clone_member {
        out.push("member of a clone group".to_string());
    }
    if loc >= LOC_HIGH_FROM {
        out.push(format!("loc {loc} \u{2265} {LOC_HIGH_FROM}"));
    } else if loc >= LOC_MEDIUM_FROM {
        out.push(format!("loc {loc} \u{2265} {LOC_MEDIUM_FROM}"));
    }
    if cyclomatic >= CYCLOMATIC_HIGH_FROM {
        out.push(format!(
            "cyclomatic {cyclomatic:.0} \u{2265} {CYCLOMATIC_HIGH_FROM:.0}"
        ));
    } else if cyclomatic >= CYCLOMATIC_MEDIUM_FROM {
        out.push(format!(
            "cyclomatic {cyclomatic:.0} \u{2265} {CYCLOMATIC_MEDIUM_FROM:.0}"
        ));
    }
    out
}

/// Pair base/head function rows for the PR's changed files, classify,
/// score, and produce the section. Pure — all inputs are plain rows/sets
/// so this is directly reusable by future MCP/feed consumers.
#[must_use]
pub fn compute_delta_health(
    base: &[FunctionMetricRow],
    head: &[FunctionMetricRow],
    pr_files: &HashSet<String, impl std::hash::BuildHasher>,
    head_clone_members: &HashSet<(String, String), impl std::hash::BuildHasher>,
    base_red_files: &HashSet<String, impl std::hash::BuildHasher>,
) -> DeltaHealthSection {
    let index = |rows: &[FunctionMetricRow]| -> HashMap<(String, String), FunctionMetricRow> {
        rows.iter()
            .filter(|r| pr_files.contains(&r.path))
            .map(|r| ((r.path.clone(), r.name.clone()), r.clone()))
            .collect()
    };
    let base_idx = index(base);
    let head_idx = index(head);

    let mut keys: Vec<(String, String)> = base_idx.keys().chain(head_idx.keys()).cloned().collect();
    keys.sort();
    keys.dedup();

    let mut counts = DeltaHealthCounts::default();
    let mut functions = Vec::new();
    let (mut good_w, mut neutral_w, mut bad_w) = (0.0_f64, 0.0_f64, 0.0_f64);

    for key in keys {
        let b = base_idx.get(&key);
        let h = head_idx.get(&key);
        // Identical rows are untouched functions — excluded entirely.
        if let (Some(b), Some(h)) = (b, h)
            && b == h
        {
            continue;
        }
        let clone_member = head_clone_members.contains(&key);
        let before = b.map(|r| classify(r.loc, r.cyclomatic, false));
        let after = h.map(|r| classify(r.loc, r.cyclomatic, clone_member));
        match (b.is_some(), h.is_some()) {
            (false, true) => counts.added += 1,
            (true, false) => counts.removed += 1,
            _ => counts.modified += 1,
        }
        let outcome = outcome_for(before, after);
        let weight = f64::from(h.or(b).map_or(0, |r| r.loc));
        let in_red_file = base_red_files.contains(&key.0);
        let mult = if in_red_file && outcome != Outcome::Neutral {
            RED_FILE_WEIGHT_MULTIPLIER
        } else {
            1.0
        };
        match outcome {
            Outcome::Good => good_w += weight * mult,
            Outcome::Neutral => neutral_w += weight,
            Outcome::Bad => bad_w += weight * mult,
        }
        let reasons = h.map_or_else(Vec::new, |r| reasons_for(r.loc, r.cyclomatic, clone_member));
        functions.push(DeltaFunctionRow {
            path: key.0,
            function: key.1,
            before,
            after,
            outcome,
            weight,
            in_red_file,
            reasons,
        });
    }

    // Changed files with no function rows at either rev.
    let covered: HashSet<&String> = base_idx
        .keys()
        .chain(head_idx.keys())
        .map(|k| &k.0)
        .collect();
    counts.skipped =
        u32::try_from(pr_files.iter().filter(|p| !covered.contains(p)).count()).unwrap_or(u32::MAX);

    let total = good_w + neutral_w + bad_w;
    if functions.is_empty() || total <= 0.0 {
        return DeltaHealthSection {
            ratio: None,
            verdict: "no-code-change".to_string(),
            counts,
            functions,
        };
    }
    let ratio = 100.0 * good_w / total;
    // A change set with no bad weight never reports `degrading`. A low
    // ratio can be driven purely by neutral changes — e.g. adding one
    // medium-sized function — which is a low-signal event, not a
    // regression. Reporting `degrading` there would block benign,
    // degradation-free PRs under the deny-degrading / min-ratio gates and
    // mislead the human-facing verdict; cap it at `indeterminate` instead.
    let verdict = if bad_w == 0.0 && ratio < RATIO_DEGRADING_BELOW {
        "indeterminate"
    } else {
        verdict_for(ratio)
    };
    DeltaHealthSection {
        ratio: Some(ratio),
        verdict: verdict.to_string(),
        counts,
        functions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_loc_boundaries() {
        assert_eq!(classify(30, 1.0, false), RiskClass::Low);
        assert_eq!(classify(31, 1.0, false), RiskClass::Medium);
        assert_eq!(classify(70, 1.0, false), RiskClass::Medium);
        assert_eq!(classify(71, 1.0, false), RiskClass::High);
    }

    #[test]
    fn classify_cyclomatic_boundaries() {
        assert_eq!(classify(10, 5.0, false), RiskClass::Low);
        assert_eq!(classify(10, 6.0, false), RiskClass::Medium);
        assert_eq!(classify(10, 10.0, false), RiskClass::Medium);
        assert_eq!(classify(10, 11.0, false), RiskClass::High);
    }

    #[test]
    fn classify_clone_membership_forces_high() {
        assert_eq!(classify(5, 1.0, true), RiskClass::High);
    }

    #[test]
    fn classify_worst_property_wins() {
        // Low LOC but high cyclomatic ⇒ High.
        assert_eq!(classify(10, 20.0, false), RiskClass::High);
        // High LOC but trivial cyclomatic ⇒ High.
        assert_eq!(classify(100, 1.0, false), RiskClass::High);
    }

    #[test]
    fn outcome_added() {
        assert_eq!(outcome_for(None, Some(RiskClass::Low)), Outcome::Good);
        assert_eq!(outcome_for(None, Some(RiskClass::Medium)), Outcome::Neutral);
        assert_eq!(outcome_for(None, Some(RiskClass::High)), Outcome::Bad);
    }

    #[test]
    fn outcome_removed() {
        assert_eq!(outcome_for(Some(RiskClass::High), None), Outcome::Good);
        assert_eq!(outcome_for(Some(RiskClass::Medium), None), Outcome::Neutral);
        assert_eq!(outcome_for(Some(RiskClass::Low), None), Outcome::Neutral);
    }

    #[test]
    fn outcome_modified_matrix() {
        use RiskClass::{High, Low, Medium};
        // Stayed low ⇒ good; improved ⇒ good (even High→Medium).
        assert_eq!(outcome_for(Some(Low), Some(Low)), Outcome::Good);
        assert_eq!(outcome_for(Some(High), Some(Medium)), Outcome::Good);
        assert_eq!(outcome_for(Some(Medium), Some(Low)), Outcome::Good);
        // Ends high or degrades ⇒ bad.
        assert_eq!(outcome_for(Some(High), Some(High)), Outcome::Bad);
        assert_eq!(outcome_for(Some(Low), Some(Medium)), Outcome::Bad);
        assert_eq!(outcome_for(Some(Medium), Some(High)), Outcome::Bad);
        // Stayed medium ⇒ neutral.
        assert_eq!(outcome_for(Some(Medium), Some(Medium)), Outcome::Neutral);
    }

    #[test]
    fn verdict_cut_points() {
        assert_eq!(verdict_for(39.9), "degrading");
        assert_eq!(verdict_for(40.0), "indeterminate");
        assert_eq!(verdict_for(70.0), "indeterminate");
        assert_eq!(verdict_for(70.1), "improving");
    }

    fn row(path: &str, name: &str, loc: u32, cyclo: f64) -> FunctionMetricRow {
        FunctionMetricRow {
            path: path.into(),
            name: name.into(),
            loc,
            cyclomatic: cyclo,
        }
    }

    fn files(paths: &[&str]) -> std::collections::HashSet<String> {
        paths.iter().map(|p| (*p).to_string()).collect()
    }

    fn no_clones() -> std::collections::HashSet<(String, String)> {
        std::collections::HashSet::new()
    }

    #[test]
    fn no_changed_functions_is_no_code_change() {
        let base = vec![row("a.rs", "f", 10, 1.0)];
        let head = vec![row("a.rs", "f", 10, 1.0)]; // identical ⇒ untouched
        let s = compute_delta_health(
            &base,
            &head,
            &files(&["a.rs", "README.md"]),
            &no_clones(),
            &files(&[]),
        );
        assert_eq!(s.verdict, "no-code-change");
        assert_eq!(s.ratio, None);
        assert!(s.functions.is_empty());
        // README.md changed but has no functions at either rev ⇒ skipped.
        assert_eq!(s.counts.skipped, 1);
    }

    #[test]
    fn added_high_risk_function_degrades() {
        let base: Vec<FunctionMetricRow> = vec![];
        let head = vec![row("a.rs", "monster", 120, 15.0)];
        let s = compute_delta_health(&base, &head, &files(&["a.rs"]), &no_clones(), &files(&[]));
        assert_eq!(s.counts.added, 1);
        assert_eq!(s.ratio, Some(0.0));
        assert_eq!(s.verdict, "degrading");
        assert_eq!(s.functions[0].outcome, Outcome::Bad);
        assert_eq!(s.functions[0].before, None);
        assert_eq!(s.functions[0].after, Some(RiskClass::High));
        assert!(!s.functions[0].reasons.is_empty());
    }

    #[test]
    fn functions_outside_pr_files_are_ignored() {
        let base = vec![row("other.rs", "f", 10, 1.0)];
        let head = vec![row("other.rs", "f", 200, 30.0)]; // differs, but not a PR file
        let s = compute_delta_health(&base, &head, &files(&["a.rs"]), &no_clones(), &files(&[]));
        assert_eq!(s.verdict, "no-code-change");
    }

    #[test]
    fn clone_member_added_function_is_bad_even_if_tiny() {
        let head = vec![row("a.rs", "pasted", 8, 1.0)];
        let clones: std::collections::HashSet<(String, String)> =
            [("a.rs".to_string(), "pasted".to_string())].into();
        let s = compute_delta_health(&[], &head, &files(&["a.rs"]), &clones, &files(&[]));
        assert_eq!(s.functions[0].after, Some(RiskClass::High));
        assert_eq!(s.functions[0].outcome, Outcome::Bad);
        assert!(
            s.functions[0].reasons.iter().any(|r| r.contains("clone")),
            "reasons: {:?}",
            s.functions[0].reasons
        );
    }

    #[test]
    fn red_file_multiplier_amplifies_good_and_bad_not_neutral() {
        // One good (head 10 LOC) + one bad (head 100 LOC) + one neutral
        // (head 41 LOC) change; weights use head LOC.
        let base = vec![
            row("red.rs", "improved", 80, 1.0), // High → Low = good
            row("red.rs", "worsened", 10, 1.0), // Low → High = bad
            row("red.rs", "meh", 40, 1.0),      // Medium → Medium = neutral
        ];
        let head = vec![
            row("red.rs", "improved", 10, 1.0),
            row("red.rs", "worsened", 100, 1.0),
            row("red.rs", "meh", 41, 1.0),
        ];
        let red = files(&["red.rs"]);
        let s = compute_delta_health(&base, &head, &files(&["red.rs"]), &no_clones(), &red);
        // good_w = 10*1.5, bad_w = 100*1.5, neutral_w = 41 (unmodulated).
        // ratio = 100 * 15 / (15 + 150 + 41)
        let expected = 100.0 * 15.0 / (15.0 + 150.0 + 41.0);
        let got = s.ratio.expect("ratio");
        assert!(
            (got - expected).abs() < 1e-9,
            "got {got}, expected {expected}"
        );
        assert!(s.functions.iter().all(|f| f.in_red_file));
    }

    #[test]
    fn removed_high_risk_function_counts_as_good_with_base_weight() {
        let base = vec![row("a.rs", "monster", 120, 15.0)];
        let s = compute_delta_health(&base, &[], &files(&["a.rs"]), &no_clones(), &files(&[]));
        assert_eq!(s.counts.removed, 1);
        assert_eq!(s.ratio, Some(100.0));
        assert_eq!(s.verdict, "improving");
        assert!((s.functions[0].weight - 120.0).abs() < f64::EPSILON);
    }

    #[test]
    fn all_neutral_change_is_indeterminate_not_degrading() {
        // Adding a single medium-risk function (LOC 31-70, cyclomatic < 11)
        // is a Neutral outcome, so good_w = bad_w = 0 and the ratio is 0.0.
        // With no bad weight the change degraded nothing, so it must read as
        // `indeterminate` (low signal), never `degrading` — otherwise the
        // deny-degrading / min-ratio gates would block a benign PR.
        let head = vec![row("a.rs", "added_medium", 50, 1.0)];
        let s = compute_delta_health(&[], &head, &files(&["a.rs"]), &no_clones(), &files(&[]));
        assert_eq!(s.counts.added, 1);
        assert_eq!(s.functions[0].outcome, Outcome::Neutral);
        assert_eq!(s.ratio, Some(0.0));
        assert_eq!(
            s.verdict, "indeterminate",
            "all-neutral change must not be labeled degrading"
        );
    }
}

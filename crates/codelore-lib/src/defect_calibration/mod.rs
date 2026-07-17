//! Own-repo defect calibration: the fix-commit oracle, the AG-SZZ linkage
//! engine, and the `defects.calib.json` artifact model.
//!
//! This module answers *does the health score actually predict where defects
//! land in THIS repository?* by mining a repo's own fix history (AG-SZZ),
//! validating code-health predictions against it, and — when the evidence
//! clears an honesty floor — tuning the eight smell weights. Everything here
//! is opt-in and vintage-stamped so default behavior stays byte-reproducible
//! without the feature.
//!
//! # Unit A — the fix-commit oracle
//!
//! [`DefectOracle`] is a dedicated, pure fix-commit classifier — deliberately
//! **separate from** the kamei `fix` regex (`kamei::enrich_fix`), which stays
//! untouched: it is a JIT-SDP feature input whose broad `issue|error|patch`
//! alternation is a documented SZZ precision trap. This oracle uses a
//! narrower, word-boundary-anchored vocabulary intended specifically for
//! linking fixes to the defects they resolve.
//!
//! # Unit B — the AG-SZZ linkage engine
//!
//! [`szz`] traces each fix commit's deleted pre-image lines back to the
//! commit that last introduced them, behind a pluggable
//! [`szz::LineOriginSource`] seam — the roadmap's "pluggable SZZ". This
//! module has no production git-subprocess implementation; that lives
//! CLI-side, shelling `git blame --porcelain` and parsing it with
//! [`szz::parse_blame_porcelain`].
//!
//! # Unit C — the historical band scan + validation report
//!
//! [`validate::band_history`] recomputes code-health bands at ≤12 evenly
//! spaced historical revisions (the same at-rev machinery `health_trend`
//! uses, but with full path coverage — no top-50 cap). [`validate::validate`]
//! matches each defect-introducing commit to the nearest band sample
//! at-or-before its date and reports the headline band table plus AUC /
//! precision@k of HEAD's `structural_risk` against the defect-implicated
//! file labels.
//!
//! # Unit D — constrained weight tuning
//!
//! [`validate::tune_weights`] runs a deterministic coordinate-descent search
//! over the eight smell weights and only adopts a tuned set when the
//! evidence clears the honesty floor and the acceptance margin — see that
//! function's rustdoc for the full design decision (biomarker-intensity
//! capture + the Rust-side risk-scoring formula it re-scores candidates
//! with).
//!
//! # Unit E — the artifact
//!
//! [`DefectArtifact`] is a versioned, compact JSON container (mirroring
//! `calibration::CalibrationArtifact`'s serde style and write idiom). It
//! records the oracle configuration used, mining stats, validation metrics,
//! the (possibly tuned) smell weights, and the tuning decision. [`save`] /
//! [`load`] round-trip it; [`check_repo_identity`] guards against applying an
//! artifact mined from a different repository.

use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{CodeLoreError, Result};

pub mod szz;
pub mod validate;

/// Artifact schema version. A [`load`]ed artifact whose `format_version`
/// differs is rejected with a hard error — an explicitly passed
/// `--defect-calibration` file that cannot be used is a configuration
/// mistake, not a degradable state (mirrors
/// `calibration::CALIBRATION_FORMAT_VERSION`).
pub const DEFECT_FORMAT_VERSION: u32 = 1;

// ─── Unit A: fix-commit oracle ───────────────────────────────────────────────

/// Configuration for [`DefectOracle`]. `extra_patterns` are additional regexes
/// OR'd in alongside the built-in classifiers — for teams with tracker-id
/// conventions (e.g. `"JIRA-\\d+"`). Defaults to no extra patterns.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OracleConfig {
    pub extra_patterns: Vec<String>,
}

/// A pure fix-commit classifier over a commit message.
///
/// A commit is a fix iff its message matches, case-insensitively, either:
/// - a conventional-commit prefix `fix:` / `fix(scope):` anchored to the
///   start of the message, or
/// - a defect-vocabulary term at word boundaries: `bug`, `fix`/`fixes`/
///   `fixed`, `defect`, `regression`, `hotfix` are matched as whole words
///   (`\b…\b` on both sides — `fixture`, `hotfixture`, `defective`, `affix`
///   never count), while `bugfix` alone is leading-boundary-only so its
///   plural compound `bugfixes` still counts,
///
/// AND the caller-supplied `is_merge` flag is false, AND the message does not
/// begin with the literal git revert prefix `Revert "`.
///
/// `extra_patterns` from [`OracleConfig`] are additional regexes OR'd into the
/// classification, compiled once here in [`DefectOracle::new`].
///
/// Deliberately separate from `kamei::enrich_fix`'s `\b(bug|fix|fixes|fixed|
/// defect|patch|hotfix|issue|error)\b` — that regex is a JIT-SDP feature input
/// left untouched; its broader alternation (`patch`, `issue`, `error`) is a
/// documented SZZ precision trap this oracle deliberately narrows.
#[derive(Debug, Clone)]
pub struct DefectOracle {
    conventional: Regex,
    word_boundary: Regex,
    extra: Vec<Regex>,
}

/// Literal prefix marking a git-generated revert commit message
/// (`Revert "<original subject>"`). Checked case-sensitively — this is the
/// exact prefix `git revert` itself produces.
const REVERT_PREFIX: &str = "Revert \"";

impl DefectOracle {
    /// Compile the built-in classifiers plus `cfg.extra_patterns`.
    ///
    /// # Errors
    ///
    /// [`CodeLoreError::InvalidOptions`] when an entry in `extra_patterns` is
    /// not a valid regex — a configuration mistake, not a degradable state.
    pub fn new(cfg: &OracleConfig) -> Result<Self> {
        let conventional = Regex::new(r"(?i)^fix(\([^)]*\))?:").map_err(|e| {
            CodeLoreError::Analysis(format!("built-in oracle conventional pattern: {e}"))
        })?;
        // `bugfix` is leading-boundary-only so `bugfixes` counts; every other
        // term is a whole word (`\b…\b`) so `fixture`/`hotfixture`/`defective`
        // — common non-defect vocabulary, verified against this repository's
        // own commit history — never classify as fixes.
        let word_boundary =
            Regex::new(r"(?i)(?:\bbugfix|\b(?:bug|fix(?:es|ed)?|defect|regression|hotfix)\b)")
                .map_err(|e| {
                    CodeLoreError::Analysis(format!("built-in oracle word-boundary pattern: {e}"))
                })?;
        let mut extra = Vec::with_capacity(cfg.extra_patterns.len());
        for pattern in &cfg.extra_patterns {
            let re = Regex::new(pattern).map_err(|e| {
                CodeLoreError::InvalidOptions(format!(
                    "defect oracle extra pattern {pattern:?}: {e}"
                ))
            })?;
            extra.push(re);
        }
        Ok(Self {
            conventional,
            word_boundary,
            extra,
        })
    }

    /// Classify `message` as a fix commit. `is_merge` is caller-supplied
    /// (this module has no notion of the commit graph) and, when true,
    /// unconditionally yields `false` regardless of message content.
    #[must_use]
    pub fn is_fix(&self, message: &str, is_merge: bool) -> bool {
        if is_merge || message.starts_with(REVERT_PREFIX) {
            return false;
        }
        self.conventional.is_match(message)
            || self.word_boundary.is_match(message)
            || self.extra.iter().any(|re| re.is_match(message))
    }
}

// ─── Unit E: artifact model ───────────────────────────────────────────────────

/// Mining-phase tallies recorded on every built artifact, whether or not any
/// fixes were found (an empty-linkage artifact is never an error — see the
/// spec's Error handling section).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MiningStats {
    /// Commits the oracle classified as fixes.
    pub fixes_found: u32,
    /// `(defect_rev, fix_rev, path)` links the AG-SZZ engine emitted.
    pub links_found: u32,
    /// Distinct `(fix_rev, path)` pairs sent through `git blame`.
    pub files_blamed: u32,
    /// Deleted pre-image lines examined across all fix hunks.
    pub lines_considered: u32,
    /// Of those, lines the AG filter dropped as cosmetic (blank / comment-only).
    pub lines_dropped_cosmetic: u32,
    /// Blame or blob-read failures — skipped, never fatal.
    pub blame_failures: u32,
    /// Fix hunks that were pure additions (no deleted lines, so no candidates).
    pub pure_addition_fixes: u32,
}

/// Validation-report metrics: does HEAD's structural risk predict where the
/// mined defects landed? Presentation follows the project's honesty framing —
/// association, not causation; every number carries its `n`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ValidationMetrics {
    /// `(band, defect_changes, share)` — share of defect-introducing changes
    /// that landed in files at each code-health band at the time.
    pub band_table: Vec<(String, u32, f64)>,
    /// AUC of HEAD's `structural_risk` against the defect-implicated-file
    /// labels, using the default (untuned) weights. `None` when either class
    /// (implicated / not) is empty.
    pub auc_default: Option<f64>,
    /// Precision among the 10 highest-risk files. `None` when fewer than 10
    /// files have health data.
    pub precision_at_10: Option<f64>,
    /// Precision among the files banded red at HEAD. `None` when zero files
    /// are red.
    pub precision_at_red: Option<f64>,
    /// Distinct files touched by at least one defect-introducing commit.
    pub implicated_files: u32,
    /// Distinct defect-introducing commits (the SZZ links' defect side,
    /// deduplicated).
    pub linked_defects: u32,
    /// RFC 3339 dates of the historical band-scan samples used to look up
    /// "health at the time" for each defect-introducing commit.
    pub sample_dates: Vec<String>,
    /// Defect-introducing commits excluded because no health sample existed
    /// at or before their date (and none is the earliest sample either).
    pub excluded_no_data: u32,
}

/// The tuning decision recorded on every artifact — which branch of the
/// honesty floor / acceptance test fired, and both AUCs so a reader can judge
/// the outcome even when defaults were kept.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome")]
pub enum TuningDecision {
    /// The tuned weights were adopted: the validation-split AUC improvement
    /// cleared the acceptance margin over the default weights.
    Applied {
        /// AUC on the training split, for the chosen (tuned) weights.
        auc_train: f64,
        /// AUC on the validation split, for the *default* weights.
        auc_validation_default: f64,
        /// AUC on the validation split, for the *tuned* weights.
        auc_validation_tuned: f64,
    },
    /// Defaults were kept. `reason` names which branch fired: an honesty-floor
    /// sample-size guard (too few linked defect-changes / implicated files) or an
    /// unmet acceptance margin.
    DefaultsKept {
        reason: String,
        /// `Some` when the margin-unmet branch computed both AUCs; `None`
        /// when an earlier sample-size floor short-circuited before any AUC
        /// existed.
        auc_validation_default: Option<f64>,
        /// See `auc_validation_default`.
        auc_validation_tuned: Option<f64>,
    },
}

/// A versioned own-repo defect-calibration artifact (`defects.calib.json`).
///
/// Mirrors `calibration::CalibrationArtifact`'s serde style: a compact
/// (non-pretty) JSON container, loaded and validated by [`load`], written by
/// [`save`]. Applying an artifact mined from a different repository is a hard
/// error unless explicitly overridden — see [`check_repo_identity`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefectArtifact {
    pub format_version: u32,
    /// SHA-256 hex (64 chars) of the canonicalized repo path at mining time —
    /// see [`repo_identity`]. Distinct from `head_at_mining`: this identifies
    /// *which repository*, not *which commit*.
    pub repo_identity: String,
    /// HEAD SHA at mining time.
    pub head_at_mining: String,
    /// Human-readable vintage, e.g. `"defects-2026-07-15"`.
    pub vintage: String,
    /// RFC 3339 build timestamp (caller-injected, like
    /// `calibration::build_from_observations`'s `generated_at`, so builds are
    /// byte-reproducible given identical inputs and timestamp).
    pub generated_at: String,
    /// The oracle configuration used to mine this artifact.
    pub oracle: OracleConfig,
    pub mining: MiningStats,
    pub validation: ValidationMetrics,
    /// The eight smell weights (tuned or left at their defaults), in
    /// `code_health::SMELL_WEIGHTS` order.
    pub weights: Vec<(String, f64)>,
    pub tuning: TuningDecision,
}

/// Serialize `artifact` as compact JSON and write it to `path`, creating
/// parent directories as needed.
///
/// Mirrors the `calibrate` command's write idiom exactly: `serde_json::to_vec`
/// (compact, not pretty) so byte-determinism tests over identical inputs are
/// meaningful, not an artifact of formatting.
///
/// # Errors
///
/// [`CodeLoreError::Analysis`] if serialization fails (should not happen for
/// a well-formed `DefectArtifact`); [`CodeLoreError::Io`] if the parent
/// directory cannot be created or the file cannot be written.
pub fn save(artifact: &DefectArtifact, path: &Path) -> Result<()> {
    let bytes = serde_json::to_vec(artifact).map_err(|e| {
        CodeLoreError::Analysis(format!("serialize defect-calibration artifact: {e}"))
    })?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, &bytes)?;
    Ok(())
}

/// Read and validate a defect-calibration artifact from `path`.
///
/// # Errors
///
/// [`CodeLoreError::RepoIo`] (read-side input, exit 3) when the file cannot
/// be read; [`CodeLoreError::Analysis`] (exit 4) when the JSON is malformed
/// or `format_version` does not match [`DEFECT_FORMAT_VERSION`] — the error
/// message names both versions. An explicitly passed `--defect-calibration`
/// file that cannot be used is a configuration mistake, not a degradable
/// state, so both failure modes are hard errors.
pub fn load(path: &Path) -> Result<DefectArtifact> {
    let bytes = std::fs::read(path).map_err(|e| {
        CodeLoreError::RepoIo(std::io::Error::new(
            e.kind(),
            format!("read defect-calibration artifact {}: {e}", path.display()),
        ))
    })?;
    let artifact: DefectArtifact = serde_json::from_slice(&bytes).map_err(|e| {
        CodeLoreError::Analysis(format!(
            "parse defect-calibration artifact {}: {e}",
            path.display()
        ))
    })?;
    if artifact.format_version != DEFECT_FORMAT_VERSION {
        return Err(CodeLoreError::Analysis(format!(
            "unknown defect-calibration format_version {} (this build supports {DEFECT_FORMAT_VERSION}): {}",
            artifact.format_version,
            path.display()
        )));
    }
    Ok(artifact)
}

/// `fs::canonicalize` wrapper that falls back to the raw path (with a debug
/// log) on failure — mirrors `cache::canonicalize_with_fallback_log` so a
/// repo's identity fingerprint is derived the same way the cache key is.
fn canonicalize_repo_path(repo_path: &Path) -> PathBuf {
    match std::fs::canonicalize(repo_path) {
        Ok(canonical) => canonical,
        Err(e) => {
            tracing::debug!(
                "defect_calibration::repo_identity: canonicalize fallback for {} ({e}); using raw path",
                repo_path.display()
            );
            repo_path.to_path_buf()
        }
    }
}

/// SHA-256 hex (all 64 chars) of the canonicalized `repo_path` — the
/// full-length counterpart to `cache::repo_hash_short`'s 8-char truncation.
/// Used both to stamp a freshly-mined artifact's `repo_identity` and, via
/// [`check_repo_identity`], to verify one before applying it.
#[must_use]
pub fn repo_identity(repo_path: &Path) -> String {
    let canonical = canonicalize_repo_path(repo_path);
    let mut hasher = Sha256::new();
    hasher.update(canonical.to_string_lossy().as_bytes());
    hex::encode(hasher.finalize())
}

/// Guard against applying an artifact mined from a different repository.
///
/// Compares `art.repo_identity` against [`repo_identity`] of `repo_path`.
/// `allow_foreign` is the `--allow-foreign-calibration` escape hatch for
/// forks: when true, the check is skipped unconditionally.
///
/// # Errors
///
/// [`CodeLoreError::Analysis`] when the identities differ and
/// `allow_foreign` is false. Never errors when `allow_foreign` is true.
pub fn check_repo_identity(
    art: &DefectArtifact,
    repo_path: &Path,
    allow_foreign: bool,
) -> Result<()> {
    if allow_foreign {
        return Ok(());
    }
    let actual = repo_identity(repo_path);
    if actual == art.repo_identity {
        return Ok(());
    }
    Err(CodeLoreError::Analysis(format!(
        "defect-calibration artifact was mined from a different repository (artifact identity {}, this repo is {actual}); pass --allow-foreign-calibration to apply it anyway",
        art.repo_identity
    )))
}

/// The smell-weight table and artifact vintage resolved by [`active_weights`].
pub type WeightsAndVintage = (Vec<(String, f64)>, String);

/// Resolve the smell weights active for `opts`: `Some((weights, vintage))`
/// when a `--defect-calibration` artifact is configured and passes both the
/// repo-identity guard and shape validation; `None` when no artifact is
/// configured.
///
/// The returned weights are the artifact's `weights` field verbatim — for a
/// `DefaultsKept` artifact those ARE the defaults, so substituting them is
/// inert by construction while the vintage stamp still records that the
/// artifact was consulted.
///
/// Shape validation pins the weights to exactly the built-in smell names, in
/// the built-in order, with finite non-negative values. Anything else is a
/// corrupted or incompatible artifact — and rejecting it here is also what
/// keeps interpolating the names into the code-health SQL injection-free.
///
/// # Errors
///
/// Propagates [`load`] failures, the [`check_repo_identity`] foreign-repo
/// guard (see `Options::allow_foreign_calibration`), and shape-validation
/// failures, all as hard errors: an explicitly configured artifact that
/// cannot be applied is a configuration mistake, not a degradable state.
pub fn active_weights(opts: &crate::Options) -> Result<Option<WeightsAndVintage>> {
    let Some(path) = &opts.defect_calibration else {
        return Ok(None);
    };
    let art = load(path)?;
    check_repo_identity(&art, &opts.repo_path, opts.allow_foreign_calibration)?;
    let expected = crate::analyses::code_health::SMELL_WEIGHTS;
    if art.weights.len() != expected.len()
        || art
            .weights
            .iter()
            .zip(expected)
            .any(|((name, w), &(exp, _))| name != exp || !w.is_finite() || *w < 0.0)
    {
        return Err(CodeLoreError::Analysis(format!(
            "defect-calibration artifact {} has malformed weights: expected the {} built-in smells in canonical order with finite non-negative values",
            path.display(),
            expected.len()
        )));
    }
    Ok(Some((art.weights, art.vintage)))
}

/// Vintage string of the defect-calibration artifact active for `opts`. A
/// thin wrapper over [`active_weights`] — the one place resolution, the
/// identity guard, and shape validation live — so the provenance stamp and
/// the weight substitution never drift apart (mirrors
/// `calibration::active_vintage`).
///
/// # Errors
///
/// Propagates [`active_weights`] failures.
pub fn active_vintage(opts: &crate::Options) -> Result<Option<String>> {
    Ok(active_weights(opts)?.map(|(_, vintage)| vintage))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oracle(extra_patterns: &[&str]) -> DefectOracle {
        let cfg = OracleConfig {
            extra_patterns: extra_patterns.iter().map(|s| (*s).to_string()).collect(),
        };
        DefectOracle::new(&cfg).expect("valid oracle config compiles")
    }

    // ─── Unit A oracle table (verbatim from the design spec) ────────────────

    #[test]
    fn conventional_prefix_matches_case_insensitively() {
        let o = oracle(&[]);
        assert!(o.is_fix("fix: null deref", false));
        assert!(o.is_fix("Fix(parser): does the thing", false));
    }

    #[test]
    fn word_boundary_terms_match() {
        let o = oracle(&[]);
        assert!(o.is_fix("bugfix for #12", false));
        assert!(o.is_fix("regression in DSM", false));
    }

    #[test]
    fn bugfix_is_leading_only_so_its_plural_compound_still_matches() {
        // `bugfix` is the one leading-boundary-only term, so its plural
        // compound "bugfixes" still counts; every other term requires a
        // trailing boundary too.
        let o = oracle(&[]);
        assert!(o.is_fix("prefix bugfixes", false));
    }

    #[test]
    fn fixture_vocabulary_never_classifies_as_a_fix() {
        // "fixture"/"fixtures" are everyday testing vocabulary — this
        // repository's own history carries test/ci commits mentioning them —
        // and the whole-word boundaries keep them out of the defect set.
        let o = oracle(&[]);
        assert!(!o.is_fix("test(fixtures): biomarker repo exercises nesting", false));
        assert!(!o.is_fix("Shared test fixtures as checked-in git bundles", false));
        assert!(!o.is_fix("hotfixture deploy", false));
        assert!(!o.is_fix("defective by design", false));
    }

    #[test]
    fn mid_word_occurrence_without_a_leading_boundary_does_not_match() {
        // "affix" contains the literal substring "fix", but it starts
        // mid-word (preceded by another word character), so the leading
        // boundary check correctly excludes it.
        let o = oracle(&[]);
        assert!(!o.is_fix("affix labels", false));
    }

    #[test]
    fn patch_is_not_in_the_strict_vocabulary() {
        // Unlike the kamei fix regex, "patch" is deliberately excluded here.
        let o = oracle(&[]);
        assert!(!o.is_fix("patch bump", false));
    }

    #[test]
    fn revert_prefix_excludes_regardless_of_body_content() {
        let o = oracle(&[]);
        assert!(!o.is_fix("Revert \"fix: x\"", false));
    }

    #[test]
    fn merge_flag_excludes_regardless_of_message() {
        let o = oracle(&[]);
        assert!(!o.is_fix("fix: this would otherwise match", true));
    }

    #[test]
    fn extra_pattern_is_ored_in() {
        let o = oracle(&["JIRA-\\d+"]);
        assert!(o.is_fix("JIRA-77 crash", false));
        // Extra patterns don't disable the built-ins.
        assert!(o.is_fix("fix: still works", false));
        // And a message matching neither built-in nor extra stays false.
        assert!(!o.is_fix("refactor: tidy imports", false));
    }

    #[test]
    fn invalid_extra_pattern_is_a_typed_configuration_error() {
        let cfg = OracleConfig {
            extra_patterns: vec!["(unclosed".to_string()],
        };
        let err = DefectOracle::new(&cfg).expect_err("invalid regex must fail to compile");
        assert!(
            matches!(err, CodeLoreError::InvalidOptions(_)),
            "invalid extra pattern must surface as InvalidOptions, got: {err:?}"
        );
    }

    // ─── artifact: identity helper ───────────────────────────────────────────

    #[test]
    fn repo_identity_is_deterministic_and_full_length_hex() {
        let dir = std::env::temp_dir();
        let a = repo_identity(&dir);
        let b = repo_identity(&dir);
        assert_eq!(
            a, b,
            "repo_identity must be deterministic for the same path"
        );
        assert_eq!(a.len(), 64, "expected full 64-char sha256 hex, got {a:?}");
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn repo_identity_differs_for_different_paths() {
        let a = repo_identity(Path::new("/tmp"));
        let b = repo_identity(Path::new("/var"));
        assert_ne!(a, b, "distinct paths must hash to distinct identities");
    }
}

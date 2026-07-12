//! Corpus-calibration artifact: per-language quantile breakpoints for raw
//! per-function metrics, plus the interpolation lookup that turns a metric
//! value into a corpus-relative percentile ("your cyclomatic complexity sits
//! at P74 versus the reference corpus").
//!
//! # The artifact
//!
//! A [`CalibrationArtifact`] is a versioned, compact JSON container. For each
//! Tier-1 language it holds a [`LanguageTable`] whose (v1: single) [`Stratum`]
//! carries one [`MetricQuantiles`] per raw metric. Each metric stores a
//! `QUANTILE_POINTS`-long, non-decreasing vector: element `i` is the value at
//! quantile `i / (QUANTILE_POINTS - 1)`, so index `750` is the corpus q0.750
//! breakpoint. Storing breakpoints (not raw observations) keeps the embedded
//! world artifact small while still supporting interpolated lookups.
//!
//! # Lookup
//!
//! [`percentile`] binary-searches a metric's breakpoint vector and linearly
//! interpolates between the two neighbours. Values below the corpus minimum
//! floor to `p = 0.0`; values above the maximum saturate to `p = 1.0` and set
//! [`CorpusPercentile::beyond_corpus`]. A language whose pooled sample is below
//! [`MIN_LANG_SAMPLE`] — too thin to trust — is treated as absent, as is an
//! unknown language or metric; those all return `None`.
//!
//! # Building
//!
//! [`build_from_observations`] pools raw per-function metric values per language
//! and reduces each pool to a breakpoint vector. The `generated_at` timestamp is
//! injected by the caller (the CLI passes the wall clock; determinism tests pass
//! a constant) so the same observations always produce byte-identical output.
//!
//! [`merge`] blends two artifacts by sample-count-weighted interpolation of
//! their quantile vectors. This is an **approximation** — see its own docs.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::{CodeLoreError, Result};

/// Artifact schema version. A `load`ed artifact whose `format_version` differs
/// is rejected; the CLI caller warns once and proceeds without the corpus lens.
pub const CALIBRATION_FORMAT_VERSION: u32 = 1;

/// Minimum pooled function count for a language to be trusted. Below this the
/// language is treated as absent (its breakpoints are too noisy to compare
/// against).
pub const MIN_LANG_SAMPLE: u64 = 500;

/// Length of every quantile-breakpoint vector: q0.000 … q1.000 inclusive at a
/// 0.001 step.
pub const QUANTILE_POINTS: usize = 1001;

/// Vintage prefix marking a not-yet-built (placeholder) embedded artifact.
/// [`embedded_world`] returns `None` for such an artifact so the corpus lens
/// stays absent-but-wired until a maintainer runs the real corpus build.
const PLACEHOLDER_VINTAGE_PREFIX: &str = "placeholder-";

/// The committed embedded world artifact. Until a maintainer runs the full
/// corpus build (see `calibration/README.md`) this is a placeholder whose
/// vintage begins with [`PLACEHOLDER_VINTAGE_PREFIX`], so [`embedded_world`]
/// yields `None`.
const EMBEDDED_WORLD_BYTES: &[u8] = include_bytes!("calibration/world.calib.json");

// ─── artifact model ──────────────────────────────────────────────────────────

/// A versioned corpus-calibration artifact: per-language quantile breakpoints
/// for raw per-function metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationArtifact {
    pub format_version: u32,
    /// Human-readable corpus vintage, e.g. `"world-2026-07"`.
    pub corpus_vintage: String,
    /// RFC 3339 build timestamp (injected by the builder's caller).
    pub generated_at: String,
    pub repos_included: u32,
    pub repos_attempted: u32,
    pub languages: Vec<LanguageTable>,
}

/// Per-language breakpoints. `language` is a [`Tier1Language`] name
/// (`crate::complexity::language::Tier1Language::as_str`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageTable {
    pub language: String,
    /// Pooled function count behind this table; drives the [`MIN_LANG_SAMPLE`]
    /// floor.
    pub sample_functions: u64,
    /// v1 world artifacts carry exactly one stratum spanning all SLOC.
    pub strata: Vec<Stratum>,
}

/// A SLOC-bounded stratum of metric breakpoints. v1: bounds are `0..=u64::MAX`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stratum {
    pub sloc_min: u64,
    pub sloc_max: u64,
    pub metrics: Vec<MetricQuantiles>,
}

/// Quantile breakpoints for one raw metric (`"cyclomatic"`, `"cognitive"`,
/// `"sloc"`, `"nargs"`, `"max_nesting"`). `quantiles` has length
/// [`QUANTILE_POINTS`] and is non-decreasing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricQuantiles {
    pub metric: String,
    pub quantiles: Vec<f64>,
}

/// The result of a corpus-percentile lookup.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CorpusPercentile {
    /// Corpus-relative rank in `0..=1`.
    pub p: f64,
    /// The looked-up value exceeded the corpus maximum breakpoint.
    pub beyond_corpus: bool,
}

// ─── load + validation ───────────────────────────────────────────────────────

impl CalibrationArtifact {
    /// Deserialize + validate an artifact from raw JSON bytes.
    ///
    /// Mirrors [`load`] without the filesystem read; used by [`embedded_world`]
    /// and tests. Returns a `String` describing the failure so callers can wrap
    /// it in their preferred [`CodeLoreError`] variant.
    fn from_slice(bytes: &[u8]) -> std::result::Result<Self, String> {
        let art: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        art.validate()?;
        Ok(art)
    }

    /// Reject a structurally-parsed-but-semantically-invalid artifact:
    /// unknown format version, wrong-length quantile vectors, or non-monotonic
    /// (decreasing) breakpoints.
    fn validate(&self) -> std::result::Result<(), String> {
        if self.format_version != CALIBRATION_FORMAT_VERSION {
            return Err(format!(
                "unknown calibration format_version {} (this build supports {CALIBRATION_FORMAT_VERSION})",
                self.format_version
            ));
        }
        for lang in &self.languages {
            for stratum in &lang.strata {
                for mq in &stratum.metrics {
                    if mq.quantiles.len() != QUANTILE_POINTS {
                        return Err(format!(
                            "{} metric {:?}: quantile vector length {} (expected {QUANTILE_POINTS})",
                            lang.language,
                            mq.metric,
                            mq.quantiles.len()
                        ));
                    }
                    for pair in mq.quantiles.windows(2) {
                        if pair[1] < pair[0] {
                            return Err(format!(
                                "{} metric {:?}: non-monotonic quantiles ({} then {})",
                                lang.language, mq.metric, pair[0], pair[1]
                            ));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// Load and validate a calibration artifact from a JSON file.
///
/// # Errors
///
/// [`CodeLoreError::RepoIo`] (read-side input, exit 3) when the file cannot be
/// read; [`CodeLoreError::Analysis`] (exit 4) when the JSON is malformed or the
/// artifact fails validation — an unknown `format_version`, a wrong-length
/// quantile vector, or non-monotonic breakpoints. The CLI caller warns once on
/// this error and proceeds without the corpus lens.
pub fn load(path: &Path) -> Result<CalibrationArtifact> {
    let bytes = std::fs::read(path).map_err(|e| {
        // Read-side input failure (unreadable `--calibration`) → exit 3,
        // mirroring `quality_gates::Thresholds::from_path`. The parse/validation
        // failure below stays `Analysis` (exit 4).
        CodeLoreError::RepoIo(std::io::Error::new(
            e.kind(),
            format!("read calibration {}: {e}", path.display()),
        ))
    })?;
    CalibrationArtifact::from_slice(&bytes)
        .map_err(|e| CodeLoreError::Analysis(format!("parse calibration {}: {e}", path.display())))
}

/// The embedded world artifact, lazily parsed once.
///
/// Returns `None` when the embedded file is the placeholder shipped before a
/// maintainer has run the real corpus build (detected by its
/// [`PLACEHOLDER_VINTAGE_PREFIX`] vintage), or if the embedded bytes fail to
/// parse. A `Some` value is a validated artifact safe to hand to [`percentile`].
#[must_use]
pub fn embedded_world() -> Option<&'static CalibrationArtifact> {
    static WORLD: OnceLock<Option<CalibrationArtifact>> = OnceLock::new();
    WORLD
        .get_or_init(|| {
            CalibrationArtifact::from_slice(EMBEDDED_WORLD_BYTES)
                .ok()
                .filter(|art| !art.corpus_vintage.starts_with(PLACEHOLDER_VINTAGE_PREFIX))
        })
        .as_ref()
}

// ─── percentile lookup ───────────────────────────────────────────────────────

/// Corpus-relative percentile of `value` for `(language, metric)`.
///
/// Binary-searches the metric's non-decreasing breakpoint vector and linearly
/// interpolates between the two straddling breakpoints. Contract:
///
/// - `value <= q[0]` → `p = 0.0`, not beyond-corpus.
/// - `value >= q[last]` → `p = 1.0`; beyond-corpus iff `value > q[last]`.
/// - unknown language / unknown metric / language pooled below
///   [`MIN_LANG_SAMPLE`] → `None`.
///
/// **Plateaus (repeated breakpoints).** Integer metrics like `nargs` and
/// `max_nesting` produce long runs of equal breakpoints. A `value` landing on an
/// *interior* plateau resolves to the run's UPPER edge — a `P(X <= value)` (CDF)
/// reading: every function whose metric equals `value` counts as at-or-below it.
/// A `value` on a plateau that touches the *minimum* (`value <= q[0]`) instead
/// returns `0.0` via the short-circuit above. Both are deliberate.
#[must_use]
pub fn percentile(
    art: &CalibrationArtifact,
    language: &str,
    metric: &str,
    value: f64,
) -> Option<CorpusPercentile> {
    let lang = art.languages.iter().find(|l| l.language == language)?;
    if lang.sample_functions < MIN_LANG_SAMPLE {
        return None;
    }
    // v1: a single stratum. Search every stratum's matching metric and take the
    // first hit, so this stays correct if later versions add SLOC strata.
    let quantiles = lang
        .strata
        .iter()
        .flat_map(|s| &s.metrics)
        .find(|m| m.metric == metric)
        .map(|m| m.quantiles.as_slice())?;

    Some(interpolate_percentile(quantiles, value))
}

/// Lossless `usize` → `f64` for the small, bounded counts this module casts:
/// quantile-vector indices/lengths (`<= QUANTILE_POINTS`) and per-language
/// sample sizes. All are orders of magnitude below `2^53`, so the conversion is
/// exact — the `cast_precision_loss` lint is a false positive here.
#[allow(clippy::cast_precision_loss)]
fn count_to_f64(n: usize) -> f64 {
    n as f64
}

/// Map `value` to a percentile over a non-decreasing breakpoint vector.
///
/// Assumes `q.len() == QUANTILE_POINTS` (guaranteed by validation). The
/// percentile of breakpoint index `i` is `i / (len - 1)`.
///
/// On a plateau of equal breakpoints, `partition_point(|b| b <= value)` counts
/// the whole run, so `value` resolves to the plateau's UPPER edge (the CDF /
/// `P(X <= value)` reading described on [`percentile`]). A plateau touching the
/// minimum is handled by the `value <= q[0]` short-circuit → `0.0`.
fn interpolate_percentile(q: &[f64], value: f64) -> CorpusPercentile {
    let last = q.len() - 1;
    let denom = count_to_f64(last);

    if value <= q[0] {
        return CorpusPercentile {
            p: 0.0,
            beyond_corpus: false,
        };
    }
    if value >= q[last] {
        return CorpusPercentile {
            p: 1.0,
            beyond_corpus: value > q[last],
        };
    }

    // `partition_point` gives the count of breakpoints `<= value`; since
    // q[0] < value < q[last] here, `hi` lands in `1..=last` and `lo = hi - 1`
    // straddle the value.
    let hi = q.partition_point(|&b| b <= value);
    let lo = hi - 1;
    let (blo, bhi) = (q[lo], q[hi]);

    // Interpolate the fractional index between the two breakpoints, then
    // normalise to a percentile. Equal neighbours (a flat run of the vector)
    // fall back to the lower index to avoid dividing by zero.
    let frac = if bhi > blo {
        (value - blo) / (bhi - blo)
    } else {
        0.0
    };
    let p = (count_to_f64(lo) + frac) / denom;
    CorpusPercentile {
        p,
        beyond_corpus: false,
    }
}

// ─── builder: pooled observations → breakpoints ──────────────────────────────

/// Raw per-function metric observations, pooled per `(language, metric)` before
/// reduction to breakpoint vectors by [`build_from_observations`].
///
/// Values accumulate in insertion order; the builder sorts each pool. Populate
/// it with [`observe`](LangObservations::observe).
#[derive(Debug, Clone, Default)]
pub struct LangObservations {
    // language → metric → raw values.
    per_lang: BTreeMap<String, BTreeMap<String, Vec<f64>>>,
}

impl LangObservations {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one raw metric value for a function of `language`.
    pub fn observe(&mut self, language: &str, metric: &str, value: f64) {
        self.per_lang
            .entry(language.to_string())
            .or_default()
            .entry(metric.to_string())
            .or_default()
            .push(value);
    }

    /// Pooled function count for a language = the max samples across its
    /// metrics (each function contributes one value per metric it has).
    fn sample_functions(metrics: &BTreeMap<String, Vec<f64>>) -> u64 {
        metrics.values().map(|v| v.len() as u64).max().unwrap_or(0)
    }
}

/// Build a calibration artifact from pooled raw observations.
///
/// Each `(language, metric)` pool is sorted and reduced to a
/// [`QUANTILE_POINTS`]-long breakpoint vector via [`quantile_breakpoints`]. The
/// `generated_at` timestamp is a caller-injected RFC 3339 string — the CLI
/// passes the current time, determinism tests pass a constant — so identical
/// observations always produce byte-identical output. `repos_included` /
/// `repos_attempted` are left at `0` here; the `calibrate` command overwrites
/// them from its per-repo tally.
#[must_use]
pub fn build_from_observations(
    vintage: &str,
    generated_at: &str,
    obs: &LangObservations,
) -> CalibrationArtifact {
    let languages = obs
        .per_lang
        .iter()
        .map(|(language, metrics)| {
            let sample_functions = LangObservations::sample_functions(metrics);
            let metric_quantiles = metrics
                .iter()
                .map(|(metric, values)| {
                    let mut sorted = values.clone();
                    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                    MetricQuantiles {
                        metric: metric.clone(),
                        quantiles: quantile_breakpoints(&sorted),
                    }
                })
                .collect();
            LanguageTable {
                language: language.clone(),
                sample_functions,
                strata: vec![Stratum {
                    sloc_min: 0,
                    sloc_max: u64::MAX,
                    metrics: metric_quantiles,
                }],
            }
        })
        .collect();

    CalibrationArtifact {
        format_version: CALIBRATION_FORMAT_VERSION,
        corpus_vintage: vintage.to_string(),
        generated_at: generated_at.to_string(),
        repos_included: 0,
        repos_attempted: 0,
        languages,
    }
}

/// Reduce a sorted sample to a [`QUANTILE_POINTS`]-long non-decreasing
/// breakpoint vector.
///
/// Breakpoint `i` is the value at quantile `i / (QUANTILE_POINTS - 1)`,
/// computed by linear interpolation over the sample's rank positions
/// (`(n - 1) * q`). An empty sample yields all-zero breakpoints; a single
/// sample yields a constant vector.
fn quantile_breakpoints(sorted: &[f64]) -> Vec<f64> {
    let last_idx = QUANTILE_POINTS - 1;
    if sorted.is_empty() {
        return vec![0.0; QUANTILE_POINTS];
    }
    let n = sorted.len();
    if n == 1 {
        return vec![sorted[0]; QUANTILE_POINTS];
    }
    (0..QUANTILE_POINTS)
        .map(|i| {
            let q = count_to_f64(i) / count_to_f64(last_idx);
            let rank = q * count_to_f64(n - 1);
            // `rank` is non-negative and at most `n - 1`, so floor/ceil are
            // exact, in range, and never negative — the truncation/sign-loss
            // lints are false positives on this bounded, checked value.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let (lo, hi) = (rank.floor() as usize, rank.ceil() as usize);
            if lo == hi {
                sorted[lo]
            } else {
                let frac = rank - count_to_f64(lo);
                sorted[lo] + (sorted[hi] - sorted[lo]) * frac
            }
        })
        .collect()
}

// ─── merge: weighted quantile blending (approximation) ───────────────────────

/// Blend two artifacts by sample-count-weighted interpolation of their quantile
/// vectors, summing sample counts.
///
/// # Approximation
///
/// This is **not** an exact pooled re-quantiling. Exactness needs the two raw
/// observation pools, which an artifact (breakpoints only) does not retain; the
/// true pooled quantile of the union is generally *not* a weighted average of
/// the per-corpus quantiles. For an exact merge, re-run `calibrate` over the
/// union manifest instead. The weighted blend here is accurate when the two
/// corpora have similar distributions and is a monotonic, order-preserving
/// estimate otherwise — adequate for the `--merge` "fold my org's repos into
/// the world corpus" use case.
///
/// A `(language, metric)` present in only one input is carried through
/// unchanged. A language present in only one input is carried through with its
/// original sample count.
#[must_use]
pub fn merge(base: CalibrationArtifact, additional: CalibrationArtifact) -> CalibrationArtifact {
    let mut add_by_lang: BTreeMap<String, LanguageTable> = additional
        .languages
        .into_iter()
        .map(|l| (l.language.clone(), l))
        .collect();

    let mut merged_languages = Vec::new();
    for base_lang in base.languages {
        match add_by_lang.remove(&base_lang.language) {
            Some(add_lang) => merged_languages.push(blend_language(base_lang, add_lang)),
            None => merged_languages.push(base_lang),
        }
    }
    // Languages only in `additional` carry through as-is (BTreeMap keeps the
    // append order deterministic).
    merged_languages.extend(add_by_lang.into_values());

    CalibrationArtifact {
        format_version: CALIBRATION_FORMAT_VERSION,
        corpus_vintage: base.corpus_vintage,
        generated_at: base.generated_at,
        repos_included: base
            .repos_included
            .saturating_add(additional.repos_included),
        repos_attempted: base
            .repos_attempted
            .saturating_add(additional.repos_attempted),
        languages: merged_languages,
    }
}

/// Blend two same-language tables: sum sample counts, weight-blend the
/// breakpoint vectors of shared metrics, carry through unshared metrics.
fn blend_language(base: LanguageTable, add: LanguageTable) -> LanguageTable {
    let wb = base.sample_functions;
    let wa = add.sample_functions;

    // v1 tables have a single stratum; index shared metrics by name within it.
    let base_metrics = single_stratum_metrics(base.strata);
    let add_metrics = single_stratum_metrics(add.strata);

    let mut add_by_metric: BTreeMap<String, Vec<f64>> = add_metrics
        .into_iter()
        .map(|m| (m.metric, m.quantiles))
        .collect();

    let mut blended = Vec::new();
    for m in base_metrics {
        match add_by_metric.remove(&m.metric) {
            Some(add_q) => blended.push(MetricQuantiles {
                metric: m.metric,
                quantiles: blend_quantiles(&m.quantiles, wb, &add_q, wa),
            }),
            None => blended.push(m),
        }
    }
    for (metric, quantiles) in add_by_metric {
        blended.push(MetricQuantiles { metric, quantiles });
    }

    LanguageTable {
        language: base.language,
        sample_functions: wb.saturating_add(wa),
        strata: vec![Stratum {
            sloc_min: 0,
            sloc_max: u64::MAX,
            metrics: blended,
        }],
    }
}

/// Flatten a v1 table's strata into a single metric list.
fn single_stratum_metrics(strata: Vec<Stratum>) -> Vec<MetricQuantiles> {
    strata.into_iter().flat_map(|s| s.metrics).collect()
}

/// Weighted per-breakpoint blend of two equal-length quantile vectors. Weights
/// are the two sample counts; a zero total weight falls back to the base vector.
fn blend_quantiles(base: &[f64], wb: u64, add: &[f64], wa: u64) -> Vec<f64> {
    let total = wb.saturating_add(wa);
    if total == 0 || base.len() != add.len() {
        return base.to_vec();
    }
    let (wb, wa, total) = (weight_to_f64(wb), weight_to_f64(wa), weight_to_f64(total));
    base.iter()
        .zip(add.iter())
        .map(|(b, a)| (b * wb + a * wa) / total)
        .collect()
}

/// Lossless `u64` → `f64` for sample-count weights. Corpus sample counts are
/// function tallies (millions at most), far below `2^53`, so the conversion is
/// exact — the `cast_precision_loss` lint is a false positive here.
#[allow(clippy::cast_precision_loss)]
fn weight_to_f64(n: u64) -> f64 {
    n as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The committed embedded artifact is the placeholder shipped before a real
    /// corpus build (its vintage carries [`PLACEHOLDER_VINTAGE_PREFIX`]), so the
    /// corpus lens is absent-but-wired: `embedded_world` yields `None`.
    #[test]
    fn embedded_world_is_absent_for_the_placeholder() {
        assert!(
            embedded_world().is_none(),
            "the shipped placeholder must resolve to None until a maintainer \
             runs the real corpus build"
        );
    }

    /// The embedded bytes must still be a structurally valid, monotonic v1
    /// artifact — the placeholder is filtered by vintage, not by being junk.
    #[test]
    fn embedded_placeholder_bytes_parse_and_validate() {
        let art = CalibrationArtifact::from_slice(EMBEDDED_WORLD_BYTES)
            .expect("embedded placeholder must be a valid artifact");
        assert!(art.corpus_vintage.starts_with(PLACEHOLDER_VINTAGE_PREFIX));
        assert_eq!(art.format_version, CALIBRATION_FORMAT_VERSION);
    }

    /// A non-placeholder vintage is surfaced (the Task-12 "real artifact → Some"
    /// path), proving the filter keys on the vintage prefix alone.
    #[test]
    fn from_slice_accepts_a_real_vintage() {
        let obs = LangObservations::new();
        let art = build_from_observations("world-2026-07", "2026-07-01T00:00:00Z", &obs);
        assert!(!art.corpus_vintage.starts_with(PLACEHOLDER_VINTAGE_PREFIX));
        // Round-trips through the same validate() path embedded_world() uses.
        let bytes = serde_json::to_vec(&art).expect("serialize");
        let back = CalibrationArtifact::from_slice(&bytes).expect("valid real-vintage artifact");
        assert_eq!(back.corpus_vintage, "world-2026-07");
    }
}

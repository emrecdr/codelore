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
//! [`conditional_tail_percentile`] is the companion lookup for pools dominated
//! by trivial (zero-complexity) functions: it conditions the percentile on the
//! non-trivial tail so a real file's complexity does not saturate the top of a
//! mostly-zero distribution.
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
/// is rejected with a hard error — an explicitly passed `--calibration` file
/// that cannot be used is a configuration mistake, not a degradable state.
pub const CALIBRATION_FORMAT_VERSION: u32 = 1;

/// Minimum pooled function count for a language to be trusted. Below this the
/// language is treated as absent (its breakpoints are too noisy to compare
/// against).
pub const MIN_LANG_SAMPLE: u64 = 500;

/// Length of every quantile-breakpoint vector: q0.000 … q1.000 inclusive at a
/// 0.001 step.
pub const QUANTILE_POINTS: usize = 1001;

/// Triviality threshold for [`conditional_tail_percentile`]: a per-function
/// complexity metric at or below this value carries no decision structure, so
/// the function is excluded from the non-trivial tail the conditional lookup
/// spreads across `[0, 1]`. Zero — "has any branching at all" — is the natural
/// boundary and is resolvable *exactly* from the stored breakpoints, so no
/// tunable magnitude enters the model.
const TRIVIALITY_THRESHOLD: f64 = 0.0;

/// Vintage prefix marking a not-yet-built (placeholder) embedded artifact.
/// [`embedded_world`] returns `None` for such an artifact so the corpus lens
/// stays absent-but-wired until a maintainer runs the real corpus build.
const PLACEHOLDER_VINTAGE_PREFIX: &str = "placeholder-";

/// The committed embedded world artifact, produced by the corpus build over
/// `calibration/corpus.toml` (see `calibration/README.md` to regenerate). Its
/// real vintage activates the corpus lens through [`embedded_world`]. A
/// placeholder whose vintage begins with [`PLACEHOLDER_VINTAGE_PREFIX`] would
/// instead resolve to `None`, keeping the lens absent-but-wired.
const EMBEDDED_WORLD_BYTES: &[u8] = include_bytes!("calibration/world.calib.json");

// ─── artifact model ──────────────────────────────────────────────────────────

/// Repo-level metric pools: one observation per corpus repo. Absent on
/// artifacts built before this section existed — absent = no lens.
///
/// Each vec in `values` is sorted ascending (enforced by [`attach_repo_metrics`]).
/// Downstream lookup via [`raw_percentile`] relies on this invariant.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RepoMetrics {
    /// Sorted ascending. Key: metric name (`"propagation_cost"`, `"cycle_file_share"`).
    pub values: std::collections::BTreeMap<String, Vec<f64>>,
}

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
    /// Optional repo-level metric pools (one value per corpus repo). Absent on
    /// artifacts built before this section existed; absent = no repo-level lens.
    /// Omitted from serialization when `None` so pre-section artifacts
    /// round-trip byte-identically.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_metrics: Option<RepoMetrics>,
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

// ─── corpus manifest ─────────────────────────────────────────────────────────

/// A corpus build manifest: the set of pinned repos the `calibrate` command
/// ingests and pools per-function metrics from.
///
/// Parsed from TOML. Each `[[repos]]` entry names a `source` (an HTTPS/SSH clone
/// URL or a local filesystem path), a pinned `sha`, and the `languages` the repo
/// contributes to the corpus (advisory: which Tier-1 languages the curator
/// expects; the command pools whatever the ingest actually finds).
#[derive(Debug, Clone, Deserialize)]
pub struct CorpusManifest {
    #[serde(default)]
    pub repos: Vec<CorpusRepo>,
}

/// One pinned repo in a [`CorpusManifest`].
#[derive(Debug, Clone, Deserialize)]
pub struct CorpusRepo {
    /// Clone URL (contains `://` or `git@…`) or a local filesystem path.
    pub source: String,
    /// Commit SHA the corpus is pinned to, checked out before ingest so the
    /// artifact is reproducible against a fixed tree.
    pub sha: String,
    /// Tier-1 language names the curator expects this repo to contribute.
    #[serde(default)]
    pub languages: Vec<String>,
}

/// Parse and return a [`CorpusManifest`] from a TOML file.
///
/// # Errors
///
/// [`CodeLoreError::RepoIo`] (read-side input, exit 3) when the file cannot be
/// read; [`CodeLoreError::Analysis`] (exit 4) when the TOML is malformed.
pub fn load_manifest(path: &Path) -> Result<CorpusManifest> {
    let raw = std::fs::read_to_string(path).map_err(|e| {
        CodeLoreError::RepoIo(std::io::Error::new(
            e.kind(),
            format!("read corpus manifest {}: {e}", path.display()),
        ))
    })?;
    toml::from_str(&raw).map_err(|e| {
        CodeLoreError::Analysis(format!("parse corpus manifest {}: {e}", path.display()))
    })
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
        if self.languages.is_empty() {
            return Err(
                "artifact contains no languages — every percentile lookup would silently \
                 report \"not in corpus\"; regenerate it from a non-empty corpus"
                    .to_string(),
            );
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
/// quantile vector, or non-monotonic breakpoints. Callers propagate this as a
/// hard error: an explicitly passed calibration file that cannot be used is a
/// configuration mistake, not a degradable state.
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

/// The single home for the active-artifact resolution precedence, shared by the
/// corpus-percentile lens and the provenance vintage stamp:
///
/// 1. `opts.calibration` path set → [`load`] it (validated), owned.
/// 2. else the [`embedded_world`] artifact (a real, non-placeholder one), borrowed.
/// 3. else `None` — no artifact active.
///
/// Returns a [`Cow`] so the embedded branch is zero-copy while a `--calibration`
/// file is owned. A bad `--calibration` file is a hard error (matching [`load`]'s
/// exit-3/4 contract); "no artifact active" returns `None` silently — the one
/// deduped notice lives at the CLI layer.
///
/// [`Cow`]: std::borrow::Cow
pub fn load_active_artifact(
    opts: &crate::Options,
) -> Result<Option<std::borrow::Cow<'static, CalibrationArtifact>>> {
    use std::borrow::Cow;
    if let Some(path) = &opts.calibration {
        return Ok(Some(Cow::Owned(load(path)?)));
    }
    Ok(embedded_world().map(Cow::Borrowed))
}

/// Vintage string of the calibration artifact active for `opts`. A thin wrapper
/// over [`load_active_artifact`] — the one place the resolution precedence lives
/// — so the provenance stamp and the corpus lens never drift apart.
pub fn active_vintage(opts: &crate::Options) -> Result<Option<String>> {
    Ok(load_active_artifact(opts)?.map(|art| art.corpus_vintage.clone()))
}

// ─── percentile lookup ───────────────────────────────────────────────────────

/// The non-decreasing breakpoint vector for `(language, metric)`, or `None`
/// when the language is unknown, pooled below [`MIN_LANG_SAMPLE`], or does not
/// carry the metric. The single lookup behind both [`percentile`] and
/// [`conditional_tail_percentile`], so the two can never disagree on which
/// `(language, metric)` pairs resolve.
///
/// v1 tables carry a single stratum; every stratum's metrics are searched and
/// the first match returned, so this stays correct if a later version adds
/// SLOC-bounded strata.
fn metric_breakpoints<'a>(
    art: &'a CalibrationArtifact,
    language: &str,
    metric: &str,
) -> Option<&'a [f64]> {
    let lang = art.languages.iter().find(|l| l.language == language)?;
    if lang.sample_functions < MIN_LANG_SAMPLE {
        return None;
    }
    lang.strata
        .iter()
        .flat_map(|s| &s.metrics)
        .find(|m| m.metric == metric)
        .map(|m| m.quantiles.as_slice())
}

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
    Some(interpolate_percentile(
        metric_breakpoints(art, language, metric)?,
        value,
    ))
}

/// Percentile of `value` within the corpus's **non-trivial** tail for
/// `(language, metric)` — the conditional lookup the anchored hotspot score
/// uses in place of [`percentile`].
///
/// The plain [`percentile`] compares against the whole pool, which for a
/// complexity metric is dominated by trivial functions — the corpus median
/// cognitive is `0` — so every real file saturates the top percentiles and the
/// anchored score stops discriminating. This conditions the comparison on the
/// functions that carry any decision structure (metric value above
/// [`TRIVIALITY_THRESHOLD`]), re-spreading the informative upper tail across
/// `[0, 1]`:
///
/// ```text
/// p0      = corpus fraction of the pool at value <= 0   (the trivial share)
/// cp_tail = clamp((cp - p0) / (1 - p0), 0, 1)            for value above 0
///         = 0                                            for value <= 0
/// ```
///
/// where `cp` is the plain [`percentile`]. `p0` is read straight off the stored
/// breakpoints by [`trivial_share`] as the highest index still holding a
/// trivial value — the percentile the lookup itself assigns to the top of the
/// zero plateau — so a value just above the plateau maps to `cp_tail ≈ 0` and
/// the corpus maximum to `cp_tail = 1`. It is pure arithmetic on breakpoints
/// every artifact already carries: no format change, no corpus rebuild.
///
/// Returns `None` under the same conditions as [`percentile`] (unknown
/// language / metric, or the language pooled below [`MIN_LANG_SAMPLE`]) **and**
/// when the pool is entirely trivial (`p0 == 1`, an empty tail with no
/// informative content) — an honest omission, never a fabricated value,
/// matching the no-anchor path the caller already handles.
#[must_use]
pub fn conditional_tail_percentile(
    art: &CalibrationArtifact,
    language: &str,
    metric: &str,
    value: f64,
) -> Option<f64> {
    let quantiles = metric_breakpoints(art, language, metric)?;
    let p0 = trivial_share(quantiles);
    let span = 1.0 - p0;
    if span <= 0.0 {
        return None; // All-trivial pool → no informative tail → no anchor.
    }
    if value <= TRIVIALITY_THRESHOLD {
        return Some(0.0); // Trivial file → the bottom of the tail.
    }
    let cp = interpolate_percentile(quantiles, value).p;
    Some(((cp - p0) / span).clamp(0.0, 1.0))
}

/// The corpus **trivial share** `p0` of a breakpoint vector: the fraction of
/// the pool at or below [`TRIVIALITY_THRESHOLD`], read as the highest
/// breakpoint index still holding a trivial value, normalised to `[0, 1]`.
///
/// `q` is non-decreasing (validated), so the trivial values are a prefix and
/// the highest such index is `partition_point(<= T) - 1`. That index's quantile
/// `i_max / (len - 1)` is exactly the percentile [`interpolate_percentile`]
/// assigns to a value just above the plateau, so subtracting it in
/// [`conditional_tail_percentile`] sends the bottom of the non-trivial tail to
/// zero. A pool with no trivial values yields `0.0`; an all-trivial pool yields
/// `1.0` (every breakpoint index holds a trivial value).
fn trivial_share(q: &[f64]) -> f64 {
    let trivial_count = q.partition_point(|&b| b <= TRIVIALITY_THRESHOLD);
    if trivial_count == 0 {
        return 0.0; // No trivial functions in the pool.
    }
    count_to_f64(trivial_count - 1) / count_to_f64(q.len() - 1)
}

/// Pooled per-function sample size behind `language`'s breakpoints — the honest
/// `n` for a Wilson confidence interval on a per-function corpus percentile.
///
/// `None` when the language is absent from the artifact or pooled below
/// [`MIN_LANG_SAMPLE`] — the same trust floor [`percentile`] applies, so a
/// percentile and its confidence interval appear (or vanish) together.
#[must_use]
pub fn language_sample_functions(art: &CalibrationArtifact, language: &str) -> Option<u64> {
    art.languages
        .iter()
        .find(|l| l.language == language)
        .filter(|l| l.sample_functions >= MIN_LANG_SAMPLE)
        .map(|l| l.sample_functions)
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

// ─── raw-values percentile + repo-metrics helper ─────────────────────────────

/// Midpoint-rank percentile of `value` among `sorted` (ascending).
///
/// Returns `None` when `sorted` is empty. For a non-empty slice of length `n`:
///
/// ```text
/// p = (count_less + 0.5 * count_equal) / n
/// ```
///
/// where `count_less` is the number of elements strictly less than `value` and
/// `count_equal` is the number of elements equal to `value`. The result is in
/// `0.0..=1.0`: a `value` below all elements yields `0.0`; a value above all
/// elements yields `1.0`. Ties receive the arithmetic midpoint of their rank
/// range (the standard "midpoint rank" or "fractional rank" formula).
///
/// Unlike [`percentile`], which operates on 1001-breakpoint quantile vectors,
/// this function works directly on raw sorted observations — used for the
/// coarse repo-level pools in [`RepoMetrics`].
///
/// # Precision note
///
/// Counts and lengths are bounded by the corpus size (~hundreds), far below
/// `2^53`, so the `usize` → `f64` conversions are exact.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn raw_percentile(sorted: &[f64], value: f64) -> Option<f64> {
    let n = sorted.len();
    if n == 0 {
        return None;
    }
    // Count elements strictly less than `value` using binary search on the
    // sorted slice: `partition_point(|x| x < value)` gives the index of the
    // first element >= value, which equals the count of elements < value.
    let count_less = sorted.partition_point(|&x| x < value);
    // Count elements equal to `value`: starting from `count_less`, scan
    // forward while elements equal `value`. Using `partition_point` again is
    // equivalent and branchless.
    let count_less_or_equal = sorted.partition_point(|&x| x <= value);
    let count_equal = count_less_or_equal - count_less;
    Some((count_less as f64 + 0.5 * count_equal as f64) / n as f64)
}

/// Attach repo-level metric pools to a calibration artifact.
///
/// Each vec in `pools.values` is sorted ascending in-place before the field is
/// set, so downstream callers can rely on the sorted invariant for binary
/// search (see [`raw_percentile`]).
///
/// An empty `pools.values` map sets `repo_metrics` to `None` — empty pools
/// carry no information and must not activate the lens.
pub fn attach_repo_metrics(artifact: &mut CalibrationArtifact, mut pools: RepoMetrics) {
    if pools.values.is_empty() {
        artifact.repo_metrics = None;
        return;
    }
    for vec in pools.values.values_mut() {
        vec.sort_by(f64::total_cmp);
    }
    artifact.repo_metrics = Some(pools);
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
                    sorted.sort_by(f64::total_cmp);
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
        repo_metrics: None,
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

    // Merge repo_metrics: when only one side has it, carry it through; when
    // both have it, concatenate + re-sort each metric vec (exact pooling,
    // unlike the quantile blend which is an approximation).
    let repo_metrics = merge_repo_metrics(base.repo_metrics, additional.repo_metrics);

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
        repo_metrics,
    }
}

/// Merge two optional [`RepoMetrics`] sections.
///
/// - Both `None` → `None`.
/// - One `Some`, one `None` → keep the `Some` unchanged.
/// - Both `Some` → concatenate each metric's raw values and re-sort ascending.
///   This is exact pooling (unlike the quantile blend, which is an
///   approximation), because the raw values are available here.
fn merge_repo_metrics(
    base: Option<RepoMetrics>,
    additional: Option<RepoMetrics>,
) -> Option<RepoMetrics> {
    match (base, additional) {
        (None, None) => None,
        (Some(b), None) => Some(b),
        (None, Some(a)) => Some(a),
        (Some(mut b), Some(a)) => {
            for (metric, mut add_vals) in a.values {
                b.values
                    .entry(metric)
                    .and_modify(|base_vals| {
                        base_vals.append(&mut add_vals);
                        base_vals.sort_by(f64::total_cmp);
                    })
                    .or_insert_with(|| {
                        add_vals.sort_by(f64::total_cmp);
                        add_vals
                    });
            }
            // Re-sort any base-only metrics that were already sorted — no-op
            // since they were not touched, but ensures the invariant holds for
            // any future caller that relies on sortedness.
            Some(b)
        }
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

    /// The committed embedded artifact is a real world-corpus build (a
    /// non-[`PLACEHOLDER_VINTAGE_PREFIX`] vintage), so the corpus lens is active:
    /// `embedded_world` yields `Some` with a real vintage.
    #[test]
    fn embedded_world_is_present_and_real() {
        let art =
            embedded_world().expect("the embedded world corpus must resolve to Some once built");
        assert!(
            !art.corpus_vintage.starts_with(PLACEHOLDER_VINTAGE_PREFIX),
            "the embedded corpus vintage must be a real build, not a placeholder"
        );
    }

    /// The embedded bytes are a structurally valid, monotonic v1 artifact whose
    /// per-language pools clear the trust floor — the real world corpus, not a
    /// placeholder.
    #[test]
    fn embedded_world_bytes_parse_and_validate() {
        let art = CalibrationArtifact::from_slice(EMBEDDED_WORLD_BYTES)
            .expect("embedded world corpus must be a valid artifact");
        assert!(!art.corpus_vintage.starts_with(PLACEHOLDER_VINTAGE_PREFIX));
        assert_eq!(art.format_version, CALIBRATION_FORMAT_VERSION);
        // Every embedded language must clear the trust floor, or the lens would
        // silently return `None` for it at lookup time.
        for lang in &art.languages {
            assert!(
                lang.sample_functions >= MIN_LANG_SAMPLE,
                "embedded language {:?} pooled {} functions, below the {MIN_LANG_SAMPLE} floor",
                lang.language,
                lang.sample_functions,
            );
        }
    }

    /// An artifact with an empty `languages` array is rejected at load time:
    /// it would pass every structural check while turning the corpus lens
    /// into a silent no-op, indistinguishable at lookup time from a file
    /// genuinely absent from the corpus.
    #[test]
    fn empty_languages_artifact_is_rejected() {
        let mut art = CalibrationArtifact::from_slice(EMBEDDED_WORLD_BYTES)
            .expect("embedded world corpus must be a valid artifact");
        art.languages.clear();
        let err = art
            .validate()
            .expect_err("an empty-languages artifact must not validate");
        assert!(err.contains("no languages"), "unexpected message: {err}");
    }

    /// A manifest with two `[[repos]]` blocks round-trips: sources, pinned
    /// SHAs, and advisory languages all parse into the typed model.
    #[test]
    fn manifest_parses_repos_and_pins() {
        let toml = r#"
            [[repos]]
            source = "https://github.com/example/one"
            sha = "abc123"
            languages = ["rust", "python"]

            [[repos]]
            source = "/local/path/to/two"
            sha = "def456"
        "#;
        let manifest: CorpusManifest = toml::from_str(toml).expect("parse manifest");
        assert_eq!(manifest.repos.len(), 2);
        assert_eq!(manifest.repos[0].source, "https://github.com/example/one");
        assert_eq!(manifest.repos[0].sha, "abc123");
        assert_eq!(manifest.repos[0].languages, ["rust", "python"]);
        // The second repo omits `languages`; it defaults to empty rather than
        // failing to parse.
        assert_eq!(manifest.repos[1].source, "/local/path/to/two");
        assert!(manifest.repos[1].languages.is_empty());
    }

    /// An empty manifest (no `[[repos]]`) parses to an empty repo list rather
    /// than erroring — the command then attempts zero repos.
    #[test]
    fn manifest_without_repos_is_empty() {
        let manifest: CorpusManifest = toml::from_str("").expect("parse empty manifest");
        assert!(manifest.repos.is_empty());
    }

    /// A non-placeholder vintage is surfaced (the "real artifact → Some"
    /// path), proving the filter keys on the vintage prefix alone.
    #[test]
    fn from_slice_accepts_a_real_vintage() {
        // One observed function, so the artifact carries a language — an
        // empty artifact is rejected by validate() regardless of vintage.
        let mut obs = LangObservations::new();
        obs.observe("rust", "cognitive", 3.0);
        let art = build_from_observations("world-2026-07", "2026-07-01T00:00:00Z", &obs);
        assert!(!art.corpus_vintage.starts_with(PLACEHOLDER_VINTAGE_PREFIX));
        // Round-trips through the same validate() path embedded_world() uses.
        let bytes = serde_json::to_vec(&art).expect("serialize");
        let back = CalibrationArtifact::from_slice(&bytes).expect("valid real-vintage artifact");
        assert_eq!(back.corpus_vintage, "world-2026-07");
    }

    // ── conditional-tail transform ──────────────────────────────────────────

    /// A single-language artifact carrying `cognitive` breakpoints verbatim, so
    /// a test controls the exact zeros share and tail shape.
    fn cognitive_artifact(
        language: &str,
        sample_functions: u64,
        q: Vec<f64>,
    ) -> CalibrationArtifact {
        CalibrationArtifact {
            format_version: CALIBRATION_FORMAT_VERSION,
            corpus_vintage: "test-fixture".into(),
            generated_at: "2026-01-01T00:00:00Z".into(),
            repos_included: 1,
            repos_attempted: 1,
            languages: vec![LanguageTable {
                language: language.into(),
                sample_functions,
                strata: vec![Stratum {
                    sloc_min: 0,
                    sloc_max: u64::MAX,
                    metrics: vec![MetricQuantiles {
                        metric: "cognitive".into(),
                        quantiles: q,
                    }],
                }],
            }],
            repo_metrics: None,
        }
    }

    /// Breakpoints with a controlled trivial (zero) prefix: indices `0..=zeros`
    /// hold `0`, then a unit ramp. With `zeros = 300` the trivial share is
    /// exactly `300 / 1000 = 0.30`, hand-verified below.
    fn zeros_prefix_breakpoints(zeros: usize) -> Vec<f64> {
        (0..QUANTILE_POINTS)
            .map(|i| {
                if i <= zeros {
                    0.0
                } else {
                    count_to_f64(i - zeros)
                }
            })
            .collect()
    }

    /// `trivial_share` reads `p0` off the breakpoints as the highest index still
    /// holding a trivial value, normalised to `[0, 1]` — hand-verified against
    /// fixtures whose zeros share is known exactly, including the 0% and 100%
    /// endpoints.
    #[test]
    fn trivial_share_matches_controlled_zeros_prefix() {
        // 30% zeros: breakpoints 0..=300 are 0 ⇒ i_max = 300 ⇒ p0 = 0.30.
        assert!((trivial_share(&zeros_prefix_breakpoints(300)) - 0.30).abs() < 1e-12);
        // 50% zeros: breakpoints 0..=500 are 0 ⇒ p0 = 0.50.
        assert!((trivial_share(&zeros_prefix_breakpoints(500)) - 0.50).abs() < 1e-12);
        // 0% zeros: all breakpoints strictly positive ⇒ p0 = 0.0.
        let all_positive: Vec<f64> = (0..QUANTILE_POINTS).map(|i| count_to_f64(i + 1)).collect();
        assert!(trivial_share(&all_positive).abs() < 1e-12);
        // 100% zeros: every breakpoint index holds 0 ⇒ p0 = 1.0.
        assert!((trivial_share(&vec![0.0; QUANTILE_POINTS]) - 1.0).abs() < 1e-12);
    }

    /// The conditional-tail lookup re-spreads the non-trivial tail across
    /// `[0, 1]`: on the 30%-zeros fixture a value just above the plateau maps to
    /// ~0, the corpus maximum to 1, and a hand-computed mid-tail value to 0.5.
    #[test]
    fn conditional_tail_percentile_spreads_the_tail() {
        // p0 = 0.30, so the ramp value that sits at raw percentile 0.65 maps to
        // (0.65 − 0.30) / 0.70 = 0.50. That value is `q[650] = 650 − 300 = 350`.
        let art = cognitive_artifact("rust", 1000, zeros_prefix_breakpoints(300));
        let ct = |v: f64| conditional_tail_percentile(&art, "rust", "cognitive", v).unwrap();
        assert!((ct(350.0) - 0.50).abs() < 1e-9, "mid-tail value halves");
        // A value just above the zero plateau lands at the bottom of the tail.
        assert!(ct(0.5) < 1e-3, "just above the plateau ⇒ ~0");
        // The corpus maximum (q[1000] = 700) saturates the tail at 1.
        assert!((ct(700.0) - 1.0).abs() < 1e-12, "corpus max ⇒ 1");
        // A larger-than-corpus value clamps to 1 (never above).
        assert!((ct(10_000.0) - 1.0).abs() < 1e-12);
    }

    /// Edges of the transform: a trivial file (value ≤ 0) reads exactly 0 on a
    /// covered pool; an all-trivial pool, a below-floor language, and an unknown
    /// language / metric all omit the anchor (`None`), never a fabricated value.
    #[test]
    fn conditional_tail_percentile_edges_and_omissions() {
        let art = cognitive_artifact("rust", 1000, zeros_prefix_breakpoints(300));
        // Trivial file (cognitive 0) on a covered, non-empty tail ⇒ Some(0.0).
        assert_eq!(
            conditional_tail_percentile(&art, "rust", "cognitive", 0.0),
            Some(0.0)
        );
        // All-trivial pool (empty tail) ⇒ None, matching the no-anchor path.
        let all_trivial = cognitive_artifact("rust", 1000, vec![0.0; QUANTILE_POINTS]);
        assert_eq!(
            conditional_tail_percentile(&all_trivial, "rust", "cognitive", 5.0),
            None
        );
        // Language pooled below the sample floor ⇒ None (same floor as `percentile`).
        let thin = cognitive_artifact("rust", MIN_LANG_SAMPLE - 1, zeros_prefix_breakpoints(300));
        assert_eq!(
            conditional_tail_percentile(&thin, "rust", "cognitive", 350.0),
            None
        );
        // Unknown language and unknown metric ⇒ None.
        assert_eq!(
            conditional_tail_percentile(&art, "cobol", "cognitive", 350.0),
            None
        );
        assert_eq!(
            conditional_tail_percentile(&art, "rust", "cyclomatic", 350.0),
            None
        );
    }

    /// With no trivial mass (`p0 = 0`) the transform is the identity: `cp_tail`
    /// equals the raw [`percentile`], so a corpus without a zero plateau is
    /// unaffected by the conditioning.
    #[test]
    fn conditional_tail_percentile_is_identity_without_trivial_mass() {
        let q: Vec<f64> = (0..QUANTILE_POINTS).map(|i| count_to_f64(i + 1)).collect();
        let art = cognitive_artifact("rust", 1000, q);
        // Raw percentile of `q[500] = 501` is 0.5; with p0 = 0 the tail lookup
        // returns the same value.
        let raw = percentile(&art, "rust", "cognitive", 501.0).unwrap().p;
        let tail = conditional_tail_percentile(&art, "rust", "cognitive", 501.0).unwrap();
        assert!((raw - 0.5).abs() < 1e-12);
        assert!((tail - raw).abs() < 1e-12);
    }
}

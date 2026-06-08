//! Run-time configuration for the codelore pipeline. Defaults match
//! code-maat for parity; see spec §1.1.

use std::path::PathBuf;
use time::Date;

/// Complexity sampling strategy. See spec §4.4.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ComplexitySample {
    /// Parse every file at HEAD only. Plan 3 default; Plan 4 ships this.
    #[default]
    Head,
    /// Adaptive: every commit for low-revision files; sampled for high-revision.
    /// Plan 5 work.
    Adaptive,
    /// Parse every revision of every changed file. Plan 5 work.
    Full,
}

/// Time-bucket granularity for coupling-family analyses (modern replacement
/// for code-maat's `--temporal-period`). Backed by `DuckDB`'s `date_trunc`
/// — produces clean non-overlapping buckets rather than the sliding-window
/// duplication code-maat does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TimeBucket {
    Day,
    Week,
    Month,
}

impl TimeBucket {
    /// SQL string for `date_trunc(?, date)`. Lowercase per `DuckDB`'s
    /// `datepart` accepted values.
    #[must_use]
    pub fn as_sql_unit(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[allow(clippy::struct_excessive_bools)] // CLI config bag mirrors many independent knobs
pub struct Options {
    // Input
    pub repo_path: PathBuf,
    pub after: Option<Date>,
    pub before: Option<Date>,
    pub commit_range: Option<String>,

    // Aggregation (Plan 4 — left here so Options shape is stable from v1)
    pub group_file: Option<PathBuf>,
    pub team_map_file: Option<PathBuf>,
    pub temporal_period_days: Option<u32>,

    // Analysis thresholds — code-maat parity
    pub min_revs: u32,
    pub min_shared_revs: u32,
    pub min_coupling_pct: u8,
    pub max_coupling_pct: u8,
    pub max_changeset_size: u32,
    pub fisher_significance: f64,

    // Specific analyses
    pub message_regex: Option<String>,
    pub age_time_now: Option<Date>,

    // Output
    pub rows_limit: Option<u32>,
    pub verbose_results: bool,
    pub include_merges: bool,
    pub strict_grouping: bool,
    pub complexity_sample: ComplexitySample,

    // Plan 7: clone detection. Minimum AST node count (post-skip) for a
    // function to be eligible as a clone-family member. Default 30 ≈ 5-8
    // statements after identifier/literal normalization — keeps trivial
    // getters/setters and empty constructors out of clone reports.
    pub min_clone_node_count: u32,

    // Plan 8 §2 Task 8: path-glob patterns to exclude from analyses.
    // Built from `--exclude` flags + any `.codeloreignore` file in repo_path.
    // Currently honored by `clones`; other analyses gain support in Plan 9.
    pub exclude_patterns: Vec<String>,

    // Plan 8 §6: clone-coupling false-positive mitigations (research brief
    // a0a6cf3534a65a643). Defaults locked from the brief.
    //
    /// Minimum `shared_revs` for a clone pair to count as "live". Below this
    /// floor the Fisher test is unreliable (small contingency-table cells).
    /// Default 3.
    pub min_clone_shared_revs: u32,
    /// Minimum similarity for a clone pair to enter the coupling intersection.
    /// `SourcererCC`'s BCB benchmark found precision/recall optimum at 0.70.
    /// Default 0.70. T1+T2 always = 1.0 today; this matters once T3 (`MinHash`) lands.
    pub clone_similarity_floor: f64,
    /// Skip clone pairs whose two files share the same parent directory
    /// (intentional structural mirroring like `foo_test.rs` ↔ `foo.rs`).
    /// Default `true`.
    pub clone_skip_same_dir: bool,

    // code-maat parity additions (2026-06-08 parity sprint).
    /// `SoC` threshold for the `soc` analysis. `None` = drop solo commits
    /// (default 1). Modern replacement for code-maat's overloaded use of
    /// `--min-revs` to mean "minimum `SoC` sum" in this one analysis.
    pub min_soc: Option<u32>,

    /// Time-bucket granularity for coupling-family analyses. `None` = raw
    /// commit grain (no bucketing). When set, coupling and friends aggregate
    /// changes by the bucket-truncated date.
    pub time_bucket: Option<TimeBucket>,

    /// Migration-helper flag. When `true`, flips internal defaults to match
    /// legacy code-maat output bit-for-bit (lying column headers, arbitrary
    /// tiebreaks, sliding-window temporal-period, etc.). Off by default —
    /// the modern surface is the recommendation; this flag exists so users
    /// with dashboards parsing code-maat CSV verbatim aren't broken on day
    /// one of migration.
    pub code_maat_compat: bool,
}

impl Options {
    /// Stable JSON-serialized snapshot of the full struct, used for cache
    /// keying and provenance manifest recording.
    ///
    /// Adding a new field to `Options` automatically propagates to BOTH the
    /// cache key and the provenance manifest with zero per-field maintenance
    /// — fixes a historical drift where new fields silently weren't hashed.
    ///
    /// Normalizations applied to keep the canonical form stable:
    /// - `exclude_patterns` is sorted (insertion order from CLI flags vs.
    ///   `.codeloreignore` parsing doesn't perturb the form).
    /// - `rows_limit` is dropped (cosmetic — affects only output truncation,
    ///   not the underlying data; setting `--rows 10` on a cached analysis
    ///   should still hit the cache).
    /// - `verbose_results` is dropped (logging knob, not a data knob).
    ///
    /// # Panics
    ///
    /// Panics only if `Options` ever gains a field whose type does not
    /// implement `Serialize`. Caught at compile time via the derive on the
    /// struct; this panic is unreachable in well-formed code.
    #[must_use]
    pub fn canonical_json(&self) -> serde_json::Value {
        let mut snapshot = self.clone();
        snapshot.exclude_patterns.sort();
        // Cosmetic knobs — exclude from canonical form so the cache hits
        // when they change.
        snapshot.rows_limit = None;
        snapshot.verbose_results = false;
        serde_json::to_value(&snapshot)
            .expect("Options derives Serialize and all fields are Serialize")
    }
}

impl Default for Options {
    fn default() -> Self {
        Self {
            repo_path: PathBuf::from("."),
            after: None,
            before: None,
            commit_range: None,
            group_file: None,
            team_map_file: None,
            temporal_period_days: None,
            min_revs: 5,
            min_shared_revs: 5,
            min_coupling_pct: 30,
            max_coupling_pct: 100,
            max_changeset_size: 30,
            fisher_significance: 0.05,
            message_regex: None,
            age_time_now: None,
            rows_limit: None,
            verbose_results: false,
            include_merges: false,
            strict_grouping: false,
            complexity_sample: ComplexitySample::Head,
            min_clone_node_count: 30,
            exclude_patterns: Vec::new(),
            min_clone_shared_revs: 3,
            clone_similarity_floor: 0.70,
            clone_skip_same_dir: true,
            // code-maat parity additions
            min_soc: None,
            time_bucket: None,
            code_maat_compat: false,
        }
    }
}

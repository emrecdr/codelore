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

    // Aggregation
    pub group_file: Option<PathBuf>,
    pub team_map_file: Option<PathBuf>,

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
    /// Print the `DuckDB` query plan + raw SQL to stderr before running
    /// the analysis. Set via `--explain`. Cosmetic — not part of the
    /// canonical cache key.
    pub explain: bool,

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
    /// tiebreaks, etc.). Off by default — the modern surface is the
    /// recommendation; this flag exists so users with dashboards parsing
    /// code-maat CSV verbatim aren't broken on day one of migration.
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
        use serde_json::json;
        use sha2::{Digest, Sha256};

        let mut snapshot = self.clone();
        snapshot.exclude_patterns.sort();
        // Cosmetic knobs — exclude from canonical form so the cache hits
        // when they change.
        snapshot.rows_limit = None;
        snapshot.verbose_results = false;
        snapshot.explain = false;
        let mut canon = serde_json::to_value(&snapshot)
            .expect("Options derives Serialize and all fields are Serialize");

        // Mutable-config content hashing: PATHS alone don't capture file
        // edits — a user editing the team-map CSV in place would otherwise
        // see stale cached results because the cache key would be byte-
        // equal across the edit. Replace the path strings with sha-256
        // digests of the file content so any edit invalidates the cache.
        // Falls back to `null` for missing files (treated as "no
        // override"), which is what code-maat does today.
        let digest_of = |path: &std::path::Path| -> Option<String> {
            std::fs::read(path).ok().map(|bytes| {
                let mut h = Sha256::new();
                h.update(&bytes);
                hex::encode(h.finalize())
            })
        };

        let team_map_digest = self.team_map_file.as_deref().and_then(digest_of);
        let group_file_digest = self.group_file.as_deref().and_then(digest_of);
        let bots_digest = digest_of(&self.repo_path.join(".codelorebots"));

        if let serde_json::Value::Object(map) = &mut canon {
            // The path strings themselves don't go into the canonical
            // form — only the content hash does. Two runs from different
            // working trees with identical team-map CONTENT must hit the
            // same cache entry.
            map.remove("team_map_file");
            map.remove("group_file");
            map.insert("team_map_digest".to_string(), json!(team_map_digest));
            map.insert("group_file_digest".to_string(), json!(group_file_digest));
            map.insert("codelorebots_digest".to_string(), json!(bots_digest));
        }
        canon
    }

    /// Clone with `rows_limit = None`. Use this WHENEVER a composite analysis
    /// invokes another analysis as an internal step (e.g. `code-health` and
    /// `clone-coupling` both invoke `run_coupling` to materialize the global
    /// coupling graph). Without this wrapper, `--rows 10` flows into the
    /// inner SQL's `LIMIT ?`, the inner result truncates to the top 10 pairs,
    /// and the composite result is computed over that arbitrary subset —
    /// e.g. coupling-centrality scores end up counting partners from a
    /// 10-pair sliver of the full graph. Worse: `canonical_json` deliberately
    /// drops `rows_limit` from the cache key (because the user-visible
    /// row-cap is cosmetic), so the corrupted result gets cached under the
    /// no-row-limit cache key and poisons subsequent runs.
    #[must_use]
    pub fn with_no_row_limit(&self) -> Self {
        Self {
            rows_limit: None,
            ..self.clone()
        }
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
            min_revs: crate::constants::DEFAULT_MIN_REVS,
            min_shared_revs: crate::constants::DEFAULT_MIN_SHARED_REVS,
            min_coupling_pct: crate::constants::DEFAULT_MIN_COUPLING_PCT,
            max_coupling_pct: crate::constants::DEFAULT_MAX_COUPLING_PCT,
            max_changeset_size: crate::constants::DEFAULT_MAX_CHANGESET_SIZE,
            fisher_significance: crate::constants::DEFAULT_FISHER_SIGNIFICANCE,
            message_regex: None,
            age_time_now: None,
            rows_limit: None,
            verbose_results: false,
            include_merges: false,
            explain: false,
            strict_grouping: false,
            complexity_sample: ComplexitySample::Head,
            min_clone_node_count: crate::constants::DEFAULT_MIN_CLONE_NODE_COUNT,
            exclude_patterns: Vec::new(),
            min_clone_shared_revs: crate::constants::DEFAULT_MIN_CLONE_SHARED_REVS,
            clone_similarity_floor: crate::constants::DEFAULT_CLONE_SIMILARITY_FLOOR,
            clone_skip_same_dir: crate::constants::DEFAULT_CLONE_SKIP_SAME_DIR,
            // code-maat parity additions
            min_soc: None,
            time_bucket: None,
            code_maat_compat: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Options;

    #[test]
    fn defaults_use_the_constants_so_drift_is_caught_at_compile_time() {
        use crate::constants::*;
        let d = Options::default();
        assert_eq!(d.min_revs, DEFAULT_MIN_REVS);
        assert_eq!(d.min_shared_revs, DEFAULT_MIN_SHARED_REVS);
        assert_eq!(d.min_coupling_pct, DEFAULT_MIN_COUPLING_PCT);
        assert_eq!(d.max_coupling_pct, DEFAULT_MAX_COUPLING_PCT);
        assert_eq!(d.max_changeset_size, DEFAULT_MAX_CHANGESET_SIZE);
        assert!((d.fisher_significance - DEFAULT_FISHER_SIGNIFICANCE).abs() < 1e-12);
        assert_eq!(d.min_clone_node_count, DEFAULT_MIN_CLONE_NODE_COUNT);
        assert_eq!(d.min_clone_shared_revs, DEFAULT_MIN_CLONE_SHARED_REVS);
        assert!((d.clone_similarity_floor - DEFAULT_CLONE_SIMILARITY_FLOOR).abs() < 1e-12);
        assert_eq!(d.clone_skip_same_dir, DEFAULT_CLONE_SKIP_SAME_DIR);
    }

    #[test]
    fn with_no_row_limit_clears_rows_limit_only() {
        let opts = Options {
            rows_limit: Some(10),
            min_revs: 7,
            min_coupling_pct: 42,
            ..Options::default()
        };
        let stripped = opts.with_no_row_limit();
        assert_eq!(stripped.rows_limit, None, "rows_limit must be cleared");
        // All other knobs must round-trip unchanged.
        assert_eq!(stripped.min_revs, 7);
        assert_eq!(stripped.min_coupling_pct, 42);
        // Original must be untouched (helper returns a clone).
        assert_eq!(opts.rows_limit, Some(10));
    }

    #[test]
    fn canonical_json_invalidates_when_team_map_content_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("team-map.csv");
        std::fs::write(&path, "author,team\nalice@x,Backend\n").unwrap();
        let base = Options {
            team_map_file: Some(path.clone()),
            ..Options::default()
        };
        let v1 = base.canonical_json();

        // Edit the team-map in place — path unchanged, content changes.
        std::fs::write(&path, "author,team\nalice@x,Frontend\n").unwrap();
        let v2 = base.canonical_json();

        assert_ne!(
            v1, v2,
            "canonical_json must differ when team-map content changes"
        );
    }

    #[test]
    fn canonical_json_strips_team_map_path_keeps_only_digest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("team-map.csv");
        std::fs::write(&path, "author,team\n").unwrap();
        let opts = Options {
            team_map_file: Some(path),
            ..Options::default()
        };
        let canon = opts.canonical_json();
        // Two runs from different machines with the same team-map content
        // must hit the same cache entry. Hence path is stripped.
        let s = canon.to_string();
        assert!(!s.contains("team-map.csv"), "path leaked into canonical form: {s}");
        assert!(s.contains("team_map_digest"), "digest field missing: {s}");
    }

    #[test]
    fn canonical_json_drops_rows_limit_so_caches_hit() {
        let a = Options {
            rows_limit: Some(10),
            ..Options::default()
        };
        let b = Options {
            rows_limit: Some(99),
            ..Options::default()
        };
        // The two Options differ only in a cosmetic field; their canonical
        // forms must be byte-equal so a cached result hits regardless of
        // the user's `--rows N` choice.
        assert_eq!(a.canonical_json(), b.canonical_json());
    }
}

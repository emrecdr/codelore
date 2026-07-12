//! Run-time configuration for the codelore pipeline. Defaults match
//! code-maat for parity; see spec §1.1.

use std::path::PathBuf;
use time::Date;

/// Complexity sampling strategy. See spec §4.4.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ComplexitySample {
    /// Parse every file at HEAD only. The default.
    #[default]
    Head,
    /// Adaptive: every commit for low-revision files; sampled for high-revision.
    Adaptive,
    /// Parse every revision of every changed file.
    Full,
}

/// Time-bucket granularity for coupling-family analyses (modern replacement
/// for code-maat's `--temporal-period`). Backed by `DuckDB`'s `date_trunc`
/// — produces clean non-overlapping buckets rather than the sliding-window
/// duplication code-maat does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
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
    pub include_merges: bool,
    pub strict_grouping: bool,
    pub complexity_sample: ComplexitySample,
    /// Print the `DuckDB` query plan + raw SQL to stderr before running
    /// the analysis. Set via `--explain`. Cosmetic — not part of the
    /// canonical cache key.
    pub explain: bool,
    /// When `true` (default), aggregations follow file renames through
    /// `changes.rename_from`. A file's pre-rename history is merged
    /// onto its current canonical path. When `false`, every commit's
    /// path is treated literally — matches code-maat's historic
    /// behaviour (it uses `--no-renames`). `--code-maat-compat`
    /// implies `use_canonical_lineage = false` for bit-for-bit parity.
    pub use_canonical_lineage: bool,

    // Minimum AST node count (post-skip) for a
    // function to be eligible as a clone-family member. Default 30 ≈ 5-8
    // statements after identifier/literal normalization — keeps trivial
    // getters/setters and empty constructors out of clone reports.
    pub min_clone_node_count: u32,

    // Path-glob patterns to exclude from analyses.
    // Built from `--exclude` flags + any `.codeloreignore` file in repo_path.
    // Currently honored by `clones`.
    pub exclude_patterns: Vec<String>,

    /// When false (default), `paths_filter::PathsFilter` auto-respects
    /// `.gitignore`, `.git/info/exclude`, and `.codeloreignore` at the
    /// repo root so vendored deps (`node_modules`), build outputs
    /// (`target`, `dist`), lockfiles, locales, etc. don't show up in
    /// hotspots or skew Kamei features. Set to `true` (via
    /// `--include-ignored`) to analyse everything regardless of those
    /// files.
    pub include_ignored: bool,

    /// T8 (knowledge-islands): an author is considered "departed" if
    /// their most recent commit anywhere in the repo is older than this
    /// many days at the anchor moment. Default 90.
    /// See `constants::DEFAULT_DEPARTED_THRESHOLD_DAYS` for the
    /// rationale on this default.
    pub departed_threshold_days: u32,

    // Clone-coupling false-positive mitigations (research brief
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

    /// Trailing window in days for activity-scoped analyses. Anchored to the
    /// repo's last commit date (not wall-clock time) so results are
    /// reproducible on old or archived repos. Valid range: 1–3650.
    /// Default: 90. Set via `--window-days`.
    pub window_days: u32,

    /// Knowledge model for `bus-factor`. Valid values: `"commits"` (default,
    /// Filatov 2010 — greedy coverage of ≥80% of commits) or `"doe"`
    /// (Cury & Avelino SBES'24 truck-factor procedure — greedy removal of
    /// the author with the most expert files until >50% of files lack an
    /// expert). Set via `--knowledge-model`.
    pub knowledge_model: String,
    /// Hunk-overlap window for rework detection in `delivery-metrics`.
    /// Pairs of hunks touching the same path where the second commit's
    /// author-date falls within this many days of the first are counted
    /// as rework candidates. Valid range: 1–365. Default: 21.
    /// Set via `--rework-window-days`.
    pub rework_window_days: u32,
    /// Glob pattern for filtering release tags in `release-cadence`.
    /// Only tags whose short name matches this glob are included.
    /// Must be non-empty. Default: `"v*"`. Set via `--release-tag-glob`.
    pub release_tag_glob: String,
    /// Target file path for analyses that operate on a single file.
    /// Currently used only by `function-xray`. Set via `--target`.
    /// Validation that the value is present when required lives in the
    /// dispatch arm, not `Options::validate`, to avoid cross-analysis coupling.
    pub target: Option<String>,

    /// Corpus-calibration artifact for the code-health corpus-percentile lens.
    /// `Some(path)` overrides the embedded world artifact with a hand-built or
    /// org-specific one; `None` falls back to the embedded artifact (or no lens
    /// when that is a placeholder). A per-invocation selector that never affects
    /// ingest, so `canonical_json` drops it from the cache key and instead hashes
    /// its content into `calibration_digest` for provenance. Set via
    /// `--calibration`.
    pub calibration: Option<PathBuf>,
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
        // Two exclusion categories, both keyed on "ingest never reads it":
        // cosmetic knobs (affect only output shaping) and per-invocation
        // selectors (pick what to analyse from already-ingested facts). A
        // future field belongs here iff changing it cannot change any row
        // the ingest writes.
        snapshot.rows_limit = None; // cosmetic — output truncation
        snapshot.explain = false; // cosmetic — help text
        // `target` is a per-invocation selector: it picks which file the
        // function analyses read, but ingest never sees it — caching
        // per-target would produce a full-size duplicate DB file for every
        // distinct `--target` path.
        snapshot.target = None;
        // `calibration` is a per-invocation selector for the code-health corpus
        // lens: it changes an additive output column, never any row ingest
        // writes. Drop the path here; its CONTENT is hashed into
        // `calibration_digest` below so provenance still captures which artifact
        // shaped the run.
        snapshot.calibration = None;
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

        // Explicit `--team-map-file` wins; otherwise the same auto-discovery
        // path `ingest()` uses (`<repo>/.codelore-teams`) must be hashed so
        // edits to the auto-loaded file also invalidate the cache.
        let team_map_digest = self
            .team_map_file
            .as_deref()
            .and_then(digest_of)
            .or_else(|| digest_of(&self.repo_path.join(".codelore-teams")));
        let group_file_digest = self.group_file.as_deref().and_then(digest_of);
        // Hash the calibration artifact's content, not its path, so two runs
        // pointing at byte-identical artifacts from different locations share a
        // provenance stamp — and editing the artifact in place is visible.
        let calibration_digest = self.calibration.as_deref().and_then(digest_of);
        let bots_digest = digest_of(&self.repo_path.join(".codelorebots"));
        // `.mailmap` is read at walk time by gix to canonicalize author
        // identities; edits change every author-bearing analysis but were
        // invisible to the cache key.
        let mailmap_digest = digest_of(&self.repo_path.join(".mailmap"));
        // `.codeloreignore` is parsed by clones extraction to filter files;
        // edits change which files get fingerprinted and were invisible too.
        let codeloreignore_digest = digest_of(&self.repo_path.join(".codeloreignore"));

        if let serde_json::Value::Object(map) = &mut canon {
            // The path strings themselves don't go into the canonical
            // form — only the content hash does. Two runs from different
            // working trees with identical team-map CONTENT must hit the
            // same cache entry.
            map.remove("team_map_file");
            map.remove("group_file");
            map.remove("calibration");
            map.insert("team_map_digest".to_string(), json!(team_map_digest));
            map.insert("group_file_digest".to_string(), json!(group_file_digest));
            map.insert("calibration_digest".to_string(), json!(calibration_digest));
            map.insert("codelorebots_digest".to_string(), json!(bots_digest));
            map.insert("mailmap_digest".to_string(), json!(mailmap_digest));
            map.insert(
                "codeloreignore_digest".to_string(),
                json!(codeloreignore_digest),
            );
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

    /// Like [`with_no_row_limit`], but also lowers `min_shared_revs` to
    /// at most `min_clone_shared_revs`. Used by `clone-coupling`'s
    /// internal call to `run_coupling`.
    ///
    /// Without this override, the inner coupling call applies the
    /// default `min_shared_revs = 5` even though `clone-coupling`'s own
    /// floor is `min_clone_shared_revs = 3`. Clone pairs that
    /// co-changed exactly 3 or 4 times were silently dropped by the
    /// inner call — `clone-coupling` then filtered the (already
    /// trimmed) result by `min_clone_shared_revs`, but the missing
    /// pairs were already gone.
    ///
    /// The min-of-both semantics is the safer choice: if a user
    /// explicitly LOWERED `--min-shared-revs` below
    /// `min-clone-shared-revs`, honour that; if they didn't, drop the
    /// floor to the clone-coupling threshold so we don't lose anything.
    ///
    /// [`with_no_row_limit`]: Self::with_no_row_limit
    #[must_use]
    pub fn for_clone_coupling_inner_coupling(&self) -> Self {
        Self {
            rows_limit: None,
            min_shared_revs: self.min_shared_revs.min(self.min_clone_shared_revs),
            ..self.clone()
        }
    }

    /// Check cross-field invariants. Caller (typically the CLI boundary)
    /// runs this once after constructing `Options` so pathological flag
    /// combinations fail loudly instead of silently producing empty
    /// output. Each invariant catches a real footgun from the CLI
    /// surface.
    ///
    /// # Errors
    ///
    /// Returns [`crate::CodeLoreError::InvalidOptions`] (exit-2
    /// configuration-error category) with a message naming the offending
    /// field pair.
    pub fn validate(&self) -> crate::Result<()> {
        if self.min_coupling_pct > self.max_coupling_pct {
            return Err(crate::CodeLoreError::InvalidOptions(format!(
                "--min-coupling ({}) must be <= --max-coupling ({})",
                self.min_coupling_pct, self.max_coupling_pct
            )));
        }
        if !(0.0..=1.0).contains(&self.clone_similarity_floor) {
            return Err(crate::CodeLoreError::InvalidOptions(format!(
                "--clone-similarity-floor must be in [0.0, 1.0]; got {}",
                self.clone_similarity_floor
            )));
        }
        if !(0.0..=1.0).contains(&self.fisher_significance) {
            return Err(crate::CodeLoreError::InvalidOptions(format!(
                "--fisher-significance must be in [0.0, 1.0]; got {}",
                self.fisher_significance
            )));
        }
        if let (Some(after), Some(before)) = (self.after, self.before)
            && after > before
        {
            return Err(crate::CodeLoreError::InvalidOptions(format!(
                "--after ({after}) must be <= --before ({before})"
            )));
        }
        if !(1..=3650).contains(&self.window_days) {
            return Err(crate::CodeLoreError::InvalidOptions(format!(
                "--window-days must be in [1, 3650]; got {}",
                self.window_days
            )));
        }
        if !matches!(self.knowledge_model.as_str(), "commits" | "doe") {
            return Err(crate::CodeLoreError::InvalidOptions(format!(
                "--knowledge-model must be 'commits' or 'doe'; got '{}'",
                self.knowledge_model
            )));
        }
        if !(1..=365).contains(&self.rework_window_days) {
            return Err(crate::CodeLoreError::InvalidOptions(format!(
                "--rework-window-days must be in [1, 365]; got {}",
                self.rework_window_days
            )));
        }
        if self.release_tag_glob.is_empty() {
            return Err(crate::CodeLoreError::InvalidOptions(
                "--release-tag-glob must not be empty".to_string(),
            ));
        }
        Ok(())
    }
}

impl Default for Options {
    fn default() -> Self {
        Self {
            repo_path: PathBuf::from("."),
            after: None,
            before: None,
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
            include_merges: false,
            explain: false,
            use_canonical_lineage: true,
            strict_grouping: false,
            complexity_sample: ComplexitySample::Head,
            min_clone_node_count: crate::constants::DEFAULT_MIN_CLONE_NODE_COUNT,
            exclude_patterns: Vec::new(),
            include_ignored: false,
            min_clone_shared_revs: crate::constants::DEFAULT_MIN_CLONE_SHARED_REVS,
            clone_similarity_floor: crate::constants::DEFAULT_CLONE_SIMILARITY_FLOOR,
            departed_threshold_days: crate::constants::DEFAULT_DEPARTED_THRESHOLD_DAYS,
            clone_skip_same_dir: crate::constants::DEFAULT_CLONE_SKIP_SAME_DIR,
            // code-maat parity additions
            min_soc: None,
            time_bucket: None,
            code_maat_compat: false,
            window_days: crate::constants::DEFAULT_WINDOW_DAYS,
            knowledge_model: "commits".to_string(),
            rework_window_days: crate::constants::DEFAULT_REWORK_WINDOW_DAYS,
            release_tag_glob: crate::constants::DEFAULT_RELEASE_TAG_GLOB.to_string(),
            target: None,
            calibration: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Options;

    #[test]
    fn validate_accepts_default_options() {
        assert!(Options::default().validate().is_ok());
    }

    #[test]
    fn validate_rejects_inverted_coupling_range() {
        let opts = Options {
            min_coupling_pct: 80,
            max_coupling_pct: 30,
            ..Options::default()
        };
        let err = opts.validate().expect_err("inverted range must fail");
        assert!(
            format!("{err}").contains("min-coupling"),
            "error must name the offending field: {err}"
        );
    }

    #[test]
    fn validate_reports_invalid_options_not_provenance() {
        // An argument conflict is a configuration error, not a
        // reproducibility-manifest (provenance) violation. Both map to
        // exit 2, but the variant must name the cause honestly so a
        // consumer matching on `Provenance` isn't tripped by a CLI typo.
        let opts = Options {
            min_coupling_pct: 80,
            max_coupling_pct: 30,
            ..Options::default()
        };
        let err = opts.validate().expect_err("inverted range must fail");
        assert!(
            matches!(err, crate::CodeLoreError::InvalidOptions(_)),
            "arg-conflict must be InvalidOptions, got: {err:?}"
        );
        assert_eq!(
            err.exit_code(),
            2,
            "config errors stay in the exit-2 bucket"
        );
    }

    #[test]
    fn validate_rejects_clone_similarity_floor_out_of_range() {
        let opts = Options {
            clone_similarity_floor: 1.5,
            ..Options::default()
        };
        assert!(opts.validate().is_err());
    }

    #[test]
    fn validate_rejects_inverted_date_range() {
        use time::macros::date;
        let opts = Options {
            after: Some(date!(2026 - 06 - 09)),
            before: Some(date!(2026 - 01 - 01)),
            ..Options::default()
        };
        let err = opts.validate().expect_err("inverted dates must fail");
        assert!(format!("{err}").contains("after"), "{err}");
    }

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
    fn canonical_json_invalidates_when_mailmap_changes() {
        let dir = tempfile::tempdir().unwrap();
        let mailmap = dir.path().join(".mailmap");
        std::fs::write(&mailmap, "Real Name <real@x> <alias@x>\n").unwrap();
        let opts = Options {
            repo_path: dir.path().to_path_buf(),
            ..Options::default()
        };
        let v1 = opts.canonical_json();
        std::fs::write(&mailmap, "Real Name <real@x> <alias2@x>\n").unwrap();
        let v2 = opts.canonical_json();
        assert_ne!(v1, v2, ".mailmap edit must invalidate cache key");
    }

    #[test]
    fn canonical_json_invalidates_when_codeloreignore_changes() {
        let dir = tempfile::tempdir().unwrap();
        let ignore = dir.path().join(".codeloreignore");
        std::fs::write(&ignore, "vendor/\n").unwrap();
        let opts = Options {
            repo_path: dir.path().to_path_buf(),
            ..Options::default()
        };
        let v1 = opts.canonical_json();
        std::fs::write(&ignore, "vendor/\nnode_modules/\n").unwrap();
        let v2 = opts.canonical_json();
        assert_ne!(v1, v2, ".codeloreignore edit must invalidate cache key");
    }

    #[test]
    fn canonical_json_invalidates_when_auto_discovered_team_map_changes() {
        // No --team-map-file flag: ingest auto-discovers `<repo>/.codelore-teams`.
        // canonical_json must hash THAT path's content too — otherwise editing
        // the auto-discovered file silently keeps the same cache key.
        let dir = tempfile::tempdir().unwrap();
        let auto = dir.path().join(".codelore-teams");
        std::fs::write(&auto, "author,team\nalice@x,Backend\n").unwrap();
        let opts = Options {
            repo_path: dir.path().to_path_buf(),
            team_map_file: None, // auto-discovery path
            ..Options::default()
        };
        let v1 = opts.canonical_json();
        std::fs::write(&auto, "author,team\nalice@x,Frontend\n").unwrap();
        let v2 = opts.canonical_json();
        assert_ne!(
            v1, v2,
            "auto-discovered .codelore-teams content must invalidate cache key"
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
        assert!(
            !s.contains("team-map.csv"),
            "path leaked into canonical form: {s}"
        );
        assert!(s.contains("team_map_digest"), "digest field missing: {s}");
    }

    #[test]
    fn window_days_default_is_90() {
        assert_eq!(Options::default().window_days, 90);
    }

    #[test]
    fn validate_rejects_window_days_zero() {
        let opts = Options {
            window_days: 0,
            ..Options::default()
        };
        let err = opts.validate().expect_err("window_days=0 must fail");
        assert!(
            format!("{err}").contains("window-days"),
            "error must name the offending field: {err}"
        );
    }

    #[test]
    fn validate_rejects_window_days_above_3650() {
        let opts = Options {
            window_days: 3651,
            ..Options::default()
        };
        let err = opts.validate().expect_err("window_days=3651 must fail");
        assert!(
            format!("{err}").contains("window-days"),
            "error must name the offending field: {err}"
        );
    }

    #[test]
    fn validate_accepts_window_days_boundary_values() {
        for days in [1, 3650] {
            let opts = Options {
                window_days: days,
                ..Options::default()
            };
            assert!(
                opts.validate().is_ok(),
                "window_days={days} must be accepted"
            );
        }
    }

    #[test]
    fn canonical_json_invalidates_when_calibration_content_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("world.calib.json");
        std::fs::write(&path, br#"{"format_version":1}"#).unwrap();
        let base = Options {
            calibration: Some(path.clone()),
            ..Options::default()
        };
        let v1 = base.canonical_json();
        // Edit the artifact in place — path unchanged, content changes.
        std::fs::write(&path, br#"{"format_version":1,"x":2}"#).unwrap();
        let v2 = base.canonical_json();
        assert_ne!(
            v1, v2,
            "calibration artifact edit must invalidate the provenance stamp"
        );
    }

    #[test]
    fn canonical_json_strips_calibration_path_keeps_only_digest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("world.calib.json");
        std::fs::write(&path, b"{}").unwrap();
        let opts = Options {
            calibration: Some(path),
            ..Options::default()
        };
        let s = opts.canonical_json().to_string();
        assert!(
            !s.contains("world.calib.json"),
            "calibration path leaked into canonical form: {s}"
        );
        assert!(
            s.contains("calibration_digest"),
            "calibration_digest field missing: {s}"
        );
    }

    #[test]
    fn canonical_json_ignores_calibration_when_content_matches() {
        // The corpus lens is additive output only — it must never split the
        // ingest cache. Two runs pointing at byte-identical artifacts in
        // different locations must produce the same canonical form.
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.calib.json");
        let b = dir.path().join("b.calib.json");
        std::fs::write(&a, b"{}").unwrap();
        std::fs::write(&b, b"{}").unwrap();
        let opts_a = Options {
            calibration: Some(a),
            ..Options::default()
        };
        let opts_b = Options {
            calibration: Some(b),
            ..Options::default()
        };
        assert_eq!(opts_a.canonical_json(), opts_b.canonical_json());
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

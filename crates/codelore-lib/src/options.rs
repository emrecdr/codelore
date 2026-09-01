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

    /// Knowledge-islands: an author is considered "departed" if
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
    /// Default 0.70. Type 1 + Type 2 always = 1.0 today; this matters once
    /// Type 3 (`MinHash`) lands.
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

    /// Apply a Benjamini-Hochberg false-discovery-rate correction across the
    /// whole family of Fisher-tested coupling pairs instead of the per-pair
    /// `fisher_p < fisher_significance` gate. Off by default — the per-test
    /// gate is unchanged. `--code-maat-compat` still bypasses both gates.
    pub fdr_correction: bool,

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

    /// Own-repo defect-calibration artifact (`defects.calib.json`, built with
    /// `codelore calibrate-defects`) whose smell weights replace the built-in
    /// code-health defaults for this run. Like `calibration`, a per-invocation
    /// selector that never affects ingest: `canonical_json` drops the path and
    /// hashes the file content into `defect_calibration_digest`. Set via
    /// `--defect-calibration`.
    pub defect_calibration: Option<PathBuf>,

    /// Skip the repo-identity guard when deliberately applying a
    /// defect-calibration artifact whose root-commit identity does not match
    /// this repository — e.g. reusing one repository's calibration on a
    /// genuinely different repo, or a shallow / non-git checkout whose root
    /// commit is unreachable (so the identity falls back to the path and no
    /// longer matches). Set via `--allow-foreign-calibration`.
    pub allow_foreign_calibration: bool,

    /// Internal ingest mode used by `codelore calibrate`: populate only the
    /// HEAD-time complexity and import facts (`entities`, `complexity_metrics`,
    /// `imports`) and leave every history table (`commits`, `changes`,
    /// `hunks`, …) empty. The full commit-history walk — and the
    /// kamei/clones passes it feeds — is skipped entirely; this also lets
    /// calibrate ingest shallow (depth-1) checkouts, which the commit
    /// walker cannot traverse.
    /// Not exposed as a CLI flag. Serialized like every other field, so
    /// `canonical_json` keys head-only cache entries apart from full ones.
    pub head_only_ingest: bool,

    /// Override the `DuckDB` spill directory (the `temp_directory` PRAGMA
    /// target used once a query exceeds the internal memory ceiling — see
    /// `constants::DEFAULT_DUCKDB_MEMORY_LIMIT`). When `None`, `facts::FactsDb`
    /// defaults to a subdirectory of the cache root, or the system temp
    /// directory when there is no cache root in play (e.g. `--no-cache`).
    /// An environment/spill selector — it changes where `DuckDB` writes
    /// scratch files, never any row an analysis reads, so `canonical_json`
    /// drops it from the cache key like `target` and `rows_limit`. Set via
    /// `--temp-dir`; validated as an existing writable directory in
    /// [`Options::validate`].
    pub temp_dir: Option<PathBuf>,
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

        // NOTE ON SCOPE: this is the FULL canonical form. It keys
        // `change_set::report_key`, which needs every report-affecting knob
        // including analysis-only ones like `min_revs` and the clone
        // thresholds. The narrower form that keys the *ingest* cache is
        // [`Self::ingest_cache_json`], which subtracts from this one — never
        // narrow this function in place, or the gate's report cache silently
        // stops noticing threshold changes and serves a wrong verdict.

        // Cache-key classification guard. Destructuring `Self` with no `..`
        // means adding a field to `Options` fails to COMPILE here until the
        // author decides which side it belongs on — silence is no longer an
        // option. The default is dangerous: a new field flows into `canon`
        // verbatim below (via serde), so an absolute-path or per-invocation
        // field would silently make every cache key machine-specific. Nothing
        // reads these bindings; the block exists solely so the two lists below
        // cannot drift from the struct. When this breaks, add the new field to
        // exactly one group and mirror the choice in the code beneath.
        let Self {
            // ----- Excluded / specially handled: dropped or content-digested
            // in the body below; MUST NOT reach `canon` verbatim. -----
            rows_limit: _,         // dropped — cosmetic output truncation
            explain: _,            // dropped — cosmetic (stderr query plan)
            target: _,             // dropped — per-invocation file selector
            temp_dir: _,           // dropped — env/spill selector
            repo_path: _,          // normalised — `cache_key` hashes the CANONICAL path
            calibration: _,        // path dropped; content → calibration_digest
            defect_calibration: _, // path dropped; content → defect_calibration_digest
            group_file: _,         // path dropped; content → group_file_digest
            team_map_file: _,      // path dropped; content → team_map_digest
            // ----- Included in the cache key: serialized verbatim into `canon`
            // (exclude_patterns is sorted first; every other field flows as-is).
            // Any change to one of these must invalidate the cache. -----
            after: _,
            before: _,
            min_revs: _,
            min_shared_revs: _,
            min_coupling_pct: _,
            max_coupling_pct: _,
            max_changeset_size: _,
            fisher_significance: _,
            message_regex: _,
            age_time_now: _,
            include_merges: _,
            strict_grouping: _,
            complexity_sample: _,
            use_canonical_lineage: _,
            min_clone_node_count: _,
            exclude_patterns: _,
            include_ignored: _,
            departed_threshold_days: _,
            min_clone_shared_revs: _,
            clone_similarity_floor: _,
            clone_skip_same_dir: _,
            min_soc: _,
            time_bucket: _,
            code_maat_compat: _,
            fdr_correction: _,
            window_days: _,
            knowledge_model: _,
            rework_window_days: _,
            release_tag_glob: _,
            allow_foreign_calibration: _,
            head_only_ingest: _,
        } = self;

        let mut snapshot = self.clone();
        snapshot.exclude_patterns.sort();
        // Two exclusion categories, both keyed on "ingest never reads it":
        // cosmetic knobs (affect only output shaping) and per-invocation
        // selectors (pick what to analyse from already-ingested facts). A
        // future field belongs here iff changing it cannot change any row
        // the ingest writes.
        snapshot.rows_limit = None; // cosmetic — output truncation
        snapshot.explain = false; // cosmetic — help text
        // `repo_path` is a per-invocation selector in the strictest sense: it
        // names WHICH repository to read, and `cache_key` already establishes
        // that identity by hashing the CANONICALISED path as its first
        // component. What arrives here is the spelling the user typed, so
        // leaving it in meant `--repo .` and `--repo $(pwd)` derived different
        // keys for one repository at one HEAD — each paying a full ingest and
        // each consuming one of the five per-repo cache slots. Normalised to a
        // constant rather than dropped, so the key's field set stays fixed.
        snapshot.repo_path = PathBuf::new();
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
        // `defect_calibration` gets the same selector treatment as
        // `calibration`: drop the path here, hash the content below.
        snapshot.defect_calibration = None;
        // `temp_dir` is a pure environment/spill selector — unlike
        // `team_map_file`/`calibration`, its CONTENT (it's a directory, not
        // a file) has no bearing on analysis output, so it is dropped
        // outright rather than digested, exactly like `target`.
        snapshot.temp_dir = None;
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
        let defect_calibration_digest = self.defect_calibration.as_deref().and_then(digest_of);
        let bots_digest = digest_of(&self.repo_path.join(".codelorebots"));
        // `.mailmap` is read at walk time by gix to canonicalize author
        // identities; edits change every author-bearing analysis but were
        // invisible to the cache key.
        let mailmap_digest = digest_of(&self.repo_path.join(".mailmap"));
        // `.codeloreignore` is parsed by clones extraction to filter files;
        // edits change which files get fingerprinted and were invisible too.
        let codeloreignore_digest = digest_of(&self.repo_path.join(".codeloreignore"));
        // `.gitignore` and `.git/info/exclude` are the other two inputs to
        // `paths_filter::PathsFilter`, which the ingest consumer applies to
        // every change row — so they decide what lands in `changes`,
        // `complexity_metrics`, `clones` and `imports`. They belong in the key
        // for the same reason `.codeloreignore` does.
        //
        // `.git/info/exclude` is the sharper of the two: it is UNTRACKED, so
        // editing it does not move `head_sha` and does not make the working
        // tree dirty. Before it was hashed here, adding a pattern to it and
        // re-running on a clean tree produced a cache hit that silently served
        // the pre-edit file set. `.gitignore` had the same gap but was masked —
        // it is tracked, so editing it dirtied the tree and the dirty-tree
        // cache-write skip happened to cover it.
        //
        // Scope caveat, inherited deliberately from the sibling digests: only
        // the repository-root files are hashed. `PathsFilter` adds exactly
        // these paths (see `paths_filter::PathsFilter::from_opts`) and does not
        // walk parent directories, so root-only is complete for what the filter
        // actually reads today; it would become partial if parent discovery
        // were ever added.
        let gitignore_digest = digest_of(&self.repo_path.join(".gitignore"));
        let info_exclude_digest = digest_of(&self.repo_path.join(".git/info/exclude"));

        if let serde_json::Value::Object(map) = &mut canon {
            // The path strings themselves don't go into the canonical
            // form — only the content hash does. Two runs from different
            // working trees with identical team-map CONTENT must hit the
            // same cache entry.
            map.remove("team_map_file");
            map.remove("group_file");
            map.remove("calibration");
            map.remove("defect_calibration");
            map.insert("team_map_digest".to_string(), json!(team_map_digest));
            map.insert("group_file_digest".to_string(), json!(group_file_digest));
            map.insert("calibration_digest".to_string(), json!(calibration_digest));
            map.insert(
                "defect_calibration_digest".to_string(),
                json!(defect_calibration_digest),
            );
            map.insert("codelorebots_digest".to_string(), json!(bots_digest));
            map.insert("mailmap_digest".to_string(), json!(mailmap_digest));
            map.insert(
                "codeloreignore_digest".to_string(),
                json!(codeloreignore_digest),
            );
            map.insert("gitignore_digest".to_string(), json!(gitignore_digest));
            map.insert(
                "info_exclude_digest".to_string(),
                json!(info_exclude_digest),
            );
        }
        canon
    }

    /// Keys in the full [`Self::canonical_json`] form that the **ingest** never
    /// reads, and so must not split the ingest cache.
    ///
    /// Verified by enumerating every `opts.<field>` token appearing anywhere
    /// under `repo/`, `facts/`, `kamei/`, `identity/`, `imports/`, `clones/`
    /// and `paths_filter.rs` — the positive direction, because a negative
    /// "none of these appear" grep cannot distinguish a true absence from a
    /// broken matcher. Twelve fields appear in code. `time_bucket` appears
    /// only inside a doc comment (`facts/ingest/grouping.rs`); the table it
    /// names is a session TEMP table built at analysis time, so it belongs
    /// here — a token count alone gets that one wrong in either direction.
    ///
    /// The two calibration digests are here for a reason the code already
    /// asserts elsewhere: the corpus lens is additive output only and "must
    /// never split the ingest cache". Neither reaches the ingest path, so
    /// keying on their content was making the code contradict its own claim.
    const ANALYSIS_ONLY_CACHE_KEYS: &'static [&'static str] = &[
        "min_revs",
        "min_shared_revs",
        "min_coupling_pct",
        "max_coupling_pct",
        "max_changeset_size",
        "fisher_significance",
        "message_regex",
        "age_time_now",
        "min_soc",
        "code_maat_compat",
        "fdr_correction",
        "window_days",
        "knowledge_model",
        "rework_window_days",
        "release_tag_glob",
        "departed_threshold_days",
        "min_clone_shared_revs",
        "clone_similarity_floor",
        "clone_skip_same_dir",
        "complexity_sample",
        "time_bucket",
        "allow_foreign_calibration",
        "calibration_digest",
        "defect_calibration_digest",
    ];

    /// The canonical form restricted to what the ingest actually reads — the
    /// fingerprint that keys the persistent fact-store cache.
    ///
    /// Derived by *subtracting* from [`Self::canonical_json`] rather than being
    /// built independently, so the two cannot drift in shape: a new `Options`
    /// field still trips the exhaustive-destructure guard over there, and
    /// `every_canonical_key_is_classified` then forces it to be sorted into
    /// ingest-affecting or analysis-only here.
    ///
    /// Why this exists: `canonical_json` folds in every threshold, so
    /// `--min-revs 5` → `--min-revs 10` re-walked the entire history and burned
    /// one of the five per-repo cache slots. Threshold sweeping is the natural
    /// way to use this tool, and it was the most expensive thing a user could
    /// do. None of those knobs can change a row the ingest writes.
    #[must_use]
    pub fn ingest_cache_json(&self) -> serde_json::Value {
        let mut canon = self.canonical_json();
        if let serde_json::Value::Object(map) = &mut canon {
            for key in Self::ANALYSIS_ONLY_CACHE_KEYS {
                map.remove(*key);
            }
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
        if self.head_only_ingest && self.group_file.is_some() {
            // Grouping rewrites the history tables (`changes` swap), which a
            // head-only ingest leaves empty — the group map would silently
            // apply to nothing. Reject loudly instead.
            return Err(crate::CodeLoreError::InvalidOptions(
                "--group-file is unsupported with the head-only ingest mode \
                 (grouping rewrites history tables, which head-only ingest leaves empty)"
                    .to_string(),
            ));
        }
        if let Some(dir) = &self.temp_dir {
            if !dir.is_dir() {
                return Err(crate::CodeLoreError::InvalidOptions(format!(
                    "--temp-dir {} must be an existing directory",
                    dir.display()
                )));
            }
            // Writability probe: create+remove a PID-suffixed throwaway file
            // rather than trust `is_dir()` alone (a read-only mount or
            // permission-denied dir still passes that check). PID-suffixed
            // so two concurrent `codelore` invocations validating the same
            // `--temp-dir` never race on the same probe filename.
            let probe = dir.join(format!(".codelore-temp-dir-probe-{}", std::process::id()));
            if let Err(e) = std::fs::write(&probe, []) {
                return Err(crate::CodeLoreError::InvalidOptions(format!(
                    "--temp-dir {} is not writable: {e}",
                    dir.display()
                )));
            }
            let _ = std::fs::remove_file(&probe);
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
            fdr_correction: false,
            window_days: crate::constants::DEFAULT_WINDOW_DAYS,
            knowledge_model: "commits".to_string(),
            rework_window_days: crate::constants::DEFAULT_REWORK_WINDOW_DAYS,
            release_tag_glob: crate::constants::DEFAULT_RELEASE_TAG_GLOB.to_string(),
            target: None,
            calibration: None,
            defect_calibration: None,
            allow_foreign_calibration: false,
            head_only_ingest: false,
            temp_dir: None,
        }
    }
}

#[cfg(all(test, feature = "test-support"))]
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
    fn canonical_json_invalidates_when_gitignore_changes() {
        let dir = tempfile::tempdir().unwrap();
        let gitignore = dir.path().join(".gitignore");
        std::fs::write(&gitignore, "target/\n").unwrap();
        let opts = Options {
            repo_path: dir.path().to_path_buf(),
            ..Options::default()
        };
        let v1 = opts.canonical_json();
        std::fs::write(&gitignore, "target/\nvendor/\n").unwrap();
        let v2 = opts.canonical_json();
        assert_ne!(
            v1, v2,
            ".gitignore decides which files reach `changes`, so an edit must \
             invalidate the cache key"
        );
    }

    #[test]
    fn canonical_json_invalidates_when_info_exclude_changes() {
        // The sharpest of the ignore-file digests. `.git/info/exclude` is
        // UNTRACKED: editing it moves neither `head_sha` nor the worktree's
        // dirty flag, so before it was hashed here nothing about the run
        // changed except which files were ingested — a cache hit silently
        // served the pre-edit file set.
        let dir = tempfile::tempdir().unwrap();
        let info = dir.path().join(".git").join("info");
        std::fs::create_dir_all(&info).unwrap();
        let exclude = info.join("exclude");
        std::fs::write(&exclude, "scratch/\n").unwrap();
        let opts = Options {
            repo_path: dir.path().to_path_buf(),
            ..Options::default()
        };
        let v1 = opts.canonical_json();
        std::fs::write(&exclude, "scratch/\ngenerated/\n").unwrap();
        let v2 = opts.canonical_json();
        assert_ne!(
            v1, v2,
            "`.git/info/exclude` edits are invisible to head_sha and to the \
             dirty-tree check, so the cache key is the only thing that can \
             notice them"
        );
    }

    #[test]
    fn every_analysis_only_key_exists_in_the_canonical_form() {
        // Guards the one way the subtraction can rot silently: a key listed in
        // ANALYSIS_ONLY_CACHE_KEYS that no longer exists (renamed field, typo)
        // removes nothing, so the option quietly starts keying the ingest cache
        // again with no failing test anywhere.
        let opts = Options {
            repo_path: std::path::PathBuf::from("pin-repo"),
            ..Options::default()
        };
        let canon = opts.canonical_json();
        let obj = canon.as_object().expect("canonical form is an object");
        for key in Options::ANALYSIS_ONLY_CACHE_KEYS {
            assert!(
                obj.contains_key(*key),
                "{key} is listed as analysis-only but is not in the canonical form — \
                 a rename or typo would make this list silently stop subtracting it"
            );
        }
    }

    #[test]
    fn the_ingest_key_is_a_strict_subset_of_the_canonical_form() {
        let opts = Options {
            repo_path: std::path::PathBuf::from("pin-repo"),
            ..Options::default()
        };
        let full = opts.canonical_json();
        let ingest = opts.ingest_cache_json();
        let full_keys = full.as_object().expect("object").len();
        let ingest_keys = ingest.as_object().expect("object").len();
        assert_eq!(
            ingest_keys,
            full_keys - Options::ANALYSIS_ONLY_CACHE_KEYS.len(),
            "the ingest form must be exactly the canonical form minus the analysis-only list"
        );
        assert!(ingest_keys > 0, "the ingest form must not be empty");
        for key in Options::ANALYSIS_ONLY_CACHE_KEYS {
            assert!(
                !ingest.as_object().unwrap().contains_key(*key),
                "{key} must not key the ingest cache"
            );
        }
    }

    #[test]
    fn analysis_only_knobs_do_not_split_the_ingest_cache() {
        // The point of the split: sweeping a threshold must reuse the cache.
        let base = Options {
            repo_path: std::path::PathBuf::from("pin-repo"),
            ..Options::default()
        };
        let key = base.ingest_cache_json().to_string();

        let swept = Options {
            min_revs: base.min_revs + 5,
            min_coupling_pct: 42,
            fisher_significance: 0.01,
            window_days: 30,
            clone_similarity_floor: 0.95,
            ..base.clone()
        };
        assert_eq!(
            key,
            swept.ingest_cache_json().to_string(),
            "changing analysis-only thresholds must not force a re-ingest"
        );
        assert_ne!(
            base.canonical_json().to_string(),
            swept.canonical_json().to_string(),
            "the FULL form must still notice them — `change_set::report_key` depends on it"
        );
    }

    #[test]
    fn ingest_affecting_knobs_still_split_the_ingest_cache() {
        // Anti-vacuity for the test above: if the subtraction were too greedy,
        // the previous test would pass for the wrong reason.
        let base = Options {
            repo_path: std::path::PathBuf::from("pin-repo"),
            ..Options::default()
        };
        let key = base.ingest_cache_json().to_string();

        for (label, changed) in [
            (
                "include_merges",
                Options {
                    include_merges: !base.include_merges,
                    ..base.clone()
                },
            ),
            (
                "include_ignored",
                Options {
                    include_ignored: !base.include_ignored,
                    ..base.clone()
                },
            ),
            (
                "exclude_patterns",
                Options {
                    exclude_patterns: vec!["vendor/**".to_string()],
                    ..base.clone()
                },
            ),
            (
                "min_clone_node_count",
                Options {
                    min_clone_node_count: base.min_clone_node_count + 1,
                    ..base.clone()
                },
            ),
            (
                "head_only_ingest",
                Options {
                    head_only_ingest: !base.head_only_ingest,
                    ..base.clone()
                },
            ),
        ] {
            assert_ne!(
                key,
                changed.ingest_cache_json().to_string(),
                "{label} reaches the ingest and must still split the cache"
            );
        }
    }

    /// Pins the cache-key field classification. The exhaustive-destructure
    /// guard in `canonical_json` makes adding an `Options` field a COMPILE
    /// error until it is classified; this test pins the RESULT so an
    /// accidental reclassification (an excluded field reverting to included, a
    /// new field silently entering the key) also trips a red test. The digest
    /// below is a known-good fingerprint of the whole canonical key — the
    /// "before/after" byte-proof. Regenerate it ONLY on a deliberate,
    /// reviewed (re)classification, never to make a red test green.
    #[test]
    fn canonical_json_cache_key_classification_is_pinned() {
        use sha2::{Digest, Sha256};

        // A fixed, nonexistent repo path so every content-digest lookup
        // resolves to `null` and the canonical form is deterministic and
        // platform-independent (no path separators, no files on disk).
        let opts = Options {
            repo_path: std::path::PathBuf::from("pin-repo"),
            ..Options::default()
        };
        let canon = opts.canonical_json();

        // Deterministic across calls.
        assert_eq!(canon.to_string(), opts.canonical_json().to_string());

        let obj = canon.as_object().expect("canonical form is a JSON object");
        // Path selectors are stripped — only their content digests key the cache.
        for stripped in [
            "team_map_file",
            "group_file",
            "calibration",
            "defect_calibration",
        ] {
            assert!(
                !obj.contains_key(stripped),
                "{stripped} path must not key the cache"
            );
        }
        for digest in [
            "team_map_digest",
            "group_file_digest",
            "calibration_digest",
            "defect_calibration_digest",
            "codelorebots_digest",
            "mailmap_digest",
            "codeloreignore_digest",
            "gitignore_digest",
            "info_exclude_digest",
        ] {
            assert!(obj.contains_key(digest), "{digest} must key the cache");
        }
        // Cosmetic / per-invocation selectors are neutralised, not dropped.
        assert!(obj["rows_limit"].is_null(), "rows_limit must be dropped");
        assert!(obj["target"].is_null(), "target must be dropped");
        assert!(obj["temp_dir"].is_null(), "temp_dir must be dropped");
        assert_eq!(
            obj["explain"].as_bool(),
            Some(false),
            "explain must be neutralised"
        );
        assert_eq!(
            obj["repo_path"].as_str(),
            Some(""),
            "repo_path must be neutralised — identity comes from the canonical \
             path `cache_key` hashes, not from how the user spelled it"
        );

        let digest = hex::encode(Sha256::digest(canon.to_string().as_bytes()));
        assert_eq!(
            digest, "3574fe8f874867508bc70802292b5a2f0aca9f922ec8b2150c1866db442537d0",
            "cache-key classification changed:\n{canon}"
        );
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
    fn validate_rejects_head_only_ingest_with_group_file() {
        let opts = Options {
            head_only_ingest: true,
            group_file: Some(std::path::PathBuf::from("groups.toml")),
            ..Options::default()
        };
        let err = opts
            .validate()
            .expect_err("head-only + group-file must fail");
        assert!(
            matches!(err, crate::CodeLoreError::InvalidOptions(_)),
            "must be InvalidOptions, got: {err:?}"
        );
        assert!(
            format!("{err}").contains("group-file"),
            "error must name the offending field: {err}"
        );
        // head-only alone stays valid.
        let head_only = Options {
            head_only_ingest: true,
            ..Options::default()
        };
        assert!(head_only.validate().is_ok());
    }

    #[test]
    fn canonical_json_strips_defect_calibration_path_keeps_only_digest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("defects.calib.json");
        std::fs::write(&path, b"{}").unwrap();
        let opts = Options {
            defect_calibration: Some(path),
            ..Options::default()
        };
        let s = opts.canonical_json().to_string();
        assert!(
            !s.contains("defects.calib.json"),
            "defect-calibration path leaked into canonical form: {s}"
        );
        assert!(
            s.contains("defect_calibration_digest"),
            "defect_calibration_digest field missing: {s}"
        );
    }

    #[test]
    fn canonical_json_invalidates_when_defect_calibration_content_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("defects.calib.json");
        std::fs::write(&path, br#"{"format_version":1}"#).unwrap();
        let base = Options {
            defect_calibration: Some(path.clone()),
            ..Options::default()
        };
        let v1 = base.canonical_json();
        // Edit the artifact in place — path unchanged, content changes.
        std::fs::write(&path, br#"{"format_version":1,"x":2}"#).unwrap();
        let v2 = base.canonical_json();
        assert_ne!(
            v1, v2,
            "defect-calibration artifact edit must invalidate the provenance stamp"
        );
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
    fn validate_accepts_existing_writable_temp_dir() {
        let dir = tempfile::tempdir().unwrap();
        let opts = Options {
            temp_dir: Some(dir.path().to_path_buf()),
            ..Options::default()
        };
        assert!(opts.validate().is_ok());
    }

    #[test]
    fn validate_rejects_missing_temp_dir() {
        let opts = Options {
            temp_dir: Some(std::path::PathBuf::from(
                "/nonexistent/codelore-temp-dir-test",
            )),
            ..Options::default()
        };
        let err = opts.validate().expect_err("missing --temp-dir must fail");
        assert!(
            format!("{err}").contains("temp-dir"),
            "error must name the offending flag: {err}"
        );
    }

    #[test]
    fn validate_rejects_temp_dir_that_is_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("not-a-dir");
        std::fs::write(&file_path, b"x").unwrap();
        let opts = Options {
            temp_dir: Some(file_path),
            ..Options::default()
        };
        assert!(opts.validate().is_err());
    }

    #[test]
    fn canonical_json_ignores_temp_dir_entirely() {
        // temp_dir is a pure environment/spill selector: two runs differing
        // only in --temp-dir must produce the identical canonical form, not
        // merely a shared digest — there is no meaningful "content" to hash
        // for a directory.
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let opts_a = Options {
            temp_dir: Some(dir_a.path().to_path_buf()),
            ..Options::default()
        };
        let opts_b = Options {
            temp_dir: Some(dir_b.path().to_path_buf()),
            ..Options::default()
        };
        let opts_none = Options::default();
        assert_eq!(opts_a.canonical_json(), opts_b.canonical_json());
        assert_eq!(opts_a.canonical_json(), opts_none.canonical_json());
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

    #[test]
    fn canonical_json_distinguishes_fdr_correction() {
        // `fdr_correction` changes which coupling rows survive, so it must
        // key the cache apart — a new bool is auto-included in the canonical
        // form (it is not on the drop-list).
        let off = Options {
            fdr_correction: false,
            ..Options::default()
        };
        let on = Options {
            fdr_correction: true,
            ..Options::default()
        };
        assert_ne!(off.canonical_json(), on.canonical_json());
    }
}

//! Provenance manifest — documents every choice that affected an analysis run.
//! Per spec §3.2 provenance table + §5 differentiator.

use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};
use serde::Serialize;

/// Pinned gix version embedded in every provenance manifest. Surfaced both
/// in JSON sidecars and the pre-flight banner header. Kept in sync with the
/// `gix` dep in workspace `Cargo.toml` via `tests/dep_versions_drift_test.rs`,
/// which fails CI if these constants and `Cargo.lock` drift apart.
pub const GIX_VERSION: &str = "0.85.0";
/// Pinned `DuckDB` version — same drift guard applies.
pub const DUCKDB_VERSION: &str = "1.10504.0";

/// Explicit schema version for the provenance manifest format. Bumped
/// whenever a field is renamed, removed, or its semantic meaning
/// changes. Additive field changes (new optional fields with
/// non-default `Some()` values) do NOT require a bump — downstream
/// consumers using serde's `#[serde(deny_unknown_fields)]` opt-in are
/// the only ones affected by adds, and they'd need to bump regardless.
///
/// Serialized first (alphabetical order via `schema_version`'s
/// position in the struct) so downstream parsers can refuse to
/// proceed on a mismatch before reading any data fields. Without
/// this, downstream tooling can't distinguish "the field I expected
/// is gone because the schema rolled forward" from "the field was
/// always optional and the producer omitted it."
pub const MANIFEST_SCHEMA_VERSION: u32 = 2;

/// Tree-sitter grammar pins as compiled into this build. Source of
/// truth is `codelore-lib/Cargo.toml`'s `tree-sitter-*` entries plus
/// `tree-sitter` itself. Captured in the provenance manifest so a
/// reproducibility auditor can confirm a given complexity score was
/// computed against the same grammar ABI — silent grammar bumps
/// renumber tree-sitter node IDs and shift cognitive-complexity
/// outputs without any obvious signal upstream.
fn grammar_pins() -> std::collections::BTreeMap<String, String> {
    let mut m = std::collections::BTreeMap::new();
    m.insert("tree-sitter".into(), "0.25.3".into());
    m.insert("tree-sitter-rust".into(), "0.23.2".into());
    m.insert("tree-sitter-python".into(), "0.23.6".into());
    m.insert("tree-sitter-java".into(), "0.23.5".into());
    m.insert("tree-sitter-javascript".into(), "0.23.1".into());
    m.insert("tree-sitter-typescript".into(), "0.23.1".into());
    m
}

/// Workspace MSRV at build time. Mirrors the `[workspace.package]
/// rust-version` value in the root `Cargo.toml`. A toolchain bump
/// invalidates HEAD-time complexity outputs (tree-sitter ABI is
/// rustc-version-stamped via `.rmeta`) so capturing this in the
/// manifest is reproducibility-load-bearing.
const RUST_VERSION: &str = "1.96";

/// Build target triple. `cfg!()` resolves at compile time so this
/// remains a `&'static str`. Captured for provenance so a binary's
/// output can be cross-checked against a known-good triple
/// (e.g. distinguishing a Homebrew-installed macOS-aarch64 build
/// from a manually-compiled Linux-x86_64 one running through
/// Rosetta or qemu).
const TARGET_TRIPLE: &str = {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "aarch64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "x86_64-pc-windows-msvc"
    } else {
        "unknown"
    }
};

#[derive(Debug, Serialize)]
pub struct Manifest {
    /// Provenance-manifest schema version. See [`MANIFEST_SCHEMA_VERSION`]
    /// for the bump contract.
    pub schema_version: u32,
    pub codelore_version: String,
    pub gix_version: String,
    pub arrow_version: String,
    pub duckdb_version: String,
    pub run_started_at: String,
    pub repo_path: String,
    /// Resolved HEAD SHA of the analysed repository at run time. Empty
    /// string when the fact store could not surface a HEAD commit
    /// (degenerate case; should not happen on a successful ingest).
    /// Reproducibility-critical: without this an auditor can't
    /// confirm two runs analysed the same code state.
    pub head_sha: String,
    /// Hex-encoded SHA-256 of the persistent cache key for this run.
    /// Lets downstream tooling join provenance to cache entries on
    /// disk without recomputing the key.
    pub cache_key_hash: String,
    pub after_date: Option<String>,
    pub before_date: Option<String>,
    pub analysis: String,
    pub min_revs: u32,
    pub min_shared_revs: u32,
    pub min_coupling_pct: u8,
    pub max_changeset_size: u32,
    pub fisher_significance: f64,
    pub include_merges: bool,
    pub age_time_now: Option<String>,
    pub merge_handling: String,
    pub complexity_sample: String,
    /// Workspace MSRV at build time. Pinned by `RUST_VERSION` in this
    /// module; mirrors `[workspace.package].rust-version` in the
    /// root `Cargo.toml`.
    pub rust_version: String,
    /// Build target triple (`<arch>-<vendor>-<os>-<env>`).
    /// Reproducibility check for cross-built binaries.
    pub target_triple: String,
    /// Tree-sitter grammar pins as compiled into this build. Map
    /// from grammar crate name to its `Cargo.toml` version pin. A
    /// silent grammar bump can shift cognitive-complexity outputs
    /// without surfacing in any other manifest field.
    pub grammars: std::collections::BTreeMap<String, String>,
    /// Complete canonical JSON of every Options field at run time. Source
    /// of truth for reproducibility — auto-derives so newly-added Options
    /// fields propagate without per-field maintenance. The flat fields above
    /// remain for human readability and grep-ability.
    pub options: serde_json::Value,
    /// Vintage string of the corpus-calibration artifact active for this run
    /// (e.g. `"world-2026-07"`). Absent (`None`, omitted from JSON) when no
    /// calibration artifact was active — no `--calibration` file was passed
    /// and the embedded world artifact is still the placeholder.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corpus_vintage: Option<String>,
}

impl Manifest {
    /// Capture the manifest from a fact store + options + analysis name.
    /// `_db` is currently unused but reserved for reading provenance table values.
    pub fn capture(db: &FactsDb, opts: &Options, analysis: &str) -> Result<Self> {
        let now = time::OffsetDateTime::now_utc();
        let run_started_at = format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            now.year(),
            u8::from(now.month()),
            now.day(),
            now.hour(),
            now.minute(),
            now.second()
        );
        let complexity_sample = match opts.complexity_sample {
            crate::options::ComplexitySample::Head => "head",
            crate::options::ComplexitySample::Adaptive => "adaptive",
            crate::options::ComplexitySample::Full => "full",
        };
        // Resolve HEAD SHA from the fact store. The commits table is
        // populated by ingest; the most-recently-inserted commit (by
        // rowid) is HEAD per the rowid-ASC invariant (rowid ASC = walk
        // order, gix walks newest-first). Falls back to empty string
        // if the query fails (no commits ingested — degenerate state).
        let head_sha: String = db
            .conn()
            .query_row(
                "SELECT rev FROM commits ORDER BY rowid ASC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap_or_default();
        // Cache key hash mirrors what `open_or_ingest_with_cache_root`
        // computes; recomputing here gives downstream tooling a stable
        // join key into the on-disk cache without exposing the cache
        // module's internals beyond what's already `pub`.
        let cache_key_bytes = crate::cache::cache_key(&opts.repo_path, &head_sha, opts);
        let cache_key_hash = hex::encode(cache_key_bytes);

        let corpus_vintage = crate::calibration::active_vintage(opts)?;
        Ok(Self {
            schema_version: MANIFEST_SCHEMA_VERSION,
            codelore_version: env!("CARGO_PKG_VERSION").to_string(),
            gix_version: GIX_VERSION.to_string(),
            arrow_version: crate::arrow_facade::ARROW_RUNTIME_VERSION.to_string(),
            duckdb_version: DUCKDB_VERSION.to_string(),
            run_started_at,
            repo_path: opts.repo_path.display().to_string(),
            head_sha,
            cache_key_hash,
            after_date: opts.after.map(|d| d.to_string()),
            before_date: opts.before.map(|d| d.to_string()),
            analysis: analysis.to_string(),
            min_revs: opts.min_revs,
            min_shared_revs: opts.min_shared_revs,
            min_coupling_pct: opts.min_coupling_pct,
            max_changeset_size: opts.max_changeset_size,
            fisher_significance: opts.fisher_significance,
            include_merges: opts.include_merges,
            age_time_now: opts.age_time_now.map(|d| d.to_string()),
            merge_handling: if opts.include_merges {
                "include"
            } else {
                "exclude"
            }
            .to_string(),
            complexity_sample: complexity_sample.to_string(),
            rust_version: RUST_VERSION.to_string(),
            target_triple: TARGET_TRIPLE.to_string(),
            grammars: grammar_pins(),
            options: opts.canonical_json(),
            corpus_vintage,
        })
    }

    /// Serialize the manifest to pretty JSON.
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| CodeLoreError::Output(format!("manifest json: {e}")))
    }
}

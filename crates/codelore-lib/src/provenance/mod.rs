//! Provenance manifest — documents every choice that affected an analysis run.
//! Per spec §3.2 provenance table + §5 differentiator.

use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};
use serde::Serialize;

/// Pinned gix version embedded in every provenance manifest. Surfaced both
/// in JSON sidecars and the pre-flight banner header. Kept in sync with the
/// `gix` dep in workspace `Cargo.toml` via `tests/dep_versions_drift_test.rs`,
/// which fails CI if these constants and `Cargo.lock` drift apart.
pub const GIX_VERSION: &str = "0.84.0";
/// Pinned `DuckDB` version — same drift guard applies.
pub const DUCKDB_VERSION: &str = "1.10503.1";

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
pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

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
    /// Complete canonical JSON of every Options field at run time. Source
    /// of truth for reproducibility — auto-derives so newly-added Options
    /// fields propagate without per-field maintenance. The flat fields above
    /// remain for human readability and grep-ability.
    pub options: serde_json::Value,
}

impl Manifest {
    /// Capture the manifest from a fact store + options + analysis name.
    /// `_db` is currently unused but reserved for reading provenance table values
    /// in Plan 5.x when we wire it.
    pub fn capture(_db: &FactsDb, opts: &Options, analysis: &str) -> Result<Self> {
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
        Ok(Self {
            schema_version: MANIFEST_SCHEMA_VERSION,
            codelore_version: env!("CARGO_PKG_VERSION").to_string(),
            gix_version: GIX_VERSION.to_string(),
            arrow_version: crate::arrow_facade::ARROW_RUNTIME_VERSION.to_string(),
            duckdb_version: DUCKDB_VERSION.to_string(),
            run_started_at,
            repo_path: opts.repo_path.display().to_string(),
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
            options: opts.canonical_json(),
        })
    }

    /// Serialize the manifest to pretty JSON.
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| CodeLoreError::Output(format!("manifest json: {e}")))
    }
}

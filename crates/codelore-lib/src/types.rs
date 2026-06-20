//! Public types for the codelore-lib API. These are the contract every
//! pipeline stage agrees on. See spec §3.1.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// Bumped on any breaking change to facts or output schemas.
///
/// Schema 3 adds `commits.committer_date TIMESTAMP NOT NULL` alongside
/// the author-date `commits.date`. The delta `(committer_date - date)`
/// is the in-flight time the `lead-time` and `delivery-friction`
/// analyses surface; without the separate column, `lead-time` silently
/// emitted zero rows for every commit.
///
/// Schema 2 promoted `commits.date` from `DATE` to `TIMESTAMP` so HEAD
/// resolution and same-day chronology are precise — no more lexicographical
/// rev tiebreaks deciding which commit "is" HEAD when multiple commits
/// share a calendar day. `CommitEvent.date` carries a full `OffsetDateTime`
/// (stored as UTC; tz offset is currently discarded at the schema boundary).
pub const SCHEMA_VERSION: u8 = 3;

/// One commit, as observed by the parser stage. Immutable event.
// Eq removed: CommitEvent contains Option<KameiFeatures> which has f64 fields (not Eq).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CommitEvent {
    pub rev: String,
    pub author_email: String,
    pub author_name: String,
    pub committer_email: String,
    /// Author time as a full `OffsetDateTime`. Stored in `DuckDB` as `TIMESTAMP`
    /// (UTC-normalised). The original tz offset is not currently persisted —
    /// see the `commits` table comment for the tz-preservation roadmap.
    pub date: OffsetDateTime,
    /// Committer time as a full `OffsetDateTime`. Stored in `DuckDB` as
    /// `TIMESTAMP` (UTC-normalised). On linear-history workflows (rebase,
    /// squash) this often equals `date`; on merge-via-merge-commit and
    /// review workflows the delta `(committer_date - date)` is the
    /// in-flight time the `lead-time` and `delivery-friction` analyses
    /// surface. Both backends (`GixRepo`, `GitCliRepo`) populate this
    /// from the raw commit object — the differential test gate
    /// asserts they agree.
    pub committer_date: OffsetDateTime,
    pub message: String,
    pub parents: Vec<String>,
    pub changes: Vec<FileChange>,
    /// Canonical author email after .mailmap resolution. None means not yet resolved.
    pub canonical_author: Option<String>,
    /// AI authorship classification. None means not yet classified.
    /// Values: "human" | "ai-assisted" | "ai-authored"
    pub ai_attribution: Option<String>,
    /// Populated by the `enrich_kamei` pipeline stage (Plan 4), NOT at gix walk-time.
    pub kamei: Option<KameiFeatures>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    pub change_type: ChangeType,
    pub loc_added: u32,
    pub loc_deleted: u32,
    pub hunks: Vec<Hunk>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ChangeType {
    Added,
    Modified,
    Deleted,
    Renamed { from: String, similarity: u8 },
    Copied { from: String, similarity: u8 },
    BinaryOrUnknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
}

/// Kamei 14-feature JIT-SDP canonical vector. Computed by the `enrich_kamei` stage in Plan 4.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct KameiFeatures {
    pub ns: u32,
    pub nd: u32,
    pub nf: u32,
    pub entropy: f64,
    pub la: u32,
    pub ld: u32,
    pub lt: f64,
    pub fix: bool,
    pub ndev: u32,
    pub age: f64,
    pub nuc: u32,
    pub exp: u32,
    pub rexp: f64,
    pub sexp: u32,
}

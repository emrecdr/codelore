//! Public types for the bca-lib API. These are the contract every
//! pipeline stage agrees on. See spec §3.1.

use serde::{Deserialize, Serialize};
use time::Date;

/// Bumped on any breaking change to facts or output schemas.
pub const SCHEMA_VERSION: u8 = 1;

/// One commit, as observed by the parser stage. Immutable event.
// Eq removed: CommitEvent contains Option<KameiFeatures> which has f64 fields (not Eq).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CommitEvent {
    pub rev: String,
    pub author_email: String,
    pub author_name: String,
    pub committer_email: String,
    pub date: Date,
    pub message: String,
    pub parents: Vec<String>,
    pub changes: Vec<FileChange>,
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

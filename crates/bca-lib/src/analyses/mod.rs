//! Analysis implementations. Each is a SQL view over the fact store
//! plus a thin Rust orchestrator. Plan 1 ships `revisions` only.

pub mod revisions;

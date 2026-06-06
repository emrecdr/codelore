//! Analysis implementations. Each is a SQL view over the fact store
//! plus a thin Rust orchestrator.

pub mod code_age;
pub mod code_health;
pub mod hotspots;
pub mod revisions;

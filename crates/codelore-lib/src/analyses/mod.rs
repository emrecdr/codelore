//! Analysis implementations. Each is a SQL view over the fact store
//! plus a thin Rust orchestrator.

pub mod authors;
pub mod churn;
pub mod clones;
pub mod code_age;
pub mod code_health;
pub mod communication;
pub mod coupling;
pub mod hotspots;
pub mod ownership;
pub mod revisions;
pub mod summary;

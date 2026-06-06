//! Analysis implementations. Each is a SQL view over the fact store
//! plus a thin Rust orchestrator.

pub mod churn;
pub mod code_age;
pub mod code_health;
pub mod communication;
pub mod hotspots;
pub mod revisions;

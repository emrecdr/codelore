//! Analysis implementations. Each is a SQL view over the fact store
//! plus a thin Rust orchestrator.

pub mod authors;
pub mod churn;
pub mod clone_coupling;
pub mod clones;
pub mod code_age;
pub mod code_health;
pub mod communication;
pub mod coupling;
pub mod entity_effort;
pub mod entity_ownership;
pub mod hotspots;
pub mod main_dev;
pub mod messages;
pub mod ownership;
pub mod revisions;
pub mod soc;
pub mod summary;

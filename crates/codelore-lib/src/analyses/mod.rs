//! Analysis implementations. Each is a SQL view over the fact store
//! plus a thin Rust orchestrator.

pub mod arch_violations;
pub mod authors;
pub mod bus_factor;
pub mod centrality;
pub mod churn;
pub mod clone_coupling;
pub mod clones;
pub mod code_age;
pub mod code_health;
pub mod communication;
pub mod communities;
pub mod coupling;
pub mod dashboard;
pub mod delivery_friction;
pub mod entity_effort;
pub mod entity_ownership;
pub mod god_classes;
pub mod grouped_complexity;
pub mod hotspots;
pub mod knowledge_islands;
pub mod lead_time;
pub mod lineage;
pub mod main_dev;
pub mod messages;
pub mod mi;
pub mod modularity_violations;
pub mod ownership;
pub mod pair_programming;
pub mod query;
pub mod revisions;
pub mod soc;
pub mod stale_code;
pub mod summary;
pub mod top_committers;
pub mod unstable_interface;

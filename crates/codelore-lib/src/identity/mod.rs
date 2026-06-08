//! Identity resolution: mailmap (in repo/), bots.toml default-deny list,
//! team-map projection, AI attribution stub.

pub mod bots;
pub mod team_map;

pub use bots::{ai_attribution, is_bot};
pub use team_map::{TeamMap, apply as apply_team_map, discover as discover_team_map};

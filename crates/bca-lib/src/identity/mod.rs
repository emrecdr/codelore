//! Identity resolution: mailmap (in repo/), bots.toml default-deny list,
//! AI attribution stub. See spec §1.1.

pub mod bots;

pub use bots::{ai_attribution, is_bot};

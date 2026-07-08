//! Decayed-knowledge and Degree-of-Expertise (DOE) materialisation layer.
//!
//! This module provides two temporary `DuckDB` tables consumed by every WS-B
//! analysis (bus-factor, knowledge-islands, etc.):
//!
//! - [`shares::knowledge_shares`] — per-author exponentially-decayed
//!   knowledge share, with reviewer credit, normalised within each path.
//! - [`shares::doe_scores`] — Degree of Expertise (Cury & Avelino, SBES'24)
//!   and expert flag per author×path.
//!
//! Call [`shares::materialize_knowledge_shares`] before running any WS-B
//! analysis. The call is idempotent — repeated invocations are no-ops.

pub mod shares;
pub mod trailers;

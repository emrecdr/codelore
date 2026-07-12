//! The single API surface `codelore-cli` imports through. Internal modules
//! stay `pub` because the integration-test crate needs deep white-box access,
//! but the CLI binary reaches the library only via these re-exports — so the
//! CLI↔library contract is enumerated in exactly one place.
pub use crate::{AnalysisName, CodeLoreError, Options, Result};
pub use crate::{
    analyses, analysis, cache, calibration, constants, external, facts, options, output,
    provenance, quality_gates, repo,
};

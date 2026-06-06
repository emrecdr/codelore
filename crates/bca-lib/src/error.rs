//! Public error type. Drives CLI exit codes at the lib/cli boundary.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, BcaError>;

#[derive(Debug, Error)]
pub enum BcaError {
    #[error("provenance violation: {0}")]
    Provenance(String),

    #[error("repository error: {0}")]
    Repo(String),

    #[error("analysis error: {0}")]
    Analysis(String),

    #[error("output error: {0}")]
    Output(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl BcaError {
    /// CLI exit code for this error variant. See spec §6.6.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Provenance(_) => 2,
            Self::Repo(_) => 3,
            Self::Analysis(_) => 4,
            Self::Output(_) | Self::Io(_) => 5,
        }
    }
}

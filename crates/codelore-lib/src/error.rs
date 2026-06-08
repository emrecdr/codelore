//! Public error type. Drives CLI exit codes at the lib/cli boundary.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, CodeLoreError>;

#[derive(Debug, Error)]
pub enum CodeLoreError {
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

    // ------------------------------------------------------------------
    // Narrow structured variants. The audit pass found that only ONE
    // call site (`codelore-cli::main::exit_code`) pattern-matches on
    // CodeLoreError today, so wholesale "every failure mode gets a
    // typed variant" would be ceremony without payoff. These three are
    // the failure modes where a CONSUMER could meaningfully recover or
    // present a better message: malformed user config (point them at
    // the line), unknown analysis name (list what's accepted), missing
    // blob (signal repo corruption distinctly from generic repo errors).
    // ------------------------------------------------------------------
    /// User-supplied team-map CSV is malformed. `line` is 1-indexed and
    /// `0` means a structural problem before any data row (e.g. missing
    /// header). CLI uses this to print a "fix `<path>` line N" hint.
    #[error("malformed team-map {} line {line}: {reason}", path.display())]
    MalformedTeamMap {
        path: std::path::PathBuf,
        line: usize,
        reason: String,
    },

    /// `--analysis foo` was passed but `foo` isn't an analysis name we
    /// know. `supported` lists what we DO accept so the CLI can echo it.
    #[error("unknown analysis {name:?}. Supported: {}", supported.join(", "))]
    UnknownAnalysisName {
        name: String,
        supported: Vec<&'static str>,
    },

    /// A blob OID resolved from a tree-walk wasn't found in the object
    /// database. Almost always means the repo is shallow-cloned (parent
    /// ancestry truncated) or corrupted; distinct from generic repo
    /// errors so consumers can refuse to retry on bad data.
    #[error("blob {oid} not found in object database — repo may be shallow or corrupted")]
    BlobNotFound { oid: String },
}

impl CodeLoreError {
    /// CLI exit code for this error variant. See spec §6.6.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Provenance(_) | Self::MalformedTeamMap { .. } => 2,
            Self::Repo(_) | Self::BlobNotFound { .. } => 3,
            Self::Analysis(_) | Self::UnknownAnalysisName { .. } => 4,
            Self::Output(_) | Self::Io(_) => 5,
        }
    }
}

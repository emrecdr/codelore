//! The closed set of analyses codelore supports. Enum, not string,
//! so the compiler catches typos that code-maat's string dispatch silently misroutes.

use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnalysisName {
    // v1 Spine — 10 core
    Hotspots,
    Coupling,
    Ownership,
    CodeAge,
    AbsChurn,
    AuthorChurn,
    EntityChurn,
    Communication,
    CodeHealth,
    Summary,
    // code-maat parity (computed as side-data on hotspots, addressable standalone)
    Revisions,
    Authors,
    // Plan 7: clone detection (T1+T2 via AST structural hashing)
    Clones,
    // Plan 8 §6: live-clone × Fisher-significant co-change intersection
    CloneCoupling,
}

impl AnalysisName {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hotspots => "hotspots",
            Self::Coupling => "coupling",
            Self::Ownership => "ownership",
            Self::CodeAge => "code-age",
            Self::AbsChurn => "abs-churn",
            Self::AuthorChurn => "author-churn",
            Self::EntityChurn => "entity-churn",
            Self::Communication => "communication",
            Self::CodeHealth => "code-health",
            Self::Summary => "summary",
            Self::Revisions => "revisions",
            Self::Authors => "authors",
            Self::Clones => "clones",
            Self::CloneCoupling => "clone-coupling",
        }
    }

    #[must_use]
    pub fn all() -> &'static [Self] {
        &[
            Self::Hotspots,
            Self::Coupling,
            Self::Ownership,
            Self::CodeAge,
            Self::AbsChurn,
            Self::AuthorChurn,
            Self::EntityChurn,
            Self::Communication,
            Self::CodeHealth,
            Self::Summary,
            Self::Revisions,
            Self::Authors,
            Self::Clones,
            Self::CloneCoupling,
        ]
    }
}

impl fmt::Display for AnalysisName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AnalysisName {
    type Err = UnknownAnalysisError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::all()
            .iter()
            .find(|a| a.as_str() == s)
            .copied()
            .ok_or_else(|| UnknownAnalysisError(s.to_string()))
    }
}

#[derive(Debug)]
pub struct UnknownAnalysisError(pub String);

impl std::error::Error for UnknownAnalysisError {}

impl fmt::Display for UnknownAnalysisError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Enumerate every public analysis name so the user sees what they can
        // pick from. Plan 8 §2 Task 6 wired `Authors`, so it's no longer filtered out.
        let names: Vec<&str> = AnalysisName::all().iter().map(|a| a.as_str()).collect();
        write!(
            f,
            "unknown analysis {:?}. Supported: {}",
            self.0,
            names.join(", ")
        )
    }
}

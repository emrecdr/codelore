//! The closed set of analyses bca supports. Enum, not string,
//! so the compiler catches typos that code-maat's string dispatch silently misroutes.

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
        ]
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

#[derive(Debug, thiserror::Error)]
#[error("unknown analysis: {0}")]
pub struct UnknownAnalysisError(pub String);

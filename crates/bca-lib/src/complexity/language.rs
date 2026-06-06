//! Path → bca-rca Language dispatch for Tier-1 languages.
//!
//! Returns None for unsupported file types. Plan 3 only handles
//! files with the listed extensions; Plan 4 may add custom mapping
//! via TOML config.

/// Tier-1 language identifier wrapping bca-rca's per-language parser types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier1Language {
    Rust,
    Python,
    Java,
    JavaScript,
    TypeScript,
}

impl Tier1Language {
    /// Returns the language for a path, if it's a Tier-1 file extension.
    #[must_use]
    pub fn from_path(path: &str) -> Option<Self> {
        let ext = path.rsplit('.').next()?;
        match ext.to_ascii_lowercase().as_str() {
            "rs" => Some(Self::Rust),
            "py" | "pyi" => Some(Self::Python),
            "java" => Some(Self::Java),
            "js" | "jsx" | "mjs" | "cjs" => Some(Self::JavaScript),
            "ts" | "tsx" => Some(Self::TypeScript),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
            Self::Java => "java",
            Self::JavaScript => "javascript",
            Self::TypeScript => "typescript",
        }
    }
}

//! Path → codelore-rca Language dispatch for Tier-1 languages.
//!
//! Returns None for unsupported file types. Only files with the
//! listed extensions are handled.

/// Tier-1 language identifier wrapping codelore-rca's per-language parser types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier1Language {
    Rust,
    Python,
    Java,
    JavaScript,
    TypeScript,
    /// `.tsx` — TypeScript with JSX. Split from [`Self::TypeScript`] because
    /// the plain TypeScript tree-sitter grammar error-recovers on JSX tags,
    /// so complexity/MI/LOC would be computed over a garbage tree; `.tsx`
    /// needs the dedicated TSX grammar. It still reports `"typescript"` from
    /// [`Self::as_str`] so its facts group with TypeScript downstream.
    Tsx,
}

impl Tier1Language {
    /// Returns the language for a path, if it's a Tier-1 file extension.
    /// Matching is ASCII-case-insensitive over `Path::extension` semantics
    /// (a dotfile like `.rs` has no extension), in deliberate parity with
    /// the clones and imports dispatchers — the parity test in
    /// `imports::language` holds the three together.
    #[must_use]
    pub fn from_path(path: &str) -> Option<Self> {
        let ext = std::path::Path::new(path).extension()?.to_str()?;
        match ext.to_ascii_lowercase().as_str() {
            "rs" => Some(Self::Rust),
            "py" | "pyi" => Some(Self::Python),
            "java" => Some(Self::Java),
            "js" | "jsx" | "mjs" | "cjs" => Some(Self::JavaScript),
            "ts" => Some(Self::TypeScript),
            "tsx" => Some(Self::Tsx),
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
            // `.tsx` deliberately shares the TypeScript label: it differs only
            // in grammar, so its facts group with `.ts` for calibration and
            // code-health. Do not give `.tsx` a distinct `"tsx"` label.
            Self::TypeScript | Self::Tsx => "typescript",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tsx_and_ts_dispatch_to_distinct_variants() {
        // `.tsx` needs the TSX grammar; `.ts` uses plain TypeScript. They
        // must not collapse to one variant, or JSX files parse as garbage.
        assert_eq!(
            Tier1Language::from_path("View.tsx"),
            Some(Tier1Language::Tsx)
        );
        assert_eq!(
            Tier1Language::from_path("app.ts"),
            Some(Tier1Language::TypeScript)
        );
    }

    #[test]
    fn tsx_reports_the_typescript_label() {
        // Same `as_str` label so `.tsx` facts group with `.ts` downstream.
        assert_eq!(Tier1Language::Tsx.as_str(), "typescript");
        assert_eq!(Tier1Language::TypeScript.as_str(), "typescript");
    }
}

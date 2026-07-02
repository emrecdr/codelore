//! Layered-architecture rule validation.
//!
//! Parses `.codelore-arch-rules.toml` at the repo root and exposes
//! the rule set to the `architecture-violations` analysis. Each
//! layer declares its path prefixes + which other layers it may
//! depend on; the analysis flags imports that cross a forbidden
//! boundary.
//!
//! Format:
//!
//! ```toml
//! [layer."domain"]    paths = ["src/domain/"]    may_depend_on = []
//! [layer."app"]       paths = ["src/app/"]       may_depend_on = ["domain"]
//! [layer."infra"]     paths = ["src/infra/"]     may_depend_on = ["app", "domain"]
//! ```
//!
//! Matching rules:
//!   - A file's layer is the first layer whose `paths` prefix-matches
//!     the file's path. Order matters when paths nest.
//!   - Imports to external (npm / pypi / std) targets are skipped —
//!     they don't have a layer.
//!   - Imports within the same layer are always allowed.
//!   - Imports to a layer not in the source's `may_depend_on` list
//!     surface as violations.
//!   - Files outside every declared layer are unclassified — their
//!     imports are not validated. Documented in the config so users
//!     opt INTO coverage rather than being forced to cover the whole
//!     repo upfront.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::{CodeLoreError, Result};

/// Conventional filename auto-discovered at the repo root, parallel
/// to `.codelore-teams` and `.codeloreignore`.
pub const ARCH_RULES_FILENAME: &str = ".codelore-arch-rules.toml";

/// One declared architectural layer.
#[derive(Debug, Clone)]
pub struct Layer {
    /// Display name — appears in violation reports.
    pub name: String,
    /// Path prefixes that classify a file into this layer. First-match
    /// wins (declaration order in the TOML file).
    pub paths: Vec<String>,
    /// Other layer names this layer is allowed to import from.
    /// Empty = isolated (the bottom of the dependency stack).
    pub may_depend_on: Vec<String>,
}

/// Complete parsed rule set.
#[derive(Debug, Clone, Default)]
pub struct LayerRules {
    /// Layers in declaration order (matters for first-match path
    /// classification).
    pub layers: Vec<Layer>,
}

impl LayerRules {
    /// Auto-discover `.codelore-arch-rules.toml` at the given repo
    /// root. Returns an empty rule set when the file doesn't exist
    /// (rules are opt-in — no file means no validation).
    ///
    /// # Errors
    ///
    /// [`CodeLoreError::Analysis`] on I/O / parse errors.
    pub fn discover(repo_root: &Path) -> Result<Self> {
        let path = repo_root.join(ARCH_RULES_FILENAME);
        if !path.exists() {
            return Ok(Self::default());
        }
        Self::from_path(&path)
    }

    /// Parse a rule set from a specific file path.
    ///
    /// # Errors
    ///
    /// [`CodeLoreError::Analysis`] on I/O / parse errors.
    pub fn from_path(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path).map_err(|e| {
            // Read-side input failure (user pointed `--arch-rules-file` at
            // something unreadable) → exit 3, mirroring `team_map::load`.
            // The parse failure below stays `Analysis` (exit 4).
            CodeLoreError::RepoIo(std::io::Error::new(
                e.kind(),
                format!("read arch-rules file {}: {e}", path.display()),
            ))
        })?;
        Self::from_text(&raw).map_err(|e| {
            CodeLoreError::Analysis(format!("parse arch-rules file {}: {e}", path.display()))
        })
    }

    /// Parse a rule set from in-memory TOML text. Used by tests and
    /// the public `from_path` after the I/O succeeds.
    ///
    /// # Errors
    ///
    /// Returns a static `&str` description of the parse failure.
    pub fn from_text(raw: &str) -> std::result::Result<Self, String> {
        // Deserialise via `toml::Table` (order-preserving) and walk
        // the `[layer.*]` keys in declaration order so `classify()`'s
        // first-match contract is reproducible across runs. The
        // default `HashMap<String, WireLayer>` shape lost order to
        // hash randomisation and made arch-violations non-deterministic
        // on inputs with nested-prefix layers.
        let table: toml::Table = toml::from_str(raw).map_err(|e| e.to_string())?;
        let mut layers: Vec<Layer> = Vec::new();
        if let Some(layer_section) = table.get("layer") {
            let layer_table = layer_section
                .as_table()
                .ok_or_else(|| "`layer` must be a TOML table".to_string())?;
            for (name, body_val) in layer_table {
                let body: WireLayer = body_val
                    .clone()
                    .try_into()
                    .map_err(|e: toml::de::Error| format!("layer `{name}`: {e}"))?;
                layers.push(Layer {
                    name: name.clone(),
                    paths: body.paths,
                    may_depend_on: body.may_depend_on,
                });
            }
        }
        // Validate that every `may_depend_on` reference resolves.
        let declared_names: std::collections::HashSet<&str> =
            layers.iter().map(|l| l.name.as_str()).collect();
        for layer in &layers {
            for dep in &layer.may_depend_on {
                if !declared_names.contains(dep.as_str()) {
                    return Err(format!(
                        "layer `{}` may_depend_on references undeclared layer `{dep}`",
                        layer.name
                    ));
                }
            }
        }
        Ok(Self { layers })
    }

    /// Classify a file path into a layer by first-match prefix scan.
    /// Returns `None` for files outside every declared layer.
    #[must_use]
    pub fn classify(&self, file_path: &str) -> Option<&str> {
        for layer in &self.layers {
            for prefix in &layer.paths {
                if file_path.starts_with(prefix.as_str()) {
                    return Some(&layer.name);
                }
            }
        }
        None
    }

    /// Validate an import edge: returns the violating layer name on a
    /// rule break, or `None` when the edge is allowed / unclassified /
    /// same-layer.
    #[must_use]
    pub fn validate(&self, src_path: &str, target_path: &str) -> Option<Violation> {
        let src_layer = self.classify(src_path)?;
        let target_layer = self.classify(target_path)?;
        if src_layer == target_layer {
            return None;
        }
        let src = self.layers.iter().find(|l| l.name == src_layer)?;
        if src.may_depend_on.iter().any(|d| d == target_layer) {
            return None;
        }
        Some(Violation {
            src_layer: src_layer.to_string(),
            target_layer: target_layer.to_string(),
        })
    }

    /// True if no layers are declared. Callers can short-circuit the
    /// validation pass entirely on empty config.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }
}

/// One detected rule break. The analysis aggregates these into
/// `(src_path, target_path, src_layer, target_layer)` rows for the
/// emitter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub src_layer: String,
    pub target_layer: String,
}

// Internal TOML deserialisation shape — kept private so the public
// `Layer` struct can carry a denormalised `name` field.
#[derive(Deserialize)]
struct WireLayer {
    #[serde(default)]
    paths: Vec<String>,
    #[serde(default)]
    may_depend_on: Vec<String>,
}

// Convenience for callers reading from the repo: build a path to the
// canonical filename without exposing the constant directly.
#[must_use]
pub fn arch_rules_path(repo_root: &Path) -> PathBuf {
    repo_root.join(ARCH_RULES_FILENAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_yields_empty_ruleset() {
        let rules = LayerRules::from_text("").unwrap();
        assert!(rules.is_empty());
    }

    #[test]
    fn three_layer_pyramid_parses() {
        let raw = r#"
[layer.domain]
paths = ["src/domain/"]
may_depend_on = []

[layer.app]
paths = ["src/app/"]
may_depend_on = ["domain"]

[layer.infra]
paths = ["src/infra/"]
may_depend_on = ["app", "domain"]
"#;
        let rules = LayerRules::from_text(raw).unwrap();
        assert_eq!(rules.layers.len(), 3);
    }

    #[test]
    fn undeclared_dependency_fails_parse() {
        let raw = r#"
[layer.app]
paths = ["src/app/"]
may_depend_on = ["ghost"]
"#;
        let err = LayerRules::from_text(raw).unwrap_err();
        assert!(err.contains("ghost"), "got {err}");
    }

    #[test]
    fn classification_matches_first_declared_prefix() {
        let raw = r#"
[layer.api]
paths = ["src/api/"]
may_depend_on = []

[layer.app]
paths = ["src/"]
may_depend_on = []
"#;
        let rules = LayerRules::from_text(raw).unwrap();
        // src/api/foo.rs matches both — first-declared (api) wins
        // deterministically via the order-preserving `toml::Table`
        // walk in `from_text`.
        assert_eq!(rules.classify("src/api/foo.rs"), Some("api"));
    }

    #[test]
    fn same_layer_imports_pass() {
        let raw = r#"
[layer.app]
paths = ["src/app/"]
may_depend_on = []
"#;
        let rules = LayerRules::from_text(raw).unwrap();
        assert_eq!(rules.validate("src/app/a.rs", "src/app/b.rs"), None);
    }

    #[test]
    fn allowed_downward_dependency_passes() {
        let raw = r#"
[layer.domain]
paths = ["src/domain/"]
may_depend_on = []

[layer.app]
paths = ["src/app/"]
may_depend_on = ["domain"]
"#;
        let rules = LayerRules::from_text(raw).unwrap();
        assert_eq!(
            rules.validate("src/app/handler.rs", "src/domain/user.rs"),
            None
        );
    }

    #[test]
    fn forbidden_upward_dependency_is_violation() {
        let raw = r#"
[layer.domain]
paths = ["src/domain/"]
may_depend_on = []

[layer.app]
paths = ["src/app/"]
may_depend_on = ["domain"]
"#;
        let rules = LayerRules::from_text(raw).unwrap();
        let v = rules
            .validate("src/domain/user.rs", "src/app/handler.rs")
            .expect("expected a violation");
        assert_eq!(v.src_layer, "domain");
        assert_eq!(v.target_layer, "app");
    }

    #[test]
    fn unclassified_files_skip_validation() {
        let raw = r#"
[layer.app]
paths = ["src/app/"]
may_depend_on = []
"#;
        let rules = LayerRules::from_text(raw).unwrap();
        assert_eq!(rules.validate("misc/util.rs", "src/app/x.rs"), None);
        assert_eq!(rules.validate("src/app/x.rs", "misc/util.rs"), None);
    }
}

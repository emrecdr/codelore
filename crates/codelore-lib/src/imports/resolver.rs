//! Target-to-tracked-path resolver for the import graph.
//!
//! Maps the raw `target` strings captured by `extractor.rs` to
//! repo-relative paths that exist in the `changes` table at HEAD,
//! when resolution is possible. Per-language strategies cover the
//! Rust `crate::` / Python `.` / JS/TS `./` patterns; Java FQN →
//! filesystem-path mapping is project-layout-specific and is not
//! attempted here.
//!
//! Architecture: every resolver takes the raw target + the importer's
//! source path + a `&HashSet<String>` of live-at-HEAD tracked paths,
//! and returns `Option<String>` (the resolved `target_path` on a hit).
//! The ingest layer iterates rows and calls the right resolver per
//! language; on hit it issues an UPDATE.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// JS/TS extension candidates checked in resolution order. `.ts` /
/// `.tsx` are tried first when the importer is a TypeScript file,
/// `.js` first when the importer is JavaScript — both rules surface
/// the most-likely-target preference of the toolchain.
const JS_EXTENSIONS: &[&str] = &["js", "jsx", "mjs", "cjs"];
const TS_EXTENSIONS: &[&str] = &["ts", "tsx"];

/// Resolve an import `target` from `importer_path` to a tracked path,
/// dispatching to the per-language resolver by the importer's file
/// extension. Returns `None` for languages without a resolver (Java
/// FQNs, etc.) or for unresolvable targets. The single dispatch point
/// shared by the HEAD ingest (`resolve_imports_at_head`) and the
/// historical scan (`architecture-trend`), so language coverage — and
/// which extensions route where — is defined in exactly one place.
#[must_use]
pub fn resolve_by_extension<S: std::hash::BuildHasher>(
    importer_path: &str,
    target: &str,
    live_paths: &HashSet<String, S>,
) -> Option<String> {
    let ext = Path::new(importer_path)
        .extension()
        .and_then(std::ffi::OsStr::to_str);
    match ext {
        Some("rs") => resolve_rust_path(importer_path, target, live_paths),
        Some("py" | "pyi") => resolve_python_relative(importer_path, target, live_paths),
        Some("js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx") => {
            resolve_js_relative(importer_path, target, live_paths)
        }
        _ => None,
    }
}

/// Resolve a Rust `use` path against the live-at-HEAD set. Handles
/// the conventional `crate::foo::bar` → `src/foo/bar.rs` |
/// `src/foo/bar/mod.rs` mapping. `self::` and `super::` resolve
/// relative to the importer's module directory (see [`module_dir`]),
/// which is the sibling `foo/` for a non-`mod.rs` `foo.rs`; each leading
/// `super` climbs one further level. External crates (anything not
/// starting with `crate::`/`self::`/`super::`) return `None`.
#[must_use]
pub fn resolve_rust_path<S: std::hash::BuildHasher>(
    importer_path: &str,
    target: &str,
    live_paths: &HashSet<String, S>,
) -> Option<String> {
    let segments: Vec<&str> = target
        .trim_end_matches(';')
        .split("::")
        .filter(|s| !s.is_empty())
        .collect();
    if segments.is_empty() {
        return None;
    }
    // Determine the file-system root. `crate::` resolves to the
    // importer's containing crate `src/` directory — for Cargo
    // workspaces (which codelore itself is) that means walking the
    // importer path backward from its `src/` boundary, NOT a literal
    // top-level `src/`. `self::` anchors at the importer's module
    // directory; each leading `super::` climbs one level above it.
    let (rest, root): (&[&str], PathBuf) = match segments[0] {
        "crate" => (&segments[1..], crate_src_root(importer_path)),
        "self" => (&segments[1..], module_dir(importer_path)),
        "super" => {
            // Each leading `super` climbs one module level from the
            // importer's own module directory, so `super::super::x`
            // resolves against the grandparent module.
            let mut climbed = 1;
            while segments.get(climbed) == Some(&"super") {
                climbed += 1;
            }
            let mut base = module_dir(importer_path);
            for _ in 0..climbed {
                base.pop();
            }
            (&segments[climbed..], base)
        }
        // Bare module paths (`foo::bar`) refer to extern crates in
        // Rust 2018+ — not the same crate. Skip resolution; these
        // come from `Cargo.toml` dependencies.
        _ => return None,
    };
    if rest.is_empty() {
        return None;
    }
    // Use the last meaningful identifier; trailing groups like
    // `{a, b, c}` or `*` are not module segments.
    let mut path_parts: Vec<&str> = rest
        .iter()
        .take_while(|s| !s.starts_with('{') && !s.starts_with('*') && !s.contains(' '))
        .copied()
        .collect();
    if path_parts.is_empty() {
        return None;
    }
    // The terminal segment may be an item INSIDE a module rather than
    // the module itself — try both shapes.
    let mut joined = root.clone();
    for part in &path_parts {
        joined.push(part);
    }
    let candidates = [
        format!("{}.rs", to_posix(&joined)),
        format!("{}/mod.rs", to_posix(&joined)),
    ];
    for c in &candidates {
        if live_paths.contains(c) {
            return Some(c.clone());
        }
    }
    // Drop the trailing identifier (item-inside-module case).
    path_parts.pop()?;
    let mut joined2 = root;
    for part in &path_parts {
        joined2.push(part);
    }
    let candidates2 = [
        format!("{}.rs", to_posix(&joined2)),
        format!("{}/mod.rs", to_posix(&joined2)),
    ];
    for c in &candidates2 {
        if live_paths.contains(c) {
            return Some(c.clone());
        }
    }
    None
}

/// Resolve a Python relative-import target (e.g. `.foo.bar`,
/// `..pkg.x`) against the live-at-HEAD path set. Handles both
/// `from . import foo` and `from .foo import bar` shapes via the
/// dot-prefix convention. External imports (no leading dot) return
/// `None`.
#[must_use]
pub fn resolve_python_relative<S: std::hash::BuildHasher>(
    importer_path: &str,
    target: &str,
    live_paths: &HashSet<String, S>,
) -> Option<String> {
    if !target.starts_with('.') {
        return None;
    }
    // Count leading dots to determine how many parents to climb.
    let dots = target.chars().take_while(|c| *c == '.').count();
    let rest = &target[dots..];
    let mut dir = parent_dir(importer_path);
    // First dot stays at importer's dir; each extra climbs one level.
    for _ in 1..dots {
        dir.pop();
    }
    let segments: Vec<&str> = rest
        .split('.')
        .filter(|s| !s.is_empty() && !s.contains(' '))
        .collect();
    if segments.is_empty() {
        // `from . import foo` — caller doesn't carry the imported
        // name in `target`; bail so we don't false-positive on the
        // importer's __init__.py.
        return None;
    }
    let mut joined = dir.clone();
    for part in &segments {
        joined.push(part);
    }
    let candidates = [
        format!("{}.py", to_posix(&joined)),
        format!("{}/__init__.py", to_posix(&joined)),
    ];
    for c in &candidates {
        if live_paths.contains(c) {
            return Some(c.clone());
        }
    }
    // Try without the last segment (item-inside-module case).
    let mut parts2 = segments.clone();
    parts2.pop()?;
    if parts2.is_empty() {
        return None;
    }
    let mut joined2 = dir;
    for part in &parts2 {
        joined2.push(part);
    }
    let candidates2 = [
        format!("{}.py", to_posix(&joined2)),
        format!("{}/__init__.py", to_posix(&joined2)),
    ];
    for c in &candidates2 {
        if live_paths.contains(c) {
            return Some(c.clone());
        }
    }
    None
}

fn parent_dir(path: &str) -> PathBuf {
    Path::new(path)
        .parent()
        .map_or_else(PathBuf::new, std::path::Path::to_path_buf)
}

/// The directory a Rust file's child modules live in — the anchor for
/// `self::` / `super::` resolution. For the module-root files
/// (`mod.rs`, `lib.rs`, `main.rs`) that's the file's own parent
/// directory; for any other `foo.rs` the children live in the sibling
/// `foo/` directory, so returning `parent/foo` corrects resolution
/// under the non-`mod.rs` module layout.
fn module_dir(importer_path: &str) -> PathBuf {
    let path = Path::new(importer_path);
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    match path.file_stem().and_then(std::ffi::OsStr::to_str) {
        Some("mod" | "lib" | "main") | None => parent.to_path_buf(),
        Some(stem) => parent.join(stem),
    }
}

/// `crate::` root resolution. For a single-crate repo this is
/// literally `src`; for a Cargo workspace it's the importer's
/// containing crate's `src/` directory. We detect the crate root by
/// scanning the importer's path for the rightmost `src/` boundary and
/// preserving every segment up to (and including) the `src` segment.
fn crate_src_root(importer_path: &str) -> PathBuf {
    let segments: Vec<&str> = importer_path.split('/').collect();
    // Walk right-to-left to find the LAST `src` segment so nested
    // module trees (`crates/foo/src/bar/src/baz.rs` is theoretical but
    // safe) anchor at the crate's `src`.
    if let Some(idx) = segments.iter().rposition(|s| *s == "src") {
        let prefix = &segments[..=idx];
        let mut p = PathBuf::new();
        for seg in prefix {
            p.push(seg);
        }
        return p;
    }
    PathBuf::from("src")
}

/// Convert a `PathBuf` to a forward-slash POSIX string regardless of
/// host platform. `live_paths` is populated from gix which always
/// emits `/`-separated paths; `PathBuf::to_string_lossy` would emit
/// `\` on Windows, breaking `HashSet::contains`.
fn to_posix(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// Resolve a JS/TS relative import target against the live-at-HEAD
/// path set. Returns the canonical repo-relative path if a match
/// exists, or `None` for unresolvable / external imports.
///
/// Handles the canonical patterns:
/// 1. `./foo` + `.ts/.js/.tsx/.jsx/.mjs/.cjs` — direct file match
/// 2. `./foo` + `index.{ts,tsx,js,...}` — package directory
/// 3. `./foo.ts` — explicit-extension form (direct hit, tried first)
/// 4. `./foo.js` from a TypeScript source — `NodeNext` names the emit
///    extension, so a trailing `.js/.jsx/.mjs/.cjs` is stripped and the
///    base is re-probed (`./foo.js` → `foo.ts`) when no literal file
///    matches.
#[must_use]
pub fn resolve_js_relative<S: std::hash::BuildHasher>(
    importer_path: &str,
    target: &str,
    live_paths: &HashSet<String, S>,
) -> Option<String> {
    if !target.starts_with("./") && !target.starts_with("../") {
        return None;
    }
    let importer_dir = Path::new(importer_path).parent().unwrap_or(Path::new(""));
    let joined = normalise_path(&importer_dir.join(target));
    let base = to_posix(&joined);

    // Determine extension priority based on importer's extension.
    let importer_ext = Path::new(importer_path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let primary: &[&str] = match importer_ext {
        "ts" | "tsx" => TS_EXTENSIONS,
        _ => JS_EXTENSIONS,
    };
    let secondary: &[&str] = match importer_ext {
        "ts" | "tsx" => JS_EXTENSIONS,
        _ => TS_EXTENSIONS,
    };

    // Pattern 3: explicit extension already present — a real
    // `./foo.mjs`/`./foo.js` file wins directly, before any strip-retry.
    if Path::new(&base).extension().is_some() && live_paths.contains(&base) {
        return Some(base);
    }
    // NodeNext/ESM `import x from "./foo.js"` names the *emit* extension
    // even when the file on disk is authored as `.ts`/`.tsx`. Strip a
    // trailing emit extension so the base+extension probes retry against
    // the source file (`./foo.js` → probe `foo.ts`, `foo.tsx`, …). A
    // non-emit extension (or none) leaves the base untouched.
    let probe_base = match Path::new(&base).extension().and_then(|e| e.to_str()) {
        Some(e @ ("js" | "jsx" | "mjs" | "cjs")) => base[..base.len() - e.len() - 1].to_string(),
        _ => base.clone(),
    };
    // Pattern 1: base + extension probe.
    for ext in primary.iter().chain(secondary.iter()) {
        let candidate = format!("{probe_base}.{ext}");
        if live_paths.contains(&candidate) {
            return Some(candidate);
        }
    }
    // Pattern 2: base/index + extension probe.
    for ext in primary.iter().chain(secondary.iter()) {
        let candidate = format!("{probe_base}/index.{ext}");
        if live_paths.contains(&candidate) {
            return Some(candidate);
        }
    }
    None
}

/// Collapse `./foo/../bar` → `bar` etc. without touching the
/// filesystem. `Path::canonicalize` would but it requires the file
/// to exist; this works purely lexically.
fn normalise_path(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in p.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            _ => out.push(component.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live(paths: &[&str]) -> HashSet<String> {
        paths.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn js_relative_resolves_to_sibling_js_file() {
        let live = live(&["src/foo.js", "src/util.js"]);
        let got = resolve_js_relative("src/foo.js", "./util", &live);
        assert_eq!(got, Some("src/util.js".to_string()));
    }

    #[test]
    fn js_relative_resolves_via_index_file() {
        let live = live(&["src/foo.js", "src/util/index.js"]);
        let got = resolve_js_relative("src/foo.js", "./util", &live);
        assert_eq!(got, Some("src/util/index.js".to_string()));
    }

    #[test]
    fn ts_importer_prefers_ts_extension() {
        let live = live(&["src/foo.ts", "src/util.ts", "src/util.js"]);
        let got = resolve_js_relative("src/foo.ts", "./util", &live);
        // .ts first (importer is TypeScript), then .js fallback.
        assert_eq!(got, Some("src/util.ts".to_string()));
    }

    #[test]
    fn relative_parent_directory_climbs_correctly() {
        let live = live(&["src/foo.js", "shared/helper.js"]);
        let got = resolve_js_relative("src/foo.js", "../shared/helper", &live);
        assert_eq!(got, Some("shared/helper.js".to_string()));
    }

    #[test]
    fn external_npm_imports_dont_resolve() {
        let live = live(&["src/foo.js"]);
        let got = resolve_js_relative("src/foo.js", "react", &live);
        // Bare specifiers (no ./ or ../ prefix) shouldn't resolve.
        assert!(got.is_none());
    }

    #[test]
    fn explicit_extension_skips_probing() {
        let live = live(&["src/foo.js", "src/util.mjs"]);
        let got = resolve_js_relative("src/foo.js", "./util.mjs", &live);
        assert_eq!(got, Some("src/util.mjs".to_string()));
    }

    #[test]
    fn missing_target_returns_none() {
        let live = live(&["src/foo.js"]);
        let got = resolve_js_relative("src/foo.js", "./nonexistent", &live);
        assert!(got.is_none());
    }

    #[test]
    fn tsx_importer_prefers_tsx_then_ts_then_js() {
        let live = live(&["src/App.tsx", "src/Button.jsx"]);
        let got = resolve_js_relative("src/App.tsx", "./Button", &live);
        // Importer is .tsx so [ts, tsx, js, jsx, mjs, cjs] is the order.
        // .ts not present; .tsx not present for Button; falls through to jsx.
        assert_eq!(got, Some("src/Button.jsx".to_string()));
    }

    #[test]
    fn nodenext_js_specifier_strips_to_ts_source() {
        // `import x from "./foo.js"` from a .ts importer resolves to the
        // authored `.ts` source — the emit extension is stripped and
        // re-probed.
        let live = live(&["src/app.ts", "src/foo.ts"]);
        let got = resolve_js_relative("src/app.ts", "./foo.js", &live);
        assert_eq!(got, Some("src/foo.ts".to_string()));
    }

    #[test]
    fn nodenext_js_specifier_strips_to_tsx_source() {
        // Same strip-retry falls through the extension priority to .tsx.
        let live = live(&["src/app.ts", "src/Widget.tsx"]);
        let got = resolve_js_relative("src/app.ts", "./Widget.js", &live);
        assert_eq!(got, Some("src/Widget.tsx".to_string()));
    }

    #[test]
    fn real_mjs_file_wins_over_ts_strip_retry() {
        // A literal `./foo.mjs` on disk is a direct Pattern-3 hit and must
        // win before the strip-retry considers the sibling `.ts`.
        let live = live(&["src/app.ts", "src/foo.mjs", "src/foo.ts"]);
        let got = resolve_js_relative("src/app.ts", "./foo.mjs", &live);
        assert_eq!(got, Some("src/foo.mjs".to_string()));
    }

    #[test]
    fn nodenext_strip_retry_still_existence_guarded() {
        // Stripping `.js` must not fabricate an edge when no source file
        // exists under any probed extension.
        let live = live(&["src/app.ts", "src/other.ts"]);
        let got = resolve_js_relative("src/app.ts", "./foo.js", &live);
        assert!(got.is_none(), "strip-retry must stay existence-guarded");
    }

    #[test]
    fn rust_self_resolves_against_sibling_dir_for_non_mod_file() {
        // `foo.rs`'s child modules live in the sibling `foo/` dir.
        let live = live(&["src/foo.rs", "src/foo/x.rs"]);
        let got = resolve_rust_path("src/foo.rs", "self::x", &live);
        assert_eq!(got, Some("src/foo/x.rs".to_string()));
    }

    #[test]
    fn rust_super_resolves_to_parent_module_for_non_mod_file() {
        // From `src/foo/bar.rs` (non-mod), `super::y` is `foo::y`.
        let live = live(&["src/foo/bar.rs", "src/foo/y.rs"]);
        let got = resolve_rust_path("src/foo/bar.rs", "super::y", &live);
        assert_eq!(got, Some("src/foo/y.rs".to_string()));
    }

    #[test]
    fn rust_self_from_mod_rs_is_unchanged() {
        // `mod.rs`'s module dir is its own parent — a strict no-op vs
        // the pre-fix behaviour.
        let live = live(&["src/foo/mod.rs", "src/foo/x.rs"]);
        let got = resolve_rust_path("src/foo/mod.rs", "self::x", &live);
        assert_eq!(got, Some("src/foo/x.rs".to_string()));
    }

    #[test]
    fn rust_super_from_mod_rs_is_unchanged() {
        // From `src/foo/mod.rs`, `super::y` climbs to the crate root.
        let live = live(&["src/foo/mod.rs", "src/y.rs"]);
        let got = resolve_rust_path("src/foo/mod.rs", "super::y", &live);
        assert_eq!(got, Some("src/y.rs".to_string()));
    }

    #[test]
    fn rust_grouped_leaves_each_resolve() {
        // Post-expansion leaves arrive one at a time; each maps home.
        let live = live(&["src/main.rs", "src/a.rs", "src/b.rs"]);
        assert_eq!(
            resolve_rust_path("src/main.rs", "crate::a", &live),
            Some("src/a.rs".to_string()),
        );
        assert_eq!(
            resolve_rust_path("src/main.rs", "crate::b", &live),
            Some("src/b.rs".to_string()),
        );
    }

    #[test]
    fn rust_super_does_not_false_edge_to_crate_root_decoy() {
        // Pre-fix, `super::x` from a non-mod file climbed to the crate
        // root and matched `src/x.rs`. It must now miss.
        let live = live(&["src/foo/bar.rs", "src/x.rs"]);
        let got = resolve_rust_path("src/foo/bar.rs", "super::x", &live);
        assert!(got.is_none(), "must not resolve to the crate-root decoy");
    }

    #[test]
    fn rust_chained_super_climbs_each_level() {
        // `super::super::x` from `src/a/b/c.rs` (non-mod) → `src/a/x.rs`.
        let live = live(&["src/a/b/c.rs", "src/a/x.rs"]);
        let got = resolve_rust_path("src/a/b/c.rs", "super::super::x", &live);
        assert_eq!(got, Some("src/a/x.rs".to_string()));
    }
}

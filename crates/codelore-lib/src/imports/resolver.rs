//! Target-to-tracked-path resolver for the import graph.
//!
//! Maps the raw `target` strings captured by `extractor.rs` to
//! repo-relative paths that exist in the `changes` table at HEAD,
//! when resolution is possible. Per-language strategies cover the
//! Rust `crate::` / Python `.` / JS/TS `./` patterns and Java FQN →
//! package-path suffix mapping.
//!
//! Architecture: every resolver takes the raw target + the importer's
//! source path + a `&HashSet<String>` of live-at-HEAD tracked paths,
//! and returns `Option<String>` (the resolved `target_path` on a hit).
//! The ingest layer iterates rows and calls the right resolver per
//! language; on hit it issues an UPDATE.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// JS/TS extension candidates checked in resolution order. `.ts` /
/// `.tsx` are tried first when the importer is a TypeScript file,
/// `.js` first when the importer is JavaScript — both rules surface
/// the most-likely-target preference of the toolchain.
const JS_EXTENSIONS: &[&str] = &["js", "jsx", "mjs", "cjs"];
const TS_EXTENSIONS: &[&str] = &["ts", "tsx"];

/// Resolve an import `target` from `importer_path` to a tracked path,
/// dispatching to the per-language resolver by the importer's file
/// extension. Returns `None` for extensions without a resolver or for
/// unresolvable targets. The single dispatch point shared by the HEAD
/// ingest (`resolve_imports_at_head`) and the historical scan
/// (`architecture-trend`), so language coverage — and which extensions
/// route where — is defined in exactly one place.
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
        Some("py" | "pyi") => resolve_python(importer_path, target, live_paths),
        Some("js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx") => {
            resolve_js_relative(importer_path, target, live_paths)
        }
        Some("java") => resolve_java(target, live_paths),
        _ => None,
    }
}

/// [`resolve_by_extension`] with the suffix index built once by the caller.
///
/// Every in-crate caller resolves many edges against one live-path set, so
/// building the index per edge would throw away the whole saving — hence a
/// second entry point rather than a cheaper body for the first, whose
/// signature is a published contract.
///
/// The two are kept as a specification and an optimisation of it, not as a
/// wrapper pair: `resolve_by_extension` above still scans, and
/// `the_suffix_index_answers_exactly_as_the_full_scan_does` asserts they
/// agree on every shape either resolver has a branch for. A delegating
/// wrapper would make that test tautological.
pub(crate) fn resolve_by_extension_indexed<S: std::hash::BuildHasher>(
    importer_path: &str,
    target: &str,
    live_paths: &HashSet<String, S>,
    index: &LivePathIndex<'_>,
) -> Option<String> {
    let ext = Path::new(importer_path)
        .extension()
        .and_then(std::ffi::OsStr::to_str);
    match ext {
        Some("rs") => resolve_rust_path(importer_path, target, live_paths),
        Some("py" | "pyi") => resolve_python_indexed(importer_path, target, live_paths, index),
        Some("js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx") => {
            resolve_js_relative(importer_path, target, live_paths)
        }
        Some("java") => resolve_java_indexed(target, index),
        _ => None,
    }
}

/// Resolve a Rust `use` path against the live-at-HEAD set. Handles
/// the conventional `crate::foo::bar` → `src/foo/bar.rs` |
/// `src/foo/bar/mod.rs` mapping. `self::` and `super::` resolve
/// relative to the importer's module directory (see [`module_dir`]),
/// which is the sibling `foo/` for a non-`mod.rs` `foo.rs`; each leading
/// `super` climbs one further level.
///
/// A bare path (`foo::bar`) is an extern crate in Rust 2018+, which is
/// usually a `Cargo.toml` dependency with no source here — but in a
/// workspace it may equally name a sibling member whose source *is*
/// tracked, so those are resolved against that member's `src/` (see
/// [`workspace_sibling_src_roots`]). Anything with no matching tracked
/// file, sibling or not, still returns `None`.
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
    let (rest, roots): (&[&str], Vec<PathBuf>) = match segments[0] {
        "crate" => (&segments[1..], vec![crate_src_root(importer_path)]),
        "self" => (&segments[1..], vec![module_dir(importer_path)]),
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
            (&segments[climbed..], vec![base])
        }
        // Bare module paths (`foo::bar`) refer to extern crates in
        // Rust 2018+ — not the same crate. Usually that means a
        // `Cargo.toml` dependency with no source in this repository, but a
        // Cargo workspace's own members are extern crates too, and their
        // sources ARE here: `use codelore_lib::analyses::hotspots` from the
        // CLI crate names a file three directories away. Left unresolved,
        // every edge between workspace members is dropped and the import
        // graph is severed at each crate boundary.
        name => {
            let roots = workspace_sibling_src_roots(importer_path, name);
            if roots.is_empty() {
                return None;
            }
            (&segments[1..], roots)
        }
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
    // the module itself — try both shapes. `roots` holds one entry for
    // `crate`/`self`/`super`; the workspace-sibling arm offers both
    // spellings of the member directory, and only one of them is ever
    // present in the live-path set.
    for root in &roots {
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
    }
    // Drop the trailing identifier (item-inside-module case).
    path_parts.pop()?;
    for root in &roots {
        let mut joined2 = root.clone();
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
    }
    None
}

/// Live paths grouped by their final path segment.
///
/// Two of the resolvers — Python absolute imports and Java FQNs — answer
/// "which tracked file ends with this suffix, and is it unique?". Both did
/// that by walking the whole live-path set once per import edge, which is
/// `O(edges x files)`: on a fifty-thousand-file Java repository with half a
/// million import statements that is tens of billions of string comparisons,
/// and `import os` in every Python file pays it too.
///
/// Only a path whose final segment equals the target's final segment can
/// possibly match, and that segment is known before the search. Grouping by
/// it turns the scan into a hash lookup plus a walk of the few files sharing
/// a basename. The predicate applied to each candidate is unchanged, so the
/// uniqueness refusal — two matches yield `None` rather than a guess — is
/// preserved exactly.
///
/// Internal on purpose: `codelore-lib` is published, so its `pub` surface is
/// a semver contract, and whether that surface is supported at all is an open
/// question for the maintainer. The published resolvers keep their signatures
/// and their behaviour; only the in-crate callers, which are the ones running
/// over whole repositories, take the indexed path.
pub(crate) struct LivePathIndex<'a> {
    by_basename: HashMap<&'a str, Vec<&'a str>>,
}

impl<'a> LivePathIndex<'a> {
    /// Group `paths` by final segment. Built once per resolution pass.
    pub(crate) fn build<I>(paths: I) -> Self
    where
        I: IntoIterator<Item = &'a String>,
    {
        let mut by_basename: HashMap<&'a str, Vec<&'a str>> = HashMap::new();
        for p in paths {
            let base = p.rsplit('/').next().unwrap_or(p.as_str());
            by_basename.entry(base).or_default().push(p.as_str());
        }
        Self { by_basename }
    }

    /// The tracked paths whose final segment is `basename`, in arbitrary
    /// order — the same order-independence the full scan had, since both
    /// refuse on more than one match rather than picking one.
    fn candidates(&self, basename: &str) -> &[&'a str] {
        self.by_basename
            .get(basename)
            .map_or(&[][..], Vec::as_slice)
    }

    /// The unique tracked path equal to `file` or ending in `/file`, or
    /// `None` when there is no match or more than one.
    ///
    /// This is the predicate both scanners applied, lifted verbatim; the only
    /// change is which paths it is applied to.
    fn unique_suffix_match(&self, file: &str) -> Option<String> {
        let basename = file.rsplit('/').next().unwrap_or(file);
        let suffix = format!("/{file}");
        let mut found: Option<&str> = None;
        for path in self.candidates(basename) {
            if *path == file || path.ends_with(&suffix) {
                if found.is_some() {
                    return None; // ambiguous suffix — refuse to guess
                }
                found = Some(path);
            }
        }
        found.map(ToString::to_string)
    }
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

/// Resolve a Python import `target` by shape: a leading-dot target is
/// relative (delegated to [`resolve_python_relative`]); everything else
/// is an absolute dotted module path resolved by suffix match.
///
/// The scanning counterpart of [`resolve_python_indexed`]. Both are kept:
/// this one is the specification the equivalence test checks the indexed
/// path against, and it is what the published dispatcher still calls.
fn resolve_python<S: std::hash::BuildHasher>(
    importer_path: &str,
    target: &str,
    live_paths: &HashSet<String, S>,
) -> Option<String> {
    if target.starts_with('.') {
        resolve_python_relative(importer_path, target, live_paths)
    } else {
        resolve_python_absolute(target, live_paths)
    }
}

/// Resolve an absolute Python module path (`mypkg.utils`, `os`) to a
/// tracked file by matching the dotted path as a repo-path suffix
/// (`a/b/c.py` or `a/b/c/__init__.py`) against the live-at-HEAD set —
/// Python has no explicit import root, so the module may live under any
/// source prefix. The match must be unique: zero or more than one
/// candidate yields `None`, so an ambiguous suffix never fabricates a
/// false edge and stdlib / third-party modules (no tracked file)
/// resolve to `None`.
#[must_use]
pub fn resolve_python_absolute<S: std::hash::BuildHasher>(
    target: &str,
    live_paths: &HashSet<String, S>,
) -> Option<String> {
    let rel = python_module_to_rel(target)?;
    python_absolute_from_index(&rel, &LivePathIndex::build(live_paths.iter()))
}

/// The body of [`resolve_python_absolute`], against a prepared index.
///
/// A module can be a file or a package `__init__.py`, and either match must be
/// unique *across both shapes* — a repository holding both `a/b/c.py` and
/// `a/b/c/__init__.py` is ambiguous, exactly as it was under the full scan.
fn python_absolute_from_index(rel: &str, index: &LivePathIndex<'_>) -> Option<String> {
    let module = index.unique_suffix_match(&format!("{rel}.py"));
    let package = index.unique_suffix_match(&format!("{rel}/__init__.py"));
    match (module, package) {
        (Some(hit), None) | (None, Some(hit)) => Some(hit),
        // Both shapes present, or neither: no unique answer.
        _ => None,
    }
}

/// [`resolve_python_absolute`] against a prepared index.
fn resolve_python_absolute_indexed(target: &str, index: &LivePathIndex<'_>) -> Option<String> {
    let rel = python_module_to_rel(target)?;
    python_absolute_from_index(&rel, index)
}

/// Python's dotted module path as a `/`-joined relative path, or `None` when
/// the target carries nothing usable. Shared so the scanning and indexed
/// entry points cannot disagree about what a module name means.
fn python_module_to_rel(target: &str) -> Option<String> {
    let rel: String = target
        .split('.')
        .filter(|s| !s.is_empty() && !s.contains(' '))
        .collect::<Vec<_>>()
        .join("/");
    (!rel.is_empty()).then_some(rel)
}

/// [`resolve_python`] against a prepared index.
fn resolve_python_indexed<S: std::hash::BuildHasher>(
    importer_path: &str,
    target: &str,
    live_paths: &HashSet<String, S>,
    index: &LivePathIndex<'_>,
) -> Option<String> {
    if target.starts_with('.') {
        // Relative imports resolve by constructing candidate paths and
        // hashing them, which is already constant-time per candidate.
        resolve_python_relative(importer_path, target, live_paths)
    } else {
        resolve_python_absolute_indexed(target, index)
    }
}

/// Resolve a Java `import` FQN (`com.foo.Bar`) to a tracked `.java` file
/// by matching the package/class path as a unique repo-path suffix
/// (`com/foo/Bar.java` — Java packages map directly to directories). The
/// match must be unique (zero or more than one candidate yields `None`)
/// so an ambiguous suffix never fabricates an edge; JDK / third-party
/// imports (`java.util.List`) have no tracked file and resolve to `None`.
/// Wildcard package imports (`com.foo.*`) name a directory, not a file,
/// and are skipped. A static-member import (`com.foo.Bar.baz`) or a
/// nested class (`com.foo.Outer.Inner`) resolves via a single strip-retry
/// to the enclosing class file — the full path is probed first, so a real
/// inner-class file wins before the strip.
#[must_use]
pub fn resolve_java<S: std::hash::BuildHasher>(
    target: &str,
    live_paths: &HashSet<String, S>,
) -> Option<String> {
    if target.contains('*') {
        return None;
    }
    let mut segments: Vec<&str> = target
        .split('.')
        .filter(|s| !s.is_empty() && !s.contains(' '))
        .collect();
    if segments.is_empty() {
        return None;
    }
    if let Some(hit) = java_suffix_match(&segments, live_paths) {
        return Some(hit);
    }
    // Inner-class / static-member import: strip the trailing member and
    // retry once against the enclosing class file.
    segments.pop();
    if segments.is_empty() {
        return None;
    }
    java_suffix_match(&segments, live_paths)
}

/// Match the `/`-joined package path (`com/foo/Bar.java`) as a unique
/// repo-path suffix. The leading `/` in the suffix guards against
/// partial-segment hits (`notcom/foo/Bar.java` for `com.foo.Bar`); the
/// `==` arm covers a class that lives at the repo root. A second match
/// yields `None` — an ambiguous suffix must not fabricate an edge.
fn java_suffix_match<S: std::hash::BuildHasher>(
    segments: &[&str],
    live_paths: &HashSet<String, S>,
) -> Option<String> {
    java_suffix_match_indexed(segments, &LivePathIndex::build(live_paths.iter()))
}

/// [`java_suffix_match`] against a prepared index.
fn java_suffix_match_indexed(segments: &[&str], index: &LivePathIndex<'_>) -> Option<String> {
    index.unique_suffix_match(&format!("{}.java", segments.join("/")))
}

/// [`resolve_java`] against a prepared index. Keeps the inner-class retry:
/// a static-member import strips its trailing member and asks once more
/// against the enclosing class file.
fn resolve_java_indexed(target: &str, index: &LivePathIndex<'_>) -> Option<String> {
    if target.contains('*') {
        return None;
    }
    let mut segments: Vec<&str> = target
        .split('.')
        .filter(|s| !s.is_empty() && !s.contains(' '))
        .collect();
    if segments.is_empty() {
        return None;
    }
    if let Some(hit) = java_suffix_match_indexed(&segments, index) {
        return Some(hit);
    }
    segments.pop();
    if segments.is_empty() {
        return None;
    }
    java_suffix_match_indexed(&segments, index)
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

/// The index of the rightmost `src` segment: the crate boundary that both
/// `crate::` resolution and sibling-member lookup anchor on. Walking
/// right-to-left keeps a nested module tree (`crates/foo/src/bar/src/baz.rs`
/// is theoretical but safe) anchored at the crate's own `src`.
///
/// Stated once because both callers below encode the same rule about where a
/// crate starts, and a layout they disagreed about would resolve `crate::`
/// against one boundary and siblings against another.
fn src_boundary(segments: &[&str]) -> Option<usize> {
    segments.iter().rposition(|s| *s == "src")
}

/// `crate::` root resolution. For a single-crate repo this is
/// literally `src`; for a Cargo workspace it's the importer's
/// containing crate's `src/` directory. We detect the crate root by
/// scanning the importer's path for the rightmost `src/` boundary and
/// preserving every segment up to (and including) the `src` segment.
fn crate_src_root(importer_path: &str) -> PathBuf {
    let segments: Vec<&str> = importer_path.split('/').collect();
    if let Some(idx) = src_boundary(&segments) {
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

/// Candidate `src/` roots for a sibling workspace member named
/// `crate_name`, derived from the importer's own position in the tree.
///
/// The importer already encodes the layout: a file at
/// `crates/codelore-cli/src/analyze.rs` sits in a member directory whose
/// parent holds the other members, so `codelore_lib` resolves against
/// `crates/codelore-lib/src`. Reading the importer path rather than the
/// workspace manifest keeps this module pure and lexical, and keeps the
/// lookup O(1) — scanning the live-path set per import would be quadratic
/// on a large repository.
///
/// A crate's name and its directory may differ by the underscore/hyphen
/// swap (`codelore_lib` ↔ `codelore-lib`) and both spellings occur in the
/// wild, so both are returned. Nothing here asserts either exists: the
/// caller matches each assembled path against the live-path set, so a real
/// third-party dependency finds no candidate and stays unresolved, exactly
/// as before.
///
/// Anchoring on the importer's own `src/` also scopes this to crate
/// *sources*, which is the intended boundary rather than a side effect. A
/// file under `tests/` or `benches/` has no `src` segment and so gains no
/// sibling root: an integration test importing the library it exercises is
/// a build-time fact, not an architectural dependency, and counting those
/// edges would give every library file afferent coupling from its own test
/// suite — flattening `instability` and inflating propagation cost with
/// usage that no production change can propagate through.
fn workspace_sibling_src_roots(importer_path: &str, crate_name: &str) -> Vec<PathBuf> {
    let segments: Vec<&str> = importer_path.split('/').collect();
    let Some(src_idx) = src_boundary(&segments) else {
        return Vec::new();
    };
    // `src/...` at the repository root is a single-crate layout: there is no
    // enclosing directory that could hold a sibling.
    if src_idx == 0 {
        return Vec::new();
    }
    let mut base = PathBuf::new();
    for seg in &segments[..src_idx - 1] {
        base.push(seg);
    }
    // A member's directory is conventionally hyphenated where the crate name
    // in code is underscored, so both spellings are offered and the live-path
    // match decides. They coincide for a name with no underscore, and the
    // second lookup would then be the first one repeated.
    let hyphenated = crate_name.replace('_', "-");
    let mut dirs = Vec::with_capacity(2);
    if hyphenated != crate_name {
        dirs.push(hyphenated);
    }
    dirs.push(crate_name.to_string());
    dirs.into_iter()
        .map(|dir| {
            let mut root = base.clone();
            root.push(dir);
            root.push("src");
            root
        })
        .collect()
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

    /// The equivalence proof for the suffix index: over a live set built to
    /// exercise every branch the two suffix resolvers have — a repo-root
    /// module, a nested one, a package `__init__.py`, a Java class under a
    /// deep source root, an inner-class import, a partial-segment near-miss
    /// (`notmypkg/` must not answer for `mypkg`), and two genuine ambiguities
    /// that must be refused — the indexed answer equals the scanning answer
    /// for every importer and target.
    ///
    /// Written as an agreement test rather than a table of expected values so
    /// it cannot drift from the behaviour it is protecting: the scanning
    /// functions are the specification, and they stay in the crate as the
    /// published entry points.
    #[test]
    fn the_suffix_index_answers_exactly_as_the_full_scan_does() {
        let paths = live(&[
            "root_module.py",
            "src/mypkg/utils.py",
            "src/mypkg/__init__.py",
            "src/notmypkg/utils.py",
            "vendor/mypkg/utils.py",
            "src/pkg/both.py",
            "src/pkg/both/__init__.py",
            "services/api/src/main/java/com/foo/Bar.java",
            "services/web/src/main/java/com/foo/Baz.java",
            "libs/a/src/main/java/com/dup/Same.java",
            "libs/b/src/main/java/com/dup/Same.java",
        ]);
        let index = LivePathIndex::build(paths.iter());

        let python_targets = [
            "root_module",
            "mypkg.utils", // ambiguous: src/ and vendor/ both carry it
            "mypkg",       // package __init__
            "pkg.both",    // ambiguous across module and package shapes
            "notmypkg.utils",
            "os",        // stdlib: no tracked file
            "",          // degenerate
            ".relative", // relative shape, handled by the other resolver
        ];
        for target in python_targets {
            assert_eq!(
                resolve_python_absolute(target, &paths),
                resolve_python_absolute_indexed(target, &index),
                "python absolute disagreed on {target:?}"
            );
        }

        let java_targets = [
            "com.foo.Bar",
            "com.foo.Baz",
            "com.dup.Same",      // ambiguous: two source roots carry it
            "com.foo.Bar.INNER", // inner-class retry
            "com.foo.*",         // wildcard: refused
            "com.absent.Thing",
            "",
        ];
        for target in java_targets {
            assert_eq!(
                resolve_java(target, &paths),
                resolve_java_indexed(target, &index),
                "java disagreed on {target:?}"
            );
        }

        // And through the dispatcher, which is what every caller uses.
        for (importer, target) in [
            ("src/mypkg/app.py", "root_module"),
            ("src/mypkg/app.py", "mypkg.utils"),
            ("services/api/src/main/java/com/foo/App.java", "com.foo.Bar"),
            (
                "services/api/src/main/java/com/foo/App.java",
                "com.dup.Same",
            ),
            ("src/lib.rs", "crate::thing"),
        ] {
            assert_eq!(
                resolve_by_extension(importer, target, &paths),
                resolve_by_extension_indexed(importer, target, &paths, &index),
                "dispatch disagreed on {importer:?} -> {target:?}"
            );
        }
    }

    /// The property the index must not quietly lose: two files with the same
    /// basename under different roots are ambiguous, and an ambiguous suffix
    /// is refused rather than guessed. Asserted directly, because grouping by
    /// basename is exactly the operation that could have collapsed them.
    #[test]
    fn an_ambiguous_suffix_is_still_refused_under_the_index() {
        let paths = live(&[
            "libs/a/src/main/java/com/dup/Same.java",
            "libs/b/src/main/java/com/dup/Same.java",
            "src/mypkg/utils.py",
            "vendor/mypkg/utils.py",
        ]);
        let index = LivePathIndex::build(paths.iter());
        assert_eq!(resolve_java_indexed("com.dup.Same", &index), None);
        assert_eq!(resolve_python_absolute_indexed("mypkg.utils", &index), None);
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
    fn rust_workspace_sibling_crate_resolves_across_the_member_boundary() {
        // The shape this repository is: a CLI member importing the library
        // member. Before, every such edge was dropped as "extern crate".
        let live = live(&[
            "crates/codelore-cli/src/analyze.rs",
            "crates/codelore-lib/src/analyses/hotspots.rs",
        ]);
        let got = resolve_rust_path(
            "crates/codelore-cli/src/analyze.rs",
            "codelore_lib::analyses::hotspots",
            &live,
        );
        assert_eq!(
            got,
            Some("crates/codelore-lib/src/analyses/hotspots.rs".to_string())
        );
    }

    #[test]
    fn rust_workspace_sibling_resolves_with_an_underscore_directory() {
        // Cargo permits the member directory to keep the underscores the
        // crate name uses; both spellings are offered and only the one that
        // exists can match.
        let live = live(&["crates/app/src/main.rs", "crates/my_lib/src/util.rs"]);
        let got = resolve_rust_path("crates/app/src/main.rs", "my_lib::util", &live);
        assert_eq!(got, Some("crates/my_lib/src/util.rs".to_string()));
    }

    #[test]
    fn rust_third_party_crate_stays_unresolved() {
        // The anti-false-positive case: a real dependency has no source in
        // the tree, so no assembled candidate is live and the import stays
        // external — exactly the pre-change behaviour.
        let live = live(&[
            "crates/codelore-cli/src/analyze.rs",
            "crates/codelore-lib/src/lib.rs",
        ]);
        for target in ["serde::Deserialize", "std::fs::read_to_string", "anyhow"] {
            assert!(
                resolve_rust_path("crates/codelore-cli/src/analyze.rs", target, &live).is_none(),
                "{target} must stay unresolved",
            );
        }
    }

    #[test]
    fn rust_single_crate_layout_has_no_siblings() {
        // `src/...` at the repository root: there is no enclosing directory
        // that could hold a sibling member, so a bare path is still extern.
        let live = live(&["src/main.rs", "src/other/src/thing.rs"]);
        let got = resolve_rust_path("src/main.rs", "other::thing", &live);
        assert!(
            got.is_none(),
            "single-crate layout must not invent a sibling"
        );
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

    #[test]
    fn python_relative_sibling_resolves() {
        let live = live(&["pkg/app.py", "pkg/x.py"]);
        let got = resolve_python("pkg/app.py", ".x", &live);
        assert_eq!(got, Some("pkg/x.py".to_string()));
    }

    #[test]
    fn python_relative_module_resolves() {
        let live = live(&["pkg/app.py", "pkg/mod.py"]);
        let got = resolve_python("pkg/app.py", ".mod", &live);
        assert_eq!(got, Some("pkg/mod.py".to_string()));
    }

    #[test]
    fn python_parent_relative_resolves() {
        // `..pkg` from `a/b/app.py` climbs to `a/` then names `pkg`.
        let live = live(&["a/b/app.py", "a/pkg.py"]);
        let got = resolve_python("a/b/app.py", "..pkg", &live);
        assert_eq!(got, Some("a/pkg.py".to_string()));
    }

    #[test]
    fn python_relative_subpackage_init_resolves() {
        let live = live(&["pkg/app.py", "pkg/sub/__init__.py"]);
        let got = resolve_python("pkg/app.py", ".sub", &live);
        assert_eq!(got, Some("pkg/sub/__init__.py".to_string()));
    }

    #[test]
    fn python_absolute_module_resolves_by_suffix() {
        let live = live(&["src/mypkg/utils.py", "src/mypkg/__init__.py"]);
        let got = resolve_python_absolute("mypkg.utils", &live);
        assert_eq!(got, Some("src/mypkg/utils.py".to_string()));
    }

    #[test]
    fn python_absolute_package_init_resolves() {
        let live = live(&["src/mypkg/__init__.py", "src/other.py"]);
        let got = resolve_python_absolute("mypkg", &live);
        assert_eq!(got, Some("src/mypkg/__init__.py".to_string()));
    }

    #[test]
    fn python_absolute_repo_root_module_resolves() {
        // A module that lives at the repo root matches via the `==` arm.
        let live = live(&["mypkg.py", "other.py"]);
        let got = resolve_python_absolute("mypkg", &live);
        assert_eq!(got, Some("mypkg.py".to_string()));
    }

    #[test]
    fn python_absolute_ambiguous_suffix_returns_none() {
        // Two tracked files end with `mypkg/utils.py` — an ambiguous
        // suffix must not fabricate an edge.
        let live = live(&["a/mypkg/utils.py", "b/mypkg/utils.py"]);
        let got = resolve_python_absolute("mypkg.utils", &live);
        assert!(got.is_none(), "ambiguous suffix must resolve to None");
    }

    #[test]
    fn python_absolute_stdlib_returns_none() {
        // `os` has no tracked file — stdlib / third-party resolve to None.
        let live = live(&["src/app.py", "src/mypkg/utils.py"]);
        let got = resolve_python_absolute("os", &live);
        assert!(got.is_none(), "stdlib module must resolve to None");
    }

    #[test]
    fn python_absolute_partial_segment_does_not_match() {
        // `mypkg.utils` must not match `notmypkg/utils.py` — the leading
        // slash in the suffix guards against partial-segment hits.
        let live = live(&["src/notmypkg/utils.py"]);
        let got = resolve_python_absolute("mypkg.utils", &live);
        assert!(got.is_none(), "partial-segment suffix must not match");
    }

    #[test]
    fn java_import_resolves_by_package_suffix() {
        let live = live(&["src/main/java/com/foo/Bar.java"]);
        let got = resolve_java("com.foo.Bar", &live);
        assert_eq!(got, Some("src/main/java/com/foo/Bar.java".to_string()));
    }

    #[test]
    fn java_repo_root_class_resolves() {
        // A class that lives at the repo root matches via the `==` arm.
        let live = live(&["Bar.java", "Other.java"]);
        let got = resolve_java("Bar", &live);
        assert_eq!(got, Some("Bar.java".to_string()));
    }

    #[test]
    fn java_jdk_import_returns_none() {
        // `java.util.List` has no tracked file — JDK / third-party
        // imports resolve to None.
        let live = live(&["src/main/java/com/foo/App.java"]);
        let got = resolve_java("java.util.List", &live);
        assert!(got.is_none(), "JDK import must resolve to None");
    }

    #[test]
    fn java_ambiguous_suffix_returns_none() {
        // Two tracked files end with `com/foo/Bar.java` — an ambiguous
        // suffix must not fabricate an edge.
        let live = live(&["a/com/foo/Bar.java", "b/com/foo/Bar.java"]);
        let got = resolve_java("com.foo.Bar", &live);
        assert!(got.is_none(), "ambiguous suffix must resolve to None");
    }

    #[test]
    fn java_partial_segment_does_not_match() {
        // `com.foo.Bar` must not match `notcom/foo/Bar.java` — the leading
        // slash in the suffix guards against partial-segment hits.
        let live = live(&["src/notcom/foo/Bar.java"]);
        let got = resolve_java("com.foo.Bar", &live);
        assert!(got.is_none(), "partial-segment suffix must not match");
    }

    #[test]
    fn java_wildcard_import_returns_none() {
        // `com.foo.*` names a directory, not a file — skipped.
        let live = live(&["src/main/java/com/foo/Bar.java"]);
        let got = resolve_java("com.foo.*", &live);
        assert!(got.is_none(), "wildcard import must resolve to None");
    }

    #[test]
    fn java_inner_class_strips_to_enclosing_file() {
        // `com.foo.Outer.Inner` with only the enclosing `Outer.java` live
        // strips one segment and resolves to it.
        let live = live(&["src/main/java/com/foo/Outer.java"]);
        let got = resolve_java("com.foo.Outer.Inner", &live);
        assert_eq!(got, Some("src/main/java/com/foo/Outer.java".to_string()));
    }

    #[test]
    fn java_static_member_strips_to_class_file() {
        // A static-member import `com.foo.Bar.baz` strips the member and
        // resolves to the enclosing class file.
        let live = live(&["src/main/java/com/foo/Bar.java"]);
        let got = resolve_java("com.foo.Bar.baz", &live);
        assert_eq!(got, Some("src/main/java/com/foo/Bar.java".to_string()));
    }

    #[test]
    fn java_full_path_wins_over_strip_retry() {
        // A real inner-class file resolves at level 0 before the
        // strip-retry considers the enclosing class.
        let live = live(&[
            "src/main/java/com/foo/Outer/Inner.java",
            "src/main/java/com/foo/Outer.java",
        ]);
        let got = resolve_java("com.foo.Outer.Inner", &live);
        assert_eq!(
            got,
            Some("src/main/java/com/foo/Outer/Inner.java".to_string()),
        );
    }

    #[test]
    fn java_strip_retry_still_ambiguity_guarded() {
        // Stripping a segment must not fabricate an edge when the
        // enclosing class name is ambiguous across two files.
        let live = live(&["a/com/foo/Outer.java", "b/com/foo/Outer.java"]);
        let got = resolve_java("com.foo.Outer.Inner", &live);
        assert!(got.is_none(), "ambiguous strip-retry must resolve to None");
    }
}

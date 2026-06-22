use codelore_lib::Options;
use codelore_lib::facts::FactsDb;
use codelore_lib::provenance::Manifest;
use codelore_lib::repo::GixRepo;

#[test]
fn manifest_captures_basic_fields() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let manifest = Manifest::capture(&db, &opts, "revisions").expect("capture");
    assert!(
        !manifest.codelore_version.is_empty(),
        "codelore_version should be populated"
    );
    assert_eq!(manifest.analysis, "revisions");
    assert_eq!(manifest.min_revs, 1);

    let json = manifest.to_json().expect("json");
    assert!(json.contains("\"analysis\""));
    assert!(json.contains("\"codelore_version\""));
}

/// Schema v2 reproducibility-critical fields: `head_sha`,
/// `cache_key_hash`, `rust_version`, `target_triple`, and
/// `grammars`. SLSA L3 verifiers depend on these — a regression
/// that dropped them would surface only when an auditor tried to
/// reproduce a build and discovered the manifest carries
/// insufficient state.
#[test]
fn manifest_captures_reproducibility_fields() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let m = Manifest::capture(&db, &opts, "revisions").expect("capture");

    // Schema version pinned at 2. Bump again when
    // future field changes break consumer compatibility.
    assert_eq!(m.schema_version, 2);

    // head_sha must round-trip through the fact store. The
    // 40-char check pins the contract: a degenerate empty value
    // here means the ingest didn't populate `commits` (test broken)
    // OR the query rolled back (regression). Both surface clearly.
    assert_eq!(
        m.head_sha.len(),
        40,
        "head_sha must be the 40-char SHA, got {} chars: {:?}",
        m.head_sha.len(),
        m.head_sha,
    );

    // Cache key hash is the SHA-256 of the cache key — 64 hex chars.
    assert_eq!(
        m.cache_key_hash.len(),
        64,
        "cache_key_hash must be 64 hex chars (SHA-256), got {} chars",
        m.cache_key_hash.len(),
    );
    assert!(
        m.cache_key_hash.chars().all(|c| c.is_ascii_hexdigit()),
        "cache_key_hash must be hex-encoded",
    );

    // Rust version + target triple. We don't assert specific values
    // (a future toolchain bump shouldn't break this test) — just
    // that they're populated.
    assert!(!m.rust_version.is_empty());
    assert_ne!(
        m.target_triple, "unknown",
        "target_triple unknown — extend the cfg!() ladder in provenance/mod.rs",
    );

    // Grammar map must include every tree-sitter dep we ship. The
    // exact pin values can drift over time (tree-sitter bumps are
    // coordinated per CLAUDE.md) but every entry must be present and
    // non-empty.
    for crate_name in [
        "tree-sitter",
        "tree-sitter-rust",
        "tree-sitter-python",
        "tree-sitter-java",
        "tree-sitter-javascript",
        "tree-sitter-typescript",
    ] {
        let pin = m
            .grammars
            .get(crate_name)
            .unwrap_or_else(|| panic!("grammar pin missing: {crate_name}"));
        assert!(
            !pin.is_empty(),
            "grammar pin for {crate_name} must not be empty",
        );
    }

    // JSON serialization must include every new field — downstream
    // consumers grep on the field names.
    let json = m.to_json().expect("json");
    for field in [
        "\"schema_version\"",
        "\"head_sha\"",
        "\"cache_key_hash\"",
        "\"rust_version\"",
        "\"target_triple\"",
        "\"grammars\"",
    ] {
        assert!(
            json.contains(field),
            "serialized manifest missing field {field}",
        );
    }
}

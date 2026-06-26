//! End-to-end coverage for the structure×history fusion analyses
//! (`modularity-violations` + `unstable-interface`) over a purpose-built
//! repo with a KNOWN import + co-change shape, so the fusion semantics
//! can be asserted precisely.
//!
//! Fixture (Rust crate, `crate::` imports resolve to `src/*.rs`):
//!   - `src/core_mod.rs` — a hub imported by `a`, `b`, `c` (fan-in 3).
//!     Co-changes with `a` and `b` (but NOT `c`, which is added alone).
//!   - `src/a.rs`, `src/b.rs`, `src/c.rs` — `use crate::core_mod;`.
//!   - `src/helper.rs` + `src/util.rs` — co-change every commit but
//!     never import each other → a textbook modularity violation.
//!
//! Expectations under `permissive_coupling_opts` (fisher = 1.0, so a
//! single shared commit yields a co-change pair):
//!   - `modularity-violations` CONTAINS (helper, util); EXCLUDES
//!     (a, `core_mod`) and (b, `core_mod`) because those have import edges.
//!   - `unstable-interface` surfaces `core_mod` with `fan_in` = 3 and
//!     `coupled_dependents` = 2 (a, b — not c, which never co-changes).

use codelore_lib::analyses::modularity_violations::run_modularity_violations;
use codelore_lib::analyses::unstable_interface::run_unstable_interface;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;
use codelore_lib::test_support::permissive_coupling_opts;
use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .expect("spawn git");
    assert!(status.success(), "git {args:?} failed");
}

fn commit(dir: &Path, iso_date: &str, msg: &str) {
    git(dir, &["add", "."]);
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["commit", "-m", msg, "--quiet"])
        .env("GIT_AUTHOR_DATE", iso_date)
        .env("GIT_COMMITTER_DATE", iso_date)
        .status()
        .expect("spawn git commit");
    assert!(status.success(), "git commit {msg} failed");
}

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// Build the hub fixture and return its tempdir (kept alive by caller).
fn build_hub_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-b", "main", "--quiet"]);
    git(p, &["config", "user.email", "hub@example.com"]);
    git(p, &["config", "user.name", "Hub"]);

    // C1: crate skeleton + the hub.
    write(
        p,
        "Cargo.toml",
        "[package]\nname=\"fix\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
    );
    write(
        p,
        "src/lib.rs",
        "pub mod core_mod;\npub mod a;\npub mod b;\npub mod c;\npub mod helper;\npub mod util;\n",
    );
    write(p, "src/core_mod.rs", "pub fn core_fn() {}\n");
    commit(p, "2026-01-01T10:00:00Z", "skeleton + hub");

    // C2: a imports the hub AND co-changes with it.
    write(
        p,
        "src/a.rs",
        "use crate::core_mod;\npub fn a() { core_mod::core_fn(); }\n",
    );
    write(p, "src/core_mod.rs", "pub fn core_fn() { let _ = 1; }\n");
    commit(p, "2026-01-02T10:00:00Z", "add a + touch hub");

    // C3: b imports the hub AND co-changes with it.
    write(p, "src/b.rs", "use crate::core_mod;\npub fn b() {}\n");
    write(p, "src/core_mod.rs", "pub fn core_fn() { let _ = 2; }\n");
    commit(p, "2026-01-03T10:00:00Z", "add b + touch hub");

    // C4: c imports the hub but is added ALONE — imports without co-change.
    write(p, "src/c.rs", "use crate::core_mod;\npub fn c() {}\n");
    commit(p, "2026-01-04T10:00:00Z", "add c alone");

    // C5: helper + util co-change but never import each other.
    write(p, "src/helper.rs", "pub fn helper() {}\n");
    write(p, "src/util.rs", "pub fn util() {}\n");
    commit(p, "2026-01-05T10:00:00Z", "helper + util together");

    dir
}

fn ingested(dir: &Path) -> (FactsDb, codelore_lib::Options) {
    let repo = GixRepo::open(dir).expect("open hub repo");
    let db = FactsDb::new_in_memory().expect("in-memory db");
    let opts = permissive_coupling_opts(dir.to_path_buf());
    db.ingest(&repo, &opts).expect("ingest hub repo");
    (db, opts)
}

#[test]
fn modularity_violations_flags_implicit_coupling_only() {
    let dir = build_hub_repo();
    let (db, opts) = ingested(dir.path());

    let rows = run_modularity_violations(&db, &opts).expect("run modularity-violations");
    let pairs: HashSet<(String, String)> = rows
        .iter()
        .map(|r| (r.entity_a.clone(), r.entity_b.clone()))
        .collect();

    // helper ↔ util co-change with NO import edge → a violation.
    assert!(
        pairs.contains(&("src/helper.rs".into(), "src/util.rs".into())),
        "helper/util co-change without an import edge must be flagged; got {pairs:?}"
    );

    // a ↔ core_mod and b ↔ core_mod co-change but HAVE an import edge →
    // excluded. (coupling canonical ordering: lexicographically smaller
    // path first, so the pair keys are (a, core_mod) / (b, core_mod).)
    assert!(
        !pairs.contains(&("src/a.rs".into(), "src/core_mod.rs".into())),
        "a→core_mod is a real import edge; must NOT be a modularity violation; got {pairs:?}"
    );
    assert!(
        !pairs.contains(&("src/b.rs".into(), "src/core_mod.rs".into())),
        "b→core_mod is a real import edge; must NOT be a modularity violation; got {pairs:?}"
    );
}

#[test]
fn unstable_interface_surfaces_the_churning_hub() {
    let dir = build_hub_repo();
    let (db, opts) = ingested(dir.path());

    let rows = run_unstable_interface(&db, &opts).expect("run unstable-interface");
    let core = rows
        .iter()
        .find(|r| r.path == "src/core_mod.rs")
        .unwrap_or_else(|| {
            panic!("core_mod should surface as an unstable interface; got {rows:?}")
        });

    // a, b, c all import the hub.
    assert_eq!(core.fan_in, 3, "core_mod is imported by a, b, c");
    // a and b co-change with the hub; c was added alone, so it is a
    // dependent but NOT a coupled dependent.
    assert_eq!(
        core.coupled_dependents, 2,
        "only a and b co-change with the hub (c is imported-but-not-coupled)"
    );
    // Composite is revisions × coupled_dependents.
    assert!((core.instability_score - f64::from(core.revisions) * 2.0).abs() < 1e-9);

    // Rows are ranked by instability score descending.
    for w in rows.windows(2) {
        assert!(
            w[0].instability_score >= w[1].instability_score,
            "rows must be sorted by instability_score descending"
        );
    }
}

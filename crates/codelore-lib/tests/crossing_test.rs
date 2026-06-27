//! End-to-end coverage for `crossing` (DV8). Fixture: `mid` is a
//! structural "X" — imported by `a`, `b`, `c` (fan-in 3) and importing
//! `x`, `y`, `z` (fan-out 3). `mid` co-changes with one importer (`a`)
//! and one import (`x`), so change flows through it both ways → a
//! crossing. `b`/`c`/`y`/`z` never co-change with it; the leaves (`a`
//! fan-in 0, `x` fan-out 0) are not crossings.
//!
//! Fixture shape matters for the co-change side: `mid` is kept OUT of
//! the skeleton/noise commits and only appears where it co-changes with
//! `a` or `x`. If `mid` instead appeared in most commits, its
//! co-occurrence would be statistically unsurprising (Fisher p → 1.0)
//! and nothing would couple to it.

use codelore_lib::analyses::crossing::run_crossing;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;
use codelore_lib::test_support::permissive_coupling_opts;
use std::collections::HashMap;
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

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

fn commit(dir: &Path, day: u32, msg: &str) {
    git(dir, &["add", "."]);
    let date = format!("2026-01-{day:02}T10:00:00Z");
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["commit", "-m", msg, "--quiet"])
        .env("GIT_AUTHOR_DATE", &date)
        .env("GIT_COMMITTER_DATE", &date)
        .status()
        .expect("spawn git commit");
    assert!(status.success(), "git commit {msg} failed");
}

fn mid_src(n: u32) -> String {
    format!("use crate::x;\nuse crate::y;\nuse crate::z;\npub fn mid() {{ let _ = {n}; }}\n")
}

#[test]
fn crossing_flags_the_bidirectional_x() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-b", "main", "--quiet"]);
    git(p, &["config", "user.email", "x@example.com"]);
    git(p, &["config", "user.name", "X"]);

    write(
        p,
        "Cargo.toml",
        "[package]\nname=\"x\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
    );
    write(
        p,
        "src/lib.rs",
        "pub mod mid;\npub mod a;\npub mod b;\npub mod c;\npub mod x;\npub mod y;\npub mod z;\n",
    );
    // C1: skeleton WITHOUT mid/a/x. b,c import mid (resolved at HEAD);
    // y,z are leaves mid imports.
    write(p, "src/b.rs", "use crate::mid;\npub fn f() {}\n");
    write(p, "src/c.rs", "use crate::mid;\npub fn f() {}\n");
    write(p, "src/y.rs", "pub fn y() {}\n");
    write(p, "src/z.rs", "pub fn z() {}\n");
    commit(p, 1, "skeleton (no mid/a/x)");

    // C2–C3: mid co-changes with importer `a`.
    for (day, n) in [(2u32, 1), (3, 2)] {
        write(p, "src/mid.rs", &mid_src(n));
        write(
            p,
            "src/a.rs",
            &format!("use crate::mid;\npub fn f() {{ let _ = {n}; mid::mid(); }}\n"),
        );
        commit(p, day, "mid + a");
    }
    // C4–C5: mid co-changes with import `x`.
    for (day, n) in [(4u32, 3), (5, 4)] {
        write(p, "src/mid.rs", &mid_src(n));
        write(p, "src/x.rs", &format!("pub fn x() {{ let _ = {n}; }}\n"));
        commit(p, day, "mid + x");
    }
    // C6: noise touching b only (keeps mid's appearance fraction low).
    write(
        p,
        "src/b.rs",
        "use crate::mid;\npub fn f() { let _ = 1; }\n",
    );
    commit(p, 6, "noise (b only)");

    let repo = GixRepo::open(p).expect("open repo");
    let db = FactsDb::new_in_memory().expect("in-memory db");
    let opts = permissive_coupling_opts(p.to_path_buf());
    db.ingest(&repo, &opts).expect("ingest repo");

    let rows = run_crossing(&db, &opts).expect("run crossing");
    let by: HashMap<&str, &_> = rows.iter().map(|r| (r.path.as_str(), r)).collect();

    let mid = by
        .get("src/mid.rs")
        .unwrap_or_else(|| panic!("mid should be a crossing; got {rows:?}"));
    assert_eq!(mid.fan_in, 3, "a, b, c import mid: {mid:?}");
    assert_eq!(mid.fan_out, 3, "mid imports x, y, z: {mid:?}");
    // `a` (importer) and `x` (import) are mid's only co-change partners.
    assert_eq!(
        mid.coupled_upstream, 1,
        "only `a` co-changes upstream: {mid:?}"
    );
    assert_eq!(
        mid.coupled_downstream, 1,
        "only `x` co-changes downstream: {mid:?}"
    );
    assert!((mid.crossing_score - 2.0).abs() < 1e-9, "{mid:?}");

    // The leaves are not crossings (one arm of the X is missing).
    assert!(
        !by.contains_key("src/a.rs"),
        "a has fan-in 0; not a crossing"
    );
    assert!(
        !by.contains_key("src/x.rs"),
        "x has fan-out 0; not a crossing"
    );
}

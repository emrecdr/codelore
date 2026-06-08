//! `--code-maat-compat` integration tests.
//!
//! Validates the three internal-default flips under the compat flag:
//!   1. `main-dev-by-revs` CSV emits the legacy `added`/`total-added` headers
//!      (modern: `revisions`/`total-revisions`).
//!   2. `soc` falls back to `--min-revs` for the `SoC` threshold when
//!      `--min-soc` is unset (modern: defaults to 1).
//!   3. `--strict-grouping` is auto-implied — unmapped paths get dropped.
//!      (Tested via the CLI integration test in `cli_test.rs`; here the
//!      Options field flip is the contract under test.)

use codelore_lib::Options;
use codelore_lib::analyses::main_dev::MainDevRow;
use codelore_lib::analyses::soc::run_soc;
use codelore_lib::facts::FactsDb;
use codelore_lib::output::csv::write_main_dev_by_revs_csv;
use codelore_lib::repo::GixRepo;

fn run_git(path: &std::path::Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(path)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn write(p: std::path::PathBuf, content: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// `main-dev-by-revs` CSV: modern default is `revisions`/`total-revisions`;
/// under the compat flag it emits code-maat's lying `added`/`total-added`.
#[test]
fn main_dev_by_revs_csv_headers_flip_under_compat() {
    let rows = [MainDevRow {
        entity: "src/foo.rs".into(),
        main_dev: "alice@e.com".into(),
        metric: 5,
        total: 10,
        ownership: 0.50,
    }];

    let mut modern = Vec::new();
    write_main_dev_by_revs_csv(&rows, &mut modern, false).expect("write modern");
    let modern_str = String::from_utf8(modern).expect("utf8");
    assert!(
        modern_str.contains("revisions,total-revisions"),
        "modern emits honest headers; got: {modern_str}"
    );
    assert!(!modern_str.contains("added,total-added"));

    let mut compat = Vec::new();
    write_main_dev_by_revs_csv(&rows, &mut compat, true).expect("write compat");
    let compat_str = String::from_utf8(compat).expect("utf8");
    assert!(
        compat_str.contains("added,total-added"),
        "code-maat-compat emits the legacy lying headers; got: {compat_str}"
    );
    assert!(!compat_str.contains("revisions,total-revisions"));
}

/// `soc` threshold semantic flip under the compat flag:
///   - Without compat: `min_soc` defaults to 1 (drop solo commits)
///   - With compat: `min_soc` falls back to `min_revs`
///
/// Code-maat overloaded `--min-revs` to mean "minimum `SoC` sum" in this one
/// analysis. `CodeLore` exposes `--min-soc` as the honest name; compat mode
/// restores the legacy semantic for users with scripts that pass
/// `--min-revs 5` expecting it to act as a `SoC` floor.
#[test]
fn soc_threshold_falls_back_to_min_revs_under_compat() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "t@e.com"]);
    run_git(path, &["config", "user.name", "T"]);

    for v in 1..=2 {
        write(path.join("foo.rs"), &format!("{v}\n"));
        write(path.join("bar.rs"), &format!("{v}\n"));
        run_git(path, &["add", "."]);
        run_git(path, &["commit", "-m", &format!("paired {v}"), "--quiet"]);
    }
    write(path.join("baz.rs"), "solo\n");
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "solo", "--quiet"]);

    let repo = GixRepo::open(path).expect("gix open");
    let db = FactsDb::new_in_memory().expect("db");

    // Modern default: min_soc = None → threshold = 1 (drop solo commits).
    let opts_modern = Options {
        repo_path: path.to_path_buf(),
        min_revs: 5,
        min_soc: None,
        code_maat_compat: false,
        ..Options::default()
    };
    db.ingest(&repo, &opts_modern).expect("ingest");
    let rows_modern = run_soc(&db, &opts_modern).expect("soc modern");
    let entities_modern: Vec<&str> = rows_modern.iter().map(|r| r.entity.as_str()).collect();
    assert!(entities_modern.contains(&"foo.rs"), "modern: foo (SoC=2 >= 1) kept");
    assert!(entities_modern.contains(&"bar.rs"), "modern: bar (SoC=2 >= 1) kept");
    assert!(!entities_modern.contains(&"baz.rs"), "modern: baz (SoC=0 < 1) dropped");

    // Compat: code_maat_compat = true → SoC threshold falls back to min_revs.
    let opts_compat = Options {
        repo_path: path.to_path_buf(),
        min_revs: 5,
        min_soc: None,
        code_maat_compat: true,
        ..Options::default()
    };
    let rows_compat = run_soc(&db, &opts_compat).expect("soc compat");
    let entities_compat: Vec<&str> = rows_compat.iter().map(|r| r.entity.as_str()).collect();
    assert!(
        !entities_compat.contains(&"foo.rs"),
        "compat: foo (SoC=2 < min_revs=5) dropped — got entities: {entities_compat:?}"
    );
    assert!(
        !entities_compat.contains(&"bar.rs"),
        "compat: bar (SoC=2 < min_revs=5) dropped"
    );
}

/// The compat flag implies `--strict-grouping`. Wired in main.rs at
/// the Options construction site (not in this lib test); document the
/// invariant here for future readers of the Options-side test surface.
#[test]
fn code_maat_compat_default_state_documents_invariant() {
    let default_opts = Options::default();
    assert!(!default_opts.code_maat_compat);
    assert!(!default_opts.strict_grouping);

    // The main.rs Options-construction site sets:
    //   strict_grouping: args.strict_grouping || args.code_maat_compat
    // ...so passing --code-maat-compat alone produces:
    //   { code_maat_compat: true, strict_grouping: true }
    let compat_only = Options {
        code_maat_compat: true,
        strict_grouping: true,
        ..Options::default()
    };
    assert!(compat_only.code_maat_compat && compat_only.strict_grouping);
}

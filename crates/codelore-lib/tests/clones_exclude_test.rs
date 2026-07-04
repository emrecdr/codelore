//! --exclude + .codeloreignore for the `clones` analysis.

use codelore_lib::Options;
use codelore_lib::analyses::clones::run_clones;
use std::io::Write;

#[test]
fn exclude_pattern_drops_matching_clones() {
    let dir = tempfile::tempdir().unwrap();

    // Two functions in src/ — should pair as Type-2 clone family.
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    let a = dir.path().join("src/a.rs");
    let b = dir.path().join("src/b.rs");
    let mut fa = std::fs::File::create(&a).unwrap();
    writeln!(
        fa,
        "fn add(a: i32, b: i32) -> i32 {{ let x = 1; let y = 2; a + b + x + y }}"
    )
    .unwrap();
    let mut fb = std::fs::File::create(&b).unwrap();
    writeln!(
        fb,
        "fn mul(p: u64, q: u64) -> u64 {{ let s = 9; let t = 7; p + q + s + t }}"
    )
    .unwrap();

    // Sanity: without exclude, the pair shows up.
    let opts = Options {
        repo_path: dir.path().to_path_buf(),
        min_clone_node_count: 0,
        ..Options::default()
    };
    assert_eq!(
        run_clones(&opts).unwrap().len(),
        2,
        "baseline: 2 clone rows expected"
    );

    // With --exclude src/b.rs only a is seen → no clone family.
    let opts_excluded = Options {
        repo_path: dir.path().to_path_buf(),
        min_clone_node_count: 0,
        exclude_patterns: vec!["src/b.rs".to_string()],
        ..Options::default()
    };
    assert_eq!(
        run_clones(&opts_excluded).unwrap().len(),
        0,
        "excluding b.rs should kill the family"
    );
}

#[test]
fn codeloreignore_file_drops_matching_clones() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::create_dir_all(dir.path().join("vendor")).unwrap();

    // Put one copy in src/, one in vendor/.
    let a = dir.path().join("src/a.rs");
    let b = dir.path().join("vendor/b.rs");
    std::fs::write(
        &a,
        "fn add(a: i32, b: i32) -> i32 { let x = 1; let y = 2; a + b + x + y }\n",
    )
    .unwrap();
    std::fs::write(
        &b,
        "fn mul(p: u64, q: u64) -> u64 { let s = 9; let t = 7; p + q + s + t }\n",
    )
    .unwrap();

    // Sanity: without ignore file, both show up.
    let opts = Options {
        repo_path: dir.path().to_path_buf(),
        min_clone_node_count: 0,
        ..Options::default()
    };
    assert_eq!(
        run_clones(&opts).unwrap().len(),
        2,
        "baseline: both files contribute"
    );

    // Drop a .codeloreignore that excludes vendor/.
    std::fs::write(
        dir.path().join(".codeloreignore"),
        "# vendor code is not user-meaningful\nvendor/**\n",
    )
    .unwrap();

    assert_eq!(
        run_clones(&opts).unwrap().len(),
        0,
        ".codeloreignore should drop the vendor copy and kill the family"
    );
}

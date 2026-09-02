//! The working-tree clone scan accounts for what it could not read.
//!
//! This is the scan `analyze`, `gate`'s change-set projection and `diff`
//! consume. It used to drop an unreadable file with `.ok()?` — no message at
//! any level — so a scan that reached five of five thousand files returned
//! the same value as one that genuinely found no duplication. The HEAD-time
//! pass had carried coverage accounting since it was written; this one did
//! not, and it is the half the product actually runs.

use codelore_lib::Options;
use codelore_lib::analyses::clones::run_clones;
use std::io::Write;

/// An unreadable file must not take the rest of the scan down with it, and
/// the clone families in the files that COULD be read must still be found.
///
/// Unix-only because the check needs a file the process cannot open, and
/// `chmod 000` is the portable-enough way to get one; Windows permissions do
/// not deny the owner by mode bits. The Windows CI leg does not select this
/// binary in any case.
#[cfg(unix)]
#[test]
fn an_unreadable_file_is_survived_rather_than_silently_dropping_the_scan() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();

    // Two readable files that pair as a clone family.
    for name in ["a.rs", "b.rs"] {
        let mut f = std::fs::File::create(dir.path().join("src").join(name)).unwrap();
        writeln!(
            f,
            "fn add(a: i32, b: i32) -> i32 {{ let x = 1; let y = 2; a + b + x + y }}"
        )
        .unwrap();
    }

    // A third eligible file the scan cannot read.
    let denied = dir.path().join("src/denied.rs");
    let mut f = std::fs::File::create(&denied).unwrap();
    writeln!(f, "fn denied() -> i32 {{ 1 }}").unwrap();
    drop(f);
    std::fs::set_permissions(&denied, std::fs::Permissions::from_mode(0o000)).unwrap();

    let opts = Options {
        repo_path: dir.path().to_path_buf(),
        // Same threshold the sibling clone tests use: these one-line
        // functions sit under the default node-count floor, which would make
        // the assertion below fail for a reason unrelated to readability.
        min_clone_node_count: 0,
        ..Options::default()
    };
    let rows = run_clones(&opts).expect("an unreadable file must not fail the whole scan");

    // Restore before the tempdir teardown, which would otherwise fail to
    // remove a mode-000 file on some platforms.
    std::fs::set_permissions(&denied, std::fs::Permissions::from_mode(0o644)).unwrap();

    assert!(
        !rows.is_empty(),
        "the readable pair must still be reported as a clone family"
    );
}

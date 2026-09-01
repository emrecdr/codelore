//! Output emitters: CSV, SARIF, JSON, Markdown, Parquet,
//! `SQLite`. The `banner` module renders the
//! stderr pre-flight banner shown at the start of every analyze run.

pub mod banner;
pub mod csv;
pub mod gha;
pub mod html;
pub mod json;
pub mod markdown;
pub mod ndjson;
pub mod parquet;
pub mod sarif;
#[cfg(feature = "spa")]
pub mod spa;
pub mod sqlite;
#[cfg(feature = "spa")]
pub mod step_summary;
pub(crate) mod template;

use std::path::{Path, PathBuf};

/// Publish a file atomically: write the payload to a process-unique temp
/// sibling of `dest`, then rename it over `dest` only after `write` succeeds.
/// An interrupted or failing `write` leaves any previous `dest` untouched — the
/// half-written bytes live only in the temp file, which is removed on error.
/// This is the single output-write choke point every emitter routes through so
/// a `^C` (or a mid-write failure) can never truncate a good previous output.
///
/// Mirrors the fact-cache publish idiom (`facts::FactsDb` cache write): temp in
/// the destination's own directory so the rename is a same-filesystem metadata
/// operation (a cross-device rename would fail), a pid suffix to isolate
/// concurrent writers to the same destination, and an fsync before the rename.
///
/// `write` receives the temp path and must create and fully write the file at
/// it. The path it receives does not exist when `write` is called (any stale
/// temp from this pid's prior aborted run is cleared first), which is exactly
/// what a `DuckDB` `ATTACH ... (TYPE SQLITE)` requires; a streaming writer
/// (`File::create`) or a `COPY ... TO` names the path directly.
///
/// The error type is the caller's: `write` returns `Result<(), E>` and the
/// helper's own I/O failures surface through the same `E` via
/// `E: From<std::io::Error>` (satisfied by both `CodeLoreError` and
/// `anyhow::Error`).
///
/// # Errors
///
/// Propagates `write`'s error, or an I/O error from the rename step. The fsync
/// is best-effort (advisory durability): atomicity — the actual guarantee —
/// comes from write-to-temp + rename, so a filesystem that refuses the re-open
/// for fsync must not fail an otherwise-complete write.
pub fn atomic_publish<F, E>(dest: &Path, write: F) -> std::result::Result<(), E>
where
    F: FnOnce(&Path) -> std::result::Result<(), E>,
    E: From<std::io::Error>,
{
    // Temp sibling: `<dest>.tmp.<pid>` in dest's own directory.
    let mut tmp_os = dest.as_os_str().to_owned();
    tmp_os.push(format!(".tmp.{}", std::process::id()));
    let tmp = PathBuf::from(tmp_os);
    // Clear any leftover temp from a prior aborted run by THIS pid so `write`
    // (and, for SQLite, the ATTACH inside it) sees a fresh path.
    let _ = std::fs::remove_file(&tmp);

    // Write the payload. On failure, remove the partial temp file and surface
    // the caller's error; `dest` is never touched.
    if let Err(e) = write(&tmp) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }

    // Best-effort fsync so the bytes are durable before the rename publishes
    // them (see the durability note above).
    if let Ok(f) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&tmp)
    {
        let _ = f.sync_all();
    }

    // Publish. `rename` atomically replaces an existing destination on Unix;
    // on the platforms/filesystems that reject rename-over-existing, remove the
    // previous destination and retry once. The Unix happy path never removes,
    // so full atomicity is preserved there.
    let published = std::fs::rename(&tmp, dest).or_else(|first| {
        if dest.exists() {
            std::fs::remove_file(dest).and_then(|()| std::fs::rename(&tmp, dest))
        } else {
            Err(first)
        }
    });
    if let Err(e) = published {
        let _ = std::fs::remove_file(&tmp);
        return Err(E::from(std::io::Error::new(
            e.kind(),
            format!("atomically publish {}: {e}", dest.display()),
        )));
    }
    Ok(())
}

/// Map a `serde_json` write failure to a [`crate::CodeLoreError`], preserving
/// an underlying I/O failure's [`std::io::ErrorKind`].
///
/// The serde-based emitters (JSON, NDJSON, SARIF) serialise straight into the
/// output writer, so a failure is either a genuine serialisation fault or the
/// sink erroring mid-write — most importantly `BrokenPipe`, raised when a
/// reader closes the pipe early (`codelore … | head`). Stringifying the error
/// into an `Output` message would erase that kind and hide the broken pipe from
/// the CLI's central quiet-exit arm, so an I/O-category failure is rebuilt as
/// [`crate::CodeLoreError::Io`] carrying the same kind. Both variants share the
/// same exit code, so non-pipe failures are unaffected beyond their message;
/// only `BrokenPipe` gains the quiet-exit treatment. `context` labels genuine
/// serialisation faults (e.g. `"json"`, `"ndjson row"`, `"sarif"`).
pub(crate) fn serde_json_io_err(context: &str, e: &serde_json::Error) -> crate::CodeLoreError {
    match e.io_error_kind() {
        Some(kind) => crate::CodeLoreError::Io(std::io::Error::from(kind)),
        None => crate::CodeLoreError::Output(format!("{context}: {e}")),
    }
}

#[cfg(all(test, feature = "test-support"))]
mod atomic_publish_tests {
    use super::atomic_publish;
    use std::io::Write as _;

    /// Names any leftover `.tmp.<pid>` sibling in `dir` — the publish must
    /// never orphan one.
    fn temp_strays(dir: &std::path::Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .expect("readdir")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".tmp."))
            .collect()
    }

    #[test]
    fn success_replaces_previous_contents() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("out.bin");
        std::fs::write(&dest, b"old").expect("seed");

        atomic_publish::<_, std::io::Error>(&dest, |tmp| {
            let mut f = std::fs::File::create(tmp)?;
            f.write_all(b"new")
        })
        .expect("publish");

        assert_eq!(std::fs::read(&dest).expect("read"), b"new");
        assert!(temp_strays(dir.path()).is_empty(), "temp file orphaned");
    }

    #[test]
    fn failed_write_leaves_previous_output_intact() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("out.bin");
        std::fs::write(&dest, b"previous-good").expect("seed");

        let err = atomic_publish::<_, std::io::Error>(&dest, |tmp| {
            // Partially write, then fail — the destination must not change.
            let mut f = std::fs::File::create(tmp)?;
            f.write_all(b"half-written")?;
            Err(std::io::Error::other("boom"))
        })
        .expect_err("closure error must propagate");

        assert_eq!(err.to_string(), "boom");
        assert_eq!(
            std::fs::read(&dest).expect("read"),
            b"previous-good",
            "an interrupted write must not truncate the previous good output"
        );
        assert!(
            temp_strays(dir.path()).is_empty(),
            "temp file orphaned on failure"
        );
    }
}

//! Build script for `codelore-lib`.
//!
//! Currently exists only to add platform-specific linker libraries that
//! the transitive `libduckdb-sys` build doesn't request itself.
//!
//! ## Windows: link `Rstrtmgr.lib`
//!
//! Bundled DuckDB (>= ~1.10) calls Windows Restart Manager APIs
//! (`RmStartSession`, `RmEndSession`, `RmRegisterResources`, `RmGetList`)
//! from `duckdb::AdditionalLockInfo` to produce friendlier error messages
//! when the database file is held by another process. Those symbols live
//! in `Rstrtmgr.lib` and aren't part of the MSVC default link set.
//!
//! On Rust 1.89.0 the link happened to succeed (the MSVC toolchain
//! transitively picked the lib up); under Rust 1.96.0's tightened
//! Windows link defaults the four symbols come out as `LNK2019:
//! unresolved external` and the test binary fails to link. Requesting
//! the lib explicitly here fixes it on every Rust version without
//! affecting the build on macOS / Linux.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-lib=dylib=Rstrtmgr");
    }
}

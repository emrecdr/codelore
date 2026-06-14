//! Build script for `codelore-lib`.
//!
//! Does two things, both gated behind orthogonal conditions:
//!
//! 1. **Windows linker fix** (always-on): adds `Rstrtmgr.lib` to the
//!    link set so bundled `DuckDB`'s Restart Manager calls resolve on
//!    Rust 1.96+ MSVC toolchains. See the
//!    `windows_restart_manager_link` block below.
//!
//! 2. **`spa` feature fetch** (opt-in): when the `spa` Cargo feature is
//!    enabled, fetches the SHA-pinned versions of Apache `ECharts` and
//!    d3-hierarchy from the jsDelivr CDN, verifies their SHA-256
//!    hashes against the [`ASSETS`] table below, and writes them to
//!    `OUT_DIR/{name}` so `crates/codelore-lib/src/output/spa.rs` can
//!    `include_str!` them at compile time.
//!
//!    Subsequent builds skip the fetch when the cached file's SHA-256
//!    still matches the pin — the fetch happens at most once per
//!    `CodeLore` release. Without the `spa` feature this code path is
//!    inert and the crate builds without internet access.
//!
//! ## Windows: link `Rstrtmgr.lib`
//!
//! Bundled `DuckDB` (>= ~1.10) calls Windows Restart Manager APIs
//! (`RmStartSession`, `RmEndSession`, `RmRegisterResources`, `RmGetList`)
//! from `duckdb::AdditionalLockInfo` to produce friendlier error messages
//! when the database file is held by another process. Those symbols live
//! in `Rstrtmgr.lib` and aren't part of the MSVC default link set. On
//! Rust 1.89.0 the link happened to succeed (the MSVC toolchain
//! transitively picked the lib up); under Rust 1.96.0's tightened
//! Windows link defaults the four symbols come out as `LNK2019:
//! unresolved external` and the test binary fails to link. Requesting
//! the lib explicitly here fixes it on every Rust version without
//! affecting the build on macOS / Linux.
//!
//! ## Updating the pinned JS deps
//!
//! 1. Pick a new version. `ECharts` releases:
//!    <https://github.com/apache/echarts/releases>. d3-hierarchy:
//!    <https://github.com/d3/d3-hierarchy/releases>.
//! 2. Update the `version` and `url` fields in [`ASSETS`].
//! 3. Locally fetch the new minified file:
//!    `curl -fsSL <url> -o /tmp/new.js`
//!    `shasum -a 256 /tmp/new.js`
//! 4. Paste the new SHA-256 into the `sha256` field below.
//! 5. Rebuild — the next build re-fetches because the on-disk cache
//!    no longer matches the new pin.
//!
//! ## Why jsDelivr
//!
//! - npm-backed; the canonical CDN cited in Apache `ECharts`' own docs
//!   and the d3-hierarchy README.
//! - Serves immutable, version-pinned URLs (no `@latest` drift).
//! - HTTPS + SHA-256 pinning means the dep cannot change under us
//!   without the build failing loudly.

use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// One vendored JS dependency. The pinned SHA-256 is the
/// supply-chain control — if the CDN ever serves bytes that don't
/// match, the build fails loud rather than silently embedding
/// tampered code.
struct AssetPin {
    /// Filename under `OUT_DIR`. Matches the `include_str!` target.
    name: &'static str,
    /// Full version-pinned URL on the jsDelivr CDN.
    url: &'static str,
    /// Hex-encoded SHA-256 of the expected file bytes.
    sha256: &'static str,
}

const ASSETS: &[AssetPin] = &[
    AssetPin {
        name: "echarts.min.js",
        url: "https://cdn.jsdelivr.net/npm/echarts@6.1.0/dist/echarts.min.js",
        sha256: "b66b25aeb4df84e33199dc21694014d336d222cbd9deb0e5a7c14bd6aa0d0fd0",
    },
    AssetPin {
        name: "d3-hierarchy.min.js",
        url: "https://cdn.jsdelivr.net/npm/d3-hierarchy@3.1.2/dist/d3-hierarchy.min.js",
        sha256: "a8771380454be89ec5ffe9a6396ba7c247081e348ae740dc9cb9629abd4c0e43",
    },
    // Alpine.js — HTML-attribute reactivity layer for the SPA dashboard.
    // ~46 KB minified; the `cdn.min.js` build is what jsDelivr ships
    // with the IIFE wrapper safe for inlining via <script>{{...}}</script>.
    AssetPin {
        name: "alpine.min.js",
        url: "https://cdn.jsdelivr.net/npm/alpinejs@3.15.12/dist/cdn.min.js",
        sha256: "57b37d7cae9a27d965fdae4adcc844245dfdc407e655aee85dcfff3a08036a3f",
    },
    // Alpine persist plugin — wires Alpine reactive state to
    // localStorage so cross-widget filter selections survive page
    // refresh. Sub-1 KB. Must be loaded AFTER alpine.min.js (Alpine's
    // global hooks need to exist for the plugin to register).
    //
    // Note: the persist plugin's `dist/cdn.min.js` is byte-identical
    // between 3.15.8 and 3.15.12 (only the core received patches),
    // so the SHA below is unchanged from the prior pin. The URL is
    // bumped to keep the version label aligned with the core.
    AssetPin {
        name: "alpine-persist.min.js",
        url: "https://cdn.jsdelivr.net/npm/@alpinejs/persist@3.15.12/dist/cdn.min.js",
        sha256: "e77d932c52ce616c2a5c5dc45530a0221911a31aab48a179e8680932d4d3aa47",
    },
];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    windows_restart_manager_link();

    // The `spa` feature is the gate for the JS-dep fetch. Without it,
    // CodeLore builds offline with zero network access.
    if std::env::var_os("CARGO_FEATURE_SPA").is_some() {
        vendor_spa_assets();
    }
}

fn windows_restart_manager_link() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-lib=dylib=Rstrtmgr");
    }
}

fn vendor_spa_assets() {
    let out_dir =
        PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR set by cargo for build scripts"));

    for asset in ASSETS {
        let dest = out_dir.join(asset.name);
        match cached_and_valid(&dest, asset.sha256) {
            Ok(true) => {
                // Cached copy is byte-identical to the pin; skip the fetch.
                continue;
            }
            Ok(false) => {
                // Missing OR SHA drifted — refetch.
                if let Err(e) = fs::remove_file(&dest)
                    && e.kind() != std::io::ErrorKind::NotFound
                {
                    panic!(
                        "build.rs: failed to remove stale cached {}: {e}",
                        dest.display()
                    );
                }
            }
            Err(e) => {
                eprintln!(
                    "cargo:warning=codelore-lib build.rs: cached \
                     {} unreadable ({e}); refetching",
                    asset.name
                );
            }
        }

        fetch_and_pin(asset, &dest);
    }
}

/// `Ok(true)` if `dest` exists and its SHA-256 matches `expected`.
/// `Ok(false)` if `dest` is missing OR its SHA-256 doesn't match.
/// `Err(_)` if `dest` exists but cannot be read.
fn cached_and_valid(dest: &Path, expected: &str) -> std::io::Result<bool> {
    if !dest.exists() {
        return Ok(false);
    }
    let bytes = fs::read(dest)?;
    Ok(sha256_hex(&bytes) == expected)
}

fn fetch_and_pin(asset: &AssetPin, dest: &Path) {
    eprintln!(
        "cargo:warning=codelore-lib build.rs: fetching {} from jsDelivr (one-time per release)",
        asset.name
    );
    let body = match download(asset.url) {
        Ok(b) => b,
        Err(e) => {
            panic!(
                "codelore-lib build.rs: failed to fetch {} from {}: {e}\n\
                 \n\
                 The `spa` feature requires fetching pinned JS deps from \
                 jsDelivr at build time. If you're building offline or \
                 behind a proxy that blocks npm CDN access, build without \
                 the feature instead: `cargo build` (default features \
                 do NOT include `spa`).",
                asset.name, asset.url
            );
        }
    };

    let actual = sha256_hex(&body);
    assert!(
        actual == asset.sha256,
        "codelore-lib build.rs: SHA-256 mismatch for {} from {}\n  \
         expected: {}\n  \
         got:      {}\n\
         \n\
         jsDelivr served bytes that don't match the pin in build.rs. \
         Either the upstream package changed under us (unlikely for an \
         immutable npm version) or there's tampering on the CDN path. \
         Investigate before updating the pin.",
        asset.name,
        asset.url,
        asset.sha256,
        actual,
    );

    fs::write(dest, &body).unwrap_or_else(|e| {
        panic!(
            "codelore-lib build.rs: failed to write {} to OUT_DIR: {e}",
            dest.display()
        )
    });
}

fn download(url: &str) -> Result<Vec<u8>, String> {
    let resp = ureq::get(url)
        .timeout(std::time::Duration::from_mins(2))
        .call()
        .map_err(|e| format!("ureq call: {e}"))?;
    if resp.status() != 200 {
        return Err(format!("HTTP {} from CDN", resp.status()));
    }
    let mut buf = Vec::with_capacity(1_200_000);
    resp.into_reader()
        .read_to_end(&mut buf)
        .map_err(|e| format!("read body: {e}"))?;
    Ok(buf)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

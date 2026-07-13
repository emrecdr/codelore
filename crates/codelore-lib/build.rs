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
//! ## CDN strategy: jsDelivr primary, unpkg fallback
//!
//! - **jsDelivr** is the primary CDN — npm-backed, the canonical CDN
//!   cited in Apache `ECharts`' own docs and the d3-hierarchy README,
//!   and serves immutable, version-pinned URLs (no `@latest` drift).
//! - **unpkg** is the fallback for each asset. Both CDNs pull from the
//!   same npm registry, so the bytes are identical for the pinned
//!   versions and the same SHA-256 validates whichever mirror
//!   responds. A jsDelivr DNS outage / regional block / rate-limit
//!   no longer breaks every downstream `--features spa` build.
//! - HTTPS + SHA-256 pinning means the dep cannot change under us
//!   without the build failing loudly. A mismatch on ANY URL is a
//!   hard fail (not "skip to next mirror"), so a tampered mirror
//!   can't be silently replaced by the next mirror's clean bytes.

use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

/// One vendored JS dependency. The pinned SHA-256 is the
/// supply-chain control — if any CDN serves bytes that don't match,
/// the build fails loud rather than silently embedding tampered code.
///
/// `url_fallbacks` exists so a jsDelivr availability incident
/// (DNS outage, regional block, rate-limit) doesn't break every
/// downstream `codelore-lib --features spa` build. npm packages are
/// immutable across CDNs (both pull from the same registry), so the
/// same SHA validates whichever mirror responds.
struct AssetPin {
    /// Filename under `OUT_DIR`. Matches the `include_str!` target.
    name: &'static str,
    /// Primary version-pinned URL — tried first.
    url: &'static str,
    /// Mirror URLs tried in order if the primary fetch fails. Each
    /// must serve the SAME bytes (verified by the shared `sha256` pin
    /// — any mirror that drifts fails the build loudly).
    url_fallbacks: &'static [&'static str],
    /// Hex-encoded SHA-256 of the expected file bytes.
    sha256: &'static str,
}

const ASSETS: &[AssetPin] = &[
    AssetPin {
        name: "echarts.min.js",
        url: "https://cdn.jsdelivr.net/npm/echarts@6.1.0/dist/echarts.min.js",
        url_fallbacks: &["https://unpkg.com/echarts@6.1.0/dist/echarts.min.js"],
        sha256: "b66b25aeb4df84e33199dc21694014d336d222cbd9deb0e5a7c14bd6aa0d0fd0",
    },
    AssetPin {
        name: "d3-hierarchy.min.js",
        url: "https://cdn.jsdelivr.net/npm/d3-hierarchy@3.1.2/dist/d3-hierarchy.min.js",
        url_fallbacks: &["https://unpkg.com/d3-hierarchy@3.1.2/dist/d3-hierarchy.min.js"],
        sha256: "a8771380454be89ec5ffe9a6396ba7c247081e348ae740dc9cb9629abd4c0e43",
    },
    // Alpine.js — HTML-attribute reactivity layer for the SPA dashboard.
    // ~46 KB minified; the `cdn.min.js` build is what jsDelivr ships
    // with the IIFE wrapper safe for inlining via <script>{{...}}</script>.
    AssetPin {
        name: "alpine.min.js",
        url: "https://cdn.jsdelivr.net/npm/alpinejs@3.15.12/dist/cdn.min.js",
        url_fallbacks: &["https://unpkg.com/alpinejs@3.15.12/dist/cdn.min.js"],
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
        url_fallbacks: &["https://unpkg.com/@alpinejs/persist@3.15.12/dist/cdn.min.js"],
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
    // Walk primary URL first, then each fallback in declaration order.
    // SHA verification is shared — npm packages are immutable across
    // CDNs, so the same SHA validates whichever mirror responds. A
    // mismatch on ANY URL is a hard fail (not "skip to next mirror"),
    // because we don't want one tampered mirror's bytes silently
    // replaced by the next mirror's clean bytes; the SHA-pin
    // discipline catches drift everywhere it happens.
    eprintln!(
        "cargo:warning=codelore-lib build.rs: fetching {} (one-time per release)",
        asset.name
    );
    let mut errors: Vec<String> = Vec::new();
    let mut body: Option<Vec<u8>> = None;
    for (idx, url) in std::iter::once(asset.url)
        .chain(asset.url_fallbacks.iter().copied())
        .enumerate()
    {
        if idx > 0 {
            eprintln!(
                "cargo:warning=codelore-lib build.rs: primary CDN failed for {}, trying mirror {}",
                asset.name, url
            );
        }
        match download(url) {
            Ok(b) => {
                body = Some(b);
                break;
            }
            Err(e) => errors.push(format!("  {url}: {e}")),
        }
    }
    let body = body.unwrap_or_else(|| {
        panic!(
            "codelore-lib build.rs: failed to fetch {} from any of {} URL(s):\n{}\n\
             \n\
             The `spa` feature requires fetching pinned JS deps at build \
             time. If you're building offline or behind a proxy that \
             blocks every configured CDN, build without the feature \
             instead: `cargo build` (default features do NOT include \
             `spa`).",
            asset.name,
            errors.len(),
            errors.join("\n")
        );
    });

    let actual = sha256_hex(&body);
    assert!(
        actual == asset.sha256,
        "codelore-lib build.rs: SHA-256 mismatch for {}\n  \
         expected: {}\n  \
         got:      {}\n\
         \n\
         A CDN served bytes that don't match the pin in build.rs. \
         Either the upstream npm package changed under us (unlikely \
         for an immutable version) or there's tampering on the CDN \
         path. Investigate before updating the pin.",
        asset.name,
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
    // ureq 3 surface notes:
    // - Per-request timeout lives under `.config().timeout_global(...)`
    //   (ureq 2 had a top-level `.timeout(...)` method; gone in v3).
    // - Non-2xx responses are returned as `Err(Error::StatusCode(code))`
    //   by default, so the explicit `resp.status() != 200` check the
    //   v2 code did is now redundant — `.call()?` covers it.
    // - Body consumption: ureq 2's `into_reader().read_to_end(&mut buf)`
    //   chain replaced by `body_mut().with_config().limit(N).read_to_vec()`.
    //   The default 10 MB limit would actually cover every CDN asset we
    //   fetch today (ECharts ≈ 1.1 MB, others all sub-100 KB), but we
    //   pin an explicit 8 MB limit anyway so an upstream CDN-side
    //   compromise that swaps one of our pinned assets for a giant
    //   payload fails fast instead of consuming heap unbounded — the
    //   SHA-256 check downstream would still catch the substitution,
    //   but the limit makes the failure cheaper.
    let mut resp = ureq::get(url)
        .config()
        .timeout_global(Some(std::time::Duration::from_mins(2)))
        .build()
        .call()
        .map_err(|e| format!("ureq call: {e}"))?;
    let buf = resp
        .body_mut()
        .with_config()
        .limit(8 * 1024 * 1024)
        .read_to_vec()
        .map_err(|e| format!("read body: {e}"))?;
    Ok(buf)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

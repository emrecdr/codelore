#!/usr/bin/env bash
# vendor-duckdb-rs.sh — idempotently vendor a patched libduckdb-sys
#
# Workaround for duckdb/duckdb-rs#786: MSVC 19.40 removed
# stdext::checked_array_iterator, breaking bundled DuckDB builds on
# Windows. The upstream patch hasn't shipped in a release yet, so we
# vendor a patched copy locally.
#
# This script is idempotent: re-running on an already-vendored tree is
# a no-op. Safe to call from `just`, the workflows, or by hand.
#
# Remove this script (and the [patch.crates-io] entry in Cargo.toml)
# once duckdb/duckdb-rs ships a release with the fix merged.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VENDOR_DIR="${REPO_ROOT}/vendor/duckdb-rs"
DUCKDB_RS_TAG="v1.10503.1"
PATCH_FILE="${REPO_ROOT}/patches/duckdb-rs-msvc-1940.patch"
STAMP="${VENDOR_DIR}/.patched-${DUCKDB_RS_TAG}"

if [[ -f "${STAMP}" ]]; then
    echo "[vendor-duckdb-rs] already vendored at ${DUCKDB_RS_TAG} — skipping"
    exit 0
fi

# Clean any previous attempt so we start from a known state.
rm -rf "${VENDOR_DIR}"
mkdir -p "$(dirname "${VENDOR_DIR}")"

echo "[vendor-duckdb-rs] cloning duckdb/duckdb-rs at ${DUCKDB_RS_TAG}..."
git clone --depth=1 --branch "${DUCKDB_RS_TAG}" \
    https://github.com/duckdb/duckdb-rs.git \
    "${VENDOR_DIR}" 2>&1 | sed 's/^/[vendor-duckdb-rs]   /'

echo "[vendor-duckdb-rs] applying patches/duckdb-rs-msvc-1940.patch..."
(cd "${VENDOR_DIR}" && git apply "${PATCH_FILE}")

# Strip the nested `.git/` directory after the patch is applied so the
# vendored tree isn't a sub-repository. Otherwise `git add` in the host
# workspace treats `vendor/duckdb-rs/` as a potential submodule and
# refuses to track the manifest stub paths that Dependabot needs (see
# the `.gitignore` block on `vendor/duckdb-rs/crates/libduckdb-sys/` for
# the wider context). The clone + patch + flatten still produces a
# deterministic working tree without the nested repo.
rm -rf "${VENDOR_DIR}/.git"

# When cargo's `[patch.crates-io]` brings the vendored libduckdb-sys into
# our workspace, its `Cargo.toml` resolves `authors / homepage / readme /
# keywords / version / license / repository / edition / rust-version` via
# `{ workspace = true }` — against OUR workspace.package, not duckdb-rs's.
# We don't carry every field duckdb-rs does (no authors / homepage /
# readme), so cargo would fail with `'workspace.package.authors' was not
# defined`. Flatten the vendored Cargo.toml to use literal values from
# duckdb-rs's workspace.package, so the manifest is self-contained and
# doesn't pull anything from our workspace.
echo "[vendor-duckdb-rs] flattening vendored libduckdb-sys manifest..."
LIBDUCKDB_MANIFEST="${VENDOR_DIR}/crates/libduckdb-sys/Cargo.toml"
python3 - "${LIBDUCKDB_MANIFEST}" "${VENDOR_DIR}/Cargo.toml" <<'PY'
import re, sys, tomllib

mf, ws = sys.argv[1], sys.argv[2]
with open(ws, "rb") as f:
    workspace = tomllib.load(f)["workspace"]
pkg = workspace.get("package", {})
ws_deps = workspace.get("dependencies", {})
src = open(mf).read()


def render_value(v):
    if isinstance(v, list):
        return "[" + ", ".join(f'"{x}"' for x in v) + "]"
    if isinstance(v, dict):
        # Inline-table form: { key = value, ... }
        pairs = []
        for k, val in v.items():
            pairs.append(f'{k} = {render_value(val)}')
        return "{ " + ", ".join(pairs) + " }"
    if isinstance(v, bool):
        return "true" if v else "false"
    return f'"{v}"'


def flatten_package_field(field, value):
    global src
    pat = rf'^{field}\s*=\s*\{{\s*workspace\s*=\s*true\s*\}}\s*$'
    src = re.sub(pat, f'{field} = {render_value(value)}', src, count=1, flags=re.M)


for field in ("version", "authors", "license", "repository", "homepage",
              "keywords", "readme", "edition", "rust-version"):
    if field in pkg:
        flatten_package_field(field, pkg[field])

# Now flatten workspace-inherited dependency entries. The pattern is:
#   bindgen = { workspace = true, features = [...], optional = true }
# becomes:
#   bindgen = { version = "0.72.1", features = [...], optional = true,
#               <plus any default-features from the workspace spec> }
def flatten_dependency_line(match):
    dep_name = match.group(1)
    extras = match.group(2)  # ", features = [...], optional = true" etc
    if dep_name not in ws_deps:
        return match.group(0)
    spec = ws_deps[dep_name]
    if isinstance(spec, str):
        ws_fields = {"version": spec}
    else:
        ws_fields = dict(spec)
        ws_fields.pop("path", None)
    parts = [f'{k} = {render_value(v)}' for k, v in ws_fields.items()]
    if extras:
        parts.append(extras.lstrip(", ").rstrip())
    return f'{dep_name} = {{ {", ".join(parts)} }}'


# Single-line form only — the libduckdb-sys manifest uses single-line
# inline-tables for its workspace-inherited deps.
src = re.sub(
    r'^(\S+)\s*=\s*\{\s*workspace\s*=\s*true\s*(,\s*[^}]+)?\}\s*$',
    flatten_dependency_line,
    src,
    flags=re.M,
)

open(mf, "w").write(src)
PY

# Stamp the directory so subsequent runs skip cleanly.
touch "${STAMP}"

echo "[vendor-duckdb-rs] ✓ vendored + patched libduckdb-sys at vendor/duckdb-rs/"

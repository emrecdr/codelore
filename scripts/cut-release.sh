#!/usr/bin/env bash
#
# cut-release.sh — codelore release-cut helper
# =============================================
#
# Automates the post-v0.1.2 lesson: GitHub repository rulesets evaluating
# `required_status_checks` against tag pushes don't reliably see GitHub
# Actions Check Runs (only the legacy Commit Status API). So cutting a
# tagged release requires a "disable ruleset → push tag → restore ruleset"
# dance. This script wraps that dance together with the pre-release prep
# (version bump, CHANGELOG flip, cargo update, commit, CI gate).
#
# USAGE
# -----
#   scripts/cut-release.sh X.Y.Z [--dry-run] [--skip-ci-wait]
#
# Examples:
#   scripts/cut-release.sh 0.1.3                 # full release cut
#   scripts/cut-release.sh 0.2.0 --dry-run       # show what would happen
#   scripts/cut-release.sh 0.1.3 --skip-ci-wait  # tag immediately (CI already verified)
#
# PRECONDITIONS (script will refuse to proceed unless ALL are met)
# ----------------------------------------------------------------
#   1. Working tree is clean (no unstaged or uncommitted changes)
#   2. Current branch is `main`
#   3. Local `main` is in sync with `origin/main` (no unpushed commits)
#   4. Tag `vX.Y.Z` does not already exist locally OR on origin
#   5. Version `X.Y.Z` parses as a semver triple (digits-only segments)
#   6. CHANGELOG.md has a `## [Unreleased]` section with non-empty content
#   7. `gh` CLI authenticated with sufficient scope to PUT rulesets
#
# SAFETY
# ------
#   - The ruleset restore is registered via `trap EXIT` so the ruleset
#     is ALWAYS re-enabled before the script exits — even on ^C, even on
#     an unexpected internal failure, even if `git push` rejects the tag.
#   - All git/gh state-mutating calls are logged before execution.
#   - `--dry-run` performs all preconditions + prints the exact actions
#     without executing any state-mutating step.
#
# WHY THIS EXISTS
# ---------------
#   Before this script, cutting a release required a 12-step manual
#   sequence and the v0.1.2 cut left the ruleset disabled for several
#   seconds while running ad-hoc `gh api` calls. A scripted release path
#   eliminates the "did I remember to restore the ruleset?" anxiety and
#   makes the procedure auditable in `git log`.

set -euo pipefail

# ──────────────────────────────────────────────────────────────────────
# Configuration
# ──────────────────────────────────────────────────────────────────────
REPO="emrecdr/codelore"
RULESET_ID=17437461                  # protect-release-tags
GH_ACTIONS_APP_ID=15368              # for required_status_checks integration_id

# ──────────────────────────────────────────────────────────────────────
# Argument parsing
# ──────────────────────────────────────────────────────────────────────
if [[ $# -lt 1 ]]; then
  echo "usage: $0 X.Y.Z [--dry-run] [--skip-ci-wait]" >&2
  exit 2
fi
VERSION="$1"
shift

DRY_RUN=false
SKIP_CI_WAIT=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run)        DRY_RUN=true ;;
    --skip-ci-wait)   SKIP_CI_WAIT=true ;;
    *) echo "unknown flag: $1" >&2; exit 2 ;;
  esac
  shift
done

TAG="v${VERSION}"

# ──────────────────────────────────────────────────────────────────────
# Logging helpers
# ──────────────────────────────────────────────────────────────────────
if [[ -t 2 ]]; then
  RED=$'\033[0;31m';   GREEN=$'\033[0;32m'
  YELLOW=$'\033[1;33m'; BLUE=$'\033[0;34m'
  DIM=$'\033[2m';      NC=$'\033[0m'
else
  RED=''; GREEN=''; YELLOW=''; BLUE=''; DIM=''; NC=''
fi

log()  { printf "${BLUE}[cut-release]${NC} %s\n" "$*" >&2; }
ok()   { printf "${BLUE}[cut-release]${NC} ${GREEN}✓${NC} %s\n" "$*" >&2; }
warn() { printf "${BLUE}[cut-release]${NC} ${YELLOW}⚠${NC} %s\n" "$*" >&2; }
die()  { printf "${BLUE}[cut-release]${NC} ${RED}✗${NC} %s\n" "$*" >&2; exit 1; }
run()  {
  if [[ "${DRY_RUN}" == "true" ]]; then
    printf "${DIM}[dry-run] would run:${NC} %s\n" "$*" >&2
  else
    log "running: $*"
    "$@"
  fi
}

# ──────────────────────────────────────────────────────────────────────
# Ruleset restore (registered via trap EXIT — runs on ANY exit path)
# ──────────────────────────────────────────────────────────────────────
# Restore-call timeout: prevents an indefinite hang if `gh api` stalls
# on a slow / rate-limited GitHub response while the trap is running.
# Without the cap, a Ctrl-C while gh is hung gives the operator no
# second chance to clean up and the ruleset stays disabled silently.
# 30s is generous for a single PUT (typical: <2s); on timeout we fall
# through to the manual-recovery branch so the operator gets a
# paste-able recovery command. Uses `gtimeout` on macOS (coreutils)
# when present, falling back to GNU `timeout` on Linux.
TIMEOUT_BIN="$(command -v gtimeout || command -v timeout || true)"

RULESET_DISABLED=false
restore_ruleset() {
  local exit_code=$?
  if [[ "${RULESET_DISABLED}" == "true" ]]; then
    warn "restoring protect-release-tags ruleset to active enforcement..."
    # Use a fresh PUT so we don't depend on a captured backup body — the
    # ruleset shape is canonical and lives here in this script.
    if ${TIMEOUT_BIN:+${TIMEOUT_BIN} 30s} gh api -X PUT "repos/${REPO}/rulesets/${RULESET_ID}" --input - >/dev/null <<RULESET
{
  "name": "protect-release-tags",
  "target": "tag",
  "enforcement": "active",
  "conditions": {
    "ref_name": {
      "include": ["refs/tags/v*"],
      "exclude": []
    }
  },
  "rules": [
    { "type": "deletion" },
    { "type": "non_fast_forward" },
    {
      "type": "required_status_checks",
      "parameters": {
        "strict_required_status_checks_policy": false,
        "required_status_checks": [
          { "context": "rustfmt", "integration_id": ${GH_ACTIONS_APP_ID} },
          { "context": "clippy", "integration_id": ${GH_ACTIONS_APP_ID} },
          { "context": "cargo-deny", "integration_id": ${GH_ACTIONS_APP_ID} },
          { "context": "test (ubuntu-latest)", "integration_id": ${GH_ACTIONS_APP_ID} },
          { "context": "test (macos-latest)", "integration_id": ${GH_ACTIONS_APP_ID} },
          { "context": "test (windows-latest)", "integration_id": ${GH_ACTIONS_APP_ID} }
        ]
      }
    }
  ]
}
RULESET
    then
      ok "ruleset restored to active enforcement"
    else
      warn "ruleset restore FAILED (or timed out after 30s) — run manually:"
      warn "  gh api repos/${REPO}/rulesets/${RULESET_ID}  # inspect current state"
      warn "  then PUT with the canonical config (see the heredoc above in this script)"
      warn "If the ruleset shows enforcement=disabled, the repo is unprotected"
      warn "for tag pushes until restored — restore as soon as possible."
    fi
  fi
  exit "${exit_code}"
}
trap restore_ruleset EXIT

# ──────────────────────────────────────────────────────────────────────
# Preconditions
# ──────────────────────────────────────────────────────────────────────
log "validating preconditions for ${TAG} release..."

if [[ ! "${VERSION}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  die "version '${VERSION}' does not match X.Y.Z (digits only). Pre-release suffixes need a different script."
fi
ok "version '${VERSION}' parses as semver"

# Check only TRACKED-file modifications. Untracked files (IDE state,
# agent-local caches, etc.) don't get added to the release commit
# because the script's `git add` later is explicit
# (`Cargo.toml Cargo.lock CHANGELOG.md`) — they can't accidentally
# slip into the tag's tree. Caring about them here would force devs
# to maintain a perfectly-tidy worktree just to cut a release,
# which adds friction without safety value.
if [[ -n "$(git status --porcelain --untracked-files=no)" ]]; then
  git status --short --untracked-files=no >&2
  die "working tree has uncommitted changes to tracked files. Commit or stash first."
fi
ok "no uncommitted changes to tracked files"
# Surface untracked files as a note so the user knows what's NOT in the
# release. Silent skipping would hide IDE state they might want to
# commit before the release.
if [[ -n "$(git status --porcelain --untracked-files=all 2>/dev/null | grep '^??')" ]]; then
  warn "untracked files present (will NOT be in the release):"
  git status --short --untracked-files=all 2>/dev/null | grep '^??' | head -10 >&2
fi

CURRENT_BRANCH="$(git symbolic-ref --short HEAD 2>/dev/null || echo '')"
if [[ "${CURRENT_BRANCH}" != "main" ]]; then
  die "not on main (currently on '${CURRENT_BRANCH}'). Switch to main first."
fi
ok "on branch main"

git fetch origin main --quiet
LOCAL="$(git rev-parse main)"
REMOTE="$(git rev-parse origin/main)"
if [[ "${LOCAL}" != "${REMOTE}" ]]; then
  die "local main (${LOCAL:0:7}) is out of sync with origin/main (${REMOTE:0:7}). Pull or push first."
fi
ok "main is in sync with origin/main at ${LOCAL:0:7}"

if git rev-parse "refs/tags/${TAG}" >/dev/null 2>&1; then
  die "tag ${TAG} already exists locally. Delete it first with: git tag -d ${TAG}"
fi
if git ls-remote --tags --exit-code origin "refs/tags/${TAG}" >/dev/null 2>&1; then
  die "tag ${TAG} already exists on origin. This version was already released."
fi
ok "tag ${TAG} does not exist yet"

if ! command -v gh >/dev/null; then die "gh CLI not on PATH"; fi
if ! gh auth status >/dev/null 2>&1; then die "gh CLI not authenticated. Run 'gh auth login'."; fi
ok "gh CLI authenticated"

# CHANGELOG must have a non-empty [Unreleased] section to flip into versioned.
# Empty Unreleased = nothing to release = probably a mistake.
UNRELEASED_BODY="$(awk '/^## \[Unreleased\]/{f=1;next} /^## \[[0-9]/{f=0} f' CHANGELOG.md | sed '/^$/d')"
if [[ -z "${UNRELEASED_BODY}" ]]; then
  die "CHANGELOG.md [Unreleased] section is empty. Add release notes before cutting."
fi
ok "CHANGELOG.md [Unreleased] section has content"

# ──────────────────────────────────────────────────────────────────────
# Pre-release prep on a new commit
# ──────────────────────────────────────────────────────────────────────
log "preparing release commit for ${TAG}..."

if [[ "${DRY_RUN}" != "true" ]]; then
  # Bump workspace.package.version. Anchored to the [workspace.package]
  # section header to avoid editing any [package] or dependency version
  # field that happens to contain the previous version literal.
  python3 - "${VERSION}" Cargo.toml <<'PY'
import re, sys
new_version, path = sys.argv[1], sys.argv[2]
src = open(path).read()
out = re.sub(
    r'(\[workspace\.package\][^\[]*?\nversion = ")[0-9]+\.[0-9]+\.[0-9]+(")',
    rf'\g<1>{new_version}\g<2>',
    src,
    count=1,
    flags=re.DOTALL,
)
if out == src:
    sys.exit(f"failed to find workspace.package.version in {path}")
open(path, "w").write(out)
PY
  ok "bumped Cargo.toml workspace version → ${VERSION}"

  # Bump internal workspace path-deps in lock-step. Cargo requires the
  # `version = "..."` constraint on a path dep to be satisfied by the
  # target crate's published version — when the workspace bumps from
  # 0.4.X to 0.5.0, internal deps still pinned at "0.4.0" no longer
  # match and `cargo update` (next step) fails with:
  #   error: failed to select a version for the requirement
  #   `codelore-rca = "^0.4.0"` … candidate versions found which
  #   didn't match: 0.5.0
  # Match any line of the shape:
  #   codelore-foo = { path = "...", version = "X.Y.Z", ... }
  # inside any crates/*/Cargo.toml, and rewrite the version literal.
  python3 - "${VERSION}" <<'PY'
import re, sys, glob
new_version = sys.argv[1]
pat = re.compile(
    r'(codelore-[a-z]+\s*=\s*\{\s*path\s*=\s*"[^"]+"\s*,\s*version\s*=\s*")[0-9]+\.[0-9]+\.[0-9]+(")'
)
for path in glob.glob("crates/*/Cargo.toml"):
    src = open(path).read()
    out = pat.sub(rf'\g<1>{new_version}\g<2>', src)
    if out != src:
        open(path, "w").write(out)
        print(f"  updated internal path-deps in {path}")
PY
  ok "bumped internal codelore-* path-dep versions → ${VERSION}"

  # Flip CHANGELOG ## [Unreleased] → ## [X.Y.Z] - YYYY-MM-DD, then prepend
  # a fresh empty [Unreleased] section above it so future commits have a
  # bucket to land in.
  TODAY="$(date -u +%Y-%m-%d)"
  python3 - "${VERSION}" "${TODAY}" CHANGELOG.md <<'PY'
import sys
version, today, path = sys.argv[1], sys.argv[2], sys.argv[3]
src = open(path).read()
needle = "## [Unreleased]\n"
if needle not in src:
    sys.exit(f"could not find '{needle.strip()}' in {path}")
replacement = f"## [Unreleased]\n\n## [{version}] - {today}\n"
out = src.replace(needle, replacement, 1)
open(path, "w").write(out)
PY
  ok "flipped CHANGELOG [Unreleased] → [${VERSION}] - ${TODAY}"

  # Sync the lockfile so cargo update doesn't churn it later.
  cargo update -p codelore-lib -p codelore-cli -p codelore-rca --quiet
  ok "Cargo.lock synced to workspace ${VERSION}"
else
  log "[dry-run] would: bump Cargo.toml + flip CHANGELOG + cargo update"
fi

# Sanity build (catches CHANGELOG flip / Cargo.toml typos before commit)
log "running local gate (matching CI's exact invocation)..."
# v0.1.3 cut failed because the script's narrower local gate
# (`cargo build --release -p codelore-cli`) didn't run clippy with the
# same flags CI uses, so a `clippy::useless_conversion` lint surfaced
# in code that local was happy with. Now we run the EXACT CI clippy
# command so the local gate is at least as strict as CI's. Tests stay
# narrow (release builds dominate the time budget here; tests already
# ran during the dev cycle that landed [Unreleased]).
run cargo clippy --workspace --all-targets --all-features -- -D warnings
run cargo fmt --all --check
run cargo build --release --quiet -p codelore-cli
if [[ "${DRY_RUN}" != "true" ]]; then
  ACTUAL_VERSION="$(./target/release/codelore --version | awk '{print $2}')"
  if [[ "${ACTUAL_VERSION}" != "${VERSION}" ]]; then
    die "binary reports ${ACTUAL_VERSION}, expected ${VERSION}. Something went wrong."
  fi
  ok "binary reports codelore ${VERSION}"
fi

run git add Cargo.toml Cargo.lock CHANGELOG.md crates/*/Cargo.toml
run git commit -m "chore(release): ${TAG}"
ok "release commit created"

# ──────────────────────────────────────────────────────────────────────
# Push release commit + wait for CI green
# ──────────────────────────────────────────────────────────────────────
run git push origin main
ok "release commit pushed to origin"

if [[ "${SKIP_CI_WAIT}" == "true" ]]; then
  warn "--skip-ci-wait passed — NOT waiting for CI to confirm green"
  warn "  the tag push will fail if CI hasn't passed on this commit"
else
  log "waiting for CI to go green on the release commit (typical: 8–12 min)..."
  if [[ "${DRY_RUN}" != "true" ]]; then
    # Find the most recent CI run on main triggered by THIS push.
    sleep 5  # give GitHub a moment to register the run
    RELEASE_SHA="$(git rev-parse HEAD)"
    RUN_ID="$(gh run list --limit 5 --branch main --workflow CI --json databaseId,headSha \
              --jq ".[] | select(.headSha == \"${RELEASE_SHA}\") | .databaseId" | head -1)"
    if [[ -z "${RUN_ID}" ]]; then
      # README-only or other paths-ignored commit. Manually dispatch CI so
      # the ruleset has the required status checks on this commit.
      warn "no CI run auto-triggered for ${RELEASE_SHA:0:7} (paths-ignore likely matched)."
      warn "dispatching CI manually via workflow_dispatch..."
      gh workflow run CI --ref main
      sleep 5
      RUN_ID="$(gh run list --limit 3 --branch main --workflow CI --json databaseId,event,headSha \
                --jq '.[] | select(.event == "workflow_dispatch") | .databaseId' | head -1)"
    fi
    if [[ -z "${RUN_ID}" ]]; then
      die "could not locate a CI run for ${RELEASE_SHA:0:7}. Check 'gh run list' manually."
    fi
    log "watching CI run ${RUN_ID}..."
    # `gh run watch --exit-status` returns 0 for both "success" AND
    # "cancelled" runs (per gh CLI source — anything that isn't "failure"
    # exits 0). We've been bitten by that conflation during v0.1.2/v0.1.3
    # cuts where a concurrency-cancelled prior run was misread as green
    # CI. Watch the run to block on completion, then verify the actual
    # conclusion via the API. The script ONLY proceeds to the tag dance
    # if conclusion == "success" — anything else (failure, cancelled,
    # timed_out, action_required, neutral, skipped) aborts.
    gh run watch "${RUN_ID}" >/dev/null || true
    CONCLUSION="$(gh run view "${RUN_ID}" --json conclusion --jq .conclusion)"
    if [[ "${CONCLUSION}" != "success" ]]; then
      die "CI conclusion was '${CONCLUSION}' (not 'success') on the release commit ${RELEASE_SHA:0:7}. Investigate via 'gh run view ${RUN_ID}' and re-run the script with --skip-ci-wait once green, or fix and re-cut."
    fi
    ok "CI green (conclusion=success) on ${RELEASE_SHA:0:7}"
  fi
fi

# ──────────────────────────────────────────────────────────────────────
# Tag dance: disable ruleset → tag → push → ruleset restored by trap
# ──────────────────────────────────────────────────────────────────────
log "disabling protect-release-tags ruleset for tag push..."
if [[ "${DRY_RUN}" != "true" ]]; then
  gh api -X PUT "repos/${REPO}/rulesets/${RULESET_ID}" --input - >/dev/null <<RULESET
{
  "name": "protect-release-tags",
  "target": "tag",
  "enforcement": "disabled",
  "conditions": { "ref_name": { "include": ["refs/tags/v*"], "exclude": [] } },
  "rules": [
    { "type": "deletion" },
    { "type": "non_fast_forward" }
  ]
}
RULESET
  RULESET_DISABLED=true
  ok "ruleset disabled — tag push window OPEN"
else
  log "[dry-run] would: PUT ruleset with enforcement=disabled"
fi

run git tag -a "${TAG}" -m "${TAG}

See CHANGELOG.md [${VERSION}] section for the full list of changes."
ok "annotated tag ${TAG} created locally"

run git push origin "${TAG}"
ok "tag ${TAG} pushed to origin"

# The trap will re-enable the ruleset on exit. Explicit ack for clarity:
log "tag push complete — ruleset will be re-enabled on script exit"

# ──────────────────────────────────────────────────────────────────────
# Watch the release workflow
# ──────────────────────────────────────────────────────────────────────
if [[ "${DRY_RUN}" != "true" ]]; then
  sleep 5  # let GitHub register the tag-triggered workflows
  log "release workflows now running on ${TAG}:"
  gh run list --limit 3 --branch "${TAG}" \
    --json databaseId,name,status --jq '.[] | "  \(.databaseId)  \(.name) — \(.status)"' >&2 || true
  ok "release.yml will produce 5 binary archives + SLSA L3 attestation"
  ok "homebrew-publish will regenerate Formula/codelore.rb on the tap"
  ok "container.yml will publish ghcr.io/${REPO}:${TAG}"
  log ""
  log "monitor progress: gh run list --branch ${TAG}"
  log "release page:     https://github.com/${REPO}/releases/tag/${TAG}"
fi

ok "${TAG} release cut complete"

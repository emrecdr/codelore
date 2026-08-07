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
RULESET_ID=17437461                  # protect-release-tags (gates the tag push)
MAIN_RULESET_ID=17437460             # protect-main (gates the release-commit push)
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

# Canonical protect-main ruleset body — PUT it with $1 as the enforcement value
# ("active" | "disabled"). Hardcoded (like protect-release-tags below) so a
# restore never depends on a captured backup body; verified against the live
# ruleset at cut time. Keep the 9 contexts in sync if protect-main changes.
main_ruleset_put() {
  ${TIMEOUT_BIN:+${TIMEOUT_BIN} 30s} gh api -X PUT "repos/${REPO}/rulesets/${MAIN_RULESET_ID}" --input - >/dev/null <<MAINRULESET
{
  "name": "protect-main",
  "target": "branch",
  "enforcement": "$1",
  "conditions": { "ref_name": { "include": ["~DEFAULT_BRANCH"], "exclude": [] } },
  "rules": [
    { "type": "deletion" },
    { "type": "non_fast_forward" },
    { "type": "required_linear_history" },
    {
      "type": "required_status_checks",
      "parameters": {
        "do_not_enforce_on_create": false,
        "strict_required_status_checks_policy": false,
        "required_status_checks": [
          { "context": "cargo-deny" },
          { "context": "clippy" },
          { "context": "dogfood" },
          { "context": "rustfmt" },
          { "context": "self-gate" },
          { "context": "spa-browser" },
          { "context": "test (macos-latest)" },
          { "context": "test (ubuntu-latest)" },
          { "context": "test (windows-latest)" }
        ]
      }
    }
  ]
}
MAINRULESET
}

RULESET_DISABLED=false
MAIN_RULESET_DISABLED=false
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
  if [[ "${MAIN_RULESET_DISABLED}" == "true" ]]; then
    warn "restoring protect-main ruleset to active enforcement..."
    if main_ruleset_put active; then
      ok "protect-main ruleset restored to active enforcement"
    else
      warn "protect-main restore FAILED (or timed out) — run manually:"
      warn "  gh api repos/${REPO}/rulesets/${MAIN_RULESET_ID}  # inspect current state"
      warn "  If it shows enforcement=disabled, main is UNPROTECTED until restored."
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

# ──────────────────────────────────────────────────────────────────────
# Ruleset-drift check (best-effort)
# ──────────────────────────────────────────────────────────────────────
# The disable + restore heredocs (protect-release-tags below) and the
# main_ruleset_put heredoc (protect-main above) each hardcode a canonical
# `required_status_checks` context list. If a LIVE ruleset was ever updated
# to require a context this script doesn't know about, the trap-driven
# restore would silently rewrite the ruleset back to the stale list —
# dropping the newer required checks. Surface that drift HERE, before the
# dance touches anything, so the maintainer reconciles the heredocs first.
# protect-release-tags drift is NON-FATAL (a stale tag-gate is recoverable);
# protect-main drift is FATAL — silently dropping a required check from the
# default branch is not something a release cut should ever do. If a live
# ruleset can't be read (gh/jq outage), skip that check rather than abort on
# an unrelated failure.
#
# $1 = ruleset id, $2 = human name, $3 = "fatal"|"warn",
# $4 = expected contexts (newline-separated, pre-sorted).
check_ruleset_drift() {
  local ruleset_id="$1" name="$2" severity="$3" expected="$4"
  local live
  live="$( { gh api "repos/${REPO}/rulesets/${ruleset_id}" 2>/dev/null \
    | jq -r '.rules[]? | select(.type=="required_status_checks")
              | .parameters.required_status_checks[]?.context' 2>/dev/null \
    | sort; } || true )"
  if [[ -z "${live}" ]]; then
    warn "could not read live ruleset ${ruleset_id} (${name}) required_status_checks"
    warn "  (gh/jq returned nothing) — skipping drift check. Eyeball the restore"
    warn "  heredoc by hand if you suspect the ruleset changed."
    return 0
  fi
  if [[ "${live}" == "${expected}" ]]; then
    ok "${name} required_status_checks match the script's hardcoded restore body"
    return 0
  fi
  warn "RULESET DRIFT: live ${name} required_status_checks differ from this"
  warn "  script's hardcoded heredocs."
  warn "  live ruleset requires:"
  printf '%s\n' "${live}"     | sed 's/^/[cut-release]       - /' >&2
  warn "  this script hardcodes:"
  printf '%s\n' "${expected}" | sed 's/^/[cut-release]       - /' >&2
  if [[ "${severity}" == "fatal" ]]; then
    die "${name} drift is fatal — reconcile the heredoc (and its expected-context list below) before cutting; proceeding would let the trap silently drop a required check from the default branch."
  fi
  warn "  RECONCILE the heredoc (and its expected-context list below) before"
  warn "  trusting the auto-restore — otherwise the trap will rewrite the ruleset"
  warn "  to the stale list. Proceeding anyway (non-fatal)."
  return 0
}

RELEASE_TAGS_EXPECTED_CONTEXTS="$(printf '%s\n' \
  "rustfmt" "clippy" "cargo-deny" \
  "test (ubuntu-latest)" "test (macos-latest)" "test (windows-latest)" \
  | sort)"
# Mirrors the protect-main main_ruleset_put heredoc above (9 contexts).
MAIN_EXPECTED_CONTEXTS="$(printf '%s\n' \
  "cargo-deny" "clippy" "dogfood" "rustfmt" "self-gate" "spa-browser" \
  "test (macos-latest)" "test (ubuntu-latest)" "test (windows-latest)" \
  | sort)"
check_ruleset_drift "${RULESET_ID}"      "protect-release-tags" "warn"  "${RELEASE_TAGS_EXPECTED_CONTEXTS}"
check_ruleset_drift "${MAIN_RULESET_ID}" "protect-main"         "fatal" "${MAIN_EXPECTED_CONTEXTS}"

# Idempotent resume: if any commit in recent history (up to 20 back) is a
# `chore(release): vX.Y.Z` commit AND Cargo.toml workspace version matches
# X.Y.Z AND CHANGELOG has a `## [X.Y.Z]` section, the prep work has already
# landed — likely a previous run aborted between "push release commit" and
# "tag dance" (e.g. transient GitHub API error during CI wait), possibly
# followed by additional commits on top (e.g. a fix-forward of the script
# itself). Skip re-prep, resume from CI wait, and **tag the release commit
# specifically** (not HEAD) so v0.7.0 captures the actual release commit
# even if HEAD has advanced.
#
# Caught from the v0.7.0 cut where a 502 mid-poll left the release commit
# pushed and the tag never created; the operator then committed a script
# fix on top before re-running.
RESUME_MODE=false
RELEASE_TARGET_SHA=""
CARGO_TOML_VERSION="$(awk '/^\[workspace\.package\]/{f=1;next} /^\[/{f=0} f && /^version = /' Cargo.toml | head -1 | sed -E 's/.*"([^"]+)".*/\1/')"
# `index($0, subj)` matches literally — `~` would treat `(` and `)` in
# `chore(release): vX.Y.Z` as regex parens and silently fail to match.
# `|| true` swallows SIGPIPE (141) from `git log` when awk's early `exit`
# closes stdin — `set -e -o pipefail` would otherwise abort the script
# on the (harmless) broken pipe.
RELEASE_TARGET_SHA="$(git log -n 20 --format='%H %s' 2>/dev/null \
                       | awk -v subj="chore(release): ${TAG}" 'index($0, subj) {print $1; exit}' \
                       || true)"
if [[ -n "${RELEASE_TARGET_SHA}" ]] \
   && [[ "${CARGO_TOML_VERSION}" == "${VERSION}" ]] \
   && grep -qE "^## \[${VERSION//./\\.}\]" CHANGELOG.md; then
  RESUME_MODE=true
  ok "detected existing release commit at ${RELEASE_TARGET_SHA:0:7} — resuming from CI wait"
  if [[ "$(git rev-parse HEAD)" != "${RELEASE_TARGET_SHA}" ]]; then
    # Retarget to HEAD rather than the historical release commit.
    #
    # Resuming after a TRANSIENT failure, the two are interchangeable. But a
    # cut can also fail because the release commit itself was wrong — the
    # v0.27.1 attempt failed CI because the commit omitted a file its own
    # re-stamp step had rewritten — and then the fix necessarily lands AFTER
    # it. Tagging the original commit there publishes the tree whose CI is
    # red, which is the one thing the CI gate below exists to prevent; it
    # would wait for a green that commit can never reach.
    #
    # HEAD is safe to target because the preconditions above already
    # established it carries the release state: Cargo.toml reads ${VERSION}
    # and CHANGELOG has its section. What HEAD adds beyond the release commit
    # is whatever landed since, so the operator is told exactly what that is
    # rather than it being folded in silently.
    warn "  HEAD has advanced past the release commit ${RELEASE_TARGET_SHA:0:7}"
    warn "  tag will target HEAD ($(git rev-parse --short HEAD)) so the release includes:"
    git log --oneline "${RELEASE_TARGET_SHA}..HEAD" | sed 's/^/    /' >&2
    RELEASE_TARGET_SHA="$(git rev-parse HEAD)"
  fi
  warn "  prep section (version bump, CHANGELOG flip, cargo update, commit, push) will be SKIPPED"
fi

# CHANGELOG must have a non-empty [Unreleased] section to flip into versioned.
# Empty Unreleased = nothing to release = probably a mistake.
# Skip this check in resume mode — the section was already flipped on
# the previous run.
if [[ "${RESUME_MODE}" != "true" ]]; then
  UNRELEASED_BODY="$(awk '/^## \[Unreleased\]/{f=1;next} /^## \[[0-9]/{f=0} f' CHANGELOG.md | sed '/^$/d')"
  if [[ -z "${UNRELEASED_BODY}" ]]; then
    die "CHANGELOG.md [Unreleased] section is empty. Add release notes before cutting."
  fi
  ok "CHANGELOG.md [Unreleased] section has content"
fi

# ──────────────────────────────────────────────────────────────────────
# Pre-release prep on a new commit
# ──────────────────────────────────────────────────────────────────────
if [[ "${RESUME_MODE}" == "true" ]]; then
  log "skipping prep (release commit already at HEAD)"
else
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

  # Re-stamp the findings ledger in the same breath. `Fixed (Unreleased)` is a
  # claim ABOUT the CHANGELOG's [Unreleased] section, so draining that section
  # above silently invalidates every such row: the finding shipped, but the
  # ledger the team reads to know what is done still says it did not.
  #
  # This has rotted twice from exactly that cause. Both times it was
  # reconciled by hand rather than at the source, so it rotted again on the
  # next cut — which is why the fix belongs here and not in another sweep.
  #
  # Both spellings are rewritten. The parenthesised and em-dash forms both
  # appear in the file, and handling only one would leave half the rows stale
  # while looking finished.
  python3 - "${VERSION}" docs/reports/deep_analysis_report.md <<'RESTAMP'
import sys

version, path = sys.argv[1], sys.argv[2]
src = open(path).read()
marks = ("Fixed (Unreleased)", "Fixed \u2014 Unreleased")
n = sum(src.count(m) for m in marks)
out = src.replace(marks[0], f"Fixed (v{version})").replace(
    marks[1], f"Fixed \u2014 v{version}"
)
open(path, "w").write(out)
print(f"  re-stamped {n} ledger row(s) to v{version}")
RESTAMP
  ok "re-stamped findings ledger Unreleased → v${VERSION}"

  # Sync the lockfile so cargo update doesn't churn it later.
  cargo update -p codelore-lib -p codelore -p codelore-rca --quiet
  ok "Cargo.lock synced to workspace ${VERSION}"
else
  log "[dry-run] would: bump Cargo.toml + flip CHANGELOG + cargo update"
fi

# Sanity build (catches CHANGELOG flip / Cargo.toml typos before commit)
log "running local gate (matching CI's exact invocation)..."
# v0.1.3 cut failed because the script's narrower local gate
# (`cargo build --release -p codelore`) didn't run clippy with the
# same flags CI uses, so a `clippy::useless_conversion` lint surfaced
# in code that local was happy with. Now we run the EXACT CI clippy
# command so the local gate is at least as strict as CI's. Tests stay
# narrow (release builds dominate the time budget here; tests already
# ran during the dev cycle that landed [Unreleased]).
run cargo clippy --workspace --all-targets --all-features -- -D warnings
run cargo fmt --all --check
run cargo build --release --quiet -p codelore
if [[ "${DRY_RUN}" != "true" ]]; then
  ACTUAL_VERSION="$(./target/release/codelore --version | awk '{print $2}')"
  if [[ "${ACTUAL_VERSION}" != "${VERSION}" ]]; then
    die "binary reports ${ACTUAL_VERSION}, expected ${VERSION}. Something went wrong."
  fi
  ok "binary reports codelore ${VERSION}"
fi

# The ledger is staged with the rest because the re-stamp above edits it.
# Leaving it out is what made the v0.27.1 cut fail its own guard: the commit
# drained CHANGELOG [Unreleased] while every ledger row still claimed to be
# backed by it — the exact rot the re-stamp exists to prevent, reintroduced by
# a step that edited a sixth file without adding it to a five-file list.
run git add Cargo.toml Cargo.lock CHANGELOG.md crates/*/Cargo.toml \
  docs/reports/deep_analysis_report.md
# Everything the prep phase touched must be in the commit. The `git add`
# above is an explicit list, and every step that writes a file has to be
# represented in it — a step added later without its file is silent: the
# edit happens, the commit omits it, and the release ships a tree that
# disagrees with what the script just did.
#
# That is not hypothetical. The ledger re-stamp shipped exactly this way,
# rewriting a sixth file against a five-file list, so the v0.27.1 cut
# drained CHANGELOG [Unreleased] while every ledger row still claimed to be
# backed by it. Listing the file fixed that instance; this catches the next
# one, whatever it edits.
#
# Untracked files are deliberately not considered — `git diff` ignores them,
# so local scratch (HANDOFF.md, reports in progress) never trips this.
if ! git diff --quiet; then
  warn "the prep phase modified tracked files that are NOT staged for the release commit:"
  git diff --name-only | sed 's/^/    /' >&2
  die "add them to the 'git add' list above, or revert them, then re-run"
fi
ok "every file the prep phase modified is staged"

run git commit -m "chore(release): ${TAG}"
ok "release commit created"

# ──────────────────────────────────────────────────────────────────────
# Push release commit + wait for CI green
# ──────────────────────────────────────────────────────────────────────
# protect-main requires 9 status checks on EVERY push to main, which a direct
# release-commit push cannot carry — GitHub rejects it. Disable protect-main
# for just this push (mirroring the tag dance's ruleset disable below), then
# re-enable it immediately after, so main is unprotected only for the push
# itself, not through the CI wait that follows. The EXIT trap re-enables it if
# the push aborts before the inline restore runs.
if [[ "${DRY_RUN}" != "true" ]]; then
  log "disabling protect-main ruleset for the release-commit push..."
  main_ruleset_put disabled
  MAIN_RULESET_DISABLED=true
  ok "protect-main disabled — release-commit push window OPEN"
else
  log "[dry-run] would: PUT protect-main ruleset with enforcement=disabled"
fi
run git push origin main
ok "release commit pushed to origin"
if [[ "${DRY_RUN}" != "true" ]]; then
  log "re-enabling protect-main ruleset..."
  if main_ruleset_put active; then
    MAIN_RULESET_DISABLED=false
    ok "protect-main re-enabled"
  else
    warn "protect-main re-enable FAILED — the EXIT trap will retry on exit"
  fi
fi
fi  # end of: if RESUME_MODE then skip-prep else prep+commit+push

if [[ "${SKIP_CI_WAIT}" == "true" ]]; then
  warn "--skip-ci-wait passed — NOT waiting for CI to confirm green"
  warn "  the tag push will fail if CI hasn't passed on this commit"
else
  log "waiting for CI to go green on the release commit (typical: 8–12 min)..."
  if [[ "${DRY_RUN}" != "true" ]]; then
    # Find the most recent CI run on main triggered by THIS push. A run
    # can take several seconds to register after the push, and a
    # not-yet-registered auto run looks identical to a paths-ignored commit
    # (both yield no match). Poll a bounded window before concluding
    # paths-ignore matched, so a slow-to-register auto run is never mistaken
    # for "no CI" and duplicated by a spurious manual dispatch.
    RELEASE_SHA="$(git rev-parse HEAD)"
    AUTO_ATTEMPTS=6
    AUTO_INTERVAL=10
    RUN_ID=""
    for (( attempt = 1; attempt <= AUTO_ATTEMPTS; attempt++ )); do
      sleep "${AUTO_INTERVAL}"
      RUN_ID="$(gh run list --limit 5 --branch main --workflow CI --json databaseId,headSha \
                --jq ".[] | select(.headSha == \"${RELEASE_SHA}\") | .databaseId" | head -1)"
      if [[ -n "${RUN_ID}" ]]; then
        break
      fi
      log "  waiting for auto-triggered CI run on ${RELEASE_SHA:0:7} (${attempt}/${AUTO_ATTEMPTS})..."
    done
    if [[ -z "${RUN_ID}" ]]; then
      # README-only or other paths-ignored commit. Manually dispatch CI so
      # the ruleset has the required status checks on this commit.
      warn "no CI run auto-triggered for ${RELEASE_SHA:0:7} after $(( AUTO_ATTEMPTS * AUTO_INTERVAL ))s (paths-ignore likely matched)."
      warn "dispatching CI manually via workflow_dispatch..."
      gh workflow run CI --ref main
      # The dispatched run takes a few seconds to register with a matching
      # headSha. Retry with a bounded window instead of a single sleep+read
      # — a run must NEVER be adopted without comparing headSha, since
      # accepting a stale workflow_dispatch run from an unrelated commit
      # would let an unchecked SHA reach the irreversible crates.io publish.
      DISPATCH_ATTEMPTS=12
      DISPATCH_INTERVAL=10
      for (( attempt = 1; attempt <= DISPATCH_ATTEMPTS; attempt++ )); do
        sleep "${DISPATCH_INTERVAL}"
        RUN_ID="$(gh run list --limit 5 --branch main --workflow CI --json databaseId,event,headSha \
                  --jq ".[] | select(.event == \"workflow_dispatch\" and .headSha == \"${RELEASE_SHA}\") | .databaseId" | head -1)"
        if [[ -n "${RUN_ID}" ]]; then
          break
        fi
        log "  waiting for dispatched CI run on ${RELEASE_SHA:0:7} (${attempt}/${DISPATCH_ATTEMPTS})..."
      done
      if [[ -z "${RUN_ID}" ]]; then
        die "dispatched CI run for ${RELEASE_SHA:0:7} did not register within $(( DISPATCH_ATTEMPTS * DISPATCH_INTERVAL ))s. Check 'gh run list' manually — never adopt a run without a matching headSha."
      fi
    fi
    if [[ -z "${RUN_ID}" ]]; then
      die "could not locate a CI run for ${RELEASE_SHA:0:7}. Check 'gh run list' manually."
    fi
    log "watching CI run ${RUN_ID}..."
    # `gh run watch --exit-status` returns 0 for both "success" AND
    # "cancelled" runs (per gh CLI source — anything that isn't "failure"
    # exits 0). We've been bitten by that conflation during v0.1.2/v0.1.3
    # cuts where a concurrency-cancelled prior run was misread as green
    # CI. So we poll status + conclusion via the API directly instead of
    # trusting gh run watch's exit code, and the script ONLY proceeds to
    # the tag dance if conclusion == "success" — anything else (failure,
    # cancelled, timed_out, action_required, neutral, skipped) aborts.
    #
    # The v0.7.0 cut hit a different failure: `gh run watch` errored with
    # an HTTP 502 from GitHub mid-poll, the subsequent `gh run view --json
    # conclusion` returned empty (because the run was still in progress
    # after watch died early), and the script died treating empty as a
    # non-success conclusion. The polling loop below tolerates transient
    # 5xx / timeout errors with retries, only declares completion when
    # `status == "completed"`, and only then reads `conclusion`.
    #
    # Hard cap: 40 minutes. CI typically completes in 12-25 min;
    # 40 min is the windows-latest p99 ceiling on this repo.
    POLL_INTERVAL=30
    MAX_WAIT_SECONDS=2400
    MAX_TRANSIENT_FAILS=5
    elapsed=0
    transient_fails=0
    STATUS=""
    CONCLUSION=""
    while (( elapsed < MAX_WAIT_SECONDS )); do
      if probe="$(gh run view "${RUN_ID}" --json status,conclusion 2>/dev/null)"; then
        STATUS="$(printf '%s' "${probe}" | jq -r '.status // ""')"
        CONCLUSION="$(printf '%s' "${probe}" | jq -r '.conclusion // ""')"
        transient_fails=0
        if [[ "${STATUS}" == "completed" ]]; then
          break
        fi
        log "  CI run ${RUN_ID}: status=${STATUS} (elapsed ${elapsed}s)"
      else
        transient_fails=$(( transient_fails + 1 ))
        warn "  transient gh API error polling run ${RUN_ID} (${transient_fails}/${MAX_TRANSIENT_FAILS})"
        if (( transient_fails >= MAX_TRANSIENT_FAILS )); then
          die "gh API repeatedly errored polling CI run ${RUN_ID} (${MAX_TRANSIENT_FAILS}× transient failures). Check 'gh run view ${RUN_ID}' manually and re-run with --skip-ci-wait once green."
        fi
      fi
      sleep "${POLL_INTERVAL}"
      elapsed=$(( elapsed + POLL_INTERVAL ))
    done
    if [[ "${STATUS}" != "completed" ]]; then
      die "CI run ${RUN_ID} did not complete within ${MAX_WAIT_SECONDS}s (last status: '${STATUS:-unknown}'). Investigate via 'gh run view ${RUN_ID}' and re-run with --skip-ci-wait once green."
    fi
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

# In resume mode, tag the actual release commit (not HEAD) — HEAD may
# have advanced past the release commit (e.g. fix-forward of script
# itself before the re-run). Without this, the tag would point at a
# post-release commit and the published archives would include
# post-release changes silently.
TAG_TARGET="${RELEASE_TARGET_SHA:-HEAD}"
if [[ "${RESUME_MODE}" == "true" ]] && [[ "${TAG_TARGET}" != "HEAD" ]]; then
  log "tagging ${TAG_TARGET:0:7} (the release commit, not HEAD ${LOCAL:0:7})"
fi
run git tag -a "${TAG}" "${TAG_TARGET}" -m "${TAG}

See CHANGELOG.md [${VERSION}] section for the full list of changes."
ok "annotated tag ${TAG} created locally at ${TAG_TARGET:0:7}"

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

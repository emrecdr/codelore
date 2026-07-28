//! The AG-SZZ linkage engine: traces each fix commit's deleted pre-image
//! lines back to the commit that last introduced them, behind a pluggable
//! [`LineOriginSource`] seam (the roadmap's "pluggable SZZ" — this is the
//! first rung; Neural-SZZ/SmartCommit can slot in later without churn).
//!
//! No production git-subprocess implementation lives here: this crate never
//! shells out to git (see `Repo::read_blob_at` for the one git-reading seam
//! this module uses). The production [`LineOriginSource`] shells
//! `git blame --porcelain` from the CLI at artifact-build time; tests use an
//! in-memory fake.
//!
//! # Algorithm
//!
//! For each fix commit (already classified by [`super::DefectOracle`]):
//! 1. [`deleted_ranges`] reads the fix's deleted pre-image lines straight
//!    from the `hunks` table.
//! 2. [`LineOriginSource::origins`] is asked, once per touched file, which
//!    commit last introduced each deleted line — queried at the fix's FIRST
//!    PARENT (the revision immediately before the fix), since that's the
//!    tree the deleted lines still existed in.
//! 3. The **tangled-commit guard** excludes an oversized fix from linkage
//!    outright (see [`TANGLED_MAX_FILES`] / [`TANGLED_MAX_CHURN`]); the
//!    **ghost guard** skips whole-file-deletion blame targets within an
//!    otherwise-kept fix.
//! 4. The **AG filter** drops candidates whose line is cosmetic — blank, or
//!    comment-only for the file's Tier-1 language — see [`is_cosmetic_line`].
//! 5. The **clock-skew guard** discards candidates that aren't strictly
//!    older than the fix itself.
//! 6. Survivors dedupe into `(defect_rev, fix_rev, path)` [`SzzLink`]s.
//!
//! # Tangled and ghost guards
//!
//! AG-SZZ over-attributes on two shapes of fix commit. A **tangled** fix
//! bundles the correction with unrelated edits, so most of its deleted lines
//! are not the fix — Herzig, Just & Zeller ("The Impact of Tangled Code
//! Changes on Defect Prediction Models", MSR 2013) show tangled changes
//! distort defect attribution. A **ghost** fix's diff cannot carry a fix at
//! all: a whole-file deletion removes code wholesale, so blaming its removed
//! lines attributes the "defect" to everyone who ever touched the file —
//! extending Kim, Zimmermann, Pan & Whitehead's AG-SZZ cosmetic filter (ASE
//! 2006) from cosmetic *lines* to file-level removals. Both guards exclude
//! rather than down-weight (links carry no weight field), and both disclose a
//! mined-vs-excluded count in the command output — never in the artifact.
//!
//! Per-file blame failures are skip-with-log, never fatal — mining
//! continues with the fix's other files and the remaining fixes; failures
//! are tallied in [`super::MiningStats::blame_failures`].

use std::collections::{HashMap, HashSet};

use crate::analyses::query::query_map_collect;
use crate::complexity::Tier1Language;
use crate::facts::FactsDb;
use crate::repo::Repo;
use crate::{CodeLoreError, Result};

use super::MiningStats;

/// Tangled-commit guard: a fix touching more than this many files is presumed
/// to mix the fix with unrelated edits (Herzig, Just & Zeller, MSR 2013), so
/// most of its deleted lines are not the fix and it is excluded from linkage.
/// Deliberately generous — an ordinary multi-file fix passes; the bound drops
/// only the mega-commits (sweeping refactors, mechanical renames, dependency
/// bumps) that carry a `fix`-word yet fan across the tree.
pub const TANGLED_MAX_FILES: u32 = 8;

/// Tangled-commit guard: a fix changing more than this many lines
/// (added + deleted, across all its files) is presumed too large to be a
/// focused fix and is excluded from linkage. The churn companion to
/// [`TANGLED_MAX_FILES`] — a fix concentrated in few files can still be a
/// tree-wide mechanical change.
pub const TANGLED_MAX_CHURN: u32 = 400;

/// Pluggable line-origin seam (roadmap's "pluggable SZZ"). Given a file at a
/// revision, returns for each requested 1-based line the commit that last
/// introduced it. The production impl (CLI) shells `git blame --porcelain`;
/// tests use an in-memory fake.
pub trait LineOriginSource {
    /// Blame `path` at `rev` for the given 1-based `lines`, returning
    /// `(line, introducing_commit)` pairs. Implementations may return fewer
    /// pairs than requested lines (e.g. a line beyond EOF at `rev`); callers
    /// must not assume a 1:1 return count.
    ///
    /// # Errors
    ///
    /// Any I/O or subprocess failure the backend hits reading `path` at
    /// `rev`. Callers treat this as a skip-with-log, never fatal.
    fn origins(&self, rev: &str, path: &str, lines: &[u32]) -> Result<Vec<(u32, String)>>;
}

/// Parse `git blame --porcelain` output into (`line_number`, commit) pairs —
/// pure function so the parser is unit-testable without git.
///
/// Porcelain format recap: each blamed line starts with a header
/// `<40-hex-sha> <orig-line> <final-line> [<group-size>]` — the trailing
/// group-size field is present only on the first line of each contiguous
/// chunk. Metadata lines (`author`, `author-mail`, `committer*`, `summary`,
/// `previous`, `filename`, `boundary`, …) follow; the chunk's content line
/// is the only one prefixed with a literal TAB. Crucially, a commit's full
/// metadata block is only emitted the FIRST time that commit appears
/// anywhere in a single invocation's output — a later, non-contiguous chunk
/// from an already-seen commit repeats only its header line, then goes
/// straight to the TAB-prefixed content. This parser only needs the header
/// (for the final line number and commit sha) and the content-line marker
/// (to know when to emit a pair); everything else is metadata and is
/// ignored.
///
/// # Errors
///
/// [`CodeLoreError::Analysis`] if a content line (TAB-prefixed) appears with
/// no preceding header — malformed porcelain input.
pub fn parse_blame_porcelain(output: &str) -> Result<Vec<(u32, String)>> {
    let mut pairs = Vec::new();
    let mut pending: Option<(String, u32)> = None;

    for line in output.lines() {
        if line.starts_with('\t') {
            let (sha, final_line) = pending.take().ok_or_else(|| {
                CodeLoreError::Analysis(
                    "blame porcelain: content line with no preceding header".to_string(),
                )
            })?;
            pairs.push((final_line, sha));
            continue;
        }
        if let Some(header) = parse_header_line(line) {
            pending = Some(header);
        }
        // Any other line (author/author-mail/committer*/summary/previous/
        // filename/boundary) is metadata for the pending header — ignored.
    }

    Ok(pairs)
}

/// Recognize a porcelain header line (`<40-hex-sha> <orig-line> <final-line>
/// [<group-size>]`) and return `(sha, final_line)`. Any other line — a
/// metadata line, since none of them start with a 40-hex-char token — is
/// `None`.
fn parse_header_line(line: &str) -> Option<(String, u32)> {
    let mut fields = line.split(' ');
    let sha = fields.next()?;
    if sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let _orig_line: u32 = fields.next()?.parse().ok()?;
    let final_line: u32 = fields.next()?.parse().ok()?;
    // An optional trailing group-size field may follow; not needed here.
    Some((sha.to_string(), final_line))
}

/// A fix commit's deleted ranges per file, straight from the `hunks` table.
/// `old_start`/`old_lines` are the pre-image (deleted) side — see
/// `schema_v1.sql`'s `hunks` table doc.
///
/// # Errors
///
/// [`CodeLoreError::Analysis`] on a `FactsDb` query failure.
pub fn deleted_ranges(db: &FactsDb, fix_rev: &str) -> Result<Vec<(String, u32, u32)>> {
    query_map_collect(
        db,
        "SELECT path, old_start, old_lines FROM hunks \
         WHERE rev = ? AND old_lines > 0 ORDER BY path, old_start",
        duckdb::params![fix_rev],
        "szz:deleted-ranges",
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, u32>(2)?,
            ))
        },
    )
}

/// One AG-SZZ link: `fix_rev` at `path` is believed to fix a defect
/// introduced by `defect_rev`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SzzLink {
    pub defect_rev: String,
    pub fix_rev: String,
    pub path: String,
}

/// The line-comment prefix for a Tier-1 language's single-line comments.
#[must_use]
pub fn line_comment_prefix(lang: Tier1Language) -> &'static str {
    match lang {
        Tier1Language::Rust
        | Tier1Language::Java
        | Tier1Language::JavaScript
        | Tier1Language::TypeScript
        | Tier1Language::Tsx => "//",
        Tier1Language::Python => "#",
    }
}

/// AG filter: a line is cosmetic when it is blank, or — for a recognized
/// Tier-1 language — starts (after trimming) with that language's
/// line-comment prefix.
///
/// Unknown language (`lang: None`, e.g. an unsupported extension) is
/// conservatively **not** cosmetic for non-blank content: without a comment
/// syntax to check against, we cannot tell, and a false "cosmetic" would
/// silently drop a real candidate link. Unreadable blobs are handled by the
/// caller ([`link_defects`]), which applies this same conservative default
/// before it ever has content to pass in here.
///
/// Documented limitation: this only recognizes single-line comment syntax.
/// A line inside a block comment that doesn't itself carry the line-comment
/// prefix is NOT filtered — the honest first rung; such lines are counted
/// in `MiningStats::lines_considered` like any other candidate.
#[must_use]
pub fn is_cosmetic_line(content: &str, lang: Option<Tier1Language>) -> bool {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return true;
    }
    match lang {
        Some(lang) => trimmed.starts_with(line_comment_prefix(lang)),
        None => false,
    }
}

/// Reconstruct a deleted line's content at the fix's FIRST PARENT — the
/// exact `(rev, line)` pair [`LineOriginSource::origins`] was itself queried
/// with — and classify it via [`is_cosmetic_line`].
///
/// This deliberately reads at the fix's parent rather than at the candidate
/// defect commit itself: the simplified `(line, commit)` origin pairs carry
/// no "original line number inside the defect commit" (real `git blame
/// --porcelain`'s `orig-line` field, which this crate's parser doesn't
/// surface — see [`parse_blame_porcelain`]), so there is no reliable line
/// index to read the defect commit's own blob at. By blame's own guarantee,
/// though, the line's text is unchanged between the defect commit and the
/// fix's parent *modulo whitespace* (a `-w` blame treats re-indented lines
/// as unchanged) — and since [`is_cosmetic_line`] trims before both its
/// blank and comment-prefix checks, the classification is invariant to
/// exactly that divergence. So for this filter's purpose this reads the
/// same content the spec calls "at the blamed revision", without an
/// unresolvable line-index translation.
///
/// Any failure to obtain the text (unreadable blob, non-UTF-8 content, an
/// out-of-range line) falls back to the same conservative default
/// `is_cosmetic_line` uses for an unrecognized language: NOT cosmetic, so
/// the candidate is kept rather than silently dropped.
fn is_candidate_cosmetic<R: Repo>(repo: &R, parent_rev: &str, path: &str, line_no: u32) -> bool {
    let Ok(Some(bytes)) = repo.read_blob_at(parent_rev, path) else {
        return false;
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return false;
    };
    let line_index = usize::try_from(line_no.saturating_sub(1)).unwrap_or(usize::MAX);
    let Some(line_content) = text.lines().nth(line_index) else {
        return false;
    };
    is_cosmetic_line(line_content, Tier1Language::from_path(path))
}

/// One fix commit's identity: its own rev, its first parent (the revision
/// blamed), and its date (the clock-skew guard's upper bound). Bundled so
/// the per-file helpers below stay under the workspace's argument-count
/// lint without resorting to `#[allow]`.
#[derive(Debug, Clone, Copy)]
struct FixContext<'a> {
    fix_rev: &'a str,
    parent_rev: &'a str,
    fix_date: &'a str,
}

/// Mutable state threaded through the per-fix, per-file blame loop.
struct LinkAccumulator {
    stats: MiningStats,
    links: Vec<SzzLink>,
    seen: HashSet<(String, String, String)>,
}

/// The AG-SZZ engine: for each fix commit, excludes the tangled ones outright
/// (tallied in `MiningStats::fixes_excluded_tangled`), then blames the
/// remaining fixes' deleted pre-image lines at the fix's FIRST PARENT —
/// skipping whole-file-deletion blame targets (the ghost guard, tallied in
/// `MiningStats::ghost_files_skipped`), dropping cosmetic candidates (the AG
/// filter), and discarding candidates that aren't strictly older than the fix
/// (the clock-skew guard) — and dedupes the survivors into
/// `(defect_rev, fix_rev, path)` links.
///
/// `fixes` is `(rev, parent_rev, date)` for every commit the oracle already
/// classified as a fix — `commit_dates` supplies the date for every
/// candidate (defect) commit, keyed by rev; a candidate whose date is
/// missing is conservatively discarded (the clock-skew guard cannot be
/// verified). Dates are compared LEXICOGRAPHICALLY for the clock-skew
/// guard, so callers must supply them in one zero-padded, consistently
/// UTC-normalized format for every commit (the fact store's `commits.date`
/// timestamps rendered as strings satisfy this; mixed timezone offsets or
/// unpadded fields would not). A fix with no deleted pre-image lines (a
/// pure-addition commit) contributes no candidates and is counted in
/// `MiningStats::pure_addition_fixes`.
///
/// Returned links are sorted by `(defect_rev, fix_rev, path)` — mining
/// determinism does not depend on hash-map iteration order.
///
/// # Errors
///
/// Only [`deleted_ranges`]' `FactsDb` query can fail here (a malformed
/// store); per-file blame failures are handled internally (skip-with-log,
/// tallied in `MiningStats::blame_failures`) and never propagate.
pub fn link_defects<R: Repo>(
    db: &FactsDb,
    repo: &R,
    origin: &dyn LineOriginSource,
    fixes: &[(String, String, String)],
    commit_dates: &HashMap<String, String, impl std::hash::BuildHasher>,
) -> Result<(Vec<SzzLink>, MiningStats)> {
    let mut acc = LinkAccumulator {
        stats: MiningStats {
            fixes_found: u32::try_from(fixes.len()).unwrap_or(u32::MAX),
            ..MiningStats::default()
        },
        links: Vec::new(),
        seen: HashSet::new(),
    };

    for (fix_rev, parent_rev, fix_date) in fixes {
        let fix = FixContext {
            fix_rev,
            parent_rev,
            fix_date,
        };
        link_one_fix(db, repo, origin, fix, commit_dates, &mut acc)?;
    }

    acc.stats.links_found = u32::try_from(acc.links.len()).unwrap_or(u32::MAX);
    acc.links.sort_by(|a, b| {
        (&a.defect_rev, &a.fix_rev, &a.path).cmp(&(&b.defect_rev, &b.fix_rev, &b.path))
    });
    Ok((acc.links, acc.stats))
}

/// Process one fix commit: group its deleted lines by path, then blame each
/// file (see [`blame_one_file`]).
fn link_one_fix<R: Repo>(
    db: &FactsDb,
    repo: &R,
    origin: &dyn LineOriginSource,
    fix: FixContext<'_>,
    commit_dates: &HashMap<String, String, impl std::hash::BuildHasher>,
    acc: &mut LinkAccumulator,
) -> Result<()> {
    // Tangled-commit guard: an oversized fix is excluded from linkage
    // outright, before any blame — most of its deleted lines are not the fix.
    if fix_is_tangled(db, fix.fix_rev)? {
        acc.stats.fixes_excluded_tangled += 1;
        return Ok(());
    }

    let deleted = deleted_ranges(db, fix.fix_rev)?;
    if deleted.is_empty() {
        acc.stats.pure_addition_fixes += 1;
        return Ok(());
    }

    // Ghost guard: a file removed wholesale in this fix (change_type
    // 'deleted') carries only removed lines, which cannot embody an in-place
    // fix — skip its blame targets rather than over-attribute to every past
    // author. Pure renames are already inert here (a content-free rename
    // produces no deleted hunk), and whitespace / comment lines are handled
    // downstream by the `-w` blame and the AG cosmetic filter.
    let ghost_paths = whole_file_deletions(db, fix.fix_rev)?;

    let mut lines_by_path: HashMap<&str, Vec<u32>> = HashMap::new();
    let mut ghost_skipped: HashSet<&str> = HashSet::new();
    for (path, start, count) in &deleted {
        if ghost_paths.contains(path.as_str()) {
            ghost_skipped.insert(path.as_str());
            continue;
        }
        lines_by_path
            .entry(path.as_str())
            .or_default()
            .extend(*start..start.saturating_add(*count));
    }
    acc.stats.ghost_files_skipped += u32::try_from(ghost_skipped.len()).unwrap_or(u32::MAX);

    for (path, lines) in lines_by_path {
        blame_one_file(repo, origin, fix, path, &lines, commit_dates, acc);
    }
    Ok(())
}

/// Tangled-commit guard predicate: true when the fix touches more than
/// [`TANGLED_MAX_FILES`] files OR changes more than [`TANGLED_MAX_CHURN`]
/// lines (added + deleted) across the `changes` table — too large to be a
/// focused fix. A fix with no `changes` rows (nothing recorded) is not
/// tangled.
///
/// # Errors
///
/// [`CodeLoreError::Analysis`] on a `FactsDb` query failure.
fn fix_is_tangled(db: &FactsDb, fix_rev: &str) -> Result<bool> {
    let footprint: Vec<(i64, i64)> = query_map_collect(
        db,
        "SELECT COUNT(*), COALESCE(SUM(loc_added + loc_deleted), 0) \
         FROM changes WHERE rev = ?",
        duckdb::params![fix_rev],
        "szz:fix-footprint",
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
    )?;
    let (files, churn) = footprint.first().copied().unwrap_or((0, 0));
    Ok(files > i64::from(TANGLED_MAX_FILES) || churn > i64::from(TANGLED_MAX_CHURN))
}

/// The ghost guard's skip set: the paths a fix removes wholesale
/// (`change_type = 'deleted'`). Their removed lines can't carry an in-place
/// fix, so blaming them over-attributes.
///
/// # Errors
///
/// [`CodeLoreError::Analysis`] on a `FactsDb` query failure.
fn whole_file_deletions(db: &FactsDb, fix_rev: &str) -> Result<HashSet<String>> {
    let paths: Vec<String> = query_map_collect(
        db,
        "SELECT path FROM changes WHERE rev = ? AND change_type = 'deleted'",
        duckdb::params![fix_rev],
        "szz:ghost-deletions",
        |r| r.get::<_, String>(0),
    )?;
    Ok(paths.into_iter().collect())
}

/// Blame one `(fix, path)` pair's deleted lines at the fix's parent, then
/// route each returned candidate through the AG filter and clock-skew guard
/// (see [`record_candidate`]). A blame failure for this file is
/// skip-with-log — tallied in `MiningStats::blame_failures`, never fatal.
fn blame_one_file<R: Repo>(
    repo: &R,
    origin: &dyn LineOriginSource,
    fix: FixContext<'_>,
    path: &str,
    lines: &[u32],
    commit_dates: &HashMap<String, String, impl std::hash::BuildHasher>,
    acc: &mut LinkAccumulator,
) {
    acc.stats.files_blamed += 1;
    acc.stats.lines_considered += u32::try_from(lines.len()).unwrap_or(u32::MAX);

    let candidates = match origin.origins(fix.parent_rev, path, lines) {
        Ok(candidates) => candidates,
        Err(e) => {
            tracing::warn!("szz: blame failed for {path}@{}: {e}", fix.parent_rev);
            acc.stats.blame_failures += 1;
            return;
        }
    };

    for (line_no, defect_rev) in candidates {
        record_candidate(repo, fix, path, line_no, defect_rev, commit_dates, acc);
    }
}

/// Apply the AG filter then the clock-skew guard to one candidate; push a
/// deduped [`SzzLink`] when both pass.
fn record_candidate<R: Repo>(
    repo: &R,
    fix: FixContext<'_>,
    path: &str,
    line_no: u32,
    defect_rev: String,
    commit_dates: &HashMap<String, String, impl std::hash::BuildHasher>,
    acc: &mut LinkAccumulator,
) {
    if is_candidate_cosmetic(repo, fix.parent_rev, path, line_no) {
        acc.stats.lines_dropped_cosmetic += 1;
        return;
    }

    let Some(defect_date) = commit_dates.get(&defect_rev) else {
        tracing::debug!(
            "szz: no commit date recorded for candidate {defect_rev}; \
             discarding (cannot verify the clock-skew guard)"
        );
        return;
    };
    if defect_date.as_str() >= fix.fix_date {
        return; // Clock-skew guard: not strictly older than the fix.
    }

    let key = (
        defect_rev.clone(),
        fix.fix_rev.to_string(),
        path.to_string(),
    );
    if acc.seen.insert(key) {
        acc.links.push(SzzLink {
            defect_rev,
            fix_rev: fix.fix_rev.to_string(),
            path: path.to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::TagInfo;
    use crate::{CommitEvent, FileChange, Hunk, Options};

    // ─── (a) porcelain parser ────────────────────────────────────────────────

    /// A real `git blame --porcelain -L 1,8 -- crates/codelore-lib/src/stats.rs`
    /// capture from this repository's own history: a 5-line contiguous chunk
    /// from one commit (lines 2-5 correctly carry only the abbreviated
    /// `<sha> <orig-line> <final-line>` header, no repeated metadata), then
    /// two more commits — the second of which (`e8486215…`) reappears at
    /// line 8, its own second, non-contiguous mention in this invocation,
    /// which likewise carries no repeated metadata block.
    const CAPTURED_PORCELAIN: &str = r"19359ed79bd51cd486ac341dad62ef5fb0c1bb5a 1 1 5
author Emre Camdere
author-mail <emre@valocom.nl>
author-time 1784103133
author-tz +0200
committer Emre Camdere
committer-mail <emre@valocom.nl>
committer-time 1784103133
committer-tz +0200
summary feat(stats): AUC and precision@k helpers
previous 400050fef8c362222a3abdb8779f0edcc424f93c crates/codelore-lib/src/stats.rs
filename crates/codelore-lib/src/stats.rs
	//! Statistical helpers used by the analyses.
19359ed79bd51cd486ac341dad62ef5fb0c1bb5a 2 2
	//!
19359ed79bd51cd486ac341dad62ef5fb0c1bb5a 3 3
	//! Fisher's exact two-tail p-value for a 2×2 contingency table, used
19359ed79bd51cd486ac341dad62ef5fb0c1bb5a 4 4
	//! by `analyses::coupling` to gate coupling pairs at
19359ed79bd51cd486ac341dad62ef5fb0c1bb5a 5 5
	//! `p < fisher_significance`.
e8486215e55737c14e0787394a0467e84b346e69 5 6 1
author Emre Camdere
author-mail <emre@valocom.nl>
author-time 1782082568
author-tz +0200
committer Emre Camdere
committer-mail <emre@valocom.nl>
committer-time 1782082568
committer-tz +0200
summary security: port fishers_exact in-tree; drop unmaintained supply-chain dep
filename crates/codelore-lib/src/stats.rs
	//!
f33423b4b6e3662a00c8886426673702aad99b70 6 7 1
author Emre Camdere
author-mail <emre@valocom.nl>
author-time 1782145461
author-tz +0200
committer Emre Camdere
committer-mail <emre@valocom.nl>
committer-time 1782145461
committer-tz +0200
summary docs: strip task-ID (F<NN>) references from all code comments
previous 73b8f2783999ca5ebe55233ac9b9f00e08ca6699 crates/codelore-lib/src/stats.rs
filename crates/codelore-lib/src/stats.rs
	//! In-tree port of the algorithm previously consumed via the
e8486215e55737c14e0787394a0467e84b346e69 7 8 1
	//! `fishers_exact` crate (last release 2018-11). The crate had no live
";

    #[test]
    fn parses_captured_porcelain_output_incl_repeated_commit_group() {
        let pairs = parse_blame_porcelain(CAPTURED_PORCELAIN).expect("parse captured porcelain");
        assert_eq!(
            pairs,
            vec![
                (1, "19359ed79bd51cd486ac341dad62ef5fb0c1bb5a".to_string()),
                (2, "19359ed79bd51cd486ac341dad62ef5fb0c1bb5a".to_string()),
                (3, "19359ed79bd51cd486ac341dad62ef5fb0c1bb5a".to_string()),
                (4, "19359ed79bd51cd486ac341dad62ef5fb0c1bb5a".to_string()),
                (5, "19359ed79bd51cd486ac341dad62ef5fb0c1bb5a".to_string()),
                (6, "e8486215e55737c14e0787394a0467e84b346e69".to_string()),
                (7, "f33423b4b6e3662a00c8886426673702aad99b70".to_string()),
                (8, "e8486215e55737c14e0787394a0467e84b346e69".to_string()),
            ]
        );
    }

    #[test]
    fn content_line_without_a_preceding_header_is_a_parse_error() {
        let err = parse_blame_porcelain("\tsome content with no header").expect_err("must fail");
        assert!(matches!(err, CodeLoreError::Analysis(_)));
    }

    #[test]
    fn metadata_only_lines_produce_no_pairs() {
        let pairs = parse_blame_porcelain("author Someone\nsummary nothing to see").expect("parse");
        assert!(pairs.is_empty());
    }

    // ─── (b) is_cosmetic_line table ──────────────────────────────────────────

    #[test]
    fn blank_content_is_cosmetic_regardless_of_language() {
        assert!(is_cosmetic_line("", Some(Tier1Language::Rust)));
        assert!(is_cosmetic_line("   ", None));
    }

    #[test]
    fn line_comment_prefix_matches_the_language() {
        assert!(is_cosmetic_line("// note", Some(Tier1Language::Rust)));
        assert!(is_cosmetic_line("# note", Some(Tier1Language::Python)));
    }

    #[test]
    fn mismatched_comment_syntax_is_not_cosmetic() {
        assert!(!is_cosmetic_line("# note", Some(Tier1Language::Rust)));
    }

    #[test]
    fn real_code_with_a_trailing_comment_is_not_cosmetic() {
        assert!(!is_cosmetic_line(
            "let x = 1; // t",
            Some(Tier1Language::Rust)
        ));
    }

    #[test]
    fn unknown_language_is_conservatively_not_cosmetic() {
        assert!(!is_cosmetic_line("// note", None));
    }

    #[test]
    fn line_comment_prefix_table_matches_the_spec() {
        assert_eq!(line_comment_prefix(Tier1Language::Rust), "//");
        assert_eq!(line_comment_prefix(Tier1Language::Java), "//");
        assert_eq!(line_comment_prefix(Tier1Language::JavaScript), "//");
        assert_eq!(line_comment_prefix(Tier1Language::TypeScript), "//");
        assert_eq!(line_comment_prefix(Tier1Language::Tsx), "//");
        assert_eq!(line_comment_prefix(Tier1Language::Python), "#");
    }

    // ─── (c) link_defects with a FakeOrigin + FakeRepo ───────────────────────

    /// Blob content keyed by `(rev, path)`; unregistered keys read as
    /// "not tracked" (`Ok(None)`), matching a real `Repo`'s contract. Every
    /// other `Repo` method is unreachable from `link_defects` and panics if
    /// ever called.
    struct FakeRepo {
        blobs: HashMap<(String, String), Vec<u8>>,
    }

    impl Repo for FakeRepo {
        fn walk_commits<'a>(
            &'a self,
            _opts: &'a Options,
        ) -> Result<Box<dyn Iterator<Item = Result<CommitEvent>> + Send + 'a>> {
            unimplemented!("szz tests never walk commits")
        }

        fn changed_files(&self, _rev: &str) -> Result<Vec<FileChange>> {
            unimplemented!("szz tests never list changed files")
        }

        fn diff_hunks(&self, _rev: &str, _path: &str) -> Result<Vec<Hunk>> {
            unimplemented!("szz tests never diff hunks directly")
        }

        fn resolve_alias(&self, _name: &str, _email: &str) -> String {
            unimplemented!("szz tests never resolve aliases")
        }

        fn head_sha(&self) -> Result<String> {
            unimplemented!("szz tests never read HEAD")
        }

        fn tracked_paths_at_head(&self) -> Result<Vec<String>> {
            unimplemented!("szz tests never list tracked paths")
        }

        fn tags(&self) -> Result<Vec<TagInfo>> {
            unimplemented!("szz tests never read tags")
        }

        fn read_blob_at(&self, rev: &str, path: &str) -> Result<Option<Vec<u8>>> {
            Ok(self
                .blobs
                .get(&(rev.to_string(), path.to_string()))
                .cloned())
        }
    }

    /// A `LineOriginSource` double keyed by `(rev, path)`; `fail_for` keys
    /// return `Err` to exercise the per-file blame-failure path.
    struct FakeOrigin {
        table: HashMap<(String, String), Vec<(u32, String)>>,
        fail_for: HashSet<(String, String)>,
    }

    impl LineOriginSource for FakeOrigin {
        fn origins(&self, rev: &str, path: &str, lines: &[u32]) -> Result<Vec<(u32, String)>> {
            let key = (rev.to_string(), path.to_string());
            if self.fail_for.contains(&key) {
                return Err(CodeLoreError::Analysis(format!(
                    "fake blame failure for {path}@{rev}"
                )));
            }
            Ok(self
                .table
                .get(&key)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|(line, _)| lines.contains(line))
                .collect())
        }
    }

    /// Seeds one `commits` row for `rev` — called once per distinct fix
    /// revision before any of its per-path [`seed_hunk`] calls (`commits.rev`
    /// is a primary key).
    fn seed_commit(db: &FactsDb, rev: &str) {
        db.conn()
            .execute(
                "INSERT INTO commits (rev, author_email, author_name, \
                 committer_email, canonical_author, date, committer_date, \
                 message, is_merge, parent_count) \
                 VALUES (?, 'a@b.com', 'A', 'a@b.com', 'A', \
                         TIMESTAMPTZ '2026-01-01', TIMESTAMPTZ '2026-01-01', \
                         'fix: test', false, 1)",
                duckdb::params![rev],
            )
            .expect("insert commit");
    }

    /// Seeds one modified-with-deletion `changes` row + its `hunks` row
    /// directly — `deleted_ranges` only ever reads the `hunks` table, so no
    /// git fixture is needed for this test. Requires [`seed_commit`] for
    /// `rev` to have already run (the FK from `changes`/`hunks` to
    /// `commits`).
    fn seed_hunk(db: &FactsDb, rev: &str, path: &str, old_start: u32, old_lines: u32) {
        db.conn()
            .execute(
                "INSERT INTO changes (rev, path, change_type, loc_added, loc_deleted) \
                 VALUES (?, ?, 'modified', 0, ?)",
                duckdb::params![rev, path, old_lines],
            )
            .expect("insert change");
        db.conn()
            .execute(
                "INSERT INTO hunks (rev, path, old_start, old_lines, new_start, new_lines) \
                 VALUES (?, ?, ?, ?, 1, 0)",
                duckdb::params![rev, path, old_start, old_lines],
            )
            .expect("insert hunk");
    }

    #[test]
    fn link_defects_links_filters_cosmetic_and_respects_clock_skew_and_blame_failures() {
        let db = FactsDb::new_in_memory().expect("in-memory db");

        // fix-old touches three files at its parent: a.rs (real code, older
        // origin A → link), c.rs (cosmetic comment, older origin B →
        // dropped), d.rs (FakeOrigin errors → blame_failures += 1, skipped).
        seed_commit(&db, "fix-old");
        seed_hunk(&db, "fix-old", "src/a.rs", 1, 1);
        seed_hunk(&db, "fix-old", "src/c.rs", 1, 1);
        seed_hunk(&db, "fix-old", "src/d.rs", 1, 1);
        // fix-new touches b.rs whose candidate origin is dated AFTER the fix
        // itself — the clock-skew guard must discard it.
        seed_commit(&db, "fix-new");
        seed_hunk(&db, "fix-new", "src/b.rs", 1, 1);

        let repo = FakeRepo {
            blobs: HashMap::from([
                (
                    ("parent-old".to_string(), "src/a.rs".to_string()),
                    b"let x = 1;\n".to_vec(),
                ),
                (
                    ("parent-old".to_string(), "src/c.rs".to_string()),
                    b"// cosmetic comment\n".to_vec(),
                ),
                (
                    ("parent-new".to_string(), "src/b.rs".to_string()),
                    b"return compute();\n".to_vec(),
                ),
            ]),
        };

        let origin = FakeOrigin {
            table: HashMap::from([
                (
                    ("parent-old".to_string(), "src/a.rs".to_string()),
                    vec![(1, "A".to_string())],
                ),
                (
                    ("parent-old".to_string(), "src/c.rs".to_string()),
                    vec![(1, "B".to_string())],
                ),
                (
                    ("parent-new".to_string(), "src/b.rs".to_string()),
                    vec![(1, "future1".to_string())],
                ),
            ]),
            fail_for: HashSet::from([("parent-old".to_string(), "src/d.rs".to_string())]),
        };

        let commit_dates = HashMap::from([
            ("A".to_string(), "2026-01-01T00:00:00Z".to_string()),
            ("B".to_string(), "2026-01-15T00:00:00Z".to_string()),
            ("future1".to_string(), "2026-06-01T00:00:00Z".to_string()),
        ]);

        let fixes = vec![
            (
                "fix-old".to_string(),
                "parent-old".to_string(),
                "2026-03-01T00:00:00Z".to_string(),
            ),
            (
                "fix-new".to_string(),
                "parent-new".to_string(),
                "2026-01-01T00:00:00Z".to_string(),
            ),
        ];

        let (links, stats) =
            link_defects(&db, &repo, &origin, &fixes, &commit_dates).expect("link_defects");

        assert_eq!(
            links,
            vec![SzzLink {
                defect_rev: "A".to_string(),
                fix_rev: "fix-old".to_string(),
                path: "src/a.rs".to_string(),
            }],
            "only the older, non-cosmetic candidate must survive: {links:?}"
        );

        assert_eq!(stats.fixes_found, 2);
        assert_eq!(stats.links_found, 1);
        assert_eq!(
            stats.files_blamed, 4,
            "a.rs, c.rs, d.rs, b.rs each attempted once"
        );
        assert_eq!(stats.lines_considered, 4);
        assert_eq!(stats.lines_dropped_cosmetic, 1, "c.rs's comment-only line");
        assert_eq!(stats.blame_failures, 1, "d.rs's fake blame failure");
        assert_eq!(stats.pure_addition_fixes, 0);
    }

    /// Insert a `changes` row only (no hunk), with an explicit `change_type`
    /// and churn — enough for the commit-footprint tangled guard, which reads
    /// only the `changes` table. Requires [`seed_commit`] for `rev` first.
    fn seed_change(
        db: &FactsDb,
        rev: &str,
        path: &str,
        change_type: &str,
        loc_added: u32,
        loc_deleted: u32,
    ) {
        db.conn()
            .execute(
                "INSERT INTO changes (rev, path, change_type, loc_added, loc_deleted) \
                 VALUES (?, ?, ?, ?, ?)",
                duckdb::params![rev, path, change_type, loc_added, loc_deleted],
            )
            .expect("insert change");
    }

    /// A `changes` + `hunks` pair with an explicit `change_type`, so both the
    /// ghost guard (reads `change_type`) and `deleted_ranges` (reads `hunks`)
    /// see it. The generalisation of [`seed_hunk`], which hard-codes
    /// `'modified'`.
    fn seed_typed_hunk(
        db: &FactsDb,
        rev: &str,
        path: &str,
        change_type: &str,
        old_start: u32,
        old_lines: u32,
    ) {
        seed_change(db, rev, path, change_type, 0, old_lines);
        db.conn()
            .execute(
                "INSERT INTO hunks (rev, path, old_start, old_lines, new_start, new_lines) \
                 VALUES (?, ?, ?, ?, 1, 0)",
                duckdb::params![rev, path, old_start, old_lines],
            )
            .expect("insert hunk");
    }

    #[test]
    fn link_defects_excludes_tangled_fixes() {
        let db = FactsDb::new_in_memory().expect("in-memory db");

        // fix-wide touches 9 files (> TANGLED_MAX_FILES = 8); f0 alone carries
        // a hunk + an older origin that WOULD link absent the guard. The
        // whole fix is excluded before any blame.
        seed_commit(&db, "fix-wide");
        seed_typed_hunk(&db, "fix-wide", "src/f0.rs", "modified", 1, 1);
        for i in 1..9 {
            seed_change(&db, "fix-wide", &format!("src/f{i}.rs"), "modified", 1, 1);
        }
        // fix-heavy touches one file but changes 500 lines (> TANGLED_MAX_CHURN
        // = 400) — also excluded, and also given a would-link hunk + origin.
        seed_commit(&db, "fix-heavy");
        seed_change(&db, "fix-heavy", "src/big.rs", "modified", 500, 0);
        db.conn()
            .execute(
                "INSERT INTO hunks (rev, path, old_start, old_lines, new_start, new_lines) \
                 VALUES ('fix-heavy', 'src/big.rs', 1, 1, 1, 0)",
                [],
            )
            .expect("insert hunk");

        let repo = FakeRepo {
            blobs: HashMap::from([
                (
                    ("parent-wide".to_string(), "src/f0.rs".to_string()),
                    b"let x = 1;\n".to_vec(),
                ),
                (
                    ("parent-heavy".to_string(), "src/big.rs".to_string()),
                    b"let y = 2;\n".to_vec(),
                ),
            ]),
        };
        let origin = FakeOrigin {
            table: HashMap::from([
                (
                    ("parent-wide".to_string(), "src/f0.rs".to_string()),
                    vec![(1, "old1".to_string())],
                ),
                (
                    ("parent-heavy".to_string(), "src/big.rs".to_string()),
                    vec![(1, "old2".to_string())],
                ),
            ]),
            fail_for: HashSet::new(),
        };
        let commit_dates = HashMap::from([
            ("old1".to_string(), "2026-01-01T00:00:00Z".to_string()),
            ("old2".to_string(), "2026-01-01T00:00:00Z".to_string()),
        ]);
        let fixes = vec![
            (
                "fix-wide".to_string(),
                "parent-wide".to_string(),
                "2026-03-01T00:00:00Z".to_string(),
            ),
            (
                "fix-heavy".to_string(),
                "parent-heavy".to_string(),
                "2026-03-01T00:00:00Z".to_string(),
            ),
        ];

        let (links, stats) =
            link_defects(&db, &repo, &origin, &fixes, &commit_dates).expect("link_defects");

        assert!(
            links.is_empty(),
            "both fixes are tangled → no links despite would-link origins: {links:?}"
        );
        assert_eq!(stats.fixes_excluded_tangled, 2, "one wide + one heavy");
        assert_eq!(
            stats.files_blamed, 0,
            "tangled fixes are excluded before any blame"
        );
        assert_eq!(stats.ghost_files_skipped, 0);
        assert_eq!(stats.links_found, 0);
    }

    #[test]
    fn link_defects_skips_whole_file_deletion_ghosts() {
        let db = FactsDb::new_in_memory().expect("in-memory db");

        // fix-ghost removes src/gone.rs wholesale (change_type 'deleted') and
        // edits src/kept.rs in place. Both have an older origin that would
        // link; only the in-place edit must survive — the whole-file deletion
        // is a ghost whose removed lines can't embody the fix.
        seed_commit(&db, "fix-ghost");
        seed_typed_hunk(&db, "fix-ghost", "src/gone.rs", "deleted", 1, 1);
        seed_typed_hunk(&db, "fix-ghost", "src/kept.rs", "modified", 1, 1);

        let repo = FakeRepo {
            blobs: HashMap::from([
                (
                    ("parent-ghost".to_string(), "src/gone.rs".to_string()),
                    b"let removed = 1;\n".to_vec(),
                ),
                (
                    ("parent-ghost".to_string(), "src/kept.rs".to_string()),
                    b"let fixed = 2;\n".to_vec(),
                ),
            ]),
        };
        let origin = FakeOrigin {
            table: HashMap::from([
                (
                    ("parent-ghost".to_string(), "src/gone.rs".to_string()),
                    vec![(1, "ghost-origin".to_string())],
                ),
                (
                    ("parent-ghost".to_string(), "src/kept.rs".to_string()),
                    vec![(1, "kept-origin".to_string())],
                ),
            ]),
            fail_for: HashSet::new(),
        };
        let commit_dates = HashMap::from([
            (
                "ghost-origin".to_string(),
                "2026-01-01T00:00:00Z".to_string(),
            ),
            (
                "kept-origin".to_string(),
                "2026-01-01T00:00:00Z".to_string(),
            ),
        ]);
        let fixes = vec![(
            "fix-ghost".to_string(),
            "parent-ghost".to_string(),
            "2026-03-01T00:00:00Z".to_string(),
        )];

        let (links, stats) =
            link_defects(&db, &repo, &origin, &fixes, &commit_dates).expect("link_defects");

        assert_eq!(
            links,
            vec![SzzLink {
                defect_rev: "kept-origin".to_string(),
                fix_rev: "fix-ghost".to_string(),
                path: "src/kept.rs".to_string(),
            }],
            "only the in-place edit links; the whole-file deletion is skipped: {links:?}"
        );
        assert_eq!(stats.ghost_files_skipped, 1, "src/gone.rs skipped as ghost");
        assert_eq!(
            stats.files_blamed, 1,
            "only src/kept.rs is blamed; the ghost is skipped before blame"
        );
        assert_eq!(stats.fixes_excluded_tangled, 0);
    }

    #[test]
    fn link_defects_counts_pure_addition_fixes() {
        let db = FactsDb::new_in_memory().expect("in-memory db");
        db.conn()
            .execute(
                "INSERT INTO commits (rev, author_email, author_name, \
                 committer_email, canonical_author, date, committer_date, \
                 message, is_merge, parent_count) \
                 VALUES ('fix-empty', 'a@b.com', 'A', 'a@b.com', 'A', \
                         TIMESTAMPTZ '2026-01-01', TIMESTAMPTZ '2026-01-01', \
                         'fix: only additions', false, 1)",
                [],
            )
            .expect("insert commit");
        // No hunks row for this rev — a pure addition has nothing deleted.

        let repo = FakeRepo {
            blobs: HashMap::new(),
        };
        let origin = FakeOrigin {
            table: HashMap::new(),
            fail_for: HashSet::new(),
        };
        let fixes = vec![(
            "fix-empty".to_string(),
            "parent-empty".to_string(),
            "2026-01-02T00:00:00Z".to_string(),
        )];
        let (links, stats) = link_defects(&db, &repo, &origin, &fixes, &HashMap::new())
            .expect("link_defects on a pure-addition fix");
        assert!(links.is_empty());
        assert_eq!(stats.pure_addition_fixes, 1);
        assert_eq!(stats.fixes_found, 1);
    }
}

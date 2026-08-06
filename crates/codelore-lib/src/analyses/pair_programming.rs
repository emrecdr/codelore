//! `pair-programming` analysis.
//!
//! Counts commits that carry one or more `Co-Authored-By:` trailers
//! and emits per-pair pair-counts. The Co-Authored-By trailer was
//! formalised by GitHub circa 2017 and is now standard signal for
//! pair / mob sessions; `CodeLore`'s identity layer already parses it
//! for the AI-attribution bot detection. This analysis just
//! aggregates the pairs.
//!
//! Trailers are extracted via regex over the message column at
//! query time. Pre-extraction into a sibling table during ingest
//! (similar to the `ai_attribution` field) is a potential future
//! hardening; query-time extraction over O(N) commits is fast
//! enough on `DuckDB` for the row counts users hit.
//!
//! ## Note on bot filtering
//!
//! Ingest appends every commit to `commits` regardless of authorship —
//! bot classification lives per-alias on `author_aliases`, and the
//! SQL analyses exclude bots by joining through `HUMAN_ALIASES_CTE`.
//! This analysis reads `commits` directly, so the `is_bot` checks below
//! are the only thing keeping bots out of the pair counts, on both
//! sides: the commit's own author, and each `Co-Authored-By:` trailer
//! (Renovate has historically added itself as one). Removing either
//! check starts counting bots as pair participants.
//!
//! The author-side check tests the resolved canonical identity rather
//! than the raw alias. A `.mailmap` that merges a bot alias and a human
//! alias into one canonical therefore classifies the pair by whichever
//! identity the canonical carries — mailmap intent wins, which is the
//! behaviour we want here.

use std::collections::HashMap;

use crate::facts::FactsDb;
use crate::{Options, Result};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PairRow {
    /// Lexicographically smaller author of the pair (canonical
    /// ordering, mirrors the coupling analysis convention).
    pub author_a: String,
    /// Lexicographically larger author of the pair.
    pub author_b: String,
    /// Number of commits where both authors appear together via
    /// `Co-Authored-By:` trailers.
    pub pair_commits: u32,
}

/// Run the `pair-programming` analysis. Returns rows ranked by
/// pair-commit count DESC.
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on `DuckDB` errors.
#[tracing::instrument(name = "pair-programming", skip_all, fields(min_revs = opts.min_revs))]
pub fn run_pair_programming(db: &FactsDb, opts: &Options) -> Result<Vec<PairRow>> {
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);

    // Pull every commit's (author, message) so we can extract trailers
    // in Rust. Doing it at query time gives us a clean post-process
    // for canonical-ordering + bot filtering without writing regex
    // matchers in DuckDB.
    let mut stmt = db
        .conn()
        .prepare(
            "SELECT canonical_author, message FROM commits \
             WHERE is_merge = FALSE",
        )
        .map_err(|e| {
            crate::CodeLoreError::Analysis(format!("prepare pair-programming scan: {e}"))
        })?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| crate::CodeLoreError::Analysis(format!("query pair-programming scan: {e}")))?;

    // Bot patterns are merged with project-level `.codelorebots`
    // (when present) so the same filter the identity layer applies at
    // ingest also covers trailer-emitted co-authors.
    let bot_patterns = crate::identity::BotPatterns::from_repo(&opts.repo_path);

    // Intern author identities into a per-run lookup so the hot pair-
    // counting loop never allocates a `String` for known authors.
    // Each unique identity gets a stable `usize` id; the
    // `(idx_a, idx_b)` integer pair becomes the HashMap key. Pair
    // lookups are pure integer hashes — no String construction per
    // probe, no `to_string()` per inner-loop iteration.
    //
    // The prior shape allocated two `String`s per pair × commit (via
    // `pair_counts.entry((a.clone(), b.clone()))`) even when the pair
    // was already counted. On repos with heavy pair-programming
    // (~100 commits per pair) that was ~200 String allocs per pair
    // wasted to discover the pair was already present.
    let mut author_idx: HashMap<String, u32> = HashMap::new();
    let mut authors: Vec<String> = Vec::new();
    let mut intern = |id: String| -> u32 {
        if let Some(&idx) = author_idx.get(&id) {
            return idx;
        }
        let idx = u32::try_from(authors.len()).unwrap_or(u32::MAX);
        author_idx.insert(id.clone(), idx);
        authors.push(id);
        idx
    };
    let mut pair_counts: HashMap<(u32, u32), u32> = HashMap::new();
    // Per-commit participant scratch buffer, reused across the commit
    // loop. `clear()` keeps the allocation; we just truncate length.
    let mut participants_buf: Vec<u32> = Vec::with_capacity(8);
    for row in rows {
        let (primary, message) = row.map_err(|e| {
            crate::CodeLoreError::Analysis(format!("row pair-programming scan: {e}"))
        })?;
        let co_authors = extract_coauthors(&message);
        if co_authors.is_empty() {
            continue;
        }
        // Normalise the primary identity to lowercased email when
        // possible so it dedups against the lowercased trailer emails.
        // canonical_author is `Name <email>` (display form) — extract
        // the email if present, else lowercase the whole token.
        let primary_norm = normalise_identity(&primary);
        if bot_patterns.is_bot(&primary_norm, &primary) {
            continue;
        }
        // Intern each participant once into the per-run author table,
        // building the per-commit `participants_buf` as a `Vec<u32>`
        // of indices. Sort + dedup the indices so within each commit
        // the inner loop emits `(idx_a, idx_b)` with `idx_a < idx_b`.
        // Indices are stable across commits (interner is append-
        // only), so the same author pair always hashes to the same
        // `(min_idx, max_idx)` tuple regardless of which commit
        // surfaced them — guaranteeing pair counts dedup correctly.
        // The eventual string ordering (author_a < author_b
        // lexicographically) is recovered at output time below; the
        // interner's encounter-order indices don't preserve lex
        // order, which is why the per-row swap is necessary.
        participants_buf.clear();
        participants_buf.push(intern(primary_norm));
        for co in co_authors {
            if bot_patterns.is_bot(&co, &co) {
                continue;
            }
            participants_buf.push(intern(co));
        }
        participants_buf.sort_unstable();
        participants_buf.dedup();
        for (i, &a) in participants_buf.iter().enumerate() {
            for &b in &participants_buf[(i + 1)..] {
                // Pure integer-pair key — `entry((a, b))` does not
                // allocate (the tuple is two stack words; the
                // HashMap stores it inline).
                *pair_counts.entry((a, b)).or_insert(0) += 1;
            }
        }
    }

    // Sort by pair_commits DESC, then author_a / author_b for stable
    // tie-break. The string ordering at output time matches what the
    // prior `(String, String)` keyed map produced — author_idx
    // assigns indices in encounter order, but the final sort is on
    // the recovered string identities, not the indices.
    let mut out: Vec<PairRow> = pair_counts
        .into_iter()
        .map(|((a_idx, b_idx), n)| {
            let a_idx = a_idx as usize;
            let b_idx = b_idx as usize;
            // The interner enforces idx in encounter order; within a
            // single commit we sort the index buffer ascending, so
            // a_idx < b_idx, but globally `authors[a_idx]` may not be
            // lexicographically less than `authors[b_idx]`. Re-sort
            // the pair lexicographically for the canonical-ordering
            // contract this analysis's docstring promises.
            let (a, b) = if authors[a_idx] <= authors[b_idx] {
                (authors[a_idx].clone(), authors[b_idx].clone())
            } else {
                (authors[b_idx].clone(), authors[a_idx].clone())
            };
            PairRow {
                author_a: a,
                author_b: b,
                pair_commits: n,
            }
        })
        .collect();
    out.sort_by(|x, y| {
        y.pair_commits
            .cmp(&x.pair_commits)
            .then(x.author_a.cmp(&y.author_a))
            .then(x.author_b.cmp(&y.author_b))
    });

    // Respect rows-limit + opts thresholds. Drop pairs below the
    // min_shared_revs floor (reuses the coupling threshold so users
    // get one consistent gate across pair-style analyses).
    let min = opts.min_shared_revs;
    out.retain(|r| r.pair_commits >= min);
    if let Ok(lim) = usize::try_from(row_limit) {
        out.truncate(lim);
    }

    Ok(out)
}

/// Normalise an identity string to a stable lowercased token used as
/// the pair-counter map key. `canonical_author` is `Name <email>`
/// (display form); the email is the stable identity surface, so we
/// strip to it when present. Falls back to the lowercased input when
/// the angle-bracket envelope is missing.
fn normalise_identity(raw: &str) -> String {
    if let (Some(lt), Some(gt)) = (raw.find('<'), raw.find('>'))
        && lt < gt
    {
        let email = raw[(lt + 1)..gt].trim();
        if !email.is_empty() {
            return email.to_lowercase();
        }
    }
    raw.trim().to_lowercase()
}

/// Delegate to the shared trailer-extraction module so `Co-Authored-By:`
/// parsing lives in one place and is also available to the knowledge-shares
/// reviewer-credit step.
fn extract_coauthors(message: &str) -> Vec<String> {
    super::knowledge::trailers::extract_coauthors(message)
}

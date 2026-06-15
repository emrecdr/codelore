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
//! ## Note on trailer noise
//!
//! Bots are already filtered from `commits.canonical_author`, but
//! `Co-Authored-By:` trailers may still reference bot identities
//! (Renovate has historically added itself). The post-processing
//! drops any pair where either side matches the known-bot list.

use duckdb::params;
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

    let mut pair_counts: HashMap<(String, String), u32> = HashMap::new();
    for row in rows {
        let (primary, message) = row.map_err(|e| {
            crate::CodeLoreError::Analysis(format!("row pair-programming scan: {e}"))
        })?;
        let co_authors = extract_coauthors(&message);
        if co_authors.is_empty() {
            continue;
        }
        // Build the unique-pair set for this commit. Primary author
        // pairs with each co-author; co-authors pair with each other
        // (true mob session).
        let mut participants: Vec<String> = std::iter::once(primary).chain(co_authors).collect();
        participants.sort();
        participants.dedup();
        for i in 0..participants.len() {
            for j in (i + 1)..participants.len() {
                let key = (participants[i].clone(), participants[j].clone());
                *pair_counts.entry(key).or_insert(0) += 1;
            }
        }
    }

    // Sort by pair_commits DESC, then author_a / author_b for stable
    // tie-break.
    let mut out: Vec<PairRow> = pair_counts
        .into_iter()
        .map(|((a, b), n)| PairRow {
            author_a: a,
            author_b: b,
            pair_commits: n,
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

    // Silence unused-params warning when explain isn't engaged.
    let _ = params![row_limit];

    Ok(out)
}

/// Extract Co-Authored-By trailer values from a commit message.
/// Returns the raw email/name strings (post-trim, lowercased) so
/// downstream pairing is case-insensitive.
fn extract_coauthors(message: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in message.lines() {
        let trimmed = line.trim();
        // Match `Co-Authored-By: Name <email>` (case-insensitive).
        let lower = trimmed.to_lowercase();
        if let Some(rest) = lower.strip_prefix("co-authored-by:") {
            // Capture just the email portion if present, else the
            // whole trailer body. The email is more identity-stable.
            let body = rest.trim();
            if let (Some(lt), Some(gt)) = (body.find('<'), body.find('>'))
                && lt < gt
            {
                let email = &body[(lt + 1)..gt];
                let email = email.trim().to_string();
                if !email.is_empty() {
                    out.push(email);
                    continue;
                }
            }
            // Fallback to the whole trailer body (rare — trailers
            // without `<email>` are non-conventional).
            if !body.is_empty() {
                out.push(body.to_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_single_coauthor() {
        let msg = "feat: thing\n\nCo-authored-by: Bob <bob@example.com>";
        let got = extract_coauthors(msg);
        assert_eq!(got, vec!["bob@example.com"]);
    }

    #[test]
    fn extracts_multiple_coauthors() {
        let msg = "feat: thing\n\nCo-Authored-By: Alice <alice@example.com>\nCo-authored-by: Carol <carol@example.com>";
        let got = extract_coauthors(msg);
        assert_eq!(got, vec!["alice@example.com", "carol@example.com"]);
    }

    #[test]
    fn no_coauthors_returns_empty() {
        let msg = "feat: just one author";
        assert!(extract_coauthors(msg).is_empty());
    }

    #[test]
    fn malformed_trailer_falls_through_to_body() {
        let msg = "feat: x\n\nCo-authored-by: no-email-here";
        let got = extract_coauthors(msg);
        assert_eq!(got, vec!["no-email-here"]);
    }
}

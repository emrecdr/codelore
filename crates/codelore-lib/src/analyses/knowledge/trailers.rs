//! Git commit trailer extraction shared between the `pair-programming`
//! analysis and the decayed-knowledge reviewer-credit step.
//!
//! Extracts `Co-Authored-By:` and `Reviewed-By:` identities from a commit
//! message. Both trailer keys are treated identically downstream — they
//! each represent a developer who invested attention in a change, and
//! Rigby & Bird (ESEC/FSE 2013) show that reviewer knowledge carries
//! meaningful transfer (66–150% gain), motivating the shared extraction.

/// Extract `Co-Authored-By:` trailer values from a commit message.
///
/// Returns the email (lowercased) when the `Name <email>` form is present,
/// or the whole body (lowercased, trimmed) as a fallback for non-standard
/// trailers. The email form is preferred because it is more identity-stable
/// than display names.
///
/// Matching is case-insensitive on the trailer key; the returned strings
/// are always lowercased so callers can do case-insensitive dedup.
#[must_use]
pub fn extract_coauthors(message: &str) -> Vec<String> {
    extract_by_key(message, "co-authored-by:")
}

/// Extract `Reviewed-By:` trailer values from a commit message.
///
/// Same extraction rules as [`extract_coauthors`]. Used by the
/// decayed-knowledge reviewer-credit step per Jabrayilzade et al.,
/// ICSE-SEIP 2022 §3.1, where reviewer identity contributes `W_REVIEWER`
/// weight to knowledge share.
#[must_use]
pub fn extract_reviewers(message: &str) -> Vec<String> {
    extract_by_key(message, "reviewed-by:")
}

/// Common extraction kernel: scan `message` for lines whose lowercase form
/// starts with `key_lower` (a pre-lowercased trailer prefix like
/// `"co-authored-by:"`), then parse out the email or fall back to the
/// body.
fn extract_by_key(message: &str, key_lower: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in message.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_lowercase();
        if let Some(rest) = lower.strip_prefix(key_lower) {
            let body = rest.trim();
            if let (Some(lt), Some(gt)) = (body.find('<'), body.find('>'))
                && lt < gt
            {
                let email = body[(lt + 1)..gt].trim();
                if !email.is_empty() {
                    out.push(email.to_string());
                    continue;
                }
            }
            // Fallback: no `<email>` form — use the whole body (rare).
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
        assert_eq!(extract_coauthors(msg), vec!["bob@example.com"]);
    }

    #[test]
    fn extracts_multiple_coauthors() {
        let msg = "feat: thing\n\nCo-Authored-By: Alice <alice@example.com>\nCo-authored-by: Carol <carol@example.com>";
        assert_eq!(
            extract_coauthors(msg),
            vec!["alice@example.com", "carol@example.com"]
        );
    }

    #[test]
    fn no_coauthors_returns_empty() {
        assert!(extract_coauthors("feat: just one author").is_empty());
    }

    #[test]
    fn malformed_trailer_falls_through_to_body() {
        let msg = "feat: x\n\nCo-authored-by: no-email-here";
        assert_eq!(extract_coauthors(msg), vec!["no-email-here"]);
    }

    #[test]
    fn extracts_reviewed_by() {
        let msg = "fix: bug\n\nReviewed-By: Dave <dave@example.com>";
        assert_eq!(extract_reviewers(msg), vec!["dave@example.com"]);
    }

    #[test]
    fn reviewed_by_case_insensitive() {
        let msg = "fix: bug\n\nreviewed-by: Eve <eve@example.com>";
        assert_eq!(extract_reviewers(msg), vec!["eve@example.com"]);
    }
}

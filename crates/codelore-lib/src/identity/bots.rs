//! Bot pattern matching + AI attribution.
//!
//! Bot detection is plain substring containment (case-insensitive after a
//! single lowercasing pass on the inputs). AI-assistance detection is the
//! same idea: lowercase the commit message once, then check for any of the
//! published 2024+ assistant signatures.

/// Default bot identifiers (case-insensitive substring match in email or
/// name; the comparison lowercases both sides).
pub const DEFAULT_BOT_PATTERNS: &[&str] = &[
    "dependabot[bot]",
    "github-actions[bot]",
    "claude-code[bot]",
    "copilot[bot]",
    "renovate[bot]",
    "pre-commit-ci[bot]",
    // Devin's GitHub integration uses this bot email shape.
    "devin-ai-integration[bot]",
];

/// AI-assistance signatures in commit messages. Case-insensitive substring
/// match against the lowercased message. Covers the 2024-2026 lineup of
/// AI coding assistants that publish a Co-Authored-By trailer or in-message
/// tag.
///
/// Patterns are stored lowercased here; the matcher lowercases the message
/// once and uses plain `str::contains` (no regex crate needed).
const AI_ASSIST_PATTERNS: &[&str] = &[
    // Co-Authored-By trailers, lowercased
    "co-authored-by: claude",
    "co-authored-by: copilot",
    "co-authored-by: github copilot",
    "co-authored-by: cursor",
    "co-authored-by: sourcegraph cody",
    "co-authored-by: cody",
    "co-authored-by: continue",
    "co-authored-by: codeium",
    "co-authored-by: windsurf",
    "co-authored-by: devin",
    "co-authored-by: tabnine",
    "co-authored-by: amazon q",
    // Aider tags its message body rather than using a trailer.
    "(aider)",
];

/// True if the email or name matches any default bot pattern. Comparison
/// is case-insensitive — `Dependabot[Bot]@noreply.github.com` matches.
#[must_use]
pub fn is_bot(email: &str, name: &str) -> bool {
    let email_lc = email.to_lowercase();
    let name_lc = name.to_lowercase();
    DEFAULT_BOT_PATTERNS
        .iter()
        .any(|p| email_lc.contains(p) || name_lc.contains(p))
}

/// Classifies commit attribution as one of `"ai-authored"`, `"ai-assisted"`,
/// or `"human"`. Bot authors → `ai-authored`; commits with a recognized
/// AI-assistant signature in the message → `ai-assisted`; otherwise → `human`.
#[must_use]
pub fn ai_attribution(email: &str, name: &str, message: &str) -> &'static str {
    if is_bot(email, name) {
        return "ai-authored";
    }
    let msg_lc = message.to_lowercase();
    if AI_ASSIST_PATTERNS.iter().any(|p| msg_lc.contains(p)) {
        return "ai-assisted";
    }
    "human"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dependabot_is_bot() {
        assert!(is_bot(
            "dependabot[bot]@noreply.github.com",
            "dependabot[bot]"
        ));
    }

    #[test]
    fn human_is_not_bot() {
        assert!(!is_bot("alice@example.com", "Alice"));
    }

    #[test]
    fn bot_email_gives_ai_authored() {
        assert_eq!(
            ai_attribution(
                "dependabot[bot]@noreply.github.com",
                "dependabot[bot]",
                "bump deps"
            ),
            "ai-authored"
        );
    }

    #[test]
    fn co_authored_claude_gives_ai_assisted() {
        assert_eq!(
            ai_attribution(
                "alice@example.com",
                "Alice",
                "feat: do stuff\n\nCo-Authored-By: Claude"
            ),
            "ai-assisted"
        );
    }

    #[test]
    fn plain_human_commit_gives_human() {
        assert_eq!(
            ai_attribution("alice@example.com", "Alice", "fix typo"),
            "human"
        );
    }

    // Regression: bug-report finding — bot match used to be case-sensitive,
    // so `Dependabot[Bot]@noreply.github.com` (mixed-case from some GitHub
    // paths) didn't classify as a bot. Now case-insensitive.
    #[test]
    fn bot_match_is_case_insensitive() {
        assert!(is_bot("Dependabot[Bot]@noreply.github.com", "Dependabot[Bot]"));
        assert!(is_bot("GITHUB-ACTIONS[BOT]@example.com", "GitHub-Actions[Bot]"));
    }

    // Regression: 2024-2026 AI-coder lineup beyond Claude/Copilot. Each
    // assistant's published signature must classify as ai-assisted.
    #[test]
    fn detects_cursor_signature() {
        let msg = "feat: refactor auth\n\nCo-Authored-By: Cursor";
        assert_eq!(ai_attribution("alice@example.com", "Alice", msg), "ai-assisted");
    }

    #[test]
    fn detects_cody_signature() {
        let msg = "fix: handle null\n\nCo-Authored-By: Sourcegraph Cody";
        assert_eq!(ai_attribution("alice@example.com", "Alice", msg), "ai-assisted");
    }

    #[test]
    fn detects_aider_in_message_body() {
        // Aider doesn't use Co-Authored-By; it tags the message body itself.
        let msg = "refactor: extract helper (aider)";
        assert_eq!(ai_attribution("alice@example.com", "Alice", msg), "ai-assisted");
    }

    #[test]
    fn detects_continue_codeium_windsurf_tabnine_amazon_q() {
        for assistant in &["Continue", "Codeium", "Windsurf", "Tabnine", "Amazon Q"] {
            let msg = format!("feat: thing\n\nCo-Authored-By: {assistant}");
            assert_eq!(
                ai_attribution("alice@example.com", "Alice", &msg),
                "ai-assisted",
                "should detect {assistant} signature"
            );
        }
    }

    #[test]
    fn devin_bot_email_classifies_as_ai_authored() {
        // Devin's GitHub integration commits as devin-ai-integration[bot].
        assert_eq!(
            ai_attribution(
                "devin-ai-integration[bot]@users.noreply.github.com",
                "Devin",
                "implement feature"
            ),
            "ai-authored"
        );
    }

    #[test]
    fn co_authored_by_match_is_case_insensitive() {
        // Some tools / pipelines uppercase headers; some humans typo.
        let msg = "feat: x\n\nCO-AUTHORED-BY: cursor";
        assert_eq!(ai_attribution("alice@example.com", "Alice", msg), "ai-assisted");
    }
}

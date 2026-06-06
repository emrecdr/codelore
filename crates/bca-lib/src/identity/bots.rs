//! Bot pattern matching + AI attribution stub.

/// Default bot identifiers (substring match in email or name).
pub const DEFAULT_BOT_PATTERNS: &[&str] = &[
    "dependabot[bot]",
    "github-actions[bot]",
    "claude-code[bot]",
    "copilot[bot]",
    "renovate[bot]",
    "pre-commit-ci[bot]",
];

/// True if the email or name matches any default bot pattern.
#[must_use]
pub fn is_bot(email: &str, name: &str) -> bool {
    DEFAULT_BOT_PATTERNS
        .iter()
        .any(|p| email.contains(p) || name.contains(p))
}

/// Classifies commit attribution as one of "ai-authored", "ai-assisted",
/// or "human" based on author and message inspection.
#[must_use]
pub fn ai_attribution(email: &str, name: &str, message: &str) -> &'static str {
    if is_bot(email, name) {
        "ai-authored"
    } else if message.contains("Co-Authored-By: Claude")
        || message.contains("Co-Authored-By: Copilot")
        || message.contains("Co-Authored-By: GitHub Copilot")
    {
        "ai-assisted"
    } else {
        "human"
    }
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
}

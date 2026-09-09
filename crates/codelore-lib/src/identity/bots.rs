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
/// Assistant product names, matched against the NAME field of a
/// `Co-authored-by:` trailer and required to *be* that name rather than
/// merely begin it.
///
/// The distinction is the whole point. These were once matched as
/// `"co-authored-by: claude"` against the raw message, which reads
/// `Co-authored-by: Claude Dubois <claude.dubois@example.com>` as an
/// Anthropic trailer — and Claude, Devin and Cody are all ordinary given
/// names, so a team with one of them had every pair-authored commit counted
/// as AI-assisted. `commits.ai_attribution` feeds the dashboard's AI lens and
/// every AI split in the author-aggregating analyses, so the error was
/// invisible and load-bearing at once.
const AI_ASSIST_COAUTHORS: &[&str] = &[
    "claude",
    "copilot",
    "github copilot",
    "cursor",
    "sourcegraph cody",
    "cody",
    "continue",
    "codeium",
    "windsurf",
    "devin",
    "tabnine",
    "amazon q",
];

/// Words an assistant's trailer name may carry after its product name, as in
/// `Cursor Agent` or `Claude Code`. A remainder that is anything else — a
/// surname — means the trailer names a person.
const AI_AGENT_NAME_SUFFIXES: &[&str] = &["agent", "ai", "assistant", "bot", "code"];

/// AI signatures that are not trailers: Aider tags the message body instead.
/// These stay plain substring matches, since no person is called `(aider)`.
const AI_ASSIST_BODY_MARKERS: &[&str] = &["(aider)"];

/// True if the email or name matches any default bot pattern. Comparison
/// is case-insensitive — `Dependabot[Bot]@noreply.github.com` matches.
///
/// **For repos with internal bot accounts** (`our-deploy-bot@example.com`,
/// `release-bot`, etc.) that aren't covered by the default list, drop a
/// `.codelorebots` file at the repo root with one extra pattern per line;
/// use [`BotPatterns::from_repo`] to merge it with the defaults and pass
/// the result to [`BotPatterns::is_bot`] instead of this free function.
/// The free function is the zero-config path that the ingest pipeline
/// uses today.
#[must_use]
pub fn is_bot(email: &str, name: &str) -> bool {
    DEFAULT_BOT_PATTERNS
        .iter()
        .any(|p| contains_ignore_ascii_case(email, p) || contains_ignore_ascii_case(name, p))
}

/// ASCII-case-insensitive substring search that allocates nothing. Bot
/// patterns are ASCII tokens (`[bot]` suffixes, CI-vendor names, email
/// domains), so ASCII case-folding matches exactly the substrings a
/// `to_lowercase().contains()` would over ASCII patterns — but without the
/// two heap allocations the lowercasing pass cost on every commit.
fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    let (hay, ndl) = (haystack.as_bytes(), needle.as_bytes());
    if ndl.is_empty() {
        return true;
    }
    if ndl.len() > hay.len() {
        return false;
    }
    hay.windows(ndl.len()).any(|w| w.eq_ignore_ascii_case(ndl))
}

/// ASCII-case-insensitive prefix strip, for reading a git trailer key off a
/// line whose capitalisation is not standardised (`Co-authored-by:`,
/// `Co-Authored-By:` and `CO-AUTHORED-BY:` all occur in the wild).
fn strip_prefix_ignore_ascii_case<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    let (head, rest) = line.split_at_checked(prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then_some(rest)
}

/// The `(name, email)` of every `Co-authored-by:` trailer in `message`.
///
/// `Co-authored-by: Claude <noreply@anthropic.com>` yields
/// `("Claude", "noreply@anthropic.com")`; a trailer with no angle-bracketed
/// address yields an empty email, which is the shape git accepts and several
/// tools emit.
fn co_authored_identities(message: &str) -> impl Iterator<Item = (&str, &str)> {
    message.lines().filter_map(|line| {
        let rest = strip_prefix_ignore_ascii_case(line.trim(), "co-authored-by:")?;
        let (name, email) = rest.split_once('<').map_or((rest, ""), |(n, e)| {
            (n, e.split_once('>').map_or(e, |(addr, _)| addr))
        });
        Some((name.trim(), email.trim()))
    })
}

/// True when `name_lc` (ASCII-lowercased, trimmed) is an assistant's own name
/// rather than a person's name that happens to begin with one.
fn is_assistant_name(name_lc: &str) -> bool {
    AI_ASSIST_COAUTHORS.iter().any(|token| {
        name_lc.strip_prefix(token).is_some_and(|rest| {
            rest.is_empty()
                || (rest.starts_with(' ')
                    && rest
                        .split_whitespace()
                        .all(|word| AI_AGENT_NAME_SUFFIXES.contains(&word)))
        })
    })
}

/// True when `message` carries an assistant's signature: a body marker, or a
/// `Co-authored-by:` trailer naming an assistant.
///
/// `is_bot` is applied to each trailer identity as well, which is what keeps
/// bot-account co-authors (`devin-ai-integration[bot]`) detected now that the
/// name match is anchored — those names carry the product token as a prefix of
/// a longer machine name, exactly the shape the anchoring rejects.
fn message_signals_ai_assist<F>(message: &str, is_bot_identity: F) -> bool
where
    F: Fn(&str, &str) -> bool,
{
    if AI_ASSIST_BODY_MARKERS
        .iter()
        .any(|marker| contains_ignore_ascii_case(message, marker))
    {
        return true;
    }
    co_authored_identities(message).any(|(name, email)| {
        is_bot_identity(email, name) || is_assistant_name(&name.to_ascii_lowercase())
    })
}

/// User-extensible bot-pattern set. Merges built-in [`DEFAULT_BOT_PATTERNS`]
/// with any patterns from a project-level `.codelorebots` file at the repo
/// root. File format mirrors `.codeloreignore`:
///
/// ```text
/// # Comments start with `#`
/// # One bot pattern per line; substring match in email or name
/// # (case-insensitive after lowercasing both sides).
/// our-deploy-bot
/// release-automation
/// ```
///
/// Blank lines and `#`-prefix comments are ignored.
#[derive(Debug, Default, Clone)]
pub struct BotPatterns {
    /// User patterns from `.codelorebots`, already lowercased.
    /// `DEFAULT_BOT_PATTERNS` is consulted separately so the user list is
    /// purely additive (the defaults can never be turned off — preserving
    /// the project invariant that GitHub-published bots always classify
    /// as bots).
    user_patterns: Vec<String>,
}

impl BotPatterns {
    /// Read `<repo_root>/.codelorebots` if present, returning a [`BotPatterns`]
    /// with the user additions parsed. Missing file → empty user set
    /// (defaults still applied via [`Self::is_bot`]).
    ///
    /// I/O errors other than `NotFound` are logged at warn level and a
    /// default [`BotPatterns`] is returned — bot detection is best-effort.
    #[must_use]
    pub fn from_repo(repo_root: &std::path::Path) -> Self {
        let path = repo_root.join(".codelorebots");
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::from_text(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => {
                tracing::warn!(
                    "failed to read .codelorebots at {}: {e}; using defaults only",
                    path.display()
                );
                Self::default()
            }
        }
    }

    /// Parse the `.codelorebots` text format.
    #[must_use]
    pub fn from_text(text: &str) -> Self {
        let user_patterns: Vec<String> = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(str::to_lowercase)
            .collect();
        Self { user_patterns }
    }

    /// Case-insensitive substring match across both the built-in defaults
    /// and any user-additions from `.codelorebots`.
    #[must_use]
    pub fn is_bot(&self, email: &str, name: &str) -> bool {
        let matches =
            |p: &str| contains_ignore_ascii_case(email, p) || contains_ignore_ascii_case(name, p);
        // Defaults are always checked first — user file can't turn them off.
        DEFAULT_BOT_PATTERNS.iter().any(|p| matches(p))
            || self.user_patterns.iter().any(|p| matches(p))
    }
}

/// Classifies commit attribution as one of `"ai-authored"`, `"ai-assisted"`,
/// or `"human"`. Bot authors → `ai-authored`; commits with a recognized
/// AI-assistant signature in the message → `ai-assisted`; otherwise → `human`.
///
/// Defaults-only path; user-extensible patterns from `.codelorebots` are
/// honoured by [`ai_attribution_with`] (the production ingest pipeline
/// uses that variant).
#[must_use]
pub fn ai_attribution(email: &str, name: &str, message: &str) -> &'static str {
    if is_bot(email, name) {
        return "ai-authored";
    }
    if message_signals_ai_assist(message, is_bot) {
        return "ai-assisted";
    }
    "human"
}

/// User-extensible variant of [`ai_attribution`]: routes the bot check
/// through a [`BotPatterns`] instance so a project-level `.codelorebots`
/// file participates — for the commit's own author, and for the identity in
/// each `Co-authored-by:` trailer. The assistant *product* names remain
/// built-in and not user-extensible; a project's own AI account is expressed
/// as a bot pattern, which is the mechanism that already exists for it.
#[must_use]
pub fn ai_attribution_with(
    patterns: &BotPatterns,
    email: &str,
    name: &str,
    message: &str,
) -> &'static str {
    if patterns.is_bot(email, name) {
        return "ai-authored";
    }
    if message_signals_ai_assist(message, |e, n| patterns.is_bot(e, n)) {
        return "ai-assisted";
    }
    "human"
}

#[cfg(all(test, feature = "test-support"))]
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

    /// The reason the trailer match is anchored on the whole name. Claude,
    /// Devin and Cody are ordinary given names, and a prefix match read every
    /// commit they co-authored as AI-assisted.
    #[test]
    fn a_human_co_author_named_after_an_assistant_stays_human() {
        for (name, email) in [
            ("Claude Dubois", "claude.dubois@example.com"),
            ("Devin Smith", "devin@example.com"),
            ("Cody Fisher", "cody.fisher@example.com"),
            ("Claudia Rossi", "claudia@example.com"),
        ] {
            let message = format!("feat: pair on the parser\n\nCo-authored-by: {name} <{email}>");
            assert_eq!(
                ai_attribution("alice@example.com", "Alice", &message),
                "human",
                "{name} is a person, not an assistant"
            );
        }
    }

    /// The control for the test above: the assistants themselves must still
    /// be detected, in the trailer shapes they actually publish — the product
    /// name alone, the product name with an agent suffix, and the bot account
    /// whose longer machine name only the bot patterns can recognise.
    #[test]
    fn assistant_co_author_trailers_are_still_detected() {
        for trailer in [
            "Co-Authored-By: Claude",
            "Co-authored-by: Claude <noreply@anthropic.com>",
            "Co-authored-by: Claude Code <noreply@anthropic.com>",
            "Co-authored-by: Copilot <198982749+Copilot@users.noreply.github.com>",
            "Co-authored-by: Cursor Agent <cursoragent@cursor.com>",
            "CO-AUTHORED-BY: Amazon Q <q@amazon.com>",
            "Co-authored-by: devin-ai-integration[bot] <devin@example.com>",
        ] {
            let message = format!("feat: do stuff\n\n{trailer}");
            assert_eq!(
                ai_attribution("alice@example.com", "Alice", &message),
                "ai-assisted",
                "trailer must still be detected: {trailer}"
            );
        }
    }

    /// Aider tags the body rather than adding a trailer, so it stays a plain
    /// substring match and must keep working.
    #[test]
    fn aider_body_marker_is_still_detected() {
        assert_eq!(
            ai_attribution("alice@example.com", "Alice", "refactor the loop (aider)"),
            "ai-assisted"
        );
    }

    /// An assistant's name in prose is not a trailer. The old matcher keyed
    /// on the whole message, so quoting the trailer — in a revert, a commit
    /// that documents the convention, or a merge of such a commit — counted.
    #[test]
    fn an_assistant_name_outside_a_trailer_is_not_a_signature() {
        assert_eq!(
            ai_attribution(
                "alice@example.com",
                "Alice",
                "docs: explain that we no longer add Co-authored-by: Claude trailers"
            ),
            "human",
            "the trailer must be a trailer, not a mention inside a sentence"
        );
    }

    /// `.codelorebots` reaches co-author identities too, so a project's own
    /// AI account is classified by the mechanism that already exists for it.
    #[test]
    fn project_bot_patterns_reach_co_author_trailers() {
        let patterns = BotPatterns::from_text("our-ai-helper\n");
        assert_eq!(
            ai_attribution_with(
                &patterns,
                "alice@example.com",
                "Alice",
                "feat: x\n\nCo-authored-by: our-ai-helper <ai@example.com>"
            ),
            "ai-assisted"
        );
    }

    // Regression: bug-report finding — bot match used to be case-sensitive,
    // so `Dependabot[Bot]@noreply.github.com` (mixed-case from some GitHub
    // paths) didn't classify as a bot. Now case-insensitive.
    #[test]
    fn bot_match_is_case_insensitive() {
        assert!(is_bot(
            "Dependabot[Bot]@noreply.github.com",
            "Dependabot[Bot]"
        ));
        assert!(is_bot(
            "GITHUB-ACTIONS[BOT]@example.com",
            "GitHub-Actions[Bot]"
        ));
    }

    // Regression: 2024-2026 AI-coder lineup beyond Claude/Copilot. Each
    // assistant's published signature must classify as ai-assisted.
    #[test]
    fn detects_cursor_signature() {
        let msg = "feat: refactor auth\n\nCo-Authored-By: Cursor";
        assert_eq!(
            ai_attribution("alice@example.com", "Alice", msg),
            "ai-assisted"
        );
    }

    #[test]
    fn detects_cody_signature() {
        let msg = "fix: handle null\n\nCo-Authored-By: Sourcegraph Cody";
        assert_eq!(
            ai_attribution("alice@example.com", "Alice", msg),
            "ai-assisted"
        );
    }

    #[test]
    fn detects_aider_in_message_body() {
        // Aider doesn't use Co-Authored-By; it tags the message body itself.
        let msg = "refactor: extract helper (aider)";
        assert_eq!(
            ai_attribution("alice@example.com", "Alice", msg),
            "ai-assisted"
        );
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
        assert_eq!(
            ai_attribution("alice@example.com", "Alice", msg),
            "ai-assisted"
        );
    }

    // BotPatterns: user-extensible bot set via .codelorebots file.
    #[test]
    fn bot_patterns_default_matches_built_in_defaults() {
        let patterns = BotPatterns::default();
        assert!(patterns.is_bot("dependabot[bot]@noreply.github.com", "dependabot[bot]"));
        assert!(!patterns.is_bot("alice@example.com", "Alice"));
    }

    #[test]
    fn bot_patterns_user_additions_classify_as_bots() {
        let patterns = BotPatterns::from_text(
            "# our internal deploy account\n\
             our-deploy-bot\n\
             \n\
             # release automation\n\
             release-automation\n",
        );
        assert!(patterns.is_bot("our-deploy-bot@example.com", "Deploy Bot"));
        assert!(patterns.is_bot("ci@example.com", "release-automation"));
        // Defaults are still applied
        assert!(patterns.is_bot("dependabot[bot]@noreply.github.com", "dependabot[bot]"));
        // Non-matching is still not a bot
        assert!(!patterns.is_bot("alice@example.com", "Alice"));
    }

    #[test]
    fn bot_patterns_user_additions_case_insensitive() {
        let patterns = BotPatterns::from_text("OUR-DEPLOY-BOT\n");
        assert!(patterns.is_bot("Our-Deploy-Bot@example.com", "Deploy Bot"));
    }

    #[test]
    fn bot_patterns_blank_lines_and_comments_ignored() {
        let patterns = BotPatterns::from_text("\n\n# only comments here\n# and another\n\n");
        // No user patterns → behaves like Default
        assert!(!patterns.is_bot("alice@example.com", "Alice"));
        // But defaults still apply
        assert!(patterns.is_bot("dependabot[bot]@x.com", "x"));
    }

    #[test]
    fn bot_patterns_from_missing_repo_file_returns_default() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let patterns = BotPatterns::from_repo(tmp.path());
        // No .codelorebots file → defaults only
        assert!(patterns.is_bot("dependabot[bot]@x.com", "x"));
        assert!(!patterns.is_bot("custom-bot@example.com", "custom-bot"));
    }

    #[test]
    fn bot_patterns_from_repo_reads_codelorebots() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join(".codelorebots"), "custom-bot\n").expect("write");
        let patterns = BotPatterns::from_repo(tmp.path());
        assert!(patterns.is_bot("custom-bot@example.com", "Custom Bot"));
    }
}

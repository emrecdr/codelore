//! The `narrate` orchestrator: the one call that turns a fact sheet into a
//! grounded advisory narrative.
//!
//! It ties together the pieces the rest of the layer builds: it derives the
//! sidecar cache key from the fact-sheet text and the client's model, serves a
//! warm cache without touching the model, and on a miss drives the versioned
//! prompt through the [`ChatClient`], runs the numeric citation check over the
//! reply, records the result in the cache, and returns it. The groundedness
//! verdict is surfaced verbatim so a caller can stamp every narrative
//! `grounded ✓` or `⚠ contains uncited claims`.

use std::path::Path;

use sha2::{Digest, Sha256};

use super::SCHEMA_VERSION;
use super::cache::{self, CachedNarrative};
use super::citation::check_citations;
use super::client::ChatClient;
use super::prompt::{self, Lens, PROMPT_VERSION};
use crate::Result;
use crate::quality_gates::ledger::now_utc_ts;

/// The outcome of a [`narrate`] call: the narrative text, its groundedness
/// verdict and unmatched tokens, the model that produced it, and whether it was
/// served from the sidecar cache rather than freshly generated.
#[derive(Debug, Clone)]
pub struct NarrativeResult {
    /// The narrative text.
    pub narrative: String,
    /// Whether every number the narrative quoted was grounded in the fact sheet.
    pub grounded: bool,
    /// The numeric tokens that matched no fact value (empty iff `grounded`).
    pub unmatched: Vec<String>,
    /// The model id that produced (or previously produced) the narrative.
    pub model: String,
    /// Whether this result came from the cache rather than a fresh completion.
    pub from_cache: bool,
}

/// The fact-sheet evidence a narration is grounded in: the canonical `text` the
/// model reads (and keys the cache on) and the numeric `values` the reply's
/// citations are checked against. Both are derived together from one fact sheet,
/// so they travel together into [`narrate`].
#[derive(Debug, Clone, Copy)]
pub struct SheetFacts<'a> {
    /// The canonical fact-sheet text — the model's input and the cache key input.
    pub text: &'a str,
    /// Every parseable numeric value on the sheet, for the citation check.
    pub values: &'a [f64],
}

/// Produce a grounded advisory narrative for `facts` under `lens`, consulting —
/// and populating — the sidecar cache.
///
/// `subject` is the stable label the narrative describes — the repo-relative
/// file path for [`Lens::FileDiagnosis`], a caller-chosen stable label for other
/// lenses. It is stored on the written entry so a later staleness check can be
/// scoped to this subject's own narratives rather than any sibling's.
///
/// The cache key is derived from `facts.text` and the client's model. Unless
/// `refresh` is set, a cache hit returns immediately with `from_cache = true`
/// and the model is never contacted. On a miss (or a forced `refresh`) the
/// client completes the lens's system/user prompt, the numbers in the reply are
/// checked against `facts.values`, the result is written to the cache, and it is
/// returned with `from_cache = false`.
///
/// # Errors
///
/// Propagates any [`CodeLoreError`](crate::CodeLoreError) the chat client
/// returns. Cache reads and writes are best-effort and never fail the call.
pub fn narrate(
    client: &dyn ChatClient,
    lens: Lens,
    subject: &str,
    facts: SheetFacts<'_>,
    cache_root: &Path,
    repo_path: &Path,
    refresh: bool,
) -> Result<NarrativeResult> {
    let model = client.model_id().to_string();
    let key = cache::cache_key(facts.text, &model);

    if !refresh && let Some(hit) = cache::read(cache_root, repo_path, &key) {
        // The key covers the narrative text — same fact-sheet text, prompt, and
        // model — but the stored verdict was computed by whatever binary wrote
        // the entry. Recompute it from the cached narrative so an improved
        // citation checker reaches warm caches; the hit's key-covered facts are
        // the same `facts.values`, and the model is still never contacted.
        let groundedness = check_citations(&hit.narrative, facts.values);
        return Ok(NarrativeResult {
            narrative: hit.narrative,
            grounded: groundedness.grounded,
            unmatched: groundedness.unmatched,
            model: hit.model,
            from_cache: true,
        });
    }

    let narrative = client.complete(
        prompt::system_prompt(lens),
        &prompt::user_prompt(lens, facts.text),
    )?;
    let groundedness = check_citations(&narrative, facts.values);

    let entry = CachedNarrative {
        narrative: narrative.clone(),
        subject: subject.to_string(),
        grounded: groundedness.grounded,
        unmatched: groundedness.unmatched.clone(),
        model: model.clone(),
        prompt_version: PROMPT_VERSION,
        schema_version: SCHEMA_VERSION,
        fact_digest: fact_digest(facts.text),
        created_at: now_utc_ts(),
    };
    cache::write(cache_root, repo_path, &key, &entry);

    Ok(NarrativeResult {
        narrative,
        grounded: groundedness.grounded,
        unmatched: groundedness.unmatched,
        model,
        from_cache: false,
    })
}

/// Uncited tokens named in the stamp before the list is truncated with a
/// `(+n more)` suffix.
const STAMP_UNCITED_PREVIEW: usize = 5;

/// The inline advisory stamp for `result`: it names the model and states whether
/// every number the narrative quoted was grounded in the fact sheet, naming the
/// uncited tokens (up to [`STAMP_UNCITED_PREVIEW`]) when it was not.
#[must_use]
pub fn stamp(result: &NarrativeResult) -> String {
    let model = &result.model;
    if result.grounded {
        format!("advisory — model {model}, grounded ✓")
    } else {
        let unmatched = &result.unmatched;
        let list = unmatched
            .iter()
            .take(STAMP_UNCITED_PREVIEW)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let more = unmatched.len().saturating_sub(STAMP_UNCITED_PREVIEW);
        if more > 0 {
            format!("advisory — model {model}, ⚠ contains uncited claims: {list} (+{more} more)")
        } else {
            format!("advisory — model {model}, ⚠ contains uncited claims: {list}")
        }
    }
}

/// Lowercase-hex SHA-256 of the fact-sheet text — the same digest
/// `fact_sheet.rs` records on a sheet, recomputed here because `narrate`
/// receives the canonical text rather than the sheet object that carries
/// `digest()`. Storing it lets a caller detect a stale narrative by comparing it
/// against a freshly built sheet's digest.
fn fact_digest(fact_sheet_text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(fact_sheet_text.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::{NarrativeResult, SheetFacts, narrate, stamp};
    use crate::Result;
    use crate::enrichment::client::ChatClient;
    use crate::enrichment::prompt::Lens;
    use std::cell::Cell;

    /// A chat client with a canned reply that counts completions, so a test can
    /// prove a cache hit never reached the model.
    struct MockChatClient {
        reply: String,
        model: String,
        calls: Cell<usize>,
    }

    impl ChatClient for MockChatClient {
        fn complete(&self, _system: &str, _user: &str) -> Result<String> {
            self.calls.set(self.calls.get() + 1);
            Ok(self.reply.clone())
        }

        fn model_id(&self) -> &str {
            &self.model
        }
    }

    fn result_with(grounded: bool) -> NarrativeResult {
        NarrativeResult {
            narrative: "text".to_string(),
            grounded,
            unmatched: if grounded {
                Vec::new()
            } else {
                vec!["4200".to_string()]
            },
            model: "mock-model".to_string(),
            from_cache: false,
        }
    }

    #[test]
    fn stamp_renders_the_grounded_verdict() {
        let s = stamp(&result_with(true));
        assert!(s.contains("mock-model"), "stamp names the model: {s}");
        assert!(s.contains("grounded ✓"), "grounded stamp: {s}");
        assert!(
            !s.contains("uncited"),
            "grounded stamp omits the warning: {s}"
        );
    }

    #[test]
    fn stamp_renders_the_uncited_verdict() {
        let mut result = result_with(false);
        result.unmatched = vec!["42.5%".to_string(), "-0.5".to_string()];
        let s = stamp(&result);
        assert!(s.contains("mock-model"), "stamp names the model: {s}");
        assert!(
            s.contains("⚠ contains uncited claims: 42.5%, -0.5"),
            "uncited stamp names the tokens: {s}"
        );
    }

    #[test]
    fn stamp_truncates_the_uncited_list_after_five() {
        let mut result = result_with(false);
        result.unmatched = vec![
            "1".to_string(),
            "2".to_string(),
            "3".to_string(),
            "4".to_string(),
            "5".to_string(),
            "6".to_string(),
            "7".to_string(),
        ];
        let s = stamp(&result);
        assert!(s.contains("(+2 more)"), "truncated stamp: {s}");
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn narrate_misses_then_hits_then_refresh_regenerates() {
        let cache_root = tempfile::tempdir().expect("cache root");
        let repo = std::path::Path::new("/tmp/repo");
        let sheet = "code-health\n  score = 87.5\n";
        let values = [87.5];
        let facts = SheetFacts {
            text: sheet,
            values: &values,
        };

        let client = MockChatClient {
            reply: "Diagnosis: the score is 87.5.".to_string(),
            model: "mock-model".to_string(),
            calls: Cell::new(0),
        };

        let first = narrate(
            &client,
            Lens::FileDiagnosis,
            "src/subject.rs",
            facts,
            cache_root.path(),
            repo,
            false,
        )
        .expect("first narrate");
        assert!(!first.from_cache, "a cold cache misses");
        assert!(first.grounded, "87.5 is grounded by the fact value");
        assert_eq!(client.calls.get(), 1);

        let second = narrate(
            &client,
            Lens::FileDiagnosis,
            "src/subject.rs",
            facts,
            cache_root.path(),
            repo,
            false,
        )
        .expect("second narrate");
        assert!(second.from_cache, "a warm cache hits");
        assert_eq!(second.narrative, first.narrative);
        assert_eq!(client.calls.get(), 1, "a hit does not reach the model");

        let refreshed = narrate(
            &client,
            Lens::FileDiagnosis,
            "src/subject.rs",
            facts,
            cache_root.path(),
            repo,
            true,
        )
        .expect("refresh narrate");
        assert!(!refreshed.from_cache, "refresh bypasses the cache");
        assert_eq!(client.calls.get(), 2, "refresh reaches the model again");
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn cache_hit_serves_a_freshly_recomputed_citation_verdict() {
        use super::cache::{self, CachedNarrative};
        use crate::enrichment::SCHEMA_VERSION;
        use crate::enrichment::prompt::PROMPT_VERSION;

        let cache_root = tempfile::tempdir().expect("cache root");
        let repo = std::path::Path::new("/tmp/repo");
        let sheet = "code-health\n  score = 87.5\n";
        let values = [87.5];
        let facts = SheetFacts {
            text: sheet,
            values: &values,
        };
        let model = "mock-model";

        // Hand-write an entry whose stored verdict is a lie under the current
        // checker: the narrative quotes a signed -0.5 that no fact value grounds,
        // yet the entry claims grounded with an empty unmatched list — exactly
        // what an older, sign-blind binary would have recorded. `narrate` must
        // ignore the stored verdict and recompute it from the narrative.
        let key = cache::cache_key(sheet, model);
        let entry = CachedNarrative {
            narrative: "Diagnosis: a delta of -0.5 appears.".to_string(),
            subject: "src/subject.rs".to_string(),
            grounded: true,
            unmatched: Vec::new(),
            model: model.to_string(),
            prompt_version: PROMPT_VERSION,
            schema_version: SCHEMA_VERSION,
            fact_digest: "stale-digest".to_string(),
            created_at: "2025-01-01T00:00:00Z".to_string(),
        };
        cache::write(cache_root.path(), repo, &key, &entry);

        let client = MockChatClient {
            reply: "unused — this call must hit the cache".to_string(),
            model: model.to_string(),
            calls: Cell::new(0),
        };

        let hit = narrate(
            &client,
            Lens::FileDiagnosis,
            "src/subject.rs",
            facts,
            cache_root.path(),
            repo,
            false,
        )
        .expect("narrate hits the hand-written entry");

        assert!(hit.from_cache, "the entry is served from the cache");
        assert_eq!(client.calls.get(), 0, "a hit never reaches the model");
        assert!(
            !hit.grounded,
            "the recomputed verdict catches the ungrounded -0.5 rather than \
             trusting the stored grounded:true"
        );
        assert_eq!(
            hit.unmatched,
            vec!["-0.5".to_string()],
            "the recomputed unmatched list names the signed token"
        );
    }
}

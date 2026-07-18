//! Sidecar narrative cache for the advisory enrichment layer.
//!
//! A generated narrative is expensive (a model round-trip) and deterministic in
//! its inputs, so each one is persisted next to the fact-store cache as a small
//! JSON sidecar. The cache key is the SHA-256 of the fact-sheet text joined with
//! the schema version, the prompt version, and the model id: change any of them
//! and the key changes, so the cached narrative text is never stale for new
//! evidence, a re-worded prompt, or a different model. Its stored groundedness
//! verdict is a separate matter — a warm read recomputes it from the cached
//! narrative (see `engine::narrate`), so an improved citation checker reaches
//! warm caches without a regeneration.
//!
//! The key is rendered as plain lowercase hex — no `sha256:` prefix — because it
//! becomes a filename, and a `:` is not a legal path component on Windows. The
//! cache is strictly best-effort: a missing entry is an ordinary miss, a corrupt
//! entry is logged and treated as a miss, and a failed write is logged and
//! swallowed. Nothing here can fail a run.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::SCHEMA_VERSION;
use super::prompt::PROMPT_VERSION;
use crate::cache::repo_cache_dir;

/// A cached advisory narrative plus the provenance a caller needs to decide
/// whether it still describes the current evidence.
///
/// `subject` is the stable label the narrative describes — the repo-relative
/// file path for a file diagnosis — so a staleness check compares only against
/// the subject's own narratives, never a sibling's. `fact_digest` is the digest
/// of the fact-sheet text the narrative was written from; comparing it against a
/// freshly built sheet's digest is how a caller detects that a cached narrative
/// has gone stale. `created_at` is an ISO-8601 UTC timestamp, so a lexical
/// ordering of two entries is a chronological one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedNarrative {
    /// The narrative text as the model produced it.
    pub narrative: String,
    /// The subject the narrative describes — the repo-relative file path for a
    /// file diagnosis, or a caller-chosen stable label for other lenses. Older
    /// sidecars written before this field existed deserialize as the empty
    /// string, which matches no real subject and so is simply never served.
    #[serde(default)]
    pub subject: String,
    /// The generation-time groundedness verdict: whether every number the
    /// narrative quoted was grounded in the fact sheet when it was written.
    /// `narrate` recomputes the verdict from the cached narrative on every read,
    /// so a checker improvement applies to warm caches; this field records what
    /// the writing binary saw.
    pub grounded: bool,
    /// The numeric tokens that matched no fact value at generation time (empty
    /// iff `grounded`). Like `grounded`, this is the writing binary's verdict;
    /// `narrate` recomputes it from the cached narrative on read.
    pub unmatched: Vec<String>,
    /// The model id that produced the narrative.
    pub model: String,
    /// The prompt-template version in force when it was generated.
    pub prompt_version: u32,
    /// The fact-sheet schema version in force when it was generated.
    pub schema_version: u32,
    /// Lowercase-hex SHA-256 of the fact-sheet text the narrative describes.
    pub fact_digest: String,
    /// ISO-8601 UTC timestamp of when the narrative was cached.
    pub created_at: String,
}

/// The cache key for a narrative: lowercase-hex SHA-256 of
/// `"{fact_sheet_text}|{SCHEMA_VERSION}|{PROMPT_VERSION}|{model}"`.
///
/// Bumping either version constant, editing the fact sheet, or pointing at a new
/// model all change the key, so a cached narrative is invalidated exactly when
/// one of its inputs moves. The digest is plain hex (a valid filename on every
/// platform) rather than the `sha256:`-prefixed form.
#[must_use]
pub fn cache_key(fact_sheet_text: &str, model: &str) -> String {
    let material = format!("{fact_sheet_text}|{SCHEMA_VERSION}|{PROMPT_VERSION}|{model}");
    let mut hasher = Sha256::new();
    hasher.update(material.as_bytes());
    hex::encode(hasher.finalize())
}

/// The on-disk path for the narrative keyed by `key`:
/// `repo_cache_dir(cache_root, repo_path)/enrichment/<key>.json`.
#[must_use]
pub fn cache_path(cache_root: &Path, repo_path: &Path, key: &str) -> PathBuf {
    enrichment_dir(cache_root, repo_path).join(format!("{key}.json"))
}

/// Read the cached narrative at `key`, or `None` when it is absent or corrupt.
///
/// A missing file is the ordinary miss and is silent. A file that exists but no
/// longer deserializes into a [`CachedNarrative`] — a truncated write, an older
/// on-disk shape — is logged at `warn` and treated as a miss, so a corrupt
/// sidecar can neither crash a run nor be served as if it were valid.
#[must_use]
pub fn read(cache_root: &Path, repo_path: &Path, key: &str) -> Option<CachedNarrative> {
    read_entry(&cache_path(cache_root, repo_path, key))
}

/// Write `entry` to the sidecar for `key`, creating the enrichment cache
/// directory if needed.
///
/// Best-effort: a cache is an optimization, so a directory-creation,
/// serialization, or write failure is logged at `warn` and swallowed — a run
/// must never fail because its advisory narrative could not be cached.
pub fn write(cache_root: &Path, repo_path: &Path, key: &str, entry: &CachedNarrative) {
    let path = cache_path(cache_root, repo_path, key);
    let Some(parent) = path.parent() else {
        tracing::warn!(
            "enrichment cache: entry path {} has no parent directory",
            path.display()
        );
        return;
    };
    if let Err(e) = std::fs::create_dir_all(parent) {
        tracing::warn!(
            "enrichment cache: could not create {}: {e}",
            parent.display()
        );
        return;
    }
    let json = match serde_json::to_string_pretty(entry) {
        Ok(json) => json,
        Err(e) => {
            tracing::warn!("enrichment cache: could not serialize entry for {key}: {e}");
            return;
        }
    };
    if let Err(e) = std::fs::write(&path, json) {
        tracing::warn!("enrichment cache: could not write {}: {e}", path.display());
    }
}

/// The newest cached narrative for `subject` in this repo by `created_at`, or
/// `None` when no readable entry describes that subject.
///
/// Callers compare its `fact_digest` against a freshly built fact sheet to warn
/// when the subject's own narrative has gone stale — scanning every sidecar but
/// keeping only entries whose `subject` equals the argument, so a narrative for
/// one file never reports a sibling as stale. Because `created_at` is a
/// fixed-width ISO-8601 UTC string, the lexical maximum is the chronological
/// maximum.
#[must_use]
pub fn latest_for_subject(
    cache_root: &Path,
    repo_path: &Path,
    subject: &str,
) -> Option<CachedNarrative> {
    let dir = enrichment_dir(cache_root, repo_path);
    let entries = std::fs::read_dir(&dir).ok()?;
    let mut newest: Option<CachedNarrative> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(parsed) = read_entry(&path) else {
            continue;
        };
        if parsed.subject != subject {
            continue;
        }
        if newest
            .as_ref()
            .is_none_or(|current| parsed.created_at > current.created_at)
        {
            newest = Some(parsed);
        }
    }
    newest
}

/// The per-repo enrichment cache directory:
/// `repo_cache_dir(cache_root, repo_path)/enrichment`.
fn enrichment_dir(cache_root: &Path, repo_path: &Path) -> PathBuf {
    repo_cache_dir(cache_root, repo_path).join("enrichment")
}

/// Read and deserialize a sidecar at an explicit path — the shared body behind
/// [`read`] and [`latest_for_subject`]. A missing/unreadable file is a silent
/// `None`; a file that fails to parse is logged at `warn` and treated as a miss.
fn read_entry(path: &Path) -> Option<CachedNarrative> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&text) {
        Ok(entry) => Some(entry),
        Err(e) => {
            tracing::warn!(
                "enrichment cache: ignoring corrupt entry {}: {e}",
                path.display()
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CachedNarrative, cache_key, cache_path};
    use crate::enrichment::SCHEMA_VERSION;
    use crate::enrichment::prompt::PROMPT_VERSION;
    use sha2::{Digest, Sha256};

    /// A sample entry for round-trip and ordering tests.
    fn sample(created_at: &str, fact_digest: &str) -> CachedNarrative {
        sample_for("src/subject.rs", created_at, fact_digest)
    }

    /// A sample entry describing an explicit `subject`, for the subject-scoped
    /// staleness tests.
    fn sample_for(subject: &str, created_at: &str, fact_digest: &str) -> CachedNarrative {
        CachedNarrative {
            narrative: "Diagnosis: score 87.5.".to_string(),
            subject: subject.to_string(),
            grounded: true,
            unmatched: Vec::new(),
            model: "mock-model".to_string(),
            prompt_version: PROMPT_VERSION,
            schema_version: SCHEMA_VERSION,
            fact_digest: fact_digest.to_string(),
            created_at: created_at.to_string(),
        }
    }

    #[test]
    fn cache_key_embeds_the_current_version_consts() {
        // Recompute the key from the live consts rather than mutating them: this
        // proves both versions are folded into the material without pinning the
        // test to their current numeric values.
        let text = "code-health\n  score = 87.5\n";
        let model = "mock-model";
        let material = format!("{text}|{SCHEMA_VERSION}|{PROMPT_VERSION}|{model}");
        let mut hasher = Sha256::new();
        hasher.update(material.as_bytes());
        let expected = hex::encode(hasher.finalize());
        assert_eq!(cache_key(text, model), expected);
    }

    #[test]
    fn cache_key_is_lowercase_hex_of_length_64() {
        let key = cache_key("sheet", "model");
        assert_eq!(key.len(), 64, "sha-256 hex is 64 chars");
        assert!(
            key.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "key must be lowercase hex: {key}"
        );
    }

    #[test]
    fn cache_key_changes_with_text_and_model() {
        let base = cache_key("sheet-a", "model-a");
        assert_ne!(
            base,
            cache_key("sheet-b", "model-a"),
            "text must move the key"
        );
        assert_ne!(
            base,
            cache_key("sheet-a", "model-b"),
            "model must move the key"
        );
    }

    #[test]
    fn cache_path_is_enrichment_key_json_under_the_repo_dir() {
        let path = cache_path(
            std::path::Path::new("/tmp/xdg-cache"),
            std::path::Path::new("/tmp/repo"),
            "deadbeef",
        );
        assert_eq!(
            path.extension().and_then(|e| e.to_str()),
            Some("json"),
            "sidecar is a .json file"
        );
        assert_eq!(
            path.file_name().and_then(|n| n.to_str()),
            Some("deadbeef.json")
        );
        assert_eq!(
            path.parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str()),
            Some("enrichment"),
            "sidecar lives in the per-repo enrichment/ dir"
        );
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn write_then_read_round_trips_the_entry() {
        let cache_root = tempfile::tempdir().expect("cache root");
        let repo = std::path::Path::new("/tmp/repo");
        let key = cache_key("sheet", "mock-model");
        let entry = sample("2025-01-01T00:00:00Z", "digest-abc");

        super::write(cache_root.path(), repo, &key, &entry);
        let got = super::read(cache_root.path(), repo, &key).expect("entry round-trips");

        assert_eq!(got.narrative, entry.narrative);
        assert_eq!(got.subject, entry.subject);
        assert_eq!(got.grounded, entry.grounded);
        assert_eq!(got.model, entry.model);
        assert_eq!(got.fact_digest, entry.fact_digest);
        assert_eq!(got.created_at, entry.created_at);
        assert_eq!(got.prompt_version, PROMPT_VERSION);
        assert_eq!(got.schema_version, SCHEMA_VERSION);
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn read_returns_none_on_missing_entry() {
        let cache_root = tempfile::tempdir().expect("cache root");
        let repo = std::path::Path::new("/tmp/repo");
        assert!(super::read(cache_root.path(), repo, "never-written").is_none());
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn read_returns_none_on_corrupt_entry_without_panicking() {
        let cache_root = tempfile::tempdir().expect("cache root");
        let repo = std::path::Path::new("/tmp/repo");
        let key = cache_key("sheet", "mock-model");
        let path = cache_path(cache_root.path(), repo, &key);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, b"{ this is not valid json").expect("write corrupt file");

        assert!(
            super::read(cache_root.path(), repo, &key).is_none(),
            "a corrupt sidecar must read as a miss, not a panic"
        );
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn latest_for_subject_returns_the_newest_entry_by_created_at() {
        let cache_root = tempfile::tempdir().expect("cache root");
        let repo = std::path::Path::new("/tmp/repo");

        // Two entries for the same subject under distinct keys with hand-set
        // timestamps so ordering is deterministic regardless of read order.
        super::write(
            cache_root.path(),
            repo,
            &cache_key("older", "mock-model"),
            &sample("2020-01-01T00:00:00Z", "digest-old"),
        );
        super::write(
            cache_root.path(),
            repo,
            &cache_key("newer", "mock-model"),
            &sample("2025-06-30T12:00:00Z", "digest-new"),
        );

        let newest = super::latest_for_subject(cache_root.path(), repo, "src/subject.rs")
            .expect("an entry exists for the subject");
        assert_eq!(
            newest.fact_digest, "digest-new",
            "latest_for_subject must return the chronologically newest entry"
        );
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn latest_for_subject_is_none_when_no_entries_exist() {
        let cache_root = tempfile::tempdir().expect("cache root");
        let repo = std::path::Path::new("/tmp/repo");
        assert!(super::latest_for_subject(cache_root.path(), repo, "src/subject.rs").is_none());
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn latest_for_subject_ignores_other_subjects() {
        // The regression pin: a narrative cached for "b.rs" must not be reported
        // as the latest for a never-narrated "a.rs". Explaining a file that has
        // no narrative of its own must therefore see no staleness signal, even
        // when a sibling was just narrated.
        let cache_root = tempfile::tempdir().expect("cache root");
        let repo = std::path::Path::new("/tmp/repo");

        super::write(
            cache_root.path(),
            repo,
            &cache_key("b-sheet", "mock-model"),
            &sample_for("b.rs", "2025-06-30T12:00:00Z", "digest-b"),
        );

        assert!(
            super::latest_for_subject(cache_root.path(), repo, "a.rs").is_none(),
            "a subject with no narrative of its own must not match a sibling's"
        );
        let for_b = super::latest_for_subject(cache_root.path(), repo, "b.rs")
            .expect("b.rs has its own narrative");
        assert_eq!(
            for_b.fact_digest, "digest-b",
            "the subject's own narrative is returned"
        );
        assert_eq!(for_b.subject, "b.rs");
    }
}

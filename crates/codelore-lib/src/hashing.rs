//! Stable content-fingerprint primitive shared across the fingerprint-emitting
//! modules (SARIF row identity in `output::sarif`, external-finding self-hash in
//! `external::sarif_parse`).
//!
//! Every fingerprint that must stay byte-stable across CI runs is a SHA-256 of
//! the identifying fields joined by a single `|` byte, rendered as
//! `sha256:<hex>`. Centralising the join + prefix here keeps those identities
//! aligned: a change to the encoding lands in one place instead of drifting
//! between the emitter and the parser.

use sha2::{Digest, Sha256};

/// SHA-256 the `parts` joined by a single `|` byte and return the digest as
/// `"sha256:<lowercase-hex>"`.
///
/// The `|` separator sits *between* parts only — no leading or trailing
/// separator — so `["a", "b"]` hashes `a|b`. Callers that fingerprint an
/// optional field stringify it first (an absent value becomes an empty part).
#[must_use]
pub fn sha256_prefixed(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            hasher.update(b"|");
        }
        hasher.update(part.as_bytes());
    }
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_parts_with_pipe_and_prefixes() {
        // Manually reproduce the contract: sha256("a|b|c") hex, prefixed.
        let mut h = Sha256::new();
        h.update(b"a|b|c");
        let expected = format!("sha256:{}", hex::encode(h.finalize()));
        assert_eq!(sha256_prefixed(&["a", "b", "c"]), expected);
    }

    #[test]
    fn single_part_has_no_separator() {
        let mut h = Sha256::new();
        h.update(b"solo");
        let expected = format!("sha256:{}", hex::encode(h.finalize()));
        assert_eq!(sha256_prefixed(&["solo"]), expected);
    }

    #[test]
    fn empty_trailing_part_is_kept() {
        // A stringified absent field (empty part) still contributes a trailing
        // separator — matches the self-hash's absent-start_line behaviour.
        let mut h = Sha256::new();
        h.update(b"engine|rule|path|");
        let expected = format!("sha256:{}", hex::encode(h.finalize()));
        assert_eq!(sha256_prefixed(&["engine", "rule", "path", ""]), expected);
    }
}

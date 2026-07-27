//! Tiny dependency-free nearest-match helper for the free-string CLI arguments
//! (`explain <topic>`, `schema <row-type>`) that are not clap value-enums and so
//! don't get clap's built-in did-you-mean. Enum-valued flags (`--analysis`,
//! `--format`, …) rely on clap's native suggestion instead.

/// Return the candidate closest to `input`, when one is close enough to be a
/// plausible typo fix — otherwise `None`.
///
/// Two heuristics, in order: a containment/prefix match (so a real name the user
/// abbreviated, e.g. `hotspot` → `hotspots`, is offered — preferring a prefix hit
/// then the shortest containing name), then a Levenshtein distance within a
/// length-scaled threshold (a third of the longer token, at least one edit) for
/// ordinary typos like `hotspt` → `hotspots`. Matching is ASCII-case-insensitive.
pub(crate) fn nearest<'a>(
    input: &str,
    candidates: impl IntoIterator<Item = &'a str>,
) -> Option<&'a str> {
    let needle = input.to_ascii_lowercase();
    let candidates: Vec<&'a str> = candidates.into_iter().collect();

    // 1) Containment / prefix: the input is a fragment of a real name. Prefer a
    //    prefix hit, then the shortest containing name (closest in length).
    if needle.len() >= 3 {
        let mut hits: Vec<&'a str> = candidates
            .iter()
            .copied()
            .filter(|c| c.to_ascii_lowercase().contains(&needle))
            .collect();
        if !hits.is_empty() {
            hits.sort_by_key(|c| (!c.to_ascii_lowercase().starts_with(&needle), c.len()));
            return hits.first().copied();
        }
    }

    // 2) Levenshtein within a length-scaled threshold.
    let mut best: Option<(&'a str, usize)> = None;
    for cand in candidates {
        let dist = levenshtein(&needle, &cand.to_ascii_lowercase());
        if best.is_none_or(|(_, bd)| dist < bd) {
            best = Some((cand, dist));
        }
    }
    let (cand, dist) = best?;
    let threshold = (needle.chars().count().max(cand.chars().count()) / 3).max(1);
    (dist <= threshold).then_some(cand)
}

/// Classic two-row iterative Levenshtein edit distance over Unicode scalar
/// values. Inputs are short CLI tokens, so the allocation is negligible.
fn levenshtein(a: &str, b: &str) -> usize {
    let b_chars: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b_chars.len()).collect();
    let mut curr: Vec<usize> = vec![0; b_chars.len() + 1];
    for (i, ac) in a.chars().enumerate() {
        curr[0] = i + 1;
        for (j, &bc) in b_chars.iter().enumerate() {
            let cost = usize::from(ac != bc);
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b_chars.len()]
}

#[cfg(test)]
mod tests {
    use super::{levenshtein, nearest};

    const TOPICS: &[&str] = &[
        "hotspot-score",
        "hotspots",
        "hotspot-velocity",
        "code-health",
    ];

    #[test]
    fn levenshtein_basics() {
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("hotspot", "hotspots"), 1);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
    }

    #[test]
    fn abbreviation_prefers_shortest_prefix_hit() {
        // `hotspot` is a prefix of three topics; the shortest wins.
        assert_eq!(nearest("hotspot", TOPICS.iter().copied()), Some("hotspots"));
    }

    #[test]
    fn typo_falls_back_to_edit_distance() {
        assert_eq!(nearest("hotspt", TOPICS.iter().copied()), Some("hotspots"));
    }

    #[test]
    fn far_off_input_suggests_nothing() {
        assert_eq!(
            nearest("definitely-not-a-topic-or-file", TOPICS.iter().copied()),
            None
        );
    }
}

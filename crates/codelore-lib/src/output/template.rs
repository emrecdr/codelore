//! Single-pass `{{KEY}}` placeholder substitution for HTML emitters.
//!
//! Replaces the previous chained `String::replace(...).replace(...)
//! .replace(...)` pattern in `output::html` and `output::spa`. The
//! chained form allocated a fresh `String` per call, copying the
//! growing intermediate buffer each time. For the SPA emitter, where
//! the template embeds a ~1.1 MB `echarts.min.js` payload plus the
//! per-analysis JSON data block (~50 KB – 2 MB) plus widget glue,
//! the chained form copied that multi-megabyte buffer 4–7 times per
//! emit. The single-pass form below allocates **one** output `String`,
//! pre-sized from the template length plus the sum of replacement
//! value lengths.

/// Substitute every `{{KEY}}` placeholder in `template` against the
/// `(key, value)` pairs in `replacements`. Unknown placeholders are
/// emitted verbatim (no panic; callers don't have to enumerate every
/// possible template placeholder).
///
/// The match is exact — `replacements[i].0` must include the surrounding
/// braces (e.g. `"{{TITLE}}"`). Matching is order-sensitive: earlier
/// entries take precedence on overlap, but in practice the keys are
/// disjoint.
pub(crate) fn substitute(template: &str, replacements: &[(&str, &str)]) -> String {
    // Capacity hint: template length plus all replacement values. The
    // actual output will be (template - sum(key lengths) +
    // sum(value lengths)), but `String::with_capacity` only avoids
    // re-allocation when the hint is at least the final size.
    let cap = template
        .len()
        .saturating_add(replacements.iter().map(|(_, v)| v.len()).sum());
    let mut out = String::with_capacity(cap);
    let mut pos = 0;
    while pos < template.len() {
        match template[pos..].find("{{") {
            None => {
                out.push_str(&template[pos..]);
                break;
            }
            Some(rel_start) => {
                let abs_start = pos + rel_start;
                out.push_str(&template[pos..abs_start]);
                let mut matched_len = 0;
                for (key, val) in replacements {
                    if template[abs_start..].starts_with(*key) {
                        out.push_str(val);
                        matched_len = key.len();
                        break;
                    }
                }
                if matched_len == 0 {
                    // Unknown placeholder — keep the literal `{{`, then
                    // continue scanning past it so we don't loop here.
                    out.push_str("{{");
                    pos = abs_start + 2;
                } else {
                    pos = abs_start + matched_len;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitute_replaces_simple_keys() {
        let html = substitute(
            "<title>{{TITLE}}</title>",
            &[("{{TITLE}}", "CodeLore Dashboard")],
        );
        assert_eq!(html, "<title>CodeLore Dashboard</title>");
    }

    #[test]
    fn substitute_replaces_multiple_disjoint_keys() {
        let html = substitute(
            "{{A}}-{{B}}-{{A}}",
            &[("{{A}}", "alpha"), ("{{B}}", "beta")],
        );
        assert_eq!(html, "alpha-beta-alpha");
    }

    #[test]
    fn substitute_leaves_unknown_placeholders_verbatim() {
        let html = substitute("{{KNOWN}}-{{UNKNOWN}}", &[("{{KNOWN}}", "x")]);
        assert_eq!(html, "x-{{UNKNOWN}}");
    }

    #[test]
    fn substitute_passes_through_text_with_no_placeholders() {
        let html = substitute("no placeholders here", &[("{{X}}", "y")]);
        assert_eq!(html, "no placeholders here");
    }

    #[test]
    fn substitute_handles_value_that_contains_double_braces() {
        // A value that contains `{{` should not be re-scanned for
        // placeholders — single-pass guarantees this.
        let html = substitute("{{A}}-end", &[("{{A}}", "<{{NESTED}}>")]);
        assert_eq!(html, "<{{NESTED}}>-end");
    }

    #[test]
    fn substitute_first_match_wins_on_overlap() {
        let html = substitute("{{X}}", &[("{{X}}", "first"), ("{{X}}", "second")]);
        assert_eq!(html, "first");
    }
}

//! Markdown output emitter for `$GITHUB_STEP_SUMMARY` and human-readable
//! CI artifacts.
//!
//! Writers are grouped into per-domain submodules and re-exported here so
//! callers keep the flat `output::markdown::write_*` paths. Shared helpers
//! live in this module.

use crate::{CodeLoreError, Result};
use std::borrow::Cow;
use std::io::Write;

mod architecture;
mod coupling;
mod delivery;
mod history;
mod hotspots;
mod knowledge;

pub use architecture::{
    write_arch_violations_markdown, write_architecture_metrics_markdown,
    write_architecture_roles_markdown, write_architecture_trend_markdown,
    write_centrality_markdown, write_communities_markdown, write_crossing_markdown,
    write_cycle_health_markdown, write_cycle_origins_markdown, write_dependency_cycles_markdown,
    write_god_classes_markdown, write_instability_markdown, write_modularity_violations_markdown,
    write_unstable_interface_markdown,
};
pub use coupling::{
    write_clone_coupling_markdown, write_clones_markdown, write_coupling_markdown,
    write_function_coupling_markdown, write_soc_markdown,
};
pub use delivery::{
    write_delivery_friction_markdown, write_delivery_metrics_markdown, write_lead_time_markdown,
    write_release_cadence_markdown,
};
pub use history::{
    write_abs_churn_markdown, write_author_churn_markdown, write_code_age_markdown,
    write_entity_churn_markdown, write_messages_markdown, write_revisions_markdown,
    write_stale_code_markdown, write_summary_markdown,
};
pub use hotspots::{
    write_code_health_markdown, write_defect_validation_markdown, write_effort_exposure_markdown,
    write_finding_hotspot_overlap_markdown, write_function_xray_markdown,
    write_health_trend_markdown, write_hotspot_velocity_markdown, write_hotspots_markdown,
    write_refactoring_targets_markdown,
};
pub use knowledge::{
    write_authors_markdown, write_bus_factor_markdown, write_code_familiarity_markdown,
    write_communication_markdown, write_coordination_needs_markdown, write_entity_effort_markdown,
    write_entity_ownership_markdown, write_knowledge_islands_markdown,
    write_main_dev_by_deletions_markdown, write_main_dev_by_revs_markdown, write_main_dev_markdown,
    write_marginal_owner_risk_markdown, write_ownership_markdown, write_pair_programming_markdown,
    write_team_composition_markdown, write_top_committers_markdown,
};

fn header<W: Write>(w: &mut W, title: &str) -> Result<()> {
    writeln!(w, "# {title}").map_err(CodeLoreError::Io)?;
    writeln!(w).map_err(CodeLoreError::Io)?;
    Ok(())
}

/// Escape a string for safe inclusion inside a GFM table cell.
///
/// Per the GFM spec, an unescaped `|` inside a cell ends the cell —
/// every subsequent character on the line lands in the next column,
/// so a single stray pipe in a path / author name / commit message
/// silently corrupts the entire row's column alignment. The escape is
/// a leading backslash on the pipe character.
///
/// Newlines and carriage returns inside a cell are GFM-unsupported
/// entirely (the table row terminates at the line break), so they get
/// replaced with the visual `↵` glyph to preserve the cell boundary
/// rather than break the row.
///
/// Returns `Cow::Borrowed` for the common case (no escape needed) so
/// the happy path is allocation-free. The borrow checker thread-of-
/// life this preserves matters here — every analysis emits hundreds
/// to thousands of rows and the markdown emitter is a hot path for
/// the `$GITHUB_STEP_SUMMARY` flow.
#[must_use]
pub fn escape_md_cell(s: &str) -> Cow<'_, str> {
    if !s.contains('|') && !s.contains('\n') && !s.contains('\r') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        match c {
            '|' => out.push_str("\\|"),
            '\n' | '\r' => out.push('↵'),
            other => out.push(other),
        }
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod escape_tests {
    use super::escape_md_cell;
    use std::borrow::Cow;

    /// Common case: no special characters → borrow the input.
    #[test]
    fn plain_string_is_borrowed() {
        let s = "src/main.rs";
        match escape_md_cell(s) {
            Cow::Borrowed(out) => assert_eq!(out, s),
            Cow::Owned(_) => panic!("expected borrow, got owned allocation"),
        }
    }

    /// A literal `|` in a path / author name / commit message gets
    /// backslash-escaped so GFM renders the row with the correct column
    /// count instead of treating the pipe as a cell terminator.
    #[test]
    fn pipe_is_backslash_escaped() {
        let out = escape_md_cell("path/with|pipe.rs");
        assert_eq!(out, "path/with\\|pipe.rs");
    }

    /// Multiple pipes within a single cell — each escaped.
    #[test]
    fn multiple_pipes_each_escaped() {
        let out = escape_md_cell("a|b|c");
        assert_eq!(out, "a\\|b\\|c");
    }

    /// Newlines inside a cell terminate the GFM row entirely. Substitute
    /// the visual `↵` glyph so the row stays intact.
    #[test]
    fn newline_becomes_arrow_glyph() {
        let out = escape_md_cell("line1\nline2");
        assert_eq!(out, "line1↵line2");
    }

    /// Mixed payload — verifies the loop handles each special character
    /// independently and preserves all other characters verbatim.
    #[test]
    fn mixed_special_chars_pipe_and_newline() {
        let out = escape_md_cell("a|b\nc");
        assert_eq!(out, "a\\|b↵c");
    }
}

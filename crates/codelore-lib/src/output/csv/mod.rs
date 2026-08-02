//! CSV emitters. Headers match code-maat exactly for golden-test parity.
//!
//! Writers are grouped into per-domain submodules and re-exported here so
//! callers keep the flat `output::csv::write_*` paths. Shared helpers live in
//! this module.

mod architecture;
mod coupling;
mod delivery;
mod history;
mod hotspots;
mod knowledge;

pub use architecture::{
    write_arch_violations_csv, write_architecture_metrics_csv, write_architecture_roles_csv,
    write_architecture_trend_csv, write_centrality_csv, write_communities_csv, write_crossing_csv,
    write_cycle_health_csv, write_cycle_origins_csv, write_dependency_cycles_csv,
    write_god_classes_csv, write_instability_csv, write_modularity_violations_csv,
    write_unstable_interface_csv,
};
pub use coupling::{
    write_clone_coupling_csv, write_clones_csv, write_coupling_csv, write_function_coupling_csv,
    write_soc_csv,
};
pub use delivery::{
    write_delivery_friction_csv, write_delivery_metrics_csv, write_lead_time_csv,
    write_release_cadence_csv,
};
pub use history::{
    write_abs_churn_csv, write_author_churn_csv, write_code_age_csv, write_entity_churn_csv,
    write_messages_csv, write_revisions_csv, write_stale_code_csv, write_summary_csv,
};
pub use hotspots::{
    write_code_health_csv, write_defect_validation_csv, write_effort_exposure_csv,
    write_finding_hotspot_overlap_csv, write_function_hotspots_csv, write_function_xray_csv,
    write_health_trend_csv, write_hotspot_velocity_csv, write_hotspots_csv,
    write_refactoring_targets_csv,
};
pub use knowledge::{
    write_authors_csv, write_bus_factor_csv, write_code_familiarity_csv, write_communication_csv,
    write_coordination_needs_csv, write_entity_effort_csv, write_entity_ownership_csv,
    write_knowledge_islands_csv, write_main_dev_by_deletions_csv, write_main_dev_by_revs_csv,
    write_main_dev_csv, write_marginal_owner_risk_csv, write_ownership_csv,
    write_pair_programming_csv, write_team_composition_csv, write_top_committers_csv,
};

fn quote_if_needed(s: &str) -> String {
    // Formula-injection guard: a cell whose FIRST character is one a
    // spreadsheet treats as a formula trigger (`=`, `+`, `-`, `@`, or a
    // leading tab) is force-quoted and prefixed with a `'` inside the
    // quotes — the standard CSV-injection mitigation. An author name or
    // path beginning with such a character would otherwise execute as a
    // formula when the CSV is opened in Excel / Sheets.
    let needs_formula_guard = matches!(
        s.as_bytes().first(),
        Some(b'=' | b'+' | b'-' | b'@' | b'\t')
    );

    // RFC 4180 §2.5: fields containing `,`, `"`, CR, or LF MUST be quoted.
    // Missing `\r` here would split a row in two if an author name or commit
    // metadata carried a bare carriage return (rare but legal in git's byte
    // stream).
    let needs_rfc_quote =
        s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r');

    if needs_formula_guard {
        let escaped = s.replace('"', "\"\"");
        format!("\"'{escaped}\"")
    } else if needs_rfc_quote {
        let escaped = s.replace('"', "\"\"");
        format!("\"{escaped}\"")
    } else {
        s.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::quote_if_needed;

    #[test]
    fn quotes_carriage_return() {
        assert_eq!(quote_if_needed("a\rb"), "\"a\rb\"");
    }

    #[test]
    fn quotes_comma_and_doubles_quotes() {
        assert_eq!(quote_if_needed("a,\"b\""), "\"a,\"\"b\"\"\"");
    }

    #[test]
    fn leaves_plain_string_alone() {
        assert_eq!(quote_if_needed("plain"), "plain");
    }

    #[test]
    fn guards_formula_injection_leading_chars() {
        // A cell whose first character is a spreadsheet formula trigger
        // (`=`, `+`, `-`, `@`, or a tab) must be force-quoted AND
        // prefixed with a single-quote inside the quotes so a
        // spreadsheet treats it as literal text, not a formula.
        assert_eq!(quote_if_needed("=cmd"), "\"'=cmd\"");
        assert_eq!(quote_if_needed("+1"), "\"'+1\"");
        assert_eq!(quote_if_needed("-2"), "\"'-2\"");
        assert_eq!(quote_if_needed("@SUM"), "\"'@SUM\"");
        assert_eq!(quote_if_needed("\tlead"), "\"'\tlead\"");
    }

    #[test]
    fn formula_guard_escapes_embedded_quotes() {
        // The guard composes with RFC-4180 quote-doubling: an embedded
        // double-quote is still escaped inside the guarded cell.
        assert_eq!(quote_if_needed("=\"x\""), "\"'=\"\"x\"\"\"");
    }
}

//! Versioned prompt templates for the advisory narrative layer.
//!
//! Each [`Lens`] pairs a fixed system prompt — the grounding contract the model
//! must honour — with a user prompt that wraps a deterministic fact sheet
//! verbatim. The templates are versioned by [`PROMPT_VERSION`]; that version
//! feeds the sidecar narrative-cache key, so editing a prompt here and bumping
//! the constant invalidates every cached narrative naturally.

/// Prompt-template version. Bump it whenever the system or user prompt text
/// changes so cached narratives keyed on the old wording are recomputed.
pub const PROMPT_VERSION: u32 = 1;

/// Which narrative a prompt drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lens {
    /// Per-file diagnosis (plus a refactoring direction when the sheet supports
    /// one).
    FileDiagnosis,
    /// Pull-request diff narrative.
    DiffNarrative,
}

/// The per-file diagnosis contract: ground every claim in the sheet, always
/// diagnose, and only suggest a refactoring direction when the sheet carries
/// the structural evidence for it.
const FILE_DIAGNOSIS_SYSTEM: &str = "\
You are an advisory code-health analyst. Your only input is a deterministic fact \
sheet describing one source file. Ground every statement in that sheet.

Rules:
- Use only facts that appear in the sheet. Never invent numbers, file names, \
authors, or trends the sheet does not contain.
- Whenever you state a value, cite the exact number from the sheet.
- If the sheet does not support something a reader might expect, say \"the data \
doesn't show\" it rather than guessing.

Always emit a \"## Diagnosis\" section: a concise read of what the facts say about \
this file's health and risk.

Emit a \"## Refactoring direction\" section only when the fact sheet contains a \
\"cycle\" or \"functions\" section with concrete evidence, and base the suggestion \
on that evidence. When neither section is present, omit \"## Refactoring \
direction\" entirely.

Be concise and reviewer-legible. No preamble, and do not restate these rules.";

/// The diff-narrative contract: the same grounding rules, narrating what a
/// change does to the codebase's health.
const DIFF_NARRATIVE_SYSTEM: &str = "\
You are an advisory change analyst. Your only input is a deterministic fact sheet \
describing one pull request's diff. Ground every statement in that sheet.

Rules:
- Use only facts that appear in the sheet. Never invent numbers, file names, \
authors, or trends the sheet does not contain.
- Whenever you state a value, cite the exact number from the sheet.
- If the sheet does not support something a reader might expect, say \"the data \
doesn't show\" it rather than guessing.

Narrate what this change does to the codebase's health in a few tight sentences: \
what moved, which files carry the risk, and whether the signals point up or down. \
No preamble, and do not restate these rules.";

/// The fixed grounding contract for `lens`.
#[must_use]
pub fn system_prompt(lens: Lens) -> &'static str {
    match lens {
        Lens::FileDiagnosis => FILE_DIAGNOSIS_SYSTEM,
        Lens::DiffNarrative => DIFF_NARRATIVE_SYSTEM,
    }
}

/// The user prompt for `lens`, embedding `fact_sheet_text` verbatim as the sole
/// evidence the model may draw on.
#[must_use]
pub fn user_prompt(lens: Lens, fact_sheet_text: &str) -> String {
    let task = match lens {
        Lens::FileDiagnosis => "Diagnose this file from its fact sheet below.",
        Lens::DiffNarrative => "Narrate this change from its diff fact sheet below.",
    };
    format!("{task} Follow the grounding rules.\n\n{fact_sheet_text}")
}

#[cfg(test)]
mod tests {
    use super::{Lens, system_prompt, user_prompt};

    #[test]
    fn file_diagnosis_system_prompt_states_grounding_and_headers() {
        let p = system_prompt(Lens::FileDiagnosis);
        assert!(p.contains("Use only facts"));
        assert!(p.contains("the data doesn't show"));
        assert!(p.contains("## Diagnosis"));
        assert!(p.contains("## Refactoring direction"));
    }

    #[test]
    fn diff_narrative_system_prompt_states_grounding() {
        let p = system_prompt(Lens::DiffNarrative);
        assert!(p.contains("Use only facts"));
        assert!(p.contains("the data doesn't show"));
    }

    #[test]
    fn user_prompt_embeds_the_sheet_verbatim() {
        let sheet = "code-health\n  score = 87.5\n  band = green\n";
        let out = user_prompt(Lens::FileDiagnosis, sheet);
        assert!(out.contains(sheet));
    }
}

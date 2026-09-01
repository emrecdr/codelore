//! Guard: the provenance-signing token is reachable only from trusted signers.
//!
//! This is the invariant that keeps the release pipeline at SLSA v1.0 Build
//! L3. The spec requires the platform to "prevent secret material used to
//! sign the provenance from being accessible to the user-defined build
//! steps". A release build runs `cargo build`, which executes `build.rs`;
//! the container build runs the Containerfile's `RUN` steps. Both are
//! repository-authored code. If the sigstore OIDC token were reachable from
//! those jobs, a compromised build script could mint provenance for an
//! artifact it never produced — exactly the forgery L3 closes.
//!
//! The rule has two sides, because "holds the token" is only dangerous when
//! something untrusted runs beside it:
//!
//!   1. In ordinary workflows, no job that declares `steps:` may hold
//!      `id-token`/`attestations`. Those jobs compile, package, and push —
//!      they execute repository-authored code by definition.
//!   2. The trusted signers may hold the token, but must earn it: they are
//!      `workflow_call`-only, and contain no `run:` step at all. No shell
//!      means no repository-authored code executes alongside the token; the
//!      only things that run are pinned actions.
//!
//! Without a check, the regression is silent and attractive — adding
//! `id-token: write` back to a build job makes an inline attestation "just
//! work", drops the pipeline to L2, and changes no visible output. The
//! attestations still appear; they are merely forgeable again.

use std::path::{Path, PathBuf};

/// Permission scopes that grant the ability to mint or persist provenance.
const SIGNING_SCOPES: &[&str] = &["id-token", "attestations"];

/// `CARGO_MANIFEST_DIR` is `<root>/crates/codelore-lib`; two levels up is the
/// workspace root. Embedded at compile time, so it resolves under CI too.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root two levels above crates/codelore-lib")
        .to_path_buf()
}

/// The workflows permitted to hold the signing token.
///
/// Membership is not a free pass — every file here is separately asserted
/// below to be `workflow_call`-only and free of `run:` steps, which is what
/// makes holding the token safe. Adding a name here without those
/// properties fails the guard rather than silencing it.
fn is_trusted_signer(rel: &str) -> bool {
    rel.ends_with("attest-artifact.yml") || rel.ends_with("attest-digest.yml")
}

/// Workflows permitted to hold `id-token` ALONE — OIDC consumers, not
/// signers. Scorecard's `publish_results` requires an OIDC token so
/// scorecard.dev can verify which repository's CI produced the score.
///
/// Membership is not a free pass, twice over. First, `attestations` stays
/// forbidden here (asserted in the main loop): that is the scope that
/// persists GitHub provenance, and `id-token` without it cannot write an
/// attestation at all. Second, an OIDC token minted in one of these jobs
/// asserts *this workflow's* identity — and the documented release
/// verification (the README's `gh attestation verify` snippet) pins
/// `--signer-workflow` to the trusted signers above, so a consumer-minted
/// signature can never verify as release provenance. The consumer count is
/// asserted below so deleting the workflow retires the exemption loudly.
fn is_oidc_consumer(rel: &str) -> bool {
    rel.ends_with("scorecard.yml")
}

#[derive(Debug)]
struct Job {
    name: String,
    line: usize,
    /// True when the job declares `steps:` — it executes things itself.
    /// False for a `uses:` reusable-workflow call, which runs nothing.
    runs_steps: bool,
    permissions: Vec<String>,
}

#[derive(Debug)]
struct Workflow {
    top_level_permissions: Vec<String>,
    jobs: Vec<Job>,
    /// True when any step anywhere in the file executes a shell.
    has_run_step: bool,
    accepts_workflow_call: bool,
    /// Triggers other than `workflow_call` (`push`, `workflow_dispatch`, …).
    other_triggers: Vec<String>,
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Scopes granted by a `permissions:` line that carries its value inline
/// rather than opening a block, or `None` when a block follows.
///
/// Three spellings grant the same thing and only one of them names a scope.
/// `write-all` grants every scope, including the signing ones, while writing
/// neither of their names — and it is the first thing anyone reaches for when
/// an attestation step fails for want of a permission. The flow map
/// `{id-token: write}` names them but not on their own lines, where the block
/// parser looks. A guard that recognises one spelling of the grant it forbids
/// leaves the other two as silent ways back to Build L2.
fn inline_permissions(trimmed: &str) -> Option<Vec<String>> {
    let value = trimmed.strip_prefix("permissions:")?.trim();
    if value.is_empty() {
        return None;
    }
    if value == "write-all" {
        return Some(SIGNING_SCOPES.iter().map(|s| (*s).to_owned()).collect());
    }
    let Some(inner) = value.strip_prefix('{').and_then(|v| v.strip_suffix('}')) else {
        // `read-all`, `{}`, or anything else that grants nothing signable.
        return Some(Vec::new());
    };
    Some(
        inner
            .split(',')
            .filter_map(|entry| entry.split_once(':'))
            .map(|(scope, _)| scope.trim().to_owned())
            .collect(),
    )
}

/// Parse the shape this guard needs out of a workflow file.
///
/// Hand-rolled rather than pulling in a YAML crate for one guard: these
/// files are machine-uniform two-space-indented GitHub workflows, so the
/// shape is `jobs:` / job name at 2 / job keys at 4 / scopes at 6.
fn parse(text: &str) -> Workflow {
    let mut wf = Workflow {
        top_level_permissions: Vec::new(),
        jobs: Vec::new(),
        has_run_step: false,
        accepts_workflow_call: false,
        other_triggers: Vec::new(),
    };
    let mut in_jobs = false;
    let mut in_triggers = false;
    // Depth of the `permissions:` key currently being collected under, if
    // any. Scopes sit exactly two spaces deeper.
    let mut perm_indent: Option<usize> = None;

    for (idx, raw) in text.lines().enumerate() {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let col = indent(raw);

        // A shell step in any form: `- run: x`, `run: |`, etc.
        let step_body = trimmed.trim_start_matches("- ").trim_start();
        if step_body.starts_with("run:") {
            wf.has_run_step = true;
        }

        // Close an open permissions block once indentation returns.
        if let Some(pi) = perm_indent
            && col <= pi
        {
            perm_indent = None;
        }

        if let Some(pi) = perm_indent
            && col == pi + 2
            && let Some((scope, _)) = trimmed.split_once(':')
        {
            let scope = scope.trim().to_owned();
            match wf.jobs.last_mut() {
                Some(job) => job.permissions.push(scope),
                None => wf.top_level_permissions.push(scope),
            }
            continue;
        }

        if col == 0 {
            in_jobs = trimmed == "jobs:";
            // `on:` is parsed as a YAML boolean by some readers; match the
            // literal text instead so the trigger list stays reliable.
            in_triggers = trimmed == "on:";
            if trimmed == "permissions:" {
                perm_indent = Some(0);
            } else if let Some(scopes) = inline_permissions(trimmed) {
                wf.top_level_permissions.extend(scopes);
            }
            continue;
        }

        if in_triggers
            && col == 2
            && let Some((trigger, _)) = trimmed.split_once(':')
        {
            let trigger = trigger.trim();
            if trigger == "workflow_call" {
                wf.accepts_workflow_call = true;
            } else {
                wf.other_triggers.push(trigger.to_owned());
            }
            continue;
        }

        if in_jobs && col == 2 && trimmed.ends_with(':') {
            wf.jobs.push(Job {
                name: trimmed.trim_end_matches(':').to_owned(),
                line: idx + 1,
                runs_steps: false,
                permissions: Vec::new(),
            });
            continue;
        }

        if in_jobs
            && col == 4
            && let Some(job) = wf.jobs.last_mut()
        {
            if trimmed == "steps:" {
                job.runs_steps = true;
            } else if trimmed == "permissions:" {
                perm_indent = Some(4);
            } else if let Some(scopes) = inline_permissions(trimmed) {
                job.permissions.extend(scopes);
            }
        }
    }

    wf
}

fn workflow_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root.join(".github/workflows")) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("yml") {
                files.push(p);
            }
        }
    }
    files.sort();
    files
}

fn grants_signing(scopes: &[String]) -> Vec<&String> {
    scopes
        .iter()
        .filter(|s| SIGNING_SCOPES.contains(&s.as_str()))
        .collect()
}

#[test]
fn signing_permissions_are_reachable_only_from_trusted_signers() {
    let root = workspace_root();
    let files = workflow_files(&root);
    assert!(
        files.len() > 3,
        "found only {} workflow file(s) — path resolution is broken, so this \
         guard would pass vacuously",
        files.len()
    );

    let mut violations: Vec<String> = Vec::new();
    let mut step_jobs = 0usize;
    let mut signers_seen = 0usize;
    let mut oidc_consumers_seen = 0usize;

    for file in &files {
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        let rel = file
            .strip_prefix(&root)
            .unwrap_or(file)
            .to_string_lossy()
            .replace('\\', "/");
        let wf = parse(&text);

        if is_trusted_signer(&rel) {
            signers_seen += 1;
            // A signer earns the token by running no shell and by being
            // callable only as a reusable workflow.
            if wf.has_run_step {
                violations.push(format!(
                    "  {rel}: trusted signer contains a `run:` step — shell beside \
                     the signing token defeats the isolation it exists to provide"
                ));
            }
            if !wf.accepts_workflow_call {
                violations.push(format!(
                    "  {rel}: trusted signer is not `workflow_call`-triggered"
                ));
            }
            if !wf.other_triggers.is_empty() {
                violations.push(format!(
                    "  {rel}: trusted signer also triggers on {:?} — it must be \
                     reachable only through a caller that it cannot influence",
                    wf.other_triggers
                ));
            }
            continue;
        }

        let oidc_consumer = is_oidc_consumer(&rel);
        if oidc_consumer {
            oidc_consumers_seen += 1;
        }
        // An OIDC consumer may hold `id-token` alone; `attestations` — the
        // scope that actually persists provenance — stays forbidden for it
        // like everywhere else.
        let permitted = |scope: &str| oidc_consumer && scope == "id-token";

        for scope in grants_signing(&wf.top_level_permissions) {
            if permitted(scope) {
                continue;
            }
            violations.push(format!(
                "  {rel}: workflow-level `permissions:` grants `{scope}` — every \
                 job in the file inherits it by default"
            ));
        }

        for job in &wf.jobs {
            if job.runs_steps {
                step_jobs += 1;
                for scope in grants_signing(&job.permissions) {
                    if permitted(scope) {
                        continue;
                    }
                    violations.push(format!(
                        "  {rel}:{}: job `{}` runs steps and grants `{scope}`",
                        job.line, job.name
                    ));
                }
            }
        }
    }

    assert!(
        step_jobs > 5,
        "only {step_jobs} step-running job(s) parsed — the parser is not \
         seeing the workflows, so this guard would pass vacuously"
    );
    assert_eq!(
        signers_seen, 2,
        "expected both trusted signers to be present; attestation has moved \
         or a signer was renamed — re-check the pipeline before adjusting \
         `is_trusted_signer`"
    );
    assert_eq!(
        oidc_consumers_seen, 1,
        "expected exactly one OIDC-consumer workflow; if scorecard.yml was \
         removed or renamed, retire or update `is_oidc_consumer` with it \
         rather than leaving a dormant exemption"
    );

    assert!(
        violations.is_empty(),
        "{} finding(s) put the provenance signing token within reach of code \
         the pipeline executes, dropping it from SLSA Build L3 to L2:\n{}\n\n\
         Signing must stay in a trusted signer that runs no \
         repository-authored code — see .github/workflows/attest-artifact.yml \
         and attest-digest.yml. Do not grant id-token/attestations to a job \
         that has `steps:`.",
        violations.len(),
        violations.join("\n"),
    );
}

#[test]
fn the_isolation_guard_detects_a_regression() {
    // A guard that cannot fail is worth nothing. Feed the parser the exact
    // shape of the regression it exists to catch — the inline-attestation
    // pattern this pipeline was migrated away from — and confirm it is seen.
    let regressed = "\
on:
  push:
    tags: ['v*']

permissions:
  contents: write

jobs:
  build:
    permissions:
      contents: read
      id-token: write
      attestations: write
    steps:
      - uses: actions/checkout@v7
      - run: cargo build --release
  attest:
    uses: ./.github/workflows/attest-artifact.yml
    permissions:
      id-token: write
";
    let wf = parse(regressed);
    assert_eq!(
        wf.top_level_permissions,
        vec!["contents"],
        "workflow-level scopes parsed"
    );
    assert_eq!(wf.jobs.len(), 2, "both jobs parsed");
    assert!(wf.has_run_step, "the `run:` step is seen");
    assert!(!wf.accepts_workflow_call, "this caller is not a signer");
    assert_eq!(wf.other_triggers, vec!["push"], "triggers parsed");

    let build = &wf.jobs[0];
    assert_eq!(build.name, "build");
    assert!(build.runs_steps, "a job with `steps:` runs steps");
    assert!(
        !grants_signing(&build.permissions).is_empty(),
        "the guard must see the signing scopes on the build job — this is \
         precisely the L2 regression"
    );

    // The same grant, written the two ways that name no scope on a line of
    // its own. Both were invisible until the parser learned the inline form:
    // `write-all` is the reflex fix when an attestation step fails for want
    // of a permission, and it drops the pipeline to L2 while looking like a
    // one-word convenience.
    for shorthand in ["permissions: write-all", "permissions: {id-token: write}"] {
        let wf = parse(&format!(
            "on:\n  push:\n\njobs:\n  build:\n    {shorthand}\n    steps:\n      - run: cargo build\n"
        ));
        let job = &wf.jobs[0];
        assert!(
            !grants_signing(&job.permissions).is_empty(),
            "a job granting signing scopes via `{shorthand}` must be seen; \
             parsed scopes were {:?}",
            job.permissions
        );
    }

    // `read-all` grants nothing signable and must not be read as a grant, or
    // the guard fails every workflow that uses the safe shorthand.
    let readonly =
        parse("jobs:\n  build:\n    permissions: read-all\n    steps:\n      - run: x\n");
    assert!(
        grants_signing(&readonly.jobs[0].permissions).is_empty(),
        "`read-all` is not a signing grant"
    );

    let attest = &wf.jobs[1];
    assert_eq!(attest.name, "attest");
    assert!(
        !attest.runs_steps,
        "a `uses:` reusable-workflow call runs no steps of its own and is \
         therefore allowed to hold the token"
    );

    // And the accepted shape: a read-only build job must not trip the guard.
    let compliant = "\
jobs:
  build:
    permissions:
      contents: read
    steps:
      - uses: actions/checkout@v7
";
    let ok = parse(compliant);
    assert!(
        grants_signing(&ok.jobs[0].permissions).is_empty(),
        "a read-only build job must not trip the guard"
    );

    // A signer that runs shell is a violation even though it is allowlisted.
    let bad_signer = "\
on:
  workflow_call:
    inputs:
      artifact-name:
        required: true
        type: string

jobs:
  attest:
    permissions:
      id-token: write
    steps:
      - run: echo untrusted
";
    let bad = parse(bad_signer);
    assert!(bad.accepts_workflow_call, "workflow_call trigger parsed");
    assert!(
        bad.has_run_step,
        "a signer containing shell must be detectable — allowlisting a file \
         must not exempt it from earning the token"
    );
}

//! Guard: every job in the release workflow that publishes outward is
//! gated to tag-triggered runs.
//!
//! `release.yml` also runs on `workflow_dispatch`, and its header advertises
//! that as a dry run. Nothing enforced it. A manual run took the `plan` job's
//! `manual-<timestamp>` tag fallback and published a real GitHub Release from
//! it, then pushed a Homebrew formula pointing at that throwaway build to the
//! public tap. Only the crates.io step was protected — so the hazard was
//! understood for the one irreversible publish and never extended.
//!
//! This is a *detector*, not a list of job names: it looks for the things
//! that actually reach the outside world — creating a GitHub Release,
//! `git push` to another repository, `cargo publish` — and requires any job
//! containing one to assert `github.ref_type == 'tag'`. A publishing job
//! added later is therefore covered without anyone remembering to update a
//! list here.
//!
//! Each job must carry its own gate. Skipping the first one would likely
//! cascade through `needs:`, but that couples an outward-facing safety
//! property to dependency-graph semantics, where editing a `needs:` list
//! silently re-enables publishing.

use std::path::{Path, PathBuf};

/// Substrings that mean "this step reaches the outside world".
///
/// Matched against comment-stripped lines: the workflow discusses
/// `cargo publish` and `action-gh-release` at length in prose, and counting
/// those would attribute publication to whichever job happened to explain
/// itself most thoroughly.
const PUBLICATION_MARKERS: &[&str] = &["action-gh-release", "cargo publish", "git push"];

/// The condition a publishing job must assert.
const TAG_GATE: &str = "github.ref_type == 'tag'";

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root two levels above crates/codelore-lib")
        .to_path_buf()
}

#[derive(Debug)]
struct Job {
    name: String,
    gated: bool,
    markers: Vec<&'static str>,
}

/// Strip a trailing `#` comment, and drop whole-line comments entirely.
///
/// Naive and sufficient here: the workflow has no `#` inside a quoted scalar
/// on a line that also carries a publication marker. A YAML-aware strip would
/// need a parser, and the failure mode of being slightly too eager is a
/// missed marker, which the vacuity assertion below catches.
fn strip_comment(line: &str) -> &str {
    match line.find('#') {
        Some(i) => &line[..i],
        None => line,
    }
}

fn parse_jobs(text: &str) -> Vec<Job> {
    let mut jobs: Vec<Job> = Vec::new();
    let mut in_jobs = false;

    for raw in text.lines() {
        let code = strip_comment(raw);
        let trimmed = code.trim();
        if trimmed.is_empty() {
            continue;
        }
        let col = code.len() - code.trim_start().len();

        if col == 0 {
            in_jobs = trimmed == "jobs:";
            continue;
        }
        if !in_jobs {
            continue;
        }

        if col == 2 && trimmed.ends_with(':') {
            jobs.push(Job {
                name: trimmed.trim_end_matches(':').to_owned(),
                gated: false,
                markers: Vec::new(),
            });
            continue;
        }

        let Some(job) = jobs.last_mut() else {
            continue;
        };
        if col == 4 && trimmed.starts_with("if:") && trimmed.contains(TAG_GATE) {
            job.gated = true;
        }
        for marker in PUBLICATION_MARKERS {
            if trimmed.contains(marker) && !job.markers.contains(marker) {
                job.markers.push(marker);
            }
        }
    }

    jobs
}

#[test]
fn every_publishing_job_is_tag_gated() {
    let root = workspace_root();
    let path = root.join(".github/workflows/release.yml");
    let text = std::fs::read_to_string(&path).expect("release.yml is readable");
    let jobs = parse_jobs(&text);

    assert!(
        jobs.len() >= 5,
        "parsed only {} job(s) from release.yml — the parser is not seeing \
         the file, so this guard would pass vacuously",
        jobs.len()
    );

    let publishing: Vec<&Job> = jobs.iter().filter(|j| !j.markers.is_empty()).collect();
    assert!(
        publishing.len() >= 3,
        "found only {} publishing job(s); expected the GitHub Release, the \
         Homebrew tap push, and the crates.io publish. Either a marker in \
         PUBLICATION_MARKERS went stale or comment-stripping ate a real \
         step — both would make this guard silently toothless",
        publishing.len()
    );

    let ungated: Vec<String> = publishing
        .iter()
        .filter(|j| !j.gated)
        .map(|j| {
            format!(
                "  job `{}` publishes via {:?} but has no tag gate",
                j.name, j.markers
            )
        })
        .collect();

    assert!(
        ungated.is_empty(),
        "{} job(s) in release.yml publish outward on ANY trigger, including a \
         manual workflow_dispatch that is documented as a dry run:\n{}\n\nAdd \
         `if: {TAG_GATE}` to each. Do not rely on a skipped `needs:` \
         dependency to cascade — that couples publication safety to the \
         dependency graph.",
        ungated.len(),
        ungated.join("\n"),
    );
}

#[test]
fn the_gate_guard_detects_an_ungated_publish_and_ignores_prose() {
    // A guard that cannot fail is worth nothing. Exercise the exact shape of
    // the defect, and the comment-stripping that keeps it honest.
    let regressed = "\
jobs:
  plan:
    runs-on: ubuntu-latest
    steps:
      - run: echo plan
  release:
    needs: [plan]
    steps:
      - uses: softprops/action-gh-release@abc
  homebrew-publish:
    if: github.ref_type == 'tag'
    steps:
      - run: git push
";
    let jobs = parse_jobs(regressed);
    assert_eq!(jobs.len(), 3, "all jobs parsed");

    let release = jobs.iter().find(|j| j.name == "release").expect("release");
    assert_eq!(
        release.markers,
        vec!["action-gh-release"],
        "marker detected"
    );
    assert!(
        !release.gated,
        "an ungated publishing job must be detectable"
    );

    let brew = jobs
        .iter()
        .find(|j| j.name == "homebrew-publish")
        .expect("homebrew-publish");
    assert!(brew.gated, "a gated job is accepted");

    let plan = jobs.iter().find(|j| j.name == "plan").expect("plan");
    assert!(
        plan.markers.is_empty(),
        "a non-publishing job carries no marker"
    );

    // Prose must not count. `release.yml` documents `cargo publish` and
    // `action-gh-release` in comments across several jobs; attributing those
    // would mark innocent jobs as publishers and, worse, let a real
    // ungated publish hide among false positives.
    let prose_only = "\
jobs:
  plan:
    # Bump procedure when softprops ships a new action-gh-release:
    #   `cargo publish` is permanent, so a manual run must never publish.
    steps:
      - run: echo hello
";
    let quiet = parse_jobs(prose_only);
    assert!(
        quiet[0].markers.is_empty(),
        "markers mentioned only in comments must not mark a job as publishing"
    );
}

//! Pre-flight banner emitted to stderr at the start of an `analyze` run.
//!
//! Two purposes in one box:
//!   1. **Visual preview** — "you are running codelore X.Y on repo R, branch B,
//!      requesting analysis A with options O". A human can confirm intent at a
//!      glance before the 5-30s ingest fires.
//!   2. **Fail-fast gate** — runs cheap pre-flight checks (path is a git repo,
//!      HEAD has commits, output path is writable) BEFORE expensive work. On
//!      failure the banner shows a `✗ <reason>` status line with a hint, and
//!      the CLI exits non-zero without touching the ingest pipeline.
//!
//! Industry-standard styling choices applied:
//! - **stderr** output. Stdout stays clean for CSV/JSON/SARIF piping.
//! - **Auto-disable on non-TTY stderr** (CI logs, file redirects). Override via
//!   `--banner` (clap layer) or by setting `CLICOLOR_FORCE=1`.
//! - **`NO_COLOR` respected** (per <https://no-color.org/>) — env-var presence
//!   strips all ANSI escapes regardless of TTY status.
//! - **Unicode box drawing** (`─`). Assumes UTF-8 terminal — every macOS, Linux,
//!   and Windows 10+ terminal supports it.
//! - **anstyle** (no allocations for color codes; same crate the cargo team
//!   uses; already in our tree transitively via clap).

use std::fmt::Write as _;
use std::io::IsTerminal as _;

use anstyle::{AnsiColor, Style};

/// Fixed 72-character box width — fits 80-col terminals with comfortable
/// padding and renders cleanly in split-screen panes. Adaptive width via
/// `terminal_size` was considered and rejected: predictable output is
/// more debuggable, and the box content is short enough that 72 cols
/// never truncates a real value in practice.
const BOX_WIDTH: usize = 72;

/// Outcome of the pre-flight gate. `Ready` is the only variant that allows
/// the analysis to proceed; everything else aborts with a clean banner-shaped
/// error and a non-zero exit code.
///
/// Each failure variant carries the data needed to render a one-line hint
/// telling the user how to fix it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preflight {
    /// All checks passed; the ingest pipeline can run.
    Ready,
    /// The given `--repo` path resolves but isn't a git repository.
    /// (`gix::open()` reports `RepositoryMissing` or similar.)
    NotARepository { repo_path: String },
    /// Repository opens but has no commits (`git init` with nothing staged
    /// yet). Every analysis needs history; abort with a clear message.
    EmptyRepository,
    /// `--output PATH` was set but PATH's parent directory doesn't exist or
    /// isn't writable. Catching this BEFORE the 30-second ingest is the whole
    /// point of a pre-flight.
    OutputNotWritable { path: String, reason: String },
    /// `--repo PATH` doesn't exist on the filesystem.
    RepoPathMissing { repo_path: String },
}

impl Preflight {
    #[must_use]
    pub fn is_ready(&self) -> bool {
        matches!(self, Preflight::Ready)
    }
}

/// Data the banner needs to render. Built by the caller (CLI) from `Options` +
/// `GixRepo` + pre-flight results; the banner module itself stays free of repo
/// and clap imports so it can be snapshot-tested without spinning either up.
#[derive(Debug, Clone)]
pub struct Banner<'a> {
    pub codelore_version: &'a str,
    pub gix_version: &'a str,
    pub duckdb_version: &'a str,
    pub repo_path: String,
    /// `None` when HEAD is detached or the repo is unopened.
    pub branch: Option<String>,
    /// Short (7-char) HEAD SHA. `None` for a fresh repo with no commits.
    pub head_short: Option<String>,
    pub analysis: &'a str,
    /// One-line tuning summary, e.g. `"min-revs=5, rows=10"`.
    pub options_summary: String,
    /// Aggregate of the pre-flight checks. `Ready` → green status; anything
    /// else → red status with a fix-hint line.
    pub preflight: Preflight,
}

impl Banner<'_> {
    /// Render the banner as a single multi-line string. `use_color` toggles
    /// ANSI escape emission — should reflect `should_color()` for production
    /// callers and `false` in snapshot tests.
    #[must_use]
    pub fn render(&self, use_color: bool) -> String {
        let title = Style::new().bold();
        let label = Style::new().dimmed();
        let head_sha = Style::new().fg_color(Some(AnsiColor::Yellow.into())).bold();
        let rule_style = Style::new().dimmed();
        let ok = Style::new().fg_color(Some(AnsiColor::Green.into())).bold();
        let fail = Style::new().fg_color(Some(AnsiColor::Red.into())).bold();

        let s = |sty: Style| if use_color { sty } else { Style::new() };
        let rule = "─".repeat(BOX_WIDTH);

        // Header band — left: bold "codelore X.Y.Z" / right: dim deps.
        let left = format!("codelore {}", self.codelore_version);
        let right = format!("gix {} · duckdb {}", self.gix_version, self.duckdb_version);
        let inner_width = BOX_WIDTH.saturating_sub(2);
        let pad = inner_width
            .saturating_sub(left.len())
            .saturating_sub(right.len());

        let mut out = String::with_capacity(BOX_WIDTH * 10);
        let _ = writeln!(out, "{}{rule}{:#}", s(rule_style), s(rule_style));
        let _ = writeln!(
            out,
            " {}{left}{:#}{} {}{right}{:#}",
            s(title),
            s(title),
            " ".repeat(pad),
            s(label),
            s(label),
        );
        let _ = writeln!(out, "{}{rule}{:#}", s(rule_style), s(rule_style));

        let _ = writeln!(
            out,
            " {}Repo:{:#}     {}",
            s(label),
            s(label),
            self.repo_path
        );

        let branch_line = match (&self.branch, &self.head_short) {
            (Some(b), Some(sha)) => format!("{b} @ {}{sha}{:#}", s(head_sha), s(head_sha)),
            (Some(b), None) => format!("{b} (no commits yet)"),
            (None, Some(sha)) => format!("(detached) @ {}{sha}{:#}", s(head_sha), s(head_sha)),
            (None, None) => "(none — no HEAD)".to_string(),
        };
        let _ = writeln!(out, " {}Branch:{:#}   {}", s(label), s(label), branch_line);

        let _ = writeln!(
            out,
            " {}Analysis:{:#} {}  ({})",
            s(label),
            s(label),
            self.analysis,
            self.options_summary
        );

        // Status row — green ✓ when ready, red ✗ with a one-line hint otherwise.
        let (mark_style, mark, status_text, hint) = match &self.preflight {
            Preflight::Ready => (s(ok), "✓", "ready".to_string(), None),
            Preflight::NotARepository { repo_path } => (
                s(fail),
                "✗",
                "not a git repository".to_string(),
                Some(format!(
                    "run `git init` in {repo_path}, or pass --repo to a git-managed directory"
                )),
            ),
            Preflight::EmptyRepository => (
                s(fail),
                "✗",
                "repository has no commits".to_string(),
                Some("codelore needs at least one commit to compute history-based metrics".into()),
            ),
            Preflight::OutputNotWritable { path, reason } => (
                s(fail),
                "✗",
                format!("--output not writable: {path}"),
                Some(format!("reason: {reason}")),
            ),
            Preflight::RepoPathMissing { repo_path } => (
                s(fail),
                "✗",
                "repo path does not exist".to_string(),
                Some(format!("no such directory: {repo_path}")),
            ),
        };
        let _ = writeln!(
            out,
            " {}Status:{:#}   {}{mark}{:#} {status_text}",
            s(label),
            s(label),
            mark_style,
            mark_style,
        );
        if let Some(h) = hint {
            let _ = writeln!(out, " {}Hint:{:#}     {h}", s(label), s(label));
        }

        let _ = writeln!(out, "{}{rule}{:#}", s(rule_style), s(rule_style));

        out
    }
}

/// Footer summary printed to stderr at the end of a successful analyze run.
/// Bracketing the header banner: header tells the user "this is what I'm
/// about to do", footer confirms "done — here's how long it took". Skipped
/// when the header was skipped (same TTY / `--no-banner` policy), so a
/// piped-into-`tee` invocation stays clean.
#[derive(Debug, Clone)]
pub struct Footer<'a> {
    pub analysis: &'a str,
    pub elapsed: std::time::Duration,
    /// Optional row-count to surface in the footer. `None` skips the row
    /// fragment entirely so emitters that don't have a meaningful row count
    /// (sqlite full-fact-store dump) can still call render.
    pub rows: Option<usize>,
}

impl Footer<'_> {
    #[must_use]
    pub fn render(&self, use_color: bool) -> String {
        let ok = Style::new().fg_color(Some(AnsiColor::Green.into())).bold();
        let label = Style::new().dimmed();
        let rule_style = Style::new().dimmed();
        let s = |sty: Style| if use_color { sty } else { Style::new() };
        let rule = "─".repeat(BOX_WIDTH);

        let elapsed = humanize_duration(self.elapsed);
        let rows_fragment = match self.rows {
            Some(n) => format!(" — {n} row{}", if n == 1 { "" } else { "s" }),
            None => String::new(),
        };

        let mut out = String::with_capacity(BOX_WIDTH * 3);
        let _ = writeln!(out, "{}{rule}{:#}", s(rule_style), s(rule_style));
        let _ = writeln!(
            out,
            " {}✓{:#} {} completed in {}{rows_fragment}",
            s(ok),
            s(ok),
            self.analysis,
            elapsed,
        );
        let _ = writeln!(out, "{}{rule}{:#}", s(rule_style), s(rule_style));
        // `label` is part of the same rendering vocabulary as the header
        // (matching dimmed labels for "Repo:"/"Branch:"/etc.); kept in scope
        // so future footer rows (e.g. cache hit / output path) can use it
        // without re-importing styles.
        let _ = label;
        out
    }
}

/// Convert a `Duration` to a short human-readable string. Breakpoints chosen
/// to match the cargo / pytest / npm convention:
/// - `< 1ms` → `"<1ms"` (avoid useless 0.0s for very fast no-op runs)
/// - `< 1s` → `"234ms"` (sub-second resolution in ms)
/// - `< 60s` → `"4.3s"` (one decimal place; resolution-vs-noise compromise)
/// - `< 1h` → `"2m 34s"` (no fractional component above a minute)
/// - `≥ 1h` → `"1h 5m 12s"` (rare for codelore but cheap to support)
#[must_use]
pub fn humanize_duration(d: std::time::Duration) -> String {
    let total_ms = d.as_millis();
    if total_ms < 1 {
        return "<1ms".to_string();
    }
    if total_ms < 1_000 {
        return format!("{total_ms}ms");
    }
    let total_secs = d.as_secs_f64();
    if total_secs < 60.0 {
        return format!("{total_secs:.1}s");
    }
    let total_secs_int = d.as_secs();
    let hours = total_secs_int / 3600;
    let minutes = (total_secs_int % 3600) / 60;
    let seconds = total_secs_int % 60;
    if hours > 0 {
        return format!("{hours}h {minutes}m {seconds}s");
    }
    format!("{minutes}m {seconds}s")
}

/// Decision policy for whether to colorize the banner. Mirrors cargo / clap:
///   1. `NO_COLOR` env var present (and non-empty) → never colorize.
///   2. `CLICOLOR_FORCE` present (non-zero) → always colorize.
///   3. Otherwise: colorize iff stderr is a TTY.
#[must_use]
pub fn should_color() -> bool {
    if std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()) {
        return false;
    }
    if std::env::var("CLICOLOR_FORCE").is_ok_and(|v| v != "0" && !v.is_empty()) {
        return true;
    }
    std::io::stderr().is_terminal()
}

/// Decision policy for whether to print the banner AT ALL. Three switches,
/// most-restrictive wins:
///   1. `quiet=true` → always off (matches the existing log suppression).
///   2. `no_banner=true` → always off (targeted opt-out).
///   3. Otherwise → on iff stderr is a TTY (so file redirects stay clean).
///
/// Note: even when `should_print` returns `false`, callers MUST still print
/// the banner when the pre-flight status is non-`Ready` — failure feedback
/// is too important to swallow on a piped invocation.
#[must_use]
pub fn should_print(quiet: bool, no_banner: bool) -> bool {
    if quiet || no_banner {
        return false;
    }
    std::io::stderr().is_terminal()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_fixture() -> Banner<'static> {
        Banner {
            codelore_version: "0.1.2",
            gix_version: "0.85.0",
            duckdb_version: "1.10504.0",
            repo_path: "/Users/emrec/Projects/greenfield-api".to_string(),
            branch: Some("feature/GFBUGS-132-tr2-licenses-modify-perm".to_string()),
            head_short: Some("a891295".to_string()),
            analysis: "hotspots",
            options_summary: "min-revs=5, rows=10".to_string(),
            preflight: Preflight::Ready,
        }
    }

    #[test]
    fn render_plain_is_pure_ascii_plus_box_drawing() {
        let plain = ready_fixture().render(false);
        assert!(
            !plain.contains('\x1b'),
            "plain render leaked an escape: {plain:?}"
        );
        for needle in [
            "codelore 0.1.2",
            "gix 0.85.0 · duckdb 1.10504.0",
            "Repo:",
            "Branch:",
            "Analysis:",
            "hotspots",
            "min-revs=5, rows=10",
            "Status:",
            "ready",
            "─",
        ] {
            assert!(plain.contains(needle), "missing {needle:?} in:\n{plain}");
        }
    }

    #[test]
    fn render_styled_contains_ansi_escapes() {
        let styled = ready_fixture().render(true);
        assert!(
            styled.contains('\x1b'),
            "styled render produced no ANSI bytes"
        );
        assert!(styled.contains("codelore 0.1.2"));
        assert!(styled.contains("hotspots"));
    }

    #[test]
    fn detached_head_renders_without_branch() {
        let mut b = ready_fixture();
        b.branch = None;
        let out = b.render(false);
        assert!(out.contains("(detached)"));
        assert!(out.contains("a891295"));
    }

    #[test]
    fn not_a_repo_renders_failure_with_hint() {
        let mut b = ready_fixture();
        b.preflight = Preflight::NotARepository {
            repo_path: "/tmp/random-folder".to_string(),
        };
        let out = b.render(false);
        assert!(out.contains("✗"), "missing fail mark");
        assert!(out.contains("not a git repository"));
        assert!(out.contains("Hint:"));
        assert!(out.contains("git init"));
    }

    #[test]
    fn empty_repo_renders_failure_with_hint() {
        let mut b = ready_fixture();
        b.head_short = None;
        b.preflight = Preflight::EmptyRepository;
        let out = b.render(false);
        assert!(out.contains("repository has no commits"));
        assert!(out.contains("Hint:"));
        assert!(out.contains("at least one commit"));
    }

    #[test]
    fn output_not_writable_renders_failure_with_reason() {
        let mut b = ready_fixture();
        b.preflight = Preflight::OutputNotWritable {
            path: "/root/out.csv".to_string(),
            reason: "Permission denied (os error 13)".to_string(),
        };
        let out = b.render(false);
        assert!(out.contains("--output not writable"));
        assert!(out.contains("/root/out.csv"));
        assert!(out.contains("Permission denied"));
    }

    #[test]
    fn preflight_is_ready_classification() {
        assert!(Preflight::Ready.is_ready());
        assert!(!Preflight::EmptyRepository.is_ready());
        assert!(
            !Preflight::NotARepository {
                repo_path: "x".into()
            }
            .is_ready()
        );
    }

    // The `should_color()` env-var policy is tested via the integration test
    // suite (see `tests/banner_color_policy_test.rs`) rather than here —
    // `std::env::set_var` requires `unsafe` in Rust 2024, and the workspace
    // `unsafe_code = "forbid"` lint blocks that even in `#[cfg(test)]` blocks.
    // The integration test runs in a fresh process so env mutations are safe.

    #[test]
    fn humanize_duration_breakpoints() {
        use std::time::Duration;
        // `Duration::new(secs, nanos)` is used over `from_secs(60)` for the
        // minute-and-up values because clippy flags "smaller unit when larger
        // would be more readable" — `new` sidesteps the lint while keeping
        // the breakpoint assertions explicit about what each input is.
        assert_eq!(humanize_duration(Duration::from_micros(500)), "<1ms");
        assert_eq!(humanize_duration(Duration::from_millis(234)), "234ms");
        assert_eq!(humanize_duration(Duration::from_millis(4_300)), "4.3s");
        assert_eq!(humanize_duration(Duration::new(59, 0)), "59.0s");
        assert_eq!(humanize_duration(Duration::new(60, 0)), "1m 0s");
        assert_eq!(humanize_duration(Duration::new(154, 0)), "2m 34s");
        assert_eq!(humanize_duration(Duration::new(3_912, 0)), "1h 5m 12s");
    }

    #[test]
    fn footer_renders_with_row_count() {
        let f = Footer {
            analysis: "hotspots",
            elapsed: std::time::Duration::from_millis(4_321),
            rows: Some(10),
        };
        let out = f.render(false);
        assert!(out.contains("hotspots"));
        assert!(out.contains("completed in 4.3s"));
        assert!(out.contains("10 rows"));
        assert!(!out.contains('\x1b'), "plain footer leaked an escape");
    }

    #[test]
    fn footer_singular_row() {
        let f = Footer {
            analysis: "summary",
            elapsed: std::time::Duration::from_millis(50),
            rows: Some(1),
        };
        let out = f.render(false);
        assert!(out.contains("1 row"));
        assert!(!out.contains("rows"), "should be singular for n=1");
    }

    #[test]
    fn footer_omits_row_count_when_none() {
        let f = Footer {
            analysis: "sqlite",
            elapsed: std::time::Duration::from_secs(12),
            rows: None,
        };
        let out = f.render(false);
        assert!(out.contains("sqlite"));
        assert!(out.contains("12.0s"));
        assert!(!out.contains("row"));
    }
}

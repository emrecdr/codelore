//! Gate-run ledger: append-only JSONL record of every evaluated gate.
//!
//! Each `codelore check` or `codelore gate` run appends one
//! [`GateRunRecord`] per evaluated gate to a per-repo JSONL file at
//! `<cache_root>/codelore/<repo_hash_8>/gate_runs.jsonl`.
//!
//! The path reuses the same `<repo_hash_8>` component that `cache.rs`
//! derives for `.duckdb` files, placing history and cache entries under
//! the same per-repo directory. `.jsonl` files are NOT touched by the
//! cache pruner (which matches `.duckdb` only).
//!
//! ## Write contract
//!
//! - File is created on first write (`O_CREAT | O_APPEND`).
//! - Every line is a valid JSON object followed by `\n`.
//! - IO errors on write are logged via `tracing::warn!` and silently
//!   dropped — a ledger write failure must never alter `codelore check`'s
//!   exit code.
//!
//! ## Read contract
//!
//! Lines that fail to parse as [`GateRunRecord`] are skipped and warned;
//! partial writes (e.g. from a crash mid-line) are tolerated.

use std::fmt::Write as FmtWrite;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Result;

/// One evaluated gate from one `codelore check` or `codelore gate`
/// invocation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GateRunRecord {
    /// ISO-8601 UTC timestamp of the check run (`"YYYY-MM-DDTHH:MM:SSZ"`).
    pub ts: String,
    /// Full HEAD SHA at the time of the check.
    pub head_sha: String,
    /// Gate name (e.g. `"code_health_min"`, `"disallow_clone_type_1"`).
    pub gate: String,
    /// Configured threshold value.
    pub threshold: f64,
    /// Measured value. For repo-wide gates without a scalar, the violation
    /// count is used.
    pub value: f64,
    /// Outcome: `"passed"` | `"failed"` | `"degraded"` | `"skipped"`.
    pub verdict: String,
    /// Invocation mode: `"check"` (full-tree gates), `"ratchet"` (snapshot
    /// comparison), or `"gate"` (working-tree change-set gates); reserved
    /// for `"diff"`.
    pub mode: String,
}

/// Compute the per-repo ledger directory:
/// `<cache_root>/codelore/<repo_hash_8>/`
///
/// Delegates to [`crate::cache::repo_cache_dir`] so the derivation is
/// defined in exactly one place (shared with the cache `.duckdb` path and
/// the external-findings sidecar).
#[must_use]
pub fn ledger_dir(cache_root: &Path, repo_path: &Path) -> PathBuf {
    crate::cache::repo_cache_dir(cache_root, repo_path)
}

/// Path to the gate-runs JSONL file for `repo_path`.
#[must_use]
pub fn ledger_path(cache_root: &Path, repo_path: &Path) -> PathBuf {
    ledger_dir(cache_root, repo_path).join("gate_runs.jsonl")
}

/// Append `records` to the gate-run ledger.
///
/// Creates the file (and parent directories) if they do not exist. Each
/// record is pre-assembled into a single buffer (JSON body + `\n`) and
/// emitted with one `write_all`, so the whole line reaches the file in one
/// `write(2)`. Combined with `O_APPEND`, that keeps concurrent writes from
/// parallel check invocations interleaving only at record boundaries — never
/// mid-line — so a reader never sees a physical line holding two half-records.
/// (POSIX guarantees an `O_APPEND` write of at most `PIPE_BUF` bytes is
/// atomic; on Windows the append is best-effort. A `writeln!` on an
/// unbuffered `File` would instead emit the body and the newline as two
/// separate `write(2)` calls, which is the interleaving this avoids.)
///
/// IO errors are **logged and silently dropped** — a ledger write failure
/// must never alter the exit code of `codelore check`.
///
/// # Errors
///
/// Returns `Ok(())` always; IO failures are emitted via `tracing::warn!`.
pub fn append_gate_runs(cache_root: &Path, repo_path: &Path, records: &[GateRunRecord]) {
    if records.is_empty() {
        return;
    }
    let path = ledger_path(cache_root, repo_path);
    if let Some(parent) = path.parent()
        && let Err(e) = fs::create_dir_all(parent)
    {
        tracing::warn!("ledger: could not create dir {}: {e}", parent.display());
        return;
    }
    let file = OpenOptions::new().create(true).append(true).open(&path);
    let mut file = match file {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!("ledger: could not open {}: {e}", path.display());
            return;
        }
    };
    for rec in records {
        match serde_json::to_string(rec) {
            Ok(mut line) => {
                // Pre-assemble body + newline so the record lands in ONE
                // write(2); a `writeln!` would split it into two.
                line.push('\n');
                if let Err(e) = file.write_all(line.as_bytes()) {
                    tracing::warn!("ledger: write failed: {e}");
                }
            }
            Err(e) => {
                tracing::warn!("ledger: serialize failed for gate {}: {e}", rec.gate);
            }
        }
    }
}

/// Read all gate-run records from the ledger, newest-last.
///
/// Malformed lines are skipped with a `tracing::warn!` — the ledger is
/// append-only so a corrupt tail-line (e.g. from a crash mid-write) must
/// not prevent reading earlier valid records.
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] only if the file exists but
/// cannot be opened (permission denied, etc.). A missing file returns an
/// empty vec.
pub fn read_gate_runs(cache_root: &Path, repo_path: &Path) -> Result<Vec<GateRunRecord>> {
    let path = ledger_path(cache_root, repo_path);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = fs::File::open(&path).map_err(|e| {
        crate::CodeLoreError::Analysis(format!("open ledger {}: {e}", path.display()))
    })?;
    let mut records = Vec::new();
    for (lineno, line) in BufReader::new(file).lines().enumerate() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!("ledger: IO error at line {lineno}: {e}");
                continue;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<GateRunRecord>(&line) {
            Ok(rec) => records.push(rec),
            Err(e) => {
                tracing::warn!("ledger: malformed line {lineno} (skipped): {e}");
            }
        }
    }
    Ok(records)
}

/// Return a UTC timestamp string in ISO-8601 format: `"YYYY-MM-DDTHH:MM:SSZ"`.
#[must_use]
pub fn now_utc_ts() -> String {
    // Use std::time — no chrono/time dep needed for second precision.
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    // Manual ISO-8601 from Unix seconds (no leap-second awareness needed).
    let sec = secs % 60;
    let min = (secs / 60) % 60;
    let hour = (secs / 3600) % 24;
    let days = secs / 86400; // days since 1970-01-01
    let (year, month, day) = days_to_ymd(days);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z")
}

/// Convert days-since-epoch to (year, month, day). Gregorian calendar.
fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    // Rata Die algorithm — Gregorian proleptic calendar.
    let mut y = 1970u64;
    loop {
        let leap = is_leap(y);
        let days_in_year = if leap { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        y += 1;
    }
    let leap = is_leap(y);
    let month_days: [u64; 12] = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut mo = 0u64;
    for &md in &month_days {
        if days < md {
            break;
        }
        days -= md;
        mo += 1;
    }
    (y, mo + 1, days + 1)
}

fn is_leap(y: u64) -> bool {
    (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400)
}

/// Print the last `n` gate-run records from `all_records` grouped by
/// `head_sha`, newest group last. Returns the formatted string.
#[must_use]
pub fn format_history(all_records: &[GateRunRecord], n: usize) -> String {
    if all_records.is_empty() {
        return "No gate runs recorded yet.\n".to_string();
    }
    let tail: Vec<&GateRunRecord> = all_records.iter().rev().take(n).collect();
    // Re-reverse to oldest-first within the tail.
    let tail: Vec<&GateRunRecord> = tail.into_iter().rev().collect();

    let mut out = String::new();
    let mut current_sha = "";
    for rec in &tail {
        if rec.head_sha != current_sha {
            current_sha = &rec.head_sha;
            let _ = write!(
                out,
                "\ncommit {}\n",
                &rec.head_sha[..rec.head_sha.len().min(12)]
            );
            out.push_str("  gate                      threshold   value      verdict\n");
            out.push_str("  ─────────────────────────────────────────────────────────\n");
        }
        let _ = writeln!(
            out,
            "  {gate:<26} {threshold:<11.2} {value:<10.2} {verdict}",
            gate = rec.gate,
            threshold = rec.threshold,
            value = rec.value,
            verdict = rec.verdict,
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn sample_record(gate: &str, verdict: &str) -> GateRunRecord {
        GateRunRecord {
            ts: "2026-07-08T10:00:00Z".into(),
            head_sha: "abc123def456".into(),
            gate: gate.into(),
            threshold: 60.0,
            value: 54.2,
            verdict: verdict.into(),
            mode: "check".into(),
        }
    }

    #[test]
    fn append_and_read_round_trips() {
        let dir = tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        let records = vec![
            sample_record("code_health_min", "failed"),
            sample_record("cognitive_max", "passed"),
        ];
        append_gate_runs(dir.path(), &repo, &records);
        let read = read_gate_runs(dir.path(), &repo).expect("read");
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].gate, "code_health_min");
        assert_eq!(read[1].verdict, "passed");
    }

    #[test]
    fn append_writes_exactly_one_physical_line_per_record() {
        // The single-write construction (body + newline in one `write_all`)
        // is the atomicity guarantee. A multiprocess interleaving race is not
        // deterministically testable, so we assert the on-disk shape the
        // construction produces — exactly one physical line per record, each
        // independently parseable — which is precisely what the append
        // guarantees so concurrent runs never fuse two records onto one line.
        let dir = tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        let records = vec![
            sample_record("code_health_min", "failed"),
            sample_record("cognitive_max", "passed"),
            sample_record("hotspot_score_max", "degraded"),
        ];
        append_gate_runs(dir.path(), &repo, &records);
        let raw = fs::read_to_string(ledger_path(dir.path(), &repo)).expect("read raw");
        assert!(raw.ends_with('\n'), "each record is newline-terminated");
        let lines: Vec<&str> = raw.lines().collect();
        assert_eq!(lines.len(), records.len(), "one physical line per record");
        for line in lines {
            serde_json::from_str::<GateRunRecord>(line).expect("each line parses standalone");
        }
    }

    #[test]
    fn missing_ledger_returns_empty() {
        let dir = tempdir().expect("tempdir");
        let repo = dir.path().join("nonexistent_repo");
        let read = read_gate_runs(dir.path(), &repo).expect("read");
        assert!(read.is_empty());
    }

    #[test]
    fn malformed_line_is_skipped() {
        let dir = tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        let path = ledger_path(dir.path(), &repo);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        // Write one valid + one corrupt + one valid line.
        fs::write(
            &path,
            b"{\"ts\":\"2026-07-08T10:00:00Z\",\"head_sha\":\"abc\",\"gate\":\"g1\",\"threshold\":1.0,\"value\":0.5,\"verdict\":\"passed\",\"mode\":\"check\"}\nNOT_JSON\n{\"ts\":\"2026-07-08T10:00:00Z\",\"head_sha\":\"abc\",\"gate\":\"g2\",\"threshold\":1.0,\"value\":0.5,\"verdict\":\"failed\",\"mode\":\"check\"}\n",
        )
        .unwrap();
        let read = read_gate_runs(dir.path(), &repo).expect("read");
        assert_eq!(read.len(), 2, "corrupt line skipped, 2 valid remain");
        assert_eq!(read[0].gate, "g1");
        assert_eq!(read[1].gate, "g2");
    }

    #[test]
    fn append_twice_produces_two_times_n_lines() {
        let dir = tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        let records = vec![sample_record("code_health_min", "failed")];
        append_gate_runs(dir.path(), &repo, &records);
        append_gate_runs(dir.path(), &repo, &records);
        let read = read_gate_runs(dir.path(), &repo).expect("read");
        assert_eq!(read.len(), 2);
    }

    #[test]
    fn degraded_verdict_round_trips() {
        let dir = tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        let records = vec![sample_record("code_health_min", "degraded")];
        append_gate_runs(dir.path(), &repo, &records);
        let read = read_gate_runs(dir.path(), &repo).expect("read");
        assert_eq!(read[0].verdict, "degraded");
    }

    #[test]
    fn format_history_groups_by_sha() {
        let records = vec![
            GateRunRecord {
                ts: "2026-07-08T10:00:00Z".into(),
                head_sha: "sha1abc".into(),
                gate: "code_health_min".into(),
                threshold: 60.0,
                value: 54.2,
                verdict: "failed".into(),
                mode: "check".into(),
            },
            GateRunRecord {
                ts: "2026-07-08T11:00:00Z".into(),
                head_sha: "sha2def".into(),
                gate: "cognitive_max".into(),
                threshold: 30.0,
                value: 25.0,
                verdict: "passed".into(),
                mode: "check".into(),
            },
        ];
        let out = format_history(&records, 20);
        assert!(out.contains("sha1abc"), "first sha present");
        assert!(out.contains("sha2def"), "second sha present");
        assert!(out.contains("code_health_min"));
        assert!(out.contains("failed"));
    }

    #[test]
    fn now_utc_ts_format() {
        let ts = now_utc_ts();
        assert_eq!(ts.len(), 20, "format: YYYY-MM-DDTHH:MM:SSZ");
        assert!(ts.ends_with('Z'));
        assert_eq!(&ts[4..5], "-");
        assert_eq!(&ts[7..8], "-");
        assert_eq!(&ts[10..11], "T");
    }
}

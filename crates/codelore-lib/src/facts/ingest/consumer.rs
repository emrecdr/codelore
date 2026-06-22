//! The Appender-owning consumer side of the ingest pipeline. Drains the
//! producer's `CommitEvent` stream from the bounded channel, resolves author
//! identity, and writes the `commits`, `changes`, `hunks`, and `author_aliases`
//! rows on the connection-owning thread. Also houses the row-append helpers and
//! the entity de-duplication used by both the commit drain and the HEAD-time
//! complexity scan.

use duckdb::Appender;

use super::{FactsDb, IngestStats};
use crate::identity;
use crate::{ChangeType, CodeLoreError, CommitEvent, Result};

pub(super) fn ingest_loop(
    db: &FactsDb,
    rx: crossbeam_channel::Receiver<CommitEvent>,
    team_map: &identity::TeamMap,
    bot_patterns: &identity::BotPatterns,
    paths_filter: &crate::paths_filter::PathsFilter,
) -> Result<IngestStats> {
    use std::collections::HashMap;

    let mut stats = IngestStats::default();
    let mut commits_app = db
        .conn()
        .appender("commits")
        .map_err(|e| CodeLoreError::Analysis(format!("appender commits: {e}")))?;
    let mut changes_app = db
        .conn()
        .appender("changes")
        .map_err(|e| CodeLoreError::Analysis(format!("appender changes: {e}")))?;
    // Hunks appender — populated alongside changes. Pre-F149 fix the
    // `Hunk` payload was parsed by `Repo::diff_hunks`, attached to
    // `FileChange.hunks`, but never written: the table existed in
    // schema, was `DELETE`-cleaned defensively in `apply_grouping`,
    // and was dumped by the SQLite output emitter — all over an
    // empty table. Now `append_change` writes one row per hunk.
    let mut hunks_app = db
        .conn()
        .appender("hunks")
        .map_err(|e| CodeLoreError::Analysis(format!("appender hunks: {e}")))?;

    // Collect unique (raw_email, canonical, is_bot) for deferred author_aliases insert.
    let mut alias_map: HashMap<String, (String, bool)> = HashMap::new();

    for mut event in rx {
        // Resolve canonical author, then apply the team-map projection.
        // Order matters: mailmap normalization happens at walk time (gix);
        // bot detection happens in parallel here; team-map is the LAST
        // projection so it takes the already-normalized identity. The
        // result lands on `event.canonical_author` so `append_commit`
        // (which reads that field) picks it up too.
        let pre_team_canonical = event
            .canonical_author
            .as_deref()
            .unwrap_or(&event.author_email);
        let canonical = identity::apply_team_map(team_map, pre_team_canonical).to_string();
        if !team_map.is_empty() {
            event.canonical_author = Some(canonical.clone());
        }
        let bot = bot_patterns.is_bot(&event.author_email, &event.author_name);
        // Re-classify ai_attribution through the user-extensible patterns
        // too. The walker computes it with the defaults-only free function
        // (no repo_path in scope there); we overwrite here so projects with
        // internal bot accounts get them tagged as `ai-authored` instead
        // of being miscounted as human contributors.
        event.ai_attribution = Some(
            identity::ai_attribution_with(
                bot_patterns,
                &event.author_email,
                &event.author_name,
                &event.message,
            )
            .to_string(),
        );
        alias_map
            .entry(event.author_email.clone())
            .or_insert((canonical, bot));

        append_commit(&mut commits_app, &event)?;
        for ch in &event.changes {
            // Skip paths excluded by --exclude, .gitignore (unless the
            // user passed --include-ignored), .git/info/exclude, or
            // .codeloreignore. Operates on the gix-emitted POSIX path
            // (already repo-relative). Same filter governs the
            // HEAD-time complexity + clones walks so the fact store
            // stays internally consistent.
            let rel_path = std::path::Path::new(&ch.path);
            if crate::paths_filter::is_git_metadata(rel_path)
                || paths_filter.is_excluded(rel_path, false)
            {
                continue;
            }
            append_change(&mut changes_app, &mut hunks_app, &event.rev, ch)?;
            stats.changes_ingested += 1;
        }
        stats.commits_ingested += 1;
    }
    commits_app
        .flush()
        .map_err(|e| CodeLoreError::Analysis(format!("flush commits: {e}")))?;
    changes_app
        .flush()
        .map_err(|e| CodeLoreError::Analysis(format!("flush changes: {e}")))?;
    hunks_app
        .flush()
        .map_err(|e| CodeLoreError::Analysis(format!("flush hunks: {e}")))?;

    // Populate author_aliases table.
    let mut aliases_app = db
        .conn()
        .appender("author_aliases")
        .map_err(|e| CodeLoreError::Analysis(format!("appender author_aliases: {e}")))?;
    for (raw_email, (canonical, is_bot)) in &alias_map {
        use duckdb::params;
        aliases_app
            .append_row(params![raw_email, canonical, is_bot])
            .map_err(|e| CodeLoreError::Analysis(format!("append author_alias: {e}")))?;
    }
    aliases_app
        .flush()
        .map_err(|e| CodeLoreError::Analysis(format!("flush author_aliases: {e}")))?;

    Ok(stats)
}

/// Format a `time::OffsetDateTime` as `YYYY-MM-DD HH:MM:SS` (UTC, no tz
/// suffix) for the `DuckDB` `TIMESTAMP` column. We don't enable the `time`
/// crate's `formatting` feature workspace-wide, so hand-format here.
///
/// The input is converted to UTC before formatting — the original tz
/// offset is discarded at the schema boundary (schema v2 doc explains the
/// tz-preservation roadmap).
fn format_timestamp(ts: time::OffsetDateTime) -> String {
    let utc = ts.to_offset(time::UtcOffset::UTC);
    let y = utc.year();
    let m = utc.month() as u8;
    let d = utc.day();
    let hh = utc.hour();
    let mm = utc.minute();
    let ss = utc.second();
    format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02}")
}

fn append_commit(app: &mut Appender<'_>, e: &CommitEvent) -> Result<()> {
    use duckdb::params;
    let date_str = format_timestamp(e.date);
    let committer_date_str = format_timestamp(e.committer_date);
    let canonical = e
        .canonical_author
        .as_deref()
        .unwrap_or(&e.author_email)
        .to_string();
    let ai_attr = e.ai_attribution.as_deref().map(str::to_string);
    app.append_row(params![
        e.rev,
        e.author_email,
        e.author_name,
        e.committer_email,
        canonical,
        ai_attr,
        date_str,
        committer_date_str,
        e.message,
        e.parents.len() > 1,
        i32::try_from(e.parents.len()).unwrap_or(i32::MAX),
        // Kamei nulls — Plan 4 fills
        Option::<i32>::None,
        Option::<i32>::None,
        Option::<i32>::None,
        Option::<f64>::None,
        Option::<i32>::None,
        Option::<i32>::None,
        Option::<f64>::None,
        Option::<bool>::None,
        Option::<i32>::None,
        Option::<f64>::None,
        Option::<i32>::None,
        Option::<i32>::None,
        Option::<f64>::None,
        Option::<i32>::None,
    ])
    .map_err(|err| CodeLoreError::Analysis(format!("append commit: {err}")))?;
    Ok(())
}

fn append_change(
    changes_app: &mut Appender<'_>,
    hunks_app: &mut Appender<'_>,
    rev: &str,
    ch: &crate::FileChange,
) -> Result<()> {
    use duckdb::params;
    let (type_str, rename_from, similarity) = match &ch.change_type {
        ChangeType::Added => ("added", None, None),
        ChangeType::Modified => ("modified", None, None),
        ChangeType::Deleted => ("deleted", None, None),
        ChangeType::Renamed { from, similarity } => {
            ("renamed", Some(from.as_str()), Some(i32::from(*similarity)))
        }
        ChangeType::Copied { from, similarity } => {
            ("copied", Some(from.as_str()), Some(i32::from(*similarity)))
        }
        ChangeType::BinaryOrUnknown => ("binary", None, None),
    };
    changes_app
        .append_row(params![
            rev,
            ch.path,
            type_str,
            rename_from,
            similarity,
            i32::try_from(ch.loc_added).unwrap_or(i32::MAX),
            i32::try_from(ch.loc_deleted).unwrap_or(i32::MAX),
        ])
        .map_err(|err| CodeLoreError::Analysis(format!("append change: {err}")))?;
    // Hunks — one row per `@@ -old_start,old_lines +new_start,new_lines @@`
    // header. The schema requires the composite key (rev, path,
    // old_start, new_start) to be unique within a single ingest; the
    // gix and git-cli backends both parse hunk headers from `git diff`
    // / gix-diff which guarantees uniqueness per (file, commit). The
    // four u32 fields can never be NULL in Rust, so the new NOT NULL
    // columns in schema_v1.sql match the producer side.
    for hunk in &ch.hunks {
        hunks_app
            .append_row(params![
                rev,
                ch.path,
                i32::try_from(hunk.old_start).unwrap_or(i32::MAX),
                i32::try_from(hunk.old_lines).unwrap_or(i32::MAX),
                i32::try_from(hunk.new_start).unwrap_or(i32::MAX),
                i32::try_from(hunk.new_lines).unwrap_or(i32::MAX),
            ])
            .map_err(|err| CodeLoreError::Analysis(format!("append hunk: {err}")))?;
    }
    Ok(())
}

/// De-duplicate a list of entities by `(name, start_line, end_line)`,
/// preserving first-occurrence order, and disambiguate the surviving
/// entries by suffixing the line range to the name so the downstream
/// DB PK `(path, name, rev)` cannot conflict on duplicate-named rows.
///
/// Tree-sitter walkers report multiple anonymous functions per file
/// with identical `name` (`"<anonymous>"` or empty string for closures,
/// lambdas, generator expressions) and language overloads can produce
/// identical names too (C++ `foo(int)` vs `foo(double)`, Java
/// constructors, Python decorated wrappers). Without disambiguation
/// every duplicate-named entry after the first violates the
/// `entities`/`complexity_metrics` PK on insert — the data lands in
/// Rust memory but the DB rejects the second row with a UNIQUE-
/// constraint violation, aborting ingest on any closures-heavy or
/// overload-heavy codebase.
///
/// The fix rewrites the entity name to
/// `"{original_name}@{start_line}-{end_line}"` (or
/// `"<anonymous>@{start_line}-{end_line}"` for unnamed entities). The
/// line range is the stable identity for unnamed entities and the
/// unique key the PK needs without a schema migration.
pub(super) fn dedup_entities(
    entities: Vec<crate::complexity::ComplexityEntity>,
) -> Vec<crate::complexity::ComplexityEntity> {
    let mut seen: std::collections::HashSet<(String, u32, u32)> = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(entities.len());
    for mut ent in entities {
        let key = (ent.name.clone(), ent.start_line, ent.end_line);
        if !seen.insert(key) {
            continue;
        }
        ent.name = if ent.name.is_empty() {
            format!("<anonymous>@{}-{}", ent.start_line, ent.end_line)
        } else {
            format!("{}@{}-{}", ent.name, ent.start_line, ent.end_line)
        };
        out.push(ent);
    }
    out
}

/// Safely clamp a finite non-negative f64 to i32 range.
fn f64_to_i32_clamped(v: f64) -> i32 {
    if v.is_finite() && v >= 0.0 {
        #[allow(clippy::cast_possible_truncation)]
        let clamped = v.round().min(f64::from(i32::MAX)) as i32;
        clamped
    } else {
        0
    }
}

pub(super) fn append_entity_row(
    app: &mut duckdb::Appender<'_>,
    path: &str,
    ent: &crate::complexity::ComplexityEntity,
    rev: &str,
) -> Result<()> {
    use duckdb::params;
    app.append_row(params![
        path,
        ent.name,
        ent.kind,
        i32::try_from(ent.start_line).unwrap_or(i32::MAX),
        i32::try_from(ent.end_line).unwrap_or(i32::MAX),
        rev,
        rev,
    ])
    .map_err(|e| CodeLoreError::Analysis(format!("append entity: {e}")))
}

pub(super) fn append_metric_row(
    app: &mut duckdb::Appender<'_>,
    path: &str,
    ent: &crate::complexity::ComplexityEntity,
    rev: &str,
) -> Result<()> {
    use duckdb::params;
    app.append_row(params![
        path,
        ent.name,
        rev,
        f64_to_i32_clamped(ent.cyclomatic),
        f64_to_i32_clamped(ent.cognitive),
        ent.halstead_volume,
        ent.halstead_difficulty,
        ent.halstead_effort,
        ent.mi,
        i32::try_from(ent.nom).unwrap_or(i32::MAX),
        i32::try_from(ent.nexits).unwrap_or(i32::MAX),
        i32::try_from(ent.loc).unwrap_or(i32::MAX),
        i32::try_from(ent.sloc).unwrap_or(i32::MAX),
        i32::try_from(ent.max_nesting).unwrap_or(i32::MAX),
        ent.mean_nesting,
        ent.sd_nesting,
        i32::try_from(ent.total_nesting).unwrap_or(i32::MAX),
    ])
    .map_err(|e| CodeLoreError::Analysis(format!("append metric: {e}")))
}

//! Stream commits from a Repo into `DuckDB` via the
//! N gix workers → bounded crossbeam channel → 1 Appender thread pattern.
//! See spec §3.2.2.
//!
//! Threading model: `duckdb::Connection` contains `RefCell`, which is `!Sync`.
//! Therefore `&FactsDb` is `!Send` and the Appender must run on the thread that
//! owns the `FactsDb`. We flip the design: the **producer** runs in a scoped
//! spawned thread (Repo is `Send + Sync`) and the **consumer** (Appender) runs
//! on the calling thread.

use crossbeam_channel::bounded;
use duckdb::Appender;

use super::FactsDb;
use crate::identity;
use crate::repo::Repo;
use crate::{ChangeType, CodeLoreError, CommitEvent, Options, Result};

const CHANNEL_CAPACITY: usize = 64;

#[derive(Debug, Default)]
pub struct IngestStats {
    pub commits_ingested: usize,
    pub changes_ingested: usize,
    /// Plan 8 §4: number of clone-family member rows inserted into the
    /// `clones` table during HEAD-time extraction. `0` if no clones found
    /// or no Tier-1 source files exist.
    pub clones_ingested: usize,
}

impl FactsDb {
    /// # Panics
    ///
    /// Panics if the producer thread panics (internal logic error, not expected in normal use).
    pub fn ingest<R: Repo>(&self, repo: &R, opts: &Options) -> Result<IngestStats> {
        // Load the team-map ONCE before walk starts. Auto-discover
        // `.codelore-teams` in the repo root if `--team-map-file` wasn't
        // passed. Empty map means the projection is a no-op (the apply
        // helper passes through unmatched authors).
        let team_map_path = opts
            .team_map_file
            .clone()
            .or_else(|| crate::identity::discover_team_map(&opts.repo_path));
        let team_map = crate::identity::team_map::load(team_map_path.as_deref())?;
        // Same shape as team_map: load `.codelorebots` ONCE here so the
        // ingest_loop can route bot detection AND ai_attribution through
        // the user-extensible patterns. Before this, the production code
        // called the free `identity::is_bot` / `identity::ai_attribution`
        // functions which only see DEFAULT_BOT_PATTERNS — `.codelorebots`
        // was loaded into a struct that nothing in production ever
        // consulted (extension hook was dead code).
        let bot_patterns = crate::identity::BotPatterns::from_repo(&opts.repo_path);

        // Plan 1: single producer gix walker → bounded channel → Appender on calling thread.
        // Plan 4 will fan out N producers.
        let (tx, rx) = bounded::<CommitEvent>(CHANNEL_CAPACITY);

        let stats = std::thread::scope(|s| -> Result<IngestStats> {
            // Producer: runs in a scoped thread. Repo: Send + Sync, opts borrows fine.
            let producer = s.spawn(|| -> Result<()> {
                let walk = repo.walk_commits(opts)?;
                for event in walk {
                    let event = event?;
                    tx.send(event)
                        .map_err(|e| CodeLoreError::Analysis(format!("channel send: {e}")))?;
                }
                drop(tx); // Signals consumer to stop.
                Ok(())
            });

            // Consumer: runs on the calling thread — FactsDb / Connection stays single-threaded.
            // Combined path filter — respects --exclude + .gitignore +
            // .codeloreignore for the commit-walk ingest path. F30 fix
            // (rel-path matching) preserved by PathsFilter internals.
            let paths_filter = crate::paths_filter::PathsFilter::from_opts(opts)?;
            let stats = ingest_loop(self, rx, &team_map, &bot_patterns, &paths_filter)?;

            // Map a panic in the commit-walker thread into a typed
            // `CodeLoreError::Repo` so `main()`'s chain-walker can
            // derive the correct spec §6.6 exit code (3 = repo).
            // Using `.expect()` here would surface the panic as
            // `thread 'main' panicked at 'producer panicked'` + exit
            // 101 — bypassing the typed-error chain and giving the
            // operator no breadcrumb back to the failing commit walk.
            let join_result = producer
                .join()
                .map_err(|payload| CodeLoreError::Repo(format_panic_payload(&payload)))?;
            join_result?;
            Ok(stats)
        })?;

        // Populate entities + complexity_metrics from blobs at HEAD via
        // `Repo::read_blob_at_head`. Avoids dirty-tree contamination and
        // works on bare repos (no working tree). Falls back to
        // `std::fs::read` for backends without a blob implementation
        // (default trait impl returns `Ok(None)`).
        self.ingest_complexity_at_head(repo, opts)?;

        // Plan 4: populate the Kamei 14-feature change vector via SQL UPDATE pass.
        // Kamei history (ndev, nuc, age) joins changes-to-changes on path;
        // without lineage, renamed files lose all their pre-rename history.
        // Routing through `changes_lineage` merges histories under the canonical
        // post-rename name.
        crate::kamei::enrich(self, opts.use_canonical_lineage)?;

        // Plan 8 §4: populate the `clones` table at HEAD so the
        // `clone-coupling` analysis (§6) can JOIN against it. Honors
        // `opts.min_clone_node_count` and `opts.exclude_patterns` (set via
        // `--exclude` + `.codeloreignore`).
        let clones_n = self.populate_clones_at_head(repo, opts)?;

        // Populate the `imports` table at HEAD so the layered-
        // architecture rules, god-class detector, and architecture
        // force-graph analyses can JOIN against it. Uses the same
        // rayon-then-serial-drain shape as the complexity + clones
        // passes — no new threading primitives.
        let imports_n = self.populate_imports_at_head(repo, opts)?;
        tracing::info!("imports: {imports_n} edges ingested at HEAD across Tier-1 source files");

        // Per-language resolver UPDATE pass. Maps the raw `target`
        // strings to repo-relative tracked paths where resolution
        // succeeds. Covers Rust / Python / JS / TS today; Java FQN
        // → file mapping is project-layout-specific and skipped.
        let resolved_n = self.resolve_imports_at_head()?;
        tracing::info!(
            "imports: {resolved_n} of {imports_n} import edges resolved to tracked paths"
        );

        // PAR-7: architectural grouping. After ingest, rewrite the
        // `changes.path` column to logical group names per --group-file.
        // Runs last so the rewrite sees all change rows from every commit.
        if let Some(group_file) = opts.group_file.as_ref() {
            let group_map = super::groups::GroupMap::from_file(group_file, opts.strict_grouping)
                .map_err(|e| CodeLoreError::Analysis(format!("--group-file: {e}")))?;
            apply_grouping(self, &group_map)?;
        }

        let mut stats = stats;
        stats.clones_ingested = clones_n;
        Ok(stats)
    }

    fn ingest_complexity_at_head<R: crate::repo::Repo>(
        &self,
        repo: &R,
        // After F59 (blob reads instead of disk), the complexity pass
        // no longer needs `opts.repo_path` — every file is sourced via
        // `repo.read_blob_at_head`. Kept on the signature for forward
        // compatibility (future per-language flags may need it).
        _opts: &Options,
    ) -> Result<()> {
        use crate::complexity::{Tier1Language, compute_for_file};
        use rayon::prelude::*;

        let live_paths = query_live_paths(self)?;
        // One HEAD rev shared by every row written in this pass — semantically
        // correct (the complexity was measured against THIS commit, not against
        // a per-path lex-max-of-SHA approximation).
        let head_rev = current_head_rev(self)?;

        // ── Parallel pass ────────────────────────────────────────────────────────
        // Each worker thread reads the file, dispatches the tree-sitter parser,
        // and de-duplicates entities.  `map_init(|| (), ...)` matches the plan's
        // design: no per-thread state is needed because `Parser::new()` is ~3 µs
        // and tree-sitter 0.25.x is both `Send + Sync`.
        // Per-file failures are logged via `tracing::warn!` but do NOT abort the
        // parallel scan; they surface as `None` entries that the serial drain skips.
        //
        // Return type: Vec<Option<(String, Vec<ComplexityEntity>)>>
        //   - None  → file skipped (no Tier-1 lang, unreadable, or parse error)
        //   - Some  → (path, deduped_entities)
        let batches: Vec<Option<(String, Vec<crate::complexity::ComplexityEntity>)>> = live_paths
            .into_par_iter()
            .map_init(
                || (),
                |_state, path| {
                    let lang = Tier1Language::from_path(&path)?;
                    // Prefer the blob at HEAD (works on bare repos, ignores
                    // dirty-tree edits). Fall back to disk if the Repo
                    // backend doesn't implement blob reads OR if the path
                    // exists on disk but isn't tracked at HEAD (a freshly
                    // ingested commit may have added paths not yet in any
                    // tree the backend has cached).
                    let source = match repo.read_blob_at_head(&path) {
                        Ok(Some(b)) => b,
                        Ok(None) => {
                            // Path not tracked at HEAD; skip (matches the
                            // HEAD-time scan semantic of "current files only").
                            tracing::debug!("complexity: {path} not tracked at HEAD; skipping");
                            return None;
                        }
                        Err(e) => {
                            // Object-database error (corrupted pack, missing
                            // shallow object). Surface as a warning and skip
                            // — the rest of the scan can still complete.
                            tracing::warn!("complexity: blob read failed for {path}: {e}");
                            return None;
                        }
                    };
                    // Skip oversized files before handing to tree-sitter.
                    // Without this guard, deeply-nested generated/minified
                    // files (sqlite3.c, .pb.cc, minified .js) can OOM or
                    // stack-overflow the AST walker.
                    // Without this guard, deeply-nested generated/minified
                    // files (sqlite3.c, .pb.cc, minified .js) can OOM or
                    // stack-overflow the AST walker. Log at debug — minified
                    // bundles in `node_modules`-style layouts are the common
                    // case and we'd otherwise drown the console.
                    if source.len() > crate::constants::DEFAULT_MAX_AST_FILE_BYTES {
                        tracing::debug!(
                            "complexity: skipping {path} ({size} bytes > {cap}-byte AST cap; \
                             likely generated/minified; excluded from complexity metrics)",
                            size = source.len(),
                            cap = crate::constants::DEFAULT_MAX_AST_FILE_BYTES,
                        );
                        return None;
                    }
                    // Path only used for error reporting in compute_for_file;
                    // the repo-relative form is more useful than the absolute
                    // working-tree path anyway.
                    let synth_path = std::path::Path::new(&path);
                    let entities = match compute_for_file(synth_path, source, lang) {
                        Ok(v) => v,
                        Err(e) => {
                            tracing::warn!("complexity: parse error {path}: {e}");
                            return None;
                        }
                    };
                    let deduped = dedup_entities(entities);
                    Some((path, deduped))
                },
            )
            .collect();

        // ── Serial drain ─────────────────────────────────────────────────────────
        // `duckdb::Appender<'conn>` is `!Send + !Sync`; it MUST live on the same
        // thread that owns the `Connection`.  We create the Appenders here (on the
        // calling/connection-owning thread) and feed them from the collected Vec.
        let mut entities_app = self
            .conn()
            .appender("entities")
            .map_err(|e| CodeLoreError::Analysis(format!("appender entities: {e}")))?;
        let mut metrics_app = self
            .conn()
            .appender("complexity_metrics")
            .map_err(|e| CodeLoreError::Analysis(format!("appender complexity_metrics: {e}")))?;

        for batch in batches {
            let Some((path, entities)) = batch else {
                continue;
            };
            for ent in &entities {
                append_entity_row(&mut entities_app, &path, ent, &head_rev)?;
                append_metric_row(&mut metrics_app, &path, ent, &head_rev)?;
            }
        }

        entities_app
            .flush()
            .map_err(|e| CodeLoreError::Analysis(format!("flush entities: {e}")))?;
        metrics_app
            .flush()
            .map_err(|e| CodeLoreError::Analysis(format!("flush metrics: {e}")))?;
        Ok(())
    }

    /// Plan 8 §4 Task 15: walk the working tree at HEAD, fingerprint every
    /// function in every Tier-1 file, group by structural digest, and INSERT
    /// one row per clone-family member into the `clones` table. Returns the
    /// number of rows inserted (0 if no clones found or no Tier-1 sources).
    ///
    /// Honors `opts.min_clone_node_count` (default 30) and `opts.exclude_patterns`
    /// (built from `--exclude` flags + `.codeloreignore`).
    fn populate_clones_at_head<R: crate::repo::Repo>(
        &self,
        repo: &R,
        opts: &Options,
    ) -> Result<usize> {
        use crate::clones::{CloneLanguage, extract_functions, group_clones};
        use rayon::prelude::*;

        let head_rev = current_head_rev(self)?;

        // Phase 1 (serial, fast): query the live-at-HEAD path set from
        // DuckDB and filter to Tier-1 source extensions. `query_live_paths`
        // returns rows the ingest already accepted (`PathsFilter` ran at
        // ingest time before any row landed in `changes`), so we don't
        // re-apply `--exclude` / `.gitignore` / `.codeloreignore` here.
        // Switched from `WalkDir::new(&opts.repo_path)` to
        // `query_live_paths` so the clones pass works on bare
        // repositories (no working tree to walk) the same way the
        // complexity pass already does — see `ingest_complexity_at_head`.
        let live_paths = query_live_paths(self)?;
        let candidates: Vec<(String, CloneLanguage)> = live_paths
            .into_iter()
            .filter_map(|rel| {
                let lang = CloneLanguage::from_path(std::path::Path::new(&rel))?;
                Some((rel, lang))
            })
            .collect();

        // Phase 2 (parallel): read each file + run tree-sitter fingerprinting on
        // the rayon pool. Mirrors the complexity pass above. Unreadable files
        // silently yield no fingerprints; extract errors short-circuit the pass
        // via `collect::<Result<_>>`.
        let per_file: Vec<Vec<_>> = candidates
            .into_par_iter()
            .map(|(rel, lang)| -> Result<Vec<_>> {
                // Read the blob at HEAD via the Repo trait. Bare-repo safe
                // and ignores dirty-tree edits. Backends without blob
                // support return Ok(None) — same skip behaviour as the
                // disk-not-found case the previous let-Ok-else handled.
                // Untracked-at-HEAD (Ok(None)) and object-DB errors
                // (Err) both skip the file — non-fatal, the rest of
                // the scan continues.
                let Ok(Some(code)) = repo.read_blob_at_head(&rel) else {
                    return Ok(Vec::new());
                };
                // F10: skip oversized files (generated / minified) before
                // tree-sitter to avoid OOM / stack-overflow on deeply
                // nested generated code. Same cap as complexity pass.
                if code.len() > crate::constants::DEFAULT_MAX_AST_FILE_BYTES {
                    tracing::debug!(
                        "clones: skipping {rel} ({size} bytes > {cap}-byte AST cap)",
                        size = code.len(),
                        cap = crate::constants::DEFAULT_MAX_AST_FILE_BYTES,
                    );
                    return Ok(Vec::new());
                }
                extract_functions(&rel, &code, lang)
                    .map_err(|e| CodeLoreError::Analysis(format!("clones: extract {rel}: {e}")))
            })
            .collect::<Result<Vec<_>>>()?;
        let all_fns: Vec<_> = per_file.into_iter().flatten().collect();

        let groups = group_clones(all_fns, opts.min_clone_node_count);
        if groups.is_empty() {
            return Ok(0);
        }

        // Second pass: INSERT one row per family member into `clones`.
        //
        // `clones` has PRIMARY KEY (clone_group_id, path, function, start_line).
        // In real source that's unique. In minified/bundled output (e.g. webpack
        // and Vite ship files like `dist/assets/index-<hash>.js`) many function
        // expressions are packed onto one line and tree-sitter walks them out
        // with the same `(function_name, start_line)`, so two members of the
        // same group collide on the PK and the appender flush fails — which
        // aborts the entire ingest, even when the user only asked for a non-
        // clones analysis. Dedup in-memory by the PK columns; log the count of
        // collapsed duplicates so the signal isn't silent. Users who want the
        // un-collapsed view should add minified bundles to `.codeloreignore`.
        let mut app = self
            .conn()
            .appender("clones")
            .map_err(|e| CodeLoreError::Analysis(format!("appender clones: {e}")))?;
        let mut n = 0usize;
        let mut collapsed = 0usize;
        let mut seen: std::collections::HashSet<(i64, String, String, u32)> =
            std::collections::HashSet::new();
        for group in groups {
            let clone_group_id = i64::from(group.clone_group_id);
            for member in &group.members {
                use duckdb::params;
                let key = (
                    clone_group_id,
                    member.path.clone(),
                    member.function_name.clone(),
                    member.start_line,
                );
                if !seen.insert(key) {
                    collapsed += 1;
                    continue;
                }
                let fp_bytes: Vec<u8> = member.fingerprint.digest.to_vec();
                app.append_row(params![
                    clone_group_id,
                    fp_bytes,
                    head_rev,
                    member.path,
                    member.function_name,
                    i32::try_from(member.start_line).unwrap_or(i32::MAX),
                    i32::try_from(member.end_line).unwrap_or(i32::MAX),
                    i32::try_from(member.fingerprint.node_count).unwrap_or(i32::MAX),
                    1.0_f64, // Type 1 + Type 2 → exact match; T3 MinHash lands in v1.x
                ])
                .map_err(|e| CodeLoreError::Analysis(format!("append clone row: {e}")))?;
                n += 1;
            }
        }
        if collapsed > 0 {
            tracing::info!(
                "clones: collapsed {collapsed} duplicate member(s) sharing \
                 (group, path, function, start_line) — typically minified/bundled \
                 output; add such files to .codeloreignore to skip them",
            );
        }
        app.flush()
            .map_err(|e| CodeLoreError::Analysis(format!("flush clones appender: {e}")))?;
        Ok(n)
    }

    /// Walk Tier-1 source files at HEAD, tree-sitter-parse each for
    /// import statements, and bulk-insert one row per import edge
    /// into the `imports` table. Returns the total row count for
    /// diagnostics.
    ///
    /// Mirrors `populate_clones_at_head`'s rayon-then-serial-drain
    /// shape: parallel blob-read + extraction, then a single Appender
    /// drain on the connection-owning thread (`DuckDB` Connection is
    /// `!Send + !Sync`). Per-file duplicates (same raw target listed
    /// twice in one file) are deduped before drain to honor the
    /// `(rev, src_path, target)` PRIMARY KEY.
    ///
    /// Every row lands with `resolved=false` / `target_path=NULL`;
    /// the companion `resolve_imports_at_head` UPDATE pass fills in
    /// resolvable targets immediately after.
    fn populate_imports_at_head<R: crate::repo::Repo>(
        &self,
        repo: &R,
        _opts: &Options,
    ) -> Result<usize> {
        use crate::imports::{ImportLanguage, RawImport, extract_imports};
        use rayon::prelude::*;
        use std::collections::HashSet;

        let head_rev = current_head_rev(self)?;

        // Phase 1 (serial): query the live-at-HEAD path set + filter
        // to Tier-1 extensions. Same source-of-truth pattern as the
        // complexity + clones passes (works on bare repos via the
        // gix ODB; PathsFilter already ran at ingest time).
        let live_paths = query_live_paths(self)?;
        let candidates: Vec<(String, ImportLanguage)> = live_paths
            .into_iter()
            .filter_map(|rel| {
                let lang = ImportLanguage::from_path(std::path::Path::new(&rel))?;
                Some((rel, lang))
            })
            .collect();

        // Phase 2 (parallel): read blob + extract imports per file.
        // Errors are logged at warn / debug and the file skipped —
        // a single malformed file shouldn't fail the whole ingest.
        let per_file: Vec<(String, Vec<RawImport>)> = candidates
            .into_par_iter()
            .filter_map(|(rel, lang)| {
                let Ok(Some(code)) = repo.read_blob_at_head(&rel) else {
                    tracing::debug!("imports: {rel} not tracked at HEAD; skipping");
                    return None;
                };
                if code.len() > crate::constants::DEFAULT_MAX_AST_FILE_BYTES {
                    tracing::debug!(
                        "imports: skipping {rel} ({size} bytes > {cap}-byte AST cap)",
                        size = code.len(),
                        cap = crate::constants::DEFAULT_MAX_AST_FILE_BYTES,
                    );
                    return None;
                }
                let imports = match extract_imports(&code, lang) {
                    Ok(v) if !v.is_empty() => v,
                    Ok(_) => return None,
                    Err(e) => {
                        tracing::warn!("imports: extract failed for {rel}: {e}");
                        return None;
                    }
                };
                Some((rel, imports))
            })
            .collect();

        // Phase 3 (serial drain): bulk-insert via the DuckDB Appender
        // on the connection-owning thread. Dedup within each file's
        // target set so the (rev, src_path, target) PK isn't violated
        // by a file that lists the same raw target twice (rare but
        // legal in JS dynamic-import patterns).
        let mut app = self
            .conn()
            .appender("imports")
            .map_err(|e| CodeLoreError::Analysis(format!("appender imports: {e}")))?;
        let mut rows_inserted = 0usize;
        for (path, imports) in per_file {
            let mut seen = HashSet::new();
            for imp in &imports {
                if !seen.insert(imp.target.clone()) {
                    continue;
                }
                app.append_row(duckdb::params![
                    &head_rev,
                    &path,
                    &imp.target,
                    false,
                    Option::<&str>::None,
                    imp.kind.as_str(),
                ])
                .map_err(|e| CodeLoreError::Analysis(format!("append imports row: {e}")))?;
                rows_inserted += 1;
            }
        }
        app.flush()
            .map_err(|e| CodeLoreError::Analysis(format!("flush imports appender: {e}")))?;
        Ok(rows_inserted)
    }

    /// Resolver pass: for every import row whose `target_path` is
    /// still NULL, attempt a per-language resolution against the
    /// live-at-HEAD tracked path set. On a hit, UPDATE the row to
    /// set `resolved=TRUE` and the canonical `target_path`. Returns
    /// the total number of rows successfully resolved.
    ///
    /// Covers Rust `crate::` / `self::` / `super::` paths, Python
    /// relative imports, and JS/TS `./` / `../` paths today. Java
    /// FQN → filesystem-path mapping is project-layout-specific
    /// and is not attempted here.
    fn resolve_imports_at_head(&self) -> Result<usize> {
        use crate::imports::{resolve_js_relative, resolve_python_relative, resolve_rust_path};

        // 1. Build the live-at-HEAD path set as a `HashSet` for O(1)
        //    candidate lookups inside the resolver.
        let live_paths: std::collections::HashSet<String> =
            query_live_paths(self)?.into_iter().collect();

        // 2. Pull every unresolved import row scoped to a language
        //    the multi-language resolver supports (JS/TS, Python,
        //    Rust). Java FQNs are project-layout-specific and stay
        //    deferred until a Java-specific resolver lands.
        let mut stmt = self
            .conn()
            .prepare(
                "SELECT src_path, target FROM imports \
                 WHERE target_path IS NULL \
                   AND (
                       src_path LIKE '%.js' OR src_path LIKE '%.jsx' OR
                       src_path LIKE '%.mjs' OR src_path LIKE '%.cjs' OR
                       src_path LIKE '%.ts' OR src_path LIKE '%.tsx' OR
                       src_path LIKE '%.py' OR src_path LIKE '%.pyi' OR
                       src_path LIKE '%.rs'
                   )",
            )
            .map_err(|e| CodeLoreError::Analysis(format!("prepare imports scan: {e}")))?;
        let candidates: Vec<(String, String)> = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| CodeLoreError::Analysis(format!("query imports scan: {e}")))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| CodeLoreError::Analysis(format!("collect imports scan: {e}")))?;

        // 3. Dispatch to the per-language resolver by file extension.
        //    First-match wins; falls through to `None` for external
        //    targets the resolver can't map to a tracked path.
        let mut hits: Vec<(String, String, String)> = Vec::new();
        for (src_path, target) in candidates {
            let ext = std::path::Path::new(&src_path)
                .extension()
                .and_then(std::ffi::OsStr::to_str);
            let resolved = match ext {
                Some("rs") => resolve_rust_path(&src_path, &target, &live_paths),
                Some("py" | "pyi") => resolve_python_relative(&src_path, &target, &live_paths),
                Some("js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx") => {
                    resolve_js_relative(&src_path, &target, &live_paths)
                }
                _ => None,
            };
            if let Some(resolved_target_path) = resolved {
                hits.push((resolved_target_path, src_path, target));
            }
        }

        // 4. Apply UPDATEs. We need the rev too — there's only one
        //    rev in the imports table per ingest pass (HEAD), so we
        //    grab it once and stamp every UPDATE with it.
        let head_rev = current_head_rev(self)?;
        let mut update_stmt = self
            .conn()
            .prepare(
                "UPDATE imports SET resolved = TRUE, target_path = ? \
                 WHERE rev = ? AND src_path = ? AND target = ?",
            )
            .map_err(|e| CodeLoreError::Analysis(format!("prepare imports update: {e}")))?;
        let mut updated = 0usize;
        for (target_path, src_path, target) in hits {
            update_stmt
                .execute(duckdb::params![target_path, head_rev, src_path, target])
                .map_err(|e| CodeLoreError::Analysis(format!("update imports row: {e}")))?;
            updated += 1;
        }
        Ok(updated)
    }
}

/// Resolve HEAD's rev from the commits table (most recent commit by date).
/// Used to stamp the `rev` column on inserted clone rows.
fn current_head_rev(db: &FactsDb) -> Result<String> {
    // F12 fix: SHA-1 lex order (`rev DESC`) is arbitrary and has no
    // relationship to git topology. When two commits share the exact
    // same second timestamp (common with bot commits, rebases,
    // scripted backfills), the lex tiebreak can pick the PARENT as
    // HEAD instead of the child. We use DuckDB's `rowid` pseudo-column
    // (insertion order) — gix walks reverse-chronologically (newest
    // first), so children arrive BEFORE their parents and get smaller
    // rowids. `rowid ASC` for same-second pairs correctly selects the
    // child as HEAD. Deterministic across runs of the same input.
    let sql = "SELECT rev FROM commits ORDER BY date DESC, rowid ASC LIMIT 1";
    let mut stmt = db
        .conn()
        .prepare(sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare head rev: {e}")))?;
    let mut rows = stmt
        .query([])
        .map_err(|e| CodeLoreError::Analysis(format!("query head rev: {e}")))?;
    if let Some(row) = rows
        .next()
        .map_err(|e| CodeLoreError::Analysis(format!("head rev row: {e}")))?
    {
        Ok(row.get::<_, String>(0).unwrap_or_default())
    } else {
        Ok(String::new())
    }
}

/// Query all paths from `changes` that are not deleted, with their most recent rev.
/// Returns the set of paths that exist at HEAD per the recorded change history.
///
/// "Live at HEAD" = the path's MOST RECENT change event in the ingested history
/// is not a deletion. Earlier implementations used
/// `WHERE change_type != 'deleted' GROUP BY path` which is wrong: it includes
/// any path that has ever had a non-deleted change, even if a later deletion
/// removed it. That produced spurious "cannot read" warnings on every working
/// tree where files had been deleted in commits on the current branch.
///
/// Ordering uses `commits.date` (chronology) — NOT `changes.rev`, which is a
/// SHA-1 hex string whose lex order has no relationship to commit order.
/// `commits.date DESC` first, then `commits.rowid ASC` (insertion order — gix
/// walks reverse-chronologically so children get smaller rowids than parents)
/// as a deterministic, topologically-correct tiebreak. F12 fix: previously
/// used `c.rev DESC` (SHA-1 lex) which is arbitrary and could pick the parent
/// commit as HEAD instead of the child on same-second pairs.
fn query_live_paths(db: &FactsDb) -> Result<Vec<String>> {
    // Hash-grouped `arg_max` picks the most-recent change_type per path in a
    // single streaming pass (O(K) memory, K = distinct paths). The ordering
    // key is `ROW(date, -rowid)` so that struct lex-compare reproduces
    // `ORDER BY date DESC, rowid ASC`: larger date wins, then smaller rowid.
    let sql = "
        WITH latest_per_path AS (
            SELECT
                c.path,
                arg_max(
                    c.change_type,
                    ROW(commits.date, -commits.rowid)
                ) AS change_type
            FROM changes c
            INNER JOIN commits ON commits.rev = c.rev
            GROUP BY c.path
        )
        SELECT path
        FROM latest_per_path
        WHERE change_type != 'deleted'
        ORDER BY path
    ";
    let mut stmt = db
        .conn()
        .prepare(sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare path query: {e}")))?;
    stmt.query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| CodeLoreError::Analysis(format!("query paths: {e}")))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect paths: {e}")))
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
fn dedup_entities(
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

fn append_entity_row(
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

fn append_metric_row(
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

fn ingest_loop(
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
            append_change(&mut changes_app, &event.rev, ch)?;
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

fn append_change(app: &mut Appender<'_>, rev: &str, ch: &crate::FileChange) -> Result<()> {
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
    app.append_row(params![
        rev,
        ch.path,
        type_str,
        rename_from,
        similarity,
        i32::try_from(ch.loc_added).unwrap_or(i32::MAX),
        i32::try_from(ch.loc_deleted).unwrap_or(i32::MAX),
    ])
    .map_err(|err| CodeLoreError::Analysis(format!("append change: {err}")))?;
    Ok(())
}

/// Materialize a session-temporary `changes_bucketed` table that collapses
/// commits within the same `date_trunc(<bucket>, commit.date)` window into
/// a single logical "commit" per (bucket, path). Coupling-family analyses
/// (`coupling`, `clone-coupling` indirectly, `soc`) query this table when
/// `opts.time_bucket.is_some()` so commits landed across the same
/// day/week/month count as one for pair-counting purposes.
///
/// The bucket key (a date string like `2024-01-15` for day-buckets) takes
/// the place of `rev`. Within a bucket, `loc_added` and `loc_deleted` are
/// summed; `change_type` collapses to MAX (string-alphabetical max — close
/// enough since the bucketed-table is consumed only by analyses that care
/// about pair counts, not type details).
///
/// Idempotent: `CREATE OR REPLACE TEMPORARY TABLE`. Call once per analysis
/// run after the main ingest finishes. Cheap — single SQL pass over
/// `changes` JOIN `commits`.
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] on any SQL error.
pub fn materialize_changes_bucketed(
    db: &super::FactsDb,
    bucket: super::super::options::TimeBucket,
    use_lineage: bool,
) -> Result<()> {
    use duckdb::params;
    let unit = bucket.as_sql_unit();
    // When canonical lineage is on, bucket on top of the lineage-resolved
    // view so rename ancestry survives the temporal collapse. Without this,
    // a renamed file's pre- and post-rename commits aggregate under
    // separate paths inside the same bucket — the composition bug §2.1
    // flagged by the 2026-06-09 deep-analysis report.
    let src = if use_lineage {
        materialize_changes_lineage(db)?;
        "changes_lineage"
    } else {
        "changes"
    };
    // unit + src come from closed enums / a small set of literals so the
    // format! interpolation is safe (no user-controlled input).
    let sql = format!(
        "CREATE OR REPLACE TEMPORARY TABLE changes_bucketed AS \
         SELECT \
             CAST(date_trunc('{unit}', m.date) AS TEXT) AS rev, \
             c.path, \
             MAX(c.change_type) AS change_type, \
             ANY_VALUE(c.rename_from) AS rename_from, \
             ANY_VALUE(c.similarity) AS similarity, \
             SUM(c.loc_added)::INTEGER AS loc_added, \
             SUM(c.loc_deleted)::INTEGER AS loc_deleted \
         FROM {src} c \
         INNER JOIN commits m ON m.rev = c.rev \
         GROUP BY date_trunc('{unit}', m.date), c.path"
    );
    db.conn().execute(&sql, params![]).map_err(|e| {
        CodeLoreError::Analysis(format!("materialize changes_bucketed ({unit}): {e}"))
    })?;
    tracing::info!("materialized changes_bucketed at {unit} granularity (lineage={use_lineage})");
    Ok(())
}

/// Materialize the rename-lineage map as a temporary table.
///
/// Walks `changes.rename_from` recursively to find the LATEST canonical path
/// for every path that has ever been renamed. `path_lineage` is a small
/// `(old_path, canonical_path)` lookup table — typically a handful of rows
/// even on large repos (renames are rare). Cycles are bounded by `depth < 50`,
/// far above any realistic rename chain; the `ROW_NUMBER() ... ORDER BY depth
/// DESC` deterministically picks the last name in the chain. Rows where
/// `old_path == canonical_path` are filtered out — the join then has nothing
/// to merge for files that have never been renamed (the common case).
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] on any SQL error.
pub fn materialize_path_lineage(db: &super::FactsDb) -> Result<()> {
    use duckdb::params;
    // The recursive CTE walks the rename graph, but a NAIVE join on
    // `c.rename_from = l.current` would conflate a recycled filename with
    // its earlier life. Example: commit 1 renames `A → B`; commit 10 takes
    // a different file `C` and renames it `C → A`. Without a chronological
    // constraint the CTE would walk `(C, A, depth=1) → (C, B, depth=2)`
    // by joining `c.rename_from = 'A'` on commit 1's row — producing a
    // fictitious `C → A → B` lineage that merges two unrelated files'
    // history into one entity.
    //
    // The fix joins each step with `commits.date` and only extends the
    // chain when the NEXT rename happened AFTER the current step's date.
    // Date is fetched via `INNER JOIN commits ON commits.rev = c.rev` in
    // both the seed and the recursive step.
    //
    // Same-second tiebreak: strict `>` on date would terminate a chain
    // where two sequential renames (A → B then B → C) land in commits
    // sharing the exact same second. Carry `commits.rowid` through the CTE
    // and break date-ties via `co.rowid < l.current_rowid`: gix walks
    // reverse-chronologically (HEAD first), so newer commits receive
    // smaller rowids during ingest; the next step in a forward rename
    // chain must come from a newer commit, hence the smaller rowid.
    let sql = "CREATE OR REPLACE TEMPORARY TABLE path_lineage AS
        WITH RECURSIVE lineage(orig, current, current_date, current_rowid, depth) AS (
            SELECT DISTINCT c.rename_from, c.path, co.date, co.rowid, 1
            FROM changes c
            INNER JOIN commits co ON co.rev = c.rev
            WHERE c.rename_from IS NOT NULL
            UNION ALL
            SELECT l.orig, c.path, co.date, co.rowid, l.depth + 1
            FROM lineage l
            INNER JOIN changes c ON c.rename_from = l.current
            INNER JOIN commits co ON co.rev = c.rev
            WHERE l.depth < 50
              AND (co.date > l.current_date
                   OR (co.date = l.current_date AND co.rowid < l.current_rowid))
        )
        SELECT orig AS old_path, current AS canonical_path
        FROM (
            SELECT orig, current, depth,
                   ROW_NUMBER() OVER (
                       PARTITION BY orig
                       -- Secondary order on `current` so ties at the same
                       -- depth (possible when a non-linear rename graph
                       -- reaches the same intermediate via multiple paths)
                       -- break deterministically and run-to-run output stays
                       -- byte-equal.
                       ORDER BY depth DESC, current ASC
                   ) AS rn
            FROM lineage
        )
        WHERE rn = 1 AND orig != current";
    db.conn()
        .execute(sql, params![])
        .map_err(|e| CodeLoreError::Analysis(format!("materialize path_lineage: {e}")))?;
    Ok(())
}

/// Materialize `changes_lineage` — `changes` with `path` canonicalized via
/// the rename-lineage map. Idempotent (`CREATE OR REPLACE`). Calls
/// [`materialize_path_lineage`] first so the lookup table is in scope.
///
/// Analyses that opt into rename-aware aggregation should `FROM
/// changes_lineage` instead of `FROM changes`. The schema is identical
/// modulo `path` being the post-rename canonical name.
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] on any SQL error.
pub fn materialize_changes_lineage(db: &super::FactsDb) -> Result<()> {
    use duckdb::params;
    materialize_path_lineage(db)?;
    let sql = "CREATE OR REPLACE TEMPORARY TABLE changes_lineage AS
        SELECT
            c.rev,
            COALESCE(pl.canonical_path, c.path) AS path,
            c.change_type,
            c.rename_from,
            c.similarity,
            c.loc_added,
            c.loc_deleted
        FROM changes c
        LEFT JOIN path_lineage pl ON pl.old_path = c.path";
    db.conn()
        .execute(sql, params![])
        .map_err(|e| CodeLoreError::Analysis(format!("materialize changes_lineage: {e}")))?;
    // Without explicit indexes the downstream GROUP BY path / GROUP BY rev
    // aggregations fall to full scans even when the base `changes` table
    // has covering indexes. Mirror them on the temp table.
    for stmt in [
        "CREATE INDEX IF NOT EXISTS idx_changes_lineage_path ON changes_lineage(path)",
        "CREATE INDEX IF NOT EXISTS idx_changes_lineage_rev ON changes_lineage(rev)",
    ] {
        db.conn()
            .execute(stmt, params![])
            .map_err(|e| CodeLoreError::Analysis(format!("index changes_lineage: {e}")))?;
    }
    tracing::info!("materialized changes_lineage with canonical rename paths");
    Ok(())
}

/// Apply architectural grouping in-place on the `changes` table. Called by
/// [`FactsDb::ingest`] after raw ingest if `opts.group_file.is_some()`.
///
/// Implementation:
///   1. Build a `(raw_path → group_name)` mapping in Rust from every distinct
///      path in `changes` against the [`GroupMap`].
///   2. Insert the mapping into a temporary table.
///   3. Build a `changes_grouped` temporary table that JOINs against the
///      mapping, replaces the path with the group name (or keeps raw under
///      non-strict mode for unmapped paths), and aggregates `loc_added` /
///      `loc_deleted` per `(rev, new_path)`.
///   4. Replace `changes` content with the aggregated rows.
///   5. Remove `hunks` rows whose `(rev, path)` no longer exists in
///      `changes` (strict mode + dropped paths produces orphans otherwise).
///
/// Strict vs non-strict:
/// - Strict (`opts.strict_grouping = true` / code-maat default): rows whose
///   path doesn't match any rule are DROPPED.
/// - Non-strict (`CodeLore` default): unmapped rows keep their raw path.
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] on any SQL error.
pub fn apply_grouping(db: &super::FactsDb, group_map: &super::GroupMap) -> Result<()> {
    use duckdb::params;

    let conn = db.conn();

    // Step 1: enumerate distinct paths in `changes` and pre-compute the
    // mapping in Rust. Doing the regex matching here avoids embedding the
    // GroupMap rules into SQL (DuckDB has regexp_matches but doesn't
    // support fancy-regex's lookaround that some code-maat fixtures need).
    let distinct_paths: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT DISTINCT path FROM changes")
            .map_err(|e| CodeLoreError::Analysis(format!("prepare distinct paths: {e}")))?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| CodeLoreError::Analysis(format!("query distinct paths: {e}")))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| CodeLoreError::Analysis(format!("collect distinct paths: {e}")))?
    };

    // Step 2: build the mapping table. Mapped paths get the group name; in
    // non-strict mode, unmapped paths get the raw path back; in strict mode,
    // unmapped paths get a sentinel NULL group_name that the WHERE filter
    // in step 3 uses to drop them.
    conn.execute(
        "CREATE OR REPLACE TEMPORARY TABLE _grouping_v1 (\
             raw_path TEXT PRIMARY KEY, group_name TEXT\
         )",
        [],
    )
    .map_err(|e| CodeLoreError::Analysis(format!("create _grouping_v1: {e}")))?;

    {
        // Step 2a: regex-match every distinct path against the group map
        // in parallel. The regex set is `Send + Sync` (immutable post-build),
        // and `Vec<String>` shares immutable refs across rayon workers.
        // Pre-`f41` this loop ran sequentially on the main thread; for
        // monorepos with N×M = paths × rules in the high millions, the
        // single-threaded matching dominated `apply_grouping` wall-clock.
        use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
        let strict = group_map.strict;
        let mapped: Vec<(String, Option<String>)> = distinct_paths
            .par_iter()
            .map(|path| {
                let raw = path.clone();
                let effective: Option<String> = if strict {
                    group_map.map_entity(path).map(str::to_owned)
                } else {
                    Some(
                        group_map
                            .map_entity(path)
                            .map_or_else(|| path.clone(), str::to_owned),
                    )
                };
                (raw, effective)
            })
            .collect();

        // Step 2b: serial INSERT (DuckDB Connection is !Send + !Sync).
        let mut stmt = conn
            .prepare("INSERT INTO _grouping_v1 (raw_path, group_name) VALUES (?, ?)")
            .map_err(|e| CodeLoreError::Analysis(format!("prepare grouping insert: {e}")))?;
        for (raw, effective) in &mapped {
            stmt.execute(params![raw, effective.as_deref()])
                .map_err(|e| CodeLoreError::Analysis(format!("grouping insert row: {e}")))?;
        }
    }

    // Step 3+4: rewrite `changes` in place. CREATE OR REPLACE TEMPORARY
    // TABLE _changes_grouped + DELETE+INSERT pattern keeps the FK from
    // hunks happy in step 5 (no period where changes is empty AND the
    // grouped data isn't yet ready to INSERT).
    conn.execute(
        "CREATE OR REPLACE TEMPORARY TABLE _changes_grouped AS \
         SELECT \
             c.rev, \
             g.group_name AS path, \
             MAX(c.change_type) AS change_type, \
             ANY_VALUE(c.rename_from) AS rename_from, \
             ANY_VALUE(c.similarity) AS similarity, \
             SUM(c.loc_added)::INTEGER AS loc_added, \
             SUM(c.loc_deleted)::INTEGER AS loc_deleted \
         FROM changes c \
         INNER JOIN _grouping_v1 g ON g.raw_path = c.path \
         WHERE g.group_name IS NOT NULL \
         GROUP BY c.rev, g.group_name",
        [],
    )
    .map_err(|e| CodeLoreError::Analysis(format!("build _changes_grouped: {e}")))?;

    // Step 5: clean hunks for paths that won't survive the swap. Do BEFORE
    // the changes-swap so the FK from hunks → changes never sees a missing
    // referent. Hunks aren't path-rewritten (line-range semantics don't
    // translate to group level), so they get dropped for any path that
    // collapsed or got removed.
    //
    // The previous `NOT IN (SELECT … )` form on a composite key
    // forces some `DuckDB` planner paths into a per-row subquery scan
    // rather than the index-friendly hash anti-join. `NOT EXISTS` with
    // a correlated `h.rev = c.rev AND h.path = c.path` predicate is the
    // textbook planner-friendly form: same semantics (both projected
    // columns are `NOT NULL` per `schema_v1.sql`, so `NOT IN`'s NULL
    // pitfall doesn't apply, but `NOT EXISTS` is uniformly preferred
    // across `DuckDB` versions). The existing
    // `apply_grouping_*` integration tests cover the semantic
    // equivalence — both forms must drop the same hunks for the same
    // group-map input.
    conn.execute(
        "DELETE FROM hunks h \
         WHERE NOT EXISTS ( \
             SELECT 1 FROM changes c \
             INNER JOIN _grouping_v1 g ON g.raw_path = c.path \
             WHERE g.group_name = c.path \
               AND c.rev = h.rev \
               AND c.path = h.path \
         )",
        [],
    )
    .map_err(|e| CodeLoreError::Analysis(format!("clean hunks: {e}")))?;

    // Swap the data in changes
    conn.execute("DELETE FROM changes", [])
        .map_err(|e| CodeLoreError::Analysis(format!("clear changes: {e}")))?;
    conn.execute(
        "INSERT INTO changes (rev, path, change_type, rename_from, similarity, loc_added, loc_deleted) \
         SELECT rev, path, change_type, rename_from, similarity, loc_added, loc_deleted \
         FROM _changes_grouped",
        [],
    )
    .map_err(|e| CodeLoreError::Analysis(format!("swap changes: {e}")))?;

    tracing::info!(
        "grouping applied: {} rules, {} distinct paths, strict={}",
        group_map.rules.len(),
        distinct_paths.len(),
        group_map.strict
    );

    Ok(())
}

/// Format a `Box<dyn Any + Send>` panic payload (the value
/// `std::thread::JoinHandle::join` returns on `Err`) into a stable
/// human-readable string. The two canonical concrete types `panic!`
/// produces are `&'static str` (from `panic!("literal")`) and `String`
/// (from `panic!("{}", val)`) — handle both. Anything else (custom
/// panic types, payload-less aborts) falls through to a generic
/// `<non-string panic payload>` so the typed-error path still
/// surfaces something operators can grep for.
pub(crate) fn format_panic_payload(payload: &Box<dyn std::any::Any + Send>) -> String {
    let detail = payload
        .downcast_ref::<&'static str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<non-string panic payload>".to_string());
    format!("commit walker thread panicked: {detail}")
}

#[cfg(test)]
mod panic_payload_tests {
    use super::format_panic_payload;

    /// `panic!("literal")` → `Box<dyn Any>` containing `&'static str`.
    #[test]
    fn extracts_static_str_payload() {
        let payload: Box<dyn std::any::Any + Send> = Box::new("walker exploded");
        let msg = format_panic_payload(&payload);
        assert_eq!(msg, "commit walker thread panicked: walker exploded");
    }

    /// `panic!("{} {}", "walker", "exploded")` → `Box<dyn Any>` containing `String`.
    #[test]
    fn extracts_string_payload() {
        let payload: Box<dyn std::any::Any + Send> = Box::new(String::from("formatted reason"));
        let msg = format_panic_payload(&payload);
        assert_eq!(msg, "commit walker thread panicked: formatted reason");
    }

    /// Anything that isn't `&'static str` or `String` falls through to
    /// the placeholder. Exercises the `unwrap_or_else` branch so a
    /// future change to the helper can't silently break it.
    #[test]
    fn unknown_payload_falls_through_to_placeholder() {
        let payload: Box<dyn std::any::Any + Send> = Box::new(42_u32);
        let msg = format_panic_payload(&payload);
        assert_eq!(
            msg,
            "commit walker thread panicked: <non-string panic payload>"
        );
    }
}

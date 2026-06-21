-- Schema v3: `commits` gains a `committer_date TIMESTAMP NOT NULL`
-- column alongside `date` (which is the author date). The delta
-- `(committer_date - date)` is the in-flight time that the
-- `lead-time` and `delivery-friction` analyses surface. On
-- linear-history workflows (rebase, squash) the two are often
-- equal; on merge-via-merge-commit + review workflows the delta is
-- the review-bottleneck signal.
--
-- Schema v2 had only `commits.date` (author date) and `lead-time`
-- silently emitted zero rows for every commit; v3 closes that gap.
--
-- `commits.date` itself remained TIMESTAMP from v2, promoted from
-- DATE so HEAD resolution and same-day chronology stay precise —
-- previously two commits sharing a calendar day forced a
-- lexicographical rev tiebreak, which silently picked the wrong
-- HEAD on the final day and stamped complexity/clones rows with
-- the wrong rev. The original author tz offset is discarded at
-- this boundary; a future schema version may add a sibling
-- `author_tz_offset_seconds INTEGER` for tz-aware analyses.

CREATE TABLE IF NOT EXISTS commits (
    rev TEXT PRIMARY KEY,
    author_email TEXT NOT NULL,
    author_name TEXT NOT NULL,
    committer_email TEXT NOT NULL,
    canonical_author TEXT NOT NULL,
    ai_attribution TEXT,
    date TIMESTAMP NOT NULL,
    committer_date TIMESTAMP NOT NULL,
    message TEXT NOT NULL,
    is_merge BOOLEAN NOT NULL,
    parent_count INTEGER NOT NULL,
    ns INTEGER, nd INTEGER, nf INTEGER, entropy DOUBLE,
    la INTEGER, ld INTEGER, lt DOUBLE,
    fix BOOLEAN,
    ndev INTEGER, age DOUBLE, nuc INTEGER,
    exp INTEGER, rexp DOUBLE, sexp INTEGER
);

CREATE TABLE IF NOT EXISTS changes (
    rev TEXT NOT NULL REFERENCES commits(rev),
    path TEXT NOT NULL,
    -- change_type carries one of: 'added' | 'modified' | 'deleted' |
    -- 'renamed' | 'copied' | 'binary'. CHECK constraint validates at
    -- INSERT/Appender time so SQL queries can rely on the closed set
    -- without typo-checks like `change_type != 'deleted'` failing
    -- silently against a misspelled enum-like value.
    --
    -- DuckDB ENUM would be a tighter encoding (1-byte tag + dictionary)
    -- but lacks `CREATE TYPE IF NOT EXISTS`, breaking re-open on the
    -- cached fact store. CHECK is idempotent and gives the same
    -- correctness invariant at slightly larger storage cost.
    change_type TEXT NOT NULL CHECK (change_type IN (
        'added', 'modified', 'deleted', 'renamed', 'copied', 'binary'
    )),
    rename_from TEXT,
    similarity INTEGER,
    loc_added INTEGER NOT NULL DEFAULT 0,
    loc_deleted INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (rev, path)
);

-- Per-hunk diff line-range rows, one per `@@ -old_start[,old_lines]
-- +new_start[,new_lines] @@` header in the diff. Populated alongside
-- `changes` by `append_change` (each FileChange.hunks slice expands
-- into N rows here, where N = number of hunks in the diff).
--
-- All four offset columns are NOT NULL (a hunk that's missing any of
-- old_start / old_lines / new_start / new_lines is malformed; the
-- parser in `repo::git_cli_repo::parse_hunk_headers` returns None on
-- such headers so they never reach the Appender, and the gix path
-- constructs `Hunk` from u32 fields that can't be NULL).
--
-- PRIMARY KEY (rev, path, old_start, new_start) — a hunk inside a
-- single (rev, path) is uniquely identified by its two start
-- offsets; a single commit cannot apply two hunks to the same file
-- starting at the same old AND new line. This also gives DuckDB a
-- physical index for the FK clean-up DELETE in `apply_grouping`,
-- replacing the unindexed table-scan that the audit flagged as
-- expensive on diff-heavy repos.
--
-- `idx_hunks_rev_path` accelerates the `apply_grouping` FK cleanup
-- (`DELETE FROM hunks h WHERE NOT EXISTS ... AND c.rev = h.rev AND
-- c.path = h.path`) on diff-heavy repos. The PK above is the
-- physical order; this secondary index is the lookup path.
CREATE TABLE IF NOT EXISTS hunks (
    rev TEXT NOT NULL,
    path TEXT NOT NULL,
    old_start INTEGER NOT NULL, old_lines INTEGER NOT NULL,
    new_start INTEGER NOT NULL, new_lines INTEGER NOT NULL,
    PRIMARY KEY (rev, path, old_start, new_start),
    FOREIGN KEY (rev, path) REFERENCES changes(rev, path)
);
CREATE INDEX IF NOT EXISTS idx_hunks_rev_path ON hunks(rev, path);

CREATE TABLE IF NOT EXISTS entities (
    path TEXT NOT NULL, name TEXT NOT NULL, kind TEXT NOT NULL,
    start_line INTEGER NOT NULL, end_line INTEGER NOT NULL,
    rev_introduced TEXT NOT NULL, rev_last_seen TEXT NOT NULL,
    PRIMARY KEY (path, name, rev_introduced)
);

CREATE TABLE IF NOT EXISTS complexity_metrics (
    path TEXT NOT NULL, name TEXT NOT NULL, rev TEXT NOT NULL,
    cyclomatic INTEGER, cognitive INTEGER,
    halstead_volume DOUBLE, halstead_difficulty DOUBLE, halstead_effort DOUBLE,
    mi DOUBLE,
    nom INTEGER, nexits INTEGER,
    loc INTEGER, sloc INTEGER,
    max_nesting INTEGER, mean_nesting DOUBLE,
    sd_nesting DOUBLE, total_nesting INTEGER,
    PRIMARY KEY (path, name, rev)
);

CREATE TABLE IF NOT EXISTS author_aliases (
    raw_email TEXT PRIMARY KEY,
    canonical TEXT NOT NULL,
    is_bot BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE TABLE IF NOT EXISTS provenance (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Plan 7: clone-detection results, populated at HEAD by clones::grouper.
-- clone_group_id groups identical AST fingerprints (Type 1 + Type 2 clones).
-- For Type 3 near-miss (MinHash, Plan 7 Task 4) the similarity column drops
-- below 1.0 and rows in the same group share an LSH bucket but differ in
-- digest.
CREATE TABLE IF NOT EXISTS clones (
    clone_group_id  INTEGER NOT NULL,
    fingerprint     BLOB NOT NULL,
    rev             TEXT NOT NULL,
    path            TEXT NOT NULL,
    function        TEXT NOT NULL,
    start_line      INTEGER NOT NULL,
    end_line        INTEGER NOT NULL,
    node_count      INTEGER NOT NULL,
    similarity      DOUBLE NOT NULL,
    PRIMARY KEY (clone_group_id, path, function, start_line)
);
CREATE INDEX IF NOT EXISTS idx_clones_group ON clones(clone_group_id);
CREATE INDEX IF NOT EXISTS idx_clones_fp ON clones(fingerprint);

-- Hot-path indexes for analysis queries (added during the modernization
-- sweep — prior to this only clones had indexes, so every JOIN against
-- changes/commits did a full table scan).
--
-- changes(path): scanned by every per-file aggregation analysis
--   (revisions, hotspots, ownership, code-health, entity-churn, etc.).
-- changes(rev):  primary JOIN column with commits; PK is (rev, path),
--   so a rev-prefix scan benefits from a dedicated index too.
-- changes(rename_from): walked by the recursive lineage CTE in
--   `facts/ingest.rs::materialize_path_lineage`. The CTE joins
--   `lineage` with `changes` on `c.rename_from = l.current` to
--   chase rename chains forward through history. NULL rename_from
--   rows (which dominate the table — typically >95%) are
--   automatically excluded from a B-tree index in DuckDB, so this
--   index is small on disk relative to the table.
-- commits(canonical_author): scanned by author-based analyses (authors,
--   author-churn, ownership, communication, code-health).
-- commits(date): scanned by abs-churn and code-age.
CREATE INDEX IF NOT EXISTS idx_changes_path        ON changes(path);
CREATE INDEX IF NOT EXISTS idx_changes_rev         ON changes(rev);
CREATE INDEX IF NOT EXISTS idx_changes_rename_from ON changes(rename_from);
CREATE INDEX IF NOT EXISTS idx_commits_author      ON commits(canonical_author);
CREATE INDEX IF NOT EXISTS idx_commits_date        ON commits(date);

-- Architecture import graph — one row per import edge at HEAD.
-- Drives the layered-architecture rule validation, god-class
-- detector (fan_in/fan_out), architecture force-graph widget, and
-- module-to-module chord diagram.
--
-- src_path is the file containing the import statement (the importer);
-- target is the raw module-path string extracted from the tree-sitter
-- AST (e.g. "std::fs::read_to_string" for Rust, "react" for JS,
-- "java.util.List" for Java).
--
-- The per-language resolver maps `target → repo-relative path`
-- where possible and sets resolved=TRUE + target_path; rows that
-- can't be mapped (external crates, npm packages, std-lib) stay
-- resolved=FALSE with target_path=NULL.
--
-- kind buckets the import semantics into a small closed set so SQL
-- can `WHERE kind = 'wildcard'` without parsing the target string.
CREATE TABLE IF NOT EXISTS imports (
    rev         TEXT NOT NULL REFERENCES commits(rev),
    src_path    TEXT NOT NULL,
    target      TEXT NOT NULL,
    resolved    BOOLEAN NOT NULL,
    target_path TEXT,
    kind        TEXT NOT NULL CHECK (kind IN (
        'absolute', 'relative', 'wildcard', 'unknown'
    )),
    PRIMARY KEY (rev, src_path, target)
);
CREATE INDEX IF NOT EXISTS idx_imports_src    ON imports(src_path);
CREATE INDEX IF NOT EXISTS idx_imports_target ON imports(target_path);

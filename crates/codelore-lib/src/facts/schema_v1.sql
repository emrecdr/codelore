-- Plan 1: walking skeleton schema. Full schema from spec §3.2 lands here.
-- We start with the subset Plan 1 actually populates and lock the rest as empty.

CREATE TABLE IF NOT EXISTS commits (
    rev TEXT PRIMARY KEY,
    author_email TEXT NOT NULL,
    author_name TEXT NOT NULL,
    committer_email TEXT NOT NULL,
    canonical_author TEXT NOT NULL,
    ai_attribution TEXT,
    date DATE NOT NULL,
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

CREATE TABLE IF NOT EXISTS hunks (
    rev TEXT NOT NULL,
    path TEXT NOT NULL,
    old_start INTEGER, old_lines INTEGER,
    new_start INTEGER, new_lines INTEGER,
    FOREIGN KEY (rev, path) REFERENCES changes(rev, path)
);

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
-- commits(canonical_author): scanned by author-based analyses (authors,
--   author-churn, ownership, communication, code-health).
-- commits(date): scanned by abs-churn and code-age.
CREATE INDEX IF NOT EXISTS idx_changes_path     ON changes(path);
CREATE INDEX IF NOT EXISTS idx_changes_rev      ON changes(rev);
CREATE INDEX IF NOT EXISTS idx_commits_author   ON commits(canonical_author);
CREATE INDEX IF NOT EXISTS idx_commits_date     ON commits(date);

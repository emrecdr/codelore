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
    change_type TEXT NOT NULL,
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

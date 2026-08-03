window.BENCHMARK_DATA = {
  "lastUpdate": 1785752453236,
  "repoUrl": "https://github.com/emrecdr/codelore",
  "entries": {
    "Benchmark": [
      {
        "commit": {
          "author": {
            "name": "Emre Camdere",
            "username": "emrecamdere",
            "email": "emre@valocom.nl"
          },
          "committer": {
            "name": "Emre Camdere",
            "username": "emrecamdere",
            "email": "emre@valocom.nl"
          },
          "id": "179dd7872c729518b18a48bf19987b32ae3f6589",
          "message": "feat(pages): publish benchmark trends + README nav; matrix job names show only the OS",
          "timestamp": "2026-07-11T20:12:38Z",
          "url": "https://github.com/emrecdr/codelore/commit/179dd7872c729518b18a48bf19987b32ae3f6589"
        },
        "date": 1783802468950,
        "tool": "cargo",
        "benches": [
          {
            "name": "ingest_tiny",
            "value": 53023895,
            "range": "± 3549764",
            "unit": "ns/iter"
          },
          {
            "name": "ingest/medium_500_commits",
            "value": 92925782,
            "range": "± 942275",
            "unit": "ns/iter"
          },
          {
            "name": "complexity_extraction/parallel_default_threads",
            "value": 93976356,
            "range": "± 2872021",
            "unit": "ns/iter"
          },
          {
            "name": "complexity_extraction/serial_1_thread",
            "value": 93127304,
            "range": "± 1683924",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/16",
            "value": 93505584,
            "range": "± 1314605",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/64",
            "value": 92787379,
            "range": "± 2044107",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/256",
            "value": 92485529,
            "range": "± 1012046",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/1024",
            "value": 91775070,
            "range": "± 2134289",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Emre Camdere",
            "username": "emrecdr",
            "email": "cemre79@gmail.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "471257622b4e7d4fdf3e12a2541c226a1eaae27e",
          "message": "fix(ingest): FK flush-ordering race aborted ingest of large repos (#89)\n\n* fix(ingest): flush FK parents before child appender chunks can outrun them\n\nLarge-repo ingest could abort with `append hunk: Failed to append: Violates\nforeign key constraint …`. The commits/changes/hunks tables are written via\nthree independent DuckDB Appenders whose buffers perform their FK-checked\nphysical write at different, uncorrelated points; a child buffer could be\nwritten while the parent rows it references were still buffered, so the\nreferent was absent.\n\nThe consumer now flushes the parent chain in FK order (commits, then changes)\nonce every STANDARD_VECTOR_SIZE child appends — a cadence far more frequent\nthan the child buffer's own FK-checked write, so every referent is physical\nbefore any child buffer is written. The guard fires only on high-volume\nhistories; small/medium repos produce byte-identical output and see no\nmeasurable overhead.\n\nAdds a file-backed ingest regression test that crosses the FK-check row\nthreshold (~205k hunks). It reproduces the abort on unfixed code and passes\nwith the guard. The existing ingest tests missed the bug because they use an\nin-memory FactsDb, whose Appenders never trip the check.\n\n* test(ingest): gate the volume regression test off windows, honestly\n\nThe doc comment claimed the windows CI subset excludes this binary; it\ndoes not — ingest_test is in the subset filter. The test is volume\ncoverage of a platform-independent mechanism, and its per-commit git\nspawns are the exact workload whose spawn overhead priced the full\nsuite off hosted windows runners, so gate it off that leg and say why.\n\n* test(ingest): gate the volume test's helper with its only caller\n\ncommit_at is called only by the windows-gated volume test, so on the\nwindows leg it became dead code — an error under the CI-wide\n-Dwarnings.\n\n---------\n\nCo-authored-by: Emre Camdere <emre@valocom.nl>",
          "timestamp": "2026-07-13T06:36:52Z",
          "url": "https://github.com/emrecdr/codelore/commit/471257622b4e7d4fdf3e12a2541c226a1eaae27e"
        },
        "date": 1783936066780,
        "tool": "cargo",
        "benches": [
          {
            "name": "ingest_tiny",
            "value": 54864857,
            "range": "± 4012959",
            "unit": "ns/iter"
          },
          {
            "name": "ingest/medium_500_commits",
            "value": 99933445,
            "range": "± 3833383",
            "unit": "ns/iter"
          },
          {
            "name": "complexity_extraction/parallel_default_threads",
            "value": 99300959,
            "range": "± 3869848",
            "unit": "ns/iter"
          },
          {
            "name": "complexity_extraction/serial_1_thread",
            "value": 101040264,
            "range": "± 1115208",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/16",
            "value": 100764245,
            "range": "± 1332582",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/64",
            "value": 99869617,
            "range": "± 1350362",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/256",
            "value": 98298437,
            "range": "± 2357908",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/1024",
            "value": 99623060,
            "range": "± 3862483",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Emre Camdere",
            "username": "emrecdr",
            "email": "cemre79@gmail.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "eb9a339ae2bee00af598298549e889296db8bc23",
          "message": "feat: agent-loop temporal gate Phase 1 — [calibration] section + change_context briefing tool (#109)\n\n* docs(spec): agent-loop temporal quality gate — change_context, gate_changes, codelore gate\n\n* docs(spec): fold validated review refinements into agent-loop gate design\n\n* docs(plan): agent-loop gate phase 1 — calibration section + change_context briefing\n\n* feat(thresholds): repo-declared defect calibration via a [calibration] section\n\nAdds a [calibration] section to .codelore-thresholds.toml with a single\ndefect_artifact path, so a repo can declare its defect-calibration\nartifact once instead of passing --defect-calibration on every\nanalyze/check/explain/mcp invocation. An explicit CLI flag (or the MCP\nserver's startup flag) always takes precedence; relative paths resolve\nagainst the repo root. The section is a config selector, not a gate —\na thresholds file containing only [calibration] still leaves `check`\nvacuously passing.\n\n* feat(knowledge-islands): batched per-path owner-activity lookup without the departed threshold\n\n* feat(repo): expose merge/rebase-in-progress state on both backends\n\n* feat(change-context): temporal pre-write briefing assembly with budgeted deterministic rendering\n\n* feat(mcp): change_context — temporal pre-write briefing tool\n\n* fix(change-context): reserve the no-history row for paths absent from every feed\n\n---------\n\nCo-authored-by: Emre Camdere <emre@valocom.nl>",
          "timestamp": "2026-07-19T23:14:21Z",
          "url": "https://github.com/emrecdr/codelore/commit/eb9a339ae2bee00af598298549e889296db8bc23"
        },
        "date": 1784541018385,
        "tool": "cargo",
        "benches": [
          {
            "name": "ingest_tiny",
            "value": 54653789,
            "range": "± 4294697",
            "unit": "ns/iter"
          },
          {
            "name": "ingest/medium_500_commits",
            "value": 97433530,
            "range": "± 2604619",
            "unit": "ns/iter"
          },
          {
            "name": "complexity_extraction/parallel_default_threads",
            "value": 95928800,
            "range": "± 1426154",
            "unit": "ns/iter"
          },
          {
            "name": "complexity_extraction/serial_1_thread",
            "value": 96023116,
            "range": "± 1702137",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/16",
            "value": 94261995,
            "range": "± 2178639",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/64",
            "value": 95786645,
            "range": "± 2260966",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/256",
            "value": 93465005,
            "range": "± 1873075",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/1024",
            "value": 93243341,
            "range": "± 6747399",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Emre Camdere",
            "username": "emrecdr",
            "email": "cemre79@gmail.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "bd4ca7973e403a80f88c2b492d83551714a45f34",
          "message": "refactor(cli): collapse the mechanical per-analysis dispatch shells (#151)\n\nThe ~48 near-verbatim 4-arm dispatch_* functions in analyze.rs (each a\ncsv/json/markdown + html + unsupported-format shell differing only in the\nrun-fn and emitter names) collapse into one `dispatch!` macro consumed by a\nsingle `run_streaming_dispatch` table — one declarative arm per analysis. The\nspecials (hotspots' sarif/ndjson/gha, code-health's ndjson + corpus notice,\nthe repo/target/store-taking analyses, the bespoke html-wired set) stay\nexplicit but share the macro core plus an extra writer line.\n\nThe dispatch now READS the seam instead of mirroring it: `supported_formats`\nand `HTML_WIRED` are promoted out of the test module to module scope, and the\nerror helpers derive from them — `unsupported_format` builds its advertised\nlist from `supported_formats(analysis)` and `html_not_wired` builds its covered\nlist from `HTML_WIRED`, so the messages and the wiring cannot drift. The\nregistration-surface tests keep the same contract, now reading the promoted\nseam.\n\nFold in the stale-string fix: the `--format html` guidance omitted\n`refactoring-targets` (which wires a real html emitter); deriving the list from\n`HTML_WIRED` restores it and prevents future drift.\n\nBehavior is byte-identical for every analysis x format combination: a full\nsweep (56 analyses x csv/json/markdown/sarif/ndjson/gha/html) over a fixed repo\ndiffs empty before/after, modulo the pre-existing nondeterministic SARIF run id\nand the known coordination-needs cochange_entropy last-ULP JSON noise.\nanalyze.rs: 4131 -> 2236 lines.\n\nCo-authored-by: Emre Camdere <emre@valocom.nl>",
          "timestamp": "2026-07-27T09:36:19Z",
          "url": "https://github.com/emrecdr/codelore/commit/bd4ca7973e403a80f88c2b492d83551714a45f34"
        },
        "date": 1785148009793,
        "tool": "cargo",
        "benches": [
          {
            "name": "ingest_tiny",
            "value": 55882588,
            "range": "± 3969859",
            "unit": "ns/iter"
          },
          {
            "name": "ingest/medium_500_commits",
            "value": 99680740,
            "range": "± 3810103",
            "unit": "ns/iter"
          },
          {
            "name": "complexity_extraction/parallel_default_threads",
            "value": 96680180,
            "range": "± 1871062",
            "unit": "ns/iter"
          },
          {
            "name": "complexity_extraction/serial_1_thread",
            "value": 98909475,
            "range": "± 2753398",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/16",
            "value": 97707097,
            "range": "± 3899576",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/64",
            "value": 99418288,
            "range": "± 2807894",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/256",
            "value": 97661148,
            "range": "± 2122869",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/1024",
            "value": 97040735,
            "range": "± 9051268",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Emre Camdere",
            "username": "emrecdr",
            "email": "cemre79@gmail.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "66e0a0252a18328d94374fad8fc531904d4ec2c1",
          "message": "test: cover calibrate total-failure exit, MCP hotspots call, entity-effort/ownership (#204)\n\ncalibrate: a manifest where every repo is unreachable (0-of-N included) now\nhas a regression test locking in the existing total-failure guard —\ncalibrate.rs already hard-errors via CodeLoreError::Analysis (exit 4) and\nwrites no artifact; this was previously covered only for partial failure.\n\nmcp: hotspots was the one tools/list entry never exercised via tools/call;\nadd a call test asserting the capped bare-array response shape, mirroring\nthe existing code_health call test.\n\nentity-ownership / entity-effort: neither analysis had any behavioral\ncoverage beyond dispatch-metadata loops. New per-analysis test files build a\nsmall 2-author, 2-file fixture (one file renamed partway through) and assert\nexact per-(entity, author) added/deleted churn and revision counts, both\nwith canonical lineage off (pre/post-rename entities stay split) and on\n(the renamed entity's pre-rename history merges into the canonical path) —\nexercising the changes_lineage rewrite both analyses opt into.\n\nCo-authored-by: Emre Camdere <emre@valocom.nl>",
          "timestamp": "2026-08-03T10:15:47Z",
          "url": "https://github.com/emrecdr/codelore/commit/66e0a0252a18328d94374fad8fc531904d4ec2c1"
        },
        "date": 1785752451591,
        "tool": "cargo",
        "benches": [
          {
            "name": "ingest_tiny",
            "value": 52410966,
            "range": "± 2420711",
            "unit": "ns/iter"
          },
          {
            "name": "ingest/medium_500_commits",
            "value": 92541558,
            "range": "± 2177341",
            "unit": "ns/iter"
          },
          {
            "name": "complexity_extraction/parallel_default_threads",
            "value": 92654085,
            "range": "± 2195833",
            "unit": "ns/iter"
          },
          {
            "name": "complexity_extraction/serial_1_thread",
            "value": 92922629,
            "range": "± 2018388",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/16",
            "value": 93547957,
            "range": "± 1394184",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/64",
            "value": 92528400,
            "range": "± 1466567",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/256",
            "value": 91904735,
            "range": "± 2124089",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/1024",
            "value": 92380779,
            "range": "± 2315786",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}
window.BENCHMARK_DATA = {
  "lastUpdate": 1788183753770,
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
          "id": "51da60681e22e05efecf308325bed89740726cbf",
          "message": "fix(output): explain what a failed sqlite export needs (#251)\n\nThe emitter ran `INSTALL sqlite; LOAD sqlite;` in the same batch as the\nATTACH and the ten table copies, mapping any failure to\n`format!(\"sqlite: {e}\")`. INSTALL fetches the extension over the network\non first use and caches it under DuckDB's home directory, so an\nair-gapped or locked-down host got a bare DuckDB error labelled \"sqlite\"\n— indistinguishable from a bug in the export itself.\n\nINSTALL/LOAD is now its own statement, so the hint attaches to the one\nstep that reaches outside the process rather than to every sqlite error.\nThe structure does the discrimination; matching DuckDB's error text would\nrot.\n\nThe first draft of the hint said only \"needs network access\".\nReproducing the failure showed the same arm is reached by an unwritable\ncache directory — a permission error with no network involved — so it\nnames both causes, where the cache lives, and the two ways out.\n\nThe test induces the failure by pointing DuckDB's own `home_directory` at\nan unwritable path, needing neither network isolation nor a mutation of\nthe process environment. The workspace's forbidden `unsafe_code` rejected\nthe first attempt, which set HOME through `std::env::set_var`; the DuckDB\nsetting is both safe and better targeted. Verified discriminating —\nstripping the hint fails it on the bare error.\n\nCloses M15 from the cycle-8 carried set, logged as F293.\n\nCo-authored-by: Emre Camdere <emre@valocom.nl>",
          "timestamp": "2026-08-10T06:53:33Z",
          "url": "https://github.com/emrecdr/codelore/commit/51da60681e22e05efecf308325bed89740726cbf"
        },
        "date": 1786349785090,
        "tool": "cargo",
        "benches": [
          {
            "name": "ingest_tiny",
            "value": 51725116,
            "range": "± 3272383",
            "unit": "ns/iter"
          },
          {
            "name": "ingest/medium_500_commits",
            "value": 91192811,
            "range": "± 1683890",
            "unit": "ns/iter"
          },
          {
            "name": "complexity_extraction/parallel_default_threads",
            "value": 91653273,
            "range": "± 1091363",
            "unit": "ns/iter"
          },
          {
            "name": "complexity_extraction/serial_1_thread",
            "value": 90917475,
            "range": "± 1118134",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/16",
            "value": 90949894,
            "range": "± 1671666",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/64",
            "value": 91392543,
            "range": "± 2041606",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/256",
            "value": 91105079,
            "range": "± 975991",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/1024",
            "value": 90779667,
            "range": "± 457495",
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
          "id": "13beb6c80c519b1bd05fbebe2a0023da8e085271",
          "message": "docs(reports): land cycle 15 with competitor figures corrected (#282)\n\nProduct/competitive analysis, revised after an independent validation\npass. The codebase claims were re-checked against source and held; the\ncompetitor figures did not.\n\nCorrected against the vendors' own pages:\n\n- repowise: ~3.6k stars -> ~5.9k; 9 MCP tools -> 10 (a count they\n  describe as a deliberate ceiling); 15 languages -> 19 parsed to AST,\n  13 at a framework-aware \"Full\" tier. The release version is dropped\n  rather than restated: it is not shown on the page, and the draft's\n  \"v0.31.0 July 2026\" would only rot again.\n- CodeScene: the free/standalone MCP tier is not \"single-file static\n  Code Health only\" -- it also performs delta reviews and business-case\n  calculations locally. Seat pricing is EUR 18 Standard / EUR 27 Pro;\n  the quoted MCP add-on price could not be confirmed and is now marked\n  unverified.\n- P3 enumerated nine of the eleven MCP tools while asserting a property\n  of all eleven. The list is now taken from source: `repo_overview` and\n  `gate_changes` were missing.\n- The depth claim compared CodeLore's 57 analyses against repowise's\n  tool count. Like for like the agent surfaces are close, 11 against\n  10; the asymmetry is behind the surface, not on it.\n\nTwo premises weakened and are marked inline rather than re-ranked:\nCodeScene ships an AGENTS.md and free-tier delta reviews (narrowing\nP2's headroom), and repowise ships cycle detection and architecture\nsummaries over MCP (so P3's gap is the narrower \"nobody exposes\n*quantified* architecture metrics\", not an empty surface).\n\nThe P1-P6 ranking is deliberately unchanged. It is a product judgement\nfor the maintainer, and this pass corrected facts rather than making\nthat call.\n\nCo-authored-by: Emre Camdere <emre@valocom.nl>",
          "timestamp": "2026-08-16T22:25:22Z",
          "url": "https://github.com/emrecdr/codelore/commit/13beb6c80c519b1bd05fbebe2a0023da8e085271"
        },
        "date": 1786951811423,
        "tool": "cargo",
        "benches": [
          {
            "name": "ingest_tiny",
            "value": 56454609,
            "range": "± 3224567",
            "unit": "ns/iter"
          },
          {
            "name": "ingest/medium_500_commits",
            "value": 103737216,
            "range": "± 1300217",
            "unit": "ns/iter"
          },
          {
            "name": "complexity_extraction/parallel_default_threads",
            "value": 98126540,
            "range": "± 1663624",
            "unit": "ns/iter"
          },
          {
            "name": "complexity_extraction/serial_1_thread",
            "value": 98615773,
            "range": "± 1974438",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/16",
            "value": 101337381,
            "range": "± 1494399",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/64",
            "value": 96819243,
            "range": "± 1471361",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/256",
            "value": 95315848,
            "range": "± 2108809",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/1024",
            "value": 94728980,
            "range": "± 1822800",
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
          "id": "d3ff722be9d63f698a4d9c6963a90e7a6955b988",
          "message": "docs: close the marker class in the manifests, ship the machete entry cycle 18 prescribed (#290)\n\nA cleanup review scoped to the whole unreleased range rather than the last\ncommit, which is what made most of this findable: three hardening-cycle\nreports sitting side by side reveal drift a single-commit diff cannot.\n\nThe convention banning plan markers and version numbers from comments was\nenforced nowhere the markers actually were. The comment-hygiene guard scans\n.rs/.sql under the product crates' src/tests, so a Cargo.toml sits outside\nit twice over -- by extension and by path -- and no guard reads UPSTREAM.md\nat all, which codelore-rca's manifest sets as `readme` and which is\ntherefore that crate's published package page. Seven marker sites across\nfour manifests and that page now describe the current contract. F308 is\nrewritten to match: its instance table went stale the moment these landed,\nits caveat excluded codelore-rca wholesale when UPSTREAM.md is\ncodelore-authored and not upstream's at all, and the axis it named\n(extension-and-path) is not the axis that hid the worst instance\n(published-versus-internal).\n\ncodelore-rca now carries [package.metadata.cargo-machete] ignored =\n[\"num-traits\"]. Cycle 18 section 3 had already prescribed exactly this;\nrejecting the machete *gate* in #287 silently took the non-gate declaration\nwith it, so a comment reading \"Do not act on that report\" shipped alone --\nreaching only someone who opens the manifest, when every report of this\ndependency so far arrived as tool output. Verified by running it: five\nfindings drop to four. The four grammar false positives stay visible on\npurpose. They are the evidence that a text scanner cannot see through a\nmacro, and a green scanner would erase the argument for rejecting the gate.\n\nThe stale-claim class now closes by rule rather than by count. An eighth\ninstance of the corrected `solely` attribution sits in the released\n[0.28.0] section and is deliberately left standing: prose giving\npresent-tense guidance gets swept, prose recording what was believed at a\npoint in time does not. Seven was never a property of the codebase, only of\nwhere the author looked.\n\nAlso corrected: a quotation carrying the machete rejection is restored to\nthe words its source used -- \"red on every pull request\", not \"red for the\nwrong reason\", which swapped a criterion about volume for one about\ncorrectness with the quote marks left in place. The original reaches a\nmachete gate directly, five false positives being red on every pull\nrequest. centrality.rs stops measuring itself against a crate that left the\nworkspace and stops linking it as a live alternative; UPSTREAM.md stops\nexplaining a different crate's feature choice, where it had already gone\nstale twice; the num-traits mechanism is stated once rather than\nre-synchronised across two entries; F310's fix ranking is un-inverted so\nthe guard matching the defect that shipped comes first; F309's deferral\nrests on a reason its own commit honours; and the grammar sweep recipe\nnames every pin site a bump has to touch.\n\nRecorded rather than fixed: F312, codelore-rca compiling a Callback\ndispatch layer with no call site outside the crate, three crates leaving\nthe graph with it but removal being a breaking change to a published crate;\nand F313, tempfile declared in both dependency tables, where the range's\nown standard is delete-and-build and this pass was at 98% disk.\n\nCo-authored-by: Emre Camdere <emre@valocom.nl>",
          "timestamp": "2026-08-24T02:08:12Z",
          "url": "https://github.com/emrecdr/codelore/commit/d3ff722be9d63f698a4d9c6963a90e7a6955b988"
        },
        "date": 1787556748673,
        "tool": "cargo",
        "benches": [
          {
            "name": "ingest_tiny",
            "value": 52167551,
            "range": "± 2019134",
            "unit": "ns/iter"
          },
          {
            "name": "ingest/medium_500_commits",
            "value": 94969750,
            "range": "± 1767324",
            "unit": "ns/iter"
          },
          {
            "name": "complexity_extraction/parallel_default_threads",
            "value": 95345671,
            "range": "± 1788250",
            "unit": "ns/iter"
          },
          {
            "name": "complexity_extraction/serial_1_thread",
            "value": 94477342,
            "range": "± 1444092",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/16",
            "value": 96553460,
            "range": "± 1236125",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/64",
            "value": 95823103,
            "range": "± 1802056",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/256",
            "value": 94248033,
            "range": "± 1088243",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/1024",
            "value": 95041926,
            "range": "± 1932333",
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
          "id": "7256e468134a75b0babb50dcbd725a0e65f32c07",
          "message": "fix(ingest): tally scan coverage for the clones and imports passes too (#313)\n\nThe HEAD complexity pass learned to separate \"no row was owed\" from \"a\nrow was owed and lost\", and to warn once when coverage falls below 90%.\nIts two sibling passes did not, and for `clones` the consequence runs\nthe wrong way: `disallow_clone_type_1` is `COUNT(DISTINCT\nclone_group_id)` and passes on zero, so a scan that failed to read the\nrepository produces the same verdict as a repository with no\nduplication — the gate reads a broken scan as an improvement. `imports`\nhas the same shape feeding `max_dependency_cycles` and the\narchitecture-violation gates, where an empty graph is indistinguishable\nfrom a codebase with no dependencies.\n\n`ScanOutcome` and `ScanCoverage` move to `facts/ingest/coverage.rs`,\ngeneric over the payload so three passes carry three different result\ntypes through one accounting. `warn_if_degraded` takes the pass name and\nits fact table, so the message says both what went thin and what reads\nit. Lifting the abstraction was deliberately deferred until a second\nconsumer existed, which is this repository's convention; the second and\nthird are now here.\n\nThe classification is a faithful move. Outcomes still split on the\nper-file log level each pass already used, so the `debug!` cases (a path\n`changes` carries that HEAD no longer tracks, a file over the AST size\ncap) stay out of the denominator and only the `warn!` cases count as\nlost. Counting routine skips as losses is the mistake that once put this\nrepository at 86% and fired the warning on a scan that had failed at\nnothing.\n\nOne case needed care in the opposite direction, and it is not in the\nfinding. A file read and parsed successfully that declares no imports —\nmost files, in most repositories — previously returned the same `None`\nas a failed blob read. Routing it to `NotCounted` is the obvious reading\nof \"produced no row\" and would have been wrong: the file *was* covered.\nIt would also have shrunk the denominator, making coverage read better\nthe more import-free files a repository holds, reproducing one level up\nthe exact blindness this accounting removes. It is now `Scored` with an\nempty payload, and the drain filters empties one stage later than the\nclassifier — which is what lets the same code answer \"what did we write\"\nand \"what did we cover\" honestly at once. Removing that third match arm\n*is* the fix: fewer branches, more information.\n\nThe clones pass gained a second method along the way. The additions\npushed `populate_clones_at_head` one line past the `too_many_lines`\nceiling at 101/100, and the honest response to that is to take the seam\nthe lint points at rather than silence it: writing rows is now\n`append_clone_groups`, and the primary-key deduplication rationale about\nminified bundles lives next to the code it explains.\n\nNo ingested fact changes — the rows written to `complexity_metrics`,\n`clones` and `imports` are identical before and after.\n\nGate: cargo clippy --workspace --all-targets --all-features -- -D\nwarnings clean (0 errors, 0 warnings); cargo fmt --all --check clean;\ncoverage 8/8, ingest_test 9/9, cache_test 11/11 including the\nwhole-fact-store digest that proves the rows are unchanged.\n\nCo-authored-by: Emre Camdere <emre@valocom.nl>",
          "timestamp": "2026-08-31T13:25:51Z",
          "url": "https://github.com/emrecdr/codelore/commit/7256e468134a75b0babb50dcbd725a0e65f32c07"
        },
        "date": 1788183751814,
        "tool": "cargo",
        "benches": [
          {
            "name": "ingest_tiny",
            "value": 54098328,
            "range": "± 3585154",
            "unit": "ns/iter"
          },
          {
            "name": "ingest/medium_500_commits",
            "value": 93903057,
            "range": "± 1817868",
            "unit": "ns/iter"
          },
          {
            "name": "complexity_extraction/parallel_default_threads",
            "value": 92610778,
            "range": "± 1829644",
            "unit": "ns/iter"
          },
          {
            "name": "complexity_extraction/serial_1_thread",
            "value": 93037279,
            "range": "± 2273697",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/16",
            "value": 93762839,
            "range": "± 1628039",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/64",
            "value": 92796044,
            "range": "± 2037845",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/256",
            "value": 93292722,
            "range": "± 1579981",
            "unit": "ns/iter"
          },
          {
            "name": "ingest_capacity_sweep/1024",
            "value": 92994065,
            "range": "± 1969731",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}
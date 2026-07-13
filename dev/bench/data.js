window.BENCHMARK_DATA = {
  "lastUpdate": 1783936067822,
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
      }
    ]
  }
}
window.BENCHMARK_DATA = {
  "lastUpdate": 1783802469455,
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
      }
    ]
  }
}
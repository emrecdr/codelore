use codelore_lib::analyses::hotspots::HotspotRow;
use codelore_lib::output::sarif::write_hotspots_sarif;
use std::io::Cursor;

#[test]
fn sarif_hotspots_valid_2_1_0() {
    let rows = vec![HotspotRow {
        path: "src/main.rs".into(),
        revisions: 12,
        cognitive: 25.0,
        code_health: 50.0,
        hotspot_score: 0.75,
        mi: Some(60.0),
        mi_rank: None,
        ai_pct: None,
    }];

    let mut buf = Vec::new();
    write_hotspots_sarif(
        &rows,
        "https://github.com/example/repo",
        &mut Cursor::new(&mut buf),
    )
    .expect("write");

    let s = String::from_utf8(buf).expect("utf8");
    let parsed: serde_json::Value = serde_json::from_str(&s).expect("parse");

    // SARIF 2.1.0 structure
    assert_eq!(parsed["version"], "2.1.0");
    assert!(parsed["$schema"].as_str().unwrap().contains("sarif-2.1.0"));

    let run = &parsed["runs"][0];
    assert_eq!(run["tool"]["driver"]["name"], "codelore");
    assert_eq!(run["tool"]["driver"]["rules"][0]["id"], "CODELORE-HOTSPOT");

    let result = &run["results"][0];
    assert_eq!(result["ruleId"], "CODELORE-HOTSPOT");
    assert!(
        result["properties"]["tags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t.as_str() == Some("behavioral"))
    );
    assert!(result["properties"]["codelore/revs"].as_u64().unwrap() == 12);

    // partialFingerprints present
    let fp = result["partialFingerprints"]["primaryLocationLineHash"]
        .as_str()
        .unwrap();
    assert!(fp.starts_with("sha256:"));
}

#[test]
fn sarif_level_warning_above_threshold() {
    // SARIF level derives from security-severity bands (matches the
    // live-clone rule pattern). security-severity = (100 - code_health) / 10:
    //   ≥ 7.0 → error
    //   ≥ 4.0 → warning
    //   < 4.0 → note
    // For "warning" we need code_health ≤ 60.
    let rows = vec![HotspotRow {
        path: "src/lib.rs".into(),
        revisions: 5,
        cognitive: 10.0,
        code_health: 55.0, // → severity 4.5 → warning band
        hotspot_score: 0.6,
        mi: Some(68.0),
        mi_rank: None,
        ai_pct: None,
    }];

    let mut buf = Vec::new();
    write_hotspots_sarif(
        &rows,
        "https://github.com/example/repo",
        &mut Cursor::new(&mut buf),
    )
    .expect("write");

    let parsed: serde_json::Value = serde_json::from_str(&String::from_utf8(buf).unwrap()).unwrap();
    let result = &parsed["runs"][0]["results"][0];
    assert_eq!(result["level"], "warning");
}

#[test]
fn sarif_level_error_for_severe_findings() {
    // Severity ≥ 7.0 → error level (the most severe SARIF band).
    let rows = vec![HotspotRow {
        path: "src/severe.rs".into(),
        revisions: 50,
        cognitive: 100.0,
        code_health: 20.0, // → severity 8.0 → error band
        hotspot_score: 0.9,
        mi: Some(40.0),
        mi_rank: None,
        ai_pct: None,
    }];

    let mut buf = Vec::new();
    write_hotspots_sarif(
        &rows,
        "https://github.com/example/repo",
        &mut Cursor::new(&mut buf),
    )
    .expect("write");

    let parsed: serde_json::Value = serde_json::from_str(&String::from_utf8(buf).unwrap()).unwrap();
    let result = &parsed["runs"][0]["results"][0];
    assert_eq!(result["level"], "error");
}

#[test]
fn sarif_level_note_below_threshold() {
    let rows = vec![HotspotRow {
        path: "src/util.rs".into(),
        revisions: 2,
        cognitive: 3.0,
        code_health: 90.0,
        hotspot_score: 0.3,
        mi: Some(88.0),
        mi_rank: None,
        ai_pct: None,
    }];

    let mut buf = Vec::new();
    write_hotspots_sarif(
        &rows,
        "https://github.com/example/repo",
        &mut Cursor::new(&mut buf),
    )
    .expect("write");

    let parsed: serde_json::Value = serde_json::from_str(&String::from_utf8(buf).unwrap()).unwrap();
    let result = &parsed["runs"][0]["results"][0];
    assert_eq!(result["level"], "note");
}

#[test]
fn sarif_security_severity_proxy() {
    let rows = vec![HotspotRow {
        path: "src/danger.rs".into(),
        revisions: 30,
        cognitive: 100.0,
        code_health: 20.0,
        hotspot_score: 0.9,
        mi: Some(35.0),
        mi_rank: None,
        ai_pct: None,
    }];

    let mut buf = Vec::new();
    write_hotspots_sarif(
        &rows,
        "https://github.com/example/repo",
        &mut Cursor::new(&mut buf),
    )
    .expect("write");

    let parsed: serde_json::Value = serde_json::from_str(&String::from_utf8(buf).unwrap()).unwrap();
    let result = &parsed["runs"][0]["results"][0];
    // (100 - 20) / 10 = 8.0
    let sev = result["properties"]["security-severity"].as_f64().unwrap();
    assert!((sev - 8.0).abs() < 1e-9, "expected 8.0, got {sev}");
}

#[test]
fn sarif_fingerprint_is_stable() {
    let row = HotspotRow {
        path: "src/main.rs".into(),
        revisions: 12,
        cognitive: 25.0,
        code_health: 50.0,
        hotspot_score: 0.75,
        mi: None,
        mi_rank: None,
        ai_pct: None,
    };

    let mut buf1 = Vec::new();
    write_hotspots_sarif(
        std::slice::from_ref(&row),
        "https://github.com/example/repo",
        &mut Cursor::new(&mut buf1),
    )
    .unwrap();
    let mut buf2 = Vec::new();
    write_hotspots_sarif(
        &[row],
        "https://github.com/example/repo",
        &mut Cursor::new(&mut buf2),
    )
    .unwrap();

    let p1: serde_json::Value = serde_json::from_str(&String::from_utf8(buf1).unwrap()).unwrap();
    let p2: serde_json::Value = serde_json::from_str(&String::from_utf8(buf2).unwrap()).unwrap();

    let fp1 = p1["runs"][0]["results"][0]["partialFingerprints"]["primaryLocationLineHash"]
        .as_str()
        .unwrap();
    let fp2 = p2["runs"][0]["results"][0]["partialFingerprints"]["primaryLocationLineHash"]
        .as_str()
        .unwrap();
    assert_eq!(fp1, fp2, "fingerprint must be stable across runs");
}

#[test]
fn sarif_information_uri_points_at_codelore_repo() {
    // Every SARIF report's tool.driver.informationUri previously
    // hardcoded `github.com/emre/codescene` — wrong org, wrong project.
    // Must be the canonical codelore repo URL so GH Code Scanning's
    // tool-details link resolves.
    let rows = vec![HotspotRow {
        path: "src/main.rs".into(),
        revisions: 1,
        cognitive: 1.0,
        code_health: 90.0,
        hotspot_score: 0.1,
        mi: None,
        mi_rank: None,
        ai_pct: None,
    }];
    let mut buf = Vec::new();
    write_hotspots_sarif(
        &rows,
        "https://github.com/example/repo",
        &mut Cursor::new(&mut buf),
    )
    .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&buf).unwrap();
    let info_uri = v["runs"][0]["tool"]["driver"]["informationUri"]
        .as_str()
        .unwrap();
    assert_eq!(info_uri, "https://github.com/emrecdr/codelore");
    assert!(!info_uri.contains("emre/codescene"));
}

#[test]
fn sarif_artifact_uri_percent_encodes_special_chars() {
    // Paths with spaces / `#` / non-ASCII used to ship as raw
    // bytes in artifactLocation.uri — invalid per RFC 3986 §4.1.
    // Three probes cover the most common breakage classes.
    let probes = [
        ("src/foo bar.rs", "src/foo%20bar.rs"),
        ("docs/foo#bar.md", "docs/foo%23bar.md"),
        ("src/café.rs", "src/caf%C3%A9.rs"),
    ];
    for (raw, encoded) in probes {
        let rows = vec![HotspotRow {
            path: raw.into(),
            revisions: 1,
            cognitive: 1.0,
            code_health: 90.0,
            hotspot_score: 0.1,
            mi: None,
            mi_rank: None,
            ai_pct: None,
        }];
        let mut buf = Vec::new();
        write_hotspots_sarif(
            &rows,
            "https://github.com/example/repo",
            &mut Cursor::new(&mut buf),
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        let uri = v["runs"][0]["results"][0]["locations"][0]["physicalLocation"]
            ["artifactLocation"]["uri"]
            .as_str()
            .unwrap();
        assert!(
            uri.ends_with(encoded),
            "expected encoded {encoded:?}, got {uri:?}"
        );
        // The raw form must NOT appear in the URI.
        assert!(!uri.contains(raw), "URI {uri:?} still contains raw {raw:?}");
    }
}

#[test]
fn sarif_automation_id_is_unique_per_run() {
    // SARIF 2.1.0 §3.17.3 wants per-run correlation IDs so GH
    // Code Scanning doesn't collapse multiple runs into a single
    // timeline. Two back-to-back emissions must differ in the
    // automationDetails.id suffix.
    let rows = vec![HotspotRow {
        path: "src/main.rs".into(),
        revisions: 1,
        cognitive: 1.0,
        code_health: 90.0,
        hotspot_score: 0.1,
        mi: None,
        mi_rank: None,
        ai_pct: None,
    }];
    let emit = || {
        let mut buf = Vec::new();
        write_hotspots_sarif(
            &rows,
            "https://github.com/example/repo",
            &mut Cursor::new(&mut buf),
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        v["runs"][0]["automationDetails"]["id"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let id1 = emit();
    // Spin briefly so SystemTime nanos advance even on coarse clocks.
    std::thread::sleep(std::time::Duration::from_micros(50));
    let id2 = emit();
    assert!(
        id1.starts_with("codelore/hotspots/run/") && id2.starts_with("codelore/hotspots/run/"),
        "expected prefix preserved, got {id1:?} / {id2:?}"
    );
    assert_ne!(
        id1, id2,
        "two emissions must produce distinct correlation suffixes"
    );
}

#[test]
fn sarif_empty_rows() {
    let mut buf = Vec::new();
    write_hotspots_sarif(
        &[],
        "https://github.com/example/repo",
        &mut Cursor::new(&mut buf),
    )
    .expect("write empty");

    let parsed: serde_json::Value = serde_json::from_str(&String::from_utf8(buf).unwrap()).unwrap();
    assert_eq!(parsed["version"], "2.1.0");
    assert_eq!(parsed["runs"][0]["results"].as_array().unwrap().len(), 0);
}

#[test]
fn write_clones_sarif_emits_well_formed_doc() {
    use codelore_lib::analyses::clones::ClonesRow;
    let rows = vec![
        ClonesRow {
            clone_group_id: 1,
            fingerprint: "abcd1234".repeat(8),
            entity: "src/a.rs".into(),
            function: "add".into(),
            start_line: 10,
            end_line: 20,
            node_count: 42,
            similarity: 1.0,
            family_size: 2,
        },
        ClonesRow {
            clone_group_id: 1,
            fingerprint: "abcd1234".repeat(8),
            entity: "src/b.rs".into(),
            function: "mul".into(),
            start_line: 5,
            end_line: 15,
            node_count: 42,
            similarity: 1.0,
            family_size: 2,
        },
    ];
    let mut buf = Vec::new();
    codelore_lib::output::sarif::write_clones_sarif(&rows, "/repo", &mut buf).unwrap();
    let s = String::from_utf8(buf).unwrap();
    let v: serde_json::Value = serde_json::from_str(&s).expect("well-formed JSON");

    assert_eq!(v["version"], "2.1.0");
    assert_eq!(v["runs"][0]["tool"]["driver"]["name"], "codelore");
    assert_eq!(
        v["runs"][0]["tool"]["driver"]["rules"][0]["id"],
        "CODELORE-CLONE"
    );
    // 2 rows in 1 family → 1 result, 2 locations
    assert_eq!(v["runs"][0]["results"].as_array().unwrap().len(), 1);
    assert_eq!(
        v["runs"][0]["results"][0]["locations"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    // partialFingerprints uses the versioned key per Plan 8 §6 research brief
    assert!(
        v["runs"][0]["results"][0]["partialFingerprints"]["cloneGroupFingerprint/v1"].is_string()
    );
}

use bca_lib::analyses::hotspots::HotspotRow;
use bca_lib::output::sarif::write_hotspots_sarif;
use std::io::Cursor;

#[test]
fn sarif_hotspots_valid_2_1_0() {
    let rows = vec![HotspotRow {
        path: "src/main.rs".into(),
        name: String::new(),
        revisions: 12,
        cognitive: 25.0,
        code_health: 50.0,
        hotspot_score: 0.75,
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
    assert_eq!(run["tool"]["driver"]["name"], "bca");
    assert_eq!(run["tool"]["driver"]["rules"][0]["id"], "BCA-HOTSPOT");

    let result = &run["results"][0];
    assert_eq!(result["ruleId"], "BCA-HOTSPOT");
    assert!(
        result["properties"]["tags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t.as_str() == Some("behavioral"))
    );
    assert!(result["properties"]["bca/revs"].as_u64().unwrap() == 12);

    // partialFingerprints present
    let fp = result["partialFingerprints"]["primaryLocationLineHash"]
        .as_str()
        .unwrap();
    assert!(fp.starts_with("sha256:"));
}

#[test]
fn sarif_level_warning_above_threshold() {
    let rows = vec![HotspotRow {
        path: "src/lib.rs".into(),
        name: String::new(),
        revisions: 5,
        cognitive: 10.0,
        code_health: 80.0,
        hotspot_score: 0.6,
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
fn sarif_level_note_below_threshold() {
    let rows = vec![HotspotRow {
        path: "src/util.rs".into(),
        name: String::new(),
        revisions: 2,
        cognitive: 3.0,
        code_health: 90.0,
        hotspot_score: 0.3,
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
        name: String::new(),
        revisions: 30,
        cognitive: 100.0,
        code_health: 20.0,
        hotspot_score: 0.9,
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
        name: String::new(),
        revisions: 12,
        cognitive: 25.0,
        code_health: 50.0,
        hotspot_score: 0.75,
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

use codelore_lib::analyses::hotspots::HotspotRow;
use codelore_lib::output::json;
use std::io::Cursor;

#[test]
fn json_hotspots_emits_array() {
    let rows = vec![HotspotRow {
        path: "src/main.rs".into(),
        revisions: 4,
        cognitive: 7.0,
        cognitive_health: 75.5,
        hotspot_score: 0.42,
        mi: Some(72.0),
        mi_rank: None,
        ai_pct: None,
        hotspot_score_anchored: None,
    }];
    let mut buf = Vec::new();
    json::write_json(&rows, &mut Cursor::new(&mut buf)).expect("write");
    let s = String::from_utf8(buf).expect("utf8");
    // Expect valid JSON with the path and a numeric score
    assert!(s.starts_with('['), "should start with array");
    assert!(s.contains("\"path\": \"src/main.rs\""));
    assert!(s.contains("\"hotspot_score\""));
    // An absent anchor is omitted entirely (serde skip) — a run without an
    // active corpus stays byte-identical to the pre-anchor output.
    assert!(
        !s.contains("hotspot_score_anchored"),
        "None anchor must not serialize: {s}"
    );
}

#[test]
fn json_hotspots_serializes_anchored_when_present() {
    let rows = vec![HotspotRow {
        path: "src/main.rs".into(),
        revisions: 4,
        cognitive: 40.0,
        cognitive_health: 60.0,
        hotspot_score: 5.24,
        mi: Some(72.0),
        mi_rank: None,
        ai_pct: None,
        hotspot_score_anchored: Some(3.5),
    }];
    let mut buf = Vec::new();
    json::write_json(&rows, &mut Cursor::new(&mut buf)).expect("write");
    let s = String::from_utf8(buf).expect("utf8");
    assert!(
        s.contains("\"hotspot_score_anchored\": 3.5"),
        "present anchor must serialize: {s}"
    );
}

#[test]
fn json_revisions_emits_entity_shape() {
    let rows = vec![
        ("src/lib.rs".to_string(), 10u32),
        ("src/main.rs".to_string(), 3u32),
    ];
    let mut buf = Vec::new();
    json::write_revisions_json(&rows, &mut Cursor::new(&mut buf)).expect("write");
    let s = String::from_utf8(buf).expect("utf8");
    assert!(s.starts_with('['), "should start with array");
    assert!(s.contains("\"entity\""));
    assert!(s.contains("\"n_revs\""));
    assert!(s.contains("\"src/lib.rs\""));
}

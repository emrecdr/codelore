use codelore_lib::analyses::hotspots::HotspotRow;
use codelore_lib::output::markdown;
use std::io::Cursor;

#[test]
fn markdown_hotspots_table() {
    let rows = vec![HotspotRow {
        path: "src/main.rs".into(),
        revisions: 4,
        cognitive: 7.0,
        cognitive_health: 75.5,
        hotspot_score: 0.42,
        mi: Some(72.0),
        mi_rank: None,
        ai_pct: None,
    }];
    let mut buf = Vec::new();
    markdown::write_hotspots_markdown(&rows, &mut Cursor::new(&mut buf)).expect("write");
    let s = String::from_utf8(buf).expect("utf8");
    assert!(s.contains("# CodeLore hotspots"));
    assert!(s.contains("| Entity"));
    assert!(s.contains("| `src/main.rs`"));
}

#[test]
fn markdown_summary_compact() {
    use codelore_lib::analyses::summary::SummaryRow;
    let rows = vec![
        SummaryRow {
            metric: "commits".into(),
            value: 5,
        },
        SummaryRow {
            metric: "changes".into(),
            value: 7,
        },
    ];
    let mut buf = Vec::new();
    markdown::write_summary_markdown(&rows, &mut Cursor::new(&mut buf)).expect("write");
    let s = String::from_utf8(buf).expect("utf8");
    assert!(s.contains("# CodeLore summary"));
    assert!(s.contains("| commits | 5 |"));
}

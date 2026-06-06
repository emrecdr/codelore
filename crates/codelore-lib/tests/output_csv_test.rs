use codelore_lib::output::csv::write_revisions_csv;
use std::io::Cursor;

#[test]
fn csv_matches_code_maat_shape() {
    let rows = vec![
        ("src/main.rs".to_string(), 4u32),
        ("src/lib.rs".to_string(), 1u32),
    ];
    let mut buf = Vec::new();
    write_revisions_csv(&rows, &mut Cursor::new(&mut buf)).expect("write");
    let csv = String::from_utf8(buf).expect("utf8");
    assert_eq!(csv, "entity,n-revs\nsrc/main.rs,4\nsrc/lib.rs,1\n");
}

#[test]
fn csv_quotes_paths_containing_commas() {
    let rows = vec![("path,with,commas.rs".to_string(), 7u32)];
    let mut buf = Vec::new();
    write_revisions_csv(&rows, &mut Cursor::new(&mut buf)).expect("write");
    let csv = String::from_utf8(buf).expect("utf8");
    assert_eq!(csv, "entity,n-revs\n\"path,with,commas.rs\",7\n");
}

#[test]
fn csv_escapes_internal_quotes() {
    let rows = vec![("path\"with\"quotes.rs".to_string(), 3u32)];
    let mut buf = Vec::new();
    write_revisions_csv(&rows, &mut Cursor::new(&mut buf)).expect("write");
    let csv = String::from_utf8(buf).expect("utf8");
    // CSV escape: " → ""
    assert_eq!(csv, "entity,n-revs\n\"path\"\"with\"\"quotes.rs\",3\n");
}

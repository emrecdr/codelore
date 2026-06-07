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

#[test]
fn write_clones_csv_locks_column_order() {
    use codelore_lib::analyses::clones::ClonesRow;
    let rows = vec![
        ClonesRow {
            clone_group_id: 1,
            fingerprint: "deadbeef".repeat(8),
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
            fingerprint: "deadbeef".repeat(8),
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
    codelore_lib::output::csv::write_clones_csv(&rows, &mut buf).unwrap();
    let s = String::from_utf8(buf).unwrap();
    let expected = "\
clone-group,fingerprint,entity,function,start-line,end-line,node-count,similarity,family-size
1,deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef,src/a.rs,add,10,20,42,1.0000,2
1,deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef,src/b.rs,mul,5,15,42,1.0000,2
";
    assert_eq!(s, expected);
}

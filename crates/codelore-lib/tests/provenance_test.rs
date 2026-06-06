use codelore_lib::Options;
use codelore_lib::facts::FactsDb;
use codelore_lib::provenance::Manifest;
use codelore_lib::repo::GixRepo;

#[test]
fn manifest_captures_basic_fields() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let manifest = Manifest::capture(&db, &opts, "revisions").expect("capture");
    assert!(
        !manifest.codelore_version.is_empty(),
        "codelore_version should be populated"
    );
    assert_eq!(manifest.analysis, "revisions");
    assert_eq!(manifest.min_revs, 1);

    let json = manifest.to_json().expect("json");
    assert!(json.contains("\"analysis\""));
    assert!(json.contains("\"codelore_version\""));
}

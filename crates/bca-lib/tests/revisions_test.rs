use bca_lib::Options;
use bca_lib::analyses::revisions::run_revisions;
use bca_lib::facts::FactsDb;
use bca_lib::repo::GixRepo;

#[test]
fn revisions_for_tiny_repo() {
    let tiny = bca_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_revisions(&db, &opts).expect("run");
    // tiny repo:
    //   commit 1: add src/main.rs        -> changes (src/main.rs, Added)
    //   commit 2: modify src/main.rs     -> changes (src/main.rs, Modified)
    //   commit 3: add src/lib.rs         -> changes (src/lib.rs, Added)
    //   commit 4: modify src/main.rs     -> changes (src/main.rs, Modified)
    //   commit 5: modify src/main.rs     -> changes (src/main.rs, Modified)
    //
    // Expected: src/main.rs in 4 commits, src/lib.rs in 1 commit
    let main = rows.iter().find(|(p, _)| p == "src/main.rs").expect("main");
    assert_eq!(main.1, 4);
    let lib = rows.iter().find(|(p, _)| p == "src/lib.rs").expect("lib");
    assert_eq!(lib.1, 1);
}

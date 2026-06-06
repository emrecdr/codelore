use bca_lib::complexity::{Tier1Language, compute_for_file};
use std::path::Path;

#[test]
fn complexity_for_rust_function() {
    let src = b"fn complex(x: i32) -> i32 {
    if x > 0 {
        for i in 0..x { println!(\"{i}\"); }
    } else if x < 0 {
        match x { -1 => return -1, _ => return -2 }
    }
    0
}
";
    let entities =
        compute_for_file(Path::new("src/test.rs"), src, Tier1Language::Rust).expect("compute");

    assert!(
        !entities.is_empty(),
        "should extract at least the file unit"
    );

    // At least one entity should have meaningful complexity
    let max_cyclomatic = entities
        .iter()
        .map(|e| e.cyclomatic)
        .fold(f64::NEG_INFINITY, f64::max);
    assert!(
        max_cyclomatic > 1.0,
        "branching code should produce cyclomatic > 1, got {max_cyclomatic}"
    );
}

#[test]
fn complexity_for_python_function() {
    let src = b"def complex(x):
    if x > 0:
        for i in range(x):
            print(i)
    elif x < 0:
        return -1
    return 0
";
    let entities =
        compute_for_file(Path::new("test.py"), src, Tier1Language::Python).expect("compute");
    assert!(!entities.is_empty());

    // Python should have a function entity (not just file)
    assert!(
        entities.iter().any(|e| e.kind == "function"),
        "should extract a function entity"
    );
}

#[test]
fn complexity_returns_empty_for_invalid_source() {
    // Empty source should produce at most a unit/file entity
    let entities =
        compute_for_file(Path::new("test.rs"), b"", Tier1Language::Rust).expect("compute");
    // Either empty (parse failure → None FuncSpace → empty Vec) or one unit entity
    assert!(entities.len() <= 1);
}

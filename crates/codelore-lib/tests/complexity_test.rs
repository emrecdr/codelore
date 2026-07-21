use codelore_lib::complexity::{Tier1Language, compute_for_file};
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
    let entities = compute_for_file(Path::new("src/test.rs"), src.to_vec(), Tier1Language::Rust)
        .expect("compute");

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
    let entities = compute_for_file(Path::new("test.py"), src.to_vec(), Tier1Language::Python)
        .expect("compute");
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
        compute_for_file(Path::new("test.rs"), Vec::new(), Tier1Language::Rust).expect("compute");
    // Either empty (parse failure → None FuncSpace → empty Vec) or one unit entity
    assert!(entities.len() <= 1);
}

/// Maximum value of a metric across all extracted entities.
fn max_by(
    entities: &[codelore_lib::complexity::ComplexityEntity],
    f: impl Fn(&codelore_lib::complexity::ComplexityEntity) -> u32,
) -> u32 {
    entities.iter().map(f).max().unwrap_or(0)
}

#[test]
fn nargs_rust_four_argument_function() {
    let src = b"fn wide(a: i32, b: i32, c: i32, d: i32) -> i32 { a + b + c + d }\n";
    let entities =
        compute_for_file(Path::new("t.rs"), src.to_vec(), Tier1Language::Rust).expect("compute");
    assert_eq!(max_by(&entities, |e| e.nargs), 4, "4-arg fn → nargs == 4");
}

#[test]
fn nargs_python_four_argument_function() {
    let src = b"def wide(a, b, c, d):\n    return a + b + c + d\n";
    let entities =
        compute_for_file(Path::new("t.py"), src.to_vec(), Tier1Language::Python).expect("compute");
    assert_eq!(max_by(&entities, |e| e.nargs), 4, "4-arg fn → nargs == 4");
}

#[test]
fn nargs_javascript_four_argument_function() {
    let src = b"function wide(a, b, c, d) { return a + b + c + d; }\n";
    let entities = compute_for_file(Path::new("t.js"), src.to_vec(), Tier1Language::JavaScript)
        .expect("compute");
    assert_eq!(max_by(&entities, |e| e.nargs), 4, "4-arg fn → nargs == 4");
}

#[test]
fn max_nesting_rust_three_deep() {
    let src = b"fn deep(a: bool, b: bool, c: bool) {
    if a {
        if b {
            if c {
                println!(\"deep\");
            }
        }
    }
}
";
    let entities =
        compute_for_file(Path::new("t.rs"), src.to_vec(), Tier1Language::Rust).expect("compute");
    assert_eq!(
        max_by(&entities, |e| e.max_nesting),
        3,
        "3-deep nested if → max_nesting == 3"
    );
}

#[test]
fn max_nesting_python_three_deep() {
    let src = b"def deep(a, b, c):
    if a:
        if b:
            if c:
                print('deep')
";
    let entities =
        compute_for_file(Path::new("t.py"), src.to_vec(), Tier1Language::Python).expect("compute");
    assert_eq!(
        max_by(&entities, |e| e.max_nesting),
        3,
        "3-deep nested if → max_nesting == 3"
    );
}

#[test]
fn bool_ops_rust_and_or() {
    let src = b"fn cond(a: bool, b: bool, c: bool) {
    if a && b || c {
        println!(\"hit\");
    }
}
";
    let entities =
        compute_for_file(Path::new("t.rs"), src.to_vec(), Tier1Language::Rust).expect("compute");
    assert_eq!(
        max_by(&entities, |e| e.bool_ops),
        2,
        "`a && b || c` → bool_ops == 2 (one && sequence, one || sequence)"
    );
}

#[test]
fn bool_ops_python_and_or() {
    let src = b"def cond(a, b, c):
    if a and b or c:
        print('hit')
";
    let entities =
        compute_for_file(Path::new("t.py"), src.to_vec(), Tier1Language::Python).expect("compute");
    assert_eq!(
        max_by(&entities, |e| e.bool_ops),
        2,
        "`a and b or c` → bool_ops == 2"
    );
}

#[test]
fn bool_ops_same_operator_counts_once() {
    // A run of the same operator is a single boolean sequence.
    let src = b"fn cond(a: bool, b: bool, c: bool) {
    if a && b && c {
        println!(\"hit\");
    }
}
";
    let entities =
        compute_for_file(Path::new("t.rs"), src.to_vec(), Tier1Language::Rust).expect("compute");
    assert_eq!(
        max_by(&entities, |e| e.bool_ops),
        1,
        "`a && b && c` → bool_ops == 1 (single && sequence)"
    );
}

#[test]
fn max_nesting_java_three_deep() {
    let src = b"class X {
    void deep(boolean a, boolean b, boolean c) {
        if (a) {
            if (b) {
                if (c) {
                    System.out.println(\"deep\");
                }
            }
        }
    }
}
";
    let entities =
        compute_for_file(Path::new("t.java"), src.to_vec(), Tier1Language::Java).expect("compute");
    assert_eq!(
        max_by(&entities, |e| e.max_nesting),
        3,
        "3-deep nested if → max_nesting == 3"
    );
}

#[test]
fn bool_ops_java_and_or() {
    let src = b"class X {
    void cond(boolean a, boolean b, boolean c) {
        if (a && b || c) {
            System.out.println(\"hit\");
        }
    }
}
";
    let entities =
        compute_for_file(Path::new("t.java"), src.to_vec(), Tier1Language::Java).expect("compute");
    assert_eq!(
        max_by(&entities, |e| e.bool_ops),
        2,
        "`a && b || c` → bool_ops == 2"
    );
}

#[test]
fn max_nesting_typescript_three_deep() {
    let src = b"function deep(a: boolean, b: boolean, c: boolean) {
    if (a) {
        if (b) {
            if (c) {
                console.log('deep');
            }
        }
    }
}
";
    let entities = compute_for_file(Path::new("t.ts"), src.to_vec(), Tier1Language::TypeScript)
        .expect("compute");
    assert_eq!(
        max_by(&entities, |e| e.max_nesting),
        3,
        "3-deep nested if → max_nesting == 3"
    );
}

/// Halstead volume of the file-level `unit` entity (the whole-file aggregate).
fn unit_halstead_volume(entities: &[codelore_lib::complexity::ComplexityEntity]) -> f64 {
    entities
        .iter()
        .find(|e| e.kind == "unit")
        .and_then(|e| e.halstead_volume)
        .expect("file unit should carry a finite Halstead volume")
}

#[test]
fn tsx_with_jsx_parses_and_scores() {
    // A real `.tsx` component: an arrow function bound to a const that returns
    // JSX, with a nested `.map` callback and a ternary. tree-sitter's error
    // recovery is robust enough that the plain-TypeScript grammar still
    // recovers the control-flow shape (cyclomatic/cognitive survive), so those
    // metrics alone can't tell the two grammars apart. Where the grammars
    // genuinely diverge is Halstead: under the plain grammar the JSX tags
    // collapse into ERROR nodes whose operators/operands go uncounted, so the
    // volume is undercounted — and that error propagates into MI. Routing
    // `.tsx` through the TSX grammar counts the JSX, giving a strictly higher
    // (correct) volume.
    let src = b"export const Grid = (items: number[]) => {
    return (
        <ul>
            {items.map((x) => (
                <li key={x}>{x > 0 ? \"pos\" : \"neg\"}</li>
            ))}
        </ul>
    );
};
";
    let entities =
        compute_for_file(Path::new("Grid.tsx"), src.to_vec(), Tier1Language::Tsx).expect("compute");
    assert!(
        !entities.is_empty(),
        "TSX component should extract entities"
    );

    // The outer arrow and the nested `.map` callback are both scored.
    let max_cognitive = entities
        .iter()
        .map(|e| e.cognitive)
        .fold(f64::NEG_INFINITY, f64::max);
    assert!(
        max_cognitive >= 1.0,
        "the JSX/ternary control flow should produce cognitive >= 1, got {max_cognitive}"
    );

    // Guard the `compute_for_file` routing (`Tsx` → the TSX grammar, not the
    // plain-TypeScript grammar): parsing the same source with the plain grammar
    // undercounts Halstead operands, so the TSX volume must be strictly larger.
    let correct_volume = unit_halstead_volume(&entities);
    let error_tree_volume = unit_halstead_volume(
        &compute_for_file(
            Path::new("Grid.tsx"),
            src.to_vec(),
            Tier1Language::TypeScript,
        )
        .expect("compute"),
    );
    assert!(
        correct_volume > error_tree_volume,
        "TSX grammar must count the JSX operands the plain-TS error tree drops: \
         correct_volume={correct_volume} should exceed error_tree_volume={error_tree_volume}"
    );
}

#[test]
fn bool_ops_typescript_and_or() {
    let src = b"function cond(a: boolean, b: boolean, c: boolean) {
    if (a && b || c) {
        console.log('hit');
    }
}
";
    let entities = compute_for_file(Path::new("t.ts"), src.to_vec(), Tier1Language::TypeScript)
        .expect("compute");
    assert_eq!(
        max_by(&entities, |e| e.bool_ops),
        2,
        "`a && b || c` → bool_ops == 2"
    );
}

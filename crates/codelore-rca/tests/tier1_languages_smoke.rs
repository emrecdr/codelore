//! Smoke tests proving each Tier-1 language produces non-zero metrics
//! for a representative sample. Catches RCA upstream API regressions.
//!
//! API used:
//!   - `codelore_rca::{RustParser, JavaParser, PythonParser, TypescriptParser, JavascriptParser}` — typed
//!     parser structs (public via the `mk_code!` macro in langs.rs)
//!   - `codelore_rca::ParserTrait::new(code, path, None)` — construct a parser from source bytes
//!   - `codelore_rca::metrics(&parser, path)` — returns `Option<FuncSpace>` (spaces.rs)
//!   - `func_space.metrics.cyclomatic.cyclomatic()` / `.cognitive.cognitive()` — per-unit totals

use std::path::Path;

use codelore_rca::{
    JavaParser, JavascriptParser, ParserTrait, PythonParser, RustParser, TypescriptParser, metrics,
};

// ---------------------------------------------------------------------------
// Helper: run RCA metrics on raw source bytes via the given parser type.
// Returns the root FuncSpace (SpaceKind::Unit) so tests can inspect .metrics.
// ---------------------------------------------------------------------------
fn run_metrics<T: ParserTrait>(source: &[u8], filename: &str) -> codelore_rca::FuncSpace {
    // RCA convention: source must end with a newline.
    let mut code = source.trim_ascii_end().to_vec();
    code.push(b'\n');
    let path = Path::new(filename);
    let parser = T::new(code, path, None);
    metrics(&parser, path).expect("metrics() returned None — parse likely failed")
}

// ---------------------------------------------------------------------------
// Representative multi-branch snippets for each Tier-1 language.
// Each exercises: if/else-if, loop, and a multi-way branch (match/switch/etc.)
// so that both Cyclomatic and Cognitive are guaranteed > 0 at the root level.
// ---------------------------------------------------------------------------

const RUST_SRC: &[u8] = b"fn complex(x: i32) -> i32 {
    if x > 0 {
        for i in 0..x { println!(\"{i}\"); }
    } else if x < 0 {
        match x { -1 => return -1, _ => return -2 }
    }
    0
}
";

const PYTHON_SRC: &[u8] = b"def complex(x):
    if x > 0:
        for i in range(x):
            print(i)
    elif x < 0:
        if x == -1:
            return -1
        return -2
    return 0
";

const TYPESCRIPT_SRC: &[u8] = b"function complex(x: number): number {
    if (x > 0) {
        for (let i = 0; i < x; i++) { console.log(i); }
    } else if (x < 0) {
        switch (x) { case -1: return -1; default: return -2; }
    }
    return 0;
}
";

const JAVASCRIPT_SRC: &[u8] = b"function complex(x) {
    if (x > 0) {
        for (let i = 0; i < x; i++) { console.log(i); }
    } else if (x < 0) {
        switch (x) { case -1: return -1; default: return -2; }
    }
    return 0;
}
";

const JAVA_SRC: &[u8] = b"class C {
    int complex(int x) {
        if (x > 0) {
            for (int i = 0; i < x; i++) { System.out.println(i); }
        } else if (x < 0) {
            switch (x) { case -1: return -1; default: return -2; }
        }
        return 0;
    }
}
";

// ---------------------------------------------------------------------------
// Tier-1 smoke tests
// ---------------------------------------------------------------------------

#[test]
fn rust_cyclomatic_and_cognitive() {
    let space = run_metrics::<RustParser>(RUST_SRC, "complex.rs");
    // cyclomatic_sum() aggregates across all spaces (unit + functions).
    // For complex branching code the sum is always > 2 (unit=1, function>=2).
    let cyclo = space.metrics.cyclomatic.cyclomatic_sum();
    // cognitive_sum() aggregates structural nesting penalties across all functions.
    let cogn = space.metrics.cognitive.cognitive_sum();
    assert!(
        cyclo > 2.0,
        "Rust cyclomatic_sum should be > 2, got {cyclo}"
    );
    assert!(cogn > 0.0, "Rust cognitive_sum should be > 0, got {cogn}");
}

#[test]
fn python_cyclomatic_and_cognitive() {
    let space = run_metrics::<PythonParser>(PYTHON_SRC, "complex.py");
    let cyclo = space.metrics.cyclomatic.cyclomatic_sum();
    let cogn = space.metrics.cognitive.cognitive_sum();
    assert!(
        cyclo > 2.0,
        "Python cyclomatic_sum should be > 2, got {cyclo}"
    );
    assert!(cogn > 0.0, "Python cognitive_sum should be > 0, got {cogn}");
}

#[test]
fn typescript_cyclomatic_and_cognitive() {
    let space = run_metrics::<TypescriptParser>(TYPESCRIPT_SRC, "complex.ts");
    let cyclo = space.metrics.cyclomatic.cyclomatic_sum();
    let cogn = space.metrics.cognitive.cognitive_sum();
    assert!(
        cyclo > 2.0,
        "TypeScript cyclomatic_sum should be > 2, got {cyclo}"
    );
    assert!(
        cogn > 0.0,
        "TypeScript cognitive_sum should be > 0, got {cogn}"
    );
}

#[test]
fn javascript_cyclomatic_and_cognitive() {
    let space = run_metrics::<JavascriptParser>(JAVASCRIPT_SRC, "complex.js");
    let cyclo = space.metrics.cyclomatic.cyclomatic_sum();
    let cogn = space.metrics.cognitive.cognitive_sum();
    assert!(
        cyclo > 2.0,
        "JavaScript cyclomatic_sum should be > 2, got {cyclo}"
    );
    assert!(
        cogn > 0.0,
        "JavaScript cognitive_sum should be > 0, got {cogn}"
    );
}

#[test]
fn java_cyclomatic_and_cognitive() {
    let space = run_metrics::<JavaParser>(JAVA_SRC, "complex.java");
    let cyclo = space.metrics.cyclomatic.cyclomatic_sum();
    let cogn = space.metrics.cognitive.cognitive_sum();
    assert!(
        cyclo > 2.0,
        "Java cyclomatic_sum should be > 2, got {cyclo}"
    );
    assert!(cogn > 0.0, "Java cognitive_sum should be > 0, got {cogn}");
}

// ---------------------------------------------------------------------------
// Optional: Halstead volume is reachable under the metrics-experimental flag.
// We don't assert correctness (upstream tracks known accuracy issues), only
// that the accessor compiles and returns a finite f64.
// ---------------------------------------------------------------------------
#[cfg(feature = "metrics-experimental")]
#[test]
fn jsts_halstead_with_experimental_flag() {
    let src = b"function f(x) { return x + 1; }";
    let space = run_metrics::<JavascriptParser>(src, "f.js");
    let vol = space.metrics.halstead.volume();
    assert!(
        vol.is_finite(),
        "Halstead volume should be finite, got {vol}"
    );
}

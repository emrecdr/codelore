use std::path::Path;
use tree_sitter::Language;

use crate::macros::{
    get_language, mk_action, mk_code, mk_emacs_mode, mk_extensions, mk_lang, mk_langs,
};
use crate::*;

mk_langs!(
    // 1) Name for enum
    // 2) Language description
    // 3) Display name
    // 4) Empty struct name to implement
    // 5) Parser name
    // 6) tree-sitter function to call to get a Language
    // 7) file extensions
    // 8) emacs modes
    // Mozjs removed: Mozilla-specific tree-sitter-js fork (bca vendor drop)
    (
        Javascript,
        "The `JavaScript` language",
        "javascript",
        JavascriptCode,
        JavascriptParser,
        tree_sitter_javascript,
        [js, jsm, mjs, jsx],
        ["js", "js2"]
    ),
    (
        Java,
        "The `Java` language",
        "java",
        JavaCode,
        JavaParser,
        tree_sitter_java,
        [java],
        ["java"]
    ),
    (
        Rust,
        "The `Rust` language",
        "rust",
        RustCode,
        RustParser,
        tree_sitter_rust,
        [rs],
        ["rust"]
    ),
    (
        Python,
        "The `Python` language",
        "python",
        PythonCode,
        PythonParser,
        tree_sitter_python,
        [py],
        ["python"]
    ),
    (
        Tsx,
        "The `Tsx` language incorporates the `JSX` syntax inside `TypeScript`",
        "typescript",
        TsxCode,
        TsxParser,
        tree_sitter_tsx,
        [tsx],
        []
    ),
    (
        Typescript,
        "The `TypeScript` language",
        "typescript",
        TypescriptCode,
        TypescriptParser,
        tree_sitter_typescript,
        [ts, jsw, jsmw],
        ["typescript"]
    )
);

pub(crate) mod fake {
    pub(crate) fn get_true<'a>(ext: &str, mode: &str) -> Option<&'a str> {
        if ext == "m"
            || ext == "mm"
            || mode == "objc"
            || mode == "objc++"
            || mode == "objective-c++"
            || mode == "objective-c"
        {
            Some("obj-c/c++")
        } else {
            None
        }
    }
}

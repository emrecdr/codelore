//! Compile-time test: confirms Repo trait shape exists.
//! Real integration tests against a fixture repo land in Task 9.

use bca_lib::CommitEvent;
use bca_lib::Options;
use bca_lib::repo::{CommitMetadata, Repo};

#[allow(
    dead_code,
    unreachable_code,
    unused_variables,
    clippy::diverging_sub_expression
)]
fn _trait_object_compiles<R: Repo>(r: &R) {
    let _: Box<dyn Iterator<Item = bca_lib::Result<CommitEvent>>> = unimplemented!();
    let opts = Options::default();
    let _: bca_lib::Result<Box<dyn Iterator<Item = bca_lib::Result<CommitEvent>> + Send + '_>> =
        r.walk_commits(&opts);
    let _: bca_lib::Result<Vec<bca_lib::FileChange>> = r.changed_files("abc");
    let _: bca_lib::Result<Vec<bca_lib::Hunk>> = r.diff_hunks("abc", "src/main.rs");
    let _: String = r.resolve_alias("a@b.com");
    let _: bca_lib::Result<CommitMetadata> = r.commit_metadata("abc");
}

//! Test fixtures. Public so CLI integration tests can reuse.
//!
//! `tiny_repo::build()` programmatically constructs a 5-commit repo via
//! shell-out to `git` so behavior is exactly reproducible. We use git CLI
//! (not gix-write) because gix-write is still maturing for trivial init in 0.84,
//! and shell-out is fast + predictable for tests.

#[cfg(feature = "test-support")]
pub mod tiny_repo {
    use std::path::PathBuf;
    use tempfile::TempDir;

    pub struct TinyRepo {
        pub dir: TempDir,
        pub head_sha: String,
    }

    /// Build a tiny 5-commit repository for testing.
    ///
    /// # Panics
    ///
    /// Panics if the OS cannot create a temporary directory or if any `git` command fails.
    #[must_use]
    pub fn build() -> TinyRepo {
        let dir = tempfile::tempdir().expect("tempdir");
        let path: PathBuf = dir.path().to_path_buf();

        run_git(&path, &["init", "-b", "main", "--quiet"]);
        run_git(&path, &["config", "user.email", "tiny@example.com"]);
        run_git(&path, &["config", "user.name", "Tiny"]);

        write(&path, "src/main.rs", "fn main() {}\n");
        run_git(&path, &["add", "."]);
        run_git(&path, &["commit", "-m", "init", "--quiet"]);

        write(&path, "src/main.rs", "fn main() { println!(\"hi\"); }\n");
        run_git(&path, &["commit", "-am", "say hi", "--quiet"]);

        write(&path, "src/lib.rs", "pub fn greet() {}\n");
        run_git(&path, &["add", "."]);
        run_git(&path, &["commit", "-m", "add lib", "--quiet"]);

        write(&path, "src/main.rs", "fn main() { println!(\"hello\"); }\n");
        run_git(&path, &["commit", "-am", "fix typo", "--quiet"]);

        write(
            &path,
            "src/main.rs",
            "fn main() { println!(\"hello, world\"); }\n",
        );
        run_git(&path, &["commit", "-am", "expand greeting", "--quiet"]);

        let head_sha = String::from_utf8(
            std::process::Command::new("git")
                .args(["-C", path.to_str().unwrap(), "rev-parse", "HEAD"])
                .output()
                .expect("git rev-parse")
                .stdout,
        )
        .expect("utf8")
        .trim()
        .to_string();

        TinyRepo { dir, head_sha }
    }

    fn run_git(path: &std::path::Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .status()
            .expect("git");
        assert!(status.success(), "git {args:?} failed");
    }

    fn write(root: &std::path::Path, rel: &str, content: &str) {
        let p = root.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, content).unwrap();
    }
}

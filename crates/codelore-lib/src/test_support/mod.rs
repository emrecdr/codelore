//! Test fixtures. Public so CLI integration tests can reuse.
//!
//! `tiny_repo::build()` programmatically constructs a 5-commit repo via
//! shell-out to `git` so behavior is exactly reproducible. We use git CLI
//! (not gix-write) because gix-write is still maturing for trivial init in 0.84,
//! and shell-out is fast + predictable for tests.
//!
//! `differential_repo::build()` constructs a 50-commit repo exercising every
//! `Repo`-trait edge case: 3 authors + 1 bot, .mailmap, 1 rename, 1 merge,
//! deterministic dates — used to assert `GixRepo` ≡ `GitCliRepo`.

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

#[cfg(feature = "test-support")]
pub mod differential_repo {
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::Mutex;
    use tempfile::TempDir;

    pub struct DifferentialRepo {
        pub dir: TempDir,
        pub head_sha: String,
    }

    /// Serializes concurrent `build()` calls across test threads.
    ///
    /// `differential_repo::build()` runs ~100 `git` invocations (50 commits ×
    /// `add` + `commit`). When multiple tests in the same binary call `build()`
    /// in parallel (cargo's default), the high rate of `git` invocations
    /// across distinct tempdirs from the same parent process produces
    /// intermittent "invalid object … Error building trees" failures even
    /// with `gc.auto = 0`. The race surfaces a few percent of the time on
    /// `cargo test --workspace --all-features` and reliably under
    /// `cargo bench` (~500 commits in `medium_repo` magnifies the rate).
    ///
    /// Each test still gets its own tempdir + git history; only the
    /// build-up of fixtures is serialized. The analysis phase that
    /// follows is fully parallel.
    static BUILD_MUTEX: Mutex<()> = Mutex::new(());

    /// Build a 50-commit fixture exercising every `Repo` trait method's edge cases:
    /// 3 authors + 1 bot, .mailmap, 1 rename, 1 merge, deterministic dates.
    ///
    /// Commit layout (counting from 1):
    ///  1      — init + .mailmap (Alice)
    ///  2..29  — 28 round-robin commits on 10 files (Alice/Bob/Carol)
    ///  30     — bot commit (dependabot) bumping `Cargo.lock`
    ///  31     — rename `src/old_name.rs` → `src/new_name.rs` (Alice)
    ///  32..47 — 16 more round-robin commits (Alice/Bob/Carol)
    ///  48     — `feature/x` branch commit (Bob) — `src/api.rs`
    ///  49     — merge --no-ff `feature/x` into main
    ///  50     — final docs commit (Carol) — `README.md`
    ///
    /// Total: 50 commits (including 1 merge).
    ///
    /// # Panics
    ///
    /// Panics if any git command fails.
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn build() -> DifferentialRepo {
        // Serialize fixture builds across test threads — concurrent `git`
        // invocation storms from the same parent process produce
        // intermittent "invalid object" errors even with `gc.auto = 0`.
        // See the BUILD_MUTEX comment above for the full diagnosis.
        let _guard = BUILD_MUTEX.lock().unwrap_or_else(std::sync::PoisonError::into_inner);

        let dir = tempfile::tempdir().expect("tempdir");
        let path: PathBuf = dir.path().to_path_buf();

        run_git(&path, &["init", "-b", "main", "--quiet"]);
        run_git(&path, &["config", "user.email", "noop@example.com"]);
        run_git(&path, &["config", "user.name", "Noop"]);
        // Disable auto-gc to remove one race source even with the mutex
        // (other parallel tests may still run their own git operations).
        run_git(&path, &["config", "gc.auto", "0"]);

        // .mailmap — committed as part of commit 1 so resolve_alias works
        write(
            &path,
            ".mailmap",
            "Alice Canonical <canonical-alice@example.com> Alice <alice-old@example.com>\n\
             Bob Canonical <canonical-bob@example.com> <bob-aliased@example.com>\n\
             Carol Lee <carol@example.com> Carol <c.lee@example.com>\n",
        );

        let authors: [(&str, &str); 3] = [
            ("Alice", "alice-old@example.com"), // → canonical-alice
            ("Bob", "bob-aliased@example.com"), // → canonical-bob
            ("Carol", "c.lee@example.com"),     // → carol
        ];
        let bot_author = (
            "dependabot[bot]",
            "49699333+dependabot[bot]@users.noreply.github.com",
        );

        let base_date = "2026-01-01T12:00:00Z";
        let mut hour: u32 = 0;

        // ── Commit 1: init ────────────────────────────────────────────────────
        write(&path, "src/main.rs", "fn main() {}\n");
        commit_as(
            &path,
            &authors[0],
            base_date,
            hour,
            "init",
            &[".mailmap", "src/main.rs"],
        );
        hour += 1;

        // ── Commits 2–29: 28 round-robin on 10 files ─────────────────────────
        // Note: src/old_name.rs is included here so it exists before the rename.
        let files: [&str; 10] = [
            "src/lib.rs",
            "src/util.rs",
            "src/parser.rs",
            "src/format.rs",
            "src/io.rs",
            "src/cli.rs",
            "src/errors.rs",
            "README.md",
            "src/old_name.rs",
            "src/api.rs",
        ];
        for i in 0..28_usize {
            let author = &authors[i % 3];
            let file = files[i % files.len()];
            let content = format!("// commit {}\nfn fn_{i}() {{}}\n", i + 2);
            write(&path, file, &content);
            commit_as(
                &path,
                author,
                base_date,
                hour,
                &format!("change {file}"),
                &[file],
            );
            hour += 1;
        }

        // ── Commit 30: bot commit ─────────────────────────────────────────────
        write(&path, "Cargo.lock", "# bumped\n");
        commit_as(
            &path,
            &bot_author,
            base_date,
            hour,
            "deps: bump",
            &["Cargo.lock"],
        );
        hour += 1;

        // ── Commit 31: rename src/old_name.rs → src/new_name.rs ──────────────
        run_git(&path, &["mv", "src/old_name.rs", "src/new_name.rs"]);
        commit_as(
            &path,
            &authors[0],
            base_date,
            hour,
            "rename old_name to new_name",
            &[], // empty → git add -A
        );
        hour += 1;

        // ── Commits 32–47: 16 more round-robin commits ────────────────────────
        // old_name.rs is gone — avoid it; use first 8 files (indices 0..7) plus
        // src/new_name.rs and src/api.rs.
        let post_rename_files: [&str; 10] = [
            "src/lib.rs",
            "src/util.rs",
            "src/parser.rs",
            "src/format.rs",
            "src/io.rs",
            "src/cli.rs",
            "src/errors.rs",
            "README.md",
            "src/new_name.rs",
            "src/api.rs",
        ];
        for i in 0..16_usize {
            let author = &authors[i % 3];
            let file = post_rename_files[i % post_rename_files.len()];
            let content = format!("// later commit {}\nfn later_{i}() {{}}\n", i + 32);
            write(&path, file, &content);
            commit_as(
                &path,
                author,
                base_date,
                hour,
                &format!("later change to {file}"),
                &[file],
            );
            hour += 1;
        }

        // ── Commits 48–49: merge sequence ─────────────────────────────────────
        // Commit 48: feature/x branch commit
        run_git(&path, &["checkout", "-b", "feature/x", "--quiet"]);
        write(&path, "src/api.rs", "// feature x\npub fn feature_x() {}\n");
        commit_as(
            &path,
            &authors[1],
            base_date,
            hour,
            "feat: feature x",
            &["src/api.rs"],
        );
        hour += 1;

        // Commit 49: merge --no-ff back into main
        run_git(&path, &["checkout", "main", "--quiet"]);
        let merge_env = commit_env(base_date, hour);
        let merge_status = Command::new("git")
            .arg("-C")
            .arg(&path)
            .args([
                "merge",
                "--no-ff",
                "-m",
                "Merge feature/x",
                "feature/x",
                "--quiet",
            ])
            .envs(merge_env)
            .status()
            .expect("git merge");
        assert!(merge_status.success(), "git merge failed");
        hour += 1;

        // ── Commit 50: final docs commit ──────────────────────────────────────
        write(&path, "README.md", "# Final\n");
        commit_as(
            &path,
            &authors[2],
            base_date,
            hour,
            "docs: final",
            &["README.md"],
        );

        let head_sha = String::from_utf8(
            Command::new("git")
                .arg("-C")
                .arg(&path)
                .args(["rev-parse", "HEAD"])
                .output()
                .expect("git rev-parse")
                .stdout,
        )
        .expect("utf8")
        .trim()
        .to_string();

        DifferentialRepo { dir, head_sha }
    }

    /// Compute the ISO-8601 datetime string for `base_date` + `hour_offset` hours.
    /// Base is 2026-01-01T12:00:00Z; we keep totals ≤ 999 hours for simplicity.
    fn commit_env(base_date: &str, hour_offset: u32) -> Vec<(String, String)> {
        // base_date must be "2026-01-01T12:00:00Z"
        let _ = base_date; // not parsed — we compute directly
        let total_hours = 12 + hour_offset;
        let day_extra = total_hours / 24;
        let hour_of_day = total_hours % 24;
        let date = format!("2026-01-{:02}T{:02}:00:00Z", 1 + day_extra, hour_of_day);
        vec![
            ("GIT_AUTHOR_DATE".to_string(), date.clone()),
            ("GIT_COMMITTER_DATE".to_string(), date),
        ]
    }

    fn commit_as(
        path: &std::path::Path,
        author: &(&str, &str),
        base_date: &str,
        hour_offset: u32,
        msg: &str,
        add_files: &[&str],
    ) {
        if add_files.is_empty() {
            run_git(path, &["add", "-A"]);
        } else {
            for f in add_files {
                run_git(path, &["add", f]);
            }
        }
        let envs = commit_env(base_date, hour_offset);
        let author_str = format!("{} <{}>", author.0, author.1);
        let status = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["commit", "-m", msg, "--author", &author_str, "--quiet"])
            .envs(envs)
            .status()
            .expect("git commit");
        assert!(status.success(), "commit failed: {msg}");
    }

    fn run_git(path: &std::path::Path, args: &[&str]) {
        let status = Command::new("git")
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

#[cfg(feature = "test-support")]
pub mod medium_repo {
    //! 500-commit fixture for criterion benchmarks. Heavier than `differential_repo`
    //! (which is 50 commits, optimized for differential-test edge-case coverage) but
    //! still small enough for a CI bench to iterate on in under ~10 seconds.
    //!
    //! Structure: 500 commits, 3 authors, 25 files, round-robin author×file, no
    //! merges, no renames, deterministic dates. Intended use: measure ingest+walk
    //! throughput without any of the edge cases `differential_repo` exercises.

    use std::path::PathBuf;
    use std::process::Command;
    use tempfile::TempDir;

    pub struct MediumRepo {
        pub dir: TempDir,
        pub head_sha: String,
    }

    /// Build a 500-commit fixture for criterion benches.
    ///
    /// # Panics
    ///
    /// Panics if any git command fails.
    #[must_use]
    pub fn build() -> MediumRepo {
        const COMMIT_COUNT: usize = 500;
        const FILE_COUNT: usize = 25;

        let dir = tempfile::tempdir().expect("tempdir");
        let path: PathBuf = dir.path().to_path_buf();
        run_git(&path, &["init", "-b", "main", "--quiet"]);
        run_git(&path, &["config", "user.email", "bench@example.com"]);
        run_git(&path, &["config", "user.name", "Bench"]);
        // Disable auto-gc: 500 rapid commits can trigger gc and prune loose
        // blobs between `git add` and the next `git commit`, producing
        // "invalid object" errors. Disable upfront for the bench fixture.
        run_git(&path, &["config", "gc.auto", "0"]);

        let authors = [
            ("Alice", "alice@example.com"),
            ("Bob", "bob@example.com"),
            ("Carol", "carol@example.com"),
        ];

        for i in 0..COMMIT_COUNT {
            let file_idx = i % FILE_COUNT;
            let rel = format!("src/mod_{file_idx:02}.rs");
            let content = format!(
                "// commit {i}\npub fn fn_{i}() -> u32 {{ {i} }}\n\n\
                 #[cfg(test)]\nmod tests {{\n    use super::*;\n    \
                 #[test] fn t_{i}() {{ assert_eq!(fn_{i}(), {i}); }}\n}}\n"
            );
            write(&path, &rel, &content);
            let (name, email) = authors[i % 3];
            commit_at(&path, name, email, i, &format!("touch {rel}"), &[&rel]);

            // Pack loose objects every 50 commits to prevent "invalid object"
            // errors on macOS/APFS where the OS may delay flushing loose blob
            // writes to the object store. `git repack -d` packs all loose
            // objects into a single packfile and prunes the loose originals,
            // eliminating the race between `git add` (write loose object) and
            // `git commit` (write-tree reads that same object). We use
            // `repack -d` instead of `gc --quiet` because `gc` has additional
            // phases (expire, reflog) that can fail when packfiles are created
            // concurrently on macOS/APFS.
            if i > 0 && i % 50 == 49 {
                run_git(&path, &["repack", "-d", "--quiet"]);
            }
        }

        let head_sha = String::from_utf8(
            Command::new("git")
                .arg("-C")
                .arg(&path)
                .args(["rev-parse", "HEAD"])
                .output()
                .expect("git rev-parse")
                .stdout,
        )
        .expect("utf8")
        .trim()
        .to_string();

        MediumRepo { dir, head_sha }
    }

    fn commit_at(
        path: &std::path::Path,
        name: &str,
        email: &str,
        sequence: usize,
        msg: &str,
        files: &[&str],
    ) {
        for f in files {
            run_git(path, &["add", f]);
        }
        let date = format!(
            "2026-01-{:02}T{:02}:{:02}:00Z",
            1 + (sequence / (24 * 60)),
            (sequence / 60) % 24,
            sequence % 60
        );
        let author = format!("{name} <{email}>");
        let status = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["commit", "-m", msg, "--author", &author, "--quiet"])
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", &date)
            .status()
            .expect("git commit");
        assert!(status.success(), "commit failed: {msg}");
    }

    fn run_git(path: &std::path::Path, args: &[&str]) {
        let status = Command::new("git")
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

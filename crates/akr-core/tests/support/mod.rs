//! A throwaway git repository for freshness tests.
//!
//! Real repositories, not a mocked git: the whole point of phase P5 is that ancestry and
//! changed-path detection are answered by the tool that understands them, and a fake
//! would only prove that the fake agrees with itself. `git init` in a temp directory is
//! milliseconds, so the honest test is also the cheap one.
//!
//! Every repository is deterministic: fixed author, fixed committer, fixed timestamps, no
//! GPG signing, no user config. Two runs produce different commit hashes only because the
//! test bodies differ, never because the environment does.

#![allow(dead_code)]

pub mod history;

#[allow(unused_imports)]
pub use history::{Step, SyntheticHistory};

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A temporary git repository, removed when it drops.
pub struct TempRepo {
    root: PathBuf,
    clock: u32,
}

impl TempRepo {
    /// Creates an empty repository with a deterministic identity.
    pub fn new(name: &str) -> Self {
        let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root =
            std::env::temp_dir().join(format!("akr-p5-{name}-{}-{unique}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp directory");

        let repo = Self { root, clock: 0 };
        repo.git(&["init", "--quiet", "--initial-branch=main"]);
        repo.git(&["config", "user.name", "AKR Test"]);
        repo.git(&["config", "user.email", "test@example.invalid"]);
        repo.git(&["config", "commit.gpgsign", "false"]);
        repo.git(&["config", "gc.auto", "0"]);
        repo
    }

    /// The repository root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Writes a file, creating parent directories.
    pub fn write(&self, path: &str, contents: &str) {
        let full = self.root.join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).expect("parent directory");
        }
        std::fs::write(full, contents).expect("write");
    }

    /// Deletes a file.
    pub fn remove(&self, path: &str) {
        std::fs::remove_file(self.root.join(path)).expect("remove");
    }

    /// Renames a file through git, so the rename is recorded as one.
    pub fn rename(&self, from: &str, to: &str) {
        let full = self.root.join(to);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).expect("parent directory");
        }
        self.git(&["mv", from, to]);
    }

    /// Stages everything and commits, returning the commit hash.
    ///
    /// Timestamps advance by a fixed step per commit, so history order is stable and
    /// nothing depends on how fast the test ran.
    pub fn commit(&mut self, message: &str) -> String {
        self.clock += 1;
        let stamp = format!("2026-01-{:02}T12:00:00+00:00", self.clock.min(28));
        self.git(&["add", "-A"]);
        self.git_env(
            &["commit", "--quiet", "--allow-empty", "-m", message],
            &[("GIT_AUTHOR_DATE", &stamp), ("GIT_COMMITTER_DATE", &stamp)],
        );
        self.rev_parse("HEAD")
    }

    /// Writes one file and commits it in one step.
    pub fn commit_file(&mut self, path: &str, contents: &str, message: &str) -> String {
        self.write(path, contents);
        self.commit(message)
    }

    /// Resolves a revision to a full hash.
    pub fn rev_parse(&self, revision: &str) -> String {
        self.git(&["rev-parse", revision]).trim().to_owned()
    }

    /// Creates and checks out a branch at the current HEAD.
    pub fn branch(&self, name: &str) {
        self.git(&["checkout", "--quiet", "-b", name]);
    }

    /// Checks out an existing branch or commit.
    pub fn checkout(&self, name: &str) {
        self.git(&["checkout", "--quiet", name]);
    }

    /// Moves the current branch to `revision`, discarding everything after it — the
    /// history rewrite a rebase or a force-push leaves behind.
    pub fn reset_hard(&self, revision: &str) {
        self.git(&["reset", "--quiet", "--hard", revision]);
    }

    /// Merges a branch into the current one, always creating a merge commit.
    pub fn merge(&mut self, name: &str) -> String {
        self.clock += 1;
        let stamp = format!("2026-02-{:02}T12:00:00+00:00", self.clock.min(28));
        self.git_env(
            &["merge", "--quiet", "--no-ff", "-m", "merge", name],
            &[("GIT_AUTHOR_DATE", &stamp), ("GIT_COMMITTER_DATE", &stamp)],
        );
        self.rev_parse("HEAD")
    }

    fn git(&self, args: &[&str]) -> String {
        self.git_env(args, &[])
    }

    fn git_env(&self, args: &[&str], env: &[(&str, &str)]) -> String {
        let mut command = Command::new("git");
        command.args(args).current_dir(&self.root);
        for (key, value) in env {
            command.env(key, value);
        }
        let output = command.output().expect("git runs");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

//! Mutual exclusion between writers, and the check that catches what it cannot cover.
//!
//! # Why the pipeline needs it
//!
//! `docs/07-cli.md` §4 is a read-modify-write: step 1 reads every source, steps 2–4 edit
//! and format a ledger held in memory, and step 5 writes each touched file whole. Each
//! file arrives through a temporary and a rename, so a file is never half-written — but
//! per-file atomicity says nothing about two writers, and there was nothing else to say
//! it. Two processes that both read before either wrote each held a complete ledger that
//! did not know about the other's record, and both then wrote that ledger out. The second
//! rename won and the first record was gone, with both processes reporting success.
//!
//! That is not a rare interleaving. Six `akr papercut` processes started together landed
//! two records and printed "wrote" six times
//! (`@akr.observation.concurrent-writes-are-silently-lost`). Several agent hosts sharing a
//! workspace is the normal way this ledger is used, so the window is open constantly.
//!
//! `base_rev` (`docs/08-mcp.md`) does not close it. That is optimistic concurrency for
//! `revise`, and it compares one key's head revision; a *new* key appended to a shared
//! records file conflicts with nobody's `base_rev` and passes the check untouched.
//!
//! # Two mechanisms, because one is not enough
//!
//! [`WriteLock`] serialises AKR's own writers, so concurrent writes all succeed rather
//! than all but one failing. It is an advisory lock held on a file in the disposable cache
//! directory, taken through [`std::fs::File::lock`]: the kernel releases it when the
//! handle closes, which includes a process that panics or is killed, so there is no stale
//! lock to reap and no timeout to tune.
//!
//! A lock only binds the processes that take it. A human editing a records file, a `git
//! checkout`, or a build of AKR too old to lock will still change a source under a
//! writer's feet — and a filesystem that does not implement locking will refuse the lock
//! outright. [`verify_unchanged`] is the backstop for all of those: before the rename, the
//! bytes on disk must still be the bytes the operation read. When they are not, the write
//! is refused with `AKR-C034` and nothing is written, which is the same promise every
//! other refusal in the pipeline makes.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The lock file, under the cache directory because it is disposable and already ignored.
///
/// It is deliberately not a `.akr` source: the loader takes files by their `.akr`
/// extension, and a lock is not part of the ledger. `.akr/cache/` is gitignored, so the
/// file never reaches a commit.
fn lock_path(akr_dir: &Path) -> PathBuf {
    akr_dir.join("cache").join("write.lock")
}

/// An exclusive claim on one workspace's write pipeline, held for the whole operation.
///
/// Dropping it releases the claim. So does the process ending, however it ends, because
/// the lock lives on an open file handle rather than on the file's existence.
#[derive(Debug)]
pub struct WriteLock {
    /// `None` when the filesystem could not provide a lock; the write still proceeds,
    /// guarded by [`verify_unchanged`] alone.
    file: Option<fs::File>,
}

impl WriteLock {
    /// Waits for exclusive access to `akr_dir`'s write pipeline.
    ///
    /// Blocks while another writer holds it. The critical section is one parse, one
    /// validation and one write of a handful of files — no git, no index — so a queue of
    /// agents moves through it in milliseconds each.
    ///
    /// A filesystem that cannot lock is not a reason to refuse a write: some network
    /// mounts have no working `flock`, and AKR worked on them before this existed. The
    /// lock is skipped there and [`verify_unchanged`] still refuses a clobber, which
    /// degrades the guarantee from "writers queue" to "a racing writer is told" rather
    /// than back to silent loss.
    #[must_use]
    pub fn acquire(akr_dir: &Path) -> Self {
        let path = lock_path(akr_dir);
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .ok()
            .filter(|file| file.lock().is_ok());
        Self { file }
    }

    /// Whether a real lock is held, as distinct from having degraded to the check alone.
    #[must_use]
    pub fn is_held(&self) -> bool {
        self.file.is_some()
    }
}

impl Drop for WriteLock {
    fn drop(&mut self) {
        if let Some(file) = &self.file {
            // Closing the handle releases it too; unlocking first makes the release a
            // statement rather than a side effect, and reports nothing because a failure
            // to unlock is followed immediately by the close that unlocks anyway.
            let _ = file.unlock();
        }
    }
}

/// Confirms that every file this write is about to replace still holds what it read.
///
/// `expected` maps a path relative to `akr_dir` to the text the operation loaded, or
/// `None` for a file that did not exist then. The first path that disagrees is returned,
/// and the caller refuses the write with it named.
///
/// An unreadable file counts as changed. That is the safe direction: the alternative is
/// to overwrite a file whose current contents could not be established.
pub fn verify_unchanged<'a>(
    akr_dir: &Path,
    expected: impl IntoIterator<Item = (&'a PathBuf, &'a Option<String>)>,
) -> Option<PathBuf> {
    for (relative, before) in expected {
        let full = akr_dir.join(relative);
        let now = match fs::read(&full) {
            Ok(bytes) => Some(String::from_utf8_lossy(&bytes).into_owned()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(_) => return Some(relative.clone()),
        };
        if now.as_deref() != before.as_deref() {
            return Some(relative.clone());
        }
    }
    None
}

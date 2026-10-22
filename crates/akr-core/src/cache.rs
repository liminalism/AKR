//! Build facts memoised on git object ids, in `.akr/cache/facts/`.
//!
//! Some of what a build derives is a pure function of git objects: a record's last
//! definitional change is fixed by the commits that touched its file, and whether a test
//! name occurs in tracked source is fixed by the trees that hold that source. Git objects
//! are immutable and named by their content, so an answer stored under the ids it was
//! computed from is the answer for as long as those ids are asked about. Re-deriving them
//! on every invocation is what made one `akr get` on SYS cost ten seconds: three hundred
//! megabytes of historical ledger text, fetched and parsed again for every command.
//!
//! # What this is not (D-019)
//!
//! Authoritative. Like the index, it is never committed, always rebuildable, and read by
//! nothing outside this crate; deleting it costs one slow build and changes no output.
//! Every entry carries the ids it was computed from and is used only when those match
//! exactly, so a stale entry is a miss, never a wrong answer. Writes are best effort and
//! atomic (a temporary file renamed into place): a read-only checkout or two concurrent
//! builds lose speed, not correctness.

use std::path::{Path, PathBuf};

/// Every entry's first line. A different tool version reads a miss, because the parser,
/// the definitional hash and the source-hit rules are all free to change between
/// releases.
const HEADER: &str = concat!("akr-facts 1 ", env!("CARGO_PKG_VERSION"));

/// The facts directory of one workspace.
#[derive(Debug, Clone)]
pub struct FactCache {
    dir: PathBuf,
}

impl FactCache {
    /// The cache under a workspace's `.akr` directory.
    #[must_use]
    pub fn new(akr_dir: &Path) -> Self {
        Self {
            dir: akr_dir.join("cache").join("facts"),
        }
    }

    /// An entry's body, when one exists and was written by this tool version.
    pub(crate) fn read(&self, name: &str) -> Option<String> {
        let text = std::fs::read_to_string(self.dir.join(name)).ok()?;
        let (header, body) = text.split_once('\n')?;
        (header == HEADER).then(|| body.to_owned())
    }

    /// Replaces an entry. Silent on failure: the next build recomputes.
    pub(crate) fn write(&self, name: &str, body: &str) {
        if std::fs::create_dir_all(&self.dir).is_err() {
            return;
        }
        let temporary = self.dir.join(format!(".{name}.{}.tmp", std::process::id()));
        if std::fs::write(&temporary, format!("{HEADER}\n{body}")).is_err() {
            let _ = std::fs::remove_file(&temporary);
            return;
        }
        if std::fs::rename(&temporary, self.dir.join(name)).is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
    }
}

//! What is sitting in the agent scratch directory, and how old it is.
//!
//! `.agent/scratch/` is where the protocol tells an agent to put working files, and it is
//! the one such place that nothing ever empties. The OS temp directory is cleared by the
//! OS; `target/` is understood to be disposable and is routinely deleted; `.akr/cache/` is
//! rebuilt from source whenever its inputs move. Scratch is inside the repository, is
//! gitignored rather than transient, and survives every session — so it grows without
//! bound until a person notices and deletes it by hand.
//!
//! This module is the measurement, not the policy. It answers "what is there, how big, how
//! old, and what did somebody say to keep", and the command layer decides what to do about
//! it. Nothing here deletes anything.
//!
//! Scratch is deliberately *not* ledger knowledge. A session's temporary files are not
//! project conclusions, and a record per scratch directory would be exactly the noise the
//! ledger exists to keep out (D-036). The one piece of durable state — "this one is still
//! needed, and here is why" — lives in the scratch directory itself, as a plain `KEEP`
//! index a person can read and edit without any tool at all.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The conventional scratch path, relative to the workspace root.
pub const SCRATCH_DIR: &str = ".agent/scratch";

/// The name of the keep index inside the scratch directory.
///
/// Capitalised so it sorts to the top of a listing and reads as a marker rather than as
/// somebody's working file.
pub const KEEP_FILE: &str = "KEEP";

/// One top-level entry in the scratch directory.
///
/// Top-level, because that is the unit a person thinks in: a session left a directory or a
/// file behind, and keeping or dropping it is one decision. Recursing into the tree would
/// produce a list nobody can act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScratchEntry {
    /// The entry's name within the scratch directory.
    pub name: String,
    /// Its total size in bytes, including everything beneath it.
    pub bytes: u64,
    /// Whole days since the newest file in it was written.
    ///
    /// Newest, not oldest: a directory somebody touched yesterday is a day old however
    /// long ago it was created, and pruning it because it *started* three weeks ago would
    /// throw away live work.
    pub age_days: u64,
    /// Why it is being kept, when the `KEEP` index names it.
    pub kept: Option<String>,
    /// Whether it is a directory.
    pub is_dir: bool,
}

impl ScratchEntry {
    /// Whether this entry may be removed at the given age threshold.
    ///
    /// A kept entry is never prunable, whatever its age: the point of the marker is that
    /// somebody has already answered this question.
    #[must_use]
    pub fn is_prunable(&self, older_than_days: u64) -> bool {
        self.kept.is_none() && self.age_days >= older_than_days
    }
}

/// A scan of the scratch directory.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Scratch {
    /// Whether the directory exists at all. A workspace with none is not a problem.
    pub present: bool,
    /// Every top-level entry, sorted by size, largest first.
    pub entries: Vec<ScratchEntry>,
    /// Names in the `KEEP` index that no longer exist.
    ///
    /// Reported rather than silently ignored: a stale keep line is how a directory gets
    /// protected forever by a reason that stopped applying.
    pub stale_keeps: Vec<String>,
}

impl Scratch {
    /// Total size of everything in the directory.
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.entries.iter().map(|entry| entry.bytes).sum()
    }

    /// Entries that may be removed at the given age threshold.
    pub fn prunable(&self, older_than_days: u64) -> impl Iterator<Item = &ScratchEntry> {
        self.entries
            .iter()
            .filter(move |entry| entry.is_prunable(older_than_days))
    }

    /// Total size of what would be pruned at the given age threshold.
    #[must_use]
    pub fn prunable_bytes(&self, older_than_days: u64) -> u64 {
        self.prunable(older_than_days)
            .map(|entry| entry.bytes)
            .sum()
    }

    /// Entries somebody has marked as still needed.
    pub fn kept(&self) -> impl Iterator<Item = &ScratchEntry> {
        self.entries.iter().filter(|entry| entry.kept.is_some())
    }
}

/// The absolute scratch path for a workspace.
#[must_use]
pub fn dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join(SCRATCH_DIR)
}

/// The absolute path of the keep index.
#[must_use]
pub fn keep_path(workspace_root: &Path) -> PathBuf {
    dir(workspace_root).join(KEEP_FILE)
}

/// Top-level scratch entries named by the keep index.
///
/// The result deliberately includes a stale marker. V-025 decides whether a cited path
/// is disposable, not whether the artifact currently exists; a marker still means that
/// `akr scratch prune` must not remove that entry if it reappears.
#[must_use]
pub fn kept_entries(workspace_root: &Path) -> BTreeSet<String> {
    read_keep(&keep_path(workspace_root)).into_keys().collect()
}

/// The entry a cited path names inside the scratch directory, if it names one at all.
///
/// A record cites an artefact by path, and a path is not a reference the ledger resolves —
/// nothing notices when the file behind it is pruned. So the check has to be textual, and
/// it has to be generous about how the same location gets written: `\` for `/` on Windows,
/// a leading `./`, and an absolute path that happens to pass through a scratch directory
/// all name the same disposable place.
///
/// Returns the first path segment beneath [`SCRATCH_DIR`], which is the unit
/// `akr scratch keep` names, so a caller can say what to keep rather than only what is
/// wrong. A cited path that is exactly the scratch root, with nothing beneath it, returns
/// an empty string: it is still under scratch, and there is no entry to name.
#[must_use]
pub fn cited_entry(path: &str) -> Option<String> {
    let normalised = path.replace('\\', "/");
    let trimmed = normalised.trim_end_matches('/');
    let bare = trimmed.trim_start_matches("./");
    if bare == SCRATCH_DIR || trimmed.ends_with(&format!("/{SCRATCH_DIR}")) {
        return Some(String::new());
    }
    let rest = trimmed
        .rsplit_once(&format!("/{SCRATCH_DIR}/"))
        .map(|(_, rest)| rest)
        .or_else(|| bare.strip_prefix(&format!("{SCRATCH_DIR}/")))?;
    Some(rest.split('/').next().unwrap_or(rest).to_owned())
}

/// Scans the scratch directory, measuring each top-level entry against `now`.
///
/// `now` is a parameter rather than a call to the clock so that a test can pin it, the same
/// reason `--today` exists for the rest of the pipeline. A missing directory is not an
/// error: it is the state a clean workspace is in.
#[must_use]
pub fn scan(workspace_root: &Path, now: SystemTime) -> Scratch {
    let root = dir(workspace_root);
    let Ok(read) = std::fs::read_dir(&root) else {
        return Scratch::default();
    };
    let keeps = read_keep(&keep_path(workspace_root));

    let mut entries = Vec::new();
    let mut seen = Vec::new();
    for entry in read.filter_map(Result::ok) {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == KEEP_FILE {
            continue;
        }
        let path = entry.path();
        let is_dir = path.is_dir();
        let (bytes, newest) = measure(&path);
        seen.push(name.clone());
        entries.push(ScratchEntry {
            bytes,
            age_days: days_between(newest, now),
            kept: keeps.get(&name).cloned(),
            name,
            is_dir,
        });
    }
    // Largest first: the listing exists to answer "what is taking the space".
    entries.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.name.cmp(&b.name)));

    let mut stale_keeps: Vec<String> = keeps
        .keys()
        .filter(|name| !seen.contains(name))
        .cloned()
        .collect();
    stale_keeps.sort();

    Scratch {
        present: true,
        entries,
        stale_keeps,
    }
}

/// Total bytes and newest modification time beneath a path.
fn measure(path: &Path) -> (u64, Option<SystemTime>) {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return (0, None);
    };
    let modified = meta.modified().ok();
    if !meta.is_dir() {
        return (meta.len(), modified);
    }
    // A symlinked directory is measured as the link, never followed: a scratch directory
    // that points at somebody's home directory should not be reported as enormous, and
    // must certainly not be offered for pruning.
    if meta.file_type().is_symlink() {
        return (0, modified);
    }
    let mut bytes = 0;
    let mut newest = modified;
    let Ok(read) = std::fs::read_dir(path) else {
        return (bytes, newest);
    };
    for entry in read.filter_map(Result::ok) {
        let (child_bytes, child_newest) = measure(&entry.path());
        bytes += child_bytes;
        newest = match (newest, child_newest) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
    }
    (bytes, newest)
}

/// Whole days between a modification time and now; zero when the time is unknown or ahead.
fn days_between(modified: Option<SystemTime>, now: SystemTime) -> u64 {
    let Some(modified) = modified else {
        return 0;
    };
    now.duration_since(modified)
        .map(|elapsed| elapsed.as_secs() / 86_400)
        .unwrap_or(0)
}

/// Reads the `KEEP` index: one `<name> <reason>` per line, `#` comments, blanks ignored.
///
/// Hand-parsed and deliberately dull. This file is read by people as often as by the tool,
/// and a format that needs a parser to understand is a format nobody edits by hand.
fn read_keep(path: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Ok(text) = std::fs::read_to_string(path) else {
        return out;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (name, reason) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        out.insert(name.to_owned(), reason.trim().to_owned());
    }
    out
}

/// Renders a byte count the way a person reads one.
#[must_use]
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [(u64, &str); 4] = [
        (1024 * 1024 * 1024, "GB"),
        (1024 * 1024, "MB"),
        (1024, "KB"),
        (1, "B"),
    ];
    for (scale, unit) in UNITS {
        if bytes >= scale {
            let whole = bytes / scale;
            let tenth = (bytes % scale) * 10 / scale;
            return if whole < 10 && *unit != *"B" {
                format!("{whole}.{tenth} {unit}")
            } else {
                format!("{whole} {unit}")
            };
        }
    }
    "0 B".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_directory_is_not_a_problem() {
        let scan = scan(Path::new("definitely-not-a-workspace"), SystemTime::now());
        assert!(!scan.present);
        assert_eq!(scan.total_bytes(), 0);
    }

    #[test]
    fn a_kept_entry_is_never_prunable_however_old() {
        let entry = ScratchEntry {
            name: "port-triage".to_owned(),
            bytes: 4096,
            age_days: 900,
            kept: Some("the comb output the next session needs".to_owned()),
            is_dir: true,
        };
        assert!(!entry.is_prunable(0));
        assert!(!entry.is_prunable(14));
    }

    #[test]
    fn an_unkept_entry_is_prunable_once_it_reaches_the_threshold() {
        let entry = ScratchEntry {
            name: "notes.txt".to_owned(),
            bytes: 10,
            age_days: 14,
            kept: None,
            is_dir: false,
        };
        assert!(entry.is_prunable(14));
        assert!(entry.is_prunable(0));
        assert!(!entry.is_prunable(15));
    }

    #[test]
    fn byte_counts_read_the_way_people_write_them() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1024), "1.0 KB");
        assert_eq!(human_bytes(1024 * 1024 * 3 + 512 * 1024), "3.5 MB");
        assert_eq!(human_bytes(1024 * 1024 * 1024 * 30), "30 GB");
    }
}

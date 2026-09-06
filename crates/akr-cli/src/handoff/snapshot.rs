//! What tree the advisor is actually looking at.
//!
//! A packet describes a workspace. If the preparing agent keeps working after writing one
//! — and it usually does — the advisor can end up reading a repository that no longer
//! matches the packet's account of it. Silently handing over a description of one tree
//! while a second model examines another is the failure mode this module exists to make
//! impossible: [`Fingerprint::capture`] records the tree, and [`Fingerprint::compare`]
//! says, at open time, whether it still holds.
//!
//! The fingerprint is deliberately *not* a snapshot of every source byte. `HEAD`, the
//! source-graph hash, and a content digest per dirty path are enough to detect drift, cost
//! one status call plus a read of the files git already says changed, and leave the
//! repository itself as the thing the advisor reads.

use crate::session::Session;
use akr_core::hash::sha256_hex;
use akr_core::json::Value;

/// A dirty path, with a digest of what it held when the packet was made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirtyPath {
    /// Repository-relative path.
    pub path: String,
    /// Hex sha256 of the file's bytes, or [`ABSENT`] when there is no file there.
    pub digest: String,
}

/// The digest recorded for a path git reports as changed but which is not on disk.
///
/// A deletion is a state a fingerprint has to be able to express: without it a deleted
/// file and an unreadable one would be indistinguishable, and a packet made after a
/// deletion would report drift against itself forever.
pub const ABSENT: &str = "absent";

/// The workspace a packet was made against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    /// `HEAD`, full 40-hex, when the workspace is in a repository with commits.
    pub head: Option<String>,
    /// `HEAD`'s subject line, for a human reading the packet.
    pub head_subject: Option<String>,
    /// The source-graph hash of the ledger.
    pub ledger_revision: String,
    /// Every path git reports as changed against `HEAD`, sorted, with digests.
    pub dirty: Vec<DirtyPath>,
}

/// Whether the workspace still matches the packet, and what moved if it does not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drift {
    /// `true` when `HEAD`, the ledger and every dirty path are unchanged.
    pub exact: bool,
    /// `HEAD` moved: the packet's commit and the current one.
    pub head_moved: Option<(String, String)>,
    /// The ledger's source-graph hash changed.
    pub ledger_moved: bool,
    /// Paths whose content differs from the packet, in either direction.
    pub changed: Vec<String>,
}

impl Drift {
    /// The one-word status `handoff open` and `handoff verify` report.
    #[must_use]
    pub const fn status(&self) -> &'static str {
        if self.exact { "exact" } else { "drifted" }
    }

    /// The JSON form, shared by every command that reports drift.
    #[must_use]
    pub fn to_json(&self) -> Value {
        Value::object(vec![
            ("workspace_status", Value::string(self.status())),
            (
                "head_moved",
                self.head_moved.as_ref().map_or(Value::Null, |(was, now)| {
                    Value::object(vec![
                        ("packet", Value::string(was.clone())),
                        ("current", Value::string(now.clone())),
                    ])
                }),
            ),
            ("ledger_moved", Value::bool(self.ledger_moved)),
            (
                "changed_since_packet",
                Value::array(self.changed.iter().cloned().map(Value::string).collect()),
            ),
        ])
    }
}

impl Fingerprint {
    /// Records the workspace as it stands.
    ///
    /// Never fails: a workspace outside a repository, or one whose `HEAD` is unborn, has
    /// no commit and no dirty set, and a packet made there is honest about that rather
    /// than refused. What it cannot do is *pretend* — an unreadable file is recorded as
    /// [`ABSENT`], not skipped.
    #[must_use]
    pub fn capture(session: &Session) -> Self {
        let repository = session.repository.as_ref();
        let head = repository.and_then(|repository| repository.head().ok());
        let head_subject = match (repository, head.as_ref()) {
            (Some(repository), Some(_)) => repository
                .session_head()
                .ok()
                .map(|(latest, _)| latest.subject),
            _ => None,
        };
        let dirty = repository
            .and_then(|repository| repository.working_tree_changes().ok())
            .unwrap_or_default()
            .into_iter()
            // Writing a packet must not make that packet drift against itself. The store
            // is gitignored in an initialised workspace, so this is belt-and-braces for a
            // repository whose `.gitignore` predates D-040 -- but without it the very
            // first `handoff open` after `handoff create` reports drift, names the packet
            // file as the thing that moved, and teaches the advisor to ignore the signal.
            .filter(|path| !path.starts_with(&format!("{}/", super::packet::DIRECTORY)))
            .map(|path| {
                let digest = std::fs::read(session.root.join(&path))
                    .map_or_else(|_| ABSENT.to_owned(), |bytes| sha256_hex(&bytes));
                DirtyPath { path, digest }
            })
            .collect();
        Self {
            head: head.map(|commit| commit.as_str().to_owned()),
            head_subject,
            ledger_revision: session.source_graph(),
            dirty,
        }
    }

    /// Compares the packet's workspace with `current`.
    ///
    /// A path counts as changed when it is dirty in one fingerprint and not the other, or
    /// dirty in both with different content. A path that was clean in both is not
    /// examined — if its content had changed, git would be calling it dirty.
    #[must_use]
    pub fn compare(&self, current: &Self) -> Drift {
        let head_moved = match (&self.head, &current.head) {
            (Some(was), Some(now)) if was != now => Some((was.clone(), now.clone())),
            (Some(was), None) => Some((was.clone(), String::new())),
            (None, Some(now)) => Some((String::new(), now.clone())),
            _ => None,
        };

        let mut changed: Vec<String> = Vec::new();
        let mut push = |path: &str| {
            if !changed.iter().any(|seen| seen == path) {
                changed.push(path.to_owned());
            }
        };
        for entry in &self.dirty {
            match current.dirty.iter().find(|other| other.path == entry.path) {
                Some(other) if other.digest == entry.digest => {}
                // Gone from the dirty set means the edit was committed or reverted, and
                // either way the file no longer holds what the packet described.
                _ => push(&entry.path),
            }
        }
        for entry in &current.dirty {
            if !self.dirty.iter().any(|mine| mine.path == entry.path) {
                push(&entry.path);
            }
        }
        changed.sort();

        let ledger_moved = self.ledger_revision != current.ledger_revision;
        Drift {
            exact: head_moved.is_none() && !ledger_moved && changed.is_empty(),
            head_moved,
            ledger_moved,
            changed,
        }
    }

    /// The stored JSON form.
    #[must_use]
    pub fn to_json(&self) -> Value {
        Value::object(vec![
            (
                "head",
                self.head
                    .as_ref()
                    .map_or(Value::Null, |head| Value::string(head.clone())),
            ),
            (
                "head_subject",
                self.head_subject
                    .as_ref()
                    .map_or(Value::Null, |subject| Value::string(subject.clone())),
            ),
            (
                "ledger_revision",
                Value::string(self.ledger_revision.clone()),
            ),
            (
                "dirty",
                Value::array(
                    self.dirty
                        .iter()
                        .map(|entry| {
                            Value::object(vec![
                                ("path", Value::string(entry.path.clone())),
                                ("digest", Value::string(entry.digest.clone())),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }

    /// Reads a fingerprint back from a stored packet.
    #[must_use]
    pub fn from_json(value: &Value) -> Self {
        Self {
            head: value
                .get("head")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            head_subject: value
                .get("head_subject")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            ledger_revision: value
                .get("ledger_revision")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            dirty: value
                .get("dirty")
                .and_then(Value::as_array)
                .unwrap_or_default()
                .iter()
                .map(|entry| DirtyPath {
                    path: entry
                        .get("path")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    digest: entry
                        .get("digest")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                })
                .collect(),
        }
    }

    /// The one-line human rendering used at the top of a packet.
    #[must_use]
    pub fn line(&self) -> String {
        let commit = self.head.as_ref().map_or_else(
            || "(no git commit)".to_owned(),
            |head| {
                let short = &head[..head.len().min(8)];
                self.head_subject
                    .as_ref()
                    .map_or_else(|| short.to_owned(), |subject| format!("{short} {subject}"))
            },
        );
        format!("{commit} — {} dirty paths", self.dirty.len())
    }
}

//! The assignment: a mode, a task, a scope, and what it inherits.
//!
//! # The mode is the epistemic contract
//!
//! Four names, one axis: how much of the parent's *judgement* the child may see.
//!
//! | Mode | Sees the parent's notes | For |
//! | --- | --- | --- |
//! | `worker` | yes, on open | continuing a job the parent began |
//! | `scout` | on request, recorded | exploring an assigned scope independently |
//! | `reviewer` | on request, recorded | checking what the parent produced |
//! | `advisor` | on request, recorded | an independent judgement, then a comparison |
//!
//! The three withholding modes are mechanically alike on purpose. Their difference is the
//! contract they state and what they are asked to return, and inventing three disclosure
//! policies to make them look different in code would have been decoration. What matters
//! is that the *default* is right for each: a worker that had to ask for the parent's
//! coverage would duplicate work, and a scout handed the parent's hypotheses would stop
//! being a scout.
//!
//! # Inheritance is by reference
//!
//! A packet names the capsules and packets it inherits and copies none of them. Rendering
//! resolves the chain, so a session capsule corrected after five packets were cut corrects
//! all five. The alternative — copying the capsule into each packet — produces five
//! snapshots that drift apart silently, which is the failure the capsules exist to remove.
//!
//! # The task is narrowed, never replaced
//!
//! A parent narrowing a job for a child is what delegation *is*. Substituting its own
//! reading of the user's request for the request is not, and from inside the child the two
//! are indistinguishable. So a packet renders the session's verbatim request above the
//! assignment: the narrowing stays visible to the agent it was done to, which is the only
//! place it can be checked.

use super::snapshot::Fingerprint;
use super::{PreparedCommand, commands_json, list, optional, strings, text};
use akr_core::hash::Sha256;
use akr_core::json::{Value, parse};
use akr_core::model::Date;
use std::path::{Path, PathBuf};

/// The stored format version. Bumped when a field's meaning changes, not when one is added.
pub const FORMAT: &str = "0.2";

/// Where capsules, packets and results live, relative to the workspace root.
pub const DIRECTORY: &str = ".agent/handoffs";

/// The file naming the open session, so a parent need not repeat the id.
pub const CURRENT: &str = "CURRENT";

/// How much of the parent's judgement a child may see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Continues a job the parent began; sees its findings and coverage.
    Worker,
    /// Explores an assigned scope independently; the parent's conclusions are withheld.
    Scout,
    /// Checks what the parent produced; its reasoning is withheld.
    Reviewer,
    /// Gives an independent judgement, then compares it against the parent's.
    Advisor,
}

impl Mode {
    /// Every mode, in the order help and documentation list them.
    pub const ALL: &'static [Self] = &[Self::Worker, Self::Scout, Self::Reviewer, Self::Advisor];

    /// The name used on the command line, in JSON and in the packet id.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Worker => "worker",
            Self::Scout => "scout",
            Self::Reviewer => "reviewer",
            Self::Advisor => "advisor",
        }
    }

    /// Parses a mode name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|mode| mode.as_str() == name)
    }

    /// The packet-id prefix, so an id says what it is in a transcript.
    #[must_use]
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::Worker => "wk",
            Self::Scout => "sc",
            Self::Reviewer => "rv",
            Self::Advisor => "ad",
        }
    }

    /// Whether an ordinary open discloses the parent's notes.
    #[must_use]
    pub const fn discloses_notes(self) -> bool {
        matches!(self, Self::Worker)
    }

    /// What the packet tells the child it is for.
    #[must_use]
    pub const fn contract(self) -> &'static str {
        match self {
            Self::Worker => {
                "You are continuing work another agent began. Its findings and its coverage \
                 are included so you do not pay for them twice. They are not authority: \
                 where they and the code disagree, the code decides."
            }
            Self::Scout => {
                "You are exploring independently inside an assigned scope. The parent's \
                 conclusions are withheld on purpose - you were sent to look, not to \
                 confirm. What it established as fact is included so you need not \
                 re-establish it."
            }
            Self::Reviewer => {
                "You are checking work another agent produced. What it did is included; why \
                 it thought so is withheld, so your review is not a re-reading of its \
                 reasoning."
            }
            Self::Advisor => {
                "You are being asked for an independent judgement. Review first. Then \
                 `handoff reveal` shows what the preparing agent thought, and the question \
                 worth answering is what either side missed."
            }
        }
    }

    /// What a result is expected to carry, when the parent does not say.
    #[must_use]
    pub const fn default_return(self) -> &'static [&'static str] {
        match self {
            Self::Worker => &[
                "what you changed, and where",
                "evidence that it works",
                "coverage: what you read, searched and ran",
                "uncertainties",
            ],
            Self::Scout => &[
                "findings, ranked by likely impact",
                "exact source locations",
                "a suggested experiment for each",
                "coverage: what you read, searched and ran",
                "uncertainties",
                "no implementation",
            ],
            Self::Reviewer => &[
                "defects, with exact locations and severity",
                "what you verified, and how",
                "coverage: what you read, searched and ran",
                "no implementation",
            ],
            Self::Advisor => &[
                "your independent findings, before the reveal",
                "after the reveal: what each side missed",
                "a recommended direction",
                "uncertainties",
            ],
        }
    }
}

/// Layer B: what the parent thinks, kept apart from what it established.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkerNotes {
    /// Where the parent suspects the answer lies.
    pub hypotheses: Vec<String>,
    /// What it read.
    pub examined: Vec<String>,
    /// What it did not read — the half of coverage that is usually lost.
    pub not_examined: Vec<String>,
    /// Approaches it would propose.
    pub approaches: Vec<String>,
    /// Searches it ran, so a query that found nothing is not run twice.
    pub searches: Vec<String>,
}

impl WorkerNotes {
    /// Whether anything was recorded at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.hypotheses.is_empty()
            && self.examined.is_empty()
            && self.not_examined.is_empty()
            && self.approaches.is_empty()
            && self.searches.is_empty()
    }

    /// The stored form.
    #[must_use]
    pub fn to_json(&self) -> Value {
        Value::object(vec![
            ("hypotheses", strings(&self.hypotheses)),
            ("examined", strings(&self.examined)),
            ("not_examined", strings(&self.not_examined)),
            ("approaches", strings(&self.approaches)),
            ("searches", strings(&self.searches)),
        ])
    }

    /// Reads them back.
    #[must_use]
    pub fn from_json(value: Option<&Value>) -> Self {
        Self {
            hypotheses: list(value, "hypotheses"),
            examined: list(value, "examined"),
            not_examined: list(value, "not_examined"),
            approaches: list(value, "approaches"),
            searches: list(value, "searches"),
        }
    }
}

/// The whole project: what an agent gets when nobody narrowed the scope.
///
/// This default matters more than it looks. A scope drawn from what the parent happened to
/// examine would quietly hand its blind spot to the child as a boundary, which is the one
/// thing an independent pass is there to escape. A parent that means to narrow says so.
pub const WHOLE_PROJECT: &str = "**";

/// One assignment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    /// The mode's prefix, then twelve hex characters.
    pub id: String,
    /// What the child may see of the parent's judgement.
    pub mode: Mode,
    /// The date it was cut.
    pub created_at: String,
    /// Who cut it: a model or harness name, for a later comparison of coverage.
    pub created_by: Option<String>,
    /// Capsule and packet ids this inherits, resolved at read time and never copied.
    pub inherits: Vec<String>,
    /// The tree this packet was cut against.
    ///
    /// Distinct from the session's fingerprint on purpose. The session records where the
    /// delegation *began*; the parent then builds, runs, edits, and cuts a packet against
    /// the tree as it stands at that moment. Drift measured from the session start would
    /// tell a child that the packet no longer matches a tree it was never handed, and a
    /// signal that fires for the wrong reason is one the child learns to ignore.
    /// `None` only for a packet stored before this field existed; such a packet falls back
    /// to the session's fingerprint when read.
    pub workspace: Option<Fingerprint>,
    /// What the child is, in two or three words: `performance-scout`, `api-reviewer`.
    pub role: Option<String>,
    /// The assignment. May be narrower than the session request, and renders beneath it.
    pub task: String,
    /// Where the child may look.
    pub scope: Vec<String>,
    /// Facts already established, so the child does not re-establish them.
    pub known: Vec<String>,
    /// Work the child must not repeat: expensive commands, discovery already done.
    pub skip: Vec<String>,
    /// What the child should return. Defaults to the mode's list.
    pub expected_return: Vec<String>,
    /// Commands specific to this assignment.
    pub commands: Vec<PreparedCommand>,
    /// Layer B.
    pub worker_notes: WorkerNotes,
    /// When the notes were revealed, if they have been.
    pub revealed_at: Option<String>,
}

impl Packet {
    /// A new packet, addressed by a digest of what makes it distinct.
    ///
    /// `ordinal` disambiguates two packets cut for the same task against the same tree on
    /// the same day; callers pass the number of packets already stored.
    #[must_use]
    pub fn new(
        mode: Mode,
        task: &str,
        created_at: Date,
        inherits: &[String],
        ordinal: usize,
    ) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(mode.as_str().as_bytes());
        hasher.update(task.as_bytes());
        for parent in inherits {
            hasher.update(parent.as_bytes());
            hasher.update(b"\x1e");
        }
        hasher.update(created_at.to_string().as_bytes());
        hasher.update(ordinal.to_string().as_bytes());
        Self {
            id: format!("{}-{}", mode.prefix(), &hasher.finish().to_hex()[..12]),
            mode,
            created_at: created_at.to_string(),
            created_by: None,
            inherits: inherits.to_vec(),
            workspace: None,
            role: None,
            task: task.to_owned(),
            scope: vec![WHOLE_PROJECT.to_owned()],
            known: Vec::new(),
            skip: Vec::new(),
            expected_return: mode
                .default_return()
                .iter()
                .map(|item| (*item).to_owned())
                .collect(),
            commands: Vec::new(),
            worker_notes: WorkerNotes::default(),
            revealed_at: None,
        }
    }

    /// Whether the notes have been revealed.
    #[must_use]
    pub const fn revealed(&self) -> bool {
        self.revealed_at.is_some()
    }

    /// Whether this read shows Layer B: the mode discloses it, or somebody revealed it.
    #[must_use]
    pub const fn shows_notes(&self) -> bool {
        self.mode.discloses_notes() || self.revealed()
    }

    /// The session capsule this inherits, if any.
    #[must_use]
    pub fn session(&self) -> Option<&str> {
        self.inherits
            .iter()
            .find(|id| id.starts_with("sx-"))
            .map(String::as_str)
    }

    /// The stored form. Field order is fixed, so a packet round-trips byte-identically.
    #[must_use]
    pub fn to_json(&self) -> Value {
        Value::object(vec![
            ("format", Value::string(FORMAT)),
            ("packet", Value::string(self.id.clone())),
            ("mode", Value::string(self.mode.as_str())),
            ("created_at", Value::string(self.created_at.clone())),
            (
                "created_by",
                self.created_by
                    .as_ref()
                    .map_or(Value::Null, |by| Value::string(by.clone())),
            ),
            ("inherits", strings(&self.inherits)),
            (
                "workspace",
                self.workspace
                    .as_ref()
                    .map_or(Value::Null, Fingerprint::to_json),
            ),
            (
                "role",
                self.role
                    .as_ref()
                    .map_or(Value::Null, |role| Value::string(role.clone())),
            ),
            ("task", Value::string(self.task.clone())),
            ("scope", strings(&self.scope)),
            ("known", strings(&self.known)),
            ("skip", strings(&self.skip)),
            ("expected_return", strings(&self.expected_return)),
            ("commands", commands_json(&self.commands)),
            ("worker_notes", self.worker_notes.to_json()),
            (
                "revealed_at",
                self.revealed_at
                    .as_ref()
                    .map_or(Value::Null, |at| Value::string(at.clone())),
            ),
        ])
    }

    /// Reads a packet back.
    ///
    /// # Errors
    /// A message naming what is missing, when the document is not a packet.
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let id = value
            .get("packet")
            .and_then(Value::as_str)
            .ok_or("no `packet` field")?
            .to_owned();
        let mode = value
            .get("mode")
            .and_then(Value::as_str)
            .and_then(Mode::from_name)
            .ok_or("no recognised `mode`")?;
        let scope = list(Some(value), "scope");
        Ok(Self {
            id,
            mode,
            created_at: text(value, "created_at"),
            created_by: optional(value, "created_by"),
            inherits: list(Some(value), "inherits"),
            workspace: value
                .get("workspace")
                .filter(|workspace| !workspace.is_null())
                .map(Fingerprint::from_json),
            role: optional(value, "role"),
            task: value
                .get("task")
                .and_then(Value::as_str)
                .ok_or("no `task` field")?
                .to_owned(),
            scope: if scope.is_empty() {
                vec![WHOLE_PROJECT.to_owned()]
            } else {
                scope
            },
            known: list(Some(value), "known"),
            skip: list(Some(value), "skip"),
            expected_return: list(Some(value), "expected_return"),
            commands: PreparedCommand::list_from_json(Some(value), "commands"),
            worker_notes: WorkerNotes::from_json(value.get("worker_notes")),
            revealed_at: optional(value, "revealed_at"),
        })
    }
}

// ---------------------------------------------------------------------------------------
// the store
// ---------------------------------------------------------------------------------------

/// Where a workspace's handoff state lives.
#[must_use]
pub fn directory(root: &Path) -> PathBuf {
    root.join(".agent").join("handoffs")
}

/// The file one addressable object is stored in.
#[must_use]
pub fn path(root: &Path, id: &str) -> PathBuf {
    directory(root).join(format!("{id}.json"))
}

/// Whether an id names a session capsule.
#[must_use]
pub fn is_session_id(id: &str) -> bool {
    id.starts_with("sx-")
}

/// Whether an id names a packet rather than a capsule or a result.
#[must_use]
pub fn is_packet_id(id: &str) -> bool {
    Mode::ALL
        .iter()
        .any(|mode| id.starts_with(mode.prefix()) && id[mode.prefix().len()..].starts_with('-'))
}

/// Every stored id the filter keeps, sorted.
///
/// Sorted rather than by modification time on purpose: two runs of `akr handoff list`
/// against the same directory print the same thing, on every filesystem.
#[must_use]
pub fn ids_with(root: &Path, keep: impl Fn(&str) -> bool) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(directory(root)) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".json")
                .filter(|stem| keep(stem))
                .map(ToOwned::to_owned)
        })
        .collect();
    ids.sort();
    ids
}

/// Every stored packet id.
#[must_use]
pub fn ids(root: &Path) -> Vec<String> {
    ids_with(root, is_packet_id)
}

/// Reads any stored object as JSON.
///
/// # Errors
/// A message when the file is missing, unreadable, or not JSON.
pub fn read_json(root: &Path, id: &str) -> Result<Value, String> {
    let path = path(root, id);
    let text =
        std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    parse(&text).map_err(|error| format!("{}: {error}", path.display()))
}

/// Writes any stored object, creating the directory if it is not there.
///
/// # Errors
/// A message when the directory or the file cannot be written.
pub fn write_json(root: &Path, id: &str, value: &Value) -> Result<PathBuf, String> {
    let directory = directory(root);
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("{}: {error}", directory.display()))?;
    let path = path(root, id);
    let mut text = value.to_pretty();
    if !text.ends_with('\n') {
        text.push('\n');
    }
    std::fs::write(&path, text).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(path)
}

/// Reads one packet.
///
/// # Errors
/// A message when the file is missing, unreadable, or not a packet.
pub fn load(root: &Path, id: &str) -> Result<Packet, String> {
    let value = read_json(root, id)?;
    Packet::from_json(&value).map_err(|reason| format!("{id}: {reason}"))
}

/// Writes one packet.
///
/// # Errors
/// A message when it cannot be written.
pub fn save(root: &Path, packet: &Packet) -> Result<PathBuf, String> {
    write_json(root, &packet.id, &packet.to_json())
}

/// Deletes one stored object. `false` when there was none.
///
/// # Errors
/// A message when the file exists and cannot be removed.
pub fn discard(root: &Path, id: &str) -> Result<bool, String> {
    let path = path(root, id);
    if !path.is_file() {
        return Ok(false);
    }
    std::fs::remove_file(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(true)
}

// ---------------------------------------------------------------------------------------
// the open session
// ---------------------------------------------------------------------------------------

/// The id of the open session, if one is open.
#[must_use]
pub fn current_session(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(directory(root).join(CURRENT)).ok()?;
    let id = text.trim().to_owned();
    (!id.is_empty() && path(root, &id).is_file()).then_some(id)
}

/// Records which session is open.
///
/// # Errors
/// A message when the pointer cannot be written.
pub fn set_current_session(root: &Path, id: &str) -> Result<(), String> {
    let directory = directory(root);
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("{}: {error}", directory.display()))?;
    let path = directory.join(CURRENT);
    std::fs::write(&path, format!("{id}\n")).map_err(|error| format!("{}: {error}", path.display()))
}

/// Forgets which session is open. Never an error when none was.
pub fn clear_current_session(root: &Path) {
    let _ = std::fs::remove_file(directory(root).join(CURRENT));
}

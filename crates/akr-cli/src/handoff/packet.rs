//! The advisor packet: an addressable object, not a message.
//!
//! `docs/17-advisor-packets.md`, D-040.
//!
//! # Why the two layers are in the type
//!
//! A packet carries two categories of thing, and confusing them is the whole failure this
//! design is built to prevent.
//!
//! **Layer A is administrative fact.** The verbatim task, `HEAD`, the ledger revision, the
//! session head, the governing records, the search envelope, build and test commands,
//! baselines already measured, artefacts already produced. The preparing agent is allowed
//! to compress all of it, because compressing it loses nothing: an advisor that reads a
//! shorter account of the build command is not a worse reviewer.
//!
//! **Layer B is worker interpretation.** Hypotheses, files examined, files not examined,
//! proposed approaches. The preparing agent may *record* these and may not *substitute*
//! them for the problem, because an advisor hired for what the first agent did not notice
//! cannot be handed the first agent's noticing as the definition of the task.
//!
//! [`AdvisorPacket`] keeps them in different fields, [`super::advisor::open`] renders only
//! the first, and [`super::advisor::reveal`] is a separate, recorded act. The point of
//! recording it is comparison: once you know an advisor read blind and then saw the notes,
//! "what did either side miss?" is a question with an answer.
//!
//! # Why the task is verbatim
//!
//! `task` is the user's request as the user wrote it. A preparing agent that has decided
//! the work is about chroma denoising must not be able to replace "optimise this project
//! for CPU and memory" with its own conclusion, because that conclusion is exactly the
//! judgement the advisor was brought in to make independently. Interpretation goes in
//! `question` or in the worker notes; the task field is not the place for it.
//!
//! # Why this is not a record
//!
//! Packets are disposable coordination state. They describe a workspace at a moment,
//! they are worthless a week later, and hundreds of them would drown the ledger the way
//! D-032 says a `commit` kind would. So they live in `.agent/handoffs/`, are gitignored,
//! and are invisible to search, context and the compiler. What survives a packet is
//! whatever the advisor's review made durable — a record, with evidence.

use akr_core::hash::Sha256;
use akr_core::json::{Value, parse};
use akr_core::model::Date;
use std::path::{Path, PathBuf};

/// The stored format version. Bumped when a field's meaning changes, not when one is added.
pub const FORMAT: &str = "0.1";

/// Where packets live, relative to the workspace root.
pub const DIRECTORY: &str = ".agent/handoffs";

/// A command the advisor can run, and what it produced when the packet was prepared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedCommand {
    /// The exact command line.
    pub command: String,
    /// What running it produced: `pass`, `fail`, a measurement, or nothing recorded.
    pub result: Option<String>,
}

/// Layer B: what the preparing agent thinks, kept apart from what it established.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkerNotes {
    /// Where the preparing agent suspects the answer lies.
    pub hypotheses: Vec<String>,
    /// What it read.
    pub examined: Vec<String>,
    /// What it did not read — the half of coverage that is usually lost.
    pub not_examined: Vec<String>,
    /// Approaches it would propose.
    pub approaches: Vec<String>,
    /// Searches it ran, so the advisor need not repeat a query that found nothing.
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
}

/// An advisor packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvisorPacket {
    /// The addressable id, `ap-` plus twelve hex characters.
    pub id: String,
    /// The date it was prepared.
    pub created_at: String,
    /// Who prepared it: a model or harness name, for a later comparison of coverage.
    pub created_by: Option<String>,
    /// **The user's request, verbatim.** Never a restatement.
    pub task: String,
    /// What the advisor is specifically being asked, when that is narrower than the task.
    pub question: Option<String>,
    /// The workspace this packet describes.
    pub workspace: super::snapshot::Fingerprint,
    /// The declared namespaces of the ledger.
    pub namespaces: Vec<String>,
    /// The rendered session head, embedded so the advisor spends nothing rediscovering it.
    pub session_head: String,
    /// The governing planning key, if the work has one.
    pub goal: Option<String>,
    /// Records that govern or constrain the work.
    pub governing: Vec<String>,
    /// **The advisor's independent search scope.** Defaults to the whole project.
    pub search_envelope: Vec<String>,
    /// Commands the advisor can run, with what they produced.
    pub commands: Vec<PreparedCommand>,
    /// Measurements already established.
    pub baselines: Vec<String>,
    /// Constraints the answer has to respect.
    pub constraints: Vec<String>,
    /// Evidence records already written.
    pub evidence: Vec<String>,
    /// Artefacts already produced, by repository path.
    pub artifacts: Vec<String>,
    /// Layer B. Withheld by [`super::advisor::open`] until revealed.
    pub worker_notes: WorkerNotes,
    /// When the notes were revealed, if they have been.
    pub revealed_at: Option<String>,
}

/// The whole project: what an advisor gets when nobody narrowed the scope.
///
/// This default matters more than it looks. A packet whose envelope defaults to the files
/// the preparing agent happened to touch would quietly hand its own blind spot to the
/// advisor as a boundary, which is the one thing the advisor is there to escape.
pub const WHOLE_PROJECT: &str = "**";

impl AdvisorPacket {
    /// A new packet for `task`, addressed by a digest of what makes it distinct.
    ///
    /// `ordinal` disambiguates two packets prepared for the same task against the same
    /// tree on the same day; callers pass the number of packets already stored.
    #[must_use]
    pub fn new(
        task: &str,
        created_at: Date,
        workspace: super::snapshot::Fingerprint,
        ordinal: usize,
    ) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(task.as_bytes());
        hasher.update(workspace.ledger_revision.as_bytes());
        hasher.update(workspace.head.as_deref().unwrap_or("").as_bytes());
        hasher.update(created_at.to_string().as_bytes());
        hasher.update(ordinal.to_string().as_bytes());
        Self {
            id: format!("ap-{}", &hasher.finish().to_hex()[..12]),
            created_at: created_at.to_string(),
            created_by: None,
            task: task.to_owned(),
            question: None,
            workspace,
            namespaces: Vec::new(),
            session_head: String::new(),
            goal: None,
            governing: Vec::new(),
            search_envelope: vec![WHOLE_PROJECT.to_owned()],
            commands: Vec::new(),
            baselines: Vec::new(),
            constraints: Vec::new(),
            evidence: Vec::new(),
            artifacts: Vec::new(),
            worker_notes: WorkerNotes::default(),
            revealed_at: None,
        }
    }

    /// Whether the notes have been revealed.
    #[must_use]
    pub const fn revealed(&self) -> bool {
        self.revealed_at.is_some()
    }

    /// The stored JSON form. Field order is fixed, so a packet round-trips byte-identically.
    #[must_use]
    pub fn to_json(&self) -> Value {
        Value::object(vec![
            ("format", Value::string(FORMAT)),
            ("packet", Value::string(self.id.clone())),
            ("kind", Value::string("advisor")),
            ("created_at", Value::string(self.created_at.clone())),
            (
                "created_by",
                self.created_by
                    .as_ref()
                    .map_or(Value::Null, |by| Value::string(by.clone())),
            ),
            ("task", Value::string(self.task.clone())),
            (
                "question",
                self.question
                    .as_ref()
                    .map_or(Value::Null, |q| Value::string(q.clone())),
            ),
            ("workspace", self.workspace.to_json()),
            (
                "project",
                Value::object(vec![
                    ("namespaces", strings(&self.namespaces)),
                    ("session_head", Value::string(self.session_head.clone())),
                ]),
            ),
            (
                "governing_context",
                Value::object(vec![
                    (
                        "goal",
                        self.goal
                            .as_ref()
                            .map_or(Value::Null, |goal| Value::string(goal.clone())),
                    ),
                    ("records", strings(&self.governing)),
                ]),
            ),
            ("search_envelope", strings(&self.search_envelope)),
            (
                "execution_state",
                Value::object(vec![
                    (
                        "commands",
                        Value::array(
                            self.commands
                                .iter()
                                .map(|prepared| {
                                    Value::object(vec![
                                        ("command", Value::string(prepared.command.clone())),
                                        (
                                            "result",
                                            prepared
                                                .result
                                                .as_ref()
                                                .map_or(Value::Null, |r| Value::string(r.clone())),
                                        ),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                    ("baselines", strings(&self.baselines)),
                    ("constraints", strings(&self.constraints)),
                    ("evidence", strings(&self.evidence)),
                    ("artifacts", strings(&self.artifacts)),
                ]),
            ),
            (
                "worker_notes",
                Value::object(vec![
                    ("hypotheses", strings(&self.worker_notes.hypotheses)),
                    ("examined", strings(&self.worker_notes.examined)),
                    ("not_examined", strings(&self.worker_notes.not_examined)),
                    ("approaches", strings(&self.worker_notes.approaches)),
                    ("searches", strings(&self.worker_notes.searches)),
                ]),
            ),
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
        let project = value.get("project");
        let governing = value.get("governing_context");
        let execution = value.get("execution_state");
        let notes = value.get("worker_notes");
        Ok(Self {
            id,
            created_at: text(value, "created_at"),
            created_by: optional(value, "created_by"),
            task: value
                .get("task")
                .and_then(Value::as_str)
                .ok_or("no `task` field")?
                .to_owned(),
            question: optional(value, "question"),
            workspace: super::snapshot::Fingerprint::from_json(
                value.get("workspace").unwrap_or(&Value::Null),
            ),
            namespaces: list(project, "namespaces"),
            session_head: project.map(|p| text(p, "session_head")).unwrap_or_default(),
            goal: governing.and_then(|g| optional(g, "goal")),
            governing: list(governing, "records"),
            search_envelope: value
                .get("search_envelope")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(ToOwned::to_owned)
                        .collect()
                })
                .unwrap_or_else(|| vec![WHOLE_PROJECT.to_owned()]),
            commands: execution
                .and_then(|e| e.get("commands"))
                .and_then(Value::as_array)
                .unwrap_or_default()
                .iter()
                .map(|entry| PreparedCommand {
                    command: text(entry, "command"),
                    result: optional(entry, "result"),
                })
                .collect(),
            baselines: list(execution, "baselines"),
            constraints: list(execution, "constraints"),
            evidence: list(execution, "evidence"),
            artifacts: list(execution, "artifacts"),
            worker_notes: WorkerNotes {
                hypotheses: list(notes, "hypotheses"),
                examined: list(notes, "examined"),
                not_examined: list(notes, "not_examined"),
                approaches: list(notes, "approaches"),
                searches: list(notes, "searches"),
            },
            revealed_at: optional(value, "revealed_at"),
        })
    }
}

fn strings(items: &[String]) -> Value {
    Value::array(items.iter().cloned().map(Value::string).collect())
}

fn text(value: &Value, field: &str) -> String {
    value
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn optional(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(ToOwned::to_owned)
}

fn list(value: Option<&Value>, field: &str) -> Vec<String> {
    value
        .and_then(|value| value.get(field))
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect()
}

// ---------------------------------------------------------------------------------------
// the store
// ---------------------------------------------------------------------------------------

/// Where a workspace's packets live.
#[must_use]
pub fn directory(root: &Path) -> PathBuf {
    root.join(".agent").join("handoffs")
}

/// The file one packet is stored in.
#[must_use]
pub fn path(root: &Path, id: &str) -> PathBuf {
    directory(root).join(format!("{id}.json"))
}

/// Every stored packet id, newest-looking last, in the sorted order the filenames give.
///
/// Sorted rather than by modification time on purpose: two runs of `akr handoff list`
/// against the same directory print the same thing, on every filesystem.
#[must_use]
pub fn ids(root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(directory(root)) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".json")
                .filter(|stem| stem.starts_with("ap-"))
                .map(ToOwned::to_owned)
        })
        .collect();
    ids.sort();
    ids
}

/// Reads one packet.
///
/// # Errors
/// A message when the file is missing, unreadable, or not a packet.
pub fn load(root: &Path, id: &str) -> Result<AdvisorPacket, String> {
    let path = path(root, id);
    let text =
        std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let value = parse(&text).map_err(|error| format!("{}: {error}", path.display()))?;
    AdvisorPacket::from_json(&value).map_err(|reason| format!("{}: {reason}", path.display()))
}

/// Writes one packet, creating the directory if it is not there.
///
/// # Errors
/// A message when the directory or the file cannot be written.
pub fn save(root: &Path, packet: &AdvisorPacket) -> Result<PathBuf, String> {
    let directory = directory(root);
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("{}: {error}", directory.display()))?;
    let path = path(root, &packet.id);
    let mut text = packet.to_json().to_pretty();
    if !text.ends_with('\n') {
        text.push('\n');
    }
    std::fs::write(&path, text).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(path)
}

/// Deletes one packet. `false` when there was none.
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

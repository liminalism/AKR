//! `akr handoff *` — preparing, opening, expanding, revealing and verifying a packet.
//!
//! The commands are deliberately small. Everything they do is assemble facts the rest of
//! the tool already computes — the session head, the workspace fingerprint, the ledger's
//! namespaces — and put them somewhere a second model can address by name. No model
//! participates in preparing a packet, exactly as no model participates in any other stage
//! of this tool.
//!
//! The one rule enforced here rather than merely documented is the blind read: [`open`]
//! renders Layer A and reports `worker_notes_available`, and nothing short of [`reveal`]
//! produces Layer B. `--reveal` on `open` exists, and takes the same recorded path — an
//! advisor that wants the notes immediately may have them, but the packet says that it did.

use super::packet::{self, AdvisorPacket, PreparedCommand, WorkerNotes};
use super::snapshot::Fingerprint;
use crate::commands::Output;
use crate::session::{EnvError, Session};
use akr_core::json::Value;

/// Everything `akr handoff create` and `knowledge.handoff_create` accept.
///
/// One struct for both surfaces, so the MCP tool cannot grow a field the command line
/// has no way to pass (`docs/08-mcp.md` §1).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CreateRequest {
    /// The user's request, verbatim.
    pub task: String,
    /// What the advisor is asked, when that is narrower than the task.
    pub question: Option<String>,
    /// Who is preparing the packet.
    pub by: Option<String>,
    /// The governing planning key.
    pub goal: Option<String>,
    /// Records that govern the work.
    pub governing: Vec<String>,
    /// The advisor's independent search scope. Empty means the whole project.
    pub envelope: Vec<String>,
    /// Commands the advisor can run, `command` or `command=result`.
    pub commands: Vec<String>,
    /// Measurements already established.
    pub baselines: Vec<String>,
    /// Constraints the answer must respect.
    pub constraints: Vec<String>,
    /// Evidence records already written.
    pub evidence: Vec<String>,
    /// Artefacts already produced.
    pub artifacts: Vec<String>,
    /// Layer B.
    pub notes: WorkerNotes,
    /// The session head's token budget.
    pub budget: Option<usize>,
}

fn env(reason: String) -> EnvError {
    EnvError::new("AKR-C042", reason)
}

fn missing(id: &str) -> EnvError {
    EnvError::new("AKR-C043", format!("no advisor packet {id}")).help(
        "`akr handoff list` shows the packets this workspace holds; \
         `akr handoff create` makes one",
    )
}

/// A `--command` value, split on the first `=` into a command and its recorded result.
fn prepared(raw: &str) -> PreparedCommand {
    match raw.split_once('=') {
        Some((command, result)) if !command.trim().is_empty() && !result.trim().is_empty() => {
            PreparedCommand {
                command: command.trim().to_owned(),
                result: Some(result.trim().to_owned()),
            }
        }
        _ => PreparedCommand {
            command: raw.trim().to_owned(),
            result: None,
        },
    }
}

/// `akr handoff create`.
///
/// # Errors
/// [`EnvError`] when the session head cannot be assembled or the packet cannot be written.
pub fn create(session: &Session, request: &CreateRequest) -> Result<Output, EnvError> {
    let head = super::assemble(session, request.budget)?;
    let workspace = Fingerprint::capture(session);
    let ordinal = packet::ids(&session.root).len();
    let mut built = AdvisorPacket::new(&request.task, session.today, workspace, ordinal);

    built.created_by = request.by.clone();
    built.question = request.question.clone();
    // The head's own trailing blank line becomes an indented blank line once the packet
    // renders it under `PROJECT STATE`, which reads as an accident rather than a break.
    built.session_head = head.text.trim_end().to_owned();
    built.namespaces = session
        .ledger
        .project
        .namespaces
        .iter()
        .map(ToString::to_string)
        .collect();
    built.goal = request.goal.clone();
    built.governing = request.governing.clone();
    if !request.envelope.is_empty() {
        built.search_envelope = request.envelope.clone();
    }
    built.commands = request.commands.iter().map(|raw| prepared(raw)).collect();
    built.baselines = request.baselines.clone();
    built.constraints = request.constraints.clone();
    built.evidence = request.evidence.clone();
    built.artifacts = request.artifacts.clone();
    built.worker_notes = request.notes.clone();

    let path = packet::save(&session.root, &built).map_err(env)?;
    let relative = path
        .strip_prefix(&session.root)
        .unwrap_or(&path)
        .display()
        .to_string()
        .replace('\\', "/");

    let mut text = format!("advisor packet {}\n", built.id);
    text.push_str(&format!("stored       {relative}\n"));
    text.push_str(&format!("workspace    {}\n", built.workspace.line()));
    text.push_str(&format!(
        "envelope     {}\n",
        built.search_envelope.join(", ")
    ));
    text.push_str(&format!(
        "worker notes {}\n",
        if built.worker_notes.is_empty() {
            "none recorded"
        } else {
            "recorded and withheld until `akr handoff reveal`"
        }
    ));
    text.push_str(&format!(
        "\nhand this to the advisor:\n  akr handoff open {}\n",
        built.id
    ));

    Ok(Output::plain(
        text,
        Value::object(vec![
            ("packet", Value::string(built.id.clone())),
            ("path", Value::string(relative)),
            (
                "search_envelope",
                Value::array(
                    built
                        .search_envelope
                        .iter()
                        .cloned()
                        .map(Value::string)
                        .collect(),
                ),
            ),
            (
                "worker_notes_available",
                Value::bool(!built.worker_notes.is_empty()),
            ),
        ]),
    ))
}

/// `akr handoff list`.
///
/// # Errors
/// [`EnvError`] when a stored packet cannot be read.
pub fn list(session: &Session) -> Result<Output, EnvError> {
    let ids = packet::ids(&session.root);
    if ids.is_empty() {
        return Ok(Output::plain(
            "no advisor packets in this workspace\n".to_owned(),
            Value::object(vec![("packets", Value::array(Vec::new()))]),
        ));
    }
    let mut text = format!(
        "{} advisor {}\n",
        ids.len(),
        if ids.len() == 1 { "packet" } else { "packets" }
    );
    let mut rows = Vec::new();
    for id in &ids {
        // A packet that will not parse is listed rather than fatal: one damaged file must
        // not make the other packets in the directory unreachable.
        let Ok(stored) = packet::load(&session.root, id) else {
            text.push_str(&format!("  {id}  (unreadable)\n"));
            rows.push(Value::object(vec![
                ("packet", Value::string(id.clone())),
                ("readable", Value::bool(false)),
            ]));
            continue;
        };
        let summary: String = stored
            .task
            .lines()
            .next()
            .unwrap_or_default()
            .chars()
            .take(60)
            .collect();
        text.push_str(&format!(
            "  {}  {}  {}{}\n",
            stored.id,
            stored.created_at,
            summary,
            if stored.revealed() {
                "  [revealed]"
            } else {
                ""
            }
        ));
        rows.push(Value::object(vec![
            ("packet", Value::string(stored.id.clone())),
            ("readable", Value::bool(true)),
            ("created_at", Value::string(stored.created_at.clone())),
            ("task", Value::string(stored.task.clone())),
            ("revealed", Value::bool(stored.revealed())),
        ]));
    }
    Ok(Output::plain(
        text,
        Value::object(vec![("packets", Value::array(rows))]),
    ))
}

/// The Layer A rendering: everything except the worker notes.
fn layer_a(stored: &AdvisorPacket, drift: &super::snapshot::Drift) -> String {
    let mut text = format!("ADVISOR PACKET {}\n", stored.id);
    text.push_str(&format!(
        "prepared     {}{}\n",
        stored.created_at,
        stored
            .created_by
            .as_ref()
            .map_or_else(String::new, |by| format!(" by {by}"))
    ));
    text.push_str(&format!(
        "workspace    {} ({})\n",
        stored.workspace.line(),
        drift.status()
    ));
    if !drift.exact {
        if let Some((was, now)) = &drift.head_moved {
            text.push_str(&format!(
                "  HEAD moved {} -> {}\n",
                &was[..was.len().min(8)],
                &now[..now.len().min(8)]
            ));
        }
        if drift.ledger_moved {
            text.push_str("  the ledger changed since this packet was prepared\n");
        }
        for path in &drift.changed {
            text.push_str(&format!("  changed since packet: {path}\n"));
        }
    }

    text.push_str("\nTASK (verbatim — this is the user's request, not a restatement)\n");
    for line in stored.task.lines() {
        text.push_str(&format!("  {line}\n"));
    }
    if let Some(question) = &stored.question {
        text.push_str("\nASKED OF THE ADVISOR\n");
        for line in question.lines() {
            text.push_str(&format!("  {line}\n"));
        }
    }

    text.push_str("\nINDEPENDENT SEARCH SCOPE\n");
    for glob in &stored.search_envelope {
        text.push_str(&format!("  {glob}\n"));
    }
    text.push_str(
        "  Look wherever this scope reaches. Nothing here narrows it to what the\n  \
         preparing agent examined.\n",
    );

    if !stored.session_head.is_empty() {
        text.push_str("\nPROJECT STATE\n");
        for line in stored.session_head.lines() {
            text.push_str(&format!("  {line}\n"));
        }
    }
    if stored.goal.is_some() || !stored.governing.is_empty() {
        text.push_str("\nGOVERNING CONTEXT\n");
        if let Some(goal) = &stored.goal {
            text.push_str(&format!("  goal  {goal}\n"));
        }
        for reference in &stored.governing {
            text.push_str(&format!("  {reference}\n"));
        }
    }
    if !stored.constraints.is_empty() {
        text.push_str("\nCONSTRAINTS\n");
        for constraint in &stored.constraints {
            text.push_str(&format!("  {constraint}\n"));
        }
    }
    if !stored.commands.is_empty() {
        text.push_str("\nCOMMANDS\n");
        for prepared in &stored.commands {
            text.push_str(&format!(
                "  {}{}\n",
                prepared.command,
                prepared
                    .result
                    .as_ref()
                    .map_or_else(String::new, |result| format!("  -> {result}"))
            ));
        }
    }
    if !stored.baselines.is_empty() {
        text.push_str("\nBASELINES\n");
        for baseline in &stored.baselines {
            text.push_str(&format!("  {baseline}\n"));
        }
    }
    if !stored.evidence.is_empty() || !stored.artifacts.is_empty() {
        text.push_str("\nALREADY PRODUCED\n");
        for reference in &stored.evidence {
            text.push_str(&format!("  evidence  {reference}\n"));
        }
        for artifact in &stored.artifacts {
            text.push_str(&format!("  artifact  {artifact}\n"));
        }
    }
    text
}

/// The Layer B rendering.
fn layer_b(notes: &WorkerNotes) -> String {
    let mut text = String::from(
        "\nWORKER NOTES (non-authoritative: what the preparing agent thought, not what\n\
         it established)\n",
    );
    let mut section = |label: &str, items: &[String]| {
        if !items.is_empty() {
            text.push_str(&format!("  {label}\n"));
            for item in items {
                text.push_str(&format!("    {item}\n"));
            }
        }
    };
    section("hypotheses", &notes.hypotheses);
    section("examined", &notes.examined);
    section("not examined", &notes.not_examined);
    section("proposed approaches", &notes.approaches);
    section("searches run", &notes.searches);
    if notes.is_empty() {
        text.push_str("  (none recorded)\n");
    }
    text
}

fn notes_json(notes: &WorkerNotes) -> Value {
    let strings =
        |items: &[String]| Value::array(items.iter().cloned().map(Value::string).collect());
    Value::object(vec![
        ("hypotheses", strings(&notes.hypotheses)),
        ("examined", strings(&notes.examined)),
        ("not_examined", strings(&notes.not_examined)),
        ("approaches", strings(&notes.approaches)),
        ("searches", strings(&notes.searches)),
    ])
}

/// `akr handoff open <id>` — the blind read.
///
/// # Errors
/// [`EnvError`] when the packet is missing or unreadable, or `--reveal` cannot record itself.
pub fn open(session: &Session, id: &str, reveal_now: bool) -> Result<Output, EnvError> {
    let mut stored = packet::load(&session.root, id).map_err(|reason| {
        if packet::path(&session.root, id).is_file() {
            env(reason)
        } else {
            missing(id)
        }
    })?;
    let drift = stored.workspace.compare(&Fingerprint::capture(session));

    // `--reveal` takes the same recorded path as `akr handoff reveal`: an advisor may
    // read the notes at once, but the packet must never be unable to say whether the
    // review that follows was independent.
    if reveal_now && !stored.revealed() {
        stored.revealed_at = Some(session.today.to_string());
        packet::save(&session.root, &stored).map_err(env)?;
    }
    let show_notes = reveal_now || stored.revealed();

    let mut text = layer_a(&stored, &drift);
    if show_notes {
        text.push_str(&layer_b(&stored.worker_notes));
    } else {
        text.push_str(&format!(
            "\nWORKER NOTES  {}\n",
            if stored.worker_notes.is_empty() {
                "none recorded".to_owned()
            } else {
                format!(
                    "withheld — review independently first, then `akr handoff reveal {}`",
                    stored.id
                )
            }
        ));
    }

    let mut fields = vec![
        ("packet", Value::string(stored.id.clone())),
        ("task", Value::string(stored.task.clone())),
        (
            "question",
            stored
                .question
                .as_ref()
                .map_or(Value::Null, |q| Value::string(q.clone())),
        ),
        ("workspace", stored.workspace.to_json()),
        ("drift", drift.to_json()),
        (
            "search_envelope",
            Value::array(
                stored
                    .search_envelope
                    .iter()
                    .cloned()
                    .map(Value::string)
                    .collect(),
            ),
        ),
        ("session_head", Value::string(stored.session_head.clone())),
        (
            "governing_context",
            Value::object(vec![
                (
                    "goal",
                    stored
                        .goal
                        .as_ref()
                        .map_or(Value::Null, |goal| Value::string(goal.clone())),
                ),
                (
                    "records",
                    Value::array(
                        stored
                            .governing
                            .iter()
                            .cloned()
                            .map(Value::string)
                            .collect(),
                    ),
                ),
            ]),
        ),
        (
            "worker_notes_available",
            Value::bool(!stored.worker_notes.is_empty()),
        ),
        ("worker_notes_revealed", Value::bool(show_notes)),
    ];
    if show_notes {
        fields.push(("worker_notes", notes_json(&stored.worker_notes)));
    }
    Ok(Output::plain(text, Value::object(fields)))
}

/// The sections `akr handoff expand` will render.
pub const SECTIONS: &[&str] = &[
    "task",
    "workspace",
    "project",
    "governing",
    "envelope",
    "execution",
    "notes",
];

/// `akr handoff expand <id> <section>`.
///
/// # Errors
/// [`EnvError`] when the packet is missing, or the section name is not one of [`SECTIONS`].
pub fn expand(session: &Session, id: &str, section: &str) -> Result<Output, EnvError> {
    let stored = packet::load(&session.root, id).map_err(|reason| {
        if packet::path(&session.root, id).is_file() {
            env(reason)
        } else {
            missing(id)
        }
    })?;
    let strings =
        |items: &[String]| Value::array(items.iter().cloned().map(Value::string).collect());
    let lines = |label: &str, items: &[String]| {
        let mut text = format!("{label}\n");
        for item in items {
            text.push_str(&format!("  {item}\n"));
        }
        text
    };

    let (text, value) = match section {
        "task" => (
            format!("{}\n", stored.task),
            Value::object(vec![
                ("task", Value::string(stored.task.clone())),
                (
                    "question",
                    stored
                        .question
                        .as_ref()
                        .map_or(Value::Null, |q| Value::string(q.clone())),
                ),
            ]),
        ),
        "workspace" => {
            let drift = stored.workspace.compare(&Fingerprint::capture(session));
            let mut text = format!("{}\n", stored.workspace.line());
            for entry in &stored.workspace.dirty {
                text.push_str(&format!(
                    "  {}  {}\n",
                    &entry.digest[..entry.digest.len().min(12)],
                    entry.path
                ));
            }
            (
                text,
                Value::object(vec![
                    ("workspace", stored.workspace.to_json()),
                    ("drift", drift.to_json()),
                ]),
            )
        }
        "project" => (
            format!(
                "namespaces {}\n\n{}",
                stored.namespaces.join(", "),
                stored.session_head
            ),
            Value::object(vec![
                ("namespaces", strings(&stored.namespaces)),
                ("session_head", Value::string(stored.session_head.clone())),
            ]),
        ),
        "governing" => (
            lines(
                &format!(
                    "goal {}",
                    stored.goal.clone().unwrap_or_else(|| "(none)".to_owned())
                ),
                &stored.governing,
            ),
            Value::object(vec![
                (
                    "goal",
                    stored
                        .goal
                        .as_ref()
                        .map_or(Value::Null, |goal| Value::string(goal.clone())),
                ),
                ("records", strings(&stored.governing)),
            ]),
        ),
        "envelope" => (
            lines("independent search scope", &stored.search_envelope),
            Value::object(vec![("search_envelope", strings(&stored.search_envelope))]),
        ),
        "execution" => {
            let mut text = String::from("commands\n");
            for prepared in &stored.commands {
                text.push_str(&format!(
                    "  {}{}\n",
                    prepared.command,
                    prepared
                        .result
                        .as_ref()
                        .map_or_else(String::new, |result| format!("  -> {result}"))
                ));
            }
            text.push_str(&lines("baselines", &stored.baselines));
            text.push_str(&lines("constraints", &stored.constraints));
            text.push_str(&lines("evidence", &stored.evidence));
            text.push_str(&lines("artifacts", &stored.artifacts));
            (
                text,
                Value::object(vec![
                    (
                        "commands",
                        Value::array(
                            stored
                                .commands
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
                    ("baselines", strings(&stored.baselines)),
                    ("constraints", strings(&stored.constraints)),
                    ("evidence", strings(&stored.evidence)),
                    ("artifacts", strings(&stored.artifacts)),
                ]),
            )
        }
        // Expanding the notes is revealing them, so it goes through `reveal` rather than
        // quietly becoming a second door into Layer B that leaves no record.
        "notes" => return reveal(session, id),
        other => {
            return Err(
                EnvError::new("AKR-C004", format!("{other:?} is not a packet section"))
                    .help(format!("sections are: {}", SECTIONS.join(", "))),
            );
        }
    };
    Ok(Output::plain(text, value))
}

/// `akr handoff reveal <id>` — Layer B, and the record that it was read.
///
/// # Errors
/// [`EnvError`] when the packet is missing or cannot be rewritten.
pub fn reveal(session: &Session, id: &str) -> Result<Output, EnvError> {
    let mut stored = packet::load(&session.root, id).map_err(|reason| {
        if packet::path(&session.root, id).is_file() {
            env(reason)
        } else {
            missing(id)
        }
    })?;
    let first = !stored.revealed();
    if first {
        stored.revealed_at = Some(session.today.to_string());
        packet::save(&session.root, &stored).map_err(env)?;
    }

    let mut text = layer_b(&stored.worker_notes);
    text.push_str(
        "\nCompare these against what you found independently. What did either side\n\
         miss? Where the two disagree, the code decides — not this packet.\n",
    );
    Ok(Output::plain(
        text,
        Value::object(vec![
            ("packet", Value::string(stored.id.clone())),
            ("worker_notes", notes_json(&stored.worker_notes)),
            (
                "revealed_at",
                stored
                    .revealed_at
                    .as_ref()
                    .map_or(Value::Null, |at| Value::string(at.clone())),
            ),
            ("first_reveal", Value::bool(first)),
        ]),
    ))
}

/// `akr handoff verify <id>` — does the packet still describe this tree?
///
/// # Errors
/// [`EnvError`] when the packet is missing or unreadable.
pub fn verify(session: &Session, id: &str) -> Result<Output, EnvError> {
    let stored = packet::load(&session.root, id).map_err(|reason| {
        if packet::path(&session.root, id).is_file() {
            env(reason)
        } else {
            missing(id)
        }
    })?;
    let drift = stored.workspace.compare(&Fingerprint::capture(session));
    let mut text = format!("{} {}\n", stored.id, drift.status());
    if let Some((was, now)) = &drift.head_moved {
        text.push_str(&format!("  HEAD  {was} -> {now}\n"));
    }
    if drift.ledger_moved {
        text.push_str("  ledger changed\n");
    }
    for path in &drift.changed {
        text.push_str(&format!("  changed  {path}\n"));
    }
    if drift.exact {
        text.push_str("  the workspace is as the packet describes it\n");
    }

    // Drift is a fact about the working tree, not a ledger contradiction, so it never
    // changes the exit status — the same standing staleness has under D-024.
    Ok(Output::plain(
        text,
        Value::object(vec![
            ("packet", Value::string(stored.id.clone())),
            ("drift", drift.to_json()),
        ]),
    ))
}

/// `akr handoff discard <id>`.
///
/// # Errors
/// [`EnvError`] when the file exists and cannot be removed.
pub fn discard(session: &Session, id: &str) -> Result<Output, EnvError> {
    let removed = packet::discard(&session.root, id).map_err(env)?;
    let text = if removed {
        format!("discarded advisor packet {id}\n")
    } else {
        format!("no advisor packet {id}\n")
    };
    Ok(Output::plain(
        text,
        Value::object(vec![
            ("packet", Value::string(id.to_owned())),
            ("discarded", Value::bool(removed)),
        ]),
    ))
}

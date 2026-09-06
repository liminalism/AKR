//! `akr handoff *` — the commands, over the capsules, packets and results.
//!
//! Everything here assembles facts the rest of the tool already computes — the session
//! head, the workspace fingerprint, the project's shape — and puts them somewhere another
//! agent can address by name. No model participates in building a packet, exactly as none
//! participates in any other stage of this tool.
//!
//! Two rules are enforced here rather than merely documented.
//!
//! **Disclosure follows the mode.** [`open`] shows Layer B only when the mode discloses it
//! or somebody revealed it, and [`reveal`] stamps the packet. `--reveal` on `open` takes
//! the same recorded path — a child that wants the notes at once may have them, but the
//! packet says that it did.
//!
//! **Inheritance is resolved, not copied.** [`open`] walks `inherits` at read time. A
//! session capsule corrected after five packets were cut corrects all five.

use super::capsule::{self, Project, SessionCapsule};
use super::packet::{self, Mode, Packet, WorkerNotes};
use super::result::{self, Coverage, Report};
use super::snapshot::Fingerprint;
use super::{PreparedCommand, block};
use crate::commands::Output;
use crate::session::{EnvError, Session};
use akr_core::json::Value;

fn env(reason: String) -> EnvError {
    EnvError::new("AKR-C042", reason)
}

fn missing(id: &str) -> EnvError {
    EnvError::new("AKR-C043", format!("no handoff object {id}")).help(
        "`akr handoff list` shows what this workspace holds; `akr handoff worker|scout|\
         reviewer|advisor` cuts a packet",
    )
}

fn no_session() -> EnvError {
    EnvError::new("AKR-C044", "no handoff session is open in this workspace").help(
        "`akr handoff session begin --request \"<the user's request, verbatim>\"` opens one; \
         every packet cut afterwards inherits it",
    )
}

fn load_packet(session: &Session, id: &str) -> Result<Packet, EnvError> {
    packet::load(&session.root, id).map_err(|reason| {
        if packet::path(&session.root, id).is_file() {
            env(reason)
        } else {
            missing(id)
        }
    })
}

// ---------------------------------------------------------------------------------------
// the project capsule
// ---------------------------------------------------------------------------------------

/// Reads the stored project capsule, deriving and storing one if there is none.
///
/// Boundaries are the one field nothing derives, so a stored capsule's are carried
/// forward: they are the part a person wrote, and re-deriving would silently drop them.
///
/// # Errors
/// [`EnvError`] when the capsule cannot be written.
pub fn project_capsule(session: &Session, refresh: bool) -> Result<Project, EnvError> {
    let stored = packet::read_json(&session.root, "project")
        .ok()
        .and_then(|value| Project::from_json(&value).ok());
    if let (Some(stored), false) = (&stored, refresh) {
        return Ok(stored.clone());
    }
    let mut derived = capsule::derive_project(session);
    if let Some(stored) = stored {
        derived.boundaries = stored.boundaries;
    }
    packet::write_json(&session.root, "project", &derived.to_json()).map_err(env)?;
    Ok(derived)
}

/// `akr handoff capsule [--refresh] [--boundary <text>]...`.
///
/// # Errors
/// [`EnvError`] when the capsule cannot be written.
pub fn capsule_show(
    session: &Session,
    refresh: bool,
    boundaries: &[String],
) -> Result<Output, EnvError> {
    let mut capsule = project_capsule(session, refresh || !boundaries.is_empty())?;
    if !boundaries.is_empty() {
        capsule.boundaries = boundaries.to_vec();
        packet::write_json(&session.root, "project", &capsule.to_json()).map_err(env)?;
    }
    Ok(Output::plain(capsule.render(), capsule.to_json()))
}

// ---------------------------------------------------------------------------------------
// the session capsule
// ---------------------------------------------------------------------------------------

/// Everything `akr handoff session begin` accepts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionRequest {
    /// The user's request, verbatim.
    pub request: String,
    /// The governing planning key.
    pub goal: Option<String>,
    /// Records that govern the work.
    pub governing: Vec<String>,
    /// Constraints the work must respect.
    pub constraints: Vec<String>,
    /// Commands run this session, `command` or `command=result`.
    pub commands: Vec<String>,
    /// Measurements established.
    pub baselines: Vec<String>,
    /// Evidence records already written.
    pub evidence: Vec<String>,
    /// Artefacts already produced.
    pub artifacts: Vec<String>,
    /// The session head's token budget.
    pub budget: Option<usize>,
}

/// `akr handoff session begin`.
///
/// # Errors
/// [`EnvError`] when the session head cannot be assembled or the capsule cannot be written.
pub fn session_begin(session: &Session, request: &SessionRequest) -> Result<Output, EnvError> {
    let project = project_capsule(session, false)?;
    let head = super::assemble(session, request.budget)?;
    let workspace = Fingerprint::capture(session);
    let ordinal = packet::ids_with(&session.root, |id| id.starts_with("sx-")).len();

    let mut capsule = SessionCapsule::new(
        &request.request,
        &project,
        workspace,
        session.today,
        ordinal,
    );
    capsule.goal = request.goal.clone();
    capsule.governing = request.governing.clone();
    // The head's trailing blank line becomes an indented blank line once a packet renders
    // it, which reads as an accident rather than a break.
    capsule.session_head = head.text.trim_end().to_owned();
    capsule.constraints = request.constraints.clone();
    capsule.commands = request
        .commands
        .iter()
        .map(|raw| PreparedCommand::parse(raw))
        .collect();
    capsule.baselines = request.baselines.clone();
    capsule.evidence = request.evidence.clone();
    capsule.artifacts = request.artifacts.clone();

    packet::write_json(&session.root, &capsule.id, &capsule.to_json()).map_err(env)?;
    packet::set_current_session(&session.root, &capsule.id).map_err(env)?;

    let mut text = format!("handoff session {}\n", capsule.id);
    text.push_str(&format!("project      {}\n", project.id));
    text.push_str(&format!("workspace    {}\n", capsule.workspace.line()));
    text.push_str(
        "\nevery packet cut from here inherits this session. Delegate with:\n  \
         akr handoff worker|scout|reviewer|advisor --role <role> --task <task>\n",
    );
    Ok(Output::plain(
        text,
        Value::object(vec![
            ("session", Value::string(capsule.id.clone())),
            ("project", Value::string(project.id)),
            ("request", Value::string(capsule.request.clone())),
        ]),
    ))
}

/// Reads the open session capsule.
///
/// # Errors
/// [`EnvError`] when none is open or it cannot be read.
pub fn open_session(session: &Session) -> Result<SessionCapsule, EnvError> {
    let id = packet::current_session(&session.root).ok_or_else(no_session)?;
    let value = packet::read_json(&session.root, &id).map_err(env)?;
    SessionCapsule::from_json(&value).map_err(|reason| env(format!("{id}: {reason}")))
}

/// `akr handoff session show`.
///
/// # Errors
/// [`EnvError`] when no session is open.
pub fn session_show(session: &Session) -> Result<Output, EnvError> {
    let capsule = open_session(session)?;
    let drift = capsule.workspace.compare(&Fingerprint::capture(session));
    let packets: Vec<Packet> = packets_of(session, &capsule.id);
    let reports: Vec<Report> = result::all(&session.root)
        .into_iter()
        .filter(|report| packets.iter().any(|p| p.id == report.packet))
        .collect();

    let mut text = format!("handoff session {}\n", capsule.id);
    text.push_str(&format!("opened       {}\n", capsule.opened_at));
    text.push_str(&format!("project      {}\n", capsule.project));
    text.push_str(&format!(
        "workspace    {} ({})\n",
        capsule.workspace.line(),
        drift.status()
    ));
    text.push_str(&format!(
        "packets      {} cut, {} answered\n",
        packets.len(),
        reports.len()
    ));
    text.push_str("\nREQUEST (verbatim)\n");
    for line in capsule.request.lines() {
        text.push_str(&format!("  {line}\n"));
    }
    for packet in &packets {
        let answered = reports.iter().any(|report| report.packet == packet.id);
        text.push_str(&format!(
            "\n  {}  {:<8} {}{}",
            packet.id,
            packet.mode.as_str(),
            packet.role.clone().unwrap_or_else(|| "-".to_owned()),
            if answered { "  [answered]\n" } else { "\n" }
        ));
    }

    Ok(Output::plain(
        text,
        Value::object(vec![
            ("session", Value::string(capsule.id.clone())),
            ("project", Value::string(capsule.project.clone())),
            ("request", Value::string(capsule.request.clone())),
            ("drift", drift.to_json()),
            (
                "packets",
                Value::array(
                    packets
                        .iter()
                        .map(|packet| {
                            Value::object(vec![
                                ("packet", Value::string(packet.id.clone())),
                                ("mode", Value::string(packet.mode.as_str())),
                                (
                                    "role",
                                    packet
                                        .role
                                        .as_ref()
                                        .map_or(Value::Null, |r| Value::string(r.clone())),
                                ),
                                (
                                    "answered",
                                    Value::bool(
                                        reports.iter().any(|report| report.packet == packet.id),
                                    ),
                                ),
                            ])
                        })
                        .collect(),
                ),
            ),
        ]),
    ))
}

/// `akr handoff session end`.
///
/// # Errors
/// [`EnvError`] when no session is open.
pub fn session_end(session: &Session) -> Result<Output, EnvError> {
    let capsule = open_session(session)?;
    packet::clear_current_session(&session.root);
    // The capsule file stays. A packet that inherits it must still resolve after the
    // session closes, or every result filed later would render against nothing.
    Ok(Output::plain(
        format!(
            "closed handoff session {}\n  its capsule and packets remain readable; \
             `akr handoff discard <id>` removes one\n",
            capsule.id
        ),
        Value::object(vec![
            ("session", Value::string(capsule.id)),
            ("closed", Value::bool(true)),
        ]),
    ))
}

/// Every packet inheriting `session_id`.
fn packets_of(session: &Session, session_id: &str) -> Vec<Packet> {
    packet::ids(&session.root)
        .into_iter()
        .filter_map(|id| packet::load(&session.root, &id).ok())
        .filter(|packet| packet.session() == Some(session_id))
        .collect()
}

// ---------------------------------------------------------------------------------------
// cutting a packet
// ---------------------------------------------------------------------------------------

/// Everything `akr handoff <mode>` and `knowledge.handoff_create` accept.
///
/// One struct for both surfaces, so the MCP tool cannot grow a field the command line has
/// no way to pass (`docs/08-mcp.md` §1).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CreateRequest {
    /// Which mode. `None` is an error the parser catches first.
    pub mode: Option<String>,
    /// The assignment. May be empty for `advisor`, which then inherits the request.
    pub task: String,
    /// What the child is, in two or three words.
    pub role: Option<String>,
    /// Who is cutting the packet.
    pub by: Option<String>,
    /// Extra ids to inherit beyond the open session.
    pub inherits: Vec<String>,
    /// Where the child may look. Empty means the whole project.
    pub scope: Vec<String>,
    /// Facts already established.
    pub known: Vec<String>,
    /// Work the child must not repeat.
    pub skip: Vec<String>,
    /// What the child should return. Empty takes the mode's default.
    pub expect: Vec<String>,
    /// Commands specific to this assignment.
    pub commands: Vec<String>,
    /// Layer B.
    pub notes: WorkerNotes,
}

/// `akr handoff worker|scout|reviewer|advisor`.
///
/// # Errors
/// [`EnvError`] when the mode is unknown, the task is missing, or the packet cannot be
/// written.
pub fn create(session: &Session, request: &CreateRequest) -> Result<Output, EnvError> {
    let mode = request
        .mode
        .as_deref()
        .and_then(Mode::from_name)
        .ok_or_else(|| {
            EnvError::new(
                "AKR-C004",
                format!(
                    "{:?} is not a handoff mode",
                    request.mode.as_deref().unwrap_or("")
                ),
            )
            .help(format!(
                "modes are: {}",
                Mode::ALL
                    .iter()
                    .map(|m| m.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?;

    // A session is optional but strongly wanted: without one the child inherits nothing
    // and the whole point of the subsystem is lost, so say so rather than silently
    // producing a thin packet.
    let capsule = packet::current_session(&session.root)
        .and_then(|id| packet::read_json(&session.root, &id).ok())
        .and_then(|value| SessionCapsule::from_json(&value).ok());

    let task = if request.task.trim().is_empty() {
        capsule
            .as_ref()
            .map(|capsule| capsule.request.clone())
            .ok_or_else(|| {
                EnvError::new("AKR-C003", "a packet needs a task").help(
                    "pass --task, or open a session with `akr handoff session begin \
                     --request ...` and an advisor packet will inherit it verbatim",
                )
            })?
    } else {
        request.task.clone()
    };

    let mut inherits: Vec<String> = Vec::new();
    if let Some(capsule) = &capsule {
        inherits.push(capsule.id.clone());
    }
    for extra in &request.inherits {
        if !inherits.contains(extra) {
            inherits.push(extra.clone());
        }
    }

    let ordinal = packet::ids(&session.root).len();
    let mut built = Packet::new(mode, &task, session.today, &inherits, ordinal);
    built.created_by = request.by.clone();
    built.role = request.role.clone();
    if !request.scope.is_empty() {
        built.scope = request.scope.clone();
    }
    built.known = request.known.clone();
    built.skip = request.skip.clone();
    if !request.expect.is_empty() {
        built.expected_return = request.expect.clone();
    }
    built.commands = request
        .commands
        .iter()
        .map(|raw| PreparedCommand::parse(raw))
        .collect();
    built.worker_notes = request.notes.clone();

    let path = packet::save(&session.root, &built).map_err(env)?;
    let relative = path
        .strip_prefix(&session.root)
        .unwrap_or(&path)
        .display()
        .to_string()
        .replace('\\', "/");

    let mut text = format!("{} packet {}\n", mode.as_str(), built.id);
    text.push_str(&format!("stored       {relative}\n"));
    text.push_str(&format!(
        "inherits     {}\n",
        if inherits.is_empty() {
            "nothing — no session is open, so this packet carries no project or session \
             context"
                .to_owned()
        } else {
            inherits.join(", ")
        }
    ));
    text.push_str(&format!("scope        {}\n", built.scope.join(", ")));
    text.push_str(&format!(
        "worker notes {}\n",
        match (built.worker_notes.is_empty(), mode.discloses_notes()) {
            (true, _) => "none recorded",
            (false, true) => "included — this mode discloses them on open",
            (false, false) => "withheld until `akr handoff reveal`",
        }
    ));
    text.push_str(&format!(
        "\nhand this to the agent:\n  akr handoff open {}\n",
        built.id
    ));

    Ok(Output::plain(
        text,
        Value::object(vec![
            ("packet", Value::string(built.id.clone())),
            ("mode", Value::string(mode.as_str())),
            ("path", Value::string(relative)),
            ("inherits", super::strings(&inherits)),
            ("scope", super::strings(&built.scope)),
            (
                "worker_notes_available",
                Value::bool(!built.worker_notes.is_empty()),
            ),
        ]),
    ))
}

// ---------------------------------------------------------------------------------------
// reading a packet
// ---------------------------------------------------------------------------------------

/// `akr handoff list`.
///
/// # Errors
/// Never; an unreadable file is listed as such rather than made fatal.
pub fn list(session: &Session) -> Result<Output, EnvError> {
    let current = packet::current_session(&session.root);
    let ids = packet::ids(&session.root);
    let reports = result::all(&session.root);
    if ids.is_empty() {
        return Ok(Output::plain(
            current.as_ref().map_or_else(
                || "no handoff packets, and no session open\n".to_owned(),
                |id| format!("session {id} is open; no packets cut yet\n"),
            ),
            Value::object(vec![
                ("session", current.map_or(Value::Null, Value::string)),
                ("packets", Value::array(Vec::new())),
            ]),
        ));
    }

    let mut text = match &current {
        Some(id) => format!("session {id}, {} packets\n", ids.len()),
        None => format!("{} packets, no session open\n", ids.len()),
    };
    let mut rows = Vec::new();
    for id in &ids {
        // A packet that will not parse is listed rather than fatal: one damaged file must
        // not make the rest of the directory unreachable.
        let Ok(stored) = packet::load(&session.root, id) else {
            text.push_str(&format!("  {id}  (unreadable)\n"));
            rows.push(Value::object(vec![
                ("packet", Value::string(id.clone())),
                ("readable", Value::bool(false)),
            ]));
            continue;
        };
        let answered = reports.iter().any(|report| &report.packet == id);
        let summary: String = stored
            .role
            .clone()
            .unwrap_or_else(|| stored.task.lines().next().unwrap_or_default().to_owned())
            .chars()
            .take(48)
            .collect();
        text.push_str(&format!(
            "  {}  {:<8}  {:<48}{}{}\n",
            stored.id,
            stored.mode.as_str(),
            summary,
            if answered { "  [answered]" } else { "" },
            if stored.revealed() {
                "  [revealed]"
            } else {
                ""
            }
        ));
        rows.push(Value::object(vec![
            ("packet", Value::string(stored.id.clone())),
            ("readable", Value::bool(true)),
            ("mode", Value::string(stored.mode.as_str())),
            (
                "role",
                stored
                    .role
                    .as_ref()
                    .map_or(Value::Null, |r| Value::string(r.clone())),
            ),
            ("task", Value::string(stored.task.clone())),
            ("answered", Value::bool(answered)),
            ("revealed", Value::bool(stored.revealed())),
        ]));
    }
    Ok(Output::plain(
        text,
        Value::object(vec![
            ("session", current.map_or(Value::Null, Value::string)),
            ("packets", Value::array(rows)),
        ]),
    ))
}

/// The inherited context a packet resolves to.
struct Inherited {
    project: Option<Project>,
    session: Option<SessionCapsule>,
    parents: Vec<Packet>,
}

fn resolve(session: &Session, stored: &Packet) -> Inherited {
    let mut resolved = Inherited {
        project: None,
        session: None,
        parents: Vec::new(),
    };
    for id in &stored.inherits {
        if id.starts_with("sx-") {
            resolved.session = packet::read_json(&session.root, id)
                .ok()
                .and_then(|value| SessionCapsule::from_json(&value).ok());
        } else if packet::is_packet_id(id)
            && let Ok(parent) = packet::load(&session.root, id)
        {
            resolved.parents.push(parent);
        }
    }
    // The project capsule comes through the session rather than being named directly: one
    // path to it, so a packet cannot inherit a session and a different project.
    if let Some(capsule) = &resolved.session {
        resolved.project = packet::read_json(&session.root, "project")
            .ok()
            .and_then(|value| Project::from_json(&value).ok())
            .filter(|project| project.id == capsule.project)
            .or_else(|| {
                packet::read_json(&session.root, "project")
                    .ok()
                    .and_then(|value| Project::from_json(&value).ok())
            });
    }
    resolved
}

/// The rendering, with Layer B included or withheld.
fn render(stored: &Packet, inherited: &Inherited, drift: &super::snapshot::Drift) -> String {
    let mut text = format!(
        "{} PACKET {}\n",
        stored.mode.as_str().to_uppercase(),
        stored.id
    );
    if let Some(role) = &stored.role {
        text.push_str(&format!("role         {role}\n"));
    }
    text.push_str(&format!(
        "cut          {}{}\n",
        stored.created_at,
        stored
            .created_by
            .as_ref()
            .map_or_else(String::new, |by| format!(" by {by}"))
    ));
    if !stored.inherits.is_empty() {
        text.push_str(&format!("inherits     {}\n", stored.inherits.join(", ")));
    }
    if let Some(capsule) = &inherited.session {
        text.push_str(&format!(
            "workspace    {} ({})\n",
            capsule.workspace.line(),
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
                text.push_str("  the ledger changed since this session opened\n");
            }
            for path in &drift.changed {
                text.push_str(&format!("  changed since the session opened: {path}\n"));
            }
        }
    }

    text.push_str(&format!("\nYOU ARE\n  {}\n", stored.mode.contract()));

    if let Some(capsule) = &inherited.session {
        text.push_str("\nORIGINAL REQUEST (from the session, verbatim)\n");
        for line in capsule.request.lines() {
            text.push_str(&format!("  {line}\n"));
        }
        // Rendered only when it differs: repeating the request under a second heading
        // would teach a reader to skip both.
        if capsule.request.trim() != stored.task.trim() {
            text.push_str("\nYOUR ASSIGNMENT (narrowed from the above)\n");
            for line in stored.task.lines() {
                text.push_str(&format!("  {line}\n"));
            }
        }
    } else {
        text.push_str("\nTASK\n");
        for line in stored.task.lines() {
            text.push_str(&format!("  {line}\n"));
        }
    }

    text.push_str("\nSCOPE\n");
    for glob in &stored.scope {
        text.push_str(&format!("  {glob}\n"));
    }
    text.push_str(
        "  Look wherever this reaches. Nothing here narrows it to what another agent\n  \
         already examined.\n",
    );

    if let Some(project) = &inherited.project {
        text.push('\n');
        text.push_str(&project.render());
    }
    if let Some(capsule) = &inherited.session {
        if !capsule.session_head.is_empty() {
            text.push_str("\nPROJECT STATE\n");
            for line in capsule.session_head.lines() {
                text.push_str(&format!("  {line}\n"));
            }
        }
        if capsule.goal.is_some() || !capsule.governing.is_empty() {
            text.push_str("\nGOVERNING CONTEXT\n");
            if let Some(goal) = &capsule.goal {
                text.push_str(&format!("  goal  {goal}\n"));
            }
            for reference in &capsule.governing {
                text.push_str(&format!("  {reference}\n"));
            }
        }
        text.push_str(&block("\nCONSTRAINTS", &capsule.constraints, "  "));
        if capsule.has_established() {
            text.push_str("\nALREADY ESTABLISHED\n");
            for command in &capsule.commands {
                text.push_str(&format!("  {}\n", command.line()));
            }
            for baseline in &capsule.baselines {
                text.push_str(&format!("  {baseline}\n"));
            }
            for reference in &capsule.evidence {
                text.push_str(&format!("  evidence  {reference}\n"));
            }
            for artifact in &capsule.artifacts {
                text.push_str(&format!("  artifact  {artifact}\n"));
            }
        }
    }
    text.push_str(&block("\nALSO ESTABLISHED", &stored.known, "  "));
    if !stored.commands.is_empty() {
        text.push_str("\nCOMMANDS FOR THIS ASSIGNMENT\n");
        for command in &stored.commands {
            text.push_str(&format!("  {}\n", command.line()));
        }
    }
    text.push_str(&block("\nDO NOT REPEAT", &stored.skip, "  "));
    text.push_str(&block("\nRETURN", &stored.expected_return, "  "));

    // Coverage from earlier packets in the chain, so a worker does not re-open what its
    // predecessor already read. This is fact, not judgement, so every mode gets it.
    let earlier: Vec<String> = inherited
        .parents
        .iter()
        .flat_map(|parent| parent.worker_notes.examined.iter().cloned())
        .collect();
    text.push_str(&block(
        "\nALREADY EXAMINED BY AN EARLIER PACKET",
        &earlier,
        "  ",
    ));

    if stored.shows_notes() {
        text.push_str(&notes_text(&stored.worker_notes));
    } else {
        text.push_str(&format!(
            "\nWORKER NOTES  {}\n",
            if stored.worker_notes.is_empty() {
                "none recorded".to_owned()
            } else {
                format!(
                    "withheld — form your own view first, then `akr handoff reveal {}`",
                    stored.id
                )
            }
        ));
    }
    text.push_str(&format!(
        "\nWhen you are done: akr handoff result {} --finding ... --read ...\n",
        stored.id
    ));
    text
}

fn notes_text(notes: &WorkerNotes) -> String {
    let mut text = String::from(
        "\nWORKER NOTES (non-authoritative: what the parent thought, not what it\n\
         established)\n",
    );
    for (label, items) in [
        ("hypotheses", &notes.hypotheses),
        ("examined", &notes.examined),
        ("not examined", &notes.not_examined),
        ("proposed approaches", &notes.approaches),
        ("searches run", &notes.searches),
    ] {
        text.push_str(&block(&format!("  {label}"), items, "    "));
    }
    if notes.is_empty() {
        text.push_str("  (none recorded)\n");
    }
    text
}

/// `akr handoff open <id> [--reveal]`.
///
/// # Errors
/// [`EnvError`] when the packet is missing, or `--reveal` cannot record itself.
pub fn open(session: &Session, id: &str, reveal_now: bool) -> Result<Output, EnvError> {
    let mut stored = load_packet(session, id)?;
    let inherited = resolve(session, &stored);
    let drift = inherited
        .session
        .as_ref()
        .map(|capsule| capsule.workspace.compare(&Fingerprint::capture(session)))
        .unwrap_or_else(|| Fingerprint::capture(session).compare(&Fingerprint::capture(session)));

    // `--reveal` takes the same recorded path as `akr handoff reveal`: a child may read
    // the notes at once, but the packet must never be unable to say whether the pass that
    // followed was independent.
    if reveal_now && !stored.revealed() {
        stored.revealed_at = Some(session.today.to_string());
        packet::save(&session.root, &stored).map_err(env)?;
    }
    let shows = stored.shows_notes();

    let text = render(&stored, &inherited, &drift);
    let mut fields = vec![
        ("packet", Value::string(stored.id.clone())),
        ("mode", Value::string(stored.mode.as_str())),
        (
            "role",
            stored
                .role
                .as_ref()
                .map_or(Value::Null, |r| Value::string(r.clone())),
        ),
        ("task", Value::string(stored.task.clone())),
        (
            "request",
            inherited
                .session
                .as_ref()
                .map_or(Value::Null, |c| Value::string(c.request.clone())),
        ),
        ("inherits", super::strings(&stored.inherits)),
        ("scope", super::strings(&stored.scope)),
        ("known", super::strings(&stored.known)),
        ("skip", super::strings(&stored.skip)),
        ("expected_return", super::strings(&stored.expected_return)),
        ("drift", drift.to_json()),
        (
            "project",
            inherited
                .project
                .as_ref()
                .map_or(Value::Null, Project::to_json),
        ),
        (
            "session_head",
            inherited
                .session
                .as_ref()
                .map_or(Value::Null, |c| Value::string(c.session_head.clone())),
        ),
        (
            "worker_notes_available",
            Value::bool(!stored.worker_notes.is_empty()),
        ),
        ("worker_notes_revealed", Value::bool(shows)),
    ];
    if shows {
        fields.push(("worker_notes", stored.worker_notes.to_json()));
    }
    Ok(Output::plain(text, Value::object(fields)))
}

/// The sections `akr handoff expand` will render.
pub const SECTIONS: &[&str] = &[
    "task",
    "workspace",
    "project",
    "session",
    "scope",
    "assignment",
    "notes",
];

/// `akr handoff expand <id> <section>`.
///
/// # Errors
/// [`EnvError`] when the packet is missing, or the section is not one of [`SECTIONS`].
pub fn expand(session: &Session, id: &str, section: &str) -> Result<Output, EnvError> {
    let stored = load_packet(session, id)?;
    let inherited = resolve(session, &stored);

    let (text, value) = match section {
        "task" => (
            format!("{}\n", stored.task),
            Value::object(vec![
                ("task", Value::string(stored.task.clone())),
                (
                    "request",
                    inherited
                        .session
                        .as_ref()
                        .map_or(Value::Null, |c| Value::string(c.request.clone())),
                ),
            ]),
        ),
        "workspace" => {
            let fingerprint = inherited
                .session
                .as_ref()
                .map_or_else(|| Fingerprint::capture(session), |c| c.workspace.clone());
            let drift = fingerprint.compare(&Fingerprint::capture(session));
            let mut text = format!("{}\n", fingerprint.line());
            for entry in &fingerprint.dirty {
                text.push_str(&format!(
                    "  {}  {}\n",
                    &entry.digest[..entry.digest.len().min(12)],
                    entry.path
                ));
            }
            (
                text,
                Value::object(vec![
                    ("workspace", fingerprint.to_json()),
                    ("drift", drift.to_json()),
                ]),
            )
        }
        "project" => inherited.project.as_ref().map_or_else(
            || {
                (
                    "no project capsule is inherited\n".to_owned(),
                    Value::object(vec![("project", Value::Null)]),
                )
            },
            |project| (project.render(), project.to_json()),
        ),
        "session" => inherited.session.as_ref().map_or_else(
            || {
                (
                    "no session capsule is inherited\n".to_owned(),
                    Value::object(vec![("session", Value::Null)]),
                )
            },
            |capsule| (capsule.session_head.clone(), capsule.to_json()),
        ),
        "scope" => (
            block("scope", &stored.scope, "  "),
            Value::object(vec![("scope", super::strings(&stored.scope))]),
        ),
        "assignment" => {
            let mut text = String::new();
            text.push_str(&block("known", &stored.known, "  "));
            text.push_str(&block("do not repeat", &stored.skip, "  "));
            text.push_str(&block("return", &stored.expected_return, "  "));
            if !stored.commands.is_empty() {
                text.push_str("commands\n");
                for command in &stored.commands {
                    text.push_str(&format!("  {}\n", command.line()));
                }
            }
            (
                text,
                Value::object(vec![
                    ("known", super::strings(&stored.known)),
                    ("skip", super::strings(&stored.skip)),
                    ("expected_return", super::strings(&stored.expected_return)),
                    ("commands", super::commands_json(&stored.commands)),
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
    let mut stored = load_packet(session, id)?;
    let first = !stored.revealed();
    if first {
        stored.revealed_at = Some(session.today.to_string());
        packet::save(&session.root, &stored).map_err(env)?;
    }

    let mut text = notes_text(&stored.worker_notes);
    text.push_str(
        "\nCompare these against what you found. What did either side miss? Where the\n\
         two disagree, the code decides — not this packet.\n",
    );
    Ok(Output::plain(
        text,
        Value::object(vec![
            ("packet", Value::string(stored.id.clone())),
            ("worker_notes", stored.worker_notes.to_json()),
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
    let stored = load_packet(session, id)?;
    let inherited = resolve(session, &stored);
    let fingerprint = inherited
        .session
        .as_ref()
        .map_or_else(|| Fingerprint::capture(session), |c| c.workspace.clone());
    let drift = fingerprint.compare(&Fingerprint::capture(session));

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
    // The project capsule is the one object whose id is not its filename: it is stored at
    // a fixed name so that every session finds the same one without a pointer. Accepting
    // its `pc-` id here keeps `discard` uniform from outside, which is where it is used.
    let file = if id.starts_with("pc-") {
        packet::read_json(&session.root, "project")
            .ok()
            .and_then(|value| Project::from_json(&value).ok())
            .filter(|project| project.id == id)
            .map_or_else(|| id.to_owned(), |_| "project".to_owned())
    } else {
        id.to_owned()
    };
    let removed = packet::discard(&session.root, &file).map_err(env)?;
    if packet::current_session(&session.root).as_deref() == Some(id) {
        packet::clear_current_session(&session.root);
    }
    let text = if removed {
        format!("discarded {id}\n")
    } else {
        format!("no handoff object {id}\n")
    };
    Ok(Output::plain(
        text,
        Value::object(vec![
            ("id", Value::string(id.to_owned())),
            ("discarded", Value::bool(removed)),
        ]),
    ))
}

// ---------------------------------------------------------------------------------------
// results and coverage
// ---------------------------------------------------------------------------------------

/// Everything `akr handoff result` accepts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResultRequest {
    /// The packet being answered.
    pub packet: String,
    /// Who is filing.
    pub by: Option<String>,
    /// Findings, most significant first.
    pub findings: Vec<String>,
    /// Evidence for them.
    pub evidence: Vec<String>,
    /// What was changed, if anything.
    pub changes: Vec<String>,
    /// What is still uncertain.
    pub uncertainties: Vec<String>,
    /// What to do next.
    pub follow_up: Vec<String>,
    /// Commands run, `command` or `command=result`.
    pub commands: Vec<String>,
    /// Files read, optionally `path:start-end`.
    pub read: Vec<String>,
    /// Queries run.
    pub searched: Vec<String>,
    /// Suites or benchmarks run.
    pub tested: Vec<String>,
    /// What was in scope and deliberately not opened.
    pub not_examined: Vec<String>,
}

/// `akr handoff result <packet>`.
///
/// # Errors
/// [`EnvError`] when the packet is missing or the result cannot be written.
pub fn file_result(session: &Session, request: &ResultRequest) -> Result<Output, EnvError> {
    // Answering a packet that does not exist is a typo, and filing it anyway would put a
    // result where no aggregate will ever find it.
    let stored = load_packet(session, &request.packet)?;

    let ordinal = packet::ids_with(&session.root, Report::is_report_id).len();
    let mut report = Report::new(&stored.id, session.today, ordinal);
    report.filed_by = request.by.clone();
    report.findings = request.findings.clone();
    report.evidence = request.evidence.clone();
    report.changes = request.changes.clone();
    report.uncertainties = request.uncertainties.clone();
    report.follow_up = request.follow_up.clone();
    report.commands = request
        .commands
        .iter()
        .map(|raw| PreparedCommand::parse(raw))
        .collect();
    report.coverage = Coverage {
        read: request.read.clone(),
        searched: request.searched.clone(),
        tested: request.tested.clone(),
        not_examined: request.not_examined.clone(),
    };

    packet::write_json(&session.root, &report.id, &report.to_json()).map_err(env)?;

    let mut text = format!("result {} for {}\n", report.id, report.packet);
    if report.coverage.is_empty() {
        text.push_str(
            "  no coverage recorded — the next agent cannot see what you already read,\n  \
             so it will read it again. --read, --searched, --tested, --not-examined\n",
        );
    }
    Ok(Output::plain(
        text,
        Value::object(vec![
            ("result", Value::string(report.id.clone())),
            ("packet", Value::string(report.packet.clone())),
            (
                "coverage_recorded",
                Value::bool(!report.coverage.is_empty()),
            ),
        ]),
    ))
}

/// `akr handoff results [<packet>]` — what came back.
///
/// # Errors
/// [`EnvError`] when no session is open and no packet was named.
pub fn results(session: &Session, only: Option<&str>) -> Result<Output, EnvError> {
    let reports = collect(session, only)?;
    if reports.is_empty() {
        return Ok(Output::plain(
            "no results filed\n".to_owned(),
            Value::object(vec![("results", Value::array(Vec::new()))]),
        ));
    }
    let mut text = String::new();
    for report in &reports {
        text.push_str(&report.render());
        text.push('\n');
    }
    Ok(Output::plain(
        text,
        Value::object(vec![(
            "results",
            Value::array(reports.iter().map(Report::to_json).collect()),
        )]),
    ))
}

/// The results in scope: one packet's, or every packet of the open session's.
fn collect(session: &Session, only: Option<&str>) -> Result<Vec<Report>, EnvError> {
    let all = result::all(&session.root);
    if let Some(packet_id) = only {
        return Ok(all
            .into_iter()
            .filter(|report| report.packet == packet_id)
            .collect());
    }
    let capsule = open_session(session)?;
    let packets = packets_of(session, &capsule.id);
    Ok(all
        .into_iter()
        .filter(|report| packets.iter().any(|packet| packet.id == report.packet))
        .collect())
}

/// `akr handoff coverage` — what the session has and has not looked at.
///
/// # Errors
/// [`EnvError`] when no session is open.
pub fn coverage(session: &Session) -> Result<Output, EnvError> {
    let capsule = open_session(session)?;
    let packets = packets_of(session, &capsule.id);
    let reports: Vec<Report> = result::all(&session.root)
        .into_iter()
        .filter(|report| packets.iter().any(|packet| packet.id == report.packet))
        .collect();
    let aggregate = result::aggregate(&packets, &reports);

    let mut text = format!(
        "coverage for session {} — {} packets, {} results\n\n",
        capsule.id,
        packets.len(),
        reports.len()
    );
    text.push_str(&aggregate.render());
    Ok(Output::plain(
        text,
        Value::object(vec![
            ("session", Value::string(capsule.id)),
            ("coverage", aggregate.to_json()),
        ]),
    ))
}

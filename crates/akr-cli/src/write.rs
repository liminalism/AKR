//! The write commands of `docs/07-cli.md` §6, over `akr_core::ops`.
//!
//! Every one of them delegates the whole of §4's pipeline — parse, apply, validate,
//! format, write — to the library, and does three things of its own: turn command-line
//! strings into the library's request types, render an [`Applied`] or a [`Refused`], and
//! choose an exit status.
//!
//! # Refusals are rendered from structure, never from prose
//!
//! [`Refused`] carries `unfinished_children` and `unsatisfied_checks` as vectors. The
//! §6 samples — the list of children a supersession must dispose of, the `--disposition`
//! lines to copy — are built from those vectors here. Nothing parses a message string,
//! so a refusal renders identically however the library words it, and the MCP surface
//! can render the same data differently without either drifting.
//!
//! # Nothing here writes
//!
//! Not one line of this module touches the filesystem. That is not a stylistic
//! preference: `docs/07` §4 guarantees that a refused write leaves the working tree
//! byte-identical, and the guarantee is only checkable if there is exactly one writer.
//! `tests/writes.rs` hashes every source file around each refusing path.

use crate::args::Command;
use crate::commands::Output;
use crate::session::{EnvError, Exit, Session};
use akr_core::json::Value;
use akr_core::model::{
    Commit, ContentSlot, EvidenceResult, Kind, LogicalKey, Outcome as DispositionOutcome, Record,
    Reference, Relation, State,
};
use akr_core::ops::{
    Applied, Change, ChangeKind, DispositionRequest, Edits, Refused, ReviseMode, WriteContext,
    WriteResult,
};
use std::collections::BTreeSet;
use std::path::Path;

fn missing_commit(session: &Session, suggest_flag: bool) -> EnvError {
    let hint = if suggest_flag {
        "make an initial commit, or pass --observed-at <commit>"
    } else {
        "make an initial commit, or pass observed_at"
    };
    if session.repository.is_some() {
        EnvError::new(
            "AKR-G001",
            "no commit to record: the repository has no commits yet",
        )
        .help(hint)
    } else {
        EnvError::new(
            "AKR-G001",
            "no commit to record: not inside a git repository",
        )
        .help(hint)
    }
}

/// Runs a write command.
///
/// # Errors
/// [`EnvError`] when an argument cannot be turned into a request at all — a malformed
/// key, an unknown kind, an unreadable `--from` file. A refusal from the library is not
/// an error here: it is an [`Output`] with exit 1, because the tool did its job.
pub fn run(session: &Session, command: &Command) -> Result<Output, EnvError> {
    let context = context_of(session);
    match command {
        Command::Propose {
            key,
            kind,
            title,
            from,
        } => {
            let key = parse_key(key)?;
            let kind = parse_kind(kind)?;
            let template = body_from(session, from.as_deref(), &key, kind)?.map(|body| body.record);
            render(
                session,
                akr_core::ops::propose(
                    &context,
                    &key,
                    kind,
                    title.as_deref().unwrap_or_default(),
                    template,
                ),
            )
        }
        Command::Revise {
            key,
            from,
            state,
            title,
            in_place,
            dispositions,
        } => {
            let key = parse_key(key)?;
            let mode = if *in_place {
                ReviseMode::InPlace
            } else {
                ReviseMode::Auto
            };
            let mut edits = Edits {
                title: title.clone(),
                state: state.as_deref().map(parse_state).transpose()?,
                replace_with: None,
            };
            // `--from` is a slot-list overlay onto the head, the same merge
            // `knowledge.revise` already does. A replacement of the whole record is how
            // unspecified slots vanish (slice-8 acceptance, phase-4 depends_on, Candidate
            // D's scope) and how a body that said `state completed` still landed proposed.
            let kind = session
                .ledger
                .head(&key)
                .map(|head| head.kind)
                .unwrap_or(Kind::Work);
            if let Some(body) = body_from(session, from.as_deref(), &key, kind)? {
                if let Ok(head) = session.ledger.head(&key) {
                    if edits.state.is_none() && body.mentioned.contains("state") {
                        edits.state = Some(body.record.state);
                    }
                    edits.replace_with =
                        Some(Box::new(overlay_from(head, &body.record, &body.mentioned)));
                } else {
                    edits.replace_with = Some(Box::new(body.record));
                }
            }
            let dispositions = parse_dispositions(dispositions)?;
            render(
                session,
                akr_core::ops::revise_with_dispositions(
                    &context,
                    &key,
                    mode,
                    &edits,
                    &dispositions,
                ),
            )
        }
        Command::Supersede {
            key,
            with,
            dispositions,
        } => {
            let key = parse_key(key)?;
            let replacement = with.as_deref().map(parse_key).transpose()?;
            let dispositions = parse_dispositions(dispositions)?;
            let result = match replacement {
                Some(replacement) if replacement != key => {
                    akr_core::ops::supersede_with(&context, &key, &replacement, &dispositions)
                }
                _ => akr_core::ops::supersede(&context, &key, &dispositions),
            };
            render(session, result)
        }
        Command::Complete { key, checks } => {
            let key = parse_key(key)?;
            let checks = parse_checks(checks)?;
            render(session, akr_core::ops::complete(&context, &key, &checks))
        }
        Command::Abandon {
            key,
            reason,
            dispositions,
        } => {
            let key = parse_key(key)?;
            let dispositions = parse_dispositions(dispositions)?;
            render(
                session,
                akr_core::ops::abandon(
                    &context,
                    &key,
                    reason.as_deref().unwrap_or_default(),
                    &dispositions,
                ),
            )
        }
        Command::Papercut {
            message,
            agent,
            namespace,
            about,
        } => papercut(
            session,
            &context,
            message,
            agent.as_deref(),
            namespace.as_deref(),
            about.as_deref(),
        ),
        Command::PapercutCollate {
            projects,
            namespace,
            about,
            all,
            dry_run,
        } => papercut_collate(
            session,
            &context,
            projects.as_deref(),
            namespace.as_deref(),
            about,
            *all,
            *dry_run,
        ),
        Command::EvidenceAdd {
            key,
            result,
            method,
            command,
            artifact,
            summary,
            observed_at,
        } => evidence_add(
            session,
            &context,
            &EvidenceRequest {
                key: key.clone(),
                result: result.clone(),
                method: method.clone(),
                title: None,
                command: command.clone(),
                artifact: artifact.clone(),
                summary: summary.clone(),
                observed_at: observed_at.clone(),
            },
        ),
        Command::EvidenceAddMany { from } => {
            let records = evidence_records_from_file(session, from)?;
            evidence_add_many(session, &context, &records)
        }
        _ => unreachable!("run is only called for write commands"),
    }
}

/// One evidence record, in the surface-agnostic form both surfaces reduce to.
///
/// The fields are strings because that is what a command line and a JSON payload both
/// have; turning them into a validated request is the job of [`evidence_record`], which is
/// the *one* place it happens. It used to happen twice — once here from flags, once in
/// `akr-mcp` from JSON — and the second copy is how `knowledge.evidence_add_many` came to
/// exist with no command-line equivalent and with ledger logic living in the MCP crate,
/// against both invariants in CLAUDE.md.
#[derive(Debug, Clone, Default)]
pub struct EvidenceRequest {
    /// The key for the new record.
    pub key: String,
    /// `pass`, `fail` or `inconclusive`.
    pub result: Option<String>,
    /// `manual`, `command` or `observation`.
    pub method: Option<String>,
    /// The one-line label; defaults to the summary, then to the key.
    pub title: Option<String>,
    /// The exact command, where there was one.
    pub command: Option<String>,
    /// A recorded artefact.
    pub artifact: Option<String>,
    /// What was seen.
    pub summary: Option<String>,
    /// The commit the check was run at; defaults to HEAD.
    pub observed_at: Option<String>,
}

/// Turns one [`EvidenceRequest`] into the record it describes, and the commit it names.
///
/// Evidence is created through [`akr_core::evidence::AddEvidence`] rather than through
/// `propose`, because an evidence record has required slots a blank template cannot
/// invent — a result, a method, and the commit it was observed at.
///
/// This does not check that the commit exists: a batch verifies its distinct commits once
/// rather than once per record, and doing it here would make that impossible.
///
/// # Errors
/// [`EnvError`] when a field is missing or not one of its accepted words.
pub fn evidence_record(
    session: &Session,
    context: &WriteContext,
    request: &EvidenceRequest,
) -> Result<(LogicalKey, Record, Commit), EnvError> {
    let key = parse_key(&request.key)?;
    let result = request.result.as_deref();
    let method = request.method.as_deref();
    let command = request.command.as_deref();
    let artifact = request.artifact.as_deref();
    let summary = request.summary.as_deref();
    let observed_at = request.observed_at.as_deref();
    let result = match result {
        Some("pass") => EvidenceResult::Pass,
        Some("fail") => EvidenceResult::Fail,
        Some("inconclusive") => EvidenceResult::Inconclusive,
        Some(other) => {
            return Err(EnvError::new(
                "AKR-C004",
                format!("--result: {other:?} is not `pass`, `fail` or `inconclusive`"),
            ));
        }
        None => {
            return Err(EnvError::new(
                "AKR-C003",
                "evidence add requires --result pass|fail|inconclusive",
            ));
        }
    };
    let method = match method {
        Some("manual") => akr_core::model::CheckMethod::Manual,
        Some("command") => akr_core::model::CheckMethod::Command,
        Some("observation") => akr_core::model::CheckMethod::Observation,
        Some(other) => {
            return Err(EnvError::new(
                "AKR-C004",
                format!("--method: {other:?} is not `manual`, `command` or `observation`"),
            ));
        }
        None => {
            return Err(EnvError::new(
                "AKR-C003",
                "evidence add requires --method manual|command|observation",
            ));
        }
    };

    // `--observed-at` defaults to HEAD (§6). An evidence record with no commit cannot be
    // tested for descendancy, so D-016's acceptance rule would never be satisfiable.
    let commit = match observed_at {
        Some(text) => Commit::new(text).map_err(|_| {
            EnvError::new(
                "AKR-C004",
                format!("--observed-at: {text:?} is not 40 lowercase hex digits"),
            )
            .help("AKR takes full commit hashes, never abbreviations (D-008)")
        })?,
        None => session
            .commit
            .clone()
            .ok_or_else(|| missing_commit(session, true))?,
    };
    let title = request
        .title
        .clone()
        .or_else(|| summary.map(ToOwned::to_owned))
        .unwrap_or_else(|| key.to_string());
    let mut built =
        akr_core::evidence::AddEvidence::new(key.clone(), &title, result, method, commit.clone());
    if let Some(command) = command {
        built = built.command(command);
    }
    if let Some(artifact) = artifact {
        built = built.artifact(artifact);
    }
    if let Some(summary) = summary {
        built = built.summary(summary);
    }
    if let Some(author) = &context.author {
        built.author = Some(author.clone());
    }
    Ok((key, built.to_record(), commit))
}

/// Refuses a commit the repository does not have, checking each distinct one once.
///
/// # Errors
/// `AKR-G011` naming the first commit that is not present.
pub fn ensure_commits_exist(session: &Session, commits: &[Commit]) -> Result<(), EnvError> {
    let Some(repository) = &session.repository else {
        return Ok(());
    };
    let mut seen = std::collections::BTreeSet::new();
    for commit in commits {
        if !seen.insert(commit.clone()) {
            continue;
        }
        if !repository.contains(commit) {
            return Err(EnvError::new(
                "AKR-G011",
                format!("observed_at {commit} is not present in this repository"),
            ));
        }
    }
    Ok(())
}

/// `akr evidence add`: one record, through the ordinary single-write pipeline.
///
/// # Errors
/// [`EnvError`] for a malformed request or an absent commit; a library refusal is an
/// [`Output`] with exit 1 rather than an error.
pub fn evidence_add(
    session: &Session,
    context: &WriteContext,
    request: &EvidenceRequest,
) -> Result<Output, EnvError> {
    let (key, record, commit) = evidence_record(session, context, request)?;
    ensure_commits_exist(session, &[commit])?;
    let title = record.title.clone();
    render(
        session,
        akr_core::ops::propose(context, &key, Kind::Evidence, &title, Some(record)),
    )
}

/// `akr evidence add-many` / `knowledge.evidence_add_many`: one atomic pipeline pass.
///
/// A verification run produces several evidence records at once, and writing them one at a
/// time re-derived the whole ledger per record — the friction behind
/// `akr.papercut.writing-several-evidence-and-lifecycle-records`. `propose_many` validates
/// the *resulting* ledger once, so either every record lands or none does.
///
/// Takes records rather than requests because the two surfaces arrive with different
/// things in hand: the command line parses a file that already holds evidence records,
/// while a JSON payload builds them field by field through [`evidence_record`]. They meet
/// here, which is what keeps one write pipeline under both.
///
/// # Errors
/// [`EnvError`] for an empty batch or a commit the repository does not have.
pub fn evidence_add_many(
    session: &Session,
    context: &WriteContext,
    records: &[Record],
) -> Result<Output, EnvError> {
    if records.is_empty() {
        return Err(EnvError::new(
            "AKR-C003",
            "evidence add-many needs at least one evidence record",
        ));
    }
    let commits: Vec<Commit> = records
        .iter()
        .filter_map(|record| match record.get(ContentSlot::ObservedAt) {
            Some(akr_core::model::ContentValue::Commit(commit)) => Some(commit.clone()),
            _ => None,
        })
        .collect();
    ensure_commits_exist(session, &commits)?;
    let count = records.len();
    render(session, akr_core::ops::propose_many(context, records)).map(|mut output| {
        output
            .text
            .push_str(&format!("{count} evidence records written in one pass\n"));
        output
    })
}

/// Reads a file of evidence records for `akr evidence add-many`.
///
/// The file is an ordinary AKR fragment holding one or more `evidence` records — the same
/// syntax `akr propose --from` accepts, and the same parser, so there is no second grammar
/// to keep honest. Anything that is not an evidence record is refused by name rather than
/// skipped: a batch that silently wrote four of five records would be worse than one that
/// wrote none.
///
/// # Errors
/// [`EnvError`] when the file cannot be read, does not parse, or holds a record of another
/// kind.
pub fn evidence_records_from_file(session: &Session, path: &Path) -> Result<Vec<Record>, EnvError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| EnvError::new("AKR-C042", format!("cannot read {}: {e}", path.display())))?;
    let text = with_header(&text, session);

    let mut sources = akr_core::diagnostics::SourceMap::new();
    let display_path = path.to_string_lossy().into_owned();
    let file = sources.add(&display_path, &text);
    let parsed = akr_core::syntax::parse(&text, file);
    let refuse = |message: String| {
        EnvError::new(
            "AKR-C031",
            format!("{} does not parse as evidence: {message}", path.display()),
        )
        .help(
            "the file holds one or more `record <key>/1 : evidence { ... }` blocks, each \
             with result, method and observed_at",
        )
    };
    let Some(tree) = parsed.file else {
        let first = parsed
            .diagnostics
            .first()
            .map_or_else(|| "no records".to_owned(), |d| d.message.clone());
        return Err(refuse(first));
    };
    let (ledger, lowered) = akr_core::syntax::lower::lower_all(&[(display_path, tree)]);
    if let Some(fatal) = parsed
        .diagnostics
        .iter()
        .chain(lowered.iter())
        .find(|d| d.severity == akr_core::diagnostics::Severity::Error)
    {
        return Err(refuse(fatal.message.clone()));
    }

    let records: Vec<Record> = ledger.records().to_vec();
    if let Some(other) = records.iter().find(|r| r.kind != Kind::Evidence) {
        return Err(EnvError::new(
            "AKR-C031",
            format!(
                "{} holds {}, which is a {} record; evidence add-many writes evidence only",
                path.display(),
                other.id,
                other.kind.name()
            ),
        ));
    }
    if records.is_empty() {
        return Err(EnvError::new(
            "AKR-C031",
            format!("{} holds no records", path.display()),
        ));
    }
    Ok(records)
}

/// The `akr`/`project` header a bare fragment is missing, so a caller need not repeat it.
fn with_header(text: &str, session: &Session) -> String {
    if text.trim_start().starts_with("akr ") {
        return text.to_owned();
    }
    format!("akr 0.1\nproject {}\n\n{text}", session.ledger.project.name)
}

/// `akr papercut -m <agent> "message"`, over [`akr_core::papercut`] (D-027).
///
/// The message is the whole ceremony: the key, the slug, the commit, the author and the
/// date are all filled in here, because a log that asks for more does not get written in
/// the moment.
pub fn papercut(
    session: &Session,
    context: &WriteContext,
    message: &str,
    agent: Option<&str>,
    namespace: Option<&str>,
    about: Option<&str>,
) -> Result<Output, EnvError> {
    let agent = match agent {
        Some(agent) => agent.to_owned(),
        None => {
            return Err(EnvError::new(
                "AKR-C003",
                "papercut requires -m <agent>: who hit it (a model or harness name)",
            ));
        }
    };
    let commit = session
        .commit
        .clone()
        .ok_or_else(|| missing_commit(session, false))?;
    let key = akr_core::papercut::allocate_key(&session.ledger, namespace, message)
        .map_err(|e| EnvError::new("AKR-C004", e.to_string()))?;
    let request = akr_core::papercut::LogPapercut {
        message: message.to_owned(),
        agent,
        observed_at: commit,
        created_at: Some(session.today),
        about: about.map(ToOwned::to_owned),
    };
    let record = request.to_record(key.clone());
    let title = record.title.clone();
    render(
        session,
        akr_core::ops::propose(
            context,
            &key,
            akr_core::model::Kind::Papercut,
            &title,
            Some(record),
        ),
    )
}

/// `akr papercut collate`, over [`akr_core::papercut::collate`] (D-030).
///
/// Reads the live papercut heads of every workspace under a scan directory — the
/// siblings of the workspace root, or `--projects` — and proposes one master papercut
/// record for every key not already listed in any collation's `collated` slot. The
/// sisters are read, never written. Nothing new is not a refusal: the command exits 0
/// and writes nothing.
#[allow(clippy::too_many_arguments)]
fn papercut_collate(
    session: &Session,
    context: &WriteContext,
    projects: Option<&Path>,
    namespace: Option<&str>,
    about: &[String],
    all: bool,
    dry_run: bool,
) -> Result<Output, EnvError> {
    let scan_dir = match projects {
        Some(path) => path.to_path_buf(),
        None => session
            .root
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| {
                EnvError::new("AKR-G001", "cannot find the siblings of the workspace root")
            })?,
    };
    if !scan_dir.is_dir() {
        return Err(EnvError::new(
            "AKR-C011",
            format!("{} is not a directory to scan", scan_dir.display()),
        ));
    }

    // With no filter, collate reports aimed at the namespace this master record belongs
    // to. Taking unrelated sister-project friction requires the explicit `--all`; this is
    // the distinction promised by the CLI help and D-033.
    let subjects: Vec<String> = if about.is_empty() && !all {
        vec![
            akr_core::papercut::select_namespace(&session.ledger, namespace)
                .map_err(|e| EnvError::new("AKR-C004", e.to_string()))?,
        ]
    } else {
        about.to_vec()
    };
    let subject = if all {
        akr_core::papercut::collate::Subject::Any
    } else {
        akr_core::papercut::collate::Subject::Named(subjects.clone())
    };
    // The record's own `about` names one subject (D-033). Several were asked for only
    // because one tool answers to several names, so the first is the one it is filed
    // under and the rest are spellings of it.
    let effective_about = if all {
        None
    } else {
        subjects.first().map(String::as_str)
    };
    let already = akr_core::papercut::collate::already_collated(&session.ledger);
    let collate =
        akr_core::papercut::collate::collect(&scan_dir, &session.root, &already, &subject);

    // Whatever the filter left behind is reported, never dropped in silence: a collation
    // that looked complete while ignoring two thirds of what it read would be worse than
    // none at all.
    let mut trailer = String::new();
    if collate.filtered_out > 0 {
        trailer.push_str(&format!(
            "  {} left behind by the subject filter:\n",
            collate.filtered_out
        ));
        // Broken down, not totalled. A bare count tells a reader they have missed
        // something and nothing about what to do next; the breakdown is what turns the
        // leftovers into a decision — which of these spellings are also this project, and
        // which belong to whoever owns that tool.
        let mut left: Vec<&akr_core::papercut::collate::SubjectTally> = collate
            .subjects_seen
            .iter()
            .filter(|tally| tally.left_behind > 0)
            .collect();
        left.sort_by(|a, b| {
            b.left_behind
                .cmp(&a.left_behind)
                .then(a.subject.cmp(&b.subject))
        });
        for tally in &left {
            trailer.push_str(&format!(
                "    {:>4}  {}\n",
                tally.left_behind,
                tally.subject.as_deref().unwrap_or("(no subject)")
            ));
        }
        if let Some(first) = left.first().and_then(|t| t.subject.as_deref()) {
            trailer.push_str(&format!(
                "  add one with `--about {}`, or take everything with `--all`\n",
                shell_quote(first)
            ));
        }
    }
    if !collate.skipped.is_empty() {
        trailer.push_str(&format!(
            "  {} workspace{} did not load and were skipped\n",
            collate.skipped.len(),
            if collate.skipped.len() == 1 { "" } else { "s" }
        ));
    }

    if collate.entries.is_empty() {
        let mut text = format!("scanned {} — nothing new to collate\n", collate.source);
        text.push_str(&trailer);
        return Ok(Output::plain(
            text,
            Value::object(vec![
                ("scanned", Value::integer(collate.projects.len() as i64)),
                ("collated", Value::integer(0)),
                ("filtered_out", Value::integer(collate.filtered_out as i64)),
                ("subjects_seen", subjects_value(&collate.subjects_seen)),
            ]),
        ));
    }

    let mut entry_projects: Vec<String> = collate
        .entries
        .iter()
        .map(|entry| entry.project.clone())
        .collect();
    entry_projects.sort();
    entry_projects.dedup();

    // A collation is a write whose shape you cannot see until it has happened: the scan
    // is global, the filter is a guess at how other people spelled a subject, and the
    // result is one record holding everything it matched. Getting that wrong means a
    // revert, which is how the `--all` run that swept in 155 unrelated papercuts ended.
    // So there is a way to look first.
    if dry_run {
        let mut text = format!(
            "would collate {} papercut{} from {} into this ledger — nothing written\n",
            collate.entries.len(),
            if collate.entries.len() == 1 { "" } else { "s" },
            entry_projects.join(", ")
        );
        for entry in &collate.entries {
            text.push_str(&format!(
                "  {} @{}{}\n",
                entry.project,
                entry.key,
                entry
                    .about
                    .as_deref()
                    .map_or_else(String::new, |about| format!(" [about {about}]"))
            ));
        }
        text.push_str(&trailer);
        return Ok(Output::plain(
            text,
            Value::object(vec![
                ("scanned", Value::integer(collate.projects.len() as i64)),
                (
                    "would_collate",
                    Value::integer(collate.entries.len() as i64),
                ),
                ("written", Value::bool(false)),
                ("filtered_out", Value::integer(collate.filtered_out as i64)),
                ("subjects_seen", subjects_value(&collate.subjects_seen)),
            ]),
        ));
    }

    let commit = session
        .commit
        .clone()
        .ok_or_else(|| missing_commit(session, false))?;
    let message = format!(
        "collated {} papercuts from {}",
        collate.entries.len(),
        entry_projects.join(", ")
    );
    let key = akr_core::papercut::allocate_key(&session.ledger, namespace, &message)
        .map_err(|e| EnvError::new("AKR-C004", e.to_string()))?;
    let request = akr_core::papercut::collate::CollateRequest {
        source: collate.source,
        projects: entry_projects,
        entries: collate.entries,
        observed_at: commit,
        created_at: session.today,
        author: context.author.clone(),
        about: effective_about.map(ToOwned::to_owned),
    };
    let record = request.to_record(key.clone());
    let title = record.title.clone();
    let mut output = render(
        session,
        akr_core::ops::propose(
            context,
            &key,
            akr_core::model::Kind::Papercut,
            &title,
            Some(record),
        ),
    )?;
    // The trailer goes on the successful path too, not only the empty one: the run that
    // absorbed something is exactly the run where "and here is what I did not absorb"
    // matters, because it is the one a reader will take as complete.
    output.text.push_str(&trailer);
    Ok(output)
}

/// The `subjects_seen` tallies, for the machine-readable result.
fn subjects_value(tallies: &[akr_core::papercut::collate::SubjectTally]) -> Value {
    Value::array(
        tallies
            .iter()
            .map(|tally| {
                Value::object(vec![
                    (
                        "subject",
                        tally
                            .subject
                            .as_ref()
                            .map_or(Value::Null, |s| Value::string(s.clone())),
                    ),
                    ("total", Value::integer(tally.total as i64)),
                    ("left_behind", Value::integer(tally.left_behind as i64)),
                ])
            })
            .collect(),
    )
}

/// Quotes a subject for the copyable `--about` hint, when it needs it.
fn shell_quote(subject: &str) -> String {
    if subject
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        subject.to_owned()
    } else {
        format!("\"{}\"", subject.replace('"', "\\\""))
    }
}

// -------------------------------------------------------------------------------------
// argument conversion
// -------------------------------------------------------------------------------------

/// The write context both surfaces use, author and strictness included.
///
/// Public because `akr-mcp` needs the *same* one. It used to build its own, which set no
/// author at all — so the identical record written over MCP and from the command line
/// differed by an `author` line, and nothing noticed until the two were compared byte for
/// byte.
pub fn context_of(session: &Session) -> WriteContext {
    let mut context = WriteContext::new(&session.akr_dir);
    context.strict = session.global.profile == crate::args::Profile::Strict;
    // The author is whatever git thinks, because AKR is not an identity system (D-005)
    // and the one honest source of a name in a repository is the repository's own config.
    if let Some(author) = git_author(session) {
        context = context.with_author(author);
    }
    context
}

/// The author to record, from `git config user.name`.
///
/// AKR is not an identity system (D-005): `author` is free text, and the one honest source
/// of a name inside a repository is the repository's own configuration. Absent is fine.
fn git_author(session: &Session) -> Option<String> {
    let output = akr_core::git::command()
        .args(["config", "user.name"])
        .current_dir(&session.root)
        .output()
        .ok()?;
    let name = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!name.is_empty()).then_some(name)
}

pub(crate) fn parse_key(text: &str) -> Result<LogicalKey, EnvError> {
    let trimmed = text.strip_prefix('@').unwrap_or(text);
    LogicalKey::parse(trimmed).map_err(|e| {
        EnvError::new("AKR-C004", format!("{text:?} is not a key: {e}")).help(
            "keys are dot-delimited — namespace.topic.slug — and the first segment must \
             be a namespace declared in .akr/project.akr",
        )
    })
}

fn parse_kind(text: &str) -> Result<Kind, EnvError> {
    Kind::from_name(text).ok_or_else(|| {
        EnvError::new("AKR-C004", format!("--kind: {text:?} is not a record kind")).help(
            "one of term, requirement, constraint, policy, decision, observation, evidence, \
             assessment, milestone, work, track, question",
        )
    })
}

fn parse_state(text: &str) -> Result<State, EnvError> {
    State::from_name(text)
        .ok_or_else(|| EnvError::new("AKR-C004", format!("--state: {text:?} is not a state")))
}

/// `child=outcome[:into][,note]` — the form §6 shows.
fn parse_dispositions(raw: &[String]) -> Result<Vec<DispositionRequest>, EnvError> {
    raw.iter().map(|text| parse_disposition(text)).collect()
}

fn parse_disposition(text: &str) -> Result<DispositionRequest, EnvError> {
    let (child, rest) = text.split_once('=').ok_or_else(|| {
        EnvError::new(
            "AKR-C004",
            format!("--disposition {text:?}: expected <child>=<outcome>[:<into>]"),
        )
    })?;
    let (outcome, into) = match rest.split_once(':') {
        Some((outcome, into)) => (outcome, Some(into)),
        None => (rest, None),
    };
    let outcome = DispositionOutcome::ALL
        .iter()
        .copied()
        .find(|candidate| candidate.name() == outcome)
        .ok_or_else(|| {
            EnvError::new(
                "AKR-C004",
                format!("--disposition: {outcome:?} is not a disposition outcome"),
            )
            .help(format!(
                "one of {}",
                DispositionOutcome::ALL
                    .iter()
                    .map(|o| o.name())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?;
    Ok(DispositionRequest {
        child: parse_key(child)?,
        outcome,
        into: into.map(parse_key).transpose()?,
        note: None,
    })
}

/// `check=@evidence` — the form `akr complete --check` takes.
fn parse_checks(raw: &[String]) -> Result<Vec<(String, Reference)>, EnvError> {
    raw.iter()
        .map(|text| {
            let (id, reference) = text.split_once('=').ok_or_else(|| {
                EnvError::new(
                    "AKR-C004",
                    format!("--check {text:?}: expected <check-id>=<evidence-reference>"),
                )
            })?;
            let reference = Reference::parse(reference).map_err(|e| {
                EnvError::new(
                    "AKR-C004",
                    format!("--check: {reference:?} is not a reference: {e}"),
                )
            })?;
            Ok((id.to_owned(), reference))
        })
        .collect()
}

/// A `--from` file: the lowered record, and the slot names the file actually wrote.
///
/// The names are the merge mask. A lowered record cannot tell "acceptance was omitted"
/// from "acceptance is empty", and treating the two as one is how a partial fragment
/// dropped unspecified slots.
struct FromBody {
    record: Record,
    mentioned: BTreeSet<String>,
}

/// The record body a `--from` file holds, if one was given.
///
/// There is deliberately no `--edit`: it is refused at parse time by `args::refuse_edit`,
/// which carries the reasoning.
fn body_from(
    session: &Session,
    from: Option<&Path>,
    key: &LogicalKey,
    kind: Kind,
) -> Result<Option<FromBody>, EnvError> {
    let Some(path) = from else {
        return Ok(None);
    };
    let text = std::fs::read_to_string(path)
        .map_err(|e| EnvError::new("AKR-C042", format!("cannot read {}: {e}", path.display())))?;
    // The command already knows the project, the key and the kind — it is being told them
    // on the command line — so demanding that the file repeat all three was ceremony that
    // sent people to `akr fmt` to find out what a file header looks like. A file that
    // brings its own header is still taken verbatim.
    let text = complete_body(&text, session, key, kind);

    let mut sources = akr_core::diagnostics::SourceMap::new();
    let display_path = path.to_string_lossy().into_owned();
    let file = sources.add(&display_path, &text);
    let parsed = akr_core::syntax::parse(&text, file);
    let refuse = |message: String| {
        EnvError::new(
            "AKR-C031",
            format!("{} does not parse as a record: {message}", path.display()),
        )
        .help(
            "pass an AKR slot-list (`intent \"\"\" ... \"\"\"` for work, `statement \"\"\" \
             ... \"\"\"` for most other kinds), not YAML or Markdown",
        )
    };
    let Some(tree) = parsed.file else {
        let first = parsed
            .diagnostics
            .first()
            .map_or_else(|| "no records".to_owned(), |d| d.message.clone());
        return Err(refuse(first));
    };
    let mentioned = mentioned_slots(&tree);
    let (ledger, lowered) = akr_core::syntax::lower::lower_all(&[(display_path, tree)]);
    if let Some(fatal) = parsed
        .diagnostics
        .iter()
        .chain(lowered.iter())
        .find(|d| d.severity == akr_core::diagnostics::Severity::Error)
    {
        return Err(refuse(fatal.message.clone()));
    }
    let record = ledger
        .records()
        .iter()
        .find(|r| r.id.key == *key)
        .or_else(|| ledger.records().first())
        .cloned()
        .ok_or_else(|| EnvError::new("AKR-C031", format!("{} holds no record", path.display())))?;
    Ok(Some(FromBody { record, mentioned }))
}

fn mentioned_slots(tree: &akr_core::syntax::cst::File) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for item in &tree.items {
        if let akr_core::syntax::cst::Item::Record(record) = item {
            for body in &record.body {
                names.insert(body.name().to_owned());
            }
        }
    }
    names
}

/// Overlay the slots a `--from` file named onto the live head.
///
/// Unmentioned heading slots, content slots, relations, acceptance, claims and
/// provenance stay. A named slot is replaced, including with emptiness. This is the
/// CLI form of `akr_mcp::tools::merged`.
fn overlay_from(head: &Record, fragment: &Record, mentioned: &BTreeSet<String>) -> Record {
    let mut record = head.clone();
    if mentioned.contains("title") {
        record.title.clone_from(&fragment.title);
    }
    if mentioned.contains("state") {
        record.state = fragment.state;
    }
    if mentioned.contains("scope") {
        record.scope.clone_from(&fragment.scope);
    }
    if mentioned.contains("topic") {
        record.topic.clone_from(&fragment.topic);
    }
    if mentioned.contains("author") {
        record.author.clone_from(&fragment.author);
    }
    if mentioned.contains("created_at") {
        record.created_at = fragment.created_at;
    }
    if mentioned.contains("acknowledged") {
        record.acknowledged = fragment.acknowledged;
    }
    if mentioned.contains("acceptance") || mentioned.contains("check") {
        record.acceptance.clone_from(&fragment.acceptance);
    }
    if mentioned.contains("claim") || mentioned.contains("claims") {
        record.claims.clone_from(&fragment.claims);
    }
    if mentioned.contains("retired_claims") {
        record.retired_claims.clone_from(&fragment.retired_claims);
    }
    if mentioned.contains("source") || mentioned.contains("sources") {
        record.sources.clone_from(&fragment.sources);
    }
    if mentioned.contains("disposition") {
        record.dispositions.clone_from(&fragment.dispositions);
    }
    for name in mentioned {
        if let Some(slot) = ContentSlot::from_name(name) {
            match fragment.content.get(&slot) {
                Some(value) => {
                    record.content.insert(slot, value.clone());
                }
                None => {
                    record.content.remove(&slot);
                }
            }
        }
        if let Some(relation) = Relation::from_name(name) {
            match fragment.relations.get(&relation) {
                Some(targets) => {
                    record.relations.insert(relation, targets.clone());
                }
                None => {
                    record.relations.remove(&relation);
                }
            }
        }
    }
    record
}

/// Wraps a partial `--from` body in whatever it is missing.
///
/// Three shapes are accepted, cheapest first:
///
/// * a whole file, with its `akr <version>` header — used as it stands;
/// * a bare `record … { … }` block — the header is prepended;
/// * a bare slot list — the header *and* the record line are supplied from the key and
///   kind the command was given.
///
/// The last case is the one worth having. Writing three lines of slots and being told the
/// file "does not parse as a record" is a poor trade for information the caller has
/// already typed.
fn complete_body(text: &str, session: &Session, key: &LogicalKey, kind: Kind) -> String {
    let first = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'));
    let header = format!(
        "akr {}\nproject {}\n\n",
        crate::session::GRAMMAR_VERSION,
        session.ledger.project.name
    );
    match first {
        Some(line) if line.starts_with("akr ") || line.starts_with("akr-lock ") => text.to_owned(),
        Some(line) if line.starts_with("record ") => format!("{header}{text}"),
        // A slot list. The revision number is 1 because a template is a body, not a
        // history: `revise` renumbers it against the head it is replacing.
        Some(_) => {
            let indented: String = text
                .lines()
                .map(|line| {
                    if line.trim().is_empty() {
                        String::new()
                    } else {
                        format!("    {line}")
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            format!(
                "{header}record {key}/1 : {} {{\n{indented}\n}}\n",
                kind.name()
            )
        }
        None => text.to_owned(),
    }
}

// -------------------------------------------------------------------------------------
// rendering
// -------------------------------------------------------------------------------------

pub(crate) fn render(session: &Session, result: WriteResult) -> Result<Output, EnvError> {
    match result {
        Ok(applied) => Ok(render_applied(session, &applied)),
        Err(refused) => Ok(render_refused(session, &refused)),
    }
}

fn render_applied(session: &Session, applied: &Applied) -> Output {
    let mut text = String::new();
    for change in &applied.changes {
        text.push_str(&change_line(change));
    }
    for file in &applied.files {
        text.push_str(&format!("wrote {}\n", display(session, file)));
    }
    if applied.lock_stale {
        // Every write invalidates the lock's seals and its source-graph hash, and no
        // write operation may invent a build (D-014). Saying so is the difference between
        // an expected `AKR-R052` on the next check and a confusing one.
        text.push_str("akr.lock is now stale; run `akr build` to refresh it\n");
    }
    for diagnostic in &applied.diagnostics {
        text.push_str(&akr_core::diagnostics::render(diagnostic, &session.sources));
    }
    for note in &applied.notes {
        text.push_str(note);
        text.push('\n');
    }

    Output {
        text,
        result: Value::object(vec![
            ("operation", Value::string(applied.operation.name())),
            (
                "changes",
                Value::array(applied.changes.iter().map(change_json).collect()),
            ),
            (
                "files",
                Value::array(
                    applied
                        .files
                        .iter()
                        .map(|f| Value::string(display(session, f)))
                        .collect(),
                ),
            ),
            ("lock_stale", Value::bool(applied.lock_stale)),
            (
                "notes",
                Value::array(applied.notes.iter().cloned().map(Value::string).collect()),
            ),
        ]),
        diagnostics: applied.diagnostics.clone(),
        exit: Exit::Ok,
        commit: session.commit.as_ref().map(|c| c.as_str().to_owned()),
        source_graph: session.source_graph(),
    }
}

fn change_line(change: &Change) -> String {
    match &change.kind {
        ChangeKind::Created => format!("created {}\n", change.id),
        ChangeKind::Edited => format!("edited {}\n", change.id),
        ChangeKind::StateChanged { from, to } => {
            format!("{} {from} -> {to}\n", change.id)
        }
    }
}

fn change_json(change: &Change) -> Value {
    let (kind, from, to) = match &change.kind {
        ChangeKind::Created => ("created", None, None),
        ChangeKind::Edited => ("edited", None, None),
        ChangeKind::StateChanged { from, to } => {
            ("state_changed", Some(from.name()), Some(to.name()))
        }
    };
    let mut fields = vec![
        ("key", Value::string(change.id.key.to_string())),
        ("rev", Value::integer(i64::from(change.id.revision))),
        ("change", Value::string(kind)),
        ("file", Value::string(change.file.to_string_lossy())),
    ];
    if let (Some(from), Some(to)) = (from, to) {
        fields.push(("from", Value::string(from)));
        fields.push(("to", Value::string(to)));
    }
    Value::object(fields)
}

/// A refusal, rendered from [`Refused`]'s structured fields.
fn render_refused(session: &Session, refused: &Refused) -> Output {
    // The refusal line first, then the structure, then the help — §6's sample order. The
    // help line names the fix, and a fix reads better after the thing it fixes than before
    // it, so the diagnostic is rendered without its help and the help is added at the end.
    let mut headline = refused.diagnostic();
    headline.help = None;
    let mut text = akr_core::diagnostics::render(&headline, &session.sources);

    if !refused.unfinished_children.is_empty() {
        let width = refused
            .unfinished_children
            .iter()
            .map(|child| child.key.to_string().len() + 1)
            .max()
            .map_or(0, |longest| longest + 3);
        text.push_str(&format!(
            "  {} unfinished {}:\n",
            refused.unfinished_children.len(),
            if refused.unfinished_children.len() == 1 {
                "child"
            } else {
                "children"
            }
        ));
        for child in &refused.unfinished_children {
            text.push_str(&format!(
                "    {:<width$}{}\n",
                format!("@{}", child.key),
                child.state.name()
            ));
        }
    }
    if !refused.unsatisfied_checks.is_empty() {
        let width = refused
            .unsatisfied_checks
            .iter()
            .map(|check| check.id.len())
            .max()
            .map_or(0, |longest| longest + 3);
        text.push_str(&format!(
            "  {} unsatisfied {}:\n",
            refused.unsatisfied_checks.len(),
            if refused.unsatisfied_checks.len() == 1 {
                "check"
            } else {
                "checks"
            }
        ));
        for check in &refused.unsatisfied_checks {
            text.push_str(&format!("    {:<width$}{}\n", check.id, check.reason));
        }
    }
    // The validation diagnostics are the information only when there is no structure: a
    // refusal that already listed its unfinished children would otherwise say the same
    // thing twice, in two voices.
    if refused.unfinished_children.is_empty() && refused.unsatisfied_checks.is_empty() {
        for diagnostic in &refused.diagnostics {
            text.push_str(&akr_core::diagnostics::render(diagnostic, &session.sources));
        }
    }
    if let Some(help) = &refused.help {
        text.push_str(&format!("help: {help}\n"));
    }
    if !refused.unfinished_children.is_empty() {
        for child in &refused.unfinished_children {
            text.push_str(&format!(
                "  --disposition {}=intentionally_dropped\n",
                child.key
            ));
        }
    }
    text.push_str("nothing written\n");

    let mut diagnostics = vec![refused.diagnostic()];
    diagnostics.extend(refused.diagnostics.clone());
    Output {
        text,
        result: Value::object(vec![
            ("operation", Value::string(refused.operation.name())),
            ("refused", Value::bool(true)),
            ("code", Value::string(refused.code.as_str())),
            (
                "unfinished_children",
                Value::array(
                    refused
                        .unfinished_children
                        .iter()
                        .map(|child| {
                            Value::object(vec![
                                ("key", Value::string(child.key.to_string())),
                                ("state", Value::string(child.state.name())),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "unsatisfied_checks",
                Value::array(
                    refused
                        .unsatisfied_checks
                        .iter()
                        .map(|check| {
                            Value::object(vec![
                                ("id", Value::string(check.id.clone())),
                                ("reason", Value::string(check.reason.clone())),
                            ])
                        })
                        .collect(),
                ),
            ),
        ]),
        diagnostics,
        exit: Exit::Diagnostics,
        commit: session.commit.as_ref().map(|c| c.as_str().to_owned()),
        source_graph: session.source_graph(),
    }
}

fn display(session: &Session, path: &Path) -> String {
    let full = session.akr_dir.join(path);
    full.strip_prefix(&session.root)
        .unwrap_or(&full)
        .to_string_lossy()
        .replace('\\', "/")
}

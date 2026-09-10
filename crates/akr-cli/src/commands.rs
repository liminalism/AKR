//! The read and build commands of `docs/07-cli.md` §6.
//!
//! Each returns its own text, its own JSON `result` object, and an [`Exit`]. Printing and
//! the envelope belong to `main`, so a command never has to know which form was asked for
//! until the last moment.

use crate::args::{Command, Profile};
use crate::session::{
    EnvError, Exit, GRAMMAR_VERSION, Session, TOOL_VERSION, VOCABULARY_VERSION, diagnostic_json,
    is_fatal, report,
};
use akr_core::context::{Request, assemble, render_json, render_text};
use akr_core::diagnostics::{Diagnostic, RuleId, Subject};
use akr_core::json::Value;
use akr_core::model::{Date, Kind, Reference, Relation, RevisionId};
use akr_core::render::{View, check_views_current, render, write_views};
use std::collections::BTreeSet;
use std::path::Path;

/// What a command produced.
pub struct Output {
    /// The human-readable form.
    pub text: String,
    /// The `result` object of the JSON envelope.
    pub result: Value,
    /// Diagnostics, already span-attached.
    pub diagnostics: Vec<Diagnostic>,
    /// The process exit status.
    pub exit: Exit,
    /// The commit the build resolved against, for the envelope.
    pub commit: Option<String>,
    /// The source-graph hash of the inputs, for the envelope.
    pub source_graph: String,
}

impl Output {
    fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            result: Value::Object(Vec::new()),
            diagnostics: Vec::new(),
            exit: Exit::Ok,
            commit: None,
            source_graph: String::new(),
        }
    }

    /// Text plus a JSON `result`, for sibling modules.
    pub(crate) fn plain(text: impl Into<String>, result: Value) -> Self {
        Self::text(text).with_result(result)
    }

    fn with_result(mut self, result: Value) -> Self {
        self.result = result;
        self
    }

    pub(crate) fn with_diagnostics(mut self, diagnostics: Vec<Diagnostic>, exit: Exit) -> Self {
        self.diagnostics = diagnostics;
        self.exit = exit;
        self
    }
}

/// Runs a command that needs no workspace.
///
/// # Errors
/// Never; the signature matches [`run`] so `main` can treat them alike.
pub fn run_standalone(command: &Command) -> Option<Result<Output, EnvError>> {
    match command {
        Command::Help => Some(Ok(Output::text(crate::args::help()))),
        Command::HelpFor { name } => Some(Ok(Output::text(
            crate::args::help_for(name).unwrap_or_else(crate::args::help),
        ))),
        Command::Version => Some(Ok(Output::text(format!("akr {TOOL_VERSION}\n")))),
        Command::Explain { subject } => Some(Ok(explain(subject))),
        Command::Init {
            project,
            namespaces,
        } => Some(crate::init::run(project.as_deref(), namespaces)),
        _ => None,
    }
}

/// Runs a command against a loaded workspace.
///
/// # Errors
/// [`EnvError`] when the environment is unusable.
pub fn run(session: &mut Session, command: &Command) -> Result<Output, EnvError> {
    // A standalone command answers from the vocabulary tables and has no arm in
    // `dispatch`, so a caller that holds a session and routes one here used to reach an
    // `unreachable!` rather than the answer. Answering it is always correct: holding a
    // session does not change what `explain` or `--version` says.
    if let Some(answer) = run_standalone(command) {
        return answer;
    }
    if command.needs_git_facts() {
        session.ensure_git_facts()?;
    }
    // The envelope's `commit` and `source_graph_hash` are properties of the build, not of
    // any one command, so they are filled in once here rather than by each command that
    // happens to remember.
    let mut output = dispatch(session, command)?;
    output.commit = session
        .inputs
        .commit
        .as_ref()
        .map(|c| c.as_str().to_owned());
    output.source_graph = session.source_graph();
    Ok(output)
}

fn dispatch(session: &mut Session, command: &Command) -> Result<Output, EnvError> {
    match command {
        Command::Check {
            scratch_clean,
            review_clean,
            views_current,
        } => check(session, *scratch_clean, *review_clean, *views_current),
        Command::HandoffCapsule {
            refresh,
            boundaries,
        } => crate::handoff::ops::capsule_show(session, *refresh, boundaries),
        Command::HandoffSessionBegin(request) => {
            crate::handoff::ops::session_begin(session, request)
        }
        Command::HandoffSessionShow => crate::handoff::ops::session_show(session),
        Command::HandoffSessionEnd => crate::handoff::ops::session_end(session),
        Command::HandoffCreate(request) => crate::handoff::ops::create(session, request),
        Command::HandoffList => crate::handoff::ops::list(session),
        Command::HandoffOpen { id, reveal } => crate::handoff::ops::open(session, id, *reveal),
        Command::HandoffExpand { id, section } => crate::handoff::ops::expand(session, id, section),
        Command::HandoffReveal { id } => crate::handoff::ops::reveal(session, id),
        Command::HandoffVerify { id } => crate::handoff::ops::verify(session, id),
        Command::HandoffDiscard { id } => crate::handoff::ops::discard(session, id),
        Command::HandoffResult(request) => crate::handoff::ops::file_result(session, request),
        Command::HandoffResults { packet } => {
            crate::handoff::ops::results(session, packet.as_deref())
        }
        Command::HandoffCoverage => crate::handoff::ops::coverage(session),
        Command::ScratchList => scratch_list(session),
        Command::ScratchPrune {
            older_than,
            dry_run,
        } => scratch_prune(session, *older_than, *dry_run),
        Command::ScratchKeep {
            name,
            reason,
            forget,
        } => scratch_keep(session, name, reason.as_deref(), *forget),
        Command::Build { check } => build(session, *check),
        Command::Fmt { check, paths } => crate::fmt::run(session, *check, paths),
        Command::View { name } => view(session, name),
        Command::Get {
            reference,
            history,
            relations,
            detail,
        } => get(session, reference, *history, *relations, *detail),
        Command::WhyCurrent { reference } => why_current(session, reference),
        Command::Impact {
            reference,
            git_diff,
        } => impact(session, reference.as_deref(), git_diff.as_deref()),
        Command::ReviewQueue {
            stale_only,
            at_risk_only,
            kinds,
        } => review_queue(session, *stale_only, *at_risk_only, kinds),
        Command::Lock { check } => lock(session, *check),
        Command::Context {
            goal,
            paths,
            budget,
        } => {
            let paths = normalize_query_paths(paths, &session.root)?;
            context(session, goal, &paths, *budget)
        }
        Command::Start {
            task,
            paths,
            budget,
        } => {
            let paths = normalize_query_paths(paths, &session.root)?;
            start(session, task, &paths, *budget)
        }
        Command::Search {
            query,
            raw_fts,
            kinds,
            states,
            limit,
        } => search(session, query, *raw_fts, kinds, states, *limit),
        Command::Import {
            path,
            namespace,
            tracking,
            dry_run,
        } => crate::import::run(
            session,
            path,
            namespace.as_deref(),
            tracking.as_deref(),
            *dry_run,
        ),
        Command::IngestPreview {
            path,
            source_kind,
            tables,
        } => crate::ingest::preview(session, path, source_kind, *tables),
        Command::IngestStart {
            path,
            source_kind,
            tables,
        } => crate::ingest::start(session, path, source_kind, *tables),
        Command::IngestShow {
            ingest_id,
            pending_only,
            limit,
        } => crate::ingest::show(session, ingest_id, *pending_only, *limit),
        Command::IngestMark {
            ingest_id,
            candidate_id,
            disposition,
            basis,
            target,
            promote_kind,
            promote_target,
            promote_attach,
            relations,
            note,
            base_version,
        } => crate::ingest::mark(
            session,
            ingest_id,
            candidate_id,
            disposition,
            basis,
            target.as_deref(),
            promote_kind,
            promote_target.as_ref(),
            *promote_attach,
            relations,
            note.as_deref(),
            *base_version,
        ),
        Command::IngestApply {
            ingest_id,
            base_version,
            dry_run,
        } => crate::ingest::apply(session, ingest_id, *base_version, *dry_run),
        Command::IngestClose {
            ingest_id,
            base_version,
        } => crate::ingest::close(session, ingest_id, *base_version),
        Command::SourceAdd {
            path,
            id,
            title,
            origin,
            observed_at,
            scope,
        } => crate::source::add(
            session,
            path,
            id.as_deref(),
            title.as_deref(),
            origin.as_deref(),
            observed_at.as_deref(),
            scope.as_deref(),
        ),
        Command::SourceList { all } => crate::source::list(session, *all),
        Command::SourceGet {
            id,
            whole,
            lines,
            section,
        } => crate::source::get(session, id, *whole, lines.as_deref(), section.as_deref()),
        #[cfg(feature = "fts5")]
        Command::SourceGetChunk { chunk, neighbors } => {
            crate::source::get_chunk(session, chunk, *neighbors)
        }
        #[cfg(feature = "fts5")]
        Command::SourceSearch {
            query,
            mode,
            documents,
            all_versions,
            limit,
        } => {
            let mode = match mode.as_str() {
                "literal" => akr_core::store::QueryMode::Literal,
                "fts" => akr_core::store::QueryMode::Fts,
                _ => akr_core::store::QueryMode::Text,
            };
            crate::source::search(session, query, mode, documents, *all_versions, *limit)
        }
        // `akr search` degrades the same way when the binary carries no FTS5, so the
        // source surface degrades with it rather than disappearing silently.
        #[cfg(not(feature = "fts5"))]
        Command::SourceGetChunk { .. } | Command::SourceSearch { .. } => Err(EnvError::new(
            "AKR-I022",
            "source search requires a full-text index; this binary was built without FTS5",
        )),
        Command::DiffStaged => crate::change::diff_staged(session),
        Command::ChangeBegin {
            kind,
            summary,
            scope,
            primary,
            related,
            note,
            untracked_reason,
        } => crate::change::begin(
            session,
            kind,
            summary,
            scope.as_deref(),
            primary.as_deref(),
            related,
            note.as_deref(),
            untracked_reason.as_deref(),
        ),
        Command::ChangeShow => crate::change::show(session),
        Command::ChangeAbort => crate::change::abort(session),
        Command::ChangePrepare { write } => crate::change::prepare(session, *write),
        Command::GitMessage => crate::change::message(session),
        Command::GitCommit { message } => crate::change::commit(session, message.as_deref()),
        Command::GitLog { reference } => crate::change::log(session, reference),
        Command::GitInstallHooks => crate::change::install_hooks(session),
        Command::GitHook { name } => crate::change::git_hook(session, name),
        Command::SourceVerify => crate::source::verify(session),
        Command::SourceSupersede {
            old_id,
            new_path,
            new_id,
        } => crate::source::supersede(session, old_id, new_path, new_id.as_deref()),
        Command::SourceStatus { id } => crate::source::status(session, id),
        Command::SourceDependents { id } => crate::source::dependents(session, id),
        Command::SourceFinalize {
            id,
            retain,
            context,
            remove_file,
            dry_run,
        } => crate::source::finalize(session, id, retain, context, *remove_file, *dry_run),
        Command::Propose { .. }
        | Command::Revise { .. }
        | Command::Supersede { .. }
        | Command::Complete { .. }
        | Command::Abandon { .. }
        | Command::Papercut { .. }
        | Command::PapercutCollate { .. }
        | Command::EvidenceAdd { .. }
        | Command::EvidenceAddMany { .. } => crate::write::run(session, command),
        Command::Help
        | Command::HelpFor { .. }
        | Command::Version
        | Command::Explain { .. }
        | Command::Init { .. } => {
            unreachable!("handled by run_standalone")
        }
    }
}

/// Normalizes `--paths` / MCP `paths` query arguments (`akr context`, `akr start`, and their
/// `knowledge.*` MCP equivalents, which are the same function called through
/// [`commands::run`](run)): native separators become `/`, and an absolute path inside the
/// repository becomes repo-root-relative. One call site for both surfaces, ahead of the one
/// place D-008 validation happens ([`akr_core::context::assemble`]), so the two never
/// disagree about what a `--paths`/`paths` argument means.
///
/// # Errors
/// [`EnvError`] `AKR-X013` when an absolute path does not lie inside `root`.
fn normalize_query_paths(
    paths: &[akr_core::model::Glob],
    root: &Path,
) -> Result<Vec<akr_core::model::Glob>, EnvError> {
    paths
        .iter()
        .map(|glob| {
            akr_core::model::normalize_query_path(glob.as_str(), root).map_err(|error| {
                EnvError::new("AKR-X013", format!("--paths {error}")).help(
                    "paths are matched against the repository; pass one inside it, relative or absolute",
                )
            })
        })
        .collect()
}

// -------------------------------------------------------------------------------------
// check
// -------------------------------------------------------------------------------------

// -------------------------------------------------------------------------------------
// `akr scratch` (D-036): the one directory nothing else empties
// -------------------------------------------------------------------------------------

/// The default age, in days, at which an unkept scratch entry may go.
const SCRATCH_DEFAULT_AGE: u64 = 14;

/// The build-fact line `akr check` prints for scratch.
///
/// A line rather than a diagnostic, and printed even when there is nothing to say, because
/// the whole problem is that scratch is invisible: an agent that never sees the directory
/// mentioned has no reason to believe it persists.
/// The build-fact line for scope paths git cannot answer for.
///
/// Not a diagnostic. An ignored path or a submodule may be exactly what the author
/// meant, so this reports rather than fails — but it does report, because staying silent
/// is what let 103 of them accumulate unseen across fifteen ledgers (V-102, D-024).
/// Whether any `.akr/` record file has uncommitted changes.
///
/// The discriminator between "this session has unsaved work" and "this repository ships
/// views that do not match its ledger". Best effort: when git cannot answer, the answer is
/// "committed", which keeps the stricter reading rather than silently softening the gate.
fn ledger_is_uncommitted(session: &Session) -> bool {
    session.repository.as_ref().is_some_and(|repository| {
        repository.working_tree_changes().is_ok_and(|changed| {
            changed
                .iter()
                .any(|path| path.starts_with(".akr/") && path.ends_with(".akr"))
        })
    })
}

/// The one-line view-currency fact for the default check's build-facts block.
///
/// Silent when the views are current, so the ordinary case stays quiet; otherwise it says
/// how many views are stale and which kind of stale, because the fix differs: a `tool:`
/// stamp needs a rebuild whenever convenient, a content difference needs one now.
fn views_fact(diagnostics: &[Diagnostic]) -> String {
    let stamp = diagnostics
        .iter()
        .filter(|d| d.code.as_str() == "AKR-E015")
        .count();
    let stale = diagnostics.len() - stamp;
    match (stale, stamp) {
        (0, 0) => String::new(),
        (0, n) => format!("    {n} view(s) carry an older `tool:` stamp — run `akr build`\n"),
        (n, 0) => format!("    {n} view(s) do not match the ledger — run `akr build`\n"),
        (n, m) => format!(
            "    {n} view(s) do not match the ledger, {m} carry an older `tool:` stamp \
             — run `akr build`\n"
        ),
    }
}

fn unverifiable_scope_fact(scopes: &[akr_core::freshness::UnverifiableScope]) -> String {
    use akr_core::freshness::UnverifiableReason;
    if scopes.is_empty() {
        return String::new();
    }
    // Counted per reason rather than by subtracting from the total: with three reasons a
    // subtraction silently files every new one under whichever arm it was written against.
    let count = |reason: UnverifiableReason| scopes.iter().filter(|s| s.reason == reason).count();
    let ignored = count(UnverifiableReason::Ignored);
    let submodule = count(UnverifiableReason::Submodule);
    let declared = count(UnverifiableReason::Declared);
    let records = scopes
        .iter()
        .map(|s| &s.id.key)
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let mut parts = Vec::new();
    if ignored > 0 {
        parts.push(format!("{ignored} the repository ignores"));
    }
    if submodule > 0 {
        parts.push(format!("{submodule} inside a submodule"));
    }
    if declared > 0 {
        parts.push(format!("{declared} declared untracked in project.akr"));
    }
    // Named, not just counted. The staleness fact earns its one line by ending in
    // `see `akr review-queue``; this one has no such command, and a fact nobody can act
    // on decays into a number people stop reading — which is how these globs accumulated.
    // A few examples cost four lines and make it actionable; the rest are a count.
    const SHOWN: usize = 4;
    let mut text = format!(
        "    {} scope path(s) on {records} record(s) git cannot answer for — {} — so those scopes cannot be verified\n",
        scopes.len(),
        parts.join(", ")
    );
    for scope in scopes.iter().take(SHOWN) {
        text.push_str(&format!(
            "      {} — {:?}, {}\n",
            scope.id, scope.glob, scope.reason
        ));
    }
    if scopes.len() > SHOWN {
        text.push_str(&format!(
            "      … and {} more\n",
            scopes.len() - SHOWN
        ));
    }
    text
}

fn scratch_fact(scratch: &akr_core::scratch::Scratch) -> String {
    use akr_core::scratch::human_bytes;
    if !scratch.present {
        return "    no .agent/scratch — nothing to prune\n".to_owned();
    }
    let prunable: Vec<_> = scratch.prunable(SCRATCH_DEFAULT_AGE).collect();
    let kept = scratch.kept().count();
    let mut line = format!(
        "    {} of scratch in {} entries",
        human_bytes(scratch.total_bytes()),
        scratch.entries.len()
    );
    if kept > 0 {
        line.push_str(&format!(", {kept} kept"));
    }
    if prunable.is_empty() {
        line.push_str(" — nothing prunable\n");
    } else {
        line.push_str(&format!(
            " — {} in {} prunable, see `akr scratch prune`\n",
            human_bytes(prunable.iter().map(|entry| entry.bytes).sum()),
            prunable.len()
        ));
    }
    line
}

/// `AKR-G042`, raised only under `akr check --scratch-clean`.
fn scratch_clean_diagnostic(
    scratch: &akr_core::scratch::Scratch,
) -> Option<akr_core::diagnostics::Diagnostic> {
    use akr_core::scratch::human_bytes;
    let prunable: Vec<_> = scratch.prunable(SCRATCH_DEFAULT_AGE).collect();
    if prunable.is_empty() {
        return None;
    }
    Some(
        akr_core::diagnostics::Diagnostic {
            code: akr_core::diagnostics::Code::new("AKR-G042"),
            severity: akr_core::diagnostics::Severity::Error,
            rule: None,
            message: format!(
                "scratch holds {} in {} prunable {}",
                human_bytes(prunable.iter().map(|entry| entry.bytes).sum()),
                prunable.len(),
                if prunable.len() == 1 {
                    "entry"
                } else {
                    "entries"
                }
            ),
            primary: akr_core::diagnostics::Label::new(akr_core::diagnostics::Subject::Ledger),
            notes: Vec::new(),
            help: None,
        }
        .help(
            "run `akr scratch prune`, or `akr scratch keep <name> --reason <why>` for the \
             ones the next session needs",
        ),
    )
}

/// `akr scratch list`
fn scratch_list(session: &mut Session) -> Result<Output, EnvError> {
    use akr_core::scratch::human_bytes;
    let scratch = akr_core::scratch::scan(&session.root, std::time::SystemTime::now());
    if !scratch.present {
        return Ok(Output::plain(
            "no .agent/scratch in this workspace\n",
            Value::object(vec![
                ("present", Value::bool(false)),
                ("entries", Value::array(Vec::new())),
                ("total_bytes", Value::integer(0)),
            ]),
        ));
    }

    let mut text = format!(
        "{} in {} entries under .agent/scratch\n",
        human_bytes(scratch.total_bytes()),
        scratch.entries.len()
    );
    for entry in &scratch.entries {
        let mark = match &entry.kept {
            Some(reason) if reason.is_empty() => "  keep".to_owned(),
            Some(reason) => format!("  keep: {reason}"),
            None if entry.is_prunable(SCRATCH_DEFAULT_AGE) => "  prunable".to_owned(),
            None => String::new(),
        };
        text.push_str(&format!(
            "  {:>9}  {:>4}d  {}{}\n",
            human_bytes(entry.bytes),
            entry.age_days,
            entry.name,
            mark
        ));
    }
    for name in &scratch.stale_keeps {
        text.push_str(&format!(
            "  KEEP names {name:?}, which is no longer there\n"
        ));
    }

    let entries = Value::array(
        scratch
            .entries
            .iter()
            .map(|entry| {
                Value::object(vec![
                    ("name", Value::string(entry.name.clone())),
                    (
                        "bytes",
                        Value::integer(i64::try_from(entry.bytes).unwrap_or(i64::MAX)),
                    ),
                    (
                        "age_days",
                        Value::integer(i64::try_from(entry.age_days).unwrap_or(i64::MAX)),
                    ),
                    (
                        "kept",
                        entry
                            .kept
                            .as_ref()
                            .map_or(Value::Null, |reason| Value::string(reason.clone())),
                    ),
                    (
                        "prunable",
                        Value::bool(entry.is_prunable(SCRATCH_DEFAULT_AGE)),
                    ),
                ])
            })
            .collect(),
    );
    Ok(Output::plain(
        text,
        Value::object(vec![
            ("present", Value::bool(true)),
            ("entries", entries),
            (
                "total_bytes",
                Value::integer(i64::try_from(scratch.total_bytes()).unwrap_or(i64::MAX)),
            ),
            (
                "prunable_bytes",
                Value::integer(
                    i64::try_from(scratch.prunable_bytes(SCRATCH_DEFAULT_AGE)).unwrap_or(i64::MAX),
                ),
            ),
        ]),
    ))
}

/// `akr scratch prune [--older-than <days>] [--dry-run]`
///
/// Removes only what the scan called prunable: unkept, and untouched for at least the
/// threshold. Everything named in `KEEP` survives regardless, which is the whole point of
/// the marker — an agent running this at the end of a session must not be able to delete
/// the thing the next session was told to expect.
fn scratch_prune(
    session: &mut Session,
    older_than: u64,
    dry_run: bool,
) -> Result<Output, EnvError> {
    use akr_core::scratch::human_bytes;
    let scratch = akr_core::scratch::scan(&session.root, std::time::SystemTime::now());
    let doomed: Vec<_> = scratch.prunable(older_than).cloned().collect();
    if doomed.is_empty() {
        return Ok(Output::plain(
            format!("nothing to prune: no unkept entry is {older_than} days old or more\n"),
            Value::object(vec![
                ("pruned", Value::integer(0)),
                ("bytes", Value::integer(0)),
                ("dry_run", Value::bool(dry_run)),
            ]),
        ));
    }

    let root = akr_core::scratch::dir(&session.root);
    let mut text = String::new();
    let mut removed = 0usize;
    let mut bytes = 0u64;
    for entry in &doomed {
        let path = root.join(&entry.name);
        if dry_run {
            text.push_str(&format!(
                "would remove  {:>9}  {:>4}d  {}\n",
                human_bytes(entry.bytes),
                entry.age_days,
                entry.name
            ));
            removed += 1;
            bytes += entry.bytes;
            continue;
        }
        let outcome = if entry.is_dir {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        match outcome {
            Ok(()) => {
                text.push_str(&format!(
                    "removed  {:>9}  {}\n",
                    human_bytes(entry.bytes),
                    entry.name
                ));
                removed += 1;
                bytes += entry.bytes;
            }
            // One locked file must not abandon the rest: report it and carry on, so the
            // command is worth running again rather than being all-or-nothing.
            Err(error) => text.push_str(&format!("kept  {}: {error}\n", entry.name)),
        }
    }
    text.push_str(&format!(
        "{} {} in {} entries\n",
        if dry_run { "would free" } else { "freed" },
        human_bytes(bytes),
        removed
    ));

    Ok(Output::plain(
        text,
        Value::object(vec![
            (
                "pruned",
                Value::integer(i64::try_from(removed).unwrap_or(0)),
            ),
            ("bytes", Value::integer(i64::try_from(bytes).unwrap_or(0))),
            ("dry_run", Value::bool(dry_run)),
        ]),
    ))
}

/// `akr scratch keep <name> --reason <text>` / `--forget`
fn scratch_keep(
    session: &mut Session,
    name: &str,
    reason: Option<&str>,
    forget: bool,
) -> Result<Output, EnvError> {
    let root = akr_core::scratch::dir(&session.root);
    let keep_path = akr_core::scratch::keep_path(&session.root);
    if !forget && !root.join(name).exists() {
        return Err(EnvError::new(
            "AKR-C042",
            format!("there is no {name:?} in .agent/scratch"),
        )
        .help("run `akr scratch list` to see what is there"));
    }
    let reason = match (forget, reason) {
        (true, _) => None,
        (false, Some(reason)) if !reason.trim().is_empty() => Some(reason.trim()),
        // A marker with no reason outlives whatever made it necessary, and the next person
        // has no way to judge it. This is the one piece of ceremony worth insisting on.
        (false, _) => {
            return Err(EnvError::new(
                "AKR-C003",
                "scratch keep requires --reason <text>: why the next session needs it",
            ));
        }
    };

    let existing = std::fs::read_to_string(&keep_path).unwrap_or_default();
    let mut lines: Vec<String> = Vec::new();
    let mut replaced = false;
    for line in existing.lines() {
        let trimmed = line.trim();
        let this = trimmed
            .split_once(char::is_whitespace)
            .map_or(trimmed, |(name, _)| name);
        if !trimmed.starts_with('#') && this == name {
            replaced = true;
            if let Some(reason) = reason {
                lines.push(format!("{name} {reason}"));
            }
            continue;
        }
        lines.push(line.to_owned());
    }
    if !replaced {
        if forget {
            return Err(EnvError::new(
                "AKR-C042",
                format!("KEEP does not name {name:?}"),
            ));
        }
        if lines.is_empty() {
            lines.push(
                "# Scratch entries the next session still needs, and why (D-036).".to_owned(),
            );
            lines.push("# One `<name> <reason>` per line. Anything not listed here is".to_owned());
            lines.push("# prunable once it goes untouched. Edit by hand freely.".to_owned());
        }
        lines.push(format!(
            "{name} {}",
            reason.expect("a reason was required above")
        ));
    }

    if let Some(parent) = keep_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| EnvError::new("AKR-C042", format!("cannot create .agent/scratch: {e}")))?;
    }
    let mut text = lines.join("\n");
    text.push('\n');
    std::fs::write(&keep_path, &text)
        .map_err(|e| EnvError::new("AKR-C042", format!("cannot write KEEP: {e}")))?;

    let message = if forget {
        format!("{name} is no longer kept; it prunes with everything else\n")
    } else {
        format!("{name} will survive `akr scratch prune`\n")
    };
    Ok(Output::plain(
        message,
        Value::object(vec![
            ("name", Value::string(name.to_owned())),
            ("kept", Value::bool(!forget)),
        ]),
    ))
}

fn check(
    session: &mut Session,
    scratch_clean: bool,
    review_clean: bool,
    views_current: bool,
) -> Result<Output, EnvError> {
    session.attach_lock();
    let model = session.resolve();
    let mut diagnostics = session.diagnostics(&model);
    let queue = session.review_queue();
    let scratch = akr_core::scratch::scan(&session.root, std::time::SystemTime::now());
    diagnostics.extend(queue.diagnostics.clone());

    // Immutable source library verification (AKR-S021).
    for diag in akr_core::source::verify_catalog(&session.root) {
        diagnostics.push(akr_core::diagnostics::Diagnostic {
            code: akr_core::diagnostics::Code::new("AKR-S021"),
            severity: akr_core::diagnostics::Severity::Error,
            rule: None,
            message: diag.to_string(),
            primary: akr_core::diagnostics::Label::new(akr_core::diagnostics::Subject::Ledger),
            notes: Vec::new(),
            help: None,
        });
    }
    diagnostics.extend(citation_diagnostics(session));

    let mut text = String::new();
    text.push_str(&format!("akr check — {}\n", session.ledger.project.name));
    text.push_str(&summary_lines(session, &model));
    text.push('\n');

    // View currency is part of every check, not an opt-in (A1/A2). `akr check` reporting
    // "no diagnostics" on a workspace whose committed views do not match its ledger was
    // the single most common defect in a 2026-09 sweep of fifteen workspaces: ten were
    // shipping mismatched views and every one of them passed a plain check. The two
    // causes are separated by severity, not by flag — a `tool:`-only difference is
    // AKR-E015 (warning, never fatal), anything else is AKR-E011/E012 (error). `--views-
    // current` now selects the per-view stage-F REPORT; it no longer decides whether the
    // comparison happens, because a gate you have to remember to ask for is not a gate.
    //
    // A ledger with no records owes no projections. `akr init` scaffolds a workspace and
    // tells the reader to write a record next; greeting them with seven missing-view
    // errors before they have written anything would make a correct scaffold look broken.
    // Views become owed at the first record, which is also the first moment a view could
    // say anything.
    let freshness = session.freshness(&queue);
    let context = akr_core::render::RenderContext::new(&model, &freshness);
    let mut view_diagnostics = if counts_of(session, &model).records == 0 {
        Vec::new()
    } else {
        check_views_current(&session.view_dir(), context)
            .map_err(|e| EnvError::new("AKR-E001", format!("cannot read the view directory: {e}")))?
    };

    // An agent that has just written a record has stale views BY CONSTRUCTION, and has not
    // reached the point in its session where it would rebuild. Failing here would make
    // "akr check is clean" unreachable mid-task and leave the same two bad ways out that
    // AKR-G004 was exempted for: rebuild prematurely, or drop to --lenient and lose every
    // other signal. So while the ledger itself is uncommitted, a stale view is AKR-E016 at
    // warning severity — you have unsaved work — rather than AKR-E011/E012.
    //
    // A COMMITTED ledger whose committed views do not match it is the defect A1 is about,
    // and stays a hard error. `akr build --check` is unaffected either way: it is the CI
    // gate of D-025 and reports the difference regardless of what is committed.
    if ledger_is_uncommitted(session) {
        for diagnostic in &mut view_diagnostics {
            if matches!(diagnostic.code.as_str(), "AKR-E011" | "AKR-E012") {
                diagnostic.code = akr_core::diagnostics::Code::new("AKR-E016");
                diagnostic.severity = akr_core::diagnostics::Severity::Warning;
                // Say WHICH of the two situations this is. A reader who cannot tell
                // "behind my own unsaved edits" from "this repository ships views that do
                // not match its ledger" has been told nothing useful.
                if let akr_core::diagnostics::Subject::File(path) = &diagnostic.primary.subject {
                    diagnostic.message =
                        format!("{path} is behind uncommitted ledger changes in the working tree");
                }
                diagnostic.help = Some(
                    "run `akr build` before committing; until then the views describe the \
                     ledger as it was last built, not as it stands"
                        .to_owned(),
                );
            }
        }
    }

    if views_current {
        text.push_str("  stages A-D                                 ok\n");
        text.push_str(&format!(
            "  stage F  emit (in memory)   {} views        {}\n",
            View::ALL.len(),
            if view_diagnostics.is_empty() {
                "ok"
            } else {
                "FAILED"
            }
        ));
        for &view in View::ALL {
            let state = if view_diagnostics
                .iter()
                .any(|d| d.message.contains(view.file_name()))
            {
                "DIFFERS"
            } else {
                "current"
            };
            text.push_str(&format!("    {:<24} {state}\n", view.file_name()));
        }
    } else {
        text.push_str(&stage_lines(session, &model));
        text.push('\n');
        text.push_str(&resolve_detail(session, &model));
        text.push('\n');
        text.push_str("  build facts (not diagnostics):\n");
        text.push_str(&format!(
            "    {} records stale, {} at risk — see `akr review-queue`\n",
            queue.stale.len(),
            queue.at_risk.len()
        ));
        text.push_str(&scratch_fact(&scratch));
        text.push_str(&unverifiable_scope_fact(&session.unverifiable_scopes()));
        let missing = crate::init::missing_gitignore_entries(&session.root);
        if !missing.is_empty() {
            text.push_str(&format!(
                "    .gitignore is missing {} entry(ies) `akr init` now writes: {} — add them, or these paths accumulate untracked\n",
                missing.len(),
                missing.join(", ")
            ));
        }
        text.push_str(&views_fact(&view_diagnostics));
    }

    diagnostics.extend(view_diagnostics);

    // V-105: a recorded command that names a test which no longer exists (AKR-G024).
    // Warning only — see `akr_core::gate` for why this is decay rather than carelessness.
    if let (Some(repository), Some(head)) =
        (session.repository.as_ref(), session.inputs.commit.as_ref())
        && let Ok(gate_diagnostics) =
            akr_core::gate::phantom_command_tokens(&session.ledger, repository, head)
    {
        diagnostics.extend(gate_diagnostics);
    }

    // Scratch is a fact about the working tree, never a ledger contradiction, so it is
    // reported always and fails only when asked — exactly the shape `--review-clean` has
    // for staleness (D-024, D-036).
    if scratch_clean && let Some(diagnostic) = scratch_clean_diagnostic(&scratch) {
        diagnostics.push(diagnostic);
    }

    if review_clean && let Some(diagnostic) = queue.review_clean_diagnostic() {
        let mut diagnostic = diagnostic;
        // Point at the first stale record, so the caret lands on something actionable.
        if let Some(first) = queue.stale.first() {
            diagnostic.primary.subject = akr_core::diagnostics::Subject::Revision(first.id.clone());
            diagnostic.primary.message = Some(format!(
                "stale: {}",
                cause_text(&first.cause, session.today)
            ));
            session.spans.attach(&mut diagnostic);
        }
        diagnostics.push(diagnostic);
    }

    diagnostics.sort_by_key(Diagnostic::sort_key);
    let fatal = diagnostics
        .iter()
        .filter(|d| is_fatal(d, session.global.profile))
        .count();
    let exit = if fatal > 0 {
        Exit::Diagnostics
    } else {
        Exit::Ok
    };

    text.push('\n');
    if diagnostics.is_empty() {
        text.push_str("no diagnostics\n");
    } else {
        let (rendered, _) = report(&diagnostics, &session.sources, session.global.profile);
        text.push_str(&rendered);
    }

    let counts = counts_of(session, &model);
    let result = Value::object(vec![
        ("records", Value::integer(counts.records as i64)),
        ("revisions", Value::integer(counts.revisions as i64)),
        ("files", Value::integer(counts.files as i64)),
        ("references", Value::integer(counts.references as i64)),
        ("heads", Value::integer(counts.heads as i64)),
        (
            "checks",
            Value::object(vec![
                ("total", Value::integer(counts.checks as i64)),
                ("satisfied", Value::integer(counts.satisfied as i64)),
            ]),
        ),
        ("stale", Value::integer(queue.stale.len() as i64)),
        ("at_risk", Value::integer(queue.at_risk.len() as i64)),
    ]);

    Ok(Output::text(text)
        .with_result(result)
        .with_diagnostics(diagnostics, exit))
}

struct Counts {
    records: usize,
    revisions: usize,
    files: usize,
    references: usize,
    heads: usize,
    checks: usize,
    satisfied: usize,
}

fn counts_of(session: &Session, model: &akr_core::resolve::ResolvedModel<'_>) -> Counts {
    Counts {
        records: session.ledger.keys().len(),
        revisions: session.ledger.records().len(),
        // Source files only. The lock is build output, not input, and the lock's own
        // `source` list is the count this has to agree with (`spec/akr-lock.md` §2).
        files: session.inputs.sources.len(),
        references: model.edges.len(),
        heads: model.heads.len(),
        checks: model.acceptance.len(),
        satisfied: model
            .acceptance
            .iter()
            .filter(|v| v.verdict.is_satisfied())
            .count(),
    }
}

/// Whether a diagnostic is about the lock being out of date rather than about the ledger.
fn is_lock_currency(diagnostic: &Diagnostic) -> bool {
    matches!(
        diagnostic.code.as_str(),
        "AKR-R051" | "AKR-R052" | "AKR-R053"
    )
}

fn summary_lines(session: &Session, model: &akr_core::resolve::ResolvedModel<'_>) -> String {
    let counts = counts_of(session, model);
    let commit = session
        .inputs
        .commit
        .as_ref()
        .map_or_else(|| "(none)".to_owned(), |c| c.as_str()[..8].to_owned());
    format!(
        "  {} records, {} revisions, {} files\n  commit {commit}, grammar {GRAMMAR_VERSION}, \
         vocabulary {VOCABULARY_VERSION}, today {}\n",
        counts.records, counts.revisions, counts.files, session.today
    )
}

fn stage_lines(session: &Session, model: &akr_core::resolve::ResolvedModel<'_>) -> String {
    let counts = counts_of(session, model);
    let mark = |bad: bool| if bad { "FAILED" } else { "ok" };
    let parse_failed = !session.parse_diagnostics.is_empty();
    let stage_failed = |stage: akr_core::diagnostics::Stage| {
        model
            .diagnostics
            .iter()
            .any(|d| d.code.stage() == Some(stage))
    };
    format!(
        "  stage A  parse          {:>2} revisions       {}\n\
         \x20 stage B  type-check     {:>2} revisions       {}\n\
         \x20 stage C  link          {:>3} references      {}\n\
         \x20 stage D  resolve        {:>2} heads           {}\n",
        counts.revisions,
        mark(parse_failed),
        counts.revisions,
        mark(stage_failed(akr_core::diagnostics::Stage::Type)),
        counts.references,
        mark(stage_failed(akr_core::diagnostics::Stage::Link)),
        counts.heads,
        mark(stage_failed(akr_core::diagnostics::Stage::Resolve)),
    )
}

fn resolve_detail(session: &Session, model: &akr_core::resolve::ResolvedModel<'_>) -> String {
    let ledger = &session.ledger;
    let live_heads = model
        .heads
        .values()
        .filter(|id| ledger.get(id).is_some_and(akr_core::model::Record::is_live))
        .count();
    let chain_heads = model.heads.len() - live_heads;

    let chains: Vec<String> = model
        .supersession
        .iter()
        .filter(|(_, chain)| chain.len() > 1)
        .map(|(key, _)| key.to_string())
        .collect();

    let unfinished: usize = ledger.records().iter().map(|r| r.dispositions.len()).sum();

    let topics: BTreeSet<String> = ledger
        .records()
        .iter()
        .filter(|r| r.is_live())
        .filter_map(|r| r.topic.as_ref().map(ToString::to_string))
        .collect();

    let plans_live = ledger
        .records()
        .iter()
        .filter(|r| r.is_live() && !r.targets(Relation::PlanOfRecord).is_empty())
        .count();
    let plans_total = ledger
        .records()
        .iter()
        .filter(|r| !r.targets(Relation::PlanOfRecord).is_empty())
        .count();

    let sealed = ledger.records().iter().filter(|r| r.is_sealed()).count();
    let matched = ledger
        .facts
        .seals
        .values()
        .filter(|fact| match (&fact.recorded, &fact.computed) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        })
        .count();

    let contradictions: usize = ledger
        .records()
        .iter()
        .map(|r| r.targets(Relation::Contradicts).len())
        .sum();
    let acknowledged: usize = ledger
        .records()
        .iter()
        .filter(|r| r.acknowledged && !r.targets(Relation::Contradicts).is_empty())
        .count();

    let counts = counts_of(session, model);
    let mut out = String::from("  resolve detail\n");
    out.push_str(&format!(
        "    heads                        {live_heads} live, {chain_heads} resolved by supersession chain\n"
    ));
    out.push_str(&format!(
        "    supersession chains          {:>2} ({})\n",
        chains.len(),
        wrap_list(&chains, 37)
    ));
    out.push_str(&format!(
        "    dispositions checked          {unfinished} unfinished children, {unfinished} dispositioned\n"
    ));
    out.push_str(&format!(
        "    topics                        {} on live records, no overlapping pair\n",
        topics.len()
    ));
    out.push_str(&format!(
        "    plan_of_record                {plans_live} live, {} superseded\n",
        plans_total - plans_live
    ));
    out.push_str(&format!(
        "    acceptance                    {} checks, {} satisfied\n",
        counts.checks, counts.satisfied
    ));
    out.push_str(&format!(
        "    sealed revisions             {sealed} hashed, {matched} match akr.lock\n"
    ));
    out.push_str(&format!(
        "    contradictions                {contradictions} declared, {acknowledged} acknowledged\n"
    ));
    out
}

/// Wraps a comma-separated list, continuing under the opening parenthesis.
fn wrap_list(items: &[String], indent: usize) -> String {
    let joined = items.join(", ");
    if joined.len() <= 40 {
        return joined;
    }
    let pad = " ".repeat(indent);
    items.join(&format!(",\n{pad}"))
}

fn cause_text(cause: &akr_core::freshness::StaleCause, today: Date) -> String {
    match cause {
        akr_core::freshness::StaleCause::Watch { glob, commit, .. } => format!(
            "watches {:?} matched by {}",
            glob.as_str(),
            &commit.as_str()[..8]
        ),
        akr_core::freshness::StaleCause::ReviewAfter { date } => {
            let days = days_between(*date, today);
            format!("review_after {date} passed {days} days ago")
        }
    }
}

/// Whole days from `from` to `to`, proleptic Gregorian.
///
/// Hand-rolled because the tool has no dependencies (`docs/13-implementation-roadmap.md`
/// §4) and because a day count is the whole of the calendar arithmetic AKR needs.
fn days_between(from: Date, to: Date) -> i64 {
    days_from_civil(to) - days_from_civil(from)
}

/// Howard Hinnant's `days_from_civil`, the inverse of the one in `session.rs`.
fn days_from_civil(date: Date) -> i64 {
    let y = i64::from(date.year) - i64::from(date.month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(date.month);
    let d = i64::from(date.day);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

// -------------------------------------------------------------------------------------
// build
// -------------------------------------------------------------------------------------

fn build(session: &mut Session, check: bool) -> Result<Output, EnvError> {
    session.attach_lock();
    let model = session.resolve();
    let mut diagnostics = session.diagnostics(&model);
    let queue = session.review_queue();
    let lock_path = session.akr_dir.join("akr.lock");
    let lock = model.to_lock();
    let rendered = lock.render();
    let lock_changed = std::fs::read_to_string(&lock_path).ok().as_deref() != Some(&rendered);

    if check {
        let mut text = String::new();
        text.push_str(&format!(
            "akr build --check — {}\n",
            session.ledger.project.name
        ));
        text.push_str(&summary_lines(session, &model));
        text.push('\n');
        text.push_str(&stage_lines(session, &model));
        text.push('\n');
        text.push_str(&resolve_detail(session, &model));
        text.push('\n');
        text.push_str("  build facts (not diagnostics):\n");
        text.push_str(&format!(
            "    {} records stale, {} at risk (see akr review-queue)\n",
            queue.stale.len(),
            queue.at_risk.len()
        ));

        let freshness = session.freshness(&queue);
        let context = akr_core::render::RenderContext::new(&model, &freshness);
        let view_diagnostics = check_views_current(&session.view_dir(), context).map_err(|e| {
            EnvError::new("AKR-E001", format!("cannot read the view directory: {e}"))
        })?;
        let views_current = view_diagnostics.is_empty();
        text.push_str(&format!(
            "  stage E  emit (in memory)      {} views        {}\n",
            View::ALL.len(),
            if views_current { "ok" } else { "FAILED" }
        ));
        for &view in View::ALL {
            let state = if view_diagnostics
                .iter()
                .any(|d| d.message.contains(view.file_name()))
            {
                "DIFFERS"
            } else {
                "current"
            };
            text.push_str(&format!("    {:<24} {state}\n", view.file_name()));
        }
        diagnostics.extend(view_diagnostics);

        diagnostics.extend(build_lock_check_diagnostics(session, &lock));

        let index_inputs = akr_core::store::IndexInputs {
            model: &model,
            queue: &queue,
            spans: &session.spans,
            diagnostics: &diagnostics,
            today: &session.today.to_string(),
        };
        let index_path = temporary_index_path();
        let index = match akr_core::store::build(&index_path, &index_inputs) {
            Ok(stats) => {
                text.push_str(&format!(
                    "  stage F  index (in memory)   {} revisions, {} indexed    ok\n",
                    stats.revisions, stats.indexed
                ));
                Some(stats)
            }
            Err(error) => {
                text.push_str("  stage F  index (in memory)   FAILED\n");
                diagnostics.push(error.diagnostic());
                None
            }
        };
        let _ = std::fs::remove_file(&index_path);

        text.push_str(&format!(
            "  lock check                   {}\n",
            if lock_changed { "changed" } else { "current" }
        ));

        diagnostics.sort_by_key(Diagnostic::sort_key);
        let fatal = diagnostics
            .iter()
            .filter(|d| is_fatal(d, session.global.profile))
            .count();

        if diagnostics.is_empty() {
            text.push_str("no diagnostics\n");
        } else {
            let (rendered, _) = report(&diagnostics, &session.sources, session.global.profile);
            text.push_str(&rendered);
        }

        let result = Value::object(vec![
            ("check", Value::bool(true)),
            ("views_written", Value::integer(0)),
            ("views_current", Value::bool(views_current)),
            ("lock_changed", Value::bool(lock_changed)),
            (
                "indexed",
                Value::integer(index.map_or(0, |s| s.revisions as i64)),
            ),
            ("stale", Value::integer(queue.stale.len() as i64)),
            ("at_risk", Value::integer(queue.at_risk.len() as i64)),
        ]);
        let exit = if fatal > 0 {
            Exit::Diagnostics
        } else {
            Exit::Ok
        };
        return Ok(Output::text(text)
            .with_result(result)
            .with_diagnostics(diagnostics, exit));
    }

    // Only lock-currency diagnostics are excluded from the halt decision when the command is
    // expected to write the lock; here, every error must stop the check.
    let filtered: Vec<_> = diagnostics
        .iter()
        .filter(|d| !is_lock_currency(d))
        .cloned()
        .collect();
    let fatal = filtered
        .iter()
        .filter(|d| is_fatal(d, session.global.profile))
        .count();
    if fatal > 0 {
        let (rendered, _) = report(&filtered, &session.sources, session.global.profile);
        return Ok(Output::text(rendered).with_diagnostics(filtered, Exit::Diagnostics));
    }

    let freshness = session.freshness(&queue);
    let context = akr_core::render::RenderContext::new(&model, &freshness);
    let written = write_views(&session.view_dir(), context)
        .map_err(|e| EnvError::new("AKR-E001", format!("cannot write views: {e}")))?;

    if lock_changed {
        std::fs::write(&lock_path, &rendered)
            .map_err(|e| EnvError::new("AKR-C042", format!("cannot write akr.lock: {e}")))?;
    }

    let counts = counts_of(session, &model);
    let mut text = String::new();
    text.push_str(&format!(
        "parsed {} revisions in {} files\n",
        counts.revisions, counts.files
    ));
    text.push_str(&format!(
        "resolved {} heads, {} superseded revisions\n",
        counts.heads,
        counts.revisions - counts.heads
    ));
    text.push_str(&format!(
        "{} stale records, {} at risk (see akr review-queue)\n",
        queue.stale.len(),
        queue.at_risk.len()
    ));
    text.push_str(&format!(
        "wrote docs/generated/ ({} views, {} changed)\n",
        View::ALL.len(),
        written.len()
    ));
    text.push_str(if lock_changed {
        "wrote akr.lock\n"
    } else {
        "akr.lock unchanged\n"
    });

    // Stage E, last: the cache is derived from everything above it, so a build that failed
    // earlier never gets as far as writing one (`docs/06-compiler-pipeline.md` §7).
    let index = index_build(session, &model, &queue, &diagnostics)?;
    match index {
        Some(stats) if stats.rebuilt => text.push_str(&format!(
            "indexed {} revisions ({} full-text)\n",
            stats.revisions, stats.indexed
        )),
        Some(_) => text.push_str("index cache current\n"),
        None => text.push_str("index cache not written (--no-rebuild)\n"),
    }

    // The source library's own generation, synced beside the record cache and never with
    // it (D-031). Silent when there is no catalog: most workspaces register no sources.
    #[cfg(feature = "fts5")]
    let source_index = source_index_build(session)?;
    #[cfg(feature = "fts5")]
    match source_index {
        Some(stats) if stats.added > 0 || stats.removed > 0 || stats.rebuilt => {
            text.push_str(&format!(
                "chunked {} source documents into {} chunks ({} added, {} removed)\n",
                stats.documents, stats.chunks, stats.added, stats.removed
            ));
        }
        Some(stats) if stats.documents > 0 => text.push_str("source index current\n"),
        _ => {}
    }

    let result = Value::object(vec![
        ("check", Value::bool(false)),
        ("views_written", Value::integer(written.len() as i64)),
        ("lock_changed", Value::bool(lock_changed)),
        ("stale", Value::integer(queue.stale.len() as i64)),
        ("at_risk", Value::integer(queue.at_risk.len() as i64)),
        (
            "indexed",
            Value::integer(index.map_or(0, |s| s.revisions as i64)),
        ),
        #[cfg(feature = "fts5")]
        (
            "source_chunks",
            Value::integer(source_index.map_or(0, |s| s.chunks as i64)),
        ),
    ]);
    Ok(Output::text(text).with_result(result))
}

fn build_lock_check_diagnostics(
    session: &Session,
    computed: &akr_core::lock::Lock,
) -> Vec<Diagnostic> {
    const V024: RuleId = RuleId(24);
    let Some(text) = session.lock_text.as_deref() else {
        return vec![
            Diagnostic::error(
                akr_core::diagnostics::codes::R052,
                V024,
                Subject::File(".akr/akr.lock".to_owned()),
                "akr.lock is missing".to_owned(),
            )
            .help("run `akr build`"),
        ];
    };

    let recorded = match akr_core::lock::Lock::parse(text) {
        Ok(lock) => lock,
        Err(error) => {
            return vec![
                Diagnostic::error(
                    akr_core::diagnostics::codes::R052,
                    V024,
                    Subject::File(".akr/akr.lock".to_owned()),
                    format!("akr.lock does not parse: {error}"),
                )
                .help("run `akr build`; never hand-merge a lock"),
            ];
        }
    };

    akr_core::lock::currency_diagnostics(&recorded, computed, ".akr/akr.lock")
}

fn temporary_index_path() -> std::path::PathBuf {
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!("akr-build-check-{suffix}.sqlite",))
}

/// Runs stage E, honouring `--no-rebuild`.
///
/// `--no-rebuild` exists for read-only checkouts, where writing a cache is not merely
/// unwanted but impossible. It suppresses the write rather than failing the build: on
/// `akr build` the flag is a contradiction in terms, and `AKR-I031` belongs to the read
/// command that needed a rebuild it was not allowed to do.
fn index_build(
    session: &Session,
    model: &akr_core::resolve::ResolvedModel,
    queue: &akr_core::freshness::ReviewQueue,
    diagnostics: &[akr_core::diagnostics::Diagnostic],
) -> Result<Option<akr_core::store::IndexStats>, EnvError> {
    if session.global.no_rebuild {
        return Ok(None);
    }
    let inputs = akr_core::store::IndexInputs {
        model,
        queue,
        spans: &session.spans,
        diagnostics,
        today: &session.today.to_string(),
    };
    let path = akr_core::store::cache_path(&session.akr_dir);
    akr_core::store::build(&path, &inputs)
        .map(Some)
        .map_err(|error| EnvError::new(error.code.as_str(), error.message))
}

/// `AKR-S022` for every record whose `source` block cites the library and misses.
///
/// This runs in `akr check` rather than as a V-rule because it is the one provenance
/// question that cannot be answered from the ledger alone: it needs the registered bytes.
/// A workspace with no catalog produces nothing, which is most workspaces.
fn citation_diagnostics(session: &Session) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for record in session.ledger.records() {
        for source in &record.sources {
            for problem in akr_core::source::check_citation_at(&session.root, source) {
                out.push(Diagnostic {
                    code: akr_core::diagnostics::Code::new("AKR-S022"),
                    severity: akr_core::diagnostics::Severity::Error,
                    rule: None,
                    message: format!("{}: {problem}", record.id),
                    primary: akr_core::diagnostics::Label::new(
                        akr_core::diagnostics::Subject::Revision(record.id.clone()),
                    ),
                    notes: Vec::new(),
                    help: Some(
                        "record citations name a registered document and an exact byte \
                         range; `akr source search` prints both"
                            .to_owned(),
                    ),
                });
            }
        }
    }
    out
}

/// Brings the source-library index into agreement with `sources/catalog.json`.
///
/// Separate from [`index_build`] on purpose, and that separation is the point of D-031:
/// the record cache is stamped with the ledger's source-graph hash and the source index
/// with the corpus hash, so a record write leaves the chunk tables untouched and a source
/// registration leaves the record tables untouched.
///
/// A source whose bytes no longer match its registration is an error here, because
/// serving a passage from a file that has been edited is the one thing the library exists
/// to prevent.
#[cfg(feature = "fts5")]
pub(crate) fn source_index_build(
    session: &Session,
) -> Result<Option<akr_core::store::SourceIndexStats>, EnvError> {
    if session.global.no_rebuild {
        return Ok(None);
    }
    let corpus = akr_core::source::load_corpus(&session.root)
        .map_err(|d| EnvError::new("AKR-S021", d.to_string()))?;
    let path = akr_core::store::sources_cache_path(&session.akr_dir);
    if corpus.is_empty() && !path.exists() {
        return Ok(None);
    }
    akr_core::store::sync_sources(
        &path,
        &corpus,
        crate::session::TOOL_VERSION,
        &session.today.to_string(),
    )
    .map(Some)
    .map_err(|error| EnvError::new(error.code.as_str(), error.message))
}

// -------------------------------------------------------------------------------------
// view, get, why-current
// -------------------------------------------------------------------------------------

fn view(session: &Session, name: &str) -> Result<Output, EnvError> {
    let Some(view) = View::from_name(name) else {
        return Err(EnvError::new(
            "AKR-E003",
            format!(
                "unknown view {name:?}; known views are {}",
                View::ALL
                    .iter()
                    .map(|v| v.name())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    };
    let model = session.resolve();
    let queue = session.review_queue();
    let freshness = session.freshness(&queue);
    let context = akr_core::render::RenderContext::new(&model, &freshness);
    // Every view renders `Some`, except `PAPERCUTS.md`, which is emitted only once the
    // ledger holds a papercut (D-027).
    let Some(text) = render(view, context) else {
        return Err(EnvError::new(
            "AKR-E003",
            "no papercuts logged; PAPERCUTS.md is emitted once one exists (D-027)",
        )
        .help("log one with `akr papercut -m <agent> \"message\"`"));
    };
    Ok(Output::text(text.clone()).with_result(Value::object(vec![
        ("view", Value::string(view.name())),
        ("text", Value::string(text)),
    ])))
}

fn get(
    session: &Session,
    reference: &str,
    history: bool,
    relations: bool,
    detail: crate::args::Detail,
) -> Result<Output, EnvError> {
    use crate::args::Detail;
    let model = session.resolve();
    let ledger = &session.ledger;
    let parsed = Reference::parse(reference)
        .map_err(|e| EnvError::new("AKR-L001", format!("{reference}: {e}")))?;
    let Some(record) = ledger.resolve(&parsed).ok().flatten() else {
        return Err(EnvError::new(
            "AKR-L001",
            format!("{reference} does not resolve to a record"),
        ));
    };

    let queue = session.review_queue();
    let freshness = session.freshness(&queue);
    let mut text = format!(
        "{} : {}    state {}    {}\n",
        record.id,
        record.kind.name(),
        record.state.name(),
        if model.is_head(&record.id) {
            "head"
        } else {
            "not head"
        }
    );
    text.push_str(&format!("  title    {}\n", record.title));
    if !record.scope.is_empty() {
        text.push_str(&format!(
            "  scope    {}\n",
            record
                .scope
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if let Some(topic) = &record.topic {
        text.push_str(&format!("  topic    {topic}\n"));
    }
    if freshness.is_stale(&record.id) {
        text.push_str("  freshness  stale\n");
    } else if let Some(flag) = freshness.at_risk(&record.id) {
        text.push_str(&format!(
            "  freshness  at_risk (depth {}, via {} -> {})\n",
            flag.depth,
            flag.via,
            flag.path
                .iter()
                .map(|id| format!("@{id}"))
                .collect::<Vec<_>>()
                .join(" -> ")
        ));
    }
    if detail != Detail::Summary
        && let Some((slot, body)) = akr_core::context::named_body_of(record)
    {
        text.push_str(&format!("\n  {}\n", slot.name()));
        for line in body.lines() {
            text.push_str(&format!("    {line}\n"));
        }
    }
    // The note, which used to be invisible here. `akr revise --from` is a slot overlay
    // and keeps every slot the overlay does not mention — but a reader who cannot see
    // `note` reads a preserved note as a lost one, and diagnoses a merge bug that is not
    // there. It is also the slot that says why a plan stopped (D-026).
    if detail != Detail::Summary
        && let Some(note) = match record.get(akr_core::model::ContentSlot::Note) {
            Some(
                akr_core::model::ContentValue::Prose(text)
                | akr_core::model::ContentValue::Text(text),
            ) if !text.trim().is_empty() => Some(text.clone()),
            _ => None,
        }
    {
        text.push_str("\n  note\n");
        for line in note.lines() {
            text.push_str(&format!("    {line}\n"));
        }
    }
    if detail != Detail::Summary && !record.claims.is_empty() {
        text.push_str("\n  claims\n");
        for claim in &record.claims {
            let first = claim.text.lines().next().unwrap_or_default();
            text.push_str(&format!("    #{:<14} {first}\n", claim.anchor.as_str()));
        }
    }
    // Acceptance, because `complete` demands a mapping for every check and a reader who
    // cannot see the checks cannot supply one. The block used to appear only under
    // `--detail canonical`, so the first `knowledge.complete` on a ten-check work item
    // was always refused for the ten mappings its author had never been shown
    // (`saveyourskin.papercut.knowledge-get-detail-body-omitted-a-work-record`).
    if detail != Detail::Summary
        && let Some(acceptance) = &record.acceptance
        && !acceptance.checks.is_empty()
    {
        let verdicts = akr_core::resolve::acceptance_verdicts(ledger);
        text.push_str("\n  acceptance\n");
        for check in &acceptance.checks {
            let verdict = verdicts
                .iter()
                .find(|v| v.owner == record.id && v.check == check.id)
                .map_or_else(|| "no evidence".to_owned(), |v| verdict_line(&v.verdict));
            text.push_str(&format!(
                "    #{:<14} {} — {verdict}\n",
                check.id.as_str(),
                check.method.name()
            ));
            for line in check.statement.lines() {
                text.push_str(&format!("      {line}\n"));
            }
            if let Some(command) = &check.command {
                text.push_str(&format!("      command  {command}\n"));
            }
            for reference in &check.verified_by {
                text.push_str(&format!("      verified_by  {reference}\n"));
            }
        }
    }
    // Provenance, printed rather than left in the file. `sources/akr-ingest-and-mcp-fix-advice.md`:
    // an agent should never have to open a `.akr` file to find out where a record came
    // from, and the locator is the thing that makes the difference between "this came
    // from the audit somewhere" and "this came from these bytes of that audit".
    if !record.sources.is_empty() {
        text.push_str("\n  sources\n");
        for source in &record.sources {
            let kind = match source.kind {
                akr_core::model::SourceKind::Legacy => "legacy",
                akr_core::model::SourceKind::External => "external",
                akr_core::model::SourceKind::Internal => "internal",
            };
            let where_ = source
                .document
                .as_ref()
                .map(|document| format!("source:{document}"))
                .or_else(|| source.path.clone())
                .or_else(|| source.url.clone())
                .unwrap_or_else(|| "(no locator)".to_owned());
            text.push_str(&format!("    {kind:<9} {where_}\n"));
            if let Some(range) = &source.range {
                text.push_str(&format!(
                    "              lines {}-{}  bytes {}..{}\n",
                    range.start_line, range.end_line, range.start_byte, range.end_byte
                ));
                text.push_str("              non-authoritative; `akr source get` prints it\n");
            }
        }
    }
    if relations && detail == Detail::Summary {
        // A count rather than the edges: summary is for deciding whether to fetch the
        // record, and "seven things depend on this" answers that as well as seven lines
        // would, for a twentieth of the payload.
        let outbound: usize = record.relations.values().map(Vec::len).sum();
        text.push_str(&format!("\n  relations  {outbound} outbound\n"));
    } else if relations {
        text.push_str("\n  relations (outbound)\n");
        for (relation, references) in &record.relations {
            for target in references {
                text.push_str(&format!("    {:<14} -> {target}\n", relation.name()));
            }
        }
        text.push_str("  relations (inbound)\n");
        for other in ledger.records() {
            for (relation, references) in &other.relations {
                for reference in references {
                    if ledger
                        .resolve(reference)
                        .ok()
                        .flatten()
                        .is_some_and(|t| t.id == record.id)
                    {
                        text.push_str(&format!("    {:<14} <- @{}\n", relation.name(), other.id));
                    }
                }
            }
        }
    }
    if history {
        text.push_str("\n  history\n");
        for revision in ledger.revisions_of(&record.id.key) {
            text.push_str(&format!(
                "    /{}  {:<12} {}\n",
                revision.id.revision,
                revision.state.name(),
                revision.title
            ));
        }
    }

    // `--detail canonical` used to add the source text to the JSON result and nothing
    // else, so asking for it at a terminal printed exactly what the default already did.
    // A flag named for a thing has to show that thing in the form it was asked in.
    if detail == Detail::Canonical
        && let Some(source) = session.inputs.canonical_text.get(&record.id)
    {
        text.push_str("\n  canonical source\n");
        for line in source.lines() {
            text.push_str(&format!("    {line}\n"));
        }
    }

    // The JSON form is the record, not a summary of it: `docs/08-mcp.md` §3 shows scope,
    // claims, relations, freshness and the canonical source text, and `knowledge.get`
    // returns this object verbatim. An agent that had to make a second call for the body
    // would be paying for a distinction between the two surfaces that should not exist.
    let mut fields = vec![
        ("key", Value::string(record.id.key.to_string())),
        ("rev", Value::integer(i64::from(record.id.revision))),
        ("kind", Value::string(record.kind.name())),
        ("class", Value::string(record.kind.class().name())),
        ("state", Value::string(record.state.name())),
        ("is_head", Value::bool(model.is_head(&record.id))),
        ("title", Value::string(record.title.clone())),
        (
            "scope",
            Value::array(record.scope.iter().map(scope_json).collect()),
        ),
    ];
    if let Some(topic) = &record.topic {
        fields.push(("topic", Value::string(topic.to_string())));
    }
    fields.push(("detail", Value::string(detail.as_str())));
    if detail == Detail::Summary {
        // Relation *summaries*: one count per relation name. Enough to know the record is
        // connected and to what sort of thing, without the closure.
        fields.push((
            "relation_counts",
            Value::Object(
                record
                    .relations
                    .iter()
                    .map(|(relation, targets)| {
                        (
                            relation.name().to_owned(),
                            Value::integer(i64::try_from(targets.len()).unwrap_or(0)),
                        )
                    })
                    .collect(),
            ),
        ));
        fields.push((
            "claim_count",
            Value::integer(i64::try_from(record.claims.len()).unwrap_or(0)),
        ));
        fields.push(("freshness", freshness_json(&freshness, &record.id)));
        fields.push(("sources", sources_json(record)));
        return Ok(Output::text(text).with_result(Value::object(fields)));
    }
    fields.push((
        "slots",
        Value::Object(
            record
                .content
                .iter()
                .map(|(slot, value)| (slot.name().to_owned(), content_json(value)))
                .collect(),
        ),
    ));
    fields.push((
        "claims",
        Value::array(
            record
                .claims
                .iter()
                .map(|claim| {
                    Value::object(vec![
                        ("anchor", Value::string(claim.anchor.to_string())),
                        ("text", Value::string(claim.text.clone())),
                        (
                            "retired",
                            Value::bool(record.retired_claims.contains(&claim.anchor)),
                        ),
                    ])
                })
                .collect(),
        ),
    ));
    if let Some(acceptance) = &record.acceptance
        && !acceptance.checks.is_empty()
    {
        let verdicts = akr_core::resolve::acceptance_verdicts(ledger);
        fields.push((
            "acceptance",
            Value::array(
                acceptance
                    .checks
                    .iter()
                    .map(|check| {
                        let mut check_fields = vec![
                            ("id", Value::string(check.id.to_string())),
                            ("statement", Value::string(check.statement.clone())),
                            ("method", Value::string(check.method.name())),
                        ];
                        if let Some(command) = &check.command {
                            check_fields.push(("command", Value::string(command.clone())));
                        }
                        check_fields.push((
                            "verified_by",
                            Value::array(
                                check
                                    .verified_by
                                    .iter()
                                    .map(|r| Value::string(r.to_string()))
                                    .collect(),
                            ),
                        ));
                        check_fields.push((
                            "verdict",
                            Value::string(
                                verdicts
                                    .iter()
                                    .find(|v| v.owner == record.id && v.check == check.id)
                                    .map_or_else(
                                        || "no evidence".to_owned(),
                                        |v| verdict_line(&v.verdict),
                                    ),
                            ),
                        ));
                        Value::object(check_fields)
                    })
                    .collect(),
            ),
        ));
    }
    if relations {
        fields.push(("relations", relations_json(session, record)));
    }
    fields.push(("freshness", freshness_json(&freshness, &record.id)));
    if history {
        fields.push((
            "history",
            Value::array(
                ledger
                    .revisions_of(&record.id.key)
                    .iter()
                    .map(|revision| {
                        Value::object(vec![
                            ("rev", Value::integer(i64::from(revision.id.revision))),
                            ("state", Value::string(revision.state.name())),
                            ("title", Value::string(revision.title.clone())),
                        ])
                    })
                    .collect(),
            ),
        ));
    }
    fields.push(("sources", sources_json(record)));
    // Canonical syntax is the biggest thing in the payload and the least often wanted, so
    // it is an explicit request rather than the default (`sources/context-reduction.md`).
    if detail == Detail::Canonical
        && let Some(source) = session.inputs.canonical_text.get(&record.id)
    {
        fields.push(("source_text", Value::string(source.clone())));
    }
    Ok(Output::text(text).with_result(Value::object(fields)))
}

/// One acceptance verdict, in plain terminal words.
///
/// The Markdown renderer's phrasing without its backticks: `get` prints to a terminal and
/// feeds a JSON field, and neither wants `**satisfied**`.
fn verdict_line(verdict: &akr_core::resolve::Verdict) -> String {
    use akr_core::resolve::Verdict;
    match verdict {
        Verdict::Satisfied { by, .. } => format!("satisfied by @{by}"),
        Verdict::NoEvidence => "not satisfied — no evidence".to_owned(),
        Verdict::Unresolved => "not satisfied — the cited evidence does not resolve".to_owned(),
        Verdict::Failing { by, result } => format!(
            "not satisfied — @{by} reports {}",
            result.map_or("no result", akr_core::model::EvidenceResult::name)
        ),
        Verdict::TooOld { by, .. } => format!("not satisfied — @{by} predates the last change"),
    }
}

/// A record's provenance, as locators rather than copied text.
///
/// A citation into the registered library renders as its document and range; the excerpt
/// is not repeated, because `akr source get` can produce it from the exact bytes and a
/// second copy in every record is a second copy to keep honest (D-031).
fn sources_json(record: &akr_core::model::Record) -> Value {
    Value::array(
        record
            .sources
            .iter()
            .map(|source| {
                let mut fields = vec![(
                    "kind",
                    Value::string(match source.kind {
                        akr_core::model::SourceKind::Legacy => "legacy",
                        akr_core::model::SourceKind::External => "external",
                        akr_core::model::SourceKind::Internal => "internal",
                    }),
                )];
                if let Some(document) = &source.document {
                    fields.push(("document", Value::string(document.clone())));
                    fields.push(("standing", Value::string("non_authoritative")));
                }
                if let Some(path) = &source.path {
                    fields.push(("path", Value::string(path.clone())));
                }
                if let Some(url) = &source.url {
                    fields.push(("url", Value::string(url.clone())));
                }
                if let Some(range) = &source.range {
                    fields.push(("start_byte", Value::integer(range.start_byte as i64)));
                    fields.push(("end_byte", Value::integer(range.end_byte as i64)));
                    fields.push(("start_line", Value::integer(i64::from(range.start_line))));
                    fields.push(("end_line", Value::integer(i64::from(range.end_line))));
                }
                Value::object(fields)
            })
            .collect(),
    )
}

/// One scope term, in the object form of `docs/08-mcp.md` §3.
fn scope_json(term: &akr_core::model::ScopeTerm) -> Value {
    match term {
        akr_core::model::ScopeTerm::All => Value::object(vec![("form", Value::string("all"))]),
        akr_core::model::ScopeTerm::Path(glob) => Value::object(vec![
            ("form", Value::string("path")),
            ("glob", Value::string(glob.as_str())),
        ]),
        akr_core::model::ScopeTerm::Ref(reference) => Value::object(vec![
            ("form", Value::string("ref")),
            ("ref", Value::string(reference.to_string())),
        ]),
    }
}

/// A content slot's value, as JSON. Arrays stay arrays; everything else is a string.
fn content_json(value: &akr_core::model::ContentValue) -> Value {
    use akr_core::model::ContentValue;
    match value {
        ContentValue::Prose(text) | ContentValue::Text(text) => Value::string(text.clone()),
        ContentValue::Date(date) => Value::string(date.to_string()),
        ContentValue::Commit(commit) => Value::string(format!("git:{}", commit.as_str())),
        ContentValue::Enum(word) => Value::string(word.to_string()),
        ContentValue::Strings(items) => {
            Value::array(items.iter().map(|s| Value::string(s.clone())).collect())
        }
        ContentValue::Globs(items) => {
            Value::array(items.iter().map(|g| Value::string(g.as_str())).collect())
        }
        ContentValue::Refs(items) => {
            Value::array(items.iter().map(|r| Value::string(r.to_string())).collect())
        }
    }
}

/// Outbound and inbound relations, both resolved.
fn relations_json(session: &Session, record: &akr_core::model::Record) -> Value {
    let ledger = &session.ledger;
    let mut outbound = Vec::new();
    for (relation, references) in &record.relations {
        for target in references {
            outbound.push(Value::object(vec![
                ("relation", Value::string(relation.name())),
                ("ref", Value::string(target.to_string())),
            ]));
        }
    }
    let mut inbound = Vec::new();
    // Key order, so two runs agree (`docs/06-compiler-pipeline.md` §11).
    let mut others: Vec<&akr_core::model::Record> = ledger.records().iter().collect();
    others.sort_by(|a, b| a.id.cmp(&b.id));
    for other in others {
        for (relation, references) in &other.relations {
            for reference in references {
                if ledger
                    .resolve(reference)
                    .ok()
                    .flatten()
                    .is_some_and(|t| t.id == record.id)
                {
                    inbound.push(Value::object(vec![
                        ("relation", Value::string(relation.name())),
                        ("ref", Value::string(format!("@{}", other.id))),
                    ]));
                }
            }
        }
    }
    Value::object(vec![
        ("outbound", Value::array(outbound)),
        ("inbound", Value::array(inbound)),
    ])
}

/// A record's freshness, as `docs/08-mcp.md` §3 shows it.
fn freshness_json(freshness: &akr_core::render::Freshness, id: &RevisionId) -> Value {
    let mut fields = vec![
        ("stale", Value::bool(freshness.is_stale(id))),
        ("at_risk", Value::bool(freshness.at_risk(id).is_some())),
    ];
    if let Some(entry) = freshness.at_risk(id) {
        fields.push(("depth", Value::integer(entry.depth as i64)));
        fields.push((
            "path",
            Value::array(
                entry
                    .path
                    .iter()
                    .map(|step| Value::string(format!("@{step}")))
                    .collect(),
            ),
        ));
    }
    Value::object(fields)
}

fn why_current(session: &Session, reference: &str) -> Result<Output, EnvError> {
    let model = session.resolve();
    let ledger = &session.ledger;
    let parsed = Reference::parse(reference)
        .map_err(|e| EnvError::new("AKR-L001", format!("{reference}: {e}")))?;
    let Some(record) = ledger.resolve(&parsed).ok().flatten() else {
        return Err(EnvError::new(
            "AKR-L001",
            format!("{reference} does not resolve to a record"),
        ));
    };
    let key = &record.id.key;

    let mut text = format!("{key} — head is revision {}\n", record.id.revision);
    text.push_str("\n  head resolution\n");
    for revision in ledger.revisions_of(key) {
        let marker = if model.is_head(&revision.id) {
            "  -> head"
        } else {
            ""
        };
        text.push_str(&format!(
            "    revision {}  {:<12} {}{marker}\n",
            revision.id.revision,
            revision.state.name(),
            if revision.is_live() { "LIVE" } else { "    " }
        ));
    }
    if let Some(hash) = model.content_hash(&record.id) {
        text.push_str(&format!(
            "    lock: resolved to /{}, hash {}\n",
            record.id.revision,
            &hash.0[..20]
        ));
    }

    let queue = session.review_queue();
    let freshness = session.freshness(&queue);
    text.push_str("\n  freshness\n");
    if let Some(stale) = queue.stale.iter().find(|s| s.id == record.id) {
        text.push_str(&format!(
            "    STALE: {}\n",
            cause_text(&stale.cause, session.today)
        ));
    } else if let Some(flag) = freshness.at_risk(&record.id) {
        text.push_str(&format!("    AT RISK (depth {})\n", flag.depth));
        for hop in &flag.path {
            text.push_str(&format!("      via {} -> @{hop}\n", flag.via));
        }
    } else {
        text.push_str("    not stale, not at risk\n");
    }

    Ok(Output::text(text).with_result(Value::object(vec![
        ("key", Value::string(key.to_string())),
        ("head", Value::integer(i64::from(record.id.revision))),
    ])))
}

// -------------------------------------------------------------------------------------
// impact, review-queue
// -------------------------------------------------------------------------------------

fn impact(
    session: &Session,
    reference: Option<&str>,
    git_diff: Option<&str>,
) -> Result<Output, EnvError> {
    let ledger = &session.ledger;
    if let Some(range) = git_diff {
        return impact_range(session, range);
    }

    let reference = reference.expect("argument parsing requires one of the two");
    let parsed = Reference::parse(reference)
        .map_err(|e| EnvError::new("AKR-L001", format!("{reference}: {e}")))?;
    let Some(record) = ledger.resolve(&parsed).ok().flatten() else {
        return Err(EnvError::new(
            "AKR-L001",
            format!("{reference} does not resolve to a record"),
        ));
    };
    let queue = session.review_queue();
    let stale: BTreeSet<RevisionId> = [record.id.clone()].into();
    let dependents = akr_core::graph::propagate_staleness(ledger, &stale);

    let flag = if queue.stale_set().contains(&record.id) {
        "  [STALE]"
    } else {
        ""
    };
    let mut text = format!(
        "@{}  {}  {}{flag}\n\ndependents along supported_by, depends_on, derived_from\n\n",
        record.id,
        record.kind.name(),
        record.state.name()
    );
    let id_width = width(dependents.iter().map(|d| d.id.to_string().len() + 1), 3);
    let kind_width = width(
        dependents
            .iter()
            .filter_map(|d| ledger.get(&d.id))
            .map(|r| r.kind.name().len()),
        2,
    );
    for entry in &dependents {
        let dependent = ledger.get(&entry.id);
        text.push_str(&format!(
            "  depth {}  {:<id_width$}{:<kind_width$}{}\n",
            entry.depth,
            format!("@{}", entry.id),
            dependent.map_or("", |r| r.kind.name()),
            dependent.map_or("", |r| r.state.name()),
        ));
        text.push_str(&via_lines(&entry.via, &entry.path, 13));
    }
    if dependents.is_empty() {
        text.push_str("  (none)\n");
    }
    text.push_str(&format!(
        "\n{} dependent{}, maximum depth {}\n",
        dependents.len(),
        if dependents.len() == 1 { "" } else { "s" },
        dependents.iter().map(|d| d.depth).max().unwrap_or(0)
    ));

    let result = Value::object(vec![
        ("mode", Value::string("ref")),
        ("key", Value::string(record.id.key.to_string())),
        ("rev", Value::integer(i64::from(record.id.revision))),
        (
            "dependents",
            Value::array(
                dependents
                    .iter()
                    .map(|d| {
                        Value::object(vec![
                            ("key", Value::string(d.id.key.to_string())),
                            ("rev", Value::integer(i64::from(d.id.revision))),
                            ("depth", Value::integer(d.depth as i64)),
                            ("via", Value::string(d.via.name())),
                            (
                                "path",
                                Value::array(
                                    d.path
                                        .iter()
                                        .map(|id| Value::string(id.key.to_string()))
                                        .collect(),
                                ),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
    ]);
    Ok(Output::text(text).with_result(result))
}

/// `akr impact --git-diff A..B`: what a range would make questionable
/// (`docs/10-freshness-and-git.md` §6).
fn impact_range(session: &Session, range: &str) -> Result<Output, EnvError> {
    let ledger = &session.ledger;
    let Some(repository) = &session.repository else {
        return Err(EnvError::new("AKR-G001", "not inside a git repository"));
    };
    // An unresolvable end is a diagnostic, not an environment failure: the checkout is
    // fine, the argument is not. `docs/07-cli.md` §3 puts `AKR-G013` outside both the
    // usage list and the environment list, which leaves exit 1.
    let Some((from, to)) = range.split_once("..") else {
        return Ok(revision_error(range, "expected A..B"));
    };
    // `git rev-parse` resolves a branch or `HEAD`; a 40-hex commit that is not in the
    // repository resolves too, so both ends are checked for membership as well.
    let resolve = |text: &str| -> Option<akr_core::model::Commit> {
        let commit = repository.rev_parse(text).ok()?;
        repository.contains(&commit).then_some(commit)
    };
    let (Some(from), Some(to)) = (resolve(from), resolve(to)) else {
        let bad = if resolve(from).is_none() { from } else { to };
        return Ok(revision_error(
            bad,
            &format!("{bad:?} is not a commit in this repository"),
        ));
    };

    // The baseline is A, not HEAD. Impact asks what the range *would* make questionable,
    // so a record that this very range invalidated must not be filtered out for being
    // stale at HEAD — which it is precisely because the range happened.
    let already = akr_core::freshness::derive(ledger, repository, &from, session.today)
        .map(|queue| queue.stale_set())
        .unwrap_or_default();
    let impact = akr_core::freshness::impact_of_range(ledger, repository, &from, &to, &already)
        .map_err(|e| EnvError::new("AKR-G002", e.to_string()))?;

    let mut text = format!(
        "range {}..{}, {} commit{}\n\n",
        &from.as_str()[..8],
        &to.as_str()[..8],
        impact.commits,
        if impact.commits == 1 { "" } else { "s" }
    );
    text.push_str("touched paths\n");
    for path in &impact.touched {
        text.push_str(&format!("  {path}\n"));
    }

    if impact.newly_stale.is_empty() {
        text.push_str("\nnewly stale:   none\n");
    } else {
        text.push_str(&format!("\nnewly stale ({})\n", impact.newly_stale.len()));
        let id_width = width(
            impact
                .newly_stale
                .iter()
                .map(|s| s.id.to_string().len() + 1),
            3,
        );
        let kind_width = width(
            impact
                .newly_stale
                .iter()
                .filter_map(|s| ledger.get(&s.id))
                .map(|r| r.kind.name().len()),
            2,
        );
        for entry in &impact.newly_stale {
            let record = ledger.get(&entry.id);
            text.push_str(&format!(
                "  {:<id_width$}{:<kind_width$}{}\n",
                format!("@{}", entry.id),
                record.map_or("", |r| r.kind.name()),
                record.map_or("", |r| r.state.name()),
            ));
            text.push_str(&format!(
                "      {}\n",
                cause_text(&entry.cause, session.today)
            ));
            if let (Some(observed), akr_core::freshness::StaleCause::Watch { commit, .. }) =
                (&entry.observed_at, &entry.cause)
            {
                text.push_str(&format!(
                    "      observed_at {}, which {} descends from\n",
                    &observed.as_str()[..8],
                    &commit.as_str()[..8]
                ));
            }
        }
    }

    if impact.newly_at_risk.is_empty() {
        text.push_str("newly at risk: none\n");
    } else {
        text.push_str(&format!(
            "\nnewly at risk ({})\n",
            impact.newly_at_risk.len()
        ));
        let id_width = width(
            impact
                .newly_at_risk
                .iter()
                .map(|r| r.id.to_string().len() + 1),
            3,
        );
        for entry in &impact.newly_at_risk {
            text.push_str(&format!(
                "  {:<id_width$}depth {}\n",
                format!("@{}", entry.id),
                entry.depth
            ));
            text.push_str(&via_lines_bare(&entry.via, &entry.path, 6));
        }
    }

    let result = Value::object(vec![
        ("mode", Value::string("git_diff")),
        (
            "range",
            Value::object(vec![
                ("from", Value::string(from.as_str())),
                ("to", Value::string(to.as_str())),
                ("commits", Value::integer(impact.commits as i64)),
            ]),
        ),
        (
            "touched_paths",
            Value::array(impact.touched.iter().map(Value::string).collect()),
        ),
        (
            "newly_stale",
            Value::array(
                impact
                    .newly_stale
                    .iter()
                    .map(|s| stale_json(session, s))
                    .collect(),
            ),
        ),
        (
            "newly_at_risk",
            Value::array(
                impact
                    .newly_at_risk
                    .iter()
                    .map(|r| at_risk_json(session, r))
                    .collect(),
            ),
        ),
    ]);
    Ok(Output::text(text).with_result(result))
}

/// The `AKR-G013` diagnostic a bad `--git-diff` end raises.
fn revision_error(argument: &str, message: &str) -> Output {
    let diagnostic = Diagnostic {
        code: akr_core::git::codes::G013,
        severity: akr_core::diagnostics::Severity::Error,
        rule: None,
        message: format!("--git-diff: {message}"),
        primary: akr_core::diagnostics::Label::new(akr_core::diagnostics::Subject::File(
            argument.to_owned(),
        )),
        notes: Vec::new(),
        help: Some("AKR takes full 40-hex commits, never abbreviations (D-008)".to_owned()),
    };
    let text = format!(
        "{}\n1 error\n",
        akr_core::diagnostics::render(&diagnostic, &akr_core::diagnostics::SourceMap::new())
    );
    Output {
        text,
        result: Value::Object(Vec::new()),
        diagnostics: vec![diagnostic],
        exit: Exit::Diagnostics,
        commit: None,
        source_graph: String::new(),
    }
}

/// A column width: the widest entry plus a gap, or nothing when there are no entries.
fn width(lengths: impl Iterator<Item = usize>, gap: usize) -> usize {
    lengths.max().map_or(0, |longest| longest + gap)
}

/// `via <relation> -> @a` and, for a deeper path, the continuation lines under it.
fn via_lines(via: &akr_core::model::Relation, path: &[RevisionId], indent: usize) -> String {
    let pad = " ".repeat(indent);
    let relation = format!("{:<12}", via.to_string());
    let hang = " ".repeat(indent + 4 + 12 + 1);
    let mut out = String::new();
    for (index, id) in path.iter().enumerate() {
        if index == 0 {
            out.push_str(&format!("{pad}via {relation} -> @{id}\n"));
        } else {
            out.push_str(&format!("{hang}-> @{id}\n"));
        }
    }
    out
}

/// The same, without the leading `via` keyword.
fn via_lines_bare(via: &akr_core::model::Relation, path: &[RevisionId], indent: usize) -> String {
    let pad = " ".repeat(indent);
    let hang = " ".repeat(indent + 12 + 1);
    let mut out = String::new();
    for (index, id) in path.iter().enumerate() {
        if index == 0 {
            out.push_str(&format!("{pad}{:<12} -> @{id}\n", via.to_string()));
        } else {
            out.push_str(&format!("{hang}-> @{id}\n"));
        }
    }
    out
}

fn review_queue(
    session: &Session,
    stale_only: bool,
    at_risk_only: bool,
    kinds: &[String],
) -> Result<Output, EnvError> {
    let ledger = &session.ledger;
    let queue = session.review_queue();
    let wanted: Vec<Kind> = kinds.iter().filter_map(|k| Kind::from_name(k)).collect();
    let keep = |id: &RevisionId| {
        wanted.is_empty()
            || ledger
                .get(id)
                .is_some_and(|record| wanted.contains(&record.kind))
    };

    let stale: Vec<_> = queue
        .stale
        .iter()
        .filter(|s| !at_risk_only && keep(&s.id))
        .collect();
    let at_risk: Vec<_> = queue
        .at_risk
        .iter()
        .filter(|r| !stale_only && keep(&r.id))
        .collect();

    let mut text = format!(
        "review queue at {}, today {}\n",
        session
            .commit
            .as_ref()
            .map_or_else(|| "(no commit)".to_owned(), |c| c.as_str()[..8].to_owned()),
        session.today
    );

    if !at_risk_only {
        text.push_str(&format!("\nSTALE ({})\n", stale.len()));
        let id_width = width(stale.iter().map(|s| s.id.to_string().len()), 3);
        let kind_width = width(
            stale
                .iter()
                .filter_map(|s| ledger.get(&s.id))
                .map(|r| r.kind.name().len()),
            2,
        );
        for entry in &stale {
            let Some(record) = ledger.get(&entry.id) else {
                continue;
            };
            text.push_str(&format!(
                "\n  {:<id_width$}{:<kind_width$}{}\n",
                entry.id.to_string(),
                record.kind.name(),
                record.state.name()
            ));
            text.push_str(&format!(
                "      cause      {}\n",
                cause_text(&entry.cause, session.today)
            ));
            if let Some(observed) = &entry.observed_at {
                text.push_str(&format!(
                    "      observed   {}{}\n",
                    &observed.as_str()[..8],
                    distance_note(session, observed, &entry.cause)
                ));
            }
            // Every watch glob, not only the one that matched: a reader deciding whether
            // to re-observe needs to know what else the record is watching.
            for glob in akr_core::freshness::watches(record) {
                let matched = matches!(
                    &entry.cause,
                    akr_core::freshness::StaleCause::Watch { glob: g, .. } if *g == glob
                );
                text.push_str(&format!(
                    "      watches    {:?}{}\n",
                    glob.as_str(),
                    if matched {
                        String::new()
                    } else {
                        " — not matched since observed_at".to_owned()
                    }
                ));
            }
            if let Some(date) = akr_core::freshness::review_after(record)
                && !matches!(
                    entry.cause,
                    akr_core::freshness::StaleCause::ReviewAfter { .. }
                )
            {
                text.push_str(&format!("      review     {date} (not yet due)\n"));
            }
            let contradicts: Vec<String> = record
                .targets(akr_core::model::Relation::Contradicts)
                .iter()
                .map(std::string::ToString::to_string)
                .collect();
            if !contradicts.is_empty() {
                text.push_str(&format!(
                    "      note       contradicts {}\n                 ({})\n",
                    contradicts.join(", "),
                    if record.acknowledged {
                        "acknowledged"
                    } else {
                        "not acknowledged"
                    }
                ));
            }
        }
    }
    if !stale_only {
        text.push_str(&format!("\nAT RISK ({})\n", at_risk.len()));
        let id_width = width(at_risk.iter().map(|r| r.id.to_string().len()), 3);
        let kind_width = width(
            at_risk
                .iter()
                .filter_map(|r| ledger.get(&r.id))
                .map(|r| r.kind.name().len()),
            2,
        );
        for entry in &at_risk {
            let record = ledger.get(&entry.id);
            text.push_str(&format!(
                "\n  depth {}  {:<id_width$}{:<kind_width$}{}\n",
                entry.depth,
                entry.id.to_string(),
                record.map_or("", |r| r.kind.name()),
                record.map_or("", |r| r.state.name())
            ));
            text.push_str(&via_lines(&entry.via, &entry.path, 13));
        }
    }
    let mut summary = Vec::new();
    if !at_risk_only {
        summary.push(format!("{} stale", stale.len()));
    }
    if !stale_only {
        summary.push(format!("{} at risk", at_risk.len()));
        if let Some(depth) = at_risk.iter().map(|r| r.depth).max() {
            summary.push(format!("maximum propagation depth {depth}"));
        }
    }
    text.push_str(&format!("\n{}\n", summary.join(", ")));

    let result = Value::object(vec![
        ("today", Value::string(session.today.to_string())),
        (
            "stale",
            Value::array(stale.iter().map(|s| stale_json(session, s)).collect()),
        ),
        (
            "at_risk",
            Value::array(at_risk.iter().map(|r| at_risk_json(session, r)).collect()),
        ),
        (
            "counts",
            Value::object(vec![
                ("stale", Value::integer(stale.len() as i64)),
                ("at_risk", Value::integer(at_risk.len() as i64)),
                (
                    "max_depth",
                    Value::integer(at_risk.iter().map(|r| r.depth).max().unwrap_or(0) as i64),
                ),
            ]),
        ),
    ]);
    // Exit 0 regardless of queue length: a non-empty queue is normal and healthy.
    Ok(Output::text(text).with_result(result))
}

/// How far the observation sits behind the commit that invalidated it.
///
/// Purely informative, and silently absent when git cannot answer — a missing count is
/// better than an impact report that fails because a commit was garbage-collected.
fn distance_note(
    session: &Session,
    observed: &akr_core::model::Commit,
    cause: &akr_core::freshness::StaleCause,
) -> String {
    let akr_core::freshness::StaleCause::Watch { commit, .. } = cause else {
        return String::new();
    };
    let Some(repository) = &session.repository else {
        return String::new();
    };
    match repository.commits_in(Some(observed), commit) {
        Ok(commits) if !commits.is_empty() => format!(
            " ({} commit{} behind the match)",
            commits.len(),
            if commits.len() == 1 { "" } else { "s" }
        ),
        _ => String::new(),
    }
}

/// The JSON form of a stale entry (`docs/07-cli.md` §5).
fn stale_json(session: &Session, entry: &akr_core::freshness::Stale) -> Value {
    let record = session.ledger.get(&entry.id);
    let mut fields = vec![
        ("key", Value::string(entry.id.key.to_string())),
        ("rev", Value::integer(i64::from(entry.id.revision))),
        ("kind", Value::string(record.map_or("", |r| r.kind.name()))),
        (
            "state",
            Value::string(record.map_or("", |r| r.state.name())),
        ),
        ("cause", Value::string(entry.cause.name())),
    ];
    match &entry.cause {
        akr_core::freshness::StaleCause::Watch { glob, commit, path } => {
            fields.push(("glob", Value::string(glob.as_str())));
            fields.push(("matched_by", Value::string(commit.as_str())));
            fields.push(("matched_path", Value::string(path.clone())));
        }
        akr_core::freshness::StaleCause::ReviewAfter { date } => {
            fields.push(("review_after", Value::string(date.to_string())));
            fields.push((
                "days_overdue",
                Value::integer(days_between(*date, session.today)),
            ));
        }
    }
    if let Some(observed) = &entry.observed_at {
        fields.push(("observed_at", Value::string(observed.as_str())));
    }
    Value::object(fields)
}

/// The JSON form of an at-risk entry.
fn at_risk_json(session: &Session, entry: &akr_core::graph::AtRisk) -> Value {
    let record = session.ledger.get(&entry.id);
    Value::object(vec![
        ("key", Value::string(entry.id.key.to_string())),
        ("rev", Value::integer(i64::from(entry.id.revision))),
        ("kind", Value::string(record.map_or("", |r| r.kind.name()))),
        ("depth", Value::integer(entry.depth as i64)),
        ("via", Value::string(entry.via.name())),
        (
            "path",
            Value::array(
                entry
                    .path
                    .iter()
                    .map(|id| Value::string(id.key.to_string()))
                    .collect(),
            ),
        ),
    ])
}

// -------------------------------------------------------------------------------------
// lock, context
// -------------------------------------------------------------------------------------

fn lock(session: &mut Session, check_only: bool) -> Result<Output, EnvError> {
    session.attach_lock();
    let model = session.resolve();
    let computed = model.to_lock();

    if !check_only {
        let path = session.akr_dir.join("akr.lock");
        std::fs::write(&path, computed.render())
            .map_err(|e| EnvError::new("AKR-C042", format!("cannot write akr.lock: {e}")))?;
        return Ok(Output::text("wrote akr.lock\n"));
    }

    let Some(text) = &session.lock_text else {
        return Err(EnvError::new("AKR-R052", "akr.lock is missing").help("run `akr build`"));
    };
    let recorded = akr_core::lock::Lock::parse(text)
        .map_err(|e| EnvError::new("AKR-R052", format!("akr.lock does not parse: {e}")))?;
    let mut diagnostics =
        akr_core::lock::currency_diagnostics(&recorded, &computed, ".akr/akr.lock");
    diagnostics.extend(
        model
            .diagnostics
            .iter()
            .filter(|d| {
                d.code == akr_core::diagnostics::codes::R051
                    || d.code == akr_core::diagnostics::codes::R052
            })
            .cloned(),
    );
    session.spans.attach_all(&mut diagnostics);

    let exit = if diagnostics.is_empty() {
        Exit::Ok
    } else {
        Exit::Diagnostics
    };
    let out = if diagnostics.is_empty() {
        "akr.lock is current\n".to_owned()
    } else {
        report(&diagnostics, &session.sources, session.global.profile).0
    };
    Ok(Output::text(out)
        .with_result(Value::object(vec![(
            "mismatches",
            Value::integer(diagnostics.len() as i64),
        )]))
        .with_diagnostics(diagnostics, exit))
}

/// `akr search` — the one command that reads the index cache.
///
/// **Search ranks; it never authorises** (`docs/09-context-assembly.md` §1). Nothing here
/// feeds `akr context`: this is a navigation aid for somebody who already knows roughly
/// what they are looking for, and the separation is structural — the assembler does not
/// call this function and could not, because ranking lives in the store module and
/// assembly does not import it.
///
/// Exit 0 even with zero results. An empty result set is an answer.
#[cfg(feature = "fts5")]
fn ensure_search_index(session: &mut Session, path: &Path) -> Result<(), EnvError> {
    let needs_rebuild =
        !path.exists() || akr_core::store::is_stale_against(path, &session.source_graph());
    if !needs_rebuild {
        return Ok(());
    }
    if session.global.no_rebuild {
        return Err(EnvError::new(
            "AKR-I031",
            "index is missing or stale and rebuilding is disabled",
        )
        .help("drop `--no-rebuild` to refresh the disposable cache, or run `akr build`"));
    }

    // A current index is a pure ledger lookup and deliberately reaches this point
    // without spawning Git.  Rebuilding materialises freshness, so upgrade only here.
    session.ensure_git_facts()?;

    // Search is a read of the ledger, but its SQLite index is explicitly disposable.
    // Rebuild only that cache: never the lock, generated views, or source records.
    session.attach_lock();
    let model = session.resolve();
    let diagnostics = session.diagnostics(&model);
    let queue = session.review_queue();
    index_build(session, &model, &queue, &diagnostics)?;
    Ok(())
}

#[cfg(feature = "fts5")]
fn search(
    session: &mut Session,
    query: &str,
    raw_fts: bool,
    kinds: &[String],
    states: &[String],
    limit: Option<usize>,
) -> Result<Output, EnvError> {
    let request = akr_core::store::Request {
        query: query.to_owned(),
        raw_fts,
        kinds: kinds.to_vec(),
        states: states.to_vec(),
        limit,
    };
    let path = akr_core::store::cache_path(&session.akr_dir);
    ensure_search_index(session, &path)?;
    let hits = akr_core::store::search(&path, &request)
        .map_err(|error| EnvError::new(error.code.as_str(), error.message))?;

    // Columns are padded to the widest cell, as `docs/07-cli.md` §6 shows them. Results are
    // read down the page rather than across, and ragged columns make that work.
    let reference: Vec<String> = hits
        .iter()
        .map(|hit| format!("{}/{}", hit.key, hit.rev))
        .collect();
    let widest = |cells: &[String]| cells.iter().map(String::len).max().unwrap_or(0);
    let kinds: Vec<String> = hits.iter().map(|hit| hit.kind.clone()).collect();
    let states: Vec<String> = hits.iter().map(|hit| hit.state.clone()).collect();
    let (key_width, kind_width, state_width) =
        (widest(&reference), widest(&kinds), widest(&states));

    let mut text = String::new();
    for (index, hit) in hits.iter().enumerate() {
        text.push_str(&format!(
            "  {:.2}  {:key_width$}  {:kind_width$} {:state_width$}  {}\n",
            hit.score, reference[index], kinds[index], states[index], hit.title
        ));
    }
    text.push_str(&format!(
        "{} result{}\n",
        hits.len(),
        if hits.len() == 1 { "" } else { "s" }
    ));

    let results: Vec<Value> = hits
        .iter()
        .map(|hit| {
            Value::object(vec![
                ("key", Value::string(hit.key.clone())),
                ("rev", Value::integer(i64::from(hit.rev))),
                ("kind", Value::string(hit.kind.clone())),
                ("state", Value::string(hit.state.clone())),
                ("title", Value::string(hit.title.clone())),
                ("score", Value::string(format!("{:.2}", hit.score))),
            ])
        })
        .collect();
    Ok(Output::text(text).with_result(Value::object(vec![
        ("query", Value::string(query.to_owned())),
        ("results", Value::array(results)),
        ("count", Value::integer(hits.len() as i64)),
        ("index_stale", Value::bool(false)),
        ("cache_stale", Value::bool(false)),
        ("backend", Value::string("fts_index")),
        ("ledger_revision", Value::string(session.source_graph())),
        (
            "cache_revision",
            Value::string(
                akr_core::store::cached_source_graph_hash(&path)
                    .map(|hash| format!("sha256:{hash}"))
                    .unwrap_or_else(|| "unknown".to_owned()),
            ),
        ),
    ])))
}

/// `akr start` — orient an ambiguous task before committing to a context anchor.
#[cfg(feature = "fts5")]
fn start(
    session: &mut Session,
    task: &str,
    paths: &[akr_core::model::Glob],
    budget: Option<usize>,
) -> Result<Output, EnvError> {
    let handoff = crate::handoff::assemble(session, budget)?;
    let output = if let Some(ledger) = &handoff.fallback_ledger {
        planning_search_fallback(ledger, task)
    } else {
        search(
            session,
            task,
            false,
            &["milestone".into(), "work".into(), "track".into()],
            &[],
            None,
        )?
    };
    Ok(finish_start(
        output,
        &session.root,
        task,
        paths,
        budget,
        handoff,
    ))
}

fn finish_start(
    mut output: Output,
    root: &Path,
    task: &str,
    paths: &[akr_core::model::Glob],
    budget: Option<usize>,
    handoff: crate::handoff::Handoff,
) -> Output {
    let results = output
        .result
        .get("results")
        .and_then(Value::as_array)
        .map(|results| results.to_vec())
        .unwrap_or_default();
    if let Value::Object(fields) = &mut output.result {
        if results.is_empty() {
            let (fallback, hit_count) = workspace_fallback(root, task);
            output.text.push_str(
                "\nNo live AKR planning record matched this task. This does not mean no plan exists.\n\
                 Next: read any user-supplied plan, inspect Git history, register outside advice\n\
                 with `akr source add`, then adopt the governing intent into an AKR record.\n",
            );
            if hit_count > 0 {
                output.text.push_str(&format!(
                    "Workspace fallback found {hit_count} non-authoritative text hit(s); inspect them before adopting a plan.\n"
                ));
            }
            fields.push((
                "coverage".into(),
                Value::object(vec![
                    ("status", Value::string("no_planning_match")),
                    (
                        "message",
                        Value::string(
                            "No live AKR planning record matched this task; this is a ledger-coverage result, not proof that no plan exists.",
                        ),
                    ),
                    (
                        "next_steps",
                        Value::array(vec![
                            Value::string("Read any user-supplied plan or path before broadening the query."),
                            Value::string("Inspect relevant Git history and the current worktree."),
                            Value::string("Register outside advice with `akr source add` (or `knowledge.source_add`) when it should remain provenance."),
                            Value::string("Adopt the governing intent into a self-contained AKR planning record before continuing implementation."),
                        ]),
                    ),
                ]),
            ));
            fields.push(("workspace_fallback".into(), fallback));
        } else {
            fields.push(("planning_candidates".into(), Value::array(results.clone())));
            if task.contains('/') || task.contains('\\') {
                let (fallback, hit_count) = workspace_fallback(root, task);
                if hit_count > 0 {
                    output.text.push_str(&format!(
                        "Workspace fallback also found {hit_count} non-authoritative text hit(s) for the supplied path; inspect them before adopting a plan.\n"
                    ));
                    fields.push(("workspace_fallback".into(), fallback));
                }
            }
        }
        if results.len() == 1
            && let Some(goal) = results[0].get("key").and_then(Value::as_str)
        {
            let mut arguments = vec![("goal".into(), Value::string(goal.to_owned()))];
            if !paths.is_empty() {
                arguments.push((
                    "paths".into(),
                    Value::array(
                        paths
                            .iter()
                            .map(|path| Value::string(path.as_str()))
                            .collect(),
                    ),
                ));
            }
            if let Some(budget) = budget {
                arguments.push(("budget".into(), Value::integer(budget as i64)));
            }
            fields.push((
                "recommended_context".into(),
                Value::object(vec![
                    ("command", Value::string("akr context")),
                    ("arguments", Value::Object(arguments)),
                ]),
            ));
        }
        fields.push(("handoff".into(), handoff.value));
        if let Some((line, value)) = delegation_notice(root) {
            output.text.push_str(&line);
            fields.push(("handoff_session".into(), value));
        }
    }
    output.text = format!("{}{}", handoff.text, output.text);
    output
}

/// The line `akr start` prints when a handoff session is open.
///
/// A child agent should open the packet it was given rather than re-deriving the project
/// from scratch: that re-derivation is the whole cost the capsules exist to remove, and
/// five children paying it produce five accounts of one answer that do not always agree
/// (D-041). AKR cannot see process ancestry, so it cannot *know* that this caller is a
/// child — but the harness can say so, and when `AKR_HANDOFF_PACKET` names a packet this
/// says exactly what to run instead. Without it, the conditional is the honest form.
///
/// It is a line rather than a diagnostic. What is open in `.agent/handoffs/` is a fact
/// about the working tree, not a contradiction in the ledger, and facts about the working
/// tree never change an exit code (D-024, D-036).
fn delegation_notice(root: &Path) -> Option<(String, Value)> {
    let session = crate::handoff::packet::current_session(root)?;
    let assigned = std::env::var("AKR_HANDOFF_PACKET")
        .ok()
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty());
    let line = match &assigned {
        Some(packet) => format!(
            "\nhandoff      you hold packet {packet} — `akr handoff open {packet}` carries \
             this orientation already;\n             this call re-derived it.\n"
        ),
        None => format!(
            "\nhandoff      session {session} is open. If you were given a packet id, \
             `akr handoff open <id>`\n             instead: it inherits this orientation \
             and names what not to repeat.\n"
        ),
    };
    let value = Value::object(vec![
        ("session", Value::string(session)),
        (
            "assigned_packet",
            assigned.map_or(Value::Null, Value::string),
        ),
    ]);
    Some((line, value))
}

/// A deterministic ledger-only orientation when the working ledger is invalid and the
/// handoff has deliberately fallen back to the separately parsed HEAD snapshot. The
/// normal path remains FTS5; this path exists so an invalid overlay cannot force start to
/// query or rebuild an index from knowledge it has just declined to trust.
fn planning_search_fallback(ledger: &akr_core::model::Ledger, task: &str) -> Output {
    let terms: Vec<String> = task
        .split(|character: char| !character.is_alphanumeric())
        .map(|term| term.to_ascii_lowercase())
        .filter(|term| term.len() >= 3)
        .collect();
    let mut hits: Vec<(usize, &akr_core::model::Record)> = ledger
        .records()
        .iter()
        .filter(|record| {
            matches!(
                record.kind,
                akr_core::model::Kind::Milestone
                    | akr_core::model::Kind::Work
                    | akr_core::model::Kind::Track
            ) && record.is_live()
                && ledger
                    .head(&record.id.key)
                    .is_ok_and(|head| head.id == record.id)
        })
        .filter_map(|record| {
            let searchable = format!(
                "{} {} {}",
                record.id.key,
                record.title,
                record
                    .get(akr_core::model::ContentSlot::Intent)
                    .map_or("", |value| match value {
                        akr_core::model::ContentValue::Text(text)
                        | akr_core::model::ContentValue::Prose(text) => text,
                        _ => "",
                    })
            )
            .to_ascii_lowercase();
            let score = terms
                .iter()
                .filter(|term| searchable.contains(term.as_str()))
                .count();
            (score > 0).then_some((score, record))
        })
        .collect();
    hits.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.id.cmp(&right.1.id))
    });
    hits.truncate(20);

    let mut text = String::new();
    let results: Vec<Value> = hits
        .iter()
        .map(|(score, record)| {
            text.push_str(&format!(
                "  {:.2}  {}/{}  {} {}  {}\n",
                *score as f64,
                record.id.key,
                record.id.revision,
                record.kind.name(),
                record.state.name(),
                record.title
            ));
            Value::object(vec![
                ("key", Value::string(record.id.key.to_string())),
                ("rev", Value::integer(i64::from(record.id.revision))),
                ("kind", Value::string(record.kind.name())),
                ("state", Value::string(record.state.name())),
                ("title", Value::string(record.title.clone())),
                ("score", Value::string(format!("{:.2}", *score as f64))),
            ])
        })
        .collect();
    text.push_str(&format!("{} results\n", results.len()));
    Output::text(text).with_result(Value::object(vec![
        ("query", Value::string(task)),
        ("results", Value::array(results)),
        ("count", Value::integer(hits.len() as i64)),
        ("index_stale", Value::bool(false)),
        ("cache_stale", Value::bool(false)),
        ("backend", Value::string("validated_head_fallback")),
    ]))
}

/// A bounded, non-authoritative fallback for a task that has no ledger planning match.
///
/// This deliberately scans the working tree itself instead of invoking an MCP connector or
/// shell utility: it is available on every supported platform and can reveal untracked intake
/// documents without claiming that their contents are adopted project knowledge.
fn workspace_fallback(root: &Path, task: &str) -> (Value, usize) {
    const MAX_FILES: usize = 10_000;
    const MAX_FILE_BYTES: u64 = 1_000_000;
    const MAX_HITS: usize = 12;
    let mut terms: Vec<String> = task
        .split(|c: char| !c.is_alphanumeric())
        .map(|term| term.to_ascii_lowercase())
        .filter(|term| term.len() >= 4)
        .filter(|term| {
            !matches!(
                term.as_str(),
                "with"
                    | "from"
                    | "that"
                    | "this"
                    | "while"
                    | "into"
                    | "their"
                    | "which"
                    | "before"
                    | "after"
            )
        })
        .collect();
    terms.sort();
    terms.dedup();
    // A multi-word task needs corroboration from more than one useful term. This keeps a
    // generic word such as "work" from drowning out a specific untracked plan document.
    let minimum_score = if terms.len() > 1 { 2 } else { 1 };
    let mut pending = vec![root.to_path_buf()];
    let mut scanned = 0usize;
    let mut hits: Vec<(usize, String, usize, String)> = Vec::new();
    while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() {
                if !matches!(
                    name.as_ref(),
                    ".git" | ".akr" | "target" | "node_modules" | ".agent" | ".agents"
                ) {
                    pending.push(path);
                }
                continue;
            }
            if scanned >= MAX_FILES
                || entry
                    .metadata()
                    .map(|meta| meta.len() > MAX_FILE_BYTES)
                    .unwrap_or(true)
            {
                continue;
            }
            scanned += 1;
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let searchable = format!(
                "{}\n{}",
                relative.to_ascii_lowercase(),
                text.to_ascii_lowercase()
            );
            let score = terms
                .iter()
                .filter(|term| searchable.contains(term.as_str()))
                .count();
            if score < minimum_score {
                continue;
            }
            let best_line = text.lines().enumerate().max_by_key(|(_, line)| {
                let lower = line.to_ascii_lowercase();
                terms
                    .iter()
                    .filter(|term| lower.contains(term.as_str()))
                    .count()
            });
            let (line_index, line) = best_line.unwrap_or((0, ""));
            let excerpt: String = line.trim().chars().take(240).collect();
            hits.push((score, relative, line_index + 1, excerpt));
        }
    }
    hits.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
    });
    hits.truncate(MAX_HITS);
    let hit_count = hits.len();
    (
        Value::object(vec![
            (
                "provenance",
                Value::string("workspace text scan; non-authoritative and not AKR knowledge"),
            ),
            ("scanned_files", Value::integer(scanned as i64)),
            (
                "terms",
                Value::array(terms.into_iter().map(Value::string).collect()),
            ),
            (
                "hits",
                Value::array(
                    hits.into_iter()
                        .map(|(score, path, line, excerpt)| {
                            Value::object(vec![
                                ("path", Value::string(path)),
                                ("line", Value::integer(line as i64)),
                                ("excerpt", Value::string(excerpt)),
                                ("matched_terms", Value::integer(score as i64)),
                            ])
                        })
                        .collect(),
                ),
            ),
        ]),
        hit_count,
    )
}

#[cfg(not(feature = "fts5"))]
fn start(
    session: &mut Session,
    task: &str,
    paths: &[akr_core::model::Glob],
    budget: Option<usize>,
) -> Result<Output, EnvError> {
    let handoff = crate::handoff::assemble(session, budget)?;
    let ledger = handoff.fallback_ledger.as_ref().unwrap_or(&session.ledger);
    let output = planning_search_fallback(ledger, task);
    Ok(finish_start(
        output,
        &session.root,
        task,
        paths,
        budget,
        handoff,
    ))
}

/// The same command in a binary built without FTS5.
///
/// P7 exit criterion 4: the failure is `AKR-I022` and it affects nothing else. Every other
/// command in this file is reachable and correct in this configuration, because none of
/// them reads the index.
#[cfg(not(feature = "fts5"))]
fn search(
    _session: &mut Session,
    _query: &str,
    _raw_fts: bool,
    _kinds: &[String],
    _states: &[String],
    _limit: Option<usize>,
) -> Result<Output, EnvError> {
    Err(EnvError::new(
        "AKR-I022",
        "search requires a full-text index; this cache was built without FTS5",
    )
    .help("use `akr get` for a known key, or `akr context` for a whole bundle"))
}

fn context(
    session: &Session,
    goal: &str,
    paths: &[akr_core::model::Glob],
    budget: Option<usize>,
) -> Result<Output, EnvError> {
    if session.ledger.records().is_empty() {
        return Err(EnvError::new(
            "AKR-X001",
            "knowledge ledger has no records yet",
        )
        .help("create the first planning record with `knowledge.propose`, then use its key as the context goal"));
    }
    let model = session.resolve();
    let queue = session.review_queue();
    let freshness = session.freshness(&queue);

    let mut request = Request::new(goal);
    request.paths = paths.to_vec();
    request.budget = budget;

    let bundle = assemble_within_budget(&model, &freshness, &request).map_err(|error| {
        let code = match &error {
            akr_core::context::ContextError::GoalUnresolved(_) => "AKR-X001",
            akr_core::context::ContextError::GoalTerminal { id, .. } => {
                return EnvError::new("AKR-X002", error.to_string())
                    .help(format!("retrieve it with `akr get {id}`; use `akr start <task>` to find live work"));
            }
            akr_core::context::ContextError::GoalKind { id, .. } => {
                return EnvError::new("AKR-X003", error.to_string())
                    .help(format!("retrieve it with `akr get {id}`; context requires a live milestone, work or track"));
            }
            akr_core::context::ContextError::GoalRevision { head, .. } => {
                return EnvError::new("AKR-X004", error.to_string())
                    .help(format!("use the current head with `akr context --goal {head}`; retrieve history with `akr get {} --history`", head.key));
            }
            akr_core::context::ContextError::GoalAnchor(goal) => {
                return EnvError::new("AKR-X005", error.to_string())
                    .help(format!("retrieve the anchor with `akr get {goal}`; remove the #anchor for context"));
            }
            akr_core::context::ContextError::BadPath { .. } => "AKR-X011",
            akr_core::context::ContextError::BudgetTooSmall { .. } => "AKR-X021",
        };
        EnvError::new(code, error.to_string())
    })?;

    let text = render_text(&bundle, &model, &freshness);
    let result = render_json(&bundle, &model);
    let delivered = delivered_tokens(&text, &result);
    let result = match budget {
        Some(requested) if requested > 0 => with_budget_accounting(result, requested, delivered),
        _ => result,
    };
    Ok(Output::text(text).with_result(result))
}

/// Assembles until the *rendered* bundle honours the budget, not just the records in it.
///
/// `--budget` and `budget_tokens` are stated in tokens of result, and the assembler counts
/// tokens of selected content: titles and prose, not the rendered lines that carry them
/// and not the JSON half MCP sends alongside. The two are far apart. A bundle assembled to
/// 4,500 was delivered at close to three times that, and the agent that asked for it got a
/// transport-truncated answer to a budget it had set deliberately small
/// (`saveyourskin.papercut.knowledge-context-for-the-focused-mealtime-work`).
///
/// So render, measure both halves, and when the result overshoots, ask the assembler for
/// proportionally less and look again. Three passes: the ratio between selected content and
/// rendered bytes is stable enough that the second lands, and a bundle marginally over is a
/// better answer than a loop with no end. The extra passes are pure assembly over a model
/// that is already resolved — no git query is repeated.
///
/// This lives here rather than in the MCP adapter because both surfaces make the same
/// promise and `docs/08-mcp.md` §1 requires them to keep it identically.
fn assemble_within_budget(
    model: &akr_core::resolve::ResolvedModel<'_>,
    freshness: &akr_core::render::Freshness,
    request: &Request,
) -> Result<akr_core::context::Bundle, akr_core::context::ContextError> {
    const PASSES: usize = 3;
    let bundle = assemble(model, freshness, request)?;
    let Some(requested) = request.budget.filter(|budget| *budget > 0) else {
        return Ok(bundle);
    };

    let mut bundle = bundle;
    let mut assembly = requested;
    for _ in 0..PASSES {
        let delivered = delivered_tokens(
            &render_text(&bundle, model, freshness),
            &render_json(&bundle, model),
        );
        if delivered <= requested {
            break;
        }
        // Proportional, then floored at a tenth of the request: the mandatory sections put
        // a hard bottom under every bundle, and driving the assembly budget to nothing
        // trades an oversized bundle for an `AKR-X021` that answers a budget nobody asked
        // for.
        let next = (assembly.saturating_mul(requested) / delivered).max(requested / 10);
        if next >= assembly {
            break;
        }
        assembly = next;
        let mut narrowed = request.clone();
        narrowed.budget = Some(assembly);
        match assemble(model, freshness, &narrowed) {
            Ok(next_bundle) => bundle = next_bundle,
            Err(_) => break,
        }
    }
    Ok(bundle)
}

/// What a bundle costs to deliver: the rendered text and the JSON half, which MCP sends
/// together and which the budget is stated in.
fn delivered_tokens(text: &str, result: &Value) -> usize {
    fn estimate(text: &str) -> usize {
        text.split_whitespace()
            .map(|word| 1 + word.chars().filter(char::is_ascii_punctuation).count() / 3)
            .sum()
    }
    estimate(text) + estimate(&result.to_pretty())
}

/// States what the budget asked for and what it got, on every budgeted bundle.
///
/// A bundle can be irreducible above the request: V-123 never truncates a relation, a
/// state, a key or an acceptance verdict, so a goal with eighty-four normative records in
/// scope costs what its structure costs however small the budget. Refusing to shrink
/// further is right; doing it silently is how a caller ends up trusting a number the
/// result never honoured. Both figures leave them the one action that works, which is
/// narrowing `paths` rather than lowering the budget again.
fn with_budget_accounting(result: Value, requested: usize, delivered: usize) -> Value {
    let mut accounting = vec![
        (
            "requested_tokens",
            Value::integer(i64::try_from(requested).unwrap_or(i64::MAX)),
        ),
        (
            "delivered_tokens",
            Value::integer(i64::try_from(delivered).unwrap_or(i64::MAX)),
        ),
    ];
    if delivered > requested {
        accounting.push((
            "note",
            Value::string(
                "the bundle's mandatory content — keys, states, relations, acceptance \
                 verdicts, contradictions and staleness — is never truncated (V-123), so it \
                 does not fit the requested budget. Narrow `paths` to the files this work \
                 touches; lowering the budget cannot reduce it further.",
            ),
        ));
    }
    let Value::Object(mut fields) = result else {
        return result;
    };
    fields.retain(|(name, _)| name != "budget");
    fields.push(("budget".to_owned(), Value::object(accounting)));
    Value::Object(fields)
}

// -------------------------------------------------------------------------------------
// explain
// -------------------------------------------------------------------------------------

const CODES_LANG: &str = include_str!("../../../spec/diagnostics/codes-lang.md");
const CODES_RUNTIME: &str = include_str!("../../../spec/diagnostics/codes-runtime.md");
const DECISIONS: &str = include_str!("../../../docs/DECISIONS.md");

/// Prints a registry entry for a code, or a catalogue entry for a rule.
///
/// The registries are compiled in: `akr explain` needs no workspace, which is the point —
/// somebody reading a CI log should be able to look a code up without a checkout.
fn explain(subject: &str) -> Output {
    let wanted = subject.to_uppercase();
    for registry in [CODES_LANG, CODES_RUNTIME] {
        for line in registry.lines() {
            if line.starts_with('|') && line.contains(&format!("`{wanted}`")) {
                let cells: Vec<&str> = line.split('|').map(str::trim).collect();
                let mut text = format!("{wanted}\n");
                for (label, cell) in ["title", "severity", "rule", "message", "cause"]
                    .iter()
                    .zip(cells.iter().skip(2))
                {
                    if !cell.is_empty() {
                        text.push_str(&format!("  {label:<10} {cell}\n"));
                    }
                }
                let registry_name = if std::ptr::eq(registry, CODES_LANG) {
                    "spec/diagnostics/codes-lang.md"
                } else {
                    "spec/diagnostics/codes-runtime.md"
                };
                text.push_str(&format!("  registry   {registry_name}\n"));
                return Output::text(text).with_result(Value::object(vec![
                    ("code", Value::string(wanted.clone())),
                    ("registry", Value::string(registry_name)),
                ]));
            }
        }
    }

    if let Some(rule) = akr_core::validate::RULES
        .iter()
        .find(|r| r.id.to_string() == wanted)
    {
        let text = format!(
            "{}\n  title      {}\n  code       {}\n  stage      {:?}\n  catalogue  docs/05-validation-rules.md\n",
            rule.id, rule.title, rule.code, rule.stage
        );
        return Output::text(text).with_result(Value::object(vec![
            ("rule", Value::string(rule.id.to_string())),
            ("code", Value::string(rule.code.as_str())),
        ]));
    }

    if let Some(kind) = akr_core::model::Kind::from_name(&subject.to_lowercase()) {
        return explain_kind(kind);
    }

    if wanted.starts_with("D-")
        && let Some(heading) = DECISIONS
            .lines()
            .find(|line| line.starts_with("## ") && line.contains(&wanted))
    {
        let title = heading.trim_start_matches("## ");
        return Output::text(format!(
            "{title}\n  catalogue  docs/DECISIONS.md\n  help       decision identifiers document design history; use `akr get` for ledger records\n"
        ))
        .with_result(Value::object(vec![
            ("decision", Value::string(wanted)),
            ("catalogue", Value::string("docs/DECISIONS.md")),
        ]));
    }

    Output {
        text: format!(
            "error[AKR-C004]: {subject:?} is neither a registered diagnostic code, a known \
             rule, nor a record kind\n"
        ),
        result: Value::Object(Vec::new()),
        diagnostics: Vec::new(),
        exit: Exit::Usage,
        commit: None,
        source_graph: String::new(),
    }
}

/// The schema of one record kind: class, lifecycle, and slots, from the same tables the
/// type-checker reads. This is how an author learns what a kind requires *before* the
/// first `AKR-T001` rather than from it.
fn explain_kind(kind: akr_core::model::Kind) -> Output {
    let class = kind.class();
    let mut text = format!("{} — a record kind\n", kind.name());
    text.push_str(&format!("  class      {}\n", class.name()));
    text.push_str(&format!(
        "  lifecycle  {}   (initial: {})\n",
        class
            .states()
            .iter()
            .map(|s| s.name())
            .collect::<Vec<_>>()
            .join(", "),
        class
            .initial()
            .iter()
            .map(|s| s.name())
            .collect::<Vec<_>>()
            .join(", "),
    ));

    let (required, optional): (Vec<_>, Vec<_>) =
        kind.content_slots().iter().partition(|spec| spec.required);
    // A closed slot prints its members, not the word `enum`. `explain` is the command the
    // write surfaces point at for "which slots may this kind have", and answering that
    // with a type name for the one slot that has a fixed vocabulary sends the author back
    // to guessing — `measurement` for an observation's `method` is the obvious guess, and
    // it is wrong (`akr.papercut.collated-19-papercuts-from-evidence-intake`).
    let names = |specs: &[&akr_core::model::ContentSlotSpec]| {
        specs
            .iter()
            .map(|spec| match kind.content_enum_values(spec.slot) {
                Some(values) => format!("{}: {}", spec.slot.name(), values.join("|")),
                None => format!("{}: {}", spec.slot.name(), spec.slot.value_type()),
            })
            .collect::<Vec<_>>()
    };
    let required = names(&required);
    let mut optional = names(&optional);
    text.push_str(&format!(
        "  required   {}{}\n",
        if required.is_empty() {
            "(no kind-specific slots)".to_owned()
        } else {
            required.join(", ")
        },
        if kind.requires_acceptance() {
            " — plus a non-empty acceptance block (V-008)"
        } else {
            ""
        },
    ));
    if kind.allows_acceptance() && !kind.requires_acceptance() {
        optional.push("acceptance".to_owned());
    }
    if !optional.is_empty() {
        text.push_str(&format!("  optional   {}\n", optional.join(", ")));
    }
    // `topic` either way round. Listing only what a kind accepts leaves an author who has
    // seen `topic` on the write surface to assume it is universal, and the refusal arrives
    // as an `AKR-C004` from a rule `explain` had every chance to mention
    // (`orderflower.papercut.while-proposing-a-work-record-the-generic`).
    if class.scope_required() {
        text.push_str("  scope      required; `topic` marks normative exclusivity (D-004b)\n");
    } else {
        text.push_str(&format!(
            "  topic      not accepted; a {} is not normative, and `topic` on one is \
             AKR-C004 (D-004b)\n",
            kind.name()
        ));
    }
    let relations = akr_core::model::Relation::ALL
        .iter()
        .filter(|relation| relation.domain().accepts(kind))
        .map(|relation| relation.name())
        .collect::<Vec<_>>();
    text.push_str(&format!("  relations  {}\n", relations.join(", ")));
    if kind == akr_core::model::Kind::Observation {
        text.push_str(
            "  standing   verified requires provenance: `method`, a source block, or supporting evidence (V-022)\n",
        );
    }
    // The state-entry rules. V-021 refused the first `active` decision an author wrote
    // because `explain` had listed `supported_by` and `implements` among seven optional
    // relations and said nothing about one of them being required to leave `proposed`
    // (`akr.papercut.collated-19-papercuts-from-evidence-intake`, from Lege-ecosystem).
    if kind == akr_core::model::Kind::Decision {
        text.push_str(
            "  standing   active requires a live `implements`, `depends_on` or `supported_by` \
             edge to a requirement, policy, constraint or evidence-bearing record (V-021); \
             `proposed` carries no such requirement\n",
        );
    }
    if relations.contains(&"contradicts") {
        text.push_str(
            "  conflict   a `contradicts` edge between two live records needs `acknowledged true` \
             on one of them, or one side superseded (V-023)\n",
        );
    }
    text.push_str("  reference  docs/02-data-model.md; spec/tables/vocabulary.json\n");

    Output::text(text).with_result(Value::object(vec![
        ("kind", Value::string(kind.name())),
        ("class", Value::string(class.name())),
        (
            "required_slots",
            Value::array(required.into_iter().map(Value::string).collect()),
        ),
        (
            "optional_slots",
            Value::array(optional.into_iter().map(Value::string).collect()),
        ),
        ("accepts_topic", Value::bool(class.scope_required())),
        (
            "lifecycle",
            Value::array(
                class
                    .states()
                    .iter()
                    .map(|state| Value::string(state.name()))
                    .collect(),
            ),
        ),
    ]))
}

/// Turns a session's diagnostics into JSON, for the envelope.
#[must_use]
pub fn diagnostics_json(
    diagnostics: &[Diagnostic],
    sources: &akr_core::diagnostics::SourceMap,
    profile: Profile,
) -> Vec<Value> {
    diagnostics
        .iter()
        .map(|d| diagnostic_json(d, sources, profile))
        .collect()
}

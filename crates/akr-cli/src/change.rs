//! `akr diff --staged`, `akr change *` and `akr git *`.
//!
//! `docs/16-change-protocol.md`. Everything here reads the *git index* rather than the
//! working tree, and everything that compares ledgers parses them rather than reading
//! `git diff` text: a reformat is not a semantic change and a textual diff cannot say so.

use crate::commands::Output;
use crate::session::{EnvError, Exit, Session};
use akr_core::change::{self, ChangeIntent, ChangeKind, SemanticDelta};
use akr_core::git::{IndexEntry, Repository};
use akr_core::json::Value;
use akr_core::model::Ledger;

/// Opens the repository, or explains that this is not one.
fn repository(session: &Session) -> Result<&Repository, EnvError> {
    session
        .repository
        .as_ref()
        .ok_or_else(|| match &session.git_unavailable {
            Some(reason) => EnvError::new(
                "AKR-G002",
                format!("the change protocol needs git, and it could not be used: {reason}"),
            ),
            None => EnvError::new(
                "AKR-G001",
                "the change protocol needs a git repository; this workspace is not inside one",
            ),
        })
}

fn to_env(error: change::ChangeError) -> EnvError {
    let code = match error {
        change::ChangeError::Git(_) => "AKR-G001",
        change::ChangeError::NoTransaction | change::ChangeError::NotPrepared => "AKR-C031",
        _ => "AKR-C032",
    };
    EnvError::new(code, error.to_string())
}

/// Parses the `.akr` files of two trees and compares them.
fn staged_delta(session: &Session) -> Result<(SemanticDelta, Vec<IndexEntry>), EnvError> {
    let (delta, entries, _) = staged_state(session)?;
    Ok((delta, entries))
}

/// The staged delta, the index entries it was computed from, and the staged ledger.
fn staged_state(session: &Session) -> Result<(SemanticDelta, Vec<IndexEntry>, Ledger), EnvError> {
    crate::secrets::ensure_untracked(session)?;
    let repository = repository(session)?;
    let entries = repository
        .staged_entries()
        .map_err(|e| EnvError::new("AKR-G001", e.to_string()))?;
    let staged_files = change::staged_akr_files(repository, &entries).map_err(to_env)?;
    let staged = ledger_of(&staged_files);

    let head_files = match repository.head() {
        Ok(commit) => change::akr_files_at(repository, &commit).map_err(to_env)?,
        // An unborn branch has no `HEAD`, so every record in the index is new. That is
        // the honest answer rather than a failure: the first commit of a repository is
        // exactly the case where everything is added.
        Err(_) => Vec::new(),
    };
    let base = (!head_files.is_empty()).then(|| ledger_of(&head_files));

    let changed = repository
        .staged_changes()
        .map_err(|e| EnvError::new("AKR-G001", e.to_string()))?;
    let delta = change::delta(base.as_ref(), &staged, &changed);
    Ok((delta, entries, staged))
}

/// A work reference in the one spelling the trailer readers look for: `@key` or
/// `@key/rev` (D-049).
///
/// `akr git log` and the session head both search for `AKR-Work: @<key>`, so a
/// transaction opened with a bare key wrote a trailer neither of them could find.
fn work_reference(text: &str) -> Result<String, EnvError> {
    let bare = text.trim().trim_start_matches('@');
    akr_core::model::Reference::parse(&format!("@{bare}"))
        .map(|reference| reference.to_string())
        .map_err(|error| {
            EnvError::new(
                "AKR-C004",
                format!("{text:?} is not a record reference: {error}"),
            )
            .help("name a work record by key, e.g. --primary sys.work.audit-lane")
        })
}

/// The record a work reference names in `ledger`, if it names one.
fn referenced<'a>(ledger: &'a Ledger, reference: &str) -> Option<&'a akr_core::model::Record> {
    let parsed = akr_core::model::Reference::parse(reference).ok()?;
    match parsed.revision {
        Some(revision) => ledger.get(&akr_core::model::RevisionId::new(
            parsed.key.clone(),
            revision,
        )),
        None => ledger
            .head(&parsed.key)
            .ok()
            .or_else(|| ledger.revisions_of(&parsed.key).last().copied()),
    }
}

/// Refuses a work reference that names nothing, or names something that is not work.
fn check_work_references(intent: &ChangeIntent, ledger: &Ledger) -> Result<(), EnvError> {
    for reference in intent.work_refs() {
        match referenced(ledger, &reference) {
            None => return Err(to_env(change::ChangeError::UnknownWork(reference))),
            Some(record) if record.kind != akr_core::model::Kind::Work => {
                return Err(to_env(change::ChangeError::NotWork {
                    reference,
                    kind: record.kind.name().to_owned(),
                }));
            }
            Some(_) => {}
        }
    }
    Ok(())
}

/// Parses a set of `(path, text)` pairs into a ledger, ignoring diagnostics.
///
/// Diagnostics are ignored on purpose: `akr diff --staged` reports what *moved*, and a
/// ledger that does not validate is `akr check`'s business. Reporting both here would
/// bury the delta in a validation report the caller did not ask for.
fn ledger_of(files: &[(String, String)]) -> Ledger {
    let mut parsed = Vec::new();
    for (index, (path, text)) in files.iter().enumerate() {
        let file_id = akr_core::diagnostics::FileId(u32::try_from(index).unwrap_or(0));
        if let Some(file) = akr_core::syntax::parse(text, file_id).file {
            parsed.push((path.clone(), file));
        }
    }
    akr_core::syntax::lower_all(&parsed).0
}

/// `akr diff --staged`.
pub fn diff_staged(session: &Session) -> Result<Output, EnvError> {
    let (delta, _) = staged_delta(session)?;
    let mut text = String::new();

    if !delta.added.is_empty() {
        text.push_str("records added\n");
        for id in &delta.added {
            text.push_str(&format!("  @{id}\n"));
        }
    }
    if !delta.revised.is_empty() {
        text.push_str("revisions added\n");
        for id in &delta.revised {
            text.push_str(&format!("  @{id}\n"));
        }
    }
    if !delta.transitions.is_empty() {
        text.push_str("state transitions\n");
        for transition in &delta.transitions {
            let from = transition
                .from
                .map_or_else(|| "new".to_owned(), |state| state.name().to_owned());
            text.push_str(&format!(
                "  {:<48} {from} -> {}\n",
                transition.id.key.to_string(),
                transition.to.name()
            ));
        }
    }
    if !delta.evidence.is_empty() {
        text.push_str("evidence added\n");
        for id in &delta.evidence {
            text.push_str(&format!("  @{id}\n"));
        }
    }
    if !delta.code.is_empty() {
        text.push_str("code\n");
        for path in &delta.code {
            text.push_str(&format!("  {path}\n"));
        }
    }
    if text.is_empty() {
        text.push_str("nothing staged\n");
    }

    let result = Value::object(vec![
        ("added", refs_json(&delta.added)),
        ("revised", refs_json(&delta.revised)),
        (
            "transitions",
            Value::array(
                delta
                    .transitions
                    .iter()
                    .map(|transition| {
                        Value::object(vec![
                            ("key", Value::string(transition.id.key.to_string())),
                            (
                                "from",
                                transition
                                    .from
                                    .map_or(Value::Null, |state| Value::string(state.name())),
                            ),
                            ("to", Value::string(transition.to.name())),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("evidence", refs_json(&delta.evidence)),
        (
            "code",
            Value::array(
                delta
                    .code
                    .iter()
                    .map(|p| Value::string(p.clone()))
                    .collect(),
            ),
        ),
    ]);
    Ok(Output::plain(text, result))
}

fn refs_json(ids: &[akr_core::model::RevisionId]) -> Value {
    Value::array(
        ids.iter()
            .map(|id| Value::string(format!("@{id}")))
            .collect(),
    )
}

/// `akr change begin`.
#[allow(clippy::too_many_arguments)]
pub fn begin(
    session: &Session,
    kind: &str,
    summary: &str,
    scope: Option<&str>,
    primary: Option<&str>,
    related: &[String],
    note: Option<&str>,
    untracked_reason: Option<&str>,
    why: &[String],
) -> Result<Output, EnvError> {
    let repository = repository(session)?;
    if let Some(open) = change::load(repository).map_err(to_env)? {
        return Err(to_env(change::ChangeError::AlreadyOpen(open.id)));
    }
    let primary = primary.map(work_reference).transpose()?;
    let related: Vec<String> = related
        .iter()
        .map(|reference| work_reference(reference))
        .collect::<Result<_, _>>()?;
    if !why.is_empty() && primary.is_none() {
        return Err(EnvError::new(
            "AKR-C003",
            "--why records a reason on the primary work record; name it with --primary",
        )
        .help(
            "a commit that advances no work record explains itself with --untracked-reason \
             instead",
        ));
    }
    let kind = ChangeKind::from_name(kind).ok_or_else(|| {
        EnvError::new(
            "AKR-C004",
            format!(
                "`{kind}` is not a change kind; the kinds are {}",
                ChangeKind::ALL
                    .iter()
                    .map(|k| k.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )
    })?;
    let base = repository
        .head()
        .map(|commit| commit.as_str().to_owned())
        .unwrap_or_default();

    let mut intent = ChangeIntent::new(&base, kind, summary);
    intent.scope = scope.map(ToOwned::to_owned);
    intent.primary_work = primary;
    intent.related_work = related;
    intent.implementation_note = note.map(ToOwned::to_owned);
    intent.untracked_reason = untracked_reason.map(ToOwned::to_owned);

    // `--why` is `akr trace` on the primary record (D-047): the reason is written once,
    // onto the work it explains, and the commit message is generated from there. A
    // refusal — an unknown key, a record that is not work, an entry over the limit —
    // opens no transaction.
    let mut traced = String::new();
    if !why.is_empty() {
        let reference = intent.primary_work.clone().unwrap_or_default();
        let key = akr_core::model::Reference::parse(&reference)
            .map(|reference| reference.key)
            .map_err(|error| EnvError::new("AKR-C004", error.to_string()))?;
        let context = crate::write::context_of(session);
        match akr_core::ops::trace_append(&context, &key, why) {
            Ok(applied) => {
                for file in &applied.files {
                    traced.push_str(&format!(
                        "traced {} — stage .akr/{} with the change\n",
                        key,
                        file.display().to_string().replace('\\', "/")
                    ));
                }
                for note in &applied.notes {
                    traced.push_str(note);
                    traced.push('\n');
                }
            }
            Err(refused) => return crate::write::render(session, Err(refused)),
        }
        intent.why = why
            .iter()
            .map(|entry| entry.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect();
    }
    change::save(repository, &intent).map_err(to_env)?;

    Ok(Output::plain(
        format!("opened change {} on {}\n{traced}", intent.id, short(&base)),
        Value::object(vec![
            ("id", Value::string(intent.id)),
            ("base_commit", Value::string(base)),
        ]),
    ))
}

/// `akr change show`.
pub fn show(session: &Session) -> Result<Output, EnvError> {
    let repository = repository(session)?;
    let Some(intent) = change::load(repository).map_err(to_env)? else {
        return Ok(Output::plain(
            "no change transaction is open\n",
            Value::object(vec![("open", Value::bool(false))]),
        ));
    };
    let mut text = format!(
        "change {}\n  base     {}\n  kind     {}\n  summary  {}\n",
        intent.id,
        short(&intent.base_commit),
        intent.kind.as_str(),
        intent.summary
    );
    if let Some(scope) = &intent.scope {
        text.push_str(&format!("  scope    {scope}\n"));
    }
    for reference in intent.work_refs() {
        text.push_str(&format!("  work     {reference}\n"));
    }
    if let Some(reason) = &intent.untracked_reason {
        text.push_str(&format!("  untracked  {reason}\n"));
    }
    for why in &intent.why {
        text.push_str(&format!("  why      {why}\n"));
    }
    text.push_str(&format!(
        "  prepared {}\n",
        intent.prepared_tree.as_deref().unwrap_or("no")
    ));
    Ok(Output::plain(
        text,
        Value::object(vec![
            ("open", Value::bool(true)),
            ("id", Value::string(intent.id.clone())),
            ("summary", Value::string(intent.summary.clone())),
            ("prepared", Value::bool(intent.prepared_tree.is_some())),
        ]),
    ))
}

/// `akr change abort`.
pub fn abort(session: &Session) -> Result<Output, EnvError> {
    let repository = repository(session)?;
    let discarded = change::discard(repository).map_err(to_env)?;
    Ok(Output::plain(
        if discarded {
            "change discarded\n"
        } else {
            "no change transaction was open\n"
        },
        Value::object(vec![("discarded", Value::bool(discarded))]),
    ))
}

/// `akr change prepare --staged` and `akr change verify --staged`.
///
/// `verify` is `prepare` without the write: the same checks, so a hook and the command an
/// author runs cannot disagree about what would be refused.
pub fn prepare(session: &Session, write: bool) -> Result<Output, EnvError> {
    let repository = repository(session)?;
    let Some(mut intent) = change::load(repository).map_err(to_env)? else {
        return Err(to_env(change::ChangeError::NoTransaction));
    };
    let (delta, entries, staged) = staged_state(session)?;
    if entries.is_empty()
        || !repository
            .has_staged_changes()
            .map_err(|e| EnvError::new("AKR-G001", e.to_string()))?
    {
        return Err(to_env(change::ChangeError::NothingStaged));
    }

    // A material code change with neither a work reference nor an explicit exemption is
    // the case the protocol exists to catch: it is how implementation and intent drift
    // apart in the first place.
    if !delta.code.is_empty()
        && intent.primary_work.is_none()
        && intent.related_work.is_empty()
        && intent.untracked_reason.is_none()
    {
        return Err(to_env(change::ChangeError::Untracked));
    }

    // A transaction names real work (D-049). A reference that resolves to nothing, or to
    // a decision or a milestone, wrote an `AKR-Work` trailer that linked no work record
    // to the commit — every such trailer in one audited project pointed at a non-work key.
    check_work_references(&intent, &staged)?;

    // Several work records moved and none was chosen: the subject can only be about one
    // of them, and guessing produces a message that misdescribes the commit.
    if delta.transitions.len() > 1 && intent.primary_work.is_none() {
        return Err(to_env(change::ChangeError::PrimaryRequired(
            delta
                .transitions
                .iter()
                .map(|transition| transition.id.key.to_string())
                .collect(),
        )));
    }

    let tree = repository
        .write_tree()
        .map_err(|e| EnvError::new("AKR-G001", e.to_string()))?;
    let digest = change::implementation_digest(&entries);

    let mut text = format!("change {} prepared\n", intent.id);
    text.push_str(&format!("  staged tree      {}\n", short(&tree)));
    text.push_str(&format!("  implementation   {}\n", short(&digest)));
    text.push_str(&format!(
        "  ledger           {} added, {} revised, {} transitions\n",
        delta.added.len(),
        delta.revised.len(),
        delta.transitions.len()
    ));
    text.push_str(&format!("  code             {} files\n", delta.code.len()));
    if !delta.trace.is_empty() {
        text.push_str(&format!(
            "  why              {} trace entr{} — the commit's AKR-Why trailers\n",
            delta.trace.len(),
            if delta.trace.len() == 1 { "y" } else { "ies" }
        ));
    }

    if write {
        intent.prepared_tree = Some(tree.clone());
        intent.prepared_digest = Some(digest.clone());
        change::save(repository, &intent).map_err(to_env)?;
        text.push_str("\nnext: akr git commit\n");
    }

    Ok(Output::plain(
        text,
        Value::object(vec![
            ("id", Value::string(intent.id)),
            ("tree", Value::string(tree)),
            ("implementation_digest", Value::string(digest)),
            ("prepared", Value::bool(write)),
        ]),
    ))
}

/// `akr git message` — the generated message, printed rather than committed.
pub fn message(session: &Session) -> Result<Output, EnvError> {
    let (intent, delta, staged, tree) = prepared(session)?;
    let graph = session.resolve().source_graph.to_string();
    let text =
        akr_core::change::commit_message(&intent, &delta, Some(&staged), Some(&tree), Some(&graph));
    Ok(Output::plain(
        text.clone(),
        Value::object(vec![("message", Value::string(text))]),
    ))
}

/// `akr git commit` — generate the message and hand the index to git.
pub fn commit(session: &Session, message_override: Option<&str>) -> Result<Output, EnvError> {
    let repository = repository(session)?;
    let (intent, delta, staged, tree) = prepared(session)?;
    let graph = session.resolve().source_graph.to_string();
    let generated =
        akr_core::change::commit_message(&intent, &delta, Some(&staged), Some(&tree), Some(&graph));
    let message = message_override.map_or(generated.clone(), |prose| {
        replace_generated_prose(&generated, prose)
    });
    let commit = repository
        .commit(&message)
        .map_err(|e| EnvError::new("AKR-G001", e.to_string()))?;
    // The transaction is scaffolding; the commit and its trailers are what is durable.
    change::discard(repository).map_err(to_env)?;
    Ok(Output::plain(
        format!(
            "{} {}\n",
            short(commit.as_str()),
            message.lines().next().unwrap_or_default()
        ),
        Value::object(vec![
            ("commit", Value::string(commit.as_str())),
            ("change", Value::string(intent.id)),
        ]),
    ))
}

/// Replaces the human prose while retaining the canonical, machine-readable trailers.
fn replace_generated_prose(generated: &str, prose: &str) -> String {
    let trailers = generated
        .find("\nAKR-Change: ")
        .map_or("", |start| generated[start..].trim());
    let prose = prose.trim();
    if trailers.is_empty() {
        format!("{prose}\n")
    } else {
        format!("{prose}\n\n{trailers}\n")
    }
}

/// `akr git log <record>` — the commits whose trailers name a record.
pub fn log(session: &Session, reference: &str) -> Result<Output, EnvError> {
    let repository = repository(session)?;
    let key = reference.trim_start_matches('@');
    let key = key.split_once('/').map_or(key, |(key, _)| key);
    let commits = repository
        .log_grep("--all", &format!("AKR-Work: @{key}"))
        .map_err(|e| EnvError::new("AKR-G001", e.to_string()))?;
    let mut text = String::new();
    if commits.is_empty() {
        text.push_str("no commits name this record\n");
    }
    for (commit, subject) in &commits {
        text.push_str(&format!("{}  {subject}\n", short(commit.as_str())));
    }
    Ok(Output::plain(
        text,
        Value::object(vec![(
            "commits",
            Value::array(
                commits
                    .iter()
                    .map(|(commit, subject)| {
                        Value::object(vec![
                            ("commit", Value::string(commit.as_str())),
                            ("subject", Value::string(subject.clone())),
                        ])
                    })
                    .collect(),
            ),
        )]),
    ))
}

/// `akr git install-hooks` — thin wrappers, so the logic stays in the binary.
///
/// A hook that contained the checks would be a second implementation nobody updates.
/// These call `akr git-hook <name>`, which is the same code path the author's own
/// `akr change verify --staged` runs.
pub fn install_hooks(session: &Session) -> Result<Output, EnvError> {
    let repository = repository(session)?;
    let written = write_hooks(repository, true)?.written;
    Ok(Output::plain(
        format!("installed {} hooks\n", written.len()),
        Value::object(vec![(
            "hooks",
            Value::array(written.iter().map(|h| Value::string(h.clone())).collect()),
        )]),
    ))
}

/// The hooks AKR installs.
const HOOKS: [&str; 2] = ["pre-commit", "commit-msg"];

/// What [`write_hooks`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HookReport {
    /// Hooks written.
    pub written: Vec<String>,
    /// Hooks that already existed, were not AKR's, and were left alone.
    pub kept: Vec<String>,
}

/// Writes the two hook wrappers into the repository's hooks directory.
///
/// With `overwrite` false — how `akr init` calls it (D-049) — a hook somebody else put
/// there is kept: installing by default must not silently replace a project's own
/// pre-commit. `akr git install-hooks` is the explicit request and overwrites.
///
/// # Errors
/// `AKR-G001` when git cannot name the hooks directory; `AKR-C042` when it cannot be
/// written.
pub fn write_hooks(repository: &Repository, overwrite: bool) -> Result<HookReport, EnvError> {
    let dir = repository
        .git_path("hooks")
        .map_err(|e| EnvError::new("AKR-G001", e.to_string()))?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| EnvError::new("AKR-C042", format!("cannot create {}: {e}", dir.display())))?;

    let mut report = HookReport::default();
    for hook in HOOKS {
        let path = dir.join(hook);
        if !overwrite
            && let Ok(existing) = std::fs::read_to_string(&path)
            && !existing.contains("akr git-hook")
        {
            report.kept.push(hook.to_owned());
            continue;
        }
        let body = format!("#!/bin/sh\nexec akr git-hook {hook} \"$@\"\n");
        std::fs::write(&path, body).map_err(|e| {
            EnvError::new("AKR-C042", format!("cannot write {}: {e}", path.display()))
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
        }
        report.written.push(hook.to_owned());
    }
    Ok(report)
}

/// Whether this worktree's repository has AKR's `pre-commit` hook.
///
/// `false` when the answer cannot be had: the session head mentions absent hooks, and a
/// repository git cannot describe is not one to nag about.
#[must_use]
pub fn hooks_installed(repository: &Repository) -> Option<bool> {
    let dir = repository.git_path("hooks").ok()?;
    Some(
        std::fs::read_to_string(dir.join("pre-commit"))
            .is_ok_and(|text| text.contains("akr git-hook")),
    )
}

/// `akr git-hook <name>` — what the installed hooks call.
///
/// A hook is a guardrail, never the architecture: `pre-commit` re-runs the same
/// verification the author could have run, and refuses rather than repairing.
pub fn git_hook(session: &Session, name: &str) -> Result<Output, EnvError> {
    match name {
        "pre-commit" if is_reword(session) => Ok(Output::plain(
            "AKR OK (reword)\n",
            Value::object(vec![
                ("ok", Value::bool(true)),
                ("reword", Value::bool(true)),
            ]),
        )),
        "pre-commit" => match prepare(session, false) {
            Ok(output) => Ok(Output::plain("AKR OK\n", output.result)),
            Err(error) => Ok(Output::plain(
                format!("{error}\n"),
                Value::object(vec![("ok", Value::bool(false))]),
            )
            .with_diagnostics(Vec::new(), Exit::Diagnostics)),
        },
        "commit-msg" => Ok(Output::plain(
            "AKR OK\n",
            Value::object(vec![("ok", Value::bool(true))]),
        )),
        other => Err(EnvError::new(
            "AKR-C004",
            format!("`{other}` is not a hook akr installs"),
        )),
    }
}

/// Whether this commit changes only the message: nothing is staged beyond `HEAD`.
///
/// The transaction closes at `akr git commit`, so every later `git commit` in the worktree
/// meets a `pre-commit` hook with no transaction to check and is refused `AKR-C031` —
/// including `git commit --amend` that only rewords the subject, which leaves the tree
/// untouched and the AKR trailers byte-identical. That left `--no-verify` as the only way
/// to fix a commit subject, which is a bad habit to teach for a change the protocol has no
/// opinion about
/// (`jpegxl-rs.papercut.amending-an-akr-generated-commit-message-to`).
///
/// An empty index against `HEAD` is exactly the reword case: an ordinary commit has
/// something staged or git refuses it before any hook runs. A transaction in flight is
/// still checked, because an author who began one and then rewords is mid-protocol and
/// wants the check. The up-front remedy is unchanged and better: `akr change begin
/// --summary` and `--kind` decide the subject before the commit exists.
fn is_reword(session: &Session) -> bool {
    let Ok(repository) = repository(session) else {
        return false;
    };
    if change::load(repository).ok().flatten().is_some() {
        return false;
    }
    repository.has_staged_changes() == Ok(false)
}

/// The prepared transaction, refusing when the staged tree has moved under it.
fn prepared(session: &Session) -> Result<(ChangeIntent, SemanticDelta, Ledger, String), EnvError> {
    let repository = repository(session)?;
    let Some(intent) = change::load(repository).map_err(to_env)? else {
        return Err(to_env(change::ChangeError::NoTransaction));
    };
    let Some(prepared_tree) = intent.prepared_tree.clone() else {
        return Err(to_env(change::ChangeError::NotPrepared));
    };
    let tree = repository
        .write_tree()
        .map_err(|e| EnvError::new("AKR-G001", e.to_string()))?;
    if tree != prepared_tree {
        return Err(to_env(change::ChangeError::StagedTreeMoved {
            prepared: prepared_tree,
            found: tree,
        }));
    }
    let entries = repository
        .staged_entries()
        .map_err(|e| EnvError::new("AKR-G001", e.to_string()))?;
    let staged_files = change::staged_akr_files(repository, &entries).map_err(to_env)?;
    let staged = ledger_of(&staged_files);
    let (delta, _) = staged_delta(session)?;
    Ok((intent, delta, staged, tree))
}

fn short(value: &str) -> String {
    let bare = value.strip_prefix("sha256:").unwrap_or(value);
    bare.chars().take(12).collect()
}

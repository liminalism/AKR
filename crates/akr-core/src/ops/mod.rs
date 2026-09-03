//! The write operations: `propose`, `revise`, `supersede`, `complete`, `abandon`.
//!
//! `docs/07-cli.md` §4 defines one pipeline and every operation here performs all of it:
//!
//! ```text
//! 1. parse     the current ledger
//! 2. apply     the requested change, in memory
//! 3. validate  the RESULTING ledger, and refuse what this write introduced (D-039)
//! 4. format    every touched record canonically
//! 5. write     touched files, atomically
//! ```
//!
//! **Validation is of the result, not the change**, and **failure writes nothing**.
//! Nothing reaches the disk before step 5, so every refusal leaves the working tree
//! byte-identical. `tests/ops_atomicity.rs` hashes every source file before and after
//! each failing path and asserts exactly that.
//!
//! # The lock is not touched
//!
//! No operation here writes `akr.lock`. A lock records a *build*: a commit, a
//! source-graph hash over every file, and the head resolutions at that commit (D-014).
//! A write operation knows none of those, and inventing them would put a fabricated build
//! in the file whose whole job is to be checkable. Every [`Outcome`] therefore carries
//! [`Outcome::lock_stale`], and the caller runs `akr build` afterwards.
//!
//! One consequence is worth stating plainly, because it looks like a bug: an operation
//! that moves a sealed record along its lifecycle — `supersede` setting the old head to
//! `superseded`, `complete` setting `completed` — changes that record's canonical text
//! and therefore its content hash. Between the write and the next `akr build`, `akr
//! check` reports `AKR-R052` (lock stale). That is correct and expected; see the note in
//! the P6 report about `docs/04` §8.3.

// `Refused` is a large `Err` variant, and deliberately so: it carries the structured
// refusal data the CLI and the MCP surface render — the unfinished children, the
// unsatisfied checks, the diagnostics and the help line. Boxing it to satisfy
// `result_large_err` would put a `Box` in the contract Writer B wires against, and add a
// dereference to a path that is not exceptional here. Refusing *is* a feature of these
// operations, not an error case to be made cheap.
#![allow(clippy::result_large_err)]

mod exclusion;
mod stage;

use crate::diagnostics::codes::cli;
use crate::diagnostics::{Code, Diagnostic, Label, RuleId, Severity, Subject, codes};
use crate::model::{
    Class, Disposition, Kind, LogicalKey, Outcome as DispositionOutcome, Record, Reference,
    Relation, RevisionId, State,
};
use crate::syntax::{cst, emit};
use crate::validate;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub use exclusion::{WriteLock, verify_unchanged};
pub use stage::{LoadError, Staged};

// -------------------------------------------------------------------------------------
// results
// -------------------------------------------------------------------------------------

/// Which operation produced a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// A new key, or a new proposed revision.
    Propose,
    /// An edit to a proposed head, or a new revision of a sealed one.
    Revise,
    /// A new revision superseding the head.
    Supersede,
    /// A planning record moved to `completed`.
    Complete,
    /// A planning record moved to `abandoned`.
    Abandon,
    /// Legacy claims drafted into `proposed` records with a tracking record (P8).
    Import,
}

impl Operation {
    /// The subcommand name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Propose => "propose",
            Self::Revise => "revise",
            Self::Supersede => "supersede",
            Self::Complete => "complete",
            Self::Abandon => "abandon",
            Self::Import => "import",
        }
    }
}

/// What happened to one revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeKind {
    /// The revision did not exist before.
    Created,
    /// The revision existed and its body changed.
    Edited,
    /// Only the revision's `state` slot changed.
    StateChanged {
        /// The state it left.
        from: State,
        /// The state it entered.
        to: State,
    },
}

/// One revision touched by an operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// Which revision.
    pub id: RevisionId,
    /// What happened to it.
    pub kind: ChangeKind,
    /// The file it lives in, relative to the `.akr` directory.
    pub file: PathBuf,
}

/// A successful write.
#[derive(Debug, Clone)]
pub struct Applied {
    /// Which operation.
    pub operation: Operation,
    /// Every revision touched, in key order.
    pub changes: Vec<Change>,
    /// Files written, relative to the `.akr` directory.
    pub files: Vec<PathBuf>,
    /// Diagnostics that did not block the write. Empty under the strict default.
    pub diagnostics: Vec<Diagnostic>,
    /// Advisory lines about what the write leaves for the caller to look at.
    ///
    /// Not diagnostics: a note never blocks a write and never fails a strict build. It
    /// carries the things the pipeline can see and the next command cannot — most of all
    /// which acceptance evidence a revision of a completed record has just put back in
    /// question, which only shows up as `AKR-R022` after the commit lands.
    pub notes: Vec<String>,
    /// Whether `akr.lock` is now stale and the caller should run `akr build`.
    pub lock_stale: bool,
}

/// An unfinished child a supersession or abandonment must account for (D-017).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnfinishedChild {
    /// The child's key.
    pub key: LogicalKey,
    /// The state it is in, which is why it counts as unfinished.
    pub state: State,
}

/// An acceptance check that is not satisfied (V-020).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsatisfiedCheck {
    /// The check identifier.
    pub id: String,
    /// Why it is not satisfied, in one clause.
    pub reason: String,
}

/// A refused write. Nothing was written.
#[derive(Debug, Clone)]
pub struct Refused {
    /// Which operation.
    pub operation: Operation,
    /// The most specific code for the refusal.
    pub code: Code,
    /// The one-line reason.
    pub message: String,
    /// Every diagnostic the resulting ledger produced.
    pub diagnostics: Vec<Diagnostic>,
    /// Children needing a disposition, for `supersede` and `abandon`.
    pub unfinished_children: Vec<UnfinishedChild>,
    /// Checks blocking a `complete`.
    pub unsatisfied_checks: Vec<UnsatisfiedCheck>,
    /// A suggested fix, ready to render under `help:`.
    pub help: Option<String>,
}

impl Refused {
    fn new(operation: Operation, code: Code, message: impl Into<String>) -> Self {
        Self {
            operation,
            code,
            message: message.into(),
            diagnostics: Vec::new(),
            unfinished_children: Vec::new(),
            unsatisfied_checks: Vec::new(),
            help: None,
        }
    }

    fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// The refusal as a diagnostic, for callers that render one stream.
    #[must_use]
    pub fn diagnostic(&self) -> Diagnostic {
        Diagnostic {
            code: self.code,
            severity: Severity::Error,
            rule: None,
            message: self.message.clone(),
            primary: Label::new(Subject::Ledger),
            notes: Vec::new(),
            help: self.help.clone(),
        }
    }
}

/// The outcome of a write operation.
pub type WriteResult = Result<Applied, Refused>;

/// Backwards-compatible alias for the success type.
pub type Outcome = Applied;

// -------------------------------------------------------------------------------------
// context and requests
// -------------------------------------------------------------------------------------

/// Where to write, and under which diagnostic profile.
#[derive(Debug, Clone)]
pub struct WriteContext {
    /// The `.akr` directory.
    pub akr_dir: PathBuf,
    /// Whether warnings count as errors. `true` is the default profile (D-013).
    pub strict: bool,
    /// Author recorded on records this operation creates.
    pub author: Option<String>,
}

impl WriteContext {
    /// A context for the given `.akr` directory, strict, with no author.
    #[must_use]
    pub fn new(akr_dir: impl Into<PathBuf>) -> Self {
        Self {
            akr_dir: akr_dir.into(),
            strict: true,
            author: None,
        }
    }

    /// Sets the author recorded on new records.
    #[must_use]
    pub fn with_author(mut self, author: impl Into<String>) -> Self {
        self.author = Some(author.into());
        self
    }
}

/// How `revise` should treat the head.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReviseMode {
    /// Edit a `proposed` head in place; create a new revision from a sealed one.
    #[default]
    Auto,
    /// Edit the head in place. A sealed head is `AKR-C032`.
    InPlace,
    /// Always create revision n+1.
    NewRevision,
}

/// An edit to apply to a record.
#[derive(Debug, Clone, Default)]
pub struct Edits {
    /// Replace the title.
    pub title: Option<String>,
    /// Move to a state. An illegal transition fails validation with `AKR-T011`.
    pub state: Option<State>,
    /// Replace the whole record. Everything but the identifier is taken from it.
    pub replace_with: Option<Box<Record>>,
}

/// A disposition supplied on the command line, before its target is checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispositionRequest {
    /// The child being dispositioned.
    pub child: LogicalKey,
    /// What happened to it.
    pub outcome: DispositionOutcome,
    /// Where it went.
    pub into: Option<LogicalKey>,
    /// Why.
    pub note: Option<String>,
}

// -------------------------------------------------------------------------------------
// the operations
// -------------------------------------------------------------------------------------

/// Creates revision 1 of a new key, in the initial state of its class.
///
/// The record is written to the conventional file for its namespace and kind group,
/// creating it if needed. An existing key is refused: use [`revise`].
///
/// # Errors
/// Refuses if the key exists, or if the resulting ledger does not validate.
pub fn propose(
    context: &WriteContext,
    key: &LogicalKey,
    kind: Kind,
    title: &str,
    template: Option<Record>,
) -> WriteResult {
    let (mut staged, _write_lock) = load_locked(context, Operation::Propose)?;
    if !staged.ledger.revisions_of(key).is_empty() {
        return Err(Refused::new(
            Operation::Propose,
            codes::L041,
            format!("{key} already exists"),
        )
        .with_help(format!("use `akr revise {key}` to change it")));
    }

    let id = RevisionId::new(key.clone(), 1);
    let mut record = template.unwrap_or_else(|| blank(&id, kind));
    record.id = id.clone();
    record.kind = kind;
    if !title.is_empty() {
        record.title = title.to_owned();
    }
    if record.title.is_empty() {
        record.title = key.to_string();
    }
    if record.author.is_none() {
        record.author.clone_from(&context.author);
    }
    if !kind.class().states().contains(&record.state) {
        record.state = kind.class().initial()[0];
    }

    let file = conventional_file(key, kind);
    apply(
        context,
        staged_mut(&mut staged),
        Operation::Propose,
        &file,
        &record,
        ChangeKind::Created,
    )
}

/// Creates several revision-one records in one atomic pipeline pass.
///
/// This is the bulk primitive used when a verification run produces several evidence
/// records. Every key is checked before the in-memory edits are applied, the resulting
/// ledger is validated once, and either every touched file is committed or none is.
///
/// # Errors
/// Refuses an empty batch, an existing or repeated key, or an invalid resulting ledger.
pub fn propose_many(context: &WriteContext, templates: &[Record]) -> WriteResult {
    let (mut staged, _write_lock) = load_locked(context, Operation::Propose)?;
    if templates.is_empty() {
        return Err(Refused::new(
            Operation::Propose,
            cli::C031,
            "a proposal batch needs at least one record",
        ));
    }

    let mut seen = BTreeSet::new();
    let mut edits = Vec::with_capacity(templates.len());
    for template in templates {
        let key = &template.id.key;
        if !seen.insert(key.clone()) || !staged.ledger.revisions_of(key).is_empty() {
            return Err(Refused::new(
                Operation::Propose,
                codes::L041,
                format!("{key} already exists"),
            )
            .with_help(format!("use `akr revise {key}` to change it")));
        }

        let mut record = template.clone();
        record.id = RevisionId::new(key.clone(), 1);
        if record.title.is_empty() {
            record.title = key.to_string();
        }
        if record.author.is_none() {
            record.author.clone_from(&context.author);
        }
        if !record.kind.class().states().contains(&record.state) {
            record.state = record.kind.class().initial()[0];
        }
        edits.push((
            conventional_file(key, record.kind),
            record,
            ChangeKind::Created,
        ));
    }

    apply_many(context, staged_mut(&mut staged), Operation::Propose, &edits)
}

/// Edits the head of a key, or creates revision n+1 of it.
///
/// A `proposed` head is edited in place: D-015 makes proposed revisions editable, and
/// creating revision 2 of a proposal nobody accepted would be noise. A sealed head
/// produces revision n+1 with a `supersedes` edge, which is the only way to change a
/// settled record.
///
/// # Errors
/// Refuses on an unknown key, an in-place edit of a sealed head (`AKR-C032`), or a
/// resulting ledger that does not validate.
pub fn revise(
    context: &WriteContext,
    key: &LogicalKey,
    mode: ReviseMode,
    edits: &Edits,
) -> WriteResult {
    revise_with_dispositions(context, key, mode, edits, &[])
}

/// Revises a record while dispositioning unfinished children of a sealed planning head.
///
/// This is the write-surface form of [`revise`]; the simpler function remains for callers
/// that know no pinned unfinished children exist.
pub fn revise_with_dispositions(
    context: &WriteContext,
    key: &LogicalKey,
    mode: ReviseMode,
    edits: &Edits,
    dispositions: &[DispositionRequest],
) -> WriteResult {
    let (mut staged, _write_lock) = load_locked(context, Operation::Revise)?;
    let head = head_of(&staged, key, Operation::Revise)?.clone();
    let file = file_of(&staged, &head.id, Operation::Revise)?;

    let in_place = match mode {
        ReviseMode::InPlace => {
            if head.is_sealed() {
                return Err(Refused::new(
                    Operation::Revise,
                    cli::C032,
                    format!(
                        "{} is sealed ({}); create a new revision",
                        head.id, head.state
                    ),
                )
                .with_help(format!("run `akr revise {key}` without --in-place")));
            }
            true
        }
        ReviseMode::NewRevision => false,
        ReviseMode::Auto => !head.is_sealed(),
    };

    if !in_place {
        // A new revision must retire the old one in the same write. Leaving both live
        // would be two live heads (V-012), and `docs/07` §4 refuses to write a ledger
        // that does not validate — so the two cannot be separated. See the P6 report on
        // `docs/04` §2.1, which describes the unretired intermediate state.
        let mut record = edited(&head, edits);
        record.id = RevisionId::new(key.clone(), head.id.revision + 1);
        let initial = head.kind.class().initial()[0];
        record.state = initial;
        if let Some(state) = edits.state {
            record.state = state;
        }
        let mut applied = retire_and_replace(
            context,
            &mut staged,
            Operation::Revise,
            &head,
            record,
            dispositions,
        )?;
        if edits.state.is_none() {
            applied.notes.insert(
                0,
                format!(
                    "{} starts {initial} (sealed head was {}); pass --state to keep it live",
                    RevisionId::new(key.clone(), head.id.revision + 1),
                    head.state
                ),
            );
        }
        return Ok(applied);
    }

    let mut record = edited(&head, edits);
    record.id = head.id.clone();
    let change = if edits.state.is_some() && edits.title.is_none() && edits.replace_with.is_none() {
        ChangeKind::StateChanged {
            from: head.state,
            to: record.state,
        }
    } else {
        ChangeKind::Edited
    };
    apply(
        context,
        staged_mut(&mut staged),
        Operation::Revise,
        &file,
        &record,
        change,
    )
}

/// The shared core of `supersede` and of `revise` on a sealed head.
///
/// Both create revision n+1, point it at n with `supersedes`, and move n to `superseded`
/// in the same write. For planning records both demand a disposition for every unfinished
/// child (D-017): the requirement belongs to the act of replacing a plan, not to the name
/// of the command that does it.
fn retire_and_replace(
    context: &WriteContext,
    staged: &mut Staged,
    operation: Operation,
    head: &Record,
    mut record: Record,
    dispositions: &[DispositionRequest],
) -> WriteResult {
    let key = head.id.key.clone();
    let file = file_of(staged, &head.id, operation)?;

    if head.kind.class() == Class::Planning {
        let children = unfinished_children(staged, &head.id);
        let supplied: BTreeSet<&LogicalKey> = dispositions.iter().map(|d| &d.child).collect();
        let missing: Vec<UnfinishedChild> = children
            .into_iter()
            .filter(|c| !supplied.contains(&c.key))
            .collect();
        if !missing.is_empty() {
            let help = missing
                .iter()
                .map(|c| format!("  --disposition {}=carried_forward:<target>", c.key))
                .collect::<Vec<_>>()
                .join("\n");
            let mut refusal = Refused::new(
                operation,
                codes::R014,
                format!(
                    "{} unfinished {} of {}",
                    missing.len(),
                    if missing.len() == 1 {
                        "child"
                    } else {
                        "children"
                    },
                    head.id
                ),
            )
            .with_help(format!("rerun with, for example,\n{help}"));
            refusal.unfinished_children = missing;
            return Err(refusal);
        }
    }

    record.relations.insert(
        Relation::Supersedes,
        supersedes_targets(staged, &key, head.id.revision),
    );
    if !dispositions.is_empty() {
        record.dispositions = dispositions.iter().map(build_disposition).collect();
        record.dispositions.sort_by(|a, b| a.target.cmp(&b.target));
    }
    if record.author.is_none() {
        record.author.clone_from(&context.author);
    }

    let mut retired = head.clone();
    retired.state = State::Superseded;

    let successor_rev = record.id.revision;
    let mut edits = vec![
        (
            file.clone(),
            retired,
            ChangeKind::StateChanged {
                from: head.state,
                to: State::Superseded,
            },
        ),
        (file, record.clone(), ChangeKind::Created),
    ];
    // Live non-historical pins of the retired revision would become AKR-L021 the
    // moment n is superseded, and the referrer cannot be moved to n+1 first because
    // n+1 does not exist yet. Follow them here, in the same write. Historical pins
    // stay: they cite that revision for good.
    let mut followed = Vec::new();
    for candidate in staged.ledger.records() {
        if candidate.id.key == key || !candidate.is_live() {
            continue;
        }
        let mut updated = candidate.clone();
        if updated.rewrite_non_historical_pins(|reference| {
            follow_pin(reference, &key, head.id.revision, successor_rev)
        }) {
            let referrer_file = file_of(staged, &updated.id, operation)?;
            followed.push(updated.id.clone());
            edits.push((referrer_file, updated, ChangeKind::Edited));
        }
    }
    followed.sort();
    edits.sort_by(|left, right| left.1.id.cmp(&right.1.id));

    let mut applied = apply_many(context, staged, operation, &edits)?;
    if !followed.is_empty() {
        applied.notes.insert(
            0,
            format!(
                "repointed {} live pin{} of {} onto {}",
                followed.len(),
                if followed.len() == 1 { "" } else { "s" },
                head.id,
                record.id,
            ),
        );
    }
    Ok(applied)
}

/// Whether `reference` is a pin of `key` at `old` that should follow onto `new`.
fn follow_pin(reference: &mut Reference, key: &LogicalKey, old: u32, new: u32) -> bool {
    if reference.key == *key && reference.revision == Some(old) {
        reference.revision = Some(new);
        true
    } else {
        false
    }
}

/// Creates a revision superseding the head, moving the old head to `superseded`.
///
/// For planning records, every unfinished `part_of` child of the old head must have a
/// disposition (D-017). Missing ones are listed in the refusal, which is the whole point
/// of the command: it is the moment the author knows the answer and the only moment
/// anyone will ask.
///
/// # Errors
/// Refuses on an unknown key, a missing disposition, or a resulting ledger that does not
/// validate.
pub fn supersede(
    context: &WriteContext,
    key: &LogicalKey,
    dispositions: &[DispositionRequest],
) -> WriteResult {
    let (mut staged, _write_lock) = load_locked(context, Operation::Supersede)?;
    let head = head_of(&staged, key, Operation::Supersede)?.clone();

    let mut record = head.clone();
    record.id = RevisionId::new(key.clone(), head.id.revision + 1);
    record.state = head.kind.class().initial()[0];
    record.dispositions.clear();

    retire_and_replace(
        context,
        &mut staged,
        Operation::Supersede,
        &head,
        record,
        dispositions,
    )
}

/// Retires one key in favour of an already-proposed head under another key.
///
/// The replacement is proposed separately so its complete body, title and scope can be
/// reviewed before this atomic graph transition. This operation adds the pinned
/// `supersedes` edge, retires the old head, and applies planning dispositions together.
///
/// # Errors
/// Refuses unless both keys resolve, the replacement is a distinct proposed head of the
/// same kind, every unfinished planning child is dispositioned, and the result validates.
pub fn supersede_with(
    context: &WriteContext,
    old_key: &LogicalKey,
    new_key: &LogicalKey,
    dispositions: &[DispositionRequest],
) -> WriteResult {
    if old_key == new_key {
        return supersede(context, old_key, dispositions);
    }

    let (mut staged, _write_lock) = load_locked(context, Operation::Supersede)?;
    let old = head_of(&staged, old_key, Operation::Supersede)?.clone();
    let mut replacement = head_of(&staged, new_key, Operation::Supersede)?.clone();

    if old.kind != replacement.kind {
        return Err(Refused::new(
            Operation::Supersede,
            codes::L031,
            format!(
                "{} is a {}, but replacement {} is a {}; `supersedes` replaces like with like",
                old.id, old.kind, replacement.id, replacement.kind
            ),
        ));
    }
    if replacement.state != State::Proposed {
        return Err(Refused::new(
            Operation::Supersede,
            cli::C032,
            format!(
                "replacement {} is sealed ({}); propose a new replacement key first",
                replacement.id, replacement.state
            ),
        ));
    }

    if old.kind.class() == Class::Planning {
        let children = unfinished_children(&staged, &old.id);
        let supplied: BTreeSet<&LogicalKey> = dispositions.iter().map(|d| &d.child).collect();
        let missing: Vec<UnfinishedChild> = children
            .into_iter()
            .filter(|child| !supplied.contains(&child.key))
            .collect();
        if !missing.is_empty() {
            let help = missing
                .iter()
                .map(|child| format!("  --disposition {}=carried_forward:<target>", child.key))
                .collect::<Vec<_>>()
                .join("\n");
            let mut refusal = Refused::new(
                Operation::Supersede,
                codes::R014,
                format!("{} unfinished children of {}", missing.len(), old.id),
            )
            .with_help(format!("rerun with, for example,\n{help}"));
            refusal.unfinished_children = missing;
            return Err(refusal);
        }
    }

    replacement.relations.insert(
        Relation::Supersedes,
        vec![Reference::pinned(old_key.clone(), old.id.revision)],
    );
    replacement.dispositions = dispositions.iter().map(build_disposition).collect();
    replacement
        .dispositions
        .sort_by(|a, b| a.target.cmp(&b.target));

    let mut retired = old.clone();
    retired.state = State::Superseded;
    let old_file = file_of(&staged, &old.id, Operation::Supersede)?;
    let new_file = file_of(&staged, &replacement.id, Operation::Supersede)?;
    apply_many(
        context,
        &mut staged,
        Operation::Supersede,
        &[
            (
                old_file,
                retired,
                ChangeKind::StateChanged {
                    from: old.state,
                    to: State::Superseded,
                },
            ),
            (new_file, replacement, ChangeKind::Edited),
        ],
    )
}

/// Moves a `milestone` or `work` record to `completed`.
///
/// Every acceptance check must be satisfied (V-020). An unsatisfied one is refused with
/// the check named, and nothing is written.
///
/// # Errors
/// Refuses on an unknown key, a non-planning kind, an unsatisfied check, or a resulting
/// ledger that does not validate.
pub fn complete(
    context: &WriteContext,
    key: &LogicalKey,
    check_evidence: &[(String, Reference)],
) -> WriteResult {
    let (mut staged, _write_lock) = load_locked(context, Operation::Complete)?;
    let head = head_of(&staged, key, Operation::Complete)?.clone();
    let file = file_of(&staged, &head.id, Operation::Complete)?;

    if !matches!(head.kind, Kind::Milestone | Kind::Work) {
        return Err(Refused::new(
            Operation::Complete,
            codes::T011,
            format!(
                "{} is a {}; only milestone and work records complete",
                head.id, head.kind
            ),
        ));
    }

    let mut record = head.clone();
    if let Some(acceptance) = &mut record.acceptance {
        for (id, reference) in check_evidence {
            if let Some(check) = acceptance.checks.iter_mut().find(|c| c.id.as_str() == id)
                && !check.verified_by.contains(reference)
            {
                check.verified_by.push(reference.clone());
            }
        }
    }
    record.state = State::Completed;

    // Ask V-020 directly, so the refusal can name the checks rather than echo a
    // diagnostic stream. The pipeline validates the whole result again below.
    let probe = {
        let mut probe = crate::model::Ledger::new(staged.ledger.project.clone());
        let mut records: Vec<Record> = staged.ledger.records().to_vec();
        for existing in &mut records {
            if existing.id == record.id {
                existing.clone_from(&record);
            }
        }
        probe.extend(records);
        validate::v020_acceptance_satisfied(&probe)
    };
    if !probe.is_empty() {
        let unsatisfied: Vec<UnsatisfiedCheck> = probe
            .iter()
            .map(|d| UnsatisfiedCheck {
                id: check_name(&d.message).unwrap_or_default(),
                reason: d.message.clone(),
            })
            .collect();
        let mut refusal = Refused::new(
            Operation::Complete,
            codes::R022,
            format!(
                "{} has {} unsatisfied acceptance check(s)",
                head.id,
                unsatisfied.len()
            ),
        )
        .with_help("record the evidence, then rerun with --check <id>=<evidence-ref>");
        refusal.diagnostics = probe;
        refusal.unsatisfied_checks = unsatisfied;
        return Err(refusal);
    }

    apply(
        context,
        staged_mut(&mut staged),
        Operation::Complete,
        &file,
        &record,
        ChangeKind::StateChanged {
            from: head.state,
            to: State::Completed,
        },
    )
}

/// Moves a planning record to `abandoned`, demanding a disposition for every unfinished
/// child.
///
/// The reason is written to the `note` slot, which D-026 added to the planning kinds for
/// exactly this. An earlier implementation used a leading comment; comments are excluded
/// from the seal hash and invisible to views, and an abandonment reason is durable
/// knowledge that belongs on the record.
///
/// # Errors
/// Refuses on an unknown key, a non-planning kind, a missing disposition, or a resulting
/// ledger that does not validate.
pub fn abandon(
    context: &WriteContext,
    key: &LogicalKey,
    reason: &str,
    dispositions: &[DispositionRequest],
) -> WriteResult {
    let (mut staged, _write_lock) = load_locked(context, Operation::Abandon)?;
    let head = head_of(&staged, key, Operation::Abandon)?.clone();
    let file = file_of(&staged, &head.id, Operation::Abandon)?;

    if head.kind.class() != Class::Planning {
        return Err(Refused::new(
            Operation::Abandon,
            codes::T011,
            format!(
                "{} is a {}; only planning records are abandoned",
                head.id, head.kind
            ),
        ));
    }
    if reason.trim().is_empty() {
        return Err(Refused::new(
            Operation::Abandon,
            cli::C031,
            "a reason is required".to_owned(),
        )
        .with_help("rerun with --reason \"<why>\""));
    }

    let children = unfinished_children(&staged, &head.id);
    let supplied: BTreeSet<&LogicalKey> = dispositions.iter().map(|d| &d.child).collect();
    let missing: Vec<UnfinishedChild> = children
        .into_iter()
        .filter(|c| !supplied.contains(&c.key))
        .collect();
    if !missing.is_empty() {
        let help = missing
            .iter()
            .map(|c| format!("  --disposition {}=intentionally_dropped", c.key))
            .collect::<Vec<_>>()
            .join("\n");
        let mut refusal = Refused::new(
            Operation::Abandon,
            codes::R014,
            format!("{} unfinished children of {}", missing.len(), head.id),
        )
        .with_help(format!(
            "abandoning a plan silently is what D-017 exists to prevent; rerun with\n{help}"
        ));
        refusal.unfinished_children = missing;
        return Err(refusal);
    }

    let mut record = head.clone();
    record.state = State::Abandoned;
    record.content.insert(
        crate::model::ContentSlot::Note,
        crate::model::ContentValue::prose(reason.trim()),
    );
    if !dispositions.is_empty() {
        record.dispositions = dispositions.iter().map(build_disposition).collect();
        record.dispositions.sort_by(|a, b| a.target.cmp(&b.target));
    }

    apply(
        context,
        staged_mut(&mut staged),
        Operation::Abandon,
        &file,
        &record,
        ChangeKind::StateChanged {
            from: head.state,
            to: State::Abandoned,
        },
    )
}

// -------------------------------------------------------------------------------------
// import
// -------------------------------------------------------------------------------------

/// One record drafted from a legacy document (`docs/12-migration.md` §3).
#[derive(Debug, Clone)]
pub struct ImportedRecord {
    /// The proposed key.
    pub key: LogicalKey,
    /// The proposed kind.
    pub kind: Kind,
    /// The proposed title.
    pub title: String,
    /// Text for the kind's first required content slot.
    pub body: String,
    /// The verbatim passage, for the `source` block's excerpt.
    pub excerpt: String,
    /// The identifier of this claim's check on the tracking record.
    pub check_id: crate::model::Segment,
}

/// Everything one `akr import` invocation writes.
#[derive(Debug, Clone)]
pub struct ImportRequest {
    /// The legacy document, repo-relative — every `source` block's `path`.
    pub document: String,
    /// The drafted records, one per durable claim.
    pub records: Vec<ImportedRecord>,
    /// The tracking `work` record (D-022). Created if absent.
    pub tracking: LogicalKey,
}

/// Drafts every record of a legacy document and its tracking record in one write.
///
/// The whole import is one [`apply_many`]: one validation of the resulting ledger, one
/// atomic write. Either every drafted record, the tracking record and its checks land
/// together, or nothing does — a half-imported document would be exactly the untracked
/// state `AKR-M031` exists to flag.
///
/// Everything lands `proposed` with a `source { kind legacy }` block. Those are
/// construction invariants here, but they are also *checked* here (`AKR-M042`,
/// `AKR-M021`): the workflow's audit trail rests on them, so a future refactor that
/// broke one should fail loudly rather than import quietly.
///
/// # Errors
/// Refuses on an undeclared namespace (`AKR-M013`), a colliding key (`AKR-M012`), a
/// tracking key that is not a `work` record, or a resulting ledger that does not
/// validate.
pub fn import(context: &WriteContext, request: &ImportRequest) -> WriteResult {
    use crate::diagnostics::codes::migration;
    use crate::model::{Acceptance, Check, CheckMethod, Source, SourceKind};

    let (mut staged, _write_lock) = load_locked(context, Operation::Import)?;
    let namespaces = &staged.ledger.project.namespaces;

    for key in request
        .records
        .iter()
        .map(|r| &r.key)
        .chain(std::iter::once(&request.tracking))
    {
        if !namespaces.contains(key.namespace()) {
            return Err(Refused::new(
                Operation::Import,
                migration::M013,
                format!(
                    "{key}: namespace {} is not declared in project.akr",
                    key.namespace()
                ),
            )
            .with_help("declare the namespace, or rerun with --namespace <ns>"));
        }
    }
    for record in &request.records {
        if !staged.ledger.revisions_of(&record.key).is_empty() {
            return Err(Refused::new(
                Operation::Import,
                migration::M012,
                format!(
                    "{} already exists; imported records may not overwrite ledger records",
                    record.key
                ),
            )
            .with_help(format!(
                "revise it deliberately instead: `akr revise {}`",
                record.key
            )));
        }
    }

    let source_block = |excerpt: &str| Source {
        kind: SourceKind::Legacy,
        role: None,
        path: Some(request.document.clone()),
        url: None,
        excerpt: Some(excerpt.to_owned()),
        // Legacy migration cites a path, not a registered document: the whole point of
        // that workflow is that the original is on its way out (D-022).
        document: None,
        range: None,
        use_note: None,
    };

    let mut edits: Vec<(PathBuf, Record, ChangeKind)> = Vec::new();
    for imported in &request.records {
        let id = RevisionId::new(imported.key.clone(), 1);
        let mut record = blank(&id, imported.kind);
        record.title.clone_from(&imported.title);
        if let Some(slot) = imported.kind.content_slots().iter().find(|s| s.required) {
            record.content.insert(
                slot.slot,
                crate::model::ContentValue::prose(imported.body.trim_end()),
            );
        }
        record.sources.push(source_block(&imported.excerpt));
        record.author.clone_from(&context.author);

        // The M042/M021 self-check. Unreachable by construction today; load-bearing the
        // day someone changes the construction. Inquiry is the one class with no
        // `proposed` state — a question lands `open`, its only initial state, which is
        // the closest thing it has (`docs/12` §3).
        let expected = if imported.kind.class() == Class::Inquiry {
            State::Open
        } else {
            State::Proposed
        };
        if record.state != expected {
            return Err(Refused::new(
                Operation::Import,
                migration::M042,
                format!(
                    "{id} was produced by import in state {}; imports land as proposed",
                    record.state
                ),
            ));
        }
        if !record.sources.iter().any(|s| s.kind == SourceKind::Legacy) {
            return Err(Refused::new(
                Operation::Import,
                migration::M021,
                format!("{id} was produced by import but has no source block with kind legacy"),
            ));
        }

        edits.push((
            conventional_file(&imported.key, imported.kind),
            record,
            ChangeKind::Created,
        ));
    }

    let checks: Vec<Check> = request
        .records
        .iter()
        .map(|imported| Check {
            id: imported.check_id.clone(),
            statement: format!(
                "\"{}\" is dispositioned: promoted as {} or declined with evidence",
                imported.title, imported.key
            ),
            method: CheckMethod::Manual,
            command: None,
            verified_by: Vec::new(),
        })
        .collect();

    if staged.ledger.revisions_of(&request.tracking).is_empty() {
        let id = RevisionId::new(request.tracking.clone(), 1);
        let mut record = blank(&id, Kind::Work);
        record.title = format!("Import {}", request.document);
        record.content.insert(
            crate::model::ContentSlot::Intent,
            crate::model::ContentValue::prose(&format!(
                "Disposition every durable claim of {} (docs/12-migration.md).",
                request.document
            )),
        );
        record.acceptance = Some(Acceptance { checks });
        record.sources.push(source_block(""));
        if let Some(source) = record.sources.last_mut() {
            source.excerpt = None;
        }
        record.author.clone_from(&context.author);
        edits.push((
            conventional_file(&request.tracking, Kind::Work),
            record,
            ChangeKind::Created,
        ));
    } else {
        let head = head_of(&staged, &request.tracking, Operation::Import)?.clone();
        if head.kind != Kind::Work {
            return Err(Refused::new(
                Operation::Import,
                codes::T011,
                format!(
                    "{} is a {}; a tracking record is work (D-022)",
                    head.id, head.kind
                ),
            ));
        }
        let file = file_of(&staged, &head.id, Operation::Import)?;
        let mut record = head.clone();
        let acceptance = record.acceptance.get_or_insert_with(Acceptance::default);
        let existing: BTreeSet<String> = acceptance
            .checks
            .iter()
            .map(|c| c.id.as_str().to_owned())
            .collect();
        acceptance.checks.extend(
            checks
                .into_iter()
                .filter(|c| !existing.contains(c.id.as_str())),
        );
        acceptance.checks.sort_by(|a, b| a.id.cmp(&b.id));
        if !record
            .sources
            .iter()
            .any(|s| s.kind == SourceKind::Legacy && s.path.as_deref() == Some(&request.document))
        {
            let mut source = source_block("");
            source.excerpt = None;
            record.sources.push(source);
        }

        if head.is_sealed() {
            // The same shape as `retire_and_replace`, inlined so the whole import stays
            // one write. A sealed tracking head gains revision n+1; D-017 still holds.
            let children = unfinished_children(&staged, &head.id);
            if !children.is_empty() {
                let mut refusal = Refused::new(
                    Operation::Import,
                    codes::R014,
                    format!("{} unfinished children of {}", children.len(), head.id),
                );
                refusal.unfinished_children = children;
                return Err(refusal.with_help(
                    "disposition the children with `akr supersede`, then rerun the import",
                ));
            }
            record.id = RevisionId::new(request.tracking.clone(), head.id.revision + 1);
            record.state = head.kind.class().initial()[0];
            record.relations.insert(
                Relation::Supersedes,
                vec![Reference::pinned(
                    request.tracking.clone(),
                    head.id.revision,
                )],
            );
            let mut retired = head.clone();
            retired.state = State::Superseded;
            edits.push((
                file.clone(),
                retired,
                ChangeKind::StateChanged {
                    from: head.state,
                    to: State::Superseded,
                },
            ));
            edits.push((file, record, ChangeKind::Created));
        } else {
            edits.push((file, record, ChangeKind::Edited));
        }
    }

    apply_many(context, staged_mut(&mut staged), Operation::Import, &edits)
}

// -------------------------------------------------------------------------------------
// pipeline
// -------------------------------------------------------------------------------------

/// Step 1 of `docs/07` §4, with the exclusive claim taken before the ledger is read.
///
/// The claim must outlive the whole operation: it is taken *before* step 1 so that the
/// bytes the ledger is built from cannot change under it, and released only once step 5
/// has renamed every file into place. Callers hold it in a binding that lives to the end
/// of the operation; dropping it early reopens exactly the window it exists to close.
fn load_locked(
    context: &WriteContext,
    operation: Operation,
) -> Result<(Staged, WriteLock), Refused> {
    let lock = WriteLock::acquire(&context.akr_dir);
    let staged = Staged::load(&context.akr_dir)
        .map_err(|error| Refused::new(operation, cli::C012, error.to_string()))?;
    Ok((staged, lock))
}

fn staged_mut(staged: &mut Staged) -> &mut Staged {
    staged
}

fn head_of<'a>(
    staged: &'a Staged,
    key: &LogicalKey,
    operation: Operation,
) -> Result<&'a Record, Refused> {
    staged.ledger.head(key).map_err(|error| {
        Refused::new(operation, codes::L001, error.to_string())
            .with_help(format!("`akr propose {key} --kind <kind>` creates it"))
    })
}

fn file_of(staged: &Staged, id: &RevisionId, operation: Operation) -> Result<PathBuf, Refused> {
    staged
        .ledger
        .get(id)
        .and_then(|r| r.file.as_ref())
        .map(PathBuf::from)
        .ok_or_else(|| Refused::new(operation, codes::L006, format!("{id} has no source file")))
}

fn apply(
    context: &WriteContext,
    staged: &mut Staged,
    operation: Operation,
    file: &Path,
    record: &Record,
    change: ChangeKind,
) -> WriteResult {
    apply_many(
        context,
        staged,
        operation,
        &[(file.to_path_buf(), record.clone(), change)],
    )
}

fn apply_many(
    context: &WriteContext,
    staged: &mut Staged,
    operation: Operation,
    edits: &[(PathBuf, Record, ChangeKind)],
) -> WriteResult {
    apply_inner(context, staged, operation, edits)
}

/// Steps 2 through 5 of `docs/07` §4.
fn apply_inner(
    context: &WriteContext,
    staged: &mut Staged,
    operation: Operation,
    edits: &[(PathBuf, Record, ChangeKind)],
) -> WriteResult {
    let project = staged.project.clone();
    let mut touched: Vec<PathBuf> = Vec::new();
    let mut changes: Vec<Change> = Vec::new();
    let mut before: BTreeMap<PathBuf, Option<String>> = BTreeMap::new();

    // Step 2: apply in memory.
    for (file, record, change) in edits {
        let Some(node) = emit::record_node(record, &project) else {
            return Err(Refused::new(
                operation,
                cli::C031,
                format!("{} could not be rendered as canonical source", record.id),
            ));
        };
        before
            .entry(file.clone())
            .or_insert_with(|| staged.texts.get(file).cloned());
        let mut tree = staged
            .trees
            .get(file)
            .cloned()
            .unwrap_or_else(|| empty_file(&project));
        splice(&mut tree, node);
        staged.set_tree(file, &tree);
        if !touched.contains(file) {
            touched.push(file.clone());
        }
        changes.push(Change {
            id: record.id.clone(),
            kind: change.clone(),
            file: file.clone(),
        });
    }

    // Step 3: validate the result, and refuse what this write introduced (D-039).
    staged.reparse();
    let mut diagnostics = staged.diagnostics.clone();
    diagnostics.extend(validate::validate_all(&staged.ledger));
    let errors = Staged::errors(&diagnostics, context.strict);
    let inherited = if errors.is_empty() {
        Vec::new()
    } else {
        inherited_of(staged, &before, context.strict, &errors)
    };
    let introduced = introduced_of(&errors, &inherited);
    if !introduced.is_empty() {
        let mut message = format!(
            "write aborted: this write would introduce {} diagnostic(s); nothing was written",
            introduced.len()
        );
        if !inherited.is_empty() {
            message.push_str(&format!(
                " ({} the ledger already had are not counted)",
                inherited.len()
            ));
        }
        let mut refusal = Refused::new(operation, cli::C031, message);
        refusal.diagnostics = introduced;
        return Err(refusal);
    }

    // Steps 4 and 5: the text is already canonical; write it.
    //
    // Last, immediately before the renames: every file this is about to replace must
    // still hold the bytes step 1 read. `WriteLock` keeps AKR's own writers out of this
    // window, so a disagreement here means something the lock does not cover changed a
    // source mid-operation — an editor, a `git checkout`, or a filesystem that could not
    // lock. Refusing is the same promise the rest of the pipeline makes; overwriting
    // would discard a change nobody asked to discard.
    if let Some(clobbered) = exclusion::verify_unchanged(&context.akr_dir, before.iter()) {
        return Err(Refused::new(
            operation,
            cli::C034,
            format!(
                "write aborted: {} changed on disk while this write was being prepared; \
                 nothing was written",
                clobbered.display()
            ),
        )
        .with_help("re-read the ledger and retry; another writer or an editor got there first"));
    }

    touched.sort();
    staged
        .commit(&touched)
        .map_err(|error| Refused::new(operation, cli::C031, format!("write failed: {error}")))?;

    changes.sort_by(|a, b| a.id.cmp(&b.id));
    let lock_stale = changes
        .iter()
        .any(|c| !matches!(c.kind, ChangeKind::Edited));
    let mut notes = Vec::new();
    if !inherited.is_empty() {
        notes.push(inherited_note(&inherited));
    }
    notes.extend(acceptance_notes(&staged.ledger, operation, edits));
    Ok(Applied {
        operation,
        changes,
        files: touched,
        // The inherited errors are deliberately not repeated here: this field is rendered
        // in full, and thirty-five of them on every write would bury what the write did.
        // The note names the count and the codes, and `akr validate` lists them.
        diagnostics: diagnostics
            .into_iter()
            .filter(|d| d.severity == Severity::Warning)
            .collect(),
        notes,
        lock_stale,
    })
}

/// The errors the ledger already had, before this operation touched it.
///
/// Restores the pre-edit text of every touched file, re-derives, and puts the edited text
/// back — two extra passes, paid only when the result has errors at all, which is the
/// path that was about to fail anyway.
fn inherited_of(
    staged: &mut Staged,
    before: &BTreeMap<PathBuf, Option<String>>,
    strict: bool,
    errors: &[Diagnostic],
) -> Vec<Diagnostic> {
    let after = swap_texts(staged, before);
    let mut diagnostics = staged.diagnostics.clone();
    diagnostics.extend(validate::validate_all(&staged.ledger));
    let baseline = Staged::errors(&diagnostics, strict);
    swap_texts(staged, &after);
    // Only what survived into the result: a diagnostic the write repaired is not
    // something the caller is still carrying.
    let mut standing: BTreeMap<String, usize> = BTreeMap::new();
    for diagnostic in errors {
        *standing.entry(fingerprint(diagnostic)).or_default() += 1;
    }
    baseline
        .into_iter()
        .filter(
            |diagnostic| match standing.get_mut(&fingerprint(diagnostic)) {
                Some(count) if *count > 0 => {
                    *count -= 1;
                    true
                }
                _ => false,
            },
        )
        .collect()
}

/// The errors in the result that the baseline did not already account for.
fn introduced_of(errors: &[Diagnostic], inherited: &[Diagnostic]) -> Vec<Diagnostic> {
    let mut remaining: BTreeMap<String, usize> = BTreeMap::new();
    for diagnostic in inherited {
        *remaining.entry(fingerprint(diagnostic)).or_default() += 1;
    }
    errors
        .iter()
        .filter(
            |diagnostic| match remaining.get_mut(&fingerprint(diagnostic)) {
                Some(count) if *count > 0 => {
                    *count -= 1;
                    false
                }
                _ => true,
            },
        )
        .cloned()
        .collect()
}

/// Swaps a set of file texts into the staged workspace, returning what was there.
///
/// Only `texts` is swapped: [`Staged::reparse`] rebuilds every tree from the texts, so
/// restoring both maps by hand would be a second way to say the same thing.
fn swap_texts(
    staged: &mut Staged,
    texts: &BTreeMap<PathBuf, Option<String>>,
) -> BTreeMap<PathBuf, Option<String>> {
    let mut previous = BTreeMap::new();
    for (path, text) in texts {
        let was = match text {
            Some(text) => staged.texts.insert(path.clone(), text.clone()),
            None => staged.texts.remove(path),
        };
        previous.insert(path.clone(), was);
    }
    staged.reparse();
    previous
}

/// What identifies a diagnostic across two derivations of the same ledger.
///
/// Not the span: rewriting a file canonically moves every line below the edit, and a
/// diagnostic about an untouched record is the same diagnostic wherever it now sits.
fn fingerprint(diagnostic: &Diagnostic) -> String {
    format!(
        "{}|{}|{:?}|{}",
        diagnostic.code,
        diagnostic.rule.map_or_else(String::new, |r| r.to_string()),
        diagnostic.primary.subject,
        diagnostic.message
    )
}

/// The advisory line for diagnostics a write inherited rather than caused.
fn inherited_note(inherited: &[Diagnostic]) -> String {
    let mut codes: Vec<String> = inherited
        .iter()
        .map(|d| d.code.to_string())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    codes.sort();
    format!(
        "the ledger still has {} diagnostic(s) this write did not introduce ({}) \u{2014} \
         `akr validate` lists them",
        inherited.len(),
        codes.join(", ")
    )
}

/// What a revision of a completed record has just put back in question.
///
/// V-020 wants every acceptance check's evidence observed at a commit that descends from
/// the last commit to change the record's content — so revising a completed record
/// invalidates *all* of its evidence at once, and refreshing three checks out of four
/// leaves the fourth to surface as `AKR-R022` on some later build, long after the session
/// that could have refreshed it (`akr.papercut.collated-19-papercuts-from-evidence-intake`,
/// from jpegXL-rs). The write pipeline cannot decide the question itself: the commit this
/// revision will land in does not exist yet. It can say which references are in play, at
/// the one moment somebody is looking, which is what stops a partial refresh from being
/// invisible.
fn acceptance_notes(
    ledger: &crate::model::Ledger,
    operation: Operation,
    edits: &[(PathBuf, Record, ChangeKind)],
) -> Vec<String> {
    if !matches!(operation, Operation::Revise | Operation::Supersede) {
        return Vec::new();
    }
    let mut notes = Vec::new();
    for (_, record, _) in edits {
        if record.state != State::Completed {
            continue;
        }
        let Some(acceptance) = &record.acceptance else {
            continue;
        };
        if acceptance.checks.is_empty() {
            continue;
        }
        notes.push(format!(
            "{} stays `completed`; V-020 will compare every acceptance reference against \
             the commit this revision lands in:",
            record.id
        ));
        for check in &acceptance.checks {
            if check.verified_by.is_empty() {
                notes.push(format!("  {} — no evidence", check.id));
                continue;
            }
            for reference in &check.verified_by {
                let observed = ledger
                    .resolve(reference)
                    .ok()
                    .flatten()
                    .and_then(|evidence| evidence.get(crate::model::ContentSlot::ObservedAt))
                    .and_then(crate::model::ContentValue::as_commit)
                    .map_or_else(
                        || "no observed_at".to_owned(),
                        |c| c.as_str()[..8].to_owned(),
                    );
                notes.push(format!("  {} — {reference} at {observed}", check.id));
            }
        }
        notes.push("  refresh every one of them, not the ones you happened to rerun".to_owned());
    }
    notes
}

/// Replaces a record in a tree, or inserts it, preserving comments where it can.
///
/// Record-level trivia and slot-level comments whose slot survives the edit are carried
/// over. Comments on a slot the edit removed are lost, which is the honest cost of
/// regenerating a record from the model.
fn splice(tree: &mut cst::File, mut node: cst::Record) {
    let existing = tree
        .items
        .iter()
        .position(|item| matches!(item, cst::Item::Record(r) if r.key == node.key && r.revision == node.revision));

    if let Some(at) = existing
        && let cst::Item::Record(old) = &tree.items[at]
    {
        node.trivia = old.trivia.clone();
        node.inner_trailing = old.inner_trailing.clone();
        for item in &mut node.body {
            let name = item.name().to_owned();
            let carried = old
                .body
                .iter()
                .find(|o| o.name() == name)
                .map(|o| o.trivia().clone());
            if let Some(trivia) = carried {
                match item {
                    cst::BodyItem::Slot(slot) => slot.trivia = trivia,
                    cst::BodyItem::Block(block) => block.trivia = trivia,
                }
            }
        }
    }

    match existing {
        Some(at) => tree.items[at] = cst::Item::Record(node),
        None => tree.items.push(cst::Item::Record(node)),
    }
}

fn empty_file(project: &str) -> cst::File {
    use crate::diagnostics::{FileId, Span};
    cst::File {
        leading: Vec::new(),
        keyword: "akr".to_owned(),
        version: "0.1".to_owned(),
        blank_before_header: false,
        project: project.to_owned(),
        items: Vec::new(),
        trailing: Vec::new(),
        span: Span {
            file: FileId(0),
            start: 0,
            end: 0,
        },
    }
}

// -------------------------------------------------------------------------------------
// helpers
// -------------------------------------------------------------------------------------

fn blank(id: &RevisionId, kind: Kind) -> Record {
    Record {
        id: id.clone(),
        kind,
        title: String::new(),
        state: kind.class().initial()[0],
        scope: if kind.class().scope_required() {
            vec![crate::model::ScopeTerm::All]
        } else {
            Vec::new()
        },
        topic: None,
        content: std::collections::BTreeMap::new(),
        claims: Vec::new(),
        retired_claims: Vec::new(),
        acceptance: None,
        dispositions: Vec::new(),
        relations: std::collections::BTreeMap::new(),
        acknowledged: false,
        author: None,
        created_at: None,
        sources: Vec::new(),
        file: None,
    }
}

fn edited(head: &Record, edits: &Edits) -> Record {
    let mut record = edits
        .replace_with
        .clone()
        .map_or_else(|| head.clone(), |r| *r);
    record.id = head.id.clone();
    record.kind = head.kind;
    record.file.clone_from(&head.file);
    if let Some(title) = &edits.title {
        record.title.clone_from(title);
    }
    if let Some(state) = edits.state {
        record.state = state;
    }
    record
}

fn build_disposition(request: &DispositionRequest) -> Disposition {
    Disposition {
        target: Reference::head(request.child.clone()),
        outcome: request.outcome,
        into: request.into.clone().map(Reference::head),
        note: request.note.clone(),
    }
}

/// The `supersedes` edges a new head of `key` must carry.
///
/// Always the revision it is replacing, plus any other same-key revision that has no
/// incoming `supersedes` edge. A history written without back-edges is otherwise
/// unambiguous only while a live head exists; the moment every revision is terminal,
/// V-001 raises `AKR-L002` and names a count instead of the missing edges.
fn supersedes_targets(staged: &Staged, key: &LogicalKey, head_revision: u32) -> Vec<Reference> {
    let revisions = staged.ledger.revisions_of(key);
    let pointed: BTreeSet<u32> = revisions
        .iter()
        .flat_map(|record| record.targets(Relation::Supersedes))
        .filter(|target| &target.key == key)
        .filter_map(|target| target.revision)
        .collect();
    let mut revs: Vec<u32> = revisions
        .iter()
        .map(|record| record.id.revision)
        .filter(|rev| *rev == head_revision || !pointed.contains(rev))
        .collect();
    if !revs.contains(&head_revision) {
        revs.push(head_revision);
    }
    revs.sort_unstable();
    revs.dedup();
    revs.into_iter()
        .map(|rev| Reference::pinned(key.clone(), rev))
        .collect()
}

/// Live planning records whose `part_of` pins the given revision (D-017, V-017).
fn unfinished_children(staged: &Staged, parent: &RevisionId) -> Vec<UnfinishedChild> {
    let mut children: Vec<UnfinishedChild> = staged
        .ledger
        .records()
        .iter()
        .filter(|candidate| {
            candidate.kind.class() == Class::Planning
                && candidate.is_live()
                && candidate.targets(Relation::PartOf).iter().any(|t| {
                    t.key == parent.key && t.revision.is_some_and(|r| r == parent.revision)
                })
        })
        .map(|c| UnfinishedChild {
            key: c.id.key.clone(),
            state: c.state,
        })
        .collect();
    children.sort_by(|a, b| a.key.cmp(&b.key));
    children.dedup_by(|a, b| a.key == b.key);
    children
}

/// The conventional file for a key's namespace and kind group (D-018).
#[must_use]
pub fn conventional_file(key: &LogicalKey, kind: Kind) -> PathBuf {
    let group = match kind {
        Kind::Term => "terms",
        Kind::Requirement => "requirements",
        Kind::Policy => "policies",
        Kind::Constraint => "constraints",
        Kind::Decision => "decisions",
        Kind::Observation => "observations",
        Kind::Evidence => "evidence",
        Kind::Assessment => "assessments",
        Kind::Papercut => "papercuts",
        Kind::Milestone => "milestones",
        Kind::Work => "work",
        Kind::Track => "tracks",
        Kind::Question => "questions",
    };
    // Built as one `/`-separated string rather than `join`ed: this path is quoted in
    // command output and tool payloads, where the repository form is `/` on every
    // platform. `PathBuf` accepts `/` for filesystem use on Windows too.
    PathBuf::from(format!("records/{}/{group}.akr", key.namespace().as_str()))
}

/// Pulls a check identifier out of a V-020 message, for the structured refusal.
fn check_name(message: &str) -> Option<String> {
    let at = message.find("check `")? + "check `".len();
    let rest = &message[at..];
    rest.find('`').map(|end| rest[..end].to_owned())
}

/// The rule a refusal corresponds to, where there is one.
#[must_use]
pub fn refusal_rule(code: Code) -> Option<RuleId> {
    validate::RULES
        .iter()
        .find(|r| r.code == code)
        .map(|r| r.id)
}

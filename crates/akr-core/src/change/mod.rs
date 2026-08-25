//! The AKR ↔ git bridge: staged semantic deltas, change transactions, commit messages.
//!
//! `docs/16-change-protocol.md`. The shape is:
//!
//! ```text
//! AKR work record → local change transaction → staged git tree → generated commit
//! ```
//!
//! # Why there is no `commit` record kind
//!
//! One work record takes many commits; one commit advances several work records; a rebase
//! changes every object id; and a commit hash cannot be written into a file contained in
//! that same commit without an amendment loop. A durable `commit` kind would therefore be
//! a second, worse copy of git's history, and hundreds of them would drown out the
//! decisions and evidence the ledger exists to hold.
//!
//! What is needed instead is short-lived: a *change transaction* that exists only while a
//! commit is being prepared, lives in this worktree's git directory, and leaves behind
//! only the commit message and its trailers. Those trailers are the durable bridge —
//! they survive rebases and cherry-picks, `git log --grep` finds them, and every
//! AKR-to-git link can be rebuilt from them.
//!
//! # Why the staged tree, not the working tree
//!
//! An agent finishing a task typically has more modified files than the change it means
//! to make. "If the code is dirty the ledger must be dirty too" is both too strict —
//! active work spans several commits without a new revision — and too loose, because it
//! says nothing about *which* dirty files belong together. The git index already answers
//! that question, so the bridge asks it rather than guessing.

use crate::git::{GitError, IndexEntry, Repository};
use crate::hash::Sha256;
use crate::model::Commit;
use crate::model::{Ledger, RevisionId, State};
use std::collections::BTreeMap;
use std::path::Path;

mod message;

pub use message::{Trailer, commit_message};

/// What sort of change a commit is, in the conventional-commits vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// A bug fix.
    Fix,
    /// A new capability.
    Feat,
    /// A performance change with no behavioural one.
    Perf,
    /// A restructuring with no behavioural change.
    Refactor,
    /// Test-only work.
    Test,
    /// Documentation.
    Docs,
    /// Build system, packaging, CI.
    Build,
    /// Repository maintenance.
    Chore,
}

impl ChangeKind {
    /// Every kind, for `--help` and for validation.
    pub const ALL: &'static [Self] = &[
        Self::Fix,
        Self::Feat,
        Self::Perf,
        Self::Refactor,
        Self::Test,
        Self::Docs,
        Self::Build,
        Self::Chore,
    ];

    /// The name used in the transaction file and the commit subject.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fix => "fix",
            Self::Feat => "feat",
            Self::Perf => "perf",
            Self::Refactor => "refactor",
            Self::Test => "test",
            Self::Docs => "docs",
            Self::Build => "build",
            Self::Chore => "chore",
        }
    }

    /// Parses a kind name, accepting the unabbreviated spelling of one.
    ///
    /// The stored name is the Conventional Commit abbreviation, because that is what a
    /// commit subject carries. But `feat`, `docs` and `perf` are abbreviations, and the
    /// word somebody reaches for first is the whole one: `--kind feature` was refused by a
    /// tool that had already understood it. Nothing is guessed here — these are the long
    /// forms of the eight kinds, not a fuzzy match.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        let name = match name {
            "feature" => "feat",
            "documentation" => "docs",
            "performance" => "perf",
            "bugfix" => "fix",
            other => other,
        };
        Self::ALL.iter().copied().find(|k| k.as_str() == name)
    }
}

/// A change being prepared: what this commit is for, and what it advances.
///
/// Everything here is information that cannot be inferred from a work record or from a
/// git diff. `summary` in particular is not redundant with the work record's title: "Slice
/// 6 uncertainty-gated chroma limiting phase" is a good planning name and a poor commit
/// subject, and commit boundaries are finer-grained than work records anyway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeIntent {
    /// Opaque id, stable for the life of the transaction.
    pub id: String,
    /// `HEAD` when the transaction began.
    pub base_commit: String,
    /// What sort of change this is.
    pub kind: ChangeKind,
    /// The commit scope, e.g. `tone` or `jp2lam`.
    pub scope: Option<String>,
    /// Imperative one-line description; becomes the commit subject.
    pub summary: String,
    /// The work record this commit mainly advances.
    pub primary_work: Option<String>,
    /// Other records the same logical change advances.
    pub related_work: Vec<String>,
    /// An explanation specific to this commit, beyond the work record's intent.
    pub implementation_note: Option<String>,
    /// Why a material change carries no work reference.
    pub untracked_reason: Option<String>,
    /// The staged tree this transaction was prepared against, once it has been.
    pub prepared_tree: Option<String>,
    /// The implementation digest of that staged tree.
    pub prepared_digest: Option<String>,
}

impl ChangeIntent {
    /// Opens a transaction against `base_commit`.
    #[must_use]
    pub fn new(base_commit: &str, kind: ChangeKind, summary: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(base_commit.as_bytes());
        hasher.update(summary.as_bytes());
        Self {
            id: format!("chg-{}", &hasher.finish().to_hex()[..16]),
            base_commit: base_commit.to_owned(),
            kind,
            scope: None,
            summary: summary.to_owned(),
            primary_work: None,
            related_work: Vec::new(),
            implementation_note: None,
            untracked_reason: None,
            prepared_tree: None,
            prepared_digest: None,
        }
    }

    /// Every work reference this transaction names, primary first, without duplicates.
    #[must_use]
    pub fn work_refs(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for reference in self.primary_work.iter().chain(self.related_work.iter()) {
            if !out.contains(reference) {
                out.push(reference.clone());
            }
        }
        out
    }

    /// The transaction file, in the same key-per-line shape as `akr.lock`'s header.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::from("change 0.1\n\n");
        let mut line = |key: &str, value: &str| {
            out.push_str(&format!("{key} {}\n", quote(value)));
        };
        line("id", &self.id);
        line("base_commit", &self.base_commit);
        line("kind", self.kind.as_str());
        if let Some(scope) = &self.scope {
            line("scope", scope);
        }
        line("summary", &self.summary);
        if let Some(primary) = &self.primary_work {
            line("primary_work", primary);
        }
        for related in &self.related_work {
            line("related_work", related);
        }
        if let Some(note) = &self.implementation_note {
            line("implementation_note", note);
        }
        if let Some(reason) = &self.untracked_reason {
            line("untracked_reason", reason);
        }
        if let Some(tree) = &self.prepared_tree {
            line("prepared_tree", tree);
        }
        if let Some(digest) = &self.prepared_digest {
            line("prepared_digest", digest);
        }
        out
    }

    /// Reads a transaction file back.
    ///
    /// # Errors
    /// [`ChangeError::Malformed`] when a required field is missing or unreadable.
    pub fn parse(text: &str) -> Result<Self, ChangeError> {
        let mut fields: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with("change ") {
                continue;
            }
            let Some((key, value)) = line.split_once(' ') else {
                continue;
            };
            fields
                .entry(key.to_owned())
                .or_default()
                .push(unquote(value.trim()));
        }
        let one = |key: &str| fields.get(key).and_then(|v| v.first()).cloned();
        let kind = one("kind")
            .and_then(|k| ChangeKind::from_name(&k))
            .ok_or_else(|| ChangeError::Malformed("kind".into()))?;
        Ok(Self {
            id: one("id").ok_or_else(|| ChangeError::Malformed("id".into()))?,
            base_commit: one("base_commit")
                .ok_or_else(|| ChangeError::Malformed("base_commit".into()))?,
            kind,
            scope: one("scope"),
            summary: one("summary").ok_or_else(|| ChangeError::Malformed("summary".into()))?,
            primary_work: one("primary_work"),
            related_work: fields.get("related_work").cloned().unwrap_or_default(),
            implementation_note: one("implementation_note"),
            untracked_reason: one("untracked_reason"),
            prepared_tree: one("prepared_tree"),
            prepared_digest: one("prepared_digest"),
        })
    }
}

fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn unquote(value: &str) -> String {
    let trimmed = value.trim();
    let inner = trimmed
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(trimmed);
    inner.replace("\\\"", "\"").replace("\\\\", "\\")
}

/// Why a change operation could not proceed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeError {
    /// No transaction is open in this worktree.
    NoTransaction,
    /// A transaction is already open.
    AlreadyOpen(String),
    /// The transaction file is unreadable.
    Malformed(String),
    /// The staged tree moved after the transaction was prepared.
    StagedTreeMoved {
        /// The tree the transaction was prepared against.
        prepared: String,
        /// The tree that is staged now.
        found: String,
    },
    /// Nothing is staged.
    NothingStaged,
    /// The transaction was never prepared.
    NotPrepared,
    /// Several work records changed state and none was named primary.
    PrimaryRequired(Vec<String>),
    /// A work reference does not resolve in the staged ledger.
    UnknownWork(String),
    /// A material change carries neither a work reference nor an exemption.
    Untracked,
    /// Git said no.
    Git(String),
}

impl std::fmt::Display for ChangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoTransaction => write!(
                f,
                "no change transaction is open in this worktree; `akr change begin` opens one"
            ),
            Self::AlreadyOpen(id) => write!(
                f,
                "change {id} is already open; `akr change abort` discards it"
            ),
            Self::Malformed(field) => {
                write!(f, "the change transaction is missing or malformed: {field}")
            }
            Self::StagedTreeMoved { prepared, found } => write!(
                f,
                "the staged tree moved after this change was prepared\nprepared against: \
                 {prepared}\nstaged now: {found}\nhelp: run `akr change prepare --staged` again"
            ),
            Self::NothingStaged => write!(
                f,
                "nothing is staged; `git add` the exact files this change is made of"
            ),
            Self::NotPrepared => write!(
                f,
                "this change has not been prepared; run `akr change prepare --staged`"
            ),
            Self::PrimaryRequired(keys) => write!(
                f,
                "{} work records changed state and none was named primary: {}\nhelp: \
                 `akr change begin --primary <key>` picks the one the subject is about",
                keys.len(),
                keys.join(", ")
            ),
            Self::UnknownWork(key) => write!(
                f,
                "{key} does not resolve in the staged ledger; stage the record that \
                 defines it, or correct the reference"
            ),
            Self::Untracked => write!(
                f,
                "this change touches implementation files but names no work record\nhelp: \
                 `akr change begin --primary <key>`, or `--untracked-reason \"...\"` for \
                 maintenance that changes no project intent"
            ),
            Self::Git(message) => write!(f, "{message}"),
        }
    }
}

impl From<GitError> for ChangeError {
    fn from(error: GitError) -> Self {
        Self::Git(error.to_string())
    }
}

/// Where the current transaction lives.
///
/// Inside the worktree's git directory, so it is local, disposable, invisible to AKR
/// search and context, and never committed. A transaction is scaffolding; only its commit
/// message is durable.
///
/// # Errors
/// [`ChangeError::Git`] when git cannot resolve the path.
pub fn transaction_path(repository: &Repository) -> Result<std::path::PathBuf, ChangeError> {
    Ok(repository.git_path("akr/current-change.akr")?)
}

/// Reads the open transaction, if there is one.
///
/// # Errors
/// [`ChangeError::Malformed`] when the file exists but cannot be read.
pub fn load(repository: &Repository) -> Result<Option<ChangeIntent>, ChangeError> {
    let path = transaction_path(repository)?;
    if !path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|error| ChangeError::Malformed(format!("{}: {error}", path.display())))?;
    ChangeIntent::parse(&text).map(Some)
}

/// Writes the transaction, creating the directory if it is not there.
///
/// # Errors
/// [`ChangeError::Malformed`] when the file cannot be written.
pub fn save(repository: &Repository, intent: &ChangeIntent) -> Result<(), ChangeError> {
    let path = transaction_path(repository)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| ChangeError::Malformed(format!("{}: {error}", parent.display())))?;
    }
    std::fs::write(&path, intent.render())
        .map_err(|error| ChangeError::Malformed(format!("{}: {error}", path.display())))
}

/// Discards the transaction. Never an error when there is none.
///
/// # Errors
/// [`ChangeError::Git`] when git cannot resolve the path.
pub fn discard(repository: &Repository) -> Result<bool, ChangeError> {
    let path = transaction_path(repository)?;
    if path.is_file() {
        let _ = std::fs::remove_file(&path);
        return Ok(true);
    }
    Ok(false)
}

// ---------------------------------------------------------------------------------------
// the implementation digest
// ---------------------------------------------------------------------------------------

/// Paths excluded from the implementation digest.
///
/// Evidence has to be able to name the code it verified without a hash cycle: writing the
/// digest into `.akr/` would change `.akr/`, which would change the digest. Excluding
/// AKR's own files and its generated views breaks the cycle and, better, makes the digest
/// mean the right thing — "the implementation I tested", not "the tree including the note
/// I just wrote about it".
#[must_use]
pub fn is_akr_metadata(path: &str) -> bool {
    path.starts_with(".akr/") || path.starts_with("docs/generated/") || path == ".akr"
}

/// A digest over the implementation portion of a staged tree.
///
/// Sorted `(path, mode, blob)` triples, so it is a pure function of what is staged and
/// independent of the order git happened to list it in.
#[must_use]
pub fn implementation_digest(entries: &[IndexEntry]) -> String {
    let mut selected: Vec<&IndexEntry> = entries
        .iter()
        .filter(|entry| !is_akr_metadata(&entry.path))
        .collect();
    selected.sort_by(|a, b| a.path.cmp(&b.path));
    let mut hasher = Sha256::new();
    for entry in selected {
        hasher.update(entry.path.as_bytes());
        hasher.update(b"\0");
        hasher.update(entry.mode.as_bytes());
        hasher.update(b"\0");
        hasher.update(entry.blob.as_bytes());
        hasher.update(b"\0");
    }
    format!("sha256:{}", hasher.finish().to_hex())
}

// ---------------------------------------------------------------------------------------
// the semantic delta
// ---------------------------------------------------------------------------------------

/// One record's transition between two ledgers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    /// The record.
    pub id: RevisionId,
    /// Its title, for the message body.
    pub title: String,
    /// The state it left, or `None` when the record is new.
    pub from: Option<State>,
    /// The state it is in now.
    pub to: State,
}

/// What changed between the `HEAD` ledger and the staged one, semantically.
///
/// Computed by comparing two parsed ledgers, never by reading `git diff` text.
/// A reformat, a reordering or a moved record is not a semantic change, and a textual
/// diff cannot tell the difference.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SemanticDelta {
    /// Keys that exist in the staged ledger and not at `HEAD`.
    pub added: Vec<RevisionId>,
    /// Revisions added to keys that already existed.
    pub revised: Vec<RevisionId>,
    /// Head state changes, keyed by logical key.
    pub transitions: Vec<Transition>,
    /// Evidence records this change introduces.
    pub evidence: Vec<RevisionId>,
    /// Implementation files this change touches, excluding AKR metadata and generated
    /// views. What the index *changes*, never what it merely holds.
    pub code: Vec<String>,
    /// AKR files this change touches.
    pub ledger_files: Vec<String>,
}

impl SemanticDelta {
    /// Whether anything about the ledger changed.
    #[must_use]
    pub fn touches_ledger(&self) -> bool {
        !self.added.is_empty() || !self.revised.is_empty() || !self.transitions.is_empty()
    }

    /// Work records whose state moved. The candidates for `--primary`.
    #[must_use]
    pub fn moved_work(&self) -> Vec<&Transition> {
        self.transitions.iter().collect()
    }
}

/// Compares two ledgers and reports what changed.
///
/// `changed` is the set of paths this index changes against `HEAD`, which is what `code`
/// and `ledger_files` mean: this change's contents, not the repository's inventory. The
/// whole staged index goes to `staged_akr_files` and `implementation_digest` instead,
/// which do want every tracked file.
#[must_use]
pub fn delta(base: Option<&Ledger>, staged: &Ledger, changed: &[String]) -> SemanticDelta {
    let mut out = SemanticDelta::default();

    let base_revisions: BTreeMap<String, &crate::model::Record> = base
        .iter()
        .flat_map(|ledger| ledger.records())
        .map(|record| (record.id.to_string(), record))
        .collect();
    let base_keys: BTreeMap<String, State> = base
        .iter()
        .flat_map(|ledger| ledger.records())
        .filter(|record| {
            base.is_some_and(|l| l.head(&record.id.key).is_ok_and(|h| h.id == record.id))
        })
        .map(|record| (record.id.key.to_string(), record.state))
        .collect();

    for record in staged.records() {
        let id = record.id.to_string();
        if base_revisions.contains_key(&id) {
            continue;
        }
        if base_keys.contains_key(&record.id.key.to_string()) {
            out.revised.push(record.id.clone());
        } else {
            out.added.push(record.id.clone());
        }
        if record.kind == crate::model::Kind::Evidence {
            out.evidence.push(record.id.clone());
        }
    }

    for record in staged.records() {
        let key = record.id.key.to_string();
        let is_head = staged.head(&record.id.key).is_ok_and(|h| h.id == record.id);
        if !is_head {
            continue;
        }
        let was = base_keys.get(&key).copied();
        if was != Some(record.state) {
            out.transitions.push(Transition {
                id: record.id.clone(),
                title: record.title.clone(),
                from: was,
                to: record.state,
            });
        }
    }

    for path in changed {
        if is_akr_metadata(path) {
            if path.starts_with(".akr/") {
                out.ledger_files.push(path.clone());
            }
        } else {
            out.code.push(path.clone());
        }
    }

    out.added.sort();
    out.revised.sort();
    out.transitions.sort_by(|a, b| a.id.cmp(&b.id));
    out.evidence.sort();
    out
}

/// Reads the `.akr` files of a tree as they are staged, so the delta compares ledgers.
///
/// # Errors
/// [`ChangeError::Git`] when a staged blob cannot be read.
pub fn staged_akr_files(
    repository: &Repository,
    entries: &[IndexEntry],
) -> Result<Vec<(String, String)>, ChangeError> {
    let mut out = Vec::new();
    for entry in entries {
        if entry.path.starts_with(".akr/") && entry.path.ends_with(".akr") {
            out.push((entry.path.clone(), repository.blob(&entry.blob)?));
        }
    }
    Ok(out)
}

/// The `.akr` files of a commit.
///
/// # Errors
/// [`ChangeError::Git`] when the tree cannot be listed.
pub fn akr_files_at(
    repository: &Repository,
    commit: &Commit,
) -> Result<Vec<(String, String)>, ChangeError> {
    let mut out = Vec::new();
    for path in repository.run_ls_tree(commit)? {
        if path.starts_with(".akr/")
            && path.ends_with(".akr")
            && let Some(text) = repository.file_at(commit, &path)?
        {
            out.push((path, text));
        }
    }
    Ok(out)
}

/// Whether `path` is inside the workspace root, canonically.
///
/// Not part of the bridge, but it lives beside it because the same rule governs both: a
/// tool that will act on a path an agent supplied has to prove that path is where it says
/// it is, before it reads or writes anything.
///
/// # Errors
/// [`ChangeError::Malformed`] naming the reason the path was rejected.
pub fn resolve_within(root: &Path, requested: &Path) -> Result<std::path::PathBuf, ChangeError> {
    if requested.is_absolute() {
        return Err(ChangeError::Malformed(
            "an absolute path is not accepted here; give a workspace-relative one".into(),
        ));
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|error| ChangeError::Malformed(format!("{}: {error}", root.display())))?;
    let canonical_file = root
        .join(requested)
        .canonicalize()
        .map_err(|error| ChangeError::Malformed(format!("{}: {error}", requested.display())))?;
    if !canonical_file.starts_with(&canonical_root) {
        return Err(ChangeError::Malformed(format!(
            "{} resolves outside the workspace",
            requested.display()
        )));
    }
    Ok(canonical_file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kind_is_accepted_by_its_whole_word_as_well_as_its_abbreviation() {
        for (given, expected) in [
            ("feat", ChangeKind::Feat),
            ("feature", ChangeKind::Feat),
            ("docs", ChangeKind::Docs),
            ("documentation", ChangeKind::Docs),
            ("perf", ChangeKind::Perf),
            ("performance", ChangeKind::Perf),
            ("fix", ChangeKind::Fix),
            ("bugfix", ChangeKind::Fix),
        ] {
            assert_eq!(ChangeKind::from_name(given), Some(expected), "{given}");
        }
        // The stored spelling stays the abbreviation, whichever word was typed.
        assert_eq!(
            ChangeKind::from_name("feature").expect("a kind").as_str(),
            "feat"
        );
        // Nothing is guessed: this is a fixed alias list, not a fuzzy match.
        assert_eq!(ChangeKind::from_name("feet"), None);
        assert_eq!(ChangeKind::from_name("features"), None);
    }

    fn entry(path: &str, blob: &str) -> IndexEntry {
        IndexEntry {
            path: path.to_owned(),
            mode: "100644".to_owned(),
            blob: blob.to_owned(),
        }
    }

    #[test]
    fn the_implementation_digest_ignores_akr_metadata() {
        let code = vec![entry("src/tone.rs", "aaa")];
        let with_ledger = vec![
            entry(".akr/records/sys/work.akr", "bbb"),
            entry("docs/generated/ACTIVE-WORK.md", "ccc"),
            entry("src/tone.rs", "aaa"),
        ];
        // The property that makes evidence able to name the code it tested: writing the
        // digest into the ledger cannot change the digest.
        assert_eq!(
            implementation_digest(&code),
            implementation_digest(&with_ledger)
        );
    }

    #[test]
    fn the_implementation_digest_moves_with_the_code() {
        let before = vec![entry("src/tone.rs", "aaa")];
        let after = vec![entry("src/tone.rs", "bbb")];
        assert_ne!(
            implementation_digest(&before),
            implementation_digest(&after)
        );
    }

    #[test]
    fn the_implementation_digest_is_order_independent() {
        let one = vec![entry("src/a.rs", "aaa"), entry("src/b.rs", "bbb")];
        let two = vec![entry("src/b.rs", "bbb"), entry("src/a.rs", "aaa")];
        assert_eq!(implementation_digest(&one), implementation_digest(&two));
    }

    #[test]
    fn a_transaction_round_trips() {
        let mut intent = ChangeIntent::new("ff74d3b2", ChangeKind::Fix, "gate highlight chroma");
        intent.scope = Some("tone".into());
        intent.primary_work = Some("@raw.work.slice-6/2".into());
        intent.related_work = vec!["@raw.work.slice-1/2".into(), "@raw.work.slice-4/2".into()];
        intent.implementation_note = Some("Restore the display-linear proxy.".into());
        let parsed = ChangeIntent::parse(&intent.render()).expect("round trip");
        assert_eq!(parsed, intent);
    }

    #[test]
    fn a_transaction_with_quotes_round_trips() {
        let mut intent =
            ChangeIntent::new("ff74d3b2", ChangeKind::Chore, "pin the \"windows\" image");
        intent.untracked_reason = Some("repository maintenance; no \\ behaviour changed".into());
        let parsed = ChangeIntent::parse(&intent.render()).expect("round trip");
        assert_eq!(parsed, intent);
    }

    #[test]
    fn the_id_is_stable_for_the_same_base_and_summary() {
        let one = ChangeIntent::new("ff74d3b2", ChangeKind::Fix, "same");
        let two = ChangeIntent::new("ff74d3b2", ChangeKind::Fix, "same");
        assert_eq!(one.id, two.id);
        assert_ne!(
            one.id,
            ChangeIntent::new("ff74d3b2", ChangeKind::Fix, "other").id
        );
    }

    #[test]
    fn work_refs_put_the_primary_first_and_deduplicate() {
        let mut intent = ChangeIntent::new("a", ChangeKind::Feat, "s");
        intent.primary_work = Some("@a/1".into());
        intent.related_work = vec!["@b/1".into(), "@a/1".into()];
        assert_eq!(
            intent.work_refs(),
            vec!["@a/1".to_owned(), "@b/1".to_owned()]
        );
    }

    #[test]
    fn a_path_outside_the_workspace_is_rejected() {
        let root = std::env::temp_dir();
        assert!(resolve_within(&root, Path::new("/etc/passwd")).is_err());
        assert!(resolve_within(&root, Path::new("../../etc/passwd")).is_err());
    }
}

//! Git, through the subprocess: commits, ancestry, and changed paths.
//!
//! Git is the clock. AKR has no notion of time other than commits and two dates, and
//! "is this observation still current?" is answered as "did any commit between its
//! `observed_at` and `HEAD` touch a path it watches?" — a question git answers exactly
//! (`docs/01-architecture.md` §5).
//!
//! # Subprocess, not a library
//!
//! `docs/13-implementation-roadmap.md` §4: "Git is invoked as a subprocess or through a
//! library; either is fine, and the subprocess route is one fewer build-time dependency."
//! Every call here is a plumbing command with a stable, machine-oriented output format —
//! `rev-parse`, `merge-base --is-ancestor`, `rev-list`, `log --name-status`, `status
//! --porcelain`, `cat-file` — and each is wrapped so that a failure becomes an
//! `AKR-G`-coded diagnostic rather than a panic.
//!
//! # Read-only, always
//!
//! Nothing in this module writes to the repository. The build never commits, never
//! stages, and never touches a `.akr` file (D-003, `docs/10-freshness-and-git.md` §8).
//! Every subcommand used here is a query.

use crate::diagnostics::{Diagnostic, RuleId, Subject};
use crate::model::{Commit, IdentError};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};

/// The freshness diagnostics this module raises.
///
/// Registered in `spec/diagnostics/codes-runtime.md`; the `G` range belongs to the runtime
/// half of the design set (`spec/diagnostics/README.md` §2), so they live here rather than
/// in [`crate::diagnostics::codes`], which is asserted to be language-stage only.
pub mod codes {
    use crate::diagnostics::Code;

    /// Not a git repository.
    pub const G001: Code = Code::new("AKR-G001");
    /// A git invocation failed.
    pub const G002: Code = Code::new("AKR-G002");
    /// History is shallow; ancestry is undecidable.
    pub const G003: Code = Code::new("AKR-G003");
    /// The working tree has uncommitted changes to watched paths.
    pub const G004: Code = Code::new("AKR-G004");
    /// An `observed_at` commit is not in the repository.
    pub const G011: Code = Code::new("AKR-G011");
    /// An `observed_at` commit is not an ancestor of HEAD.
    pub const G012: Code = Code::new("AKR-G012");
    /// An unknown revision argument.
    pub const G013: Code = Code::new("AKR-G013");
    /// A malformed explicit commit-rewrite map.
    pub const G014: Code = Code::new("AKR-G014");
    /// A malformed watch glob.
    pub const G021: Code = Code::new("AKR-G021");
    /// A watch glob that matches nothing.
    pub const G022: Code = Code::new("AKR-G022");
    /// A scope glob that matches nothing.
    pub const G023: Code = Code::new("AKR-G023");
    /// A recorded command names a test that exists nowhere in tracked source.
    pub const G024: Code = Code::new("AKR-G024");
    /// A recorded command's exit status cannot mean what the check rests on.
    pub const G025: Code = Code::new("AKR-G025");
    /// `project.akr` declares a tree untracked that git in fact tracks.
    pub const G026: Code = Code::new("AKR-G026");
    /// `review_after` precedes `created_at`.
    pub const G031: Code = Code::new("AKR-G031");
    /// The review queue is not empty, under `--review-clean`.
    pub const G041: Code = Code::new("AKR-G041");

    /// Every freshness code this crate can raise.
    pub const ALL: &[Code] = &[
        G001, G002, G003, G004, G011, G012, G013, G014, G021, G022, G023, G024, G025, G026, G031,
        G041,
    ];
}

/// V-101 through V-104 (`docs/10-freshness-and-git.md` §9).
pub(crate) const V101: RuleId = RuleId(101);
pub(crate) const V102: RuleId = RuleId(102);
pub(crate) const V103: RuleId = RuleId(103);
pub(crate) const V104: RuleId = RuleId(104);

// -------------------------------------------------------------------------------------
// Errors
// -------------------------------------------------------------------------------------

/// Why a git query could not be answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitError {
    /// The path is not inside a git repository (`AKR-G001`).
    NotARepository(PathBuf),
    /// A `git` subcommand failed, or `git` is not installed (`AKR-G002`).
    CommandFailed {
        /// The subcommand, for the message.
        subcommand: String,
        /// What git said.
        stderr: String,
    },
    /// The repository has a truncated history (`AKR-G003`).
    ShallowHistory,
    /// A revision argument does not name a commit (`AKR-G013`).
    UnknownRevision(String),
    /// `.akr-commit-map` does not contain valid old/replacement pairs (`AKR-G014`).
    InvalidRewriteMap {
        /// One-based source line, or zero for a file-level error.
        line: usize,
        /// What was wrong.
        message: String,
    },
    /// Git returned something this module could not read as a commit.
    MalformedOutput(String),
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotARepository(path) => {
                write!(f, "{} is not inside a git repository", path.display())
            }
            Self::CommandFailed { subcommand, stderr } => {
                write!(f, "git {subcommand} failed: {}", stderr.trim())
            }
            Self::ShallowHistory => f.write_str("repository history is shallow"),
            Self::UnknownRevision(rev) => write!(f, "{rev} is not a commit in this repository"),
            Self::InvalidRewriteMap { line, message } if *line > 0 => {
                write!(f, ".akr-commit-map line {line}: {message}")
            }
            Self::InvalidRewriteMap { message, .. } => {
                write!(f, ".akr-commit-map: {message}")
            }
            Self::MalformedOutput(text) => write!(f, "unreadable git output: {text}"),
        }
    }
}

impl std::error::Error for GitError {}

impl From<IdentError> for GitError {
    fn from(error: IdentError) -> Self {
        Self::MalformedOutput(error.to_string())
    }
}

impl GitError {
    /// The diagnostic this error renders as.
    #[must_use]
    pub fn to_diagnostic(&self) -> Diagnostic {
        let (code, rule) = match self {
            Self::NotARepository(_) => (codes::G001, V101),
            Self::ShallowHistory => (codes::G003, V101),
            Self::UnknownRevision(_) => (codes::G013, V101),
            Self::InvalidRewriteMap { .. } => (codes::G014, V101),
            Self::CommandFailed { .. } | Self::MalformedOutput(_) => (codes::G002, V101),
        };
        Diagnostic::error(code, rule, Subject::Ledger, self.to_string())
    }
}

// -------------------------------------------------------------------------------------
// Repository
// -------------------------------------------------------------------------------------

/// One path touched by one commit.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Touch {
    /// The commit that touched it.
    pub commit: Commit,
    /// The repo-root-relative path, with forward slashes.
    pub path: String,
}

/// A handle on a git repository.
///
/// Cheap to construct and cheap to clone: it holds a working directory and a memo of the
/// answers already fetched. Nothing is cached across instances, so a test that mutates a
/// repository makes a new handle and sees the new history.
#[derive(Debug)]
pub struct Repository {
    root: PathBuf,
    memo: Arc<Mutex<Memo>>,
    /// Pre-rewrite commit -> reviewed reachable replacement.
    rewrites: Arc<BTreeMap<String, Commit>>,
    /// Where answers that outlive this process are kept, when the caller has a workspace.
    facts: Option<crate::cache::FactCache>,
}

/// Read the repository-owned history-rewrite declarations. Each non-comment
/// line is `<old-commit> <replacement-commit>`. The map is deliberately
/// explicit: AKR never guesses equivalence from a matching subject or patch.
fn load_rewrites(root: &Path) -> Result<BTreeMap<String, Commit>, GitError> {
    let path = root.join(".akr-commit-map");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => {
            return Err(GitError::InvalidRewriteMap {
                line: 0,
                message: error.to_string(),
            });
        }
    };
    let mut out = BTreeMap::new();
    for (offset, raw) in text.lines().enumerate() {
        let line = raw.split_once('#').map_or(raw, |(before, _)| before).trim();
        if line.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() != 2 {
            return Err(GitError::InvalidRewriteMap {
                line: offset + 1,
                message: "expected `<old-commit> <replacement-commit>`".to_owned(),
            });
        }
        let old = Commit::new(fields[0]).map_err(|error| GitError::InvalidRewriteMap {
            line: offset + 1,
            message: format!("invalid old commit: {error}"),
        })?;
        let replacement = Commit::new(fields[1]).map_err(|error| GitError::InvalidRewriteMap {
            line: offset + 1,
            message: format!("invalid replacement commit: {error}"),
        })?;
        if old == replacement {
            return Err(GitError::InvalidRewriteMap {
                line: offset + 1,
                message: "old and replacement commits are identical".to_owned(),
            });
        }
        if let Some(previous) = out.insert(old.as_str().to_owned(), replacement.clone())
            && previous != replacement
        {
            return Err(GitError::InvalidRewriteMap {
                line: offset + 1,
                message: format!("{} already maps to {previous}", old.as_str()),
            });
        }
    }
    if let Some((old, replacement)) = out
        .iter()
        .find(|(_, replacement)| out.contains_key(replacement.as_str()))
    {
        return Err(GitError::InvalidRewriteMap {
            line: 0,
            message: format!(
                "rewrite chains are not allowed: {old} maps to {replacement}, which is also replaced"
            ),
        });
    }
    Ok(out)
}

/// The memo behind [`Repository`].
///
/// Only questions whose answer cannot change while a build runs are memoised: whether an
/// object name exists, whether one commit descends from another, and whether a path is
/// ignored. Git's object graph only grows and the ignore rules are files on disk, so a
/// second identical question inside one build has the same answer as the first. Nothing
/// about the working tree or the index is memoised, because those do change.
///
/// This matters more than it looks: freshness asks the same ancestry questions once per
/// record per watched path, and on Windows every answer that is not memoised costs a
/// process creation — hundreds of them, which is seconds of wall clock.
#[derive(Debug, Default)]
struct Memo {
    /// Object name -> does it resolve.
    exists: BTreeMap<String, bool>,
    /// (descendant, ancestor) -> is the first reachable from the second.
    ancestry: BTreeMap<(String, String), bool>,
    /// Commit -> everything reachable from it, once the walk has been worth doing.
    reachable: BTreeMap<String, BTreeSet<String>>,
    /// Commit -> how many ancestry questions have been asked about it.
    probes: BTreeMap<String, u32>,
    /// The parent graph of one commit range, once a caller has said it will ask ancestry
    /// questions inside it. See [`Repository::prime_range`].
    range: Option<RangeGraph>,
    /// Path -> its touching commits, newest first.
    touching: BTreeMap<String, Vec<Commit>>,
    /// (commit, path) -> the file's content there, or `None` when it did not exist.
    blobs: BTreeMap<(String, String), Option<String>>,
    /// (from, to) -> the paths every commit in that range touched.
    touches: BTreeMap<(String, String), Vec<Touch>>,
    /// Commit -> every path tracked at it.
    trees: BTreeMap<String, BTreeSet<String>>,
    /// Commit -> every submodule (gitlink) path tracked at it.
    submodules: BTreeMap<String, BTreeSet<String>>,
    /// A commit set -> that set in topological order.
    topological: BTreeMap<String, Vec<Commit>>,
    /// (head, commit set) -> which of them that head does not reach.
    unreachable: BTreeMap<String, BTreeSet<Commit>>,
    /// The working tree's changed paths, for the one snapshot this memo describes.
    worktree: Option<BTreeSet<String>>,
    /// Path -> is it ignored.
    ignored: BTreeMap<String, bool>,
    /// Revision argument -> the commit it names. `HEAD` is asked for once per session.
    revisions: BTreeMap<String, Commit>,
    /// `HEAD` and the newest commit naming project work, for the handoff briefing.
    session: Option<(CommitBrief, Option<CommitBrief>)>,
}

/// The commits of one range `(from, to]` and their parents, from a single `rev-list`.
#[derive(Debug, Default)]
struct RangeGraph {
    /// The range it describes, as the `rev-list` argument.
    range: String,
    /// The excluded lower bound, when there is one.
    from: Option<String>,
    /// Commit in the range -> its parents (in or out of the range).
    parents: BTreeMap<String, Vec<String>>,
    /// Commit -> its ancestors inside the range, computed on first ask.
    ancestors: BTreeMap<String, BTreeSet<String>>,
}

impl RangeGraph {
    /// Whether `ancestor` is reachable from `descendant`, when both lie in the range.
    ///
    /// Exact without leaving the range: every commit on a path from `descendant` down to
    /// `ancestor` descends from `ancestor`, so none of them is reachable from `from` —
    /// that would make `ancestor` reachable from it too — and all of them lie in the
    /// range. `from` itself answers too: everything it reaches is outside the range.
    /// `None` for any other commit outside it, which this graph cannot answer.
    fn reaches(&mut self, descendant: &str, ancestor: &str) -> Option<bool> {
        if !self.parents.contains_key(ancestor) {
            return None;
        }
        if self.from.as_deref() == Some(descendant) {
            return Some(false);
        }
        if !self.parents.contains_key(descendant) {
            return None;
        }
        if !self.ancestors.contains_key(descendant) {
            let mut seen = BTreeSet::new();
            let mut stack = vec![descendant.to_owned()];
            while let Some(commit) = stack.pop() {
                for parent in self.parents.get(&commit).into_iter().flatten() {
                    if self.parents.contains_key(parent) && seen.insert(parent.clone()) {
                        stack.push(parent.clone());
                    }
                }
            }
            self.ancestors.insert(descendant.to_owned(), seen);
        }
        Some(self.ancestors[descendant].contains(ancestor))
    }
}

/// How many `merge-base` calls about one commit are worth paying before walking its
/// history once instead.
///
/// Each `merge-base --is-ancestor` is a process; one `rev-list` is also a process but
/// answers every future question about that commit. Freshness asks one question per
/// empirical record, so any ledger past a handful of records comes out ahead. Below the
/// threshold the pairwise form stays cheaper, which is what a small ledger wants.
const REACHABILITY_PROBE_LIMIT: u32 = 3;

/// Pathspecs V-105 must not search. Ledger self-quotes, generated views, agent
/// scratch, and data dumps that quote commands from when a since-deleted test still
/// existed. Counting those as source hides a true phantom, and `git grep -o` over
/// them is what made `akr check` hang for tens of minutes on SaveYourSkin.
const SOURCE_HIT_EXCLUDES: &[&str] = &[
    ":(exclude).akr",
    ":(exclude)docs/generated",
    ":(exclude).agent",
    ":(exclude,glob)**/*.jsonl",
    ":(exclude,glob)**/*.log",
    ":(exclude,glob)**/*.csv",
    ":(exclude,glob)**/*.txt",
    ":(exclude,glob)**/*.json",
];

/// Pathspecs V-105 excludes in its own search on top of [`SOURCE_HIT_EXCLUDES`].
///
/// Captured run output and bulk data are not source either, and they are where a
/// since-deleted test's name survives: a status file or a log from the day it still
/// existed. They are also where the bytes are — a tracked 70 MB JSONL made the search run
/// for half an hour.
const SOURCE_DUMP_EXCLUDES: &[&str] = &[
    ":(exclude)*.log",
    ":(exclude)*.jsonl",
    ":(exclude)*.csv",
    ":(exclude)*.txt",
    ":(exclude)*.json",
];

impl Clone for Repository {
    /// Clones the handle, not the memo: a new handle sees the repository as it is now.
    fn clone(&self) -> Self {
        Self {
            root: self.root.clone(),
            memo: Arc::default(),
            rewrites: self.rewrites.clone(),
            facts: self.facts.clone(),
        }
    }
}

/// Memos that outlive one handle, for hosts that answer many questions in one process.
///
/// Keyed by repository root, and held only while the repository is demonstrably unchanged:
/// each [`Repository::shared`] recomputes `HEAD` and the porcelain status, and any
/// difference throws the whole memo away. That is deliberately blunt. A finer key could
/// keep more of the memo across a commit, but the thing being cached is the evidence a
/// knowledge ledger reports as fact, and "recompute it" is a cheaper mistake than "report
/// yesterday's answer".
static SHARED: Mutex<BTreeMap<PathBuf, SharedMemo>> = Mutex::new(BTreeMap::new());

#[derive(Debug)]
struct SharedMemo {
    /// `HEAD` plus the working-tree state that the memo was built against.
    key: String,
    memo: Arc<Mutex<Memo>>,
}

impl Repository {
    fn memo<T>(&self, read: impl FnOnce(&mut Memo) -> T) -> T {
        // A poisoned memo is not a reason to fail a build: the worst case is a
        // recomputed answer, and the answers are pure.
        let mut memo = self.memo.lock().unwrap_or_else(|error| error.into_inner());
        read(&mut memo)
    }

    /// Opens a repository whose memo is shared with earlier handles on the same root.
    ///
    /// For a one-shot command this is [`Repository::open`] plus two cheap queries. For a
    /// long-lived host — the MCP server answering call after call about one workspace —
    /// it is the difference between re-asking git everything per call and asking only
    /// what changed. The memo survives only while `HEAD` and the working tree are
    /// identical; any edit, stage, or commit discards it.
    ///
    /// # Errors
    /// As [`Repository::open`].
    pub fn shared(path: &Path) -> Result<Self, GitError> {
        let (probe, head) = Self::open_with_head(path)?;
        let (key, status) = probe.worktree_key(head);
        let mut registry = SHARED.lock().unwrap_or_else(|error| error.into_inner());
        let entry = registry
            .entry(probe.root.clone())
            .or_insert_with(|| SharedMemo {
                key: key.clone(),
                memo: Arc::default(),
            });
        if entry.key != key {
            entry.key = key;
            entry.memo = Arc::default();
        }
        let repository = Self {
            root: probe.root,
            memo: entry.memo.clone(),
            rewrites: probe.rewrites,
            facts: None,
        };
        drop(registry);
        // The fingerprint was read from the very status this memo describes, so seeding it
        // is not a guess: it is the answer, already paid for.
        if let Some(text) = status {
            repository.memo(|memo| {
                memo.worktree
                    .get_or_insert_with(|| Self::parse_status(&text));
            });
        }
        Ok(repository)
    }

    /// A fingerprint of everything a memoised answer could depend on: the commit being
    /// resolved against, and every difference between it and the files on disk.
    ///
    /// An unreadable answer fingerprints as unknown, which never compares equal to a
    /// previous one, so a repository git cannot describe is never served from a memo.
    ///
    /// `head` is passed in rather than asked for: [`Self::open_with_head`] already read it
    /// in the same `rev-parse` that located the root, and on a slow filesystem a redundant
    /// spawn costs more than everything this fingerprint protects.
    fn worktree_key(&self, head: Option<String>) -> (String, Option<String>) {
        let head = head.unwrap_or_else(|| "?".to_owned());
        // The NUL-separated form, because this is the same question
        // [`Self::working_tree_changes`] asks. Running it in one format to fingerprint the
        // memo and again in another to read the paths meant walking the tree twice, and on
        // this repository one `git status` costs 300ms against 19ms for everything else a
        // session opens with. The raw text is handed back so the memo it validates can be
        // seeded from it.
        match self.run(&["status", "--porcelain", "-z", "--untracked-files=all"]) {
            Ok(text) => {
                let digest = crate::hash::content_hash(&text).to_string();
                (format!("{head} {digest}"), Some(text))
            }
            Err(_) => (format!("{head} ?"), None),
        }
    }

    /// Parses `git status --porcelain -z` into the changed-path set.
    fn parse_status(text: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for entry in text.split('\0').filter(|e| e.len() > 3) {
            // `XY <path>`; rename entries carry the origin in the following NUL field,
            // which this loop sees as a short entry and skips.
            out.insert(entry[3..].trim().to_owned());
        }
        out
    }
}

/// One entry of the git index: what is staged, at which mode, as which blob.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct IndexEntry {
    /// The repository-relative path, `/`-separated on every platform.
    pub path: String,
    /// The file mode git recorded, e.g. `100644`.
    pub mode: String,
    /// The blob object id of the staged content.
    pub blob: String,
}

/// One commit as the session-head briefing needs to present it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitBrief {
    /// The commit object.
    pub commit: Commit,
    /// Its one-line subject.
    pub subject: String,
    /// `AKR-Work` trailer values, in authored order.
    pub work: Vec<String>,
}

impl Repository {
    /// Opens the repository containing `path`.
    ///
    /// # Errors
    /// [`GitError::NotARepository`] when there is no repository, and
    /// [`GitError::ShallowHistory`] when the clone is `--depth`-limited: a truncated
    /// history cannot answer the descendant question that D-016 and D-024 depend on.
    pub fn open(path: &Path) -> Result<Self, GitError> {
        Self::open_with_head(path).map(|(repository, _)| repository)
    }

    /// [`Self::open`], also returning `HEAD` when the repository has one.
    ///
    /// `git rev-parse` answers several questions per invocation, one line each. Asking the
    /// three that every session needs — the root, whether the clone is shallow, and the
    /// commit being resolved against — in one spawn rather than three is worth doing
    /// because a session pays this on every open, and the MCP server opens a session per
    /// tool call. `HEAD` is optional: a repository with no commits yet answers the first
    /// two questions and fails the third, which is not an error here.
    ///
    /// # Errors
    /// As [`Self::open`].
    fn open_with_head(path: &Path) -> Result<(Self, Option<String>), GitError> {
        let text = run_in(
            path,
            &[
                "rev-parse",
                "--show-toplevel",
                "--is-shallow-repository",
                "HEAD",
            ],
        )
        .or_else(|_| {
            run_in(
                path,
                &["rev-parse", "--show-toplevel", "--is-shallow-repository"],
            )
        })
        .map_err(|_| GitError::NotARepository(path.to_path_buf()))?;

        let mut lines = text.lines();
        let root = lines
            .next()
            .ok_or_else(|| GitError::NotARepository(path.to_path_buf()))?;
        let shallow = lines.next().unwrap_or("false").trim() == "true";
        let head = lines
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        if shallow {
            return Err(GitError::ShallowHistory);
        }
        Ok((
            Self {
                root: PathBuf::from(root.trim()),
                memo: Arc::default(),
                rewrites: Arc::new(load_rewrites(Path::new(root.trim()))?),
                facts: None,
            },
            head,
        ))
    }

    /// The reviewed post-rewrite identity of `commit`, or the commit itself.
    fn canonical_commit(&self, commit: &Commit) -> Commit {
        self.rewrites
            .get(commit.as_str())
            .cloned()
            .unwrap_or_else(|| commit.clone())
    }

    /// This handle, keeping answers that outlive the process in `cache`.
    ///
    /// Only answers that are pure functions of immutable git objects go there: the paths
    /// each commit touched, and the commits that touched a path as of one `HEAD`. Every
    /// other question is asked of git as before.
    #[must_use]
    pub fn with_fact_cache(mut self, cache: crate::cache::FactCache) -> Self {
        self.facts = Some(cache);
        self
    }

    /// The repository root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolves a revision to a commit.
    ///
    /// # Errors
    /// [`GitError::UnknownRevision`] when the revision does not name a commit.
    pub fn rev_parse(&self, revision: &str) -> Result<Commit, GitError> {
        // A revision name resolves to one commit for as long as this memo lives: the
        // shared memo is thrown away the moment `HEAD` or the working tree moves, so a
        // second answer here could not differ from the first. `HEAD` is the one every
        // session asks for, and a session per MCP tool call made that a spawn per call.
        if let Some(known) = self.memo(|memo| memo.revisions.get(revision).cloned()) {
            return Ok(known);
        }
        let text = self
            .run(&["rev-parse", "--verify", &format!("{revision}^{{commit}}")])
            .map_err(|_| GitError::UnknownRevision(revision.to_owned()))?;
        let commit = Commit::new(text.trim()).map_err(GitError::from)?;
        self.memo(|memo| memo.revisions.insert(revision.to_owned(), commit.clone()));
        Ok(commit)
    }

    /// The current `HEAD` commit.
    ///
    /// # Errors
    /// [`GitError::UnknownRevision`] in a repository with no commits.
    pub fn head(&self) -> Result<Commit, GitError> {
        self.rev_parse("HEAD")
    }

    /// HEAD and the newest reachable commit carrying an `AKR-Work` trailer.
    ///
    /// These are deliberately separate answers: a maintenance commit remains the newest
    /// Git fact while the handoff focus stays on the last commit that named project work.
    /// Only history reachable from HEAD is considered; another branch must not become the
    /// current session merely because it was authored later.
    ///
    /// # Errors
    /// [`GitError::CommandFailed`] when Git cannot read the history.
    pub fn session_head(&self) -> Result<(CommitBrief, Option<CommitBrief>), GitError> {
        // Two spawns, both pure functions of `HEAD` — which is half of what this memo is
        // keyed on. `knowledge.start` opens every handoff with this question, and it was
        // the slowest tool on the surface.
        if let Some(known) = self.memo(|memo| memo.session.clone()) {
            return Ok(known);
        }
        let latest = self.commit_brief(&["show", "-s", "--format=%H%x1f%s%x1f%B", "HEAD"])?;
        let linked_text = self.run(&[
            "log",
            "HEAD",
            "-1",
            "--extended-regexp",
            "--grep=^AKR-Work:",
            "--format=%H%x1f%s%x1f%B",
        ])?;
        let linked = if linked_text.trim().is_empty() {
            None
        } else {
            Some(self.parse_commit_brief(&linked_text)?)
        };
        self.memo(|memo| memo.session = Some((latest.clone(), linked.clone())));
        Ok((latest, linked))
    }

    fn commit_brief(&self, args: &[&str]) -> Result<CommitBrief, GitError> {
        let text = self.run(args)?;
        self.parse_commit_brief(&text)
    }

    fn parse_commit_brief(&self, text: &str) -> Result<CommitBrief, GitError> {
        let mut fields = text.splitn(3, '\u{1f}');
        let oid = fields.next().unwrap_or_default().trim();
        let subject = fields.next().unwrap_or_default().trim().to_owned();
        let body = fields.next().unwrap_or_default();
        let commit = Commit::new(oid).map_err(GitError::from)?;
        let work = body
            .lines()
            .filter_map(|line| line.trim().strip_prefix("AKR-Work:"))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        Ok(CommitBrief {
            commit,
            subject,
            work,
        })
    }

    /// Whether a commit exists in this repository.
    ///
    /// Memoised: an object name that resolves once resolves for the rest of the build.
    #[must_use]
    pub fn contains(&self, commit: &Commit) -> bool {
        if let Some(known) = self.memo(|memo| memo.exists.get(commit.as_str()).copied()) {
            return known;
        }
        let canonical = self.canonical_commit(commit);
        let exists = self.rev_parse(canonical.as_str()).is_ok();
        self.memo(|memo| memo.exists.insert(commit.as_str().to_owned(), exists));
        exists
    }

    /// Whether `descendant` is `ancestor`, or is reachable from it.
    ///
    /// This is the D-016 condition — "evidence observed at a commit that descends from the
    /// last content change" — and the D-024 reachability test, asked of git rather than
    /// reconstructed, so merges and octopus merges are handled by the tool that
    /// understands them.
    ///
    /// # Errors
    /// [`GitError::UnknownRevision`] when either commit is absent.
    pub fn is_descendant(&self, descendant: &Commit, ancestor: &Commit) -> Result<bool, GitError> {
        let canonical_descendant = self.canonical_commit(descendant);
        let canonical_ancestor = self.canonical_commit(ancestor);
        if canonical_descendant == canonical_ancestor {
            return Ok(true);
        }
        let key = (descendant.as_str().to_owned(), ancestor.as_str().to_owned());
        if let Some(known) = self.memo(|memo| memo.ancestry.get(&key).copied()) {
            return Ok(known);
        }
        if let Some(verdict) = self.memo(|memo| {
            memo.range.as_mut().and_then(|graph| {
                graph.reaches(canonical_descendant.as_str(), canonical_ancestor.as_str())
            })
        }) {
            self.memo(|memo| memo.ancestry.insert(key, verdict));
            return Ok(verdict);
        }
        // Once the same commit has been asked about often enough, walk its history once
        // and answer this and every later question about it from the walk.
        let probes = self.memo(|memo| {
            let probes = memo
                .probes
                .entry(descendant.as_str().to_owned())
                .or_insert(0);
            *probes += 1;
            *probes
        });
        if probes > REACHABILITY_PROBE_LIMIT {
            self.walk_reachable(&canonical_descendant);
        }
        if let Some(reachable) =
            self.memo(|memo| memo.reachable.get(canonical_descendant.as_str()).cloned())
        {
            // Reachable is definitive for yes. For no, the commit may be absent rather
            // than merely unreachable, and those are different answers.
            let verdict = reachable.contains(canonical_ancestor.as_str());
            if !verdict && !self.contains(ancestor) {
                return Err(GitError::UnknownRevision(ancestor.as_str().to_owned()));
            }
            self.memo(|memo| memo.ancestry.insert(key, verdict));
            return Ok(verdict);
        }
        for commit in [&canonical_descendant, &canonical_ancestor] {
            if !self.contains(commit) {
                return Err(GitError::UnknownRevision(commit.as_str().to_owned()));
            }
        }
        // `merge-base --is-ancestor A B` exits 0 when A is an ancestor of B.
        let verdict = match self.status(&[
            "merge-base",
            "--is-ancestor",
            canonical_ancestor.as_str(),
            canonical_descendant.as_str(),
        ]) {
            Ok(0) => true,
            Ok(_) => false,
            Err(error) => return Err(error),
        };
        self.memo(|memo| memo.ancestry.insert(key, verdict));
        Ok(verdict)
    }

    /// Reads the parent graph of `(from, to]` in one `rev-list`, so that every ancestry
    /// question between two commits of that range is answered without a process.
    ///
    /// Freshness asks whether each record's `observed_at` already contains each commit
    /// that touched a watched path — all of them in the range it is about to walk. Asked
    /// one `merge-base` at a time that was 155 processes on SYS. Best effort, like
    /// [`Self::prime_reachable`]: if the walk fails, the pairwise form keeps answering.
    pub fn prime_range(&self, from: Option<&Commit>, to: &Commit) {
        let to = self.canonical_commit(to);
        let from = from.map(|commit| self.canonical_commit(commit).as_str().to_owned());
        let range = match &from {
            Some(from) => format!("{from}..{}", to.as_str()),
            None => to.as_str().to_owned(),
        };
        if self.memo(|memo| {
            memo.range
                .as_ref()
                .is_some_and(|graph| graph.range == range)
        }) {
            return;
        }
        let Ok(text) = self.run(&["rev-list", "--parents", &range]) else {
            return;
        };
        let mut parents = BTreeMap::new();
        for line in text.lines() {
            let mut hashes = line.split_whitespace().map(ToOwned::to_owned);
            if let Some(commit) = hashes.next() {
                parents.insert(commit, hashes.collect());
            }
        }
        self.memo(|memo| {
            memo.range = Some(RangeGraph {
                range,
                from,
                parents,
                ancestors: BTreeMap::new(),
            });
        });
    }

    /// Walks `commit`'s history once, up front, so ancestry questions about it are free.
    ///
    /// The adaptive path below does this on its own after enough questions, but a caller
    /// that already knows it is about to ask one question per record — freshness, about
    /// `HEAD` — can skip the pairwise calls entirely by saying so.
    pub fn prime_reachable(&self, commit: &Commit) {
        self.walk_reachable(&self.canonical_commit(commit));
    }

    /// Records everything reachable from `commit`, in one `rev-list`.
    ///
    /// Best effort, and silent on failure: an absent walk only means the pairwise form
    /// keeps answering. A commit that does not resolve is left unwalked so that the
    /// caller still reports it as unknown.
    fn walk_reachable(&self, commit: &Commit) {
        if self.memo(|memo| memo.reachable.contains_key(commit.as_str())) {
            return;
        }
        let Ok(text) = self.run(&["rev-list", commit.as_str()]) else {
            return;
        };
        let reachable: BTreeSet<String> = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        self.memo(|memo| {
            // Everything the walk listed demonstrably exists.
            for oid in &reachable {
                memo.exists.insert(oid.clone(), true);
            }
            memo.reachable.insert(commit.as_str().to_owned(), reachable);
        });
    }

    /// The commits in `(from, to]`, newest first.
    ///
    /// # Errors
    /// [`GitError::CommandFailed`] if git cannot walk the range.
    pub fn commits_in(&self, from: Option<&Commit>, to: &Commit) -> Result<Vec<Commit>, GitError> {
        let from = from.map(|commit| self.canonical_commit(commit));
        let to = self.canonical_commit(to);
        let range = match &from {
            Some(from) => format!("{}..{}", from.as_str(), to.as_str()),
            None => to.as_str().to_owned(),
        };
        let text = self.run(&["rev-list", &range])?;
        text.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| Commit::new(l.trim()).map_err(GitError::from))
            .collect()
    }

    /// Every `(commit, path)` touched in `(from, to]`, sorted.
    ///
    /// A rename contributes **both** of its paths: an observation watching the old
    /// location and one watching the new location have each had their subject moved out
    /// from under them, which is exactly the change staleness exists to notice.
    ///
    /// This is the single bulk query of `docs/10-freshness-and-git.md` §3 step 3. It is
    /// issued once for a whole build rather than once per record, which is what turns
    /// O(records × history) into O(history + records × globs).
    ///
    /// # Errors
    /// [`GitError::CommandFailed`] if git cannot walk the range.
    pub fn touches_in(&self, from: Option<&Commit>, to: &Commit) -> Result<Vec<Touch>, GitError> {
        let canonical_from = from.map(|commit| self.canonical_commit(commit));
        let canonical_to = self.canonical_commit(to);
        let range = match &canonical_from {
            Some(from) => format!("{}..{}", from.as_str(), canonical_to.as_str()),
            None => canonical_to.as_str().to_owned(),
        };
        let key = (
            from.map(|c| c.as_str().to_owned()).unwrap_or_default(),
            to.as_str().to_owned(),
        );
        if let Some(known) = self.memo(|memo| memo.touches.get(&key).cloned()) {
            return Ok(known);
        }
        if let Some(touches) = self.cached_touches(from, to)? {
            self.memo(|memo| memo.touches.insert(key, touches.clone()));
            return Ok(touches);
        }
        // `-m` so a merge reports the paths it changed rather than nothing at all.
        let text = self.run(&[
            "log",
            "--name-status",
            "-M",
            "-m",
            "--format=commit %H",
            &range,
        ])?;

        let mut out = BTreeSet::new();
        let mut current: Option<Commit> = None;
        for line in text.lines() {
            if let Some(hash) = line.strip_prefix("commit ") {
                current = Commit::new(hash.trim()).ok();
                continue;
            }
            let Some(commit) = &current else { continue };
            let mut fields = line.split('\t');
            let Some(status) = fields.next() else {
                continue;
            };
            if status.trim().is_empty() {
                continue;
            }
            for path in fields.filter(|p| !p.trim().is_empty()) {
                out.insert(Touch {
                    commit: commit.clone(),
                    path: path.trim().to_owned(),
                });
            }
        }
        let touches: Vec<Touch> = out.into_iter().collect();
        self.memo(|memo| memo.touches.insert(key, touches.clone()));
        Ok(touches)
    }

    /// [`Self::touches_in`] from the fact cache, when this handle has one.
    ///
    /// What a commit touched is fixed by the commit, so the cache keeps it per commit and
    /// asks git only about commits it has not seen: the range's commits come from the same
    /// single `rev-list` the ancestry graph is read from, and the unseen ones go to one
    /// `log --no-walk --stdin` with the flags [`Self::touches_in`] uses, which prints each
    /// commit exactly as the range walk would. On SYS the range walk was a second of every
    /// command; the cached form is a file read. `None` without a cache, or when the range
    /// cannot be listed, so the caller walks it as before.
    fn cached_touches(
        &self,
        from: Option<&Commit>,
        to: &Commit,
    ) -> Result<Option<Vec<Touch>>, GitError> {
        const ENTRY: &str = "touches";
        let Some(cache) = &self.facts else {
            return Ok(None);
        };
        self.prime_range(from, to);
        let to = self.canonical_commit(to);
        let from = from.map(|commit| self.canonical_commit(commit).as_str().to_owned());
        let range = match &from {
            Some(from) => format!("{from}..{}", to.as_str()),
            None => to.as_str().to_owned(),
        };
        let Some(commits) = self.memo(|memo| {
            memo.range
                .as_ref()
                .filter(|graph| graph.range == range)
                .map(|graph| graph.parents.keys().cloned().collect::<Vec<_>>())
        }) else {
            return Ok(None);
        };
        let mut known = cache
            .read(ENTRY)
            .map(|text| Self::parse_touches(&text))
            .unwrap_or_default();
        let unseen: Vec<&String> = commits.iter().filter(|c| !known.contains_key(*c)).collect();
        if !unseen.is_empty() {
            let input: String = unseen.iter().map(|commit| format!("{commit}\n")).collect();
            let text = self.run_with_stdin(
                &[
                    "log",
                    "--no-walk=unsorted",
                    "--stdin",
                    "--name-status",
                    "-M",
                    "-m",
                    "--format=commit %H",
                ],
                &input,
            )?;
            let mut fetched = Self::parse_touches(&text);
            // A commit that touched nothing still prints its header; one that somehow did
            // not is recorded as touching nothing rather than asked about forever.
            for commit in unseen {
                fetched.entry(commit.clone()).or_default();
            }
            known.extend(fetched);
            let mut body = String::new();
            for (commit, paths) in &known {
                body.push_str(&format!("commit {commit}\n"));
                for path in paths {
                    body.push_str(&format!("\t{path}\n"));
                }
            }
            cache.write(ENTRY, &body);
        }
        let mut out = BTreeSet::new();
        for commit in &commits {
            let Ok(parsed) = Commit::new(commit) else {
                continue;
            };
            for path in known.get(commit).into_iter().flatten() {
                out.insert(Touch {
                    commit: parsed.clone(),
                    path: path.clone(),
                });
            }
        }
        Ok(Some(out.into_iter().collect()))
    }

    /// Commit -> touched paths, from `log --name-status --format=commit %H` output or from
    /// the cache entry (whose path lines are a tab and the path).
    fn parse_touches(text: &str) -> BTreeMap<String, BTreeSet<String>> {
        let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut current: Option<String> = None;
        for line in text.lines() {
            if let Some(hash) = line.strip_prefix("commit ") {
                let hash = hash.trim().to_owned();
                out.entry(hash.clone()).or_default();
                current = Some(hash);
                continue;
            }
            let Some(commit) = &current else { continue };
            let mut fields = line.split('\t');
            let Some(status) = fields.next() else {
                continue;
            };
            // Cache lines start with a tab, so their status field is empty; git's never is
            // on a path line, and a blank line has no paths at all.
            let paths: Vec<&str> = fields.filter(|p| !p.trim().is_empty()).collect();
            if status.trim().is_empty() && !line.starts_with('\t') {
                continue;
            }
            let entry = out.entry(commit.clone()).or_default();
            for path in paths {
                entry.insert(path.trim().to_owned());
            }
        }
        out
    }

    /// Paths with uncommitted changes in the working tree or the index.
    ///
    /// Freshness is computed from committed history only, so these are invisible to it.
    /// `AKR-G004` reports them so that nobody is misled by a clean queue on a dirty tree
    /// (`docs/10-freshness-and-git.md` §8).
    ///
    /// # Errors
    /// [`GitError::CommandFailed`] if git cannot read the status.
    pub fn working_tree_changes(&self) -> Result<BTreeSet<String>, GitError> {
        // The working tree is volatile, but the shared memo it lands in is discarded the
        // moment `git status` differs, so within one snapshot this is a stable answer.
        if let Some(known) = self.memo(|memo| memo.worktree.clone()) {
            return Ok(known);
        }
        let text = self.run(&["status", "--porcelain", "-z", "--untracked-files=all"])?;
        let out = Self::parse_status(&text);
        self.memo(|memo| memo.worktree = Some(out.clone()));
        Ok(out)
    }

    /// The commits that touched one path, newest first.
    ///
    /// # Errors
    /// [`GitError::CommandFailed`] if git cannot walk the history.
    pub fn commits_touching(&self, path: &str) -> Result<Vec<Commit>, GitError> {
        if let Some(known) = self.memo(|memo| memo.touching.get(path).cloned()) {
            return Ok(known);
        }
        let text = self.run(&["log", "--format=%H", "--follow", "--", path])?;
        let commits: Vec<Commit> = text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| Commit::new(l.trim()).map_err(GitError::from))
            .collect::<Result<_, _>>()?;
        self.memo(|memo| memo.touching.insert(path.to_owned(), commits.clone()));
        Ok(commits)
    }

    /// Answers [`Self::commits_touching`] for many paths at once, on parallel processes.
    ///
    /// Each answer is one `git log --follow`, which walks the whole history and cannot be
    /// batched into one invocation; the questions are independent, so they run side by
    /// side and fill the memo. Best effort: a path whose query fails is simply asked again,
    /// and reports its error, when [`Self::commits_touching`] is called for it.
    ///
    /// With a fact cache the answers are also kept against `HEAD`: `git log --follow --
    /// <path>` walks from `HEAD`, so at the same `HEAD` it says the same thing, and a
    /// repeated command asks git nothing at all.
    pub fn prime_commits_touching<'a>(&self, paths: impl IntoIterator<Item = &'a str>) {
        const ENTRY: &str = "touching";
        let requested: Vec<&str> = paths.into_iter().collect();
        let head = self
            .facts
            .as_ref()
            .and_then(|_| self.head().ok())
            .map(|head| format!("head {}", head.as_str()));
        if let (Some(cache), Some(head)) = (&self.facts, &head)
            && let Some(body) = cache.read(ENTRY)
            && let Some((first, rest)) = body.split_once('\n')
            && first == head
        {
            let mut current: Option<(String, Vec<Commit>)> = None;
            let mut cached: Vec<(String, Vec<Commit>)> = Vec::new();
            for line in rest.lines() {
                if let Some(path) = line.strip_prefix("path ") {
                    cached.extend(current.take());
                    current = Some((path.to_owned(), Vec::new()));
                } else if let (Some((_, commits)), Ok(commit)) = (&mut current, Commit::new(line)) {
                    commits.push(commit);
                }
            }
            cached.extend(current);
            self.memo(|memo| {
                for (path, commits) in cached {
                    memo.touching.entry(path).or_insert(commits);
                }
            });
        }
        let wanted: Vec<&str> = requested
            .iter()
            .copied()
            .filter(|path| self.memo(|memo| !memo.touching.contains_key(*path)))
            .collect();
        let workers = std::thread::available_parallelism()
            .map_or(1, std::num::NonZeroUsize::get)
            .min(8)
            .min(wanted.len());
        if workers >= 2 {
            let next = std::sync::atomic::AtomicUsize::new(0);
            std::thread::scope(|scope| {
                for _ in 0..workers {
                    scope.spawn(|| {
                        while let Some(path) =
                            wanted.get(next.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
                        {
                            let _ = self.commits_touching(path);
                        }
                    });
                }
            });
        }
        if let (Some(cache), Some(head)) = (&self.facts, &head)
            && !wanted.is_empty()
        {
            let mut body = format!("{head}\n");
            self.memo(|memo| {
                for (path, commits) in &memo.touching {
                    body.push_str(&format!("path {path}\n"));
                    for commit in commits {
                        body.push_str(commit.as_str());
                        body.push('\n');
                    }
                }
            });
            cache.write(ENTRY, &body);
        }
    }

    /// The contents of a path at a commit, or `None` when it did not exist there.
    ///
    /// # Errors
    /// [`GitError::CommandFailed`] only for a failure that is not "no such path".
    pub fn file_at(&self, commit: &Commit, path: &str) -> Result<Option<String>, GitError> {
        let key = (commit.as_str().to_owned(), path.to_owned());
        if let Some(known) = self.memo(|memo| memo.blobs.get(&key).cloned()) {
            return Ok(known);
        }
        let canonical = self.canonical_commit(commit);
        let content = match self.run(&["show", &format!("{}:{path}", canonical.as_str())]) {
            Ok(text) => Some(text),
            Err(GitError::CommandFailed { .. }) => None,
            Err(other) => return Err(other),
        };
        self.memo(|memo| memo.blobs.insert(key, content.clone()));
        Ok(content)
    }

    /// Reads one path at many commits through a single `cat-file --batch` process.
    fn file_versions(
        &self,
        commits: &[Commit],
        path: &str,
    ) -> Result<BTreeMap<Commit, Option<String>>, GitError> {
        // A path at a commit is immutable, so the batch only has to ask for the versions
        // the memo has not already seen — often none at all on a second call.
        let mut out = BTreeMap::new();
        let mut wanted = Vec::new();
        for commit in commits {
            let key = (commit.as_str().to_owned(), path.to_owned());
            match self.memo(|memo| memo.blobs.get(&key).cloned()) {
                Some(known) => {
                    out.insert(commit.clone(), known);
                }
                None => wanted.push(commit.clone()),
            }
        }
        if wanted.is_empty() {
            return Ok(out);
        }
        let commits: &[Commit] = &wanted;
        let input = commits
            .iter()
            .map(|commit| format!("{}:{path}\n", commit.as_str()))
            .collect::<String>();
        let bytes = self.run_bytes_with_stdin(&["cat-file", "--batch"], &input)?;
        let mut cursor = 0usize;
        // `out` already holds every version the memo answered; the fetched ones are added
        // to it. Rebinding it here instead dropped the memo hits, so a partially warm call
        // returned fewer versions than it was asked for — and the caller reads a missing
        // version as "the file did not exist at that commit".
        let mut fetched = BTreeMap::new();
        for commit in commits {
            let Some(relative_end) = bytes[cursor..].iter().position(|byte| *byte == b'\n') else {
                return Err(GitError::MalformedOutput(
                    "cat-file --batch response ended before its header".to_owned(),
                ));
            };
            let header_end = cursor + relative_end;
            let header = String::from_utf8_lossy(&bytes[cursor..header_end]);
            cursor = header_end + 1;
            if header.ends_with(" missing") {
                fetched.insert(commit.clone(), None);
                continue;
            }
            let size = header
                .split_whitespace()
                .nth(2)
                .and_then(|value| value.parse::<usize>().ok())
                .ok_or_else(|| GitError::MalformedOutput(header.into_owned()))?;
            let content_end = cursor.saturating_add(size);
            if content_end > bytes.len() {
                return Err(GitError::MalformedOutput(
                    "cat-file --batch response ended inside an object".to_owned(),
                ));
            }
            let text = String::from_utf8_lossy(&bytes[cursor..content_end]).into_owned();
            cursor = content_end;
            if bytes.get(cursor) == Some(&b'\n') {
                cursor += 1;
            }
            fetched.insert(commit.clone(), Some(text));
        }
        self.memo(|memo| {
            for (commit, content) in &fetched {
                memo.blobs.insert(
                    (commit.as_str().to_owned(), path.to_owned()),
                    content.clone(),
                );
            }
        });
        out.extend(fetched);
        Ok(out)
    }

    /// Every tracked path at a commit, sorted.
    ///
    /// Used by V-102's "matches nothing" check, which needs the tree rather than the
    /// history.
    ///
    /// # Errors
    /// [`GitError::CommandFailed`] if git cannot read the tree.
    pub fn run_ls_tree(&self, commit: &Commit) -> Result<BTreeSet<String>, GitError> {
        if let Some(known) = self.memo(|memo| memo.trees.get(commit.as_str()).cloned()) {
            return Ok(known);
        }
        let canonical = self.canonical_commit(commit);
        let text = self.run(&["ls-tree", "-r", "--name-only", canonical.as_str()])?;
        let listing: BTreeSet<String> = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        self.memo(|memo| {
            memo.trees
                .insert(commit.as_str().to_owned(), listing.clone())
        });
        Ok(listing)
    }

    /// Every submodule path at a commit, sorted.
    ///
    /// A submodule is a gitlink: `ls-tree` reports it with mode `160000` and never
    /// recurses into it, so a glob like `libbpg/**` matches nothing even though `libbpg`
    /// itself is perfectly well tracked. That is a third way a scope becomes impossible
    /// to verify, and unlike a deleted path or an ignored one it looks entirely healthy
    /// to a reader. V-102 needs to tell it apart from a genuine miss.
    ///
    /// # Errors
    /// [`GitError::CommandFailed`] if git cannot read the tree.
    pub fn run_ls_tree_submodules(&self, commit: &Commit) -> Result<BTreeSet<String>, GitError> {
        if let Some(known) = self.memo(|memo| memo.submodules.get(commit.as_str()).cloned()) {
            return Ok(known);
        }
        let canonical = self.canonical_commit(commit);
        let text = self.run(&["ls-tree", "-r", "-t", canonical.as_str()])?;
        let listing: BTreeSet<String> = text
            .lines()
            .filter_map(|line| {
                let rest = line.strip_prefix("160000 commit ")?;
                let (_, path) = rest.split_once('\t')?;
                Some(path.trim().to_owned())
            })
            .filter(|p| !p.is_empty())
            .collect();
        self.memo(|memo| {
            memo.submodules
                .insert(commit.as_str().to_owned(), listing.clone())
        });
        Ok(listing)
    }

    /// Which of `needles` appear anywhere in tracked source at `commit`.
    ///
    /// One `git grep` process in total, for the same reason [`Repository::prime_ignored`]
    /// batches: the per-needle form costs a process each and the caller asks once per
    /// token in the ledger.
    ///
    /// The needles are not handed to git. `git grep -o -F -e a -e b …` tests every
    /// pattern separately against every line and restarts after each match, so its cost
    /// grows with needles × matches. On SYS, 613 needles over 2711 records took one
    /// `akr validate` from under three minutes to eleven, where 160 of them took 48 s and
    /// 40 took 4 s. Instead git lists every identifier word once — a single pattern, one
    /// linear pass, about a second on the same tree — and the needles are looked up in
    /// that vocabulary here. A needle made only of identifier characters can only occur
    /// inside one identifier word, so "is a substring of some word" is exactly "occurs in
    /// the text". It is also more correct than `-o` was: `-o` reports one match per
    /// position, so a needle that only ever appeared inside a longer needle's match was
    /// reported absent. A needle containing anything else cannot be answered this way and
    /// is reported absent, which no caller produces.
    ///
    /// `.akr/`, `docs/generated/` and `.agent/` are excluded. The first two are the ledger
    /// quoting itself — a name found only in a record is exactly the case being looked for.
    /// `.agent/` is the same category and worse: handoff packets and captured `just check`
    /// logs contain the output of runs from back when a since-deleted test still existed,
    /// so counting them as source would silently suppress a true positive. It also churns
    /// while agents work.
    ///
    /// Data dumps (`*.jsonl`, `*.log`, `*.csv`, `*.txt`, `*.json`) are excluded for the
    /// same reason, and for cost: `git grep -o` emits every occurrence, and a JSONL
    /// handoff trace quoting a command is not the test that command names. On SaveYourSkin
    /// those files were 70 MB of the 180 MB tree the gate walked. Binary files are skipped
    /// (`-I`); their matches were always discarded, and reading them was pure cost.
    ///
    /// Needles with no ASCII letter are dropped before the lookup. The caller should not
    /// produce them; this is the backstop that keeps a lone `_` from matching every
    /// underscore in the tree.
    ///
    /// With a `cache`, each needle's answer is kept under a fingerprint of the trees the
    /// search reads (see [`Self::source_fingerprint`]), so the tree is searched again only
    /// when that source changes or a needle is new. A commit that changes only the ledger
    /// leaves the fingerprint alone.
    ///
    /// # Errors
    /// [`GitError::CommandFailed`] if git cannot search the tree.
    pub fn tracked_source_hits<'a>(
        &self,
        commit: &Commit,
        needles: impl IntoIterator<Item = &'a str>,
        cache: Option<&crate::cache::FactCache>,
    ) -> Result<BTreeSet<String>, GitError> {
        let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_';
        let needles: BTreeSet<&str> = needles
            .into_iter()
            .filter(|needle| needle.chars().any(|c| c.is_ascii_alphabetic()))
            .filter(|needle| needle.chars().all(is_word))
            .collect();
        if needles.is_empty() {
            return Ok(BTreeSet::new());
        }
        let canonical = self.canonical_commit(commit);
        const ENTRY: &str = "source-hits";
        let keyed = cache.and_then(|cache| {
            self.source_fingerprint(&canonical)
                .ok()
                .map(|fingerprint| (cache, format!("tree {fingerprint}")))
        });
        // needle -> present, for every needle answered against this fingerprint.
        let mut known: BTreeMap<String, bool> = BTreeMap::new();
        if let Some((cache, tree)) = &keyed
            && let Some(body) = cache.read(ENTRY)
            && let Some((first, rest)) = body.split_once('\n')
            && first == tree
        {
            for line in rest.lines() {
                if let Some(needle) = line.strip_prefix("hit ") {
                    known.insert(needle.to_owned(), true);
                } else if let Some(needle) = line.strip_prefix("miss ") {
                    known.insert(needle.to_owned(), false);
                }
            }
        }
        let unknown: BTreeSet<&str> = needles
            .iter()
            .copied()
            .filter(|needle| !known.contains_key(*needle))
            .collect();
        if !unknown.is_empty() {
            let hits = self.vocabulary_hits(&canonical, &unknown)?;
            for needle in unknown {
                known.insert(needle.to_owned(), hits.contains(needle));
            }
            if let Some((cache, tree)) = &keyed {
                let mut body = format!("{tree}\n");
                for (needle, present) in &known {
                    body.push_str(if *present { "hit " } else { "miss " });
                    body.push_str(needle);
                    body.push('\n');
                }
                cache.write(ENTRY, &body);
            }
        }
        Ok(needles
            .into_iter()
            .filter(|needle| known.get(*needle) == Some(&true))
            .map(ToOwned::to_owned)
            .collect())
    }

    /// Which identifier-shaped `needles` occur in tracked source at `canonical`, by one
    /// pass over its vocabulary (see [`Self::tracked_source_hits`]).
    fn vocabulary_hits(
        &self,
        canonical: &Commit,
        needles: &BTreeSet<&str>,
    ) -> Result<BTreeSet<String>, GitError> {
        let mut args: Vec<String> = vec![
            "grep".into(),
            "--no-color".into(),
            "-I".into(),
            "-h".into(),
            "-o".into(),
            "-E".into(),
            "-e".into(),
            "[A-Za-z0-9_]+".into(),
        ];
        args.push(canonical.as_str().to_owned());
        args.push("--".into());
        for exclude in SOURCE_HIT_EXCLUDES.iter().chain(SOURCE_DUMP_EXCLUDES) {
            args.push((*exclude).into());
        }
        // Streamed rather than buffered: the occurrences run to millions of lines (7.6 M on
        // SYS), the distinct words to a few hundred thousand.
        let mut vocabulary: std::collections::HashSet<Vec<u8>> = std::collections::HashSet::new();
        self.for_each_line_allowing_no_match(&args, |line| {
            if !vocabulary.contains(line) {
                vocabulary.insert(line.to_vec());
            }
        })?;
        // Most needles are whole identifiers and hit exactly; only the rest pay for a scan.
        let mut hits = BTreeSet::new();
        let mut missing = Vec::new();
        for &needle in needles {
            if vocabulary.contains(needle.as_bytes()) {
                hits.insert(needle.to_owned());
            } else {
                missing.push(needle);
            }
        }
        if !missing.is_empty() {
            for word in &vocabulary {
                missing.retain(|needle| {
                    let found = word
                        .windows(needle.len())
                        .any(|window| window == needle.as_bytes());
                    if found {
                        hits.insert((*needle).to_owned());
                    }
                    !found
                });
                if missing.is_empty() {
                    break;
                }
            }
        }
        Ok(hits)
    }

    /// A fingerprint of everything [`Self::tracked_source_hits`] searches at `canonical`.
    ///
    /// The object ids of the tree's top-level entries, except that an excluded path is
    /// left out and a directory holding one is listed entry by entry instead — so
    /// `docs/generated` and `.akr`, which change on every ledger commit, never move it.
    /// Coarse in the safe direction: a change anywhere else under a listed tree changes
    /// the fingerprint, whether or not the search would have read the changed file.
    ///
    /// # Errors
    /// [`GitError::CommandFailed`] if git cannot list the tree.
    fn source_fingerprint(&self, canonical: &Commit) -> Result<String, GitError> {
        let excluded: BTreeSet<&str> = SOURCE_HIT_EXCLUDES
            .iter()
            .filter_map(|spec| spec.strip_prefix(":(exclude)"))
            .filter(|path| !path.contains(['*', '?', '[']))
            .collect();
        let expand: BTreeSet<String> = excluded
            .iter()
            .flat_map(|path| {
                let parts: Vec<&str> = path.split('/').collect();
                (1..parts.len()).map(move |depth| parts[..depth].join("/"))
            })
            .collect();
        // The search's own rules are part of what it answers.
        let mut lines: Vec<String> = SOURCE_HIT_EXCLUDES
            .iter()
            .chain(SOURCE_DUMP_EXCLUDES)
            .map(|spec| format!("exclude {spec}"))
            .collect();
        let mut pending = vec![String::new()];
        while let Some(directory) = pending.pop() {
            let listing = if directory.is_empty() {
                self.run(&["ls-tree", canonical.as_str()])?
            } else {
                self.run(&[
                    "ls-tree",
                    canonical.as_str(),
                    "--",
                    &format!("{directory}/"),
                ])?
            };
            for line in listing.lines() {
                let Some((_, path)) = line.split_once('\t') else {
                    continue;
                };
                if excluded.contains(path) {
                    continue;
                }
                if expand.contains(path) {
                    pending.push(path.to_owned());
                    continue;
                }
                lines.push(line.to_owned());
            }
        }
        lines.sort();
        Ok(crate::hash::sha256_hex(lines.join("\n").as_bytes()))
    }

    /// Answers [`Repository::is_ignored`] for many paths in one invocation.
    ///
    /// The per-path form costs a process each, and freshness asks it once per scope glob in
    /// the ledger. This asks once for all of them and fills the memo, so the individual
    /// calls that follow are free. Best effort: if the batch fails for any reason the memo
    /// is simply left unprimed and the per-path form runs as before.
    pub fn prime_ignored<'a>(&self, paths: impl IntoIterator<Item = &'a str>) {
        let wanted: Vec<String> = paths
            .into_iter()
            .filter(|path| self.memo(|memo| !memo.ignored.contains_key(*path)))
            .map(ToOwned::to_owned)
            .collect();
        if wanted.is_empty() {
            return;
        }
        // `-z` makes both the input and the output NUL-separated, so a path is never split
        // on a character it is allowed to contain. check-ignore exits 1 when nothing
        // matched, which is an answer and not a failure.
        let Ok(output) = self.run_bytes_allowing_no_match(
            &["check-ignore", "--no-index", "-z", "--stdin"],
            &wanted.join("\0"),
        ) else {
            return;
        };
        let text = String::from_utf8_lossy(&output);
        let ignored: BTreeSet<&str> = text.split('\0').filter(|path| !path.is_empty()).collect();
        self.memo(|memo| {
            for path in wanted {
                let verdict = ignored.contains(path.as_str());
                memo.ignored.insert(path, verdict);
            }
        });
    }

    /// Whether a repository-relative path or glob is intentionally ignored.
    ///
    /// `--no-index` also answers for paths that do not currently exist, which is needed
    /// when a scope deliberately governs a disposable cache or generated target.
    ///
    /// # Errors
    /// [`GitError::CommandFailed`] when git cannot execute the query.
    pub fn is_ignored(&self, path: &str) -> Result<bool, GitError> {
        if let Some(known) = self.memo(|memo| memo.ignored.get(path).copied()) {
            return Ok(known);
        }
        let ignored = match self.status(&["check-ignore", "--quiet", "--no-index", "--", path])? {
            0 => true,
            1 => false,
            code => {
                return Err(GitError::CommandFailed {
                    subcommand: "check-ignore --quiet --no-index".to_owned(),
                    stderr: format!("exited with status {code}"),
                });
            }
        };
        self.memo(|memo| memo.ignored.insert(path.to_owned(), ignored));
        Ok(ignored)
    }

    /// Every entry of the git index, in path order.
    ///
    /// The *index*, deliberately, and not the working tree. `docs/16-change-protocol.md`
    /// §3: the staged tree is the synchronisation boundary, because a working tree with
    /// eighteen modified files does not say which of them belong to the change being made.
    ///
    /// # Errors
    /// [`GitError::CommandFailed`] when git cannot read the index.
    pub fn staged_entries(&self) -> Result<Vec<IndexEntry>, GitError> {
        let text = self.run(&["ls-files", "--stage"])?;
        let mut out = Vec::new();
        for line in text.lines() {
            // `<mode> <object> <stage>\t<path>`
            let Some((meta, path)) = line.split_once('\t') else {
                continue;
            };
            let fields: Vec<&str> = meta.split_whitespace().collect();
            if fields.len() < 3 {
                continue;
            }
            out.push(IndexEntry {
                mode: fields[0].to_owned(),
                blob: fields[1].to_owned(),
                path: path.replace('\\', "/"),
            });
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(out)
    }

    /// The contents of a blob.
    ///
    /// # Errors
    /// [`GitError::CommandFailed`] when the object is missing or unreadable.
    pub fn blob(&self, oid: &str) -> Result<String, GitError> {
        self.run(&["cat-file", "blob", oid])
    }

    /// Writes the current index out as a tree and returns its object id.
    ///
    /// Cheap, and it changes neither `HEAD` nor the index — `write-tree` only
    /// materialises what is already staged. It is how a prepared transaction notices that
    /// the staged tree moved underneath it.
    ///
    /// # Errors
    /// [`GitError::CommandFailed`] when the index cannot be written as a tree.
    pub fn write_tree(&self) -> Result<String, GitError> {
        Ok(self.run(&["write-tree"])?.trim().to_owned())
    }

    /// A path inside the git directory of *this worktree*, resolved by git.
    ///
    /// `--git-path` rather than `.git/`: in a linked worktree those are different
    /// directories, and a change transaction is per worktree by construction.
    ///
    /// # Errors
    /// [`GitError::CommandFailed`] when git cannot resolve the path.
    pub fn git_path(&self, relative: &str) -> Result<PathBuf, GitError> {
        let text = self.run(&["rev-parse", "--git-path", relative])?;
        let raw = PathBuf::from(text.trim());
        Ok(if raw.is_absolute() {
            raw
        } else {
            self.root.join(raw)
        })
    }

    /// Whether the index differs from `HEAD` at all.
    ///
    /// # Errors
    /// [`GitError::CommandFailed`] when git cannot compare them.
    pub fn has_staged_changes(&self) -> Result<bool, GitError> {
        Ok(self.status(&["diff", "--cached", "--quiet"])? != 0)
    }

    /// The paths this index changes against `HEAD`.
    ///
    /// A different question from [`Self::staged_entries`], which lists the whole index.
    /// "What is tracked" and "what is this commit changing" differ by the entire
    /// repository, and reading the first as the second made a ledger-only commit report
    /// 388 implementation files and then refuse itself for touching them
    /// (`akr.papercut.akr-change-prepare-staged-reports-code-n-files`). An unborn `HEAD`
    /// has no tree to diff against, so there every tracked path is a change.
    ///
    /// `-z` because git quotes a path with a space or a non-ASCII byte otherwise, and a
    /// quoted path matches nothing.
    ///
    /// # Errors
    /// [`GitError::CommandFailed`] when the diff fails.
    pub fn staged_changes(&self) -> Result<Vec<String>, GitError> {
        if self.head().is_err() {
            return Ok(self
                .staged_entries()?
                .into_iter()
                .map(|entry| entry.path)
                .collect());
        }
        let text = self.run(&["diff", "--cached", "--name-only", "-z"])?;
        let mut out: Vec<String> = text
            .split('\0')
            .filter(|path| !path.is_empty())
            .map(|path| path.replace('\\', "/"))
            .collect();
        out.sort();
        out.dedup();
        Ok(out)
    }

    /// Commits the index with `message`, returning the new commit.
    ///
    /// Git is asked to make the commit rather than reimplemented, which is the whole
    /// shape of the bridge (`docs/16-change-protocol.md` §1): AKR leads the intent, git
    /// seals the snapshot.
    ///
    /// The commit moves `HEAD` and empties the index, so the entries this handle
    /// memoized for the old snapshot are dropped before `HEAD` is re-read. Without
    /// that the shared memo — which is only discarded when the *next* handle opens —
    /// answered with the previous commit, and `akr git commit` printed the old hash
    /// beside the new subject. Entries keyed by commit id stay valid: history only
    /// grows.
    ///
    /// # Errors
    /// [`GitError::CommandFailed`] when the commit is refused — an empty index, a failing
    /// hook, or an unconfigured identity.
    pub fn commit(&self, message: &str) -> Result<Commit, GitError> {
        self.run(&["commit", "-m", message])?;
        self.memo(|memo| {
            memo.revisions.remove("HEAD");
            memo.session = None;
            memo.worktree = None;
        });
        self.head()
    }

    /// Commits reachable from `range` whose message carries `trailer`.
    ///
    /// # Errors
    /// [`GitError::CommandFailed`] when the range does not resolve.
    pub fn log_grep(&self, range: &str, trailer: &str) -> Result<Vec<(Commit, String)>, GitError> {
        let text = self.run(&[
            "log",
            range,
            &format!("--grep={trailer}"),
            "--format=%H\t%s",
        ])?;
        let mut out = Vec::new();
        for line in text.lines() {
            let Some((oid, subject)) = line.split_once('\t') else {
                continue;
            };
            if let Ok(commit) = Commit::new(oid.trim()) {
                out.push((commit, subject.to_owned()));
            }
        }
        Ok(out)
    }

    /// Which of `commits` the repository actually has, in one call.
    ///
    /// One `cat-file --batch-check` instead of one `rev-parse` per commit. On a ledger
    /// with a few hundred evidence citations the difference is a few hundred process
    /// spawns, which is most of what made `akr check` take two minutes on a real
    /// repository (`saveyourskin.papercut.akr-check-akr-build-akr-lock-update-each-took`).
    ///
    /// # Errors
    /// [`GitError::CommandFailed`] when git cannot be started. A commit git does not
    /// recognise is absent from the result, not an error: that is the question being asked.
    pub fn contains_all(&self, commits: &[Commit]) -> Result<BTreeSet<Commit>, GitError> {
        if commits.is_empty() {
            return Ok(BTreeSet::new());
        }
        // Whether an object name resolves is exactly what `memo.exists` records, and git's
        // object graph only grows, so a commit known to be present stays present. Asking
        // only about the names this memo has not seen turns a repeated call — one per
        // session, and a session per MCP tool call — into no spawn at all.
        let mut out = BTreeSet::new();
        let mut wanted = Vec::new();
        for commit in commits {
            match self.memo(|memo| memo.exists.get(commit.as_str()).copied()) {
                Some(true) => {
                    out.insert(commit.clone());
                }
                Some(false) => {}
                None => wanted.push(commit.clone()),
            }
        }
        if wanted.is_empty() {
            return Ok(out);
        }
        let canonical: Vec<Commit> = wanted
            .iter()
            .map(|commit| self.canonical_commit(commit))
            .collect();
        let mut input = String::new();
        for commit in &canonical {
            input.push_str(commit.as_str());
            input.push('\n');
        }
        let text = self.run_with_stdin(
            &["cat-file", "--batch-check=%(objectname) %(objecttype)"],
            &input,
        )?;
        let mut present = BTreeSet::new();
        for line in text.lines() {
            let mut fields = line.split_whitespace();
            let (Some(oid), Some(kind)) = (fields.next(), fields.next()) else {
                continue;
            };
            if kind == "commit"
                && let Ok(commit) = Commit::new(oid)
            {
                present.insert(commit);
            }
        }
        self.memo(|memo| {
            for (commit, canonical) in wanted.iter().zip(&canonical) {
                let exists = present.contains(canonical);
                memo.exists.insert(commit.as_str().to_owned(), exists);
                if exists {
                    out.insert(commit.clone());
                }
            }
        });
        Ok(out)
    }

    /// `commits` in topological order, newest first, from one history walk.
    ///
    /// This replaces a comparison sort whose comparator spawned `git merge-base
    /// --is-ancestor`: O(n log n) processes to answer a question git will answer once.
    /// `--topo-order` guarantees that no commit is listed before one that descends from
    /// it, which is exactly the order the ancestry table needs.
    ///
    /// Commits git does not place are omitted; the caller falls back to a deterministic
    /// tiebreak for those rather than inventing an ancestry.
    ///
    /// # Errors
    /// [`GitError::CommandFailed`] when the walk fails.
    pub fn topological_order(&self, commits: &[Commit]) -> Result<Vec<Commit>, GitError> {
        if commits.is_empty() {
            return Ok(Vec::new());
        }
        let key = commits
            .iter()
            .map(Commit::as_str)
            .collect::<Vec<_>>()
            .join(" ");
        if let Some(known) = self.memo(|memo| memo.topological.get(&key).cloned()) {
            return Ok(known);
        }
        let mut originals_by_canonical: BTreeMap<String, Vec<Commit>> = BTreeMap::new();
        for commit in commits {
            originals_by_canonical
                .entry(self.canonical_commit(commit).as_str().to_owned())
                .or_default()
                .push(commit.clone());
        }
        for originals in originals_by_canonical.values_mut() {
            originals.sort();
            originals.dedup();
        }
        let mut args: Vec<String> = vec!["rev-list".to_owned(), "--topo-order".to_owned()];
        args.extend(originals_by_canonical.keys().cloned());
        let text = self.run(&args)?;
        let mut ordered = Vec::with_capacity(commits.len());
        for line in text.lines().map(str::trim) {
            if let Some(originals) = originals_by_canonical.get(line) {
                ordered.extend(originals.iter().cloned());
            }
        }
        self.memo(|memo| memo.topological.insert(key, ordered.clone()));
        Ok(ordered)
    }

    /// Which of `commits` are **not** reachable from `head`.
    ///
    /// One walk with `--not <head>`, filtered back to the commits asked about, and free in
    /// the ordinary case: when every commit is an ancestor, git prints nothing.
    ///
    /// A rewritten history — a rebase, a squashed re-upload — leaves records pointing at
    /// commits that still exist and sit on no branch this one contains. That is a
    /// different fault from evidence being old, with a different repair, and V-020 says
    /// which of the two it is looking at (`AKR-G012`).
    ///
    /// # Errors
    /// [`GitError::CommandFailed`] when the walk fails.
    pub fn unreachable_from(
        &self,
        head: &Commit,
        commits: &[Commit],
    ) -> Result<BTreeSet<Commit>, GitError> {
        if commits.is_empty() {
            return Ok(BTreeSet::new());
        }
        let key = format!(
            "{}|{}",
            head.as_str(),
            commits
                .iter()
                .map(Commit::as_str)
                .collect::<Vec<_>>()
                .join(" ")
        );
        if let Some(known) = self.memo(|memo| memo.unreachable.get(&key).cloned()) {
            return Ok(known);
        }
        let canonical_head = self.canonical_commit(head);
        let canonical: Vec<Commit> = commits
            .iter()
            .map(|commit| self.canonical_commit(commit))
            .collect();
        let mut args: Vec<String> = vec!["rev-list".to_owned()];
        args.extend(canonical.iter().map(|c| c.as_str().to_owned()));
        args.push("--not".to_owned());
        args.push(canonical_head.as_str().to_owned());
        let text = self.run(&args)?;
        let unreachable: BTreeSet<&str> = text.lines().map(str::trim).collect();
        let out: BTreeSet<Commit> = commits
            .iter()
            .zip(&canonical)
            .filter(|(_, canonical)| unreachable.contains(canonical.as_str()))
            .map(|(original, _)| original.clone())
            .collect();
        self.memo(|memo| memo.unreachable.insert(key, out.clone()));
        Ok(out)
    }

    // -- process plumbing -------------------------------------------------------------

    fn run<S: AsRef<OsStr>>(&self, args: &[S]) -> Result<String, GitError> {
        run_in(&self.root, args)
    }

    fn run_with_stdin<S: AsRef<OsStr>>(&self, args: &[S], input: &str) -> Result<String, GitError> {
        use std::io::Write as _;
        use std::process::Stdio;
        let mut child = command()
            .args(args)
            .current_dir(&self.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| GitError::CommandFailed {
                subcommand: describe(args),
                stderr: e.to_string(),
            })?;
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all(input.as_bytes());
        }
        drop(child.stdin.take());
        let output = child
            .wait_with_output()
            .map_err(|e| GitError::CommandFailed {
                subcommand: describe(args),
                stderr: e.to_string(),
            })?;
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn run_bytes_with_stdin<S: AsRef<OsStr>>(
        &self,
        args: &[S],
        input: &str,
    ) -> Result<Vec<u8>, GitError> {
        use std::io::Write as _;
        use std::process::Stdio;
        let mut child = command()
            .args(args)
            .current_dir(&self.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| GitError::CommandFailed {
                subcommand: describe(args),
                stderr: e.to_string(),
            })?;
        if let Some(stdin) = child.stdin.as_mut() {
            stdin
                .write_all(input.as_bytes())
                .map_err(|e| GitError::CommandFailed {
                    subcommand: describe(args),
                    stderr: e.to_string(),
                })?;
        }
        drop(child.stdin.take());
        let output = child
            .wait_with_output()
            .map_err(|e| GitError::CommandFailed {
                subcommand: describe(args),
                stderr: e.to_string(),
            })?;
        if !output.status.success() {
            return Err(GitError::CommandFailed {
                subcommand: describe(args),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        Ok(output.stdout)
    }

    /// Like [`Self::run_bytes_with_stdin`], but treats exit 1 as an empty answer.
    ///
    /// `check-ignore` uses exit 1 for "nothing matched", which is a verdict rather than a
    /// failure. Anything above 1 is still an error.
    fn run_bytes_allowing_no_match<S: AsRef<OsStr>>(
        &self,
        args: &[S],
        input: &str,
    ) -> Result<Vec<u8>, GitError> {
        use std::io::Write as _;
        use std::process::Stdio;
        let mut child = command()
            .args(args)
            .current_dir(&self.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| GitError::CommandFailed {
                subcommand: describe(args),
                stderr: e.to_string(),
            })?;
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all(input.as_bytes());
        }
        drop(child.stdin.take());
        let output = child
            .wait_with_output()
            .map_err(|e| GitError::CommandFailed {
                subcommand: describe(args),
                stderr: e.to_string(),
            })?;
        match output.status.code() {
            Some(0 | 1) => Ok(output.stdout),
            _ => Err(GitError::CommandFailed {
                subcommand: describe(args),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            }),
        }
    }

    /// Like [`Self::run_bytes_allowing_no_match`], but hands stdout to `each` one line at a
    /// time instead of holding all of it, for output far larger than what is kept of it.
    ///
    /// Lines are passed without their terminator. Stderr is drained on its own thread so a
    /// chatty failure cannot fill its pipe and stall the stdout reader.
    fn for_each_line_allowing_no_match<S: AsRef<OsStr>>(
        &self,
        args: &[S],
        mut each: impl FnMut(&[u8]),
    ) -> Result<(), GitError> {
        use std::io::{BufRead as _, Read as _};
        use std::process::Stdio;
        let failed = |e: std::io::Error| GitError::CommandFailed {
            subcommand: describe(args),
            stderr: e.to_string(),
        };
        let mut child = command()
            .args(args)
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(failed)?;
        let mut stderr = child.stderr.take().expect("stderr is piped");
        let drain = std::thread::spawn(move || {
            let mut text = Vec::new();
            let _ = stderr.read_to_end(&mut text);
            text
        });
        let mut stdout = std::io::BufReader::new(child.stdout.take().expect("stdout is piped"));
        let mut line = Vec::new();
        loop {
            line.clear();
            if stdout.read_until(b'\n', &mut line).map_err(failed)? == 0 {
                break;
            }
            let content = line.strip_suffix(b"\n").unwrap_or(&line);
            each(content.strip_suffix(b"\r").unwrap_or(content));
        }
        let status = child.wait().map_err(failed)?;
        let stderr = drain.join().unwrap_or_default();
        match status.code() {
            Some(0 | 1) => Ok(()),
            _ => Err(GitError::CommandFailed {
                subcommand: describe(args),
                stderr: String::from_utf8_lossy(&stderr).into_owned(),
            }),
        }
    }

    fn status<S: AsRef<OsStr>>(&self, args: &[S]) -> Result<i32, GitError> {
        let output = command()
            .args(args)
            .current_dir(&self.root)
            .output()
            .map_err(|e| GitError::CommandFailed {
                subcommand: describe(args),
                stderr: e.to_string(),
            })?;
        Ok(output.status.code().unwrap_or(-1))
    }
}

/// The `git` executable, resolved through `PATH` exactly once per process.
///
/// [`Command::new`] with a bare program name re-walks every `PATH` entry on every spawn,
/// and a build asks git tens of times. On a long `PATH` — 38 entries is ordinary — that
/// is tens of failed `execve` per spawn, each one a filesystem lookup; over a network or
/// FUSE mount it measured as more than half the cost of a trivial `git rev-parse HEAD`.
/// The answer cannot change while the process runs, so it is found once and kept.
///
/// Falls back to the bare name when `PATH` holds no `git`, so that a missing git still
/// fails where it did before, with the same error.
fn program() -> &'static OsStr {
    static PROGRAM: OnceLock<PathBuf> = OnceLock::new();
    PROGRAM
        .get_or_init(|| {
            let Some(path) = std::env::var_os("PATH") else {
                return PathBuf::from("git");
            };
            let name = if cfg!(windows) { "git.exe" } else { "git" };
            std::env::split_paths(&path)
                .map(|dir| dir.join(name))
                .find(|candidate| candidate.is_file())
                .unwrap_or_else(|| PathBuf::from("git"))
        })
        .as_os_str()
}

/// A `git` command, configured so it never steals the foreground.
///
/// Every git invocation in the workspace goes through here. On Windows a child
/// console application spawned from a process without a console — the desktop
/// review shell is a `windows_subsystem = "windows"` binary — allocates one of
/// its own, which flashes a console window on screen and takes focus away from
/// whatever the user was typing into. `CREATE_NO_WINDOW` suppresses that; it
/// changes nothing about the command's output, which is captured through pipes
/// either way. On other platforms this is a plain [`Command`] over [`program`].
pub fn command() -> Command {
    let command = Command::new(program());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        /// `CREATE_NO_WINDOW` from the Win32 process-creation flags.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut command = command;
        command.creation_flags(CREATE_NO_WINDOW);
        command
    }
    #[cfg(not(windows))]
    command
}

fn describe<S: AsRef<OsStr>>(args: &[S]) -> String {
    args.iter()
        .map(|a| a.as_ref().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

fn run_in<S: AsRef<OsStr>>(dir: &Path, args: &[S]) -> Result<String, GitError> {
    let output = command()
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| GitError::CommandFailed {
            subcommand: describe(args),
            stderr: e.to_string(),
        })?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(GitError::CommandFailed {
            subcommand: describe(args),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

// -------------------------------------------------------------------------------------
// Ancestry for LedgerFacts
// -------------------------------------------------------------------------------------

/// Builds the [`Ancestry`](crate::model::Ancestry) that V-020 reads, over the commits a
/// ledger actually references.
///
/// The commits are sorted oldest-first by asking git, then chained parent-to-child, so
/// that `is_descendant` over any pair of them returns what git would have returned.
///
/// # Precision
///
/// [`Ancestry`](crate::model::Ancestry) holds one parent per commit, so it represents a
/// chain rather than a DAG. That is exact whenever the referenced commits are totally
/// ordered by ancestry — linear history, and any history where the commits AKR cites sit
/// on one line of development, which is the normal case for `observed_at` values. Where
/// two referenced commits are on genuinely divergent branches, the chain reports the pair
/// as unrelated, which is the conservative answer: an acceptance check reads as *not*
/// satisfied rather than falsely satisfied.
///
/// Prefer [`Repository::is_descendant`] wherever a repository is in hand; this exists to
/// fill the facts for rules that have only the ledger.
///
/// # Errors
/// Propagates any git failure.
pub fn ancestry_over(
    repository: &Repository,
    commits: impl IntoIterator<Item = Commit>,
) -> Result<crate::model::Ancestry, GitError> {
    let distinct: Vec<Commit> = commits
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    // One `cat-file --batch-check` rather than one `rev-parse` each.
    let present = repository.contains_all(&distinct)?;
    let known: Vec<Commit> = distinct
        .into_iter()
        .filter(|commit| present.contains(commit))
        .collect();

    // One history walk rather than a comparison sort whose comparator forks git.
    //
    // The old shape was O(n log n) `merge-base --is-ancestor` processes, and on a ledger
    // with a few hundred evidence citations over a few hundred commits that dominated
    // `akr check`, `akr build` and `akr lock --update` alike — two minutes each, reported
    // from a real port session. `rev-list --topo-order` answers the whole question once.
    let mut ordered = repository.topological_order(&known)?;
    ordered.reverse(); // oldest first: `a` before `b` when `b` descends from `a`.

    // Anything git would not place — it should place everything, but a corrupt or grafted
    // history is not this function's problem to diagnose — keeps a deterministic position
    // rather than an invented ancestry.
    if ordered.len() != known.len() {
        let placed: BTreeSet<&Commit> = ordered.iter().collect();
        let mut missing: Vec<Commit> = known
            .iter()
            .filter(|commit| !placed.contains(commit))
            .cloned()
            .collect();
        missing.sort();
        missing.extend(ordered);
        ordered = missing;
    }

    let pairs: Vec<(Commit, Commit)> = ordered
        .windows(2)
        .map(|pair| (pair[1].clone(), pair[0].clone()))
        .collect();
    Ok(crate::model::Ancestry::from_pairs(pairs))
}

/// The last commit at which a record's canonical text reached its current value.
///
/// This is the `last_change` of D-016 — the commit an acceptance check's evidence must
/// descend from. It is computed over the record, not over its file: adding an unrelated
/// record to the same file must not invalidate every check in it.
///
/// Walks the commits that touched the record's file, newest first, hashing the record at
/// each; the answer is the oldest commit whose hash still equals the current one. A
/// commit where the record did not yet exist ends the walk, so a record's introduction is
/// its own last change.
///
/// Returns `None` when the record is not in the file at `HEAD`, or when the file is not
/// tracked — an uncommitted record has no last change, and inventing one would make an
/// acceptance verdict out of nothing.
///
/// # Errors
/// Propagates any git failure.
pub fn last_change_of(
    repository: &Repository,
    path: &str,
    key: &crate::model::LogicalKey,
    revision: u32,
) -> Result<Option<Commit>, GitError> {
    let commits = repository.commits_touching(path)?;
    let Some(newest) = commits.first() else {
        return Ok(None);
    };
    let Some(current) = hash_at(repository, newest, path, key, revision)? else {
        return Ok(None);
    };

    let mut answer = newest.clone();
    for commit in commits.iter().skip(1) {
        // At or below the revision, so that the walk crosses the commit that introduced
        // it into the history of the revision it was revised from, whenever the two have
        // the same definition (D-038).
        match definition_at_or_below_in(repository, commit, path, key, revision)? {
            Some(hash) if hash == current => answer = commit.clone(),
            _ => break,
        }
    }
    Ok(Some(answer))
}

/// The definitional hash of the newest revision of `key` at or below `revision`, at one
/// commit.
fn definition_at_or_below_in(
    repository: &Repository,
    commit: &Commit,
    path: &str,
    key: &crate::model::LogicalKey,
    revision: u32,
) -> Result<Option<crate::model::ContentHash>, GitError> {
    for candidate in (1..=revision).rev() {
        if let Some(hash) = hash_at(repository, commit, path, key, candidate)? {
            return Ok(Some(hash));
        }
    }
    Ok(None)
}

/// The *definitional* content hash of one record as it stood at one commit (D-029, D-038):
/// the canonical text with lifecycle and completion bookkeeping (`state`, `note`, a check's
/// `verified_by`) removed, along with the revision number and the `supersedes` edge to the
/// revision it replaced, so `last_change_of` reports the last redefinition rather than the
/// last lifecycle transition or the last time somebody revised the record.
fn hash_at(
    repository: &Repository,
    commit: &Commit,
    path: &str,
    key: &crate::model::LogicalKey,
    revision: u32,
) -> Result<Option<crate::model::ContentHash>, GitError> {
    let Some(text) = repository.file_at(commit, path)? else {
        return Ok(None);
    };
    let parsed = crate::syntax::parse(&text, crate::diagnostics::FileId(0));
    let Some(file) = parsed.file else {
        return Ok(None);
    };
    for (at, item) in file.items.iter().enumerate() {
        let crate::syntax::cst::Item::Record(record) = item else {
            continue;
        };
        if record.revision != revision || record.key != key.to_string() {
            continue;
        }
        // D-029: `last_change` tracks the last *definitional* change, so a completion or
        // abandonment (state, a check's `verified_by`, or a `note`) does not count as the
        // record's newest content change and strand the evidence that closes it. D-038
        // extends that across the revision boundary by dropping the revision number and
        // the `supersedes` edge back to the revision this one replaced, which are how a
        // revision is made rather than what it says. The D-015 seal still hashes the whole
        // record via `canonical_record_text`.
        if let Some(definitional) =
            crate::resolve::revision_independent_definitional_text(&file, at)
        {
            return Ok(Some(crate::hash::content_hash(&definitional)));
        }
    }
    Ok(None)
}

/// Fills `last_change` for every revision of a ledger whose file is tracked.
///
/// # Errors
/// Propagates any git failure.
pub fn last_changes(
    repository: &Repository,
    ledger: &crate::model::Ledger,
) -> Result<BTreeMap<crate::model::RevisionId, Commit>, GitError> {
    // Records are conventionally grouped into a handful of ledger files. Walking the
    // same file history once per record made session startup O(records × file history)
    // and spawned hundreds of git processes in medium-sized projects. Read and parse
    // each historical file version once, then advance every record in that file together.
    let mut by_file: BTreeMap<String, BTreeSet<crate::model::RevisionId>> = BTreeMap::new();
    for record in crate::graph::sorted_records(ledger) {
        let Some(path) = &record.file else { continue };
        by_file
            .entry(path.clone())
            .or_default()
            .insert(record.id.clone());
    }

    let mut out = BTreeMap::new();
    for (path, wanted) in by_file {
        let commits = repository.commits_touching(&path)?;
        let Some(newest) = commits.first() else {
            continue;
        };
        let versions = repository.file_versions(&commits, &path)?;
        let current = versions
            .get(newest)
            .and_then(Option::as_deref)
            .map_or_else(BTreeMap::new, |text| definitional_hashes(text, &wanted));
        let mut unresolved: BTreeSet<crate::model::RevisionId> = current.keys().cloned().collect();
        let mut answers: BTreeMap<crate::model::RevisionId, Commit> = unresolved
            .iter()
            .cloned()
            .map(|id| (id, newest.clone()))
            .collect();

        for commit in commits.iter().skip(1) {
            if unresolved.is_empty() {
                break;
            }
            let older = versions
                .get(commit)
                .and_then(Option::as_deref)
                .map_or_else(BTreeMap::new, |text| definitional_hashes(text, &unresolved));
            unresolved.retain(|id| {
                // D-038: at or below, so the walk crosses the commit that introduced this
                // revision into the identical definition it was revised from.
                if definition_at_or_below(&older, id).as_ref() == current.get(id) {
                    answers.insert(id.clone(), commit.clone());
                    true
                } else {
                    false
                }
            });
        }
        out.extend(answers);
    }
    Ok(out)
}

/// [`last_changes`], answered from a per-file timeline memoised in the fact cache.
///
/// The walk in [`last_changes`] reads and parses every historical version of every
/// ledger file on every invocation — 315 MB on SYS, seven seconds of each command, and
/// growing with file size times history. Its answer is a function of the commits that
/// touched the file and the bytes the file held at each, both immutable, so this keeps
/// what the walk needs from them: per logical key, the indices in the file's history at
/// which that key's revision -> definitional-hash map changes. Usually one or two
/// entries per key, against one parsed copy per commit.
///
/// A file whose history is unchanged is answered without fetching or parsing anything.
/// One that gained commits on top of the cached history fetches and parses only those.
/// Any other change — a rebase, a rewritten history — rebuilds that file's timeline in
/// full. The answers are identical to [`last_changes`] in every case; the cache changes
/// the cost, never the output.
///
/// # Errors
/// Propagates any git failure that makes the question unanswerable.
pub fn last_changes_cached(
    repository: &Repository,
    ledger: &crate::model::Ledger,
    cache: &crate::cache::FactCache,
) -> Result<BTreeMap<crate::model::RevisionId, Commit>, GitError> {
    let mut by_file: BTreeMap<String, BTreeSet<&crate::model::LogicalKey>> = BTreeMap::new();
    for record in crate::graph::sorted_records(ledger) {
        let Some(path) = &record.file else { continue };
        by_file
            .entry(path.clone())
            .or_default()
            .insert(&record.id.key);
    }
    // One `log --follow` per file is unavoidable — it is what says whether the cached
    // history still holds — but the files are independent, so ask about them together.
    repository.prime_commits_touching(by_file.keys().map(String::as_str));

    let mut out = BTreeMap::new();
    for (path, keys) in by_file {
        let commits = repository.commits_touching(&path)?;
        if commits.is_empty() {
            continue;
        }
        let name = format!(
            "history-{}",
            &crate::hash::sha256_hex(path.as_bytes())[..16]
        );
        let cached = cache
            .read(&name)
            .and_then(|text| Timeline::parse(&text, &path));
        let timeline = match cached {
            Some(timeline) if timeline.commits == commits => timeline,
            Some(timeline) if timeline.is_suffix_of(&commits) => {
                let added = &commits[..commits.len() - timeline.commits.len()];
                let versions = repository.file_versions(added, &path)?;
                let maps = added
                    .iter()
                    .map(|commit| version_maps(versions.get(commit)))
                    .collect::<Vec<_>>();
                let timeline = timeline.extended(&path, commits.clone(), &maps);
                cache.write(&name, &timeline.render());
                timeline
            }
            _ => {
                let versions = repository.file_versions(&commits, &path)?;
                let maps = commits
                    .iter()
                    .map(|commit| version_maps(versions.get(commit)))
                    .collect::<Vec<_>>();
                let timeline = Timeline::build(&path, commits.clone(), &maps);
                cache.write(&name, &timeline.render());
                timeline
            }
        };
        out.extend(timeline.answers(&keys));
    }
    Ok(out)
}

/// One logical key's revision -> definitional-hash map in one file version.
type KeyMap = BTreeMap<u32, crate::model::ContentHash>;

/// Every key's map in one file version; a version that is absent or unreadable is empty.
fn version_maps(text: Option<&Option<String>>) -> BTreeMap<String, KeyMap> {
    let Some(Some(text)) = text else {
        return BTreeMap::new();
    };
    let parsed = crate::syntax::parse(text, crate::diagnostics::FileId(0));
    let Some(file) = parsed.file else {
        return BTreeMap::new();
    };
    let mut out: BTreeMap<String, KeyMap> = BTreeMap::new();
    for (at, item) in file.items.iter().enumerate() {
        let crate::syntax::cst::Item::Record(record) = item else {
            continue;
        };
        let Ok(key) = crate::model::LogicalKey::parse(&record.key) else {
            continue;
        };
        if let Some(definitional) =
            crate::resolve::revision_independent_definitional_text(&file, at)
        {
            out.entry(key.to_string())
                .or_default()
                .insert(record.revision, crate::hash::content_hash(&definitional));
        }
    }
    out
}

/// A file's history as [`last_changes_cached`] keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Timeline {
    /// The file's path, so an entry can never answer for another file.
    path: String,
    /// The commits that touched the file, newest first: index 0 is the newest version.
    commits: Vec<Commit>,
    /// Logical key -> `(first index, map)` runs in index order. The first run starts at
    /// 0, and each holds until the next run starts; a key absent from a version has an
    /// empty map there. A key absent from every version has no entry.
    keys: BTreeMap<String, Vec<(usize, KeyMap)>>,
}

impl Timeline {
    /// The timeline of `commits`, from each version's maps in the same order.
    fn build(path: &str, commits: Vec<Commit>, maps: &[BTreeMap<String, KeyMap>]) -> Self {
        let mut keys: BTreeMap<String, Vec<(usize, KeyMap)>> = BTreeMap::new();
        let present: BTreeSet<&String> = maps.iter().flat_map(BTreeMap::keys).collect();
        for key in present {
            let runs = keys.entry(key.clone()).or_default();
            for (index, version) in maps.iter().enumerate() {
                push_run(runs, index, version.get(key).cloned().unwrap_or_default());
            }
        }
        Self {
            path: path.to_owned(),
            commits,
            keys,
        }
    }

    /// Whether this timeline's history is the older end of `commits`.
    fn is_suffix_of(&self, commits: &[Commit]) -> bool {
        commits.len() > self.commits.len()
            && commits[commits.len() - self.commits.len()..] == self.commits[..]
    }

    /// This timeline with newer versions in front: `commits` is the full new history and
    /// `maps` the versions it adds, newest first. Equal to [`Self::build`] over the whole
    /// history, because the old runs are re-laid with the same merging rule.
    fn extended(self, path: &str, commits: Vec<Commit>, maps: &[BTreeMap<String, KeyMap>]) -> Self {
        let shift = maps.len();
        let present: BTreeSet<String> = self
            .keys
            .keys()
            .cloned()
            .chain(maps.iter().flat_map(|version| version.keys().cloned()))
            .collect();
        let mut keys = BTreeMap::new();
        for key in present {
            let mut runs = Vec::new();
            for (index, version) in maps.iter().enumerate() {
                push_run(
                    &mut runs,
                    index,
                    version.get(&key).cloned().unwrap_or_default(),
                );
            }
            match self.keys.get(&key) {
                Some(old) => {
                    for (start, map) in old {
                        push_run(&mut runs, start + shift, map.clone());
                    }
                }
                None => push_run(&mut runs, shift, KeyMap::new()),
            }
            keys.insert(key, runs);
        }
        Self {
            path: path.to_owned(),
            commits,
            keys,
        }
    }

    /// The last definitional change of every revision present in the newest version
    /// under `keys` — exactly the set, and the answers, [`last_changes`] produces.
    fn answers(
        &self,
        keys: &BTreeSet<&crate::model::LogicalKey>,
    ) -> BTreeMap<crate::model::RevisionId, Commit> {
        let mut out = BTreeMap::new();
        for key in keys {
            let Some(runs) = self.keys.get(&key.to_string()) else {
                continue;
            };
            let Some((0, newest)) = runs.first() else {
                continue;
            };
            for (revision, hash) in newest {
                let id = crate::model::RevisionId::new((*key).clone(), *revision);
                // The walk keeps going while each older version still holds this
                // definition at or below this revision; a run is one stretch of
                // identical versions, so it either holds throughout or not at all.
                let mut last = 0;
                for (position, (_, map)) in runs.iter().enumerate() {
                    if run_definition_at_or_below(map, *revision) != Some(hash) {
                        break;
                    }
                    last = runs
                        .get(position + 1)
                        .map_or(self.commits.len() - 1, |(next, _)| next - 1);
                }
                out.insert(id, self.commits[last].clone());
            }
        }
        out
    }

    /// The cache entry's body.
    fn render(&self) -> String {
        let mut out = format!("path {}\n", self.path);
        for commit in &self.commits {
            out.push_str(&format!("commit {}\n", commit.as_str()));
        }
        for (key, runs) in &self.keys {
            out.push_str(&format!("key {key}\n"));
            for (start, map) in runs {
                out.push_str(&format!("run {start}"));
                for (revision, hash) in map {
                    out.push_str(&format!(" {revision} {}", hash.0));
                }
                out.push('\n');
            }
        }
        out
    }

    /// Reads an entry back, or `None` for anything malformed or for another path.
    fn parse(text: &str, path: &str) -> Option<Self> {
        let mut lines = text.lines();
        if lines.next()?.strip_prefix("path ")? != path {
            return None;
        }
        let mut commits = Vec::new();
        let mut keys: BTreeMap<String, Vec<(usize, KeyMap)>> = BTreeMap::new();
        let mut current: Option<String> = None;
        for line in lines {
            if let Some(hash) = line.strip_prefix("commit ") {
                if current.is_some() {
                    return None;
                }
                commits.push(Commit::new(hash).ok()?);
            } else if let Some(key) = line.strip_prefix("key ") {
                keys.entry(key.to_owned()).or_default();
                current = Some(key.to_owned());
            } else {
                let mut fields = line.strip_prefix("run ")?.split(' ');
                let start: usize = fields.next()?.parse().ok()?;
                let mut map = KeyMap::new();
                while let Some(revision) = fields.next() {
                    let hash = fields.next()?;
                    map.insert(
                        revision.parse().ok()?,
                        crate::model::ContentHash(hash.to_owned()),
                    );
                }
                let runs = keys.get_mut(current.as_ref()?)?;
                if runs
                    .last()
                    .map_or(start != 0, |(previous, _)| *previous >= start)
                    || start >= commits.len()
                {
                    return None;
                }
                runs.push((start, map));
            }
        }
        if commits.is_empty() || keys.values().any(Vec::is_empty) {
            return None;
        }
        Some(Self {
            path: path.to_owned(),
            commits,
            keys,
        })
    }
}

/// Appends a run unless it repeats the one before it.
fn push_run(runs: &mut Vec<(usize, KeyMap)>, start: usize, map: KeyMap) {
    if runs.last().is_none_or(|(_, previous)| *previous != map) {
        runs.push((start, map));
    }
}

/// [`definition_at_or_below`] over one key's map.
fn run_definition_at_or_below(map: &KeyMap, revision: u32) -> Option<&crate::model::ContentHash> {
    map.range(1..=revision).next_back().map(|(_, hash)| hash)
}

/// Definitional hashes for every revision of the wanted keys in one historical file
/// version, with the revision's own identity removed (D-038).
///
/// Every revision rather than only the wanted ones, because the walk continues past the
/// commit that introduced revision `n+1` into revision `n`'s history, and it can only do
/// that if `n`'s hash is here to compare against.
fn definitional_hashes(
    text: &str,
    wanted: &BTreeSet<crate::model::RevisionId>,
) -> BTreeMap<crate::model::RevisionId, crate::model::ContentHash> {
    let keys: BTreeSet<&crate::model::LogicalKey> = wanted.iter().map(|id| &id.key).collect();
    let parsed = crate::syntax::parse(text, crate::diagnostics::FileId(0));
    let Some(file) = parsed.file else {
        return BTreeMap::new();
    };
    let mut out = BTreeMap::new();
    for (at, item) in file.items.iter().enumerate() {
        let crate::syntax::cst::Item::Record(record) = item else {
            continue;
        };
        let Ok(key) = crate::model::LogicalKey::parse(&record.key) else {
            continue;
        };
        if !keys.contains(&key) {
            continue;
        }
        let id = crate::model::RevisionId::new(key, record.revision);
        if let Some(definitional) =
            crate::resolve::revision_independent_definitional_text(&file, at)
        {
            out.insert(id, crate::hash::content_hash(&definitional));
        }
    }
    out
}

/// The newest revision of `id`'s key at or below `id`, and its hash, in one file version.
///
/// The walk asks "was this record's definition already here?", and once it steps past the
/// commit that created revision `n+1` the answer lives under revision `n`. Searching
/// downwards rather than only at `n-1` covers a file version that predates several
/// revisions at once, which a squashed or rebased history produces.
fn definition_at_or_below(
    hashes: &BTreeMap<crate::model::RevisionId, crate::model::ContentHash>,
    id: &crate::model::RevisionId,
) -> Option<crate::model::ContentHash> {
    (1..=id.revision).rev().find_map(|revision| {
        hashes
            .get(&crate::model::RevisionId::new(id.key.clone(), revision))
            .cloned()
    })
}

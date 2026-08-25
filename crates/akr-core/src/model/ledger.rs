//! The ledger: a set of record revisions, plus the facts later phases attach to it.

use super::ident::{Commit, LogicalKey, Segment};
use super::record::Record;
use super::refs::{Reference, RevisionId};
use super::relation::Relation;
use super::state::State;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// The project declaration: its name and its declared namespaces.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Project {
    /// The project name, matching the `project` line of every source file.
    pub name: String,
    /// Declared namespaces. A key whose first segment is absent here fails V-002.
    pub namespaces: BTreeSet<Segment>,
}

impl Project {
    /// A project with the given name and namespaces.
    ///
    /// # Panics
    /// Panics if a namespace is not a valid segment. Intended for tests and for callers
    /// that have already validated their input.
    #[must_use]
    pub fn new(name: &str, namespaces: &[&str]) -> Self {
        Self {
            name: name.to_owned(),
            namespaces: namespaces
                .iter()
                .map(|n| Segment::new(n).expect("valid namespace segment"))
                .collect(),
        }
    }
}

/// Why a key has no single head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeadError {
    /// No revision of this key exists.
    UnknownKey(LogicalKey),
    /// More than one revision is live (V-012).
    MultipleLive(LogicalKey, Vec<u32>),
    /// No revision is live and more than one is unsuperseded.
    AmbiguousChainEnd(LogicalKey, Vec<u32>),
}

impl fmt::Display for HeadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownKey(k) => write!(f, "no record with key {k}"),
            Self::MultipleLive(k, rs) => write!(f, "{k} has {} live revisions", rs.len()),
            Self::AmbiguousChainEnd(k, rs) => {
                write!(
                    f,
                    "{k} has no single head; {} revisions are unsuperseded",
                    rs.len()
                )
            }
        }
    }
}

/// A content hash, `sha256:` plus 64 hex digits. Computed in P3, once canonical text
/// exists; P1 only compares them.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentHash(pub String);

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What the lock records about one sealed revision, and what the build computed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SealFact {
    /// The hash in `akr.lock`, if the lock has an entry.
    pub recorded: Option<ContentHash>,
    /// The lifecycle state recorded beside the hash in `akr.lock`.
    pub recorded_state: Option<State>,
    /// The hash the build computed, if it computed one.
    pub computed: Option<ContentHash>,
}

/// Commit ancestry, as a child-to-parent map. Filled by P5 from git.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Ancestry {
    parents: BTreeMap<Commit, Commit>,
}

impl Ancestry {
    /// Builds an ancestry from child-to-parent pairs.
    #[must_use]
    pub fn from_pairs(pairs: impl IntoIterator<Item = (Commit, Commit)>) -> Self {
        Self {
            parents: pairs.into_iter().collect(),
        }
    }

    /// Whether `descendant` is `ancestor` or is reachable from it.
    ///
    /// Returns `None` when either commit is unknown to this ancestry, so that a caller
    /// can skip a check rather than silently pass it. That distinction is the whole
    /// reason this returns an `Option`.
    #[must_use]
    pub fn is_descendant(&self, descendant: &Commit, ancestor: &Commit) -> Option<bool> {
        if descendant == ancestor {
            return Some(true);
        }
        if !self.knows(descendant) || !self.knows(ancestor) {
            return None;
        }
        let mut cursor = descendant;
        let mut guard = 0usize;
        while let Some(parent) = self.parents.get(cursor) {
            if parent == ancestor {
                return Some(true);
            }
            cursor = parent;
            guard += 1;
            if guard > self.parents.len() {
                return Some(false);
            }
        }
        Some(false)
    }

    /// Whether `c` is a commit this ancestry has facts about.
    ///
    /// `pub(crate)` because D-028 needs it directly: a legacy-sourced record's evidence
    /// still has to cite a commit the repository actually has, even once the
    /// descendant-commit comparison itself is waived.
    #[must_use]
    pub(crate) fn knows(&self, c: &Commit) -> bool {
        self.parents.contains_key(c) || self.parents.values().any(|p| p == c)
    }

    /// Whether this ancestry carries no facts at all — P1's no-git case, or a build with
    /// no repository. Distinguishes "we never asked git" from "we asked, and this
    /// particular commit isn't one it knows".
    #[must_use]
    pub(crate) fn has_facts(&self) -> bool {
        !self.parents.is_empty()
    }
}

/// Facts about a ledger that later phases supply.
///
/// P1 leaves these empty and the rules that need them say so explicitly rather than
/// passing vacuously. P3 fills `seals`; P5 fills `last_change` and `ancestry`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LedgerFacts {
    /// Recorded and computed hashes per sealed revision (V-024).
    pub seals: BTreeMap<RevisionId, SealFact>,
    /// Whether a lock file was present at all. `false` suppresses V-024 entirely.
    pub lock_present: bool,
    /// The last commit that changed each revision's content (V-020, D-016).
    pub last_change: BTreeMap<RevisionId, Commit>,
    /// Commit ancestry (V-020, D-016).
    pub ancestry: Ancestry,
    /// Referenced commits the resolved head does not reach (V-020, `AKR-G012`).
    ///
    /// [`Ancestry`] is a topological *order* over the commits a ledger mentions, not the
    /// commit graph: one walk instead of a process per comparison. An order always puts
    /// one of two commits first, so it cannot tell "older" from "on a branch this history
    /// does not contain". This set can, and only the wording depends on it — a verdict is
    /// never changed by what is in here.
    pub off_branch: BTreeSet<Commit>,
    /// Top-level scratch entries protected by `.agent/scratch/KEEP` (V-025).
    ///
    /// This is an input fact rather than ledger knowledge: the keep index is ordinary
    /// workspace state, but V-025 needs to distinguish a disposable scratch artefact
    /// from one the author explicitly made persistent.
    pub scratch_kept: BTreeSet<String>,
}

/// A child-to-parent index over `part_of`, used by the D-010 ref-term overlap test.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PartOfIndex {
    parents: BTreeMap<LogicalKey, LogicalKey>,
}

impl PartOfIndex {
    /// An index with no edges. Ref terms then overlap only when their keys are equal.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Adds one child-to-parent edge.
    pub fn insert(&mut self, child: LogicalKey, parent: LogicalKey) {
        self.parents.insert(child, parent);
    }

    /// Whether `ancestor` is reachable from `descendant` by following `part_of`.
    ///
    /// Cycle-safe: a cyclic `part_of` graph is V-015's problem, and this must not hang
    /// while that diagnostic is being produced.
    #[must_use]
    pub fn is_ancestor(&self, ancestor: &LogicalKey, descendant: &LogicalKey) -> bool {
        let mut cursor = descendant;
        let mut guard = 0usize;
        while let Some(parent) = self.parents.get(cursor) {
            if parent == ancestor {
                return true;
            }
            cursor = parent;
            guard += 1;
            if guard > self.parents.len() {
                return false;
            }
        }
        false
    }
}

/// A set of record revisions and the project that declares their namespaces.
///
/// Head resolution lives here because the validation rules need it and P3's resolver
/// builds on it rather than replacing it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Ledger {
    /// The project declaration.
    pub project: Project,
    /// Facts supplied by later phases.
    pub facts: LedgerFacts,
    records: Vec<Record>,
}

impl Ledger {
    /// An empty ledger for the given project.
    #[must_use]
    pub fn new(project: Project) -> Self {
        Self {
            project,
            facts: LedgerFacts::default(),
            records: Vec::new(),
        }
    }

    /// Adds a record. Duplicate revision identifiers are kept, so that V-001 can report
    /// them (`AKR-L041`) rather than the ledger silently dropping one.
    pub fn insert(&mut self, record: Record) {
        self.records.push(record);
    }

    /// Adds several records.
    pub fn extend(&mut self, records: impl IntoIterator<Item = Record>) {
        self.records.extend(records);
    }

    /// Every record revision, in insertion order.
    #[must_use]
    pub fn records(&self) -> &[Record] {
        &self.records
    }

    /// Every key present, sorted.
    #[must_use]
    pub fn keys(&self) -> Vec<&LogicalKey> {
        let mut keys: Vec<_> = self.records.iter().map(|r| &r.id.key).collect();
        keys.sort();
        keys.dedup();
        keys
    }

    /// Every revision of one key, sorted by revision number.
    #[must_use]
    pub fn revisions_of(&self, key: &LogicalKey) -> Vec<&Record> {
        let mut rs: Vec<_> = self.records.iter().filter(|r| &r.id.key == key).collect();
        rs.sort_by_key(|r| r.id.revision);
        rs
    }

    /// One revision by identifier.
    #[must_use]
    pub fn get(&self, id: &RevisionId) -> Option<&Record> {
        self.records.iter().find(|r| &r.id == id)
    }

    /// The head of a key, by the two-tier algorithm of `docs/04` §3.
    ///
    /// The live revision if there is one; otherwise the end of the supersession chain,
    /// so that a floating reference to a completed milestone still resolves. Whether the
    /// result is *live* is a separate question, asked by V-019 where it matters.
    ///
    /// # Errors
    /// Returns [`HeadError`] when the key is unknown or has no single head.
    pub fn head(&self, key: &LogicalKey) -> Result<&Record, HeadError> {
        let revisions = self.revisions_of(key);
        if revisions.is_empty() {
            return Err(HeadError::UnknownKey(key.clone()));
        }
        let live: Vec<_> = revisions.iter().filter(|r| r.is_live()).collect();
        match live.len() {
            1 => return Ok(live[0]),
            0 => {}
            _ => {
                return Err(HeadError::MultipleLive(
                    key.clone(),
                    live.iter().map(|r| r.id.revision).collect(),
                ));
            }
        }
        let superseded: BTreeSet<u32> = revisions
            .iter()
            .flat_map(|r| r.targets(Relation::Supersedes))
            .filter(|t| &t.key == key)
            .filter_map(|t| t.revision)
            .collect();
        let ends: Vec<_> = revisions
            .iter()
            .filter(|r| !superseded.contains(&r.id.revision))
            .collect();
        match ends.len() {
            1 => Ok(ends[0]),
            _ => Err(HeadError::AmbiguousChainEnd(
                key.clone(),
                ends.iter().map(|r| r.id.revision).collect(),
            )),
        }
    }

    /// Resolves a reference to the revision it names.
    ///
    /// Pinned references go straight to that revision; floating ones go through
    /// [`Ledger::head`].
    ///
    /// # Errors
    /// Returns [`HeadError`] when the key is unknown or has no single head. A pinned
    /// reference to a missing revision returns `Ok(None)`, which V-001 reports as
    /// `AKR-L003`.
    pub fn resolve(&self, reference: &Reference) -> Result<Option<&Record>, HeadError> {
        match reference.revision {
            Some(revision) => {
                if self.revisions_of(&reference.key).is_empty() {
                    return Err(HeadError::UnknownKey(reference.key.clone()));
                }
                Ok(self.get(&RevisionId::new(reference.key.clone(), revision)))
            }
            None => self.head(&reference.key).map(Some),
        }
    }

    /// The `part_of` index over current heads, for scope overlap.
    #[must_use]
    pub fn part_of_index(&self) -> PartOfIndex {
        let mut index = PartOfIndex::empty();
        for record in &self.records {
            if let Some(parent) = record.targets(Relation::PartOf).first() {
                index.insert(record.id.key.clone(), parent.key.clone());
            }
        }
        index
    }
}

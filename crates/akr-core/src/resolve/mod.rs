//! Stage C and stage D: linking, head resolution, and the resolved model.
//!
//! # What this module is, and what it is not
//!
//! It is **not** a second head-resolution algorithm.
//! [`Ledger::head`](crate::model::Ledger::head) and
//! [`Ledger::resolve`](crate::model::Ledger::resolve) already implement the two-tier rule
//! of `docs/04-references-and-versioning.md` §3, and the validation rules are written
//! against them. This module *wraps* them: it resolves every reference once, caches the
//! answers, and builds the derived structures later stages read — the head map, the
//! supersession chains, the resolution log that becomes `akr.lock`, and the content
//! hashes that seal sealed revisions.
//!
//! Reimplementing head resolution here would give the project two answers to the same
//! question, which is exactly the failure the whole design exists to avoid.
//!
//! # The resolved model
//!
//! [`ResolvedModel`] is the data structure of `docs/06-compiler-pipeline.md` §6, minus
//! the parts later phases own: `freshness` is P5's and `acceptance` verdicts need git
//! ancestry, so both are computed elsewhere and attached through
//! [`LedgerFacts`](crate::model::LedgerFacts).
//!
//! # Determinism
//!
//! Every collection here is a `BTreeMap`, a `BTreeSet`, or a `Vec` built by iterating
//! records in [`RevisionId`] order. Nothing depends on insertion order, hash iteration, or
//! filesystem order (`docs/06-compiler-pipeline.md` §11).

mod source;

pub use source::{
    SpanIndex, Workspace, canonical_record_text, definitional_record_text, load_workspace,
    revision_independent_definitional_text,
};

use crate::diagnostics::Diagnostic;
use crate::graph::{AtRisk, DiGraph, dependency_graph, propagate_staleness, sorted_records};
use crate::hash::{content_hash, source_graph_hash};
use crate::lock::{Build, Lock, ResolutionEntry, SealEntry, SourceEntry};
use crate::model::{
    Commit, ContentHash, ContentSlot, ContentValue, EvidenceResult, HeadError, Ledger, LogicalKey,
    Record, Reference, Relation, RevisionId, ScopeTerm, Segment, SourceKind,
};
use crate::validate;
use std::collections::{BTreeMap, BTreeSet};

/// The hash recorded for a revision whose canonical text the build did not have.
///
/// Sixty-four zeros: obviously not a real digest, and never equal to one. A lock entry
/// carrying it is a build that ran without a formatter, which the seam in
/// [`BuildInputs::canonical_text`] describes.
pub const UNKNOWN_HASH: &str =
    "sha256:0000000000000000000000000000000000000000000000000000000000000000";

/// One `.akr` file the build read.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SourceFile {
    /// Repo-root-relative path with forward slashes.
    pub path: String,
    /// SHA-256 over the file's **raw bytes on disk** (`spec/schema/akr-lock.md` §3.1).
    pub hash: ContentHash,
    /// How many record revisions it contains.
    pub records: u32,
    /// Size in bytes on disk, for `sources.byte_len` in the stage E cache.
    ///
    /// Not part of any hash and not written to the lock: the file hash already settles
    /// whether a file changed. This is here because the index records it, and recording it
    /// is cheaper than reopening every source file to find out.
    pub byte_len: u64,
}

/// Everything a build knows that the ledger itself does not.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BuildInputs {
    /// Tool name and version, for example `akr 0.1.0`.
    pub tool: String,
    /// Grammar version of the sources.
    pub grammar: String,
    /// `vocabulary_version` from `spec/tables/vocabulary.json`.
    pub vocabulary: String,
    /// The commit the build resolved against.
    pub commit: Option<Commit>,
    /// UTC timestamp for `build.built_at`. Informational only, and excluded from every
    /// comparison — see [`Lock::verify`].
    pub built_at: String,
    /// The source files, in any order; sorted where it matters.
    pub sources: Vec<SourceFile>,
    /// Canonical text per revision, keyed by revision identifier.
    ///
    /// # Seam
    ///
    /// Canonical text is the phase P2 formatter's output. Until it lands, a caller
    /// supplies text that is already canonical — which every committed `.akr` file is —
    /// or supplies nothing, in which case content hashes are [`UNKNOWN_HASH`] and
    /// [`ResolvedModel::missing_hashes`] names every revision affected. V-024 then reports
    /// nothing rather than accusing anyone of editing a sealed record, because
    /// [`crate::lock::Lock::apply_facts`] leaves `computed` unset for those revisions.
    pub canonical_text: BTreeMap<RevisionId, String>,
}

/// Where in a record a reference was written.
///
/// [`Record::references`](crate::model::Record::references) labels each reference with its
/// [`Relation`] where it has one, which is all the validation rules need. The lock needs
/// more: `spec/schema/akr-lock.md` §2.3 records "the slot the reference appeared in", and
/// `exceptions`, `disposition` and `into` are slots without being relations. This enum
/// carries that distinction, and [`RefSite::slot_name`] is what the lock writes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum RefSite {
    /// A relation slot: `depends_on`, `supersedes`, and the rest.
    Relation(Relation),
    /// A `ref` term in the `scope` slot.
    Scope,
    /// A reference-valued content slot, such as `exceptions`.
    Content(ContentSlot),
    /// The `supported_by` slot of a `claim` block.
    ClaimSupport(Segment),
    /// The `verified_by` slot of a `check` block.
    CheckEvidence(Segment),
    /// The head of a `disposition` block: the child being dispositioned.
    DispositionTarget,
    /// The `into` slot of a `disposition` block.
    DispositionInto,
}

impl RefSite {
    /// The slot name the lock records for this site.
    #[must_use]
    pub fn slot_name(&self) -> &str {
        match self {
            Self::Relation(r) => r.name(),
            Self::Scope => "scope",
            Self::Content(slot) => slot.name(),
            Self::ClaimSupport(_) => Relation::SupportedBy.name(),
            Self::CheckEvidence(_) => Relation::VerifiedBy.name(),
            Self::DispositionTarget => "disposition",
            Self::DispositionInto => "into",
        }
    }

    /// The relation this site implies, where it implies one.
    #[must_use]
    pub fn relation(&self) -> Option<Relation> {
        match self {
            Self::Relation(r) => Some(*r),
            Self::ClaimSupport(_) => Some(Relation::SupportedBy),
            Self::CheckEvidence(_) => Some(Relation::VerifiedBy),
            _ => None,
        }
    }
}

/// One resolved reference occurrence: an edge in the reference graph of stage C.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ResolvedEdge {
    /// The referring revision.
    pub from: RevisionId,
    /// Where in the referring record the reference was written.
    pub site: RefSite,
    /// The reference as authored.
    pub reference: Reference,
    /// What it resolved to, if it resolved.
    pub to: Option<RevisionId>,
    /// Whether the reference was pinned. Floating references are the ones the lock records.
    pub pinned: bool,
}

impl ResolvedEdge {
    /// The relation this edge carries, where it carries one.
    #[must_use]
    pub fn relation(&self) -> Option<Relation> {
        self.site.relation()
    }
}

/// Every reference in a record, labelled with the slot it was written in.
///
/// The traversal order matches
/// [`Record::references`](crate::model::Record::references): relation slots first in
/// [`Relation`] order — the vocabulary's declaration order, which is not alphabetical —
/// then scope, content, claims, checks and dispositions. The two agree about which
/// references exist and differ only in how precisely each is labelled.
#[must_use]
pub fn reference_sites(record: &Record) -> Vec<(RefSite, &Reference)> {
    let mut out: Vec<(RefSite, &Reference)> = Vec::new();
    for (relation, refs) in &record.relations {
        out.extend(refs.iter().map(|r| (RefSite::Relation(*relation), r)));
    }
    for term in &record.scope {
        if let ScopeTerm::Ref(reference) = term {
            out.push((RefSite::Scope, reference));
        }
    }
    for (slot, value) in &record.content {
        if let Some(refs) = value.as_refs() {
            out.extend(refs.iter().map(|r| (RefSite::Content(*slot), r)));
        }
    }
    for claim in &record.claims {
        out.extend(
            claim
                .supported_by
                .iter()
                .map(|r| (RefSite::ClaimSupport(claim.anchor.clone()), r)),
        );
    }
    if let Some(acceptance) = &record.acceptance {
        for check in &acceptance.checks {
            out.extend(
                check
                    .verified_by
                    .iter()
                    .map(|r| (RefSite::CheckEvidence(check.id.clone()), r)),
            );
        }
    }
    for disposition in &record.dispositions {
        out.push((RefSite::DispositionTarget, &disposition.target));
        if let Some(into) = &disposition.into {
            out.push((RefSite::DispositionInto, into));
        }
    }
    out
}

/// Why an acceptance check is or is not satisfied (D-016).
///
/// The four negative cases are distinguished because "not satisfied" without a reason is
/// a fact an agent cannot act on: `docs/09-context-assembly.md` §4 step 7 renders each of
/// them differently, and `ROADMAP.md` renders them into its acceptance table.
///
/// Not ordered: [`EvidenceResult`] has no ordering, and inventing one so that a verdict
/// could be sorted would imply a severity ranking the design does not have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Passing evidence, observed at a commit descending from the last content change.
    Satisfied {
        /// The evidence that satisfied it.
        by: RevisionId,
        /// Where that evidence was observed.
        observed_at: Commit,
        /// The last commit that changed the verified record, when the facts supply one.
        last_change: Option<Commit>,
    },
    /// The check cites no evidence at all.
    NoEvidence,
    /// The cited evidence does not resolve. V-001 reports the reference itself.
    Unresolved,
    /// Evidence exists and does not report `pass`.
    Failing {
        /// The evidence.
        by: RevisionId,
        /// What it reported.
        result: Option<EvidenceResult>,
    },
    /// Passing evidence, observed before the last content change — the condition that
    /// stops a test from 200 commits ago closing a milestone redefined yesterday.
    TooOld {
        /// The evidence.
        by: RevisionId,
        /// Where it was observed.
        observed_at: Commit,
        /// The last content change it fails to descend from.
        last_change: Commit,
    },
}

impl Verdict {
    /// Whether the check is satisfied.
    #[must_use]
    pub fn is_satisfied(&self) -> bool {
        matches!(self, Self::Satisfied { .. })
    }
}

/// One acceptance check and its verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckVerdict {
    /// The milestone or work record carrying the check.
    pub owner: RevisionId,
    /// The check identifier.
    pub check: Segment,
    /// Whether it is satisfied, and why not when it is not.
    pub verdict: Verdict,
}

/// Computes the verdict for every acceptance check in the ledger, in canonical order.
///
/// Mirrors V-020's selection exactly — the first citation that passes and descends wins;
/// otherwise the **last** failure reason is kept — so that the renderer and the rule can
/// never disagree about why a milestone is not done. V-020 turns an unsatisfied verdict
/// into `AKR-R022` for `completed` records only; this runs over every record, because a
/// roadmap has to show progress on the ones that are not finished.
///
/// The descendant condition applies only when
/// [`LedgerFacts`](crate::model::LedgerFacts) supplies `last_change` and an ancestry
/// (phase P5). Absent them, passing evidence satisfies a check and the verdict records
/// `last_change: None`, which is what V-020 does and for the same reason.
#[must_use]
pub fn acceptance_verdicts(ledger: &Ledger) -> Vec<CheckVerdict> {
    let mut out = Vec::new();
    for record in sorted_records(ledger) {
        let Some(acceptance) = &record.acceptance else {
            continue;
        };
        for check in &acceptance.checks {
            let mut verdict = Verdict::NoEvidence;
            for reference in &check.verified_by {
                let Some(evidence) = ledger.resolve(reference).ok().flatten() else {
                    verdict = Verdict::Unresolved;
                    continue;
                };
                let facts = citation_facts(ledger, &record.id, evidence);
                if facts.result != Some(EvidenceResult::Pass) {
                    verdict = Verdict::Failing {
                        by: evidence.id.clone(),
                        result: facts.result,
                    };
                    continue;
                }
                match (facts.descends, facts.observed_at, facts.last_change) {
                    (true, Some(observed_at), last_change) => {
                        verdict = Verdict::Satisfied {
                            by: evidence.id.clone(),
                            observed_at,
                            // A co-commit is the prepared-change case, not an ancestry
                            // statement worth rendering. Keeping it absent also makes a
                            // projection built before the commit byte-stable afterwards.
                            last_change: if facts.co_committed {
                                None
                            } else {
                                last_change
                            },
                        };
                        break;
                    }
                    (false, Some(observed_at), Some(last_change)) => {
                        verdict = Verdict::TooOld {
                            by: evidence.id.clone(),
                            observed_at,
                            last_change,
                        };
                    }
                    _ => {
                        verdict = Verdict::Unresolved;
                    }
                }
            }
            out.push(CheckVerdict {
                owner: record.id.clone(),
                check: check.id.clone(),
                verdict,
            });
        }
    }
    out
}

/// What one citation of a check says, before any selection between citations.
///
/// Split out so that [`acceptance_verdicts`] — which picks *one* citation per check — and
/// [`evidence_links`] — which records *every* citation — cannot disagree about what a
/// citation means. The selection differs; the facts must not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CitationFacts {
    /// What the evidence reported, where it reported anything.
    pub result: Option<EvidenceResult>,
    /// The commit the evidence was observed at.
    pub observed_at: Option<Commit>,
    /// The last commit that changed the verified record, when the facts supply one.
    pub last_change: Option<Commit>,
    /// Whether `observed_at` descends from `last_change`.
    ///
    /// True when either is unknown, which is what V-020 does: absent git facts, evidence
    /// is taken at its word rather than accused of being stale.
    pub descends: bool,
    /// Whether evidence and verified content have the same last-change commit.
    pub co_committed: bool,
}

/// Evaluates one citation of one check (D-016, D-028).
#[must_use]
pub fn citation_facts(ledger: &Ledger, owner: &RevisionId, evidence: &Record) -> CitationFacts {
    let result = evidence
        .get(ContentSlot::Result)
        .and_then(ContentValue::as_enum)
        .and_then(|e| EvidenceResult::from_name(e.as_str()));
    let observed_at = evidence
        .get(ContentSlot::ObservedAt)
        .and_then(ContentValue::as_commit)
        .cloned();
    let last_change = ledger.facts.last_change.get(owner).cloned();
    // D-028: a legacy-sourced record's own git introduction date says nothing about when
    // the work happened, so the descendant-commit comparison is waived for it. The
    // evidence commit still has to be one this repository actually has, whenever git
    // facts were supplied at all — that containment check is not waived.
    let is_legacy = ledger
        .get(owner)
        .is_some_and(|record| record.sources.iter().any(|s| s.kind == SourceKind::Legacy));
    let co_committed = last_change.as_ref().is_some_and(|owner_change| {
        ledger.facts.last_change.get(&evidence.id) == Some(owner_change)
    });
    let descends = if co_committed {
        // The record and its evidence landing together is the change-transaction shape:
        // the evidence necessarily names the prepared parent because this commit did not
        // exist yet. Equal last-change commits distinguish it from genuinely old evidence.
        true
    } else if is_legacy {
        match &observed_at {
            Some(commit) if ledger.facts.ancestry.has_facts() => {
                ledger.facts.ancestry.knows(commit)
            }
            _ => true,
        }
    } else {
        match (&observed_at, &last_change) {
            (Some(observed), Some(changed)) => ledger
                .facts
                .ancestry
                .is_descendant(observed, changed)
                .unwrap_or(true),
            _ => true,
        }
    };
    CitationFacts {
        result,
        observed_at,
        last_change,
        descends,
        co_committed,
    }
}

/// One `check -> evidence` citation, evaluated, as `evidence_links` records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceLink {
    /// The verified record.
    pub owner: RevisionId,
    /// Which check of it.
    pub check: Segment,
    /// The evidence cited.
    pub evidence: RevisionId,
    /// What the evidence reported.
    pub result: EvidenceResult,
    /// Where it was observed.
    pub observed_at: Commit,
    /// The last commit that changed the verified content.
    pub last_change: Commit,
    /// Whether `observed_at` descends from `last_change`.
    pub descends: bool,
    /// `result = pass AND descends`, which is what satisfies a check.
    pub satisfies: bool,
}

/// Every citation the ledger makes, in canonical order.
///
/// Only citations whose result, observation commit and last-change commit are all known
/// appear: the row exists to record the descendant verdict, and without those three there
/// is no verdict to record. A build with no git facts therefore produces none, which is
/// the same condition under which V-020 declines to judge.
#[must_use]
pub fn evidence_links(ledger: &Ledger) -> Vec<EvidenceLink> {
    let mut out = Vec::new();
    for record in sorted_records(ledger) {
        let Some(acceptance) = &record.acceptance else {
            continue;
        };
        for check in &acceptance.checks {
            for reference in &check.verified_by {
                let Some(evidence) = ledger.resolve(reference).ok().flatten() else {
                    continue;
                };
                let facts = citation_facts(ledger, &record.id, evidence);
                let (Some(result), Some(observed_at), Some(last_change)) =
                    (facts.result, facts.observed_at, facts.last_change)
                else {
                    continue;
                };
                out.push(EvidenceLink {
                    owner: record.id.clone(),
                    check: check.id.clone(),
                    evidence: evidence.id.clone(),
                    result,
                    observed_at,
                    last_change,
                    descends: facts.descends,
                    satisfies: result == EvidenceResult::Pass && facts.descends,
                });
            }
        }
    }
    out
}

/// The output of stages C and D: what every later stage and read command consumes.
#[derive(Debug, Clone)]
pub struct ResolvedModel<'a> {
    ledger: &'a Ledger,
    /// The commit the build resolved against.
    pub commit: Option<Commit>,
    /// Tool version.
    pub tool_version: String,
    /// Grammar version.
    pub grammar_version: String,
    /// Vocabulary version.
    pub vocabulary_version: String,
    /// The source-graph hash over the raw bytes of every source file.
    pub source_graph: ContentHash,
    /// The `build.built_at` timestamp for the lock, taken from the build inputs.
    ///
    /// Informational only, excluded from every comparison, and never read from a clock
    /// inside the build — it is an input, so two builds with the same inputs still
    /// produce the same lock (`spec/schema/akr-lock.md` §2.1, §6).
    pub built_at: String,
    /// The source files, sorted by path.
    pub sources: Vec<SourceFile>,
    /// The head of every key that has one.
    pub heads: BTreeMap<LogicalKey, RevisionId>,
    /// Why a key has no single head, for the keys that do not.
    pub head_errors: BTreeMap<LogicalKey, HeadError>,
    /// Content hash per revision, for every revision whose canonical text was supplied.
    pub content_hashes: BTreeMap<RevisionId, ContentHash>,
    /// Revisions whose canonical text was not supplied.
    pub missing_hashes: BTreeSet<RevisionId>,
    /// Every resolved reference occurrence, in canonical order.
    pub edges: Vec<ResolvedEdge>,
    /// The floating resolutions the lock records, deduplicated per §2.3 and sorted.
    pub resolutions: Vec<ResolutionEntry>,
    /// Supersession chains: for each key, its revisions oldest-first along `supersedes`.
    pub supersession: BTreeMap<LogicalKey, Vec<RevisionId>>,
    /// Every acceptance check and its verdict, in canonical order.
    pub acceptance: Vec<CheckVerdict>,
    /// Diagnostics from stages A–D, in rule then code then subject order.
    pub diagnostics: Vec<Diagnostic>,
}

impl<'a> ResolvedModel<'a> {
    /// Runs stages C and D over a ledger.
    ///
    /// Linking, head resolution, supersession chains, content hashing, the resolution
    /// log, and the full rule catalogue. The rules are run through
    /// [`validate::validate_all`] rather than re-expressed, so there is one catalogue and
    /// one ordering.
    #[must_use]
    pub fn build(ledger: &'a Ledger, inputs: &BuildInputs) -> Self {
        let mut sources = inputs.sources.clone();
        sources.sort();
        sources.dedup_by(|a, b| a.path == b.path);
        let source_graph = source_graph_hash(sources.iter().map(|s| (s.path.as_str(), &s.hash)));

        let mut content_hashes = BTreeMap::new();
        let mut missing_hashes = BTreeSet::new();
        for record in sorted_records(ledger) {
            match inputs.canonical_text.get(&record.id) {
                Some(text) => {
                    content_hashes.insert(record.id.clone(), content_hash(text));
                }
                None => {
                    missing_hashes.insert(record.id.clone());
                }
            }
        }

        let mut heads = BTreeMap::new();
        let mut head_errors = BTreeMap::new();
        for key in ledger.keys() {
            match ledger.head(key) {
                Ok(record) => {
                    heads.insert(key.clone(), record.id.clone());
                }
                Err(error) => {
                    head_errors.insert(key.clone(), error);
                }
            }
        }

        let edges = link(ledger);
        let resolutions = resolution_log(&edges, &content_hashes);
        let supersession = supersession_chains(ledger);

        Self {
            ledger,
            commit: inputs.commit.clone(),
            tool_version: inputs.tool.clone(),
            grammar_version: inputs.grammar.clone(),
            vocabulary_version: inputs.vocabulary.clone(),
            source_graph,
            built_at: inputs.built_at.clone(),
            sources,
            heads,
            head_errors,
            content_hashes,
            missing_hashes,
            edges,
            resolutions,
            supersession,
            acceptance: acceptance_verdicts(ledger),
            diagnostics: validate::validate_all(ledger),
        }
    }

    /// The ledger this model was built from.
    #[must_use]
    pub fn ledger(&self) -> &'a Ledger {
        self.ledger
    }

    /// Whether a revision is the head of its key.
    #[must_use]
    pub fn is_head(&self, id: &RevisionId) -> bool {
        self.heads.get(&id.key) == Some(id)
    }

    /// The acceptance checks of one record, in check-identifier order.
    #[must_use]
    pub fn checks_of(&self, owner: &RevisionId) -> Vec<&CheckVerdict> {
        self.acceptance
            .iter()
            .filter(|v| &v.owner == owner)
            .collect()
    }

    /// The content hash of a revision, if the build computed one.
    #[must_use]
    pub fn content_hash(&self, id: &RevisionId) -> Option<&ContentHash> {
        self.content_hashes.get(id)
    }

    /// Whether the build produced any diagnostic of severity `error`.
    ///
    /// Applying the `--strict` profile — under which warnings are errors — is the
    /// caller's job, not this crate's (D-013).
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == crate::diagnostics::Severity::Error)
    }

    /// Walks forward along `supersedes`: what replaced this revision, transitively.
    ///
    /// Returns the revision itself when nothing supersedes it. Cycle-safe: a supersession
    /// cycle is V-014's problem, and this must not hang while that diagnostic is produced.
    #[must_use]
    pub fn current(&self, id: &RevisionId) -> RevisionId {
        let chain = self.supersession.get(&id.key);
        let Some(chain) = chain else {
            return id.clone();
        };
        match chain.iter().position(|c| c == id) {
            Some(at) => chain[chain.len() - 1].clone().max(chain[at].clone()),
            None => id.clone(),
        }
    }

    /// Walks backward along `supersedes`: the revisions this one replaced, newest first.
    #[must_use]
    pub fn history(&self, id: &RevisionId) -> Vec<RevisionId> {
        let Some(chain) = self.supersession.get(&id.key) else {
            return vec![id.clone()];
        };
        match chain.iter().position(|c| c == id) {
            Some(at) => chain[..=at].iter().rev().cloned().collect(),
            None => vec![id.clone()],
        }
    }

    /// The dependency graph over the three propagating relations.
    #[must_use]
    pub fn dependency_graph(&self) -> DiGraph<RevisionId> {
        dependency_graph(self.ledger)
    }

    /// Propagates staleness from a set of stale revisions (D-024).
    ///
    /// The set itself is P5's to compute, from `observed_at`, `watches` and
    /// `review_after`. The walk is here because it is a graph operation over the resolved
    /// model, and because implementing it now means P5 adds a predicate rather than an
    /// algorithm.
    #[must_use]
    pub fn at_risk(&self, stale: &BTreeSet<RevisionId>) -> Vec<AtRisk> {
        propagate_staleness(self.ledger, stale)
    }

    /// Builds the `akr.lock` this model implies.
    ///
    /// Every field is derived: sources and their raw-byte hashes from the build inputs,
    /// the source-graph hash from those, resolutions from the link log, and one seal per
    /// revision in a state other than `proposed` (D-015). Ordering is applied by
    /// [`Lock::render`].
    #[must_use]
    pub fn to_lock(&self) -> Lock {
        let seals = sorted_records(self.ledger)
            .into_iter()
            .filter(|r| r.is_sealed())
            .map(|r| SealEntry {
                id: r.id.clone(),
                state: r.state,
                hash: self
                    .content_hashes
                    .get(&r.id)
                    .cloned()
                    .unwrap_or_else(|| ContentHash(UNKNOWN_HASH.to_owned())),
            })
            .collect();

        Lock {
            project: self.ledger.project.name.clone(),
            build: Build {
                tool: self.tool_version.clone(),
                grammar: self.grammar_version.clone(),
                vocabulary: self.vocabulary_version.clone(),
                commit: self.commit.clone(),
                source_graph: self.source_graph.clone(),
                built_at: self.built_at.clone(),
            },
            sources: self
                .sources
                .iter()
                .map(|s| SourceEntry {
                    path: s.path.clone(),
                    hash: s.hash.clone(),
                    records: s.records,
                })
                .collect(),
            resolutions: self.resolutions.clone(),
            seals,
        }
    }
}

// -------------------------------------------------------------------------------------
// Stage C
// -------------------------------------------------------------------------------------

/// Stage C: resolve every reference occurrence in the ledger.
///
/// Records are visited in [`RevisionId`] order and, within a record, in the order
/// [`reference_sites`] yields — relation slots first in [`Relation`] order, then scope,
/// content, claims, checks and dispositions. That makes the resolution log byte-stable
/// (`docs/06-compiler-pipeline.md` §5).
///
/// Resolution failures are not reported here. V-001 owns that message, and producing it
/// twice would double every diagnostic about a typo.
#[must_use]
pub fn link(ledger: &Ledger) -> Vec<ResolvedEdge> {
    let mut edges = Vec::new();
    for record in sorted_records(ledger) {
        for (site, reference) in reference_sites(record) {
            edges.push(ResolvedEdge {
                from: record.id.clone(),
                site,
                reference: reference.clone(),
                to: ledger
                    .resolve(reference)
                    .ok()
                    .flatten()
                    .map(|target| target.id.clone()),
                pinned: reference.is_pinned(),
            });
        }
    }
    edges
}

/// The lock's `resolution` entries: one per distinct (referring revision, slot, target
/// key) among the **floating** references that resolved (`spec/schema/akr-lock.md` §2.3).
///
/// Pinned references are excluded: a pinned reference cannot change what it points at, so
/// locking it would be noise. Anchors are excluded too — the anchor is part of the
/// referring record's text, and the resolution being locked is the revision.
#[must_use]
pub fn resolution_log(
    edges: &[ResolvedEdge],
    hashes: &BTreeMap<RevisionId, ContentHash>,
) -> Vec<ResolutionEntry> {
    let mut seen: BTreeMap<(RevisionId, String, LogicalKey), ResolutionEntry> = BTreeMap::new();

    for edge in edges {
        if edge.pinned {
            continue;
        }
        let Some(to) = &edge.to else { continue };
        let slot = edge.site.slot_name().to_owned();
        seen.entry((edge.from.clone(), slot.clone(), to.key.clone()))
            .or_insert_with(|| ResolutionEntry {
                from: edge.from.clone(),
                slot,
                to: to.clone(),
                hash: hashes
                    .get(to)
                    .cloned()
                    .unwrap_or_else(|| ContentHash(UNKNOWN_HASH.to_owned())),
            });
    }
    seen.into_values().collect()
}

/// Supersession chains: for each key, its revisions oldest-first along `supersedes`.
///
/// Only edges within one key contribute; a `supersedes` edge across keys is V-014's
/// problem (`AKR-R017`) and is ignored here rather than producing a nonsensical chain.
/// Revisions that no chain reaches are appended in revision order, so every revision of a
/// key appears exactly once and the result is a total order even for a key with no
/// `supersedes` edges at all.
#[must_use]
pub fn supersession_chains(ledger: &Ledger) -> BTreeMap<LogicalKey, Vec<RevisionId>> {
    let mut out = BTreeMap::new();

    for key in ledger.keys() {
        let revisions = ledger.revisions_of(key);
        // successor[old] = new, from `supersedes` edges within this key.
        let mut successor: BTreeMap<u32, u32> = BTreeMap::new();
        for record in &revisions {
            for target in record.targets(Relation::Supersedes) {
                if &target.key != key {
                    continue;
                }
                if let Some(old) = target.revision.or_else(|| {
                    ledger
                        .head(&target.key)
                        .ok()
                        .map(|record| record.id.revision)
                }) {
                    successor.entry(old).or_insert(record.id.revision);
                }
            }
        }

        let superseded: BTreeSet<u32> = successor.keys().copied().collect();
        let successors: BTreeSet<u32> = successor.values().copied().collect();
        let mut chain: Vec<u32> = Vec::new();
        let mut placed: BTreeSet<u32> = BTreeSet::new();

        // Start from every revision that supersedes nothing, in ascending order.
        for record in &revisions {
            let revision = record.id.revision;
            if successors.contains(&revision) || placed.contains(&revision) {
                continue;
            }
            let mut cursor = revision;
            loop {
                if !placed.insert(cursor) {
                    break; // cycle; V-014 reports it
                }
                chain.push(cursor);
                match successor.get(&cursor) {
                    Some(next) if !placed.contains(next) => cursor = *next,
                    _ => break,
                }
            }
        }
        // Anything a chain did not reach — a cycle member, or an orphan.
        for record in &revisions {
            if placed.insert(record.id.revision) {
                chain.push(record.id.revision);
            }
        }
        debug_assert_eq!(chain.len(), revisions.len());
        let _ = &superseded;

        out.insert(
            key.clone(),
            chain
                .into_iter()
                .map(|revision| RevisionId::new(key.clone(), revision))
                .collect(),
        );
    }
    out
}

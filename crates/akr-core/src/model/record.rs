//! Records: the unit of knowledge, and the blocks they contain.

use super::ident::{Commit, Date, Glob, Segment};
use super::kind::{ContentSlot, Kind};
use super::refs::{Reference, RevisionId};
use super::relation::Relation;
use super::scope::ScopeTerm;
use super::state::State;
use std::collections::BTreeMap;

/// The value of a content slot.
///
/// Deliberately small: the model distinguishes the value shapes the rules care about and
/// nothing more.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentValue {
    /// A single-line quoted string.
    Text(String),
    /// A triple-quoted prose block.
    Prose(String),
    /// A bare date.
    Date(Date),
    /// A `git:`-prefixed commit.
    Commit(Commit),
    /// A bare enum member.
    Enum(Segment),
    /// An array of quoted strings.
    Strings(Vec<String>),
    /// An array of path globs.
    Globs(Vec<Glob>),
    /// An array of references.
    Refs(Vec<Reference>),
}

impl ContentValue {
    /// Convenience constructor for prose.
    #[must_use]
    pub fn prose(text: &str) -> Self {
        Self::Prose(text.to_owned())
    }

    /// The commit, if this is one.
    #[must_use]
    pub fn as_commit(&self) -> Option<&Commit> {
        match self {
            Self::Commit(c) => Some(c),
            _ => None,
        }
    }

    /// The enum member, if this is one.
    #[must_use]
    pub fn as_enum(&self) -> Option<&Segment> {
        match self {
            Self::Enum(s) => Some(s),
            _ => None,
        }
    }

    /// The references, if this is a reference array.
    #[must_use]
    pub fn as_refs(&self) -> Option<&[Reference]> {
        match self {
            Self::Refs(r) => Some(r),
            _ => None,
        }
    }
}

/// An individually addressable assertion within a record (D-011).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    /// The anchor, unique within the record.
    pub anchor: Segment,
    /// The claim text.
    pub text: String,
    /// Empirical records backing this claim.
    pub supported_by: Vec<Reference>,
}

/// How an acceptance check is carried out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckMethod {
    /// A person checks.
    Manual,
    /// A command is run.
    Command,
    /// An observation is recorded.
    Observation,
}

impl CheckMethod {
    /// Every check method, in the order `spec/tables/vocabulary.json` declares them for
    /// the `check` block's `method` slot.
    pub const ALL: &'static [CheckMethod] = &[
        CheckMethod::Manual,
        CheckMethod::Command,
        CheckMethod::Observation,
    ];

    /// The name as written in source.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Command => "command",
            Self::Observation => "observation",
        }
    }
}

/// One acceptance check (D-016).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    /// The check identifier, unique within the acceptance block; also an anchor.
    pub id: Segment,
    /// The observable outcome.
    pub statement: String,
    /// How it is carried out.
    pub method: CheckMethod,
    /// The exact command, where there is one.
    pub command: Option<String>,
    /// Evidence records that satisfy it. The relation runs this way only.
    pub verified_by: Vec<Reference>,
}

/// What "done" means for a planning record.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Acceptance {
    /// The checks, canonically sorted by identifier.
    pub checks: Vec<Check>,
}

/// What happened to an unfinished child across a supersession (D-017).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Still to be done, under the target named by `into`.
    CarriedForward,
    /// Already done, by the work named by `into`.
    CompletedElsewhere,
    /// Decided against. `into` is forbidden.
    IntentionallyDropped,
    /// Still needed, outside any current plan. `into` is optional.
    StillRequiredSeparately,
}

impl Outcome {
    /// Every outcome.
    pub const ALL: &'static [Outcome] = &[
        Outcome::CarriedForward,
        Outcome::CompletedElsewhere,
        Outcome::IntentionallyDropped,
        Outcome::StillRequiredSeparately,
    ];

    /// The name as written in source.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::CarriedForward => "carried_forward",
            Self::CompletedElsewhere => "completed_elsewhere",
            Self::IntentionallyDropped => "intentionally_dropped",
            Self::StillRequiredSeparately => "still_required_separately",
        }
    }

    /// Whether `into` is required for this outcome.
    #[must_use]
    pub const fn requires_into(self) -> bool {
        matches!(self, Self::CarriedForward | Self::CompletedElsewhere)
    }

    /// Whether `into` is forbidden for this outcome.
    #[must_use]
    pub const fn forbids_into(self) -> bool {
        matches!(self, Self::IntentionallyDropped)
    }
}

/// The disposition of one unfinished child (D-017).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disposition {
    /// The child being dispositioned; the block head.
    pub target: Reference,
    /// What happened to it.
    pub outcome: Outcome,
    /// Where it went.
    pub into: Option<Reference>,
    /// Why.
    pub note: Option<String>,
}

/// The result recorded by an evidence record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceResult {
    /// The check passed.
    Pass,
    /// The check failed. A legitimate and valuable record.
    Fail,
    /// The check ran and settled nothing.
    Inconclusive,
}

impl EvidenceResult {
    /// The name as written in source.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Inconclusive => "inconclusive",
        }
    }

    /// Looks up a result by name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "pass" => Some(Self::Pass),
            "fail" => Some(Self::Fail),
            "inconclusive" => Some(Self::Inconclusive),
            _ => None,
        }
    }
}

/// Where a `source` block's content came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// A legacy document being migrated (D-022).
    Legacy,
    /// Something outside the repository.
    External,
    /// Something inside it.
    Internal,
}

/// What a source contributes to a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceRole {
    /// The record originated in the source material.
    Origin,
    /// The source explains why the project chose this direction.
    Rationale,
    /// The source is treated as an externally supplied observation.
    Evidence,
    /// The source supplies a constraint the record must respect.
    Constraint,
    /// The source is retained as an illustrative example.
    Example,
}

impl SourceRole {
    /// The record language's spelling of this role.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Origin => "origin",
            Self::Rationale => "rationale",
            Self::Evidence => "evidence",
            Self::Constraint => "constraint",
            Self::Example => "example",
        }
    }

    /// Parses the record language's spelling; `None` for anything else.
    // Inherent rather than `FromStr`: these parse the *record language's* spelling and return
    // `Option`, where the trait demands an associated error type and a blanket `parse()`
    // that would invite parsing arbitrary strings into them.
    #[allow(clippy::should_implement_trait)]
    #[must_use]
    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "origin" => Some(Self::Origin),
            "rationale" => Some(Self::Rationale),
            "evidence" => Some(Self::Evidence),
            "constraint" => Some(Self::Constraint),
            "example" => Some(Self::Example),
            _ => None,
        }
    }
}

/// Provenance for a record's content. Provenance, not identity: a record does not become
/// invalid when its source file is deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// Which sort of source.
    pub kind: SourceKind,
    /// The role this source plays in the record.
    pub role: Option<SourceRole>,
    /// Repo-relative path.
    pub path: Option<String>,
    /// URL, for external sources.
    pub url: Option<String>,
    /// The passage this record came from.
    pub excerpt: Option<String>,
    /// The id of a registered source document (D-031), when this is a citation into the
    /// immutable library rather than a loose path.
    ///
    /// A path can move, be rewritten or be deleted; a registered document is
    /// content-hashed and append-only, so a citation naming one still resolves years
    /// later. That is the difference between provenance that survives and provenance
    /// that decays into a broken link.
    pub document: Option<String>,
    /// The exact byte range within that document.
    ///
    /// Bytes, not a chunk id: chunk boundaries belong to a rebuildable index and move
    /// when the scanner improves, and a citation that moved with them would silently
    /// start pointing somewhere else.
    pub range: Option<SourceRange>,
    /// What the project adopted or retained from the source.
    pub use_note: Option<String>,
}

/// An exact locator into a registered source document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRange {
    /// First byte, inclusive.
    pub start_byte: u64,
    /// One past the last byte.
    pub end_byte: u64,
    /// One-based first line. For people and rendered citations only.
    pub start_line: u32,
    /// One-based last line.
    pub end_line: u32,
    /// `sha256:` over the cited bytes, when the author recorded one.
    ///
    /// The range says where; this says that the passage is still the passage. A
    /// registered document cannot change without changing its own content hash, so this
    /// is belt and braces — but it is what turns "the range is in bounds" into "the range
    /// is the text the record was written about".
    pub excerpt_hash: Option<String>,
}

/// One revision of one record.
///
/// Construct these with [`super::RecordBuilder`] in tests, or from parsed text in P2.
/// Every field is public: P3 builds a resolved view over these, and hiding them behind
/// accessors would buy nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// Key and revision number.
    pub id: RevisionId,
    /// What sort of claim this is.
    pub kind: Kind,
    /// One-line human label; the heading in every generated view.
    pub title: String,
    /// Where in its lifecycle this revision sits.
    pub state: State,
    /// What it governs or was observed about.
    pub scope: Vec<ScopeTerm>,
    /// The opt-in exclusivity handle (D-004b). Normative kinds only.
    pub topic: Option<Segment>,
    /// Kind-specific content slots.
    pub content: BTreeMap<ContentSlot, ContentValue>,
    /// Addressable claims, canonically sorted by anchor.
    pub claims: Vec<Claim>,
    /// Anchors the previous revision had and this one drops (D-011).
    pub retired_claims: Vec<Segment>,
    /// What "done" means, for planning kinds.
    pub acceptance: Option<Acceptance>,
    /// Dispositions of unfinished children (D-017).
    pub dispositions: Vec<Disposition>,
    /// Relation slots.
    pub relations: BTreeMap<Relation, Vec<Reference>>,
    /// Marks a declared contradiction as knowingly tolerated (D-023).
    pub acknowledged: bool,
    /// Who authored this revision. Free text, not an identity system.
    pub author: Option<String>,
    /// When this revision was authored.
    pub created_at: Option<Date>,
    /// Provenance blocks.
    pub sources: Vec<Source>,
    /// The source file this record was loaded from, for V-003. `None` when built in code.
    pub file: Option<String>,
}

impl Record {
    /// Whether this revision is live for its kind's class.
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.state.is_live_in(self.kind.class())
    }

    /// Whether this revision is terminal for its kind's class.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        self.state.is_terminal_in(self.kind.class())
    }

    /// Whether this revision is sealed: any state other than `proposed` (D-015).
    #[must_use]
    pub fn is_sealed(&self) -> bool {
        self.state != State::Proposed
    }

    /// The targets of one relation, or an empty slice.
    #[must_use]
    pub fn targets(&self, relation: Relation) -> &[Reference] {
        self.relations.get(&relation).map_or(&[], Vec::as_slice)
    }

    /// A content slot's value, if present.
    #[must_use]
    pub fn get(&self, slot: ContentSlot) -> Option<&ContentValue> {
        self.content.get(&slot)
    }

    /// Whether a claim or check anchor exists on this revision.
    #[must_use]
    pub fn has_anchor(&self, anchor: &Segment) -> bool {
        self.claims.iter().any(|c| &c.anchor == anchor)
            || self
                .acceptance
                .as_ref()
                .is_some_and(|a| a.checks.iter().any(|c| &c.id == anchor))
    }

    /// Every reference this record makes, paired with the slot it appears in.
    ///
    /// Covers relation slots, scope `ref` terms, reference-valued content slots, claim
    /// support, check evidence, and disposition heads and targets. This is the traversal
    /// V-001, V-004, V-005 and V-006 are built on.
    ///
    /// # Order
    ///
    /// Relation slots come first, in [`Relation`] **declaration order** — the order of
    /// [`Relation::ALL`], which is the vocabulary's order — because `relations` is a
    /// `BTreeMap` keyed by the enum. That is not alphabetical by name. Scope, content,
    /// claim, check and disposition references follow, in that order. The order is
    /// stable across runs, which is what callers actually depend on; nothing should rely
    /// on it being any particular order beyond that.
    #[must_use]
    pub fn references(&self) -> Vec<(Option<Relation>, &Reference)> {
        let mut out: Vec<(Option<Relation>, &Reference)> = Vec::new();
        for (rel, refs) in &self.relations {
            out.extend(refs.iter().map(|r| (Some(*rel), r)));
        }
        for term in &self.scope {
            if let ScopeTerm::Ref(r) = term {
                out.push((None, r));
            }
        }
        for value in self.content.values() {
            if let ContentValue::Refs(refs) = value {
                out.extend(refs.iter().map(|r| (None, r)));
            }
        }
        for claim in &self.claims {
            out.extend(
                claim
                    .supported_by
                    .iter()
                    .map(|r| (Some(Relation::SupportedBy), r)),
            );
        }
        if let Some(acceptance) = &self.acceptance {
            for check in &acceptance.checks {
                out.extend(
                    check
                        .verified_by
                        .iter()
                        .map(|r| (Some(Relation::VerifiedBy), r)),
                );
            }
        }
        for d in &self.dispositions {
            out.push((None, &d.target));
            if let Some(into) = &d.into {
                out.push((None, into));
            }
        }
        out
    }

    /// Rewrites every non-historical pinned reference this record makes.
    ///
    /// Covers relation slots that are not [`Relation::is_historical`], claim
    /// `supported_by`, and check `verified_by`. Historical pins (`supersedes`,
    /// `contradicts`, `derived_from`) stay, and so does `part_of`: V-017's
    /// dispositions name the children of a *specific* plan revision, and moving
    /// those pins onto the successor would make every disposition miss.
    ///
    /// Returns whether `rewrite` reported a change on any reference.
    pub fn rewrite_non_historical_pins(
        &mut self,
        mut rewrite: impl FnMut(&mut Reference) -> bool,
    ) -> bool {
        let mut changed = false;
        for (relation, targets) in &mut self.relations {
            if relation.is_historical() || *relation == Relation::PartOf {
                continue;
            }
            for target in targets {
                changed |= rewrite(target);
            }
        }
        for claim in &mut self.claims {
            for target in &mut claim.supported_by {
                changed |= rewrite(target);
            }
        }
        if let Some(acceptance) = &mut self.acceptance {
            for check in &mut acceptance.checks {
                for target in &mut check.verified_by {
                    changed |= rewrite(target);
                }
            }
        }
        changed
    }
}

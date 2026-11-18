//! The twelve relations, with their domains, ranges, cardinality and graph properties.

use super::kind::Kind;
use crate::diagnostics::RuleId;
use std::fmt;

use Kind as K;

/// What kinds may declare a relation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Domain {
    /// Any kind.
    Any,
    /// Only these kinds.
    Kinds(&'static [Kind]),
}

impl Domain {
    /// Whether a kind may declare the relation.
    #[must_use]
    pub fn accepts(self, kind: Kind) -> bool {
        match self {
            Self::Any => true,
            Self::Kinds(ks) => ks.contains(&kind),
        }
    }
}

/// What kinds a relation may target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    /// Any kind.
    Any,
    /// The same kind as the source. Supersession replaces like with like.
    SameKind,
    /// Only these kinds.
    Kinds(&'static [Kind]),
}

impl Range {
    /// Whether `target` is an acceptable target for a relation declared by `source`.
    #[must_use]
    pub fn accepts(self, source: Kind, target: Kind) -> bool {
        match self {
            Self::Any => true,
            // D-046: a retired kind may be superseded by the kind it folds into, which
            // is how a record leaves a retired kind without rewriting sealed history.
            Self::SameKind => source == target || target.folds_into() == Some(source),
            Self::Kinds(ks) => ks.contains(&target),
        }
    }
}

/// How many targets a relation may have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cardinality {
    /// At most one target.
    One,
    /// Any number of targets.
    Many,
}

/// The twelve relations. Each carries a mechanical consequence; see `docs/02` §7.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Relation {
    /// The source's standing rests on the target.
    SupportedBy,
    /// If the target goes, the source goes.
    DependsOn,
    /// Puts the target into `superseded`.
    Supersedes,
    /// Symmetric; must be dispositioned, and is always surfaced.
    Contradicts,
    /// Ties intended change to what motivates it.
    Implements,
    /// Answers a question.
    Resolves,
    /// Provenance between records.
    DerivedFrom,
    /// Containment. Single-parent, and pinned for plan revisions.
    PartOf,
    /// Hard ordering.
    After,
    /// A live blocker justifies `blocked`.
    Blocks,
    /// Satisfies acceptance. Runs one way only (D-016).
    VerifiedBy,
    /// Designates the authoritative plan for a milestone or track.
    PlanOfRecord,
}

/// Every kind except `evidence`.
const ALL_BUT_EVIDENCE: &[Kind] = &[
    K::Term,
    K::Requirement,
    K::Policy,
    K::Constraint,
    K::Decision,
    K::Observation,
    K::Assessment,
    K::Milestone,
    K::Work,
    K::Track,
    K::Question,
];

/// Every kind except `question`.
const ALL_BUT_QUESTION: &[Kind] = &[
    K::Term,
    K::Requirement,
    K::Policy,
    K::Constraint,
    K::Decision,
    K::Observation,
    K::Evidence,
    K::Assessment,
    K::Milestone,
    K::Work,
    K::Track,
];

impl Relation {
    /// Every relation, in vocabulary order.
    pub const ALL: &'static [Relation] = &[
        Relation::SupportedBy,
        Relation::DependsOn,
        Relation::Supersedes,
        Relation::Contradicts,
        Relation::Implements,
        Relation::Resolves,
        Relation::DerivedFrom,
        Relation::PartOf,
        Relation::After,
        Relation::Blocks,
        Relation::VerifiedBy,
        Relation::PlanOfRecord,
    ];

    /// The slot name, used verbatim in source (D-012).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::SupportedBy => "supported_by",
            Self::DependsOn => "depends_on",
            Self::Supersedes => "supersedes",
            Self::Contradicts => "contradicts",
            Self::Implements => "implements",
            Self::Resolves => "resolves",
            Self::DerivedFrom => "derived_from",
            Self::PartOf => "part_of",
            Self::After => "after",
            Self::Blocks => "blocks",
            Self::VerifiedBy => "verified_by",
            Self::PlanOfRecord => "plan_of_record",
        }
    }

    /// Looks up a relation by slot name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|r| r.name() == name)
    }

    /// Which kinds may declare it.
    ///
    /// Note that `verified_by` is also declared by `check` blocks, which are not a kind;
    /// that case is carried by [`super::Check::verified_by`] rather than by this table.
    #[must_use]
    pub const fn domain(self) -> Domain {
        match self {
            Self::SupportedBy => Domain::Kinds(ALL_BUT_EVIDENCE),
            Self::DependsOn => Domain::Kinds(ALL_BUT_EVIDENCE),
            Self::Supersedes | Self::Contradicts | Self::DerivedFrom => Domain::Any,
            Self::Implements => Domain::Kinds(&[K::Work, K::Decision]),
            Self::Resolves => Domain::Kinds(&[K::Decision, K::Observation, K::Evidence, K::Work]),
            Self::PartOf => Domain::Kinds(&[
                K::Work,
                K::Milestone,
                K::Requirement,
                K::Policy,
                K::Question,
            ]),
            Self::After => Domain::Kinds(&[K::Milestone, K::Work]),
            Self::Blocks => Domain::Kinds(&[
                K::Question,
                K::Work,
                K::Observation,
                K::Constraint,
                K::Policy,
            ]),
            Self::VerifiedBy => Domain::Kinds(&[
                K::Milestone,
                K::Work,
                K::Requirement,
                K::Policy,
                K::Assessment,
                K::Observation,
            ]),
            Self::PlanOfRecord => Domain::Kinds(&[K::Work]),
        }
    }

    /// What kinds it may target.
    #[must_use]
    pub const fn range(self) -> Range {
        match self {
            Self::SupportedBy => Range::Kinds(&[K::Observation, K::Evidence, K::Assessment]),
            Self::DependsOn => Range::Kinds(ALL_BUT_QUESTION),
            Self::Supersedes => Range::SameKind,
            Self::Contradicts | Self::DerivedFrom => Range::Any,
            Self::Implements => {
                Range::Kinds(&[K::Requirement, K::Policy, K::Constraint, K::Decision])
            }
            Self::Resolves => Range::Kinds(&[K::Question]),
            Self::PartOf => Range::Kinds(&[K::Milestone, K::Track, K::Work]),
            Self::After => Range::Kinds(&[K::Milestone, K::Work]),
            Self::Blocks => Range::Kinds(&[K::Milestone, K::Work, K::Decision]),
            Self::VerifiedBy => Range::Kinds(&[K::Evidence]),
            Self::PlanOfRecord => Range::Kinds(&[K::Milestone, K::Track]),
        }
    }

    /// How many targets are permitted.
    #[must_use]
    pub const fn cardinality(self) -> Cardinality {
        match self {
            Self::PartOf | Self::PlanOfRecord => Cardinality::One,
            _ => Cardinality::Many,
        }
    }

    /// Whether the relation's graph must be acyclic.
    ///
    /// `contradicts` is the exception: a contradiction cycle is what you want reported,
    /// not rejected.
    #[must_use]
    pub const fn acyclic(self) -> bool {
        !matches!(self, Self::Contradicts)
    }

    /// Whether the relation is symmetric (`contradicts` alone).
    #[must_use]
    pub const fn symmetric(self) -> bool {
        matches!(self, Self::Contradicts)
    }

    /// Whether staleness propagates from target to source along it (D-024).
    #[must_use]
    pub const fn propagates_staleness(self) -> bool {
        matches!(
            self,
            Self::SupportedBy | Self::DependsOn | Self::DerivedFrom | Self::VerifiedBy
        )
    }

    /// Whether it may point at a terminal record (D-004 §5, `docs/04` §5).
    ///
    /// The three historical relations exist to point backwards.
    #[must_use]
    pub const fn is_historical(self) -> bool {
        matches!(
            self,
            Self::Supersedes | Self::Contradicts | Self::DerivedFrom
        )
    }

    /// The rule that enforces the relation's shape.
    #[must_use]
    pub const fn enforced_by(self) -> RuleId {
        match self {
            Self::SupportedBy => RuleId(5),
            Self::DependsOn | Self::Implements | Self::PartOf | Self::Blocks => RuleId(15),
            Self::Supersedes => RuleId(14),
            Self::Contradicts => RuleId(23),
            Self::Resolves => RuleId(11),
            Self::DerivedFrom => RuleId(15),
            Self::After => RuleId(16),
            Self::VerifiedBy => RuleId(20),
            Self::PlanOfRecord => RuleId(18),
        }
    }
}

impl fmt::Display for Relation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

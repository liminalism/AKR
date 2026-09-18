//! The twelve kinds, the four classes, and the kind-specific content slots.

use super::state::State;
use std::fmt;

/// The four classes. Lifecycles, rules and context ordering are defined per class, not
/// per kind (D-002).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Class {
    /// States what ought to be true; binds future work.
    Normative,
    /// States what was found to be true, at a stated point in history.
    Empirical,
    /// States what is intended, in what order, and when it is done.
    Planning,
    /// States what is not yet known.
    Inquiry,
}

impl Class {
    /// Every class.
    pub const ALL: &'static [Class] = &[
        Class::Normative,
        Class::Empirical,
        Class::Planning,
        Class::Inquiry,
    ];

    /// The name used in `spec/tables/vocabulary.json`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Normative => "normative",
            Self::Empirical => "empirical",
            Self::Planning => "planning",
            Self::Inquiry => "inquiry",
        }
    }

    /// Whether `scope` is required on this class (true for normative only).
    #[must_use]
    pub const fn scope_required(self) -> bool {
        matches!(self, Self::Normative)
    }

    /// Whether `topic` may appear (true for normative only, D-004b).
    #[must_use]
    pub const fn topic_allowed(self) -> bool {
        matches!(self, Self::Normative)
    }

    /// The kinds in this class.
    #[must_use]
    pub fn kinds(self) -> Vec<Kind> {
        Kind::ALL
            .iter()
            .copied()
            .filter(|k| k.class() == self)
            .collect()
    }
}

impl fmt::Display for Class {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The thirteen record kinds (D-001, extended by D-027). There is no `plan` kind and no
/// `goal` kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    /// Fixes the project's meaning for a word.
    Term,
    /// Something the delivered system must do or be.
    Requirement,
    /// A standing rule about how the project works.
    Policy,
    /// A limit the project must respect but did not choose.
    Constraint,
    /// A choice made between alternatives.
    Decision,
    /// What was found true of the system at a specific commit.
    Observation,
    /// The outcome of a check that was actually run.
    Evidence,
    /// A judgement drawn from observations.
    Assessment,
    /// A small friction hit while working, logged in the moment (D-027).
    Papercut,
    /// A named point at which defined acceptance checks pass.
    Milestone,
    /// A unit of intended change.
    Work,
    /// Standing work no milestone contains.
    Track,
    /// An open matter that blocks or endangers something.
    Question,
}

impl Kind {
    /// Every kind, in vocabulary order.
    pub const ALL: &'static [Kind] = &[
        Kind::Term,
        Kind::Requirement,
        Kind::Policy,
        Kind::Constraint,
        Kind::Decision,
        Kind::Observation,
        Kind::Evidence,
        Kind::Assessment,
        Kind::Papercut,
        Kind::Milestone,
        Kind::Work,
        Kind::Track,
        Kind::Question,
    ];

    /// The name used in source text and in the vocabulary.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Term => "term",
            Self::Requirement => "requirement",
            Self::Policy => "policy",
            Self::Constraint => "constraint",
            Self::Decision => "decision",
            Self::Observation => "observation",
            Self::Evidence => "evidence",
            Self::Assessment => "assessment",
            Self::Papercut => "papercut",
            Self::Milestone => "milestone",
            Self::Work => "work",
            Self::Track => "track",
            Self::Question => "question",
        }
    }

    /// Looks up a kind by name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|k| k.name() == name)
    }

    /// The class this kind belongs to.
    #[must_use]
    pub const fn class(self) -> Class {
        match self {
            Self::Term | Self::Requirement | Self::Policy | Self::Constraint | Self::Decision => {
                Class::Normative
            }
            Self::Observation | Self::Evidence | Self::Assessment | Self::Papercut => {
                Class::Empirical
            }
            Self::Milestone | Self::Work | Self::Track => Class::Planning,
            Self::Question => Class::Inquiry,
        }
    }

    /// Whether `state` is legal for this kind (V-007).
    #[must_use]
    pub fn allows_state(self, state: State) -> bool {
        self.class().states().contains(&state)
    }

    /// The kind's content slots, in canonical order (D-012 step 5).
    #[must_use]
    pub const fn content_slots(self) -> &'static [ContentSlotSpec] {
        match self {
            Self::Term => TERM_SLOTS,
            Self::Requirement => REQUIREMENT_SLOTS,
            Self::Policy => POLICY_SLOTS,
            Self::Constraint => CONSTRAINT_SLOTS,
            Self::Decision => DECISION_SLOTS,
            Self::Observation => OBSERVATION_SLOTS,
            Self::Evidence => EVIDENCE_SLOTS,
            Self::Assessment => ASSESSMENT_SLOTS,
            Self::Papercut => PAPERCUT_SLOTS,
            Self::Milestone | Self::Work => PLAN_SLOTS,
            Self::Track => TRACK_SLOTS,
            Self::Question => QUESTION_SLOTS,
        }
    }

    /// The permitted values for an enum-valued content slot on this kind.
    ///
    /// `None` means that the slot is not an enum belonging to this kind. Keeping these
    /// sets beside the kind's content-slot table gives lowering one typed source for the
    /// constraints declared in `spec/tables/vocabulary.json`.
    #[must_use]
    pub const fn content_enum_values(self, slot: ContentSlot) -> Option<&'static [&'static str]> {
        match (self, slot) {
            (Self::Observation, ContentSlot::Method) => Some(OBSERVATION_METHOD_VALUES),
            (Self::Evidence, ContentSlot::Result) => Some(EVIDENCE_RESULT_VALUES),
            (Self::Evidence, ContentSlot::Method) => Some(EVIDENCE_METHOD_VALUES),
            (Self::Assessment, ContentSlot::Confidence) => Some(ASSESSMENT_CONFIDENCE_VALUES),
            _ => None,
        }
    }

    /// Whether an `acceptance` block is required (milestones only).
    #[must_use]
    pub const fn requires_acceptance(self) -> bool {
        matches!(self, Self::Milestone)
    }

    /// Whether an `acceptance` block is permitted (milestones and work).
    #[must_use]
    pub const fn allows_acceptance(self) -> bool {
        matches!(self, Self::Milestone | Self::Work)
    }

    /// Whether `disposition` blocks are permitted (planning kinds only).
    #[must_use]
    pub const fn allows_disposition(self) -> bool {
        matches!(self.class(), Class::Planning)
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

const fn req(slot: ContentSlot) -> ContentSlotSpec {
    ContentSlotSpec {
        slot,
        required: true,
    }
}

const fn opt(slot: ContentSlot) -> ContentSlotSpec {
    ContentSlotSpec {
        slot,
        required: false,
    }
}

use ContentSlot as C;

const TERM_SLOTS: &[ContentSlotSpec] = &[req(C::Definition), opt(C::Aliases)];
const REQUIREMENT_SLOTS: &[ContentSlotSpec] = &[req(C::Statement), opt(C::Rationale)];
const POLICY_SLOTS: &[ContentSlotSpec] = &[req(C::Rule), opt(C::Rationale), opt(C::Exceptions)];
const CONSTRAINT_SLOTS: &[ContentSlotSpec] =
    &[req(C::Statement), opt(C::Measure), opt(C::Rationale)];
const DECISION_SLOTS: &[ContentSlotSpec] =
    &[req(C::Decision), opt(C::Context), opt(C::Consequences)];
const OBSERVATION_SLOTS: &[ContentSlotSpec] = &[
    req(C::Statement),
    req(C::ObservedAt),
    opt(C::Method),
    opt(C::Watches),
    opt(C::ReviewAfter),
];
const EVIDENCE_SLOTS: &[ContentSlotSpec] = &[
    req(C::Result),
    req(C::Method),
    req(C::ObservedAt),
    opt(C::Command),
    opt(C::Artifact),
    opt(C::Summary),
];
const ASSESSMENT_SLOTS: &[ContentSlotSpec] = &[req(C::Statement), opt(C::Confidence), opt(C::AsOf)];
// `observation` was accepted by released AKR 0.1 builds and occurs in sealed sister
// ledgers. Revisions are immutable and every write validates the complete history, so
// removing it here would make those ledgers permanently unwritable. Keep it in the
// vocabulary alongside the more specific provenance methods.
const OBSERVATION_METHOD_VALUES: &[&str] = &["manual", "command", "instrumented", "observation"];
const EVIDENCE_RESULT_VALUES: &[&str] = &["pass", "fail", "inconclusive"];
const EVIDENCE_METHOD_VALUES: &[&str] = &["manual", "command", "observation"];
const ASSESSMENT_CONFIDENCE_VALUES: &[&str] = &["low", "medium", "high"];
// D-027: both slots are filled by the tooling — no `watches`, so a papercut never
// enters the review queue.
const PAPERCUT_SLOTS: &[ContentSlotSpec] = &[
    req(C::Statement),
    req(C::ObservedAt),
    opt(C::About),
    opt(C::Collated),
    opt(C::ClosedAt),
];
const PLAN_SLOTS: &[ContentSlotSpec] = &[req(C::Intent), opt(C::Target), opt(C::Note)];
const TRACK_SLOTS: &[ContentSlotSpec] = &[req(C::Intent), opt(C::Cadence), opt(C::Note)];
const QUESTION_SLOTS: &[ContentSlotSpec] = &[req(C::Question), opt(C::Resolution)];

/// One entry of a kind's content-slot table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentSlotSpec {
    /// The slot.
    pub slot: ContentSlot,
    /// Whether the kind requires it.
    pub required: bool,
}

/// Every kind-specific content slot in the vocabulary.
///
/// Common slots — `title`, `state`, `scope`, `topic`, relations, metadata — are typed
/// fields on [`super::Record`] rather than members of this enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[allow(
    missing_docs,
    reason = "each variant is the slot of the same name; see docs/02 §4"
)]
pub enum ContentSlot {
    Definition,
    Aliases,
    Statement,
    Rationale,
    Rule,
    Exceptions,
    Measure,
    Decision,
    Context,
    Consequences,
    ObservedAt,
    Method,
    Watches,
    ReviewAfter,
    Result,
    Command,
    Artifact,
    Summary,
    Confidence,
    AsOf,
    Intent,
    Target,
    Cadence,
    Question,
    Resolution,
    Note,
    About,
    Collated,
    ClosedAt,
}

impl ContentSlot {
    /// Every content slot.
    pub const ALL: &'static [ContentSlot] = &[
        Self::Definition,
        Self::Aliases,
        Self::Statement,
        Self::Rationale,
        Self::Rule,
        Self::Exceptions,
        Self::Measure,
        Self::Decision,
        Self::Context,
        Self::Consequences,
        Self::ObservedAt,
        Self::Method,
        Self::Watches,
        Self::ReviewAfter,
        Self::Result,
        Self::Command,
        Self::Artifact,
        Self::Summary,
        Self::Confidence,
        Self::AsOf,
        Self::Intent,
        Self::Target,
        Self::Cadence,
        Self::Question,
        Self::Resolution,
        Self::Note,
        Self::About,
        Self::Collated,
        Self::ClosedAt,
    ];

    /// The slot name as written in source (snake_case, D-005).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Definition => "definition",
            Self::Aliases => "aliases",
            Self::Statement => "statement",
            Self::Rationale => "rationale",
            Self::Rule => "rule",
            Self::Exceptions => "exceptions",
            Self::Measure => "measure",
            Self::Decision => "decision",
            Self::Context => "context",
            Self::Consequences => "consequences",
            Self::ObservedAt => "observed_at",
            Self::Method => "method",
            Self::Watches => "watches",
            Self::ReviewAfter => "review_after",
            Self::Result => "result",
            Self::Command => "command",
            Self::Artifact => "artifact",
            Self::Summary => "summary",
            Self::Confidence => "confidence",
            Self::AsOf => "as_of",
            Self::Intent => "intent",
            Self::Target => "target",
            Self::Cadence => "cadence",
            Self::Question => "question",
            Self::Resolution => "resolution",
            Self::Note => "note",
            Self::About => "about",
            Self::Collated => "collated",
            Self::ClosedAt => "closed_at",
        }
    }

    /// Looks up a content slot by name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|s| s.name() == name)
    }

    /// The author-facing value type used by explain output and tool schemas.
    #[must_use]
    pub const fn value_type(self) -> &'static str {
        match self {
            Self::ObservedAt | Self::AsOf => "commit (git:<40-hex>)",
            Self::ReviewAfter | Self::Target | Self::ClosedAt => "date (YYYY-MM-DD)",
            Self::Watches => "glob[]",
            Self::Exceptions => "reference[]",
            Self::Aliases | Self::Collated => "string[]",
            Self::Method | Self::Result | Self::Confidence => "enum",
            _ => "text",
        }
    }
}

impl fmt::Display for ContentSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

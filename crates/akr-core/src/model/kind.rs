//! The thirteen kinds (eight written, five retired by D-046), the four classes, and the
//! kind-specific content slots.

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
/// `goal` kind. Five are retired (D-046): they still parse, hash and render, so sealed
/// history reads as it was written, but no new key may take one. See [`Kind::is_retired`].
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
            Self::Milestone => PLAN_SLOTS,
            Self::Work => WORK_SLOTS,
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
            (Self::Assessment | Self::Observation, ContentSlot::Confidence) => {
                Some(ASSESSMENT_CONFIDENCE_VALUES)
            }
            _ => None,
        }
    }

    /// Whether the kind is retired (D-046).
    ///
    /// A retired kind remains part of the language: removing it would turn every sealed
    /// record that carries it into an unknown kind and break every reference to it. What
    /// a retired kind loses is the write surface — no new key may be created with it.
    #[must_use]
    pub const fn is_retired(self) -> bool {
        self.folds_into().is_some()
    }

    /// The kind a retired kind's records fold into, and the only kind allowed to
    /// supersede it across kinds (D-046). `None` for a kind that is still written.
    #[must_use]
    pub const fn folds_into(self) -> Option<Kind> {
        match self {
            Self::Assessment => Some(Self::Observation),
            Self::Requirement | Self::Constraint | Self::Term => Some(Self::Policy),
            Self::Track => Some(Self::Milestone),
            _ => None,
        }
    }

    /// Every kind a new key may be created with: all but the retired ones.
    #[must_use]
    pub fn written() -> Vec<Kind> {
        Self::ALL
            .iter()
            .copied()
            .filter(|kind| !kind.is_retired())
            .collect()
    }

    /// What the kind is for, as `spec/tables/vocabulary.json` states it.
    #[must_use]
    pub const fn purpose(self) -> &'static str {
        match self {
            Self::Term => "Fixes the project's meaning for a word that would otherwise drift.",
            Self::Requirement => "States something the delivered system must do or must be.",
            Self::Policy => {
                "States a standing rule, requirement or limit the project must respect (D-046)."
            }
            Self::Constraint => "States a limit the project must respect but did not choose.",
            Self::Decision => {
                "Records a choice made between alternatives, and what follows from it."
            }
            Self::Observation => {
                "Records what was found to be true of the system at a specific commit, or a judgement drawn from such findings."
            }
            Self::Evidence => "Records the outcome of a check that was actually run.",
            Self::Assessment => {
                "Records a judgement drawn from observations, distinct from the observations themselves."
            }
            Self::Papercut => {
                "Records a small friction hit while working: what was being done, what got in the way, and optionally a guess at the cause or fix (D-027)."
            }
            Self::Milestone => "A named point at which a defined set of acceptance checks passes.",
            Self::Work => {
                "A unit of intended change. A work record designated plan_of_record is a plan (D-001)."
            }
            Self::Track => "Standing, non-terminating work that milestones do not contain.",
            Self::Question => "An open matter that blocks or endangers a decision or a plan.",
        }
    }

    /// What a content slot of this kind should hold, where the vocabulary says so
    /// (D-049). One source for `akr explain` and the MCP slot schema, so the guidance an
    /// agent reads cannot drift between them.
    #[must_use]
    pub const fn slot_doc(self, slot: ContentSlot) -> Option<&'static str> {
        match (self, slot) {
            (Self::Policy, C::Rule) => Some(
                "The standing rule, requirement or limit, stated so a reader can tell whether a change breaks it.",
            ),
            (Self::Policy, C::Rationale) => {
                Some("Why the rule exists: what it protects, and what was seen without it.")
            }
            (Self::Policy, C::Measure) => {
                Some("The limit as a number or a testable condition, when there is one.")
            }
            (Self::Policy, C::Aliases) => {
                Some("Other words a reader might search for this rule, or for the word it defines.")
            }
            (Self::Decision, C::Decision) => {
                Some("The choice, in a sentence or two. Not a list of the files it changed.")
            }
            (Self::Decision, C::Context) => Some(
                "What forced the choice: the measurement, failure or ruling, and the alternatives rejected and why.",
            ),
            (Self::Decision, C::Consequences) => Some(
                "What follows: the trade-off accepted, what must not be undone, what was deliberately left out.",
            ),
            (Self::Observation, C::Statement) => Some(
                "What was found, with the numbers, and why the obvious explanation or fix is wrong when it is.",
            ),
            (Self::Observation, C::Confidence) => Some(
                "How far the finding can be trusted, when it is a judgement rather than a measurement.",
            ),
            (Self::Evidence, C::Summary) => Some(
                "What was observed, when the title does not already say it. Omit it rather than repeat the title.",
            ),
            (Self::Milestone | Self::Work, C::Intent) => Some(
                "The outcome wanted and what makes it necessary. Not a list of files or functions: the diff shows those.",
            ),
            (Self::Milestone | Self::Work | Self::Track, C::Note) => Some(
                "Free-form operator commentary. Informational only; no validation consequence. Rendered by views for records in terminal states (D-026).",
            ),
            (Self::Work, C::Trace) => Some(
                "Reasoning steps, one short line each, appended as the work proceeds: why this approach, what the code as found forced, what was tried and dropped. Aim for 25 words an entry; 40 is the limit. Append with `akr trace`; never restate the diff (D-047).",
            ),
            (Self::Papercut, C::About) => Some(
                "What the friction was with, when that is not this project: a tool name such as `akr`. Absent means this project's own code or setup. `akr papercut collate --about <subject>` gathers the sisters' papercuts aimed at one subject (D-033).",
            ),
            (Self::Papercut, C::Collated) => Some(
                "Source papercut keys this collation record absorbed from sister projects; written by `akr papercut collate`, the dedup set for the next run (D-030).",
            ),
            (Self::Papercut, C::ClosedAt) => Some(
                "Date this papercut was closed; closed papercuts are excluded from future collation.",
            ),
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
// D-046: `measure` and `aliases` came over from `constraint` and `term` when those kinds
// were retired into `policy`. They sit after the original three so that every policy
// sealed before the change still formats to the bytes it was sealed with.
const POLICY_SLOTS: &[ContentSlotSpec] = &[
    req(C::Rule),
    opt(C::Rationale),
    opt(C::Exceptions),
    opt(C::Measure),
    opt(C::Aliases),
];
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
    // D-046: carried over from the retired `assessment`.
    opt(C::Confidence),
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
// D-047: `trace` is the work record's reasoning trail. Milestones do not carry one — a
// milestone is a set of checks, and the reasoning belongs to the work that meets them.
const WORK_SLOTS: &[ContentSlotSpec] =
    &[req(C::Intent), opt(C::Target), opt(C::Note), opt(C::Trace)];
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
    Trace,
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
        Self::Trace,
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
            Self::Trace => "trace",
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
            Self::Aliases | Self::Collated | Self::Trace => "string[]",
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

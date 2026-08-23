//! The validation rules, `V-001` through `V-025`.
//!
//! Each rule is a named function over a [`Ledger`](crate::model::Ledger) returning
//! diagnostics. Rules are pure: they read the ledger and allocate diagnostics, and do
//! nothing else. They are also independent — running one does not require running
//! another — which is what makes per-rule unit tests possible.
//!
//! # Staleness is not here
//!
//! Stale and at-risk records carry no code and never change an exit status (D-024). They
//! are computed in P5 and reported separately.
//!
//! # Rules that need facts P1 does not have
//!
//! V-020's descendant-commit condition needs git, and V-024 needs the lock file. Both
//! read [`LedgerFacts`](crate::model::LedgerFacts), which is empty in P1; each rule
//! documents exactly what it skips when the facts are absent, and skips rather than
//! passing vacuously.

mod rules;

use crate::diagnostics::{Code, Diagnostic, RuleId, Stage};
use crate::model::Ledger;

pub use rules::*;

/// One entry of the rule catalogue.
#[derive(Debug, Clone, Copy)]
pub struct RuleSpec {
    /// The rule identifier.
    pub id: RuleId,
    /// The rule's one-line title, matching `docs/05-validation-rules.md`.
    pub title: &'static str,
    /// The primary code it raises. A rule may raise secondary codes as well.
    pub code: Code,
    /// The stage it runs at.
    pub stage: Stage,
    /// The check itself.
    pub check: fn(&Ledger) -> Vec<Diagnostic>,
}

macro_rules! catalogue {
    ($($id:literal, $f:ident, $code:expr, $stage:expr, $title:literal;)*) => {
        /// Every rule, in identifier order.
        pub const RULES: &[RuleSpec] = &[$(RuleSpec {
            id: RuleId($id),
            title: $title,
            code: $code,
            stage: $stage,
            check: $f,
        }),*];
    };
}

use crate::diagnostics::codes as c;

catalogue! {
    1,  v001_references_resolve, c::L001, Stage::Link, "Every reference resolves";
    2,  v002_namespaces_declared, c::L004, Stage::Link, "Namespaces are declared";
    3,  v003_one_key_one_file, c::L006, Stage::Link, "One key, one file";
    4,  v004_anchors_exist, c::L012, Stage::Link, "Anchors exist, and retired anchors say so";
    5,  v005_targets_kind_correct, c::L031, Stage::Link, "Relation and slot targets are kind-correct";
    6,  v006_historical_references, c::L021, Stage::Link, "Terminal records are cited, not built on";
    7,  v007_state_legal, c::T011, Stage::Type, "The kind and state combination is legal";
    8,  v008_slots_present, c::T001, Stage::Type, "Required slots present, unknown slots rejected";
    9,  v009_observation_commit, c::T021, Stage::Type, "Observations carry observed_at";
    10, v010_evidence_slots, c::T022, Stage::Type, "Evidence carries result, method, observed_at";
    11, v011_resolved_question, c::T031, Stage::Type, "A resolved question has a resolution and a resolver";
    12, v012_one_live_head, c::R001, Stage::Resolve, "One live revision per key";
    13, v013_topic_exclusivity, c::R002, Stage::Resolve, "Normative exclusivity by topic and scope";
    14, v014_supersession_acyclic, c::R011, Stage::Resolve, "The supersession graph is acyclic";
    15, v015_structural_acyclic, c::R012, Stage::Resolve, "Structural relation graphs are acyclic";
    16, v016_after_acyclic, c::R013, Stage::Resolve, "The after graph is acyclic";
    17, v017_disposition_complete, c::R014, Stage::Resolve, "Supersession disposes of unfinished children";
    18, v018_one_plan_of_record, c::R018, Stage::Resolve, "One plan of record";
    19, v019_live_not_on_terminal, c::R021, Stage::Resolve, "Live records do not rely on invalid terminal records";
    20, v020_acceptance_satisfied, c::R022, Stage::Resolve, "Completion requires satisfied acceptance";
    21, v021_decision_cites, c::R031, Stage::Resolve, "Active decisions cite something";
    22, v022_observation_provenance, c::R032, Stage::Resolve, "Live observations have provenance";
    23, v023_contradiction_dispositioned, c::R041, Stage::Resolve, "Contradictions are dispositioned";
    24, v024_seals_match, c::R051, Stage::Resolve, "Sealed revisions match their recorded hash";
    25, v025_evidence_artifact_durable, c::T023, Stage::Type, "Evidence artefacts are cited from durable paths";
}

/// Looks up a rule by identifier.
#[must_use]
pub fn rule(id: RuleId) -> Option<&'static RuleSpec> {
    RULES.iter().find(|r| r.id == id)
}

/// Runs every rule and returns the diagnostics, deterministically ordered.
///
/// Ordering is by rule, then by code, then by subject — a total order that does not
/// depend on spans, hash iteration, or insertion order. P2 re-sorts by span once files
/// exist; until then this is what makes output stable across runs.
#[must_use]
pub fn validate_all(ledger: &Ledger) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for spec in RULES {
        let mut found = (spec.check)(ledger);
        found.sort_by_key(Diagnostic::sort_key);
        out.extend(found);
    }
    out
}

/// Runs every rule at one stage.
#[must_use]
pub fn validate_stage(ledger: &Ledger, stage: Stage) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for spec in RULES.iter().filter(|s| s.stage == stage) {
        let mut found = (spec.check)(ledger);
        found.sort_by_key(Diagnostic::sort_key);
        out.extend(found);
    }
    out
}

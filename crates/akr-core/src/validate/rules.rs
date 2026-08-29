//! `V-001`..`V-025` over the in-memory model.
//!
//! Every function here corresponds to one section of `docs/05-validation-rules.md`, and
//! the fixture that exercises it is named for the same rule in `fixtures/validate/err/`.

use crate::diagnostics::codes as c;
use crate::diagnostics::{Diagnostic, Label, RuleId, SlotRef, Subject};
use crate::model::{
    Class, ContentSlot, ContentValue, EvidenceResult, HeadError, Kind, Ledger, LogicalKey, Range,
    Record, Reference, Relation, RevisionId, ScopeTerm, SourceKind, State, scopes_overlap,
};
use std::collections::{BTreeMap, BTreeSet};

// ---------------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------------

/// Records in a deterministic order, independent of insertion order.
fn ordered(ledger: &Ledger) -> Vec<&Record> {
    let mut rs: Vec<&Record> = ledger.records().iter().collect();
    rs.sort_by(|a, b| a.id.cmp(&b.id));
    rs
}

fn subject(record: &Record) -> Subject {
    Subject::Revision(record.id.clone())
}

fn slot_subject(record: &Record, slot: SlotRef) -> Subject {
    Subject::Slot(record.id.clone(), slot)
}

fn rel_subject(record: &Record, relation: Option<Relation>) -> Subject {
    match relation {
        Some(r) => slot_subject(record, SlotRef::Relation(r)),
        None => subject(record),
    }
}

/// Finds one cycle in a directed graph, deterministically.
///
/// Nodes are visited in sorted order and edges are sorted, so a graph with several
/// cycles always reports the same one.
fn find_cycle<T, F>(nodes: &[T], edges: F) -> Option<Vec<T>>
where
    T: Ord + Clone,
    F: Fn(&T) -> Vec<T>,
{
    fn sorted_edges<T: Ord + Clone, F: Fn(&T) -> Vec<T>>(edges: &F, node: &T) -> Vec<T> {
        let mut e = edges(node);
        e.sort();
        e.dedup();
        e
    }

    let mut sorted: Vec<T> = nodes.to_vec();
    sorted.sort();
    sorted.dedup();
    let mut done: BTreeSet<T> = BTreeSet::new();

    for start in &sorted {
        if done.contains(start) {
            continue;
        }
        let mut stack: Vec<(T, Vec<T>)> = vec![(start.clone(), sorted_edges(&edges, start))];
        let mut on_path: Vec<T> = vec![start.clone()];
        let mut in_path: BTreeSet<T> = [start.clone()].into();

        while let Some((_, pending)) = stack.last_mut() {
            if let Some(next) = pending.pop() {
                if in_path.contains(&next) {
                    let at = on_path.iter().position(|n| *n == next).unwrap_or(0);
                    let mut cycle = on_path[at..].to_vec();
                    cycle.push(next);
                    return Some(cycle);
                }
                if done.contains(&next) {
                    continue;
                }
                let e = sorted_edges(&edges, &next);
                in_path.insert(next.clone());
                on_path.push(next.clone());
                stack.push((next, e));
            } else {
                let (node, _) = stack.pop().expect("stack is non-empty");
                on_path.pop();
                in_path.remove(&node);
                done.insert(node);
            }
        }
    }
    None
}

fn cycle_text<T: std::fmt::Display>(cycle: &[T]) -> String {
    cycle
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" -> ")
}

/// The record a reference names, ignoring resolution failures (V-001 reports those).
fn target_of<'a>(ledger: &'a Ledger, reference: &Reference) -> Option<&'a Record> {
    ledger.resolve(reference).ok().flatten()
}

/// Relations that assert a live dependency, for V-019.
const LIVENESS_RELATIONS: &[Relation] = &[
    Relation::DependsOn,
    Relation::Implements,
    Relation::PlanOfRecord,
    Relation::SupportedBy,
];

/// Relations whose graphs V-015 requires to be acyclic.
const STRUCTURAL_RELATIONS: &[Relation] = &[
    Relation::DependsOn,
    Relation::DerivedFrom,
    Relation::PartOf,
    Relation::Implements,
    Relation::Blocks,
];

// ---------------------------------------------------------------------------------
// V-001
// ---------------------------------------------------------------------------------

/// V-001: every reference resolves to a declared key and an existing revision.
///
/// Raises `AKR-L001` for an unknown key, `AKR-L003` for a missing revision, `AKR-L002`
/// when a key has no single head, and `AKR-L041` for a duplicated revision identifier.
#[must_use]
pub fn v001_references_resolve(ledger: &Ledger) -> Vec<Diagnostic> {
    const RULE: RuleId = RuleId(1);
    let mut out = Vec::new();

    let mut seen: BTreeSet<&RevisionId> = BTreeSet::new();
    for record in ordered(ledger) {
        if !seen.insert(&record.id) {
            out.push(Diagnostic::error(
                c::L041,
                RULE,
                subject(record),
                format!("{} is defined twice", record.id),
            ));
        }
    }

    for record in ordered(ledger) {
        for (relation, reference) in record.references() {
            match ledger.resolve(reference) {
                Ok(Some(_)) => {}
                Ok(None) => out.push(
                    Diagnostic::error(
                        c::L003,
                        RULE,
                        rel_subject(record, relation),
                        format!(
                            "{} has no revision {}",
                            reference.key,
                            reference.revision.unwrap_or(0)
                        ),
                    )
                    .help("pin an existing revision, or float the reference"),
                ),
                Err(HeadError::UnknownKey(key)) => out.push(
                    Diagnostic::error(
                        c::L001,
                        RULE,
                        rel_subject(record, relation),
                        format!("no record with key {key}"),
                    )
                    .note(Label::with_message(subject(record), "referenced here")),
                ),
                Err(HeadError::AmbiguousChainEnd(key, revisions)) => out.push(
                    Diagnostic::error(
                        c::L002,
                        RULE,
                        Subject::Key(key.clone()),
                        HeadError::AmbiguousChainEnd(key.clone(), revisions.clone()).to_string(),
                    )
                    .help(
                        "name those revisions in the successor's `supersedes` list; \
                         `akr revise` adds missing same-key edges when it creates the \
                         next head",
                    ),
                ),
                // Two live revisions is V-012's diagnostic, not this rule's.
                Err(HeadError::MultipleLive(..)) => {}
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------
// V-002
// ---------------------------------------------------------------------------------

/// V-002: every key's first segment is declared in `project.akr`.
///
/// A ledger with no declared namespaces is skipped: turning a missing project file into
/// one diagnostic per record helps nobody.
#[must_use]
pub fn v002_namespaces_declared(ledger: &Ledger) -> Vec<Diagnostic> {
    const RULE: RuleId = RuleId(2);
    if ledger.project.namespaces.is_empty() {
        return Vec::new();
    }
    ordered(ledger)
        .into_iter()
        .filter(|r| !ledger.project.namespaces.contains(r.id.key.namespace()))
        .map(|r| {
            Diagnostic::error(
                c::L004,
                RULE,
                subject(r),
                format!(
                    "namespace `{}` is not declared in project.akr",
                    r.id.key.namespace()
                ),
            )
            .help("correct the namespace, or declare it if it is genuinely new")
        })
        .collect()
}

// ---------------------------------------------------------------------------------
// V-003
// ---------------------------------------------------------------------------------

/// V-003: every revision of a key lives in one file.
///
/// Records with no `file` are skipped: a ledger built in code has no files, and this is
/// a review-ergonomics rule rather than a semantic one.
#[must_use]
pub fn v003_one_key_one_file(ledger: &Ledger) -> Vec<Diagnostic> {
    const RULE: RuleId = RuleId(3);
    let mut out = Vec::new();
    for key in ledger.keys() {
        let files: BTreeSet<&String> = ledger
            .revisions_of(key)
            .iter()
            .filter_map(|r| r.file.as_ref())
            .collect();
        if files.len() > 1 {
            let list: Vec<&str> = files.iter().map(|f| f.as_str()).collect();
            out.push(Diagnostic::error(
                c::L006,
                RULE,
                Subject::Key(key.clone()),
                format!("revisions of {key} appear in {}", list.join(" and ")),
            ));
        }
    }
    out
}

// ---------------------------------------------------------------------------------
// V-004
// ---------------------------------------------------------------------------------

/// V-004: claim and check anchors resolve; a retired anchor gets its own diagnostic.
#[must_use]
pub fn v004_anchors_exist(ledger: &Ledger) -> Vec<Diagnostic> {
    const RULE: RuleId = RuleId(4);
    let mut out = Vec::new();
    for record in ordered(ledger) {
        for (relation, reference) in record.references() {
            let Some(anchor) = &reference.anchor else {
                continue;
            };
            let Some(target) = target_of(ledger, reference) else {
                continue;
            };
            if target.has_anchor(anchor) {
                continue;
            }
            if target.retired_claims.contains(anchor) {
                out.push(
                    Diagnostic::error(
                        c::L012,
                        RULE,
                        rel_subject(record, relation),
                        format!(
                            "claim `{anchor}` was retired at revision {}",
                            target.id.revision
                        ),
                    )
                    .note(Label::with_message(
                        Subject::Revision(target.id.clone()),
                        "retired here",
                    ))
                    .help("pin a revision that had it, or cite the claim that replaced it"),
                );
            } else {
                out.push(Diagnostic::error(
                    c::L011,
                    RULE,
                    rel_subject(record, relation),
                    format!("{} has no claim or check `{anchor}`", target.id),
                ));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------
// V-005
// ---------------------------------------------------------------------------------

/// V-005: relations respect their domain and range; kind-restricted slots respect theirs.
///
/// Raises `AKR-L031` (range), `AKR-L032` (domain) and `AKR-L033` (`exceptions`, `into`
/// and `ref` scope terms).
#[must_use]
pub fn v005_targets_kind_correct(ledger: &Ledger) -> Vec<Diagnostic> {
    const RULE: RuleId = RuleId(5);
    const SCOPE_KINDS: &[Kind] = &[Kind::Milestone, Kind::Track, Kind::Constraint];
    const EXCEPTION_KINDS: &[Kind] = &[Kind::Milestone, Kind::Track, Kind::Work, Kind::Constraint];
    const INTO_KINDS: &[Kind] = &[Kind::Milestone, Kind::Track, Kind::Work];
    let mut out = Vec::new();

    for record in ordered(ledger) {
        for (relation, targets) in &record.relations {
            if !relation.domain().accepts(record.kind) {
                let legal = Relation::ALL
                    .iter()
                    .filter(|candidate| candidate.domain().accepts(record.kind))
                    .map(|candidate| candidate.name())
                    .collect::<Vec<_>>()
                    .join(", ");
                out.push(
                    Diagnostic::error(
                        c::L032,
                        RULE,
                        slot_subject(record, SlotRef::Relation(*relation)),
                        format!("a {} may not declare `{relation}`", record.kind),
                    )
                    .help(format!("valid {} relations: {legal}", record.kind)),
                );
            }
            for reference in targets {
                let Some(target) = target_of(ledger, reference) else {
                    continue;
                };
                if !relation.range().accepts(record.kind, target.kind) {
                    let detail = match relation.range() {
                        Range::SameKind => format!("`{relation}` replaces like with like"),
                        _ => format!("`{relation}` may not target a {}", target.kind),
                    };
                    let alternatives = Relation::ALL
                        .iter()
                        .filter(|candidate| {
                            candidate.domain().accepts(record.kind)
                                && candidate.range().accepts(record.kind, target.kind)
                        })
                        .map(|candidate| candidate.name())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let mut diagnostic = Diagnostic::error(
                        c::L031,
                        RULE,
                        slot_subject(record, SlotRef::Relation(*relation)),
                        detail,
                    );
                    if !alternatives.is_empty() {
                        diagnostic = diagnostic.help(format!(
                            "relations from {} that may target {}: {alternatives}",
                            record.kind, target.kind
                        ));
                    }
                    out.push(diagnostic);
                }
            }
        }

        let mut restricted: Vec<(SlotRef, &Reference, &[Kind], &str)> = Vec::new();
        for term in &record.scope {
            if let ScopeTerm::Ref(reference) = term {
                restricted.push((SlotRef::Scope, reference, SCOPE_KINDS, "a `ref` scope term"));
            }
        }
        if let Some(ContentValue::Refs(refs)) = record.get(ContentSlot::Exceptions) {
            for reference in refs {
                restricted.push((
                    SlotRef::Content(ContentSlot::Exceptions),
                    reference,
                    EXCEPTION_KINDS,
                    "`exceptions`",
                ));
            }
        }
        for disposition in &record.dispositions {
            if let Some(into) = &disposition.into {
                restricted.push((
                    SlotRef::Disposition(disposition.target.clone()),
                    into,
                    INTO_KINDS,
                    "`into`",
                ));
            }
        }
        for (slot, reference, allowed, label) in restricted {
            let Some(target) = target_of(ledger, reference) else {
                continue;
            };
            if !allowed.contains(&target.kind) {
                out.push(Diagnostic::error(
                    c::L033,
                    RULE,
                    slot_subject(record, slot),
                    format!("{label} may not reference a {}", target.kind),
                ));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------
// V-006
// ---------------------------------------------------------------------------------

/// V-006: a **pinned** reference from a live record to a terminal record is an error
/// unless the slot is historical.
///
/// `after`, `blocks` and `verified_by` are excluded: a completed predecessor, a lifted
/// blocker and a recorded test run are all normal. `part_of` is excluded for completed
/// targets, and permitted for a superseded plan revision when the referring record is
/// dispositioned by whatever superseded it (`docs/04` §5.1) — which is what makes V-017
/// bite rather than being vacuous.
///
/// Floating references whose head has become terminal are V-019's business.
#[must_use]
pub fn v006_historical_references(ledger: &Ledger) -> Vec<Diagnostic> {
    const RULE: RuleId = RuleId(6);
    let mut out = Vec::new();

    for record in ordered(ledger) {
        if !record.is_live() {
            continue;
        }
        for (relation, reference) in record.references() {
            let Some(relation) = relation else { continue };
            if !reference.is_pinned() || relation.is_historical() {
                continue;
            }
            if matches!(
                relation,
                Relation::After | Relation::Blocks | Relation::VerifiedBy
            ) {
                continue;
            }
            let Some(target) = target_of(ledger, reference) else {
                continue;
            };
            if !target.is_terminal() {
                continue;
            }
            // Completion satisfies a prerequisite; abandonment and supersession still
            // invalidate it. Keeping this edge preserves the milestone chain and avoids
            // a bookkeeping revision merely to forget a successfully met dependency.
            if relation == Relation::DependsOn && target.state == State::Completed {
                continue;
            }
            if relation == Relation::PartOf {
                if target.state == State::Completed {
                    continue;
                }
                if target.state == State::Superseded
                    && is_dispositioned_by_successor(ledger, &record.id.key, &target.id)
                {
                    continue;
                }
            }
            out.push(
                Diagnostic::error(
                    c::L021,
                    RULE,
                    slot_subject(record, SlotRef::Relation(relation)),
                    format!(
                        "slot `{relation}` may not reference {}, which is {}",
                        target.id, target.state
                    ),
                )
                .help(pinned_reference_help(ledger, target)),
            );
        }
    }
    out
}

/// What to do about a pinned reference to a terminal revision.
///
/// The common case has a shape worth naming outright: the reference was written pinned to
/// the head of the day, and then the target was revised — so writing the answer to a
/// question, and pinning the question revision the answer was written against, invalidates
/// the answer in the same transition that records it. It reads as circular, and the way
/// out is not obvious from "point at the live head", because at the moment the author
/// reads that help there is no live head yet
/// (`akr.papercut.collated-19-papercuts-from-evidence-intake`, from
/// Evidence-intake-project). An unpinned `@key` follows the head through the revision and
/// needs no second edit.
fn pinned_reference_help(ledger: &Ledger, target: &Record) -> String {
    let successor = ledger
        .revisions_of(&target.id.key)
        .into_iter()
        .find(|revision| revision.is_live());
    match successor {
        Some(live) if target.state == State::Superseded => format!(
            "drop the revision and write `@{}`, which follows the head — {} superseded \
             this one. Pin a revision only to cite that revision for good, and then use \
             a historical relation",
            target.id.key, live.id
        ),
        _ => "point at the live head, or use a historical relation and pin it".to_owned(),
    }
}

/// Whether `child` is dispositioned by a record that supersedes `superseded`.
fn is_dispositioned_by_successor(
    ledger: &Ledger,
    child: &LogicalKey,
    superseded: &RevisionId,
) -> bool {
    ledger.records().iter().any(|candidate| {
        candidate
            .targets(Relation::Supersedes)
            .iter()
            .any(|t| t.key == superseded.key && t.revision == Some(superseded.revision))
            && candidate
                .dispositions
                .iter()
                .any(|d| &d.target.key == child)
    })
}

// ---------------------------------------------------------------------------------
// V-007
// ---------------------------------------------------------------------------------

/// V-007: a record's state belongs to its kind's class lifecycle.
#[must_use]
pub fn v007_state_legal(ledger: &Ledger) -> Vec<Diagnostic> {
    const RULE: RuleId = RuleId(7);
    ordered(ledger)
        .into_iter()
        .filter(|r| !r.kind.allows_state(r.state))
        .map(|r| {
            let legal: Vec<&str> = r.kind.class().states().iter().map(|s| s.name()).collect();
            Diagnostic::error(
                c::T011,
                RULE,
                slot_subject(r, SlotRef::State),
                format!(
                    "{} is not a valid state for {} ({}); expected one of {}",
                    r.state,
                    r.kind,
                    r.kind.class(),
                    legal.join(", ")
                ),
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------------
// V-008
// ---------------------------------------------------------------------------------

/// V-008: required slots and blocks are present, and nothing appears that the kind does
/// not define.
///
/// Raises `AKR-T001` (missing slot), `AKR-T002` (slot not valid for the kind),
/// `AKR-T005` (block not permitted), `AKR-T006` (missing required block) and
/// `AKR-T034` (`topic` on a non-normative kind).
///
/// # Slots a more specific rule owns
///
/// V-008 does **not** report a missing `observed_at` on an observation, or a missing
/// `result`, `method` or `observed_at` on evidence. V-009 and V-010 own those slots and
/// say something more useful about them. One fault raises one code: a reader who sees
/// both `AKR-T001` and `AKR-T021` for a single missing commit learns nothing from the
/// first, and a fixture that expects both is asserting an implementation detail rather
/// than a rule.
#[must_use]
pub fn v008_slots_present(ledger: &Ledger) -> Vec<Diagnostic> {
    const RULE: RuleId = RuleId(8);
    let mut out = Vec::new();

    for record in ordered(ledger) {
        let allowed: BTreeSet<ContentSlot> =
            record.kind.content_slots().iter().map(|s| s.slot).collect();

        for spec in record.kind.content_slots() {
            if owned_by_specific_rule(record.kind, spec.slot) {
                continue;
            }
            if spec.required && !record.content.contains_key(&spec.slot) {
                out.push(Diagnostic::error(
                    c::T001,
                    RULE,
                    subject(record),
                    format!("{} requires slot `{}`", record.kind, spec.slot),
                ));
            }
        }
        for slot in record.content.keys() {
            if !allowed.contains(slot) {
                let legal = record
                    .kind
                    .content_slots()
                    .iter()
                    .map(|spec| format!("{} ({})", spec.slot, spec.slot.value_type()))
                    .collect::<Vec<_>>()
                    .join(", ");
                out.push(
                    Diagnostic::error(
                        c::T002,
                        RULE,
                        slot_subject(record, SlotRef::Content(*slot)),
                        format!("{} has no slot `{slot}`", record.kind),
                    )
                    .help(format!("valid {} slots: {legal}", record.kind)),
                );
            }
        }
        if record.title.is_empty() {
            out.push(Diagnostic::error(
                c::T001,
                RULE,
                subject(record),
                format!("{} requires slot `title`", record.kind),
            ));
        }
        if record.kind.class().scope_required() && record.scope.is_empty() {
            out.push(Diagnostic::error(
                c::T001,
                RULE,
                subject(record),
                format!("{} requires slot `scope`", record.kind),
            ));
        }
        if record.topic.is_some() && !record.kind.class().topic_allowed() {
            out.push(Diagnostic::error(
                c::T034,
                RULE,
                slot_subject(record, SlotRef::Topic),
                "`topic` applies only to normative kinds".to_owned(),
            ));
        }
        if record.acceptance.is_some() && !record.kind.allows_acceptance() {
            out.push(Diagnostic::error(
                c::T005,
                RULE,
                slot_subject(record, SlotRef::Acceptance),
                format!("{} may not contain an `acceptance` block", record.kind),
            ));
        }
        if record.kind.requires_acceptance() && record.acceptance.is_none() {
            out.push(Diagnostic::error(
                c::T006,
                RULE,
                subject(record),
                format!("{} requires an `acceptance` block", record.kind),
            ));
        }
        if !record.dispositions.is_empty() && !record.kind.allows_disposition() {
            out.push(Diagnostic::error(
                c::T005,
                RULE,
                subject(record),
                format!("{} may not contain a `disposition` block", record.kind),
            ));
        }
    }
    out
}

/// Whether a required slot belongs to a rule more specific than V-008.
///
/// The overlap is real and was found by cross-validating the fixture corpus: without
/// this, one missing `observed_at` produced both `AKR-T001` and `AKR-T021`.
fn owned_by_specific_rule(kind: Kind, slot: ContentSlot) -> bool {
    match kind {
        // V-009 owns `observed_at`.
        Kind::Observation => slot == ContentSlot::ObservedAt,
        // V-010 owns all three of evidence's required slots.
        Kind::Evidence => {
            matches!(
                slot,
                ContentSlot::Result | ContentSlot::Method | ContentSlot::ObservedAt
            )
        }
        // V-011 owns `resolution`, which the vocabulary marks conditional rather than
        // required, so V-008 would not have reported it in any case.
        Kind::Question => slot == ContentSlot::Resolution,
        _ => false,
    }
}

// ---------------------------------------------------------------------------------
// V-009 / V-010 / V-011
// ---------------------------------------------------------------------------------

/// V-009: every observation carries `observed_at`.
#[must_use]
pub fn v009_observation_commit(ledger: &Ledger) -> Vec<Diagnostic> {
    const RULE: RuleId = RuleId(9);
    ordered(ledger)
        .into_iter()
        .filter(|r| r.kind == Kind::Observation && r.get(ContentSlot::ObservedAt).is_none())
        .map(|r| {
            Diagnostic::error(
                c::T021,
                RULE,
                subject(r),
                "observation requires `observed_at`",
            )
            .help("an observation without a commit is a rumour, and can never go stale")
        })
        .collect()
}

/// V-010: every evidence record carries `result`, `method` and `observed_at`.
#[must_use]
pub fn v010_evidence_slots(ledger: &Ledger) -> Vec<Diagnostic> {
    const RULE: RuleId = RuleId(10);
    let mut out = Vec::new();
    for record in ordered(ledger)
        .into_iter()
        .filter(|r| r.kind == Kind::Evidence)
    {
        for slot in [
            ContentSlot::Result,
            ContentSlot::Method,
            ContentSlot::ObservedAt,
        ] {
            if record.get(slot).is_none() {
                out.push(Diagnostic::error(
                    c::T022,
                    RULE,
                    subject(record),
                    format!("evidence requires `{slot}`"),
                ));
            }
        }
    }
    out
}

/// V-011: a `resolved` question has a `resolution` and a live `resolves` edge.
#[must_use]
pub fn v011_resolved_question(ledger: &Ledger) -> Vec<Diagnostic> {
    const RULE: RuleId = RuleId(11);
    let mut out = Vec::new();
    for record in ordered(ledger)
        .into_iter()
        .filter(|r| r.kind == Kind::Question && r.state == State::Resolved)
    {
        if record.get(ContentSlot::Resolution).is_none() {
            out.push(Diagnostic::error(
                c::T031,
                RULE,
                subject(record),
                "question in state `resolved` requires a `resolution` slot",
            ));
        }
        let resolved_by_something = ledger.records().iter().any(|other| {
            other.is_live()
                && other
                    .targets(Relation::Resolves)
                    .iter()
                    .any(|t| t.key == record.id.key)
        });
        if !resolved_by_something {
            out.push(
                Diagnostic::error(
                    c::T031,
                    RULE,
                    subject(record),
                    "nothing declares `resolves` for this question",
                )
                .help("add `resolves` to whatever answered it, or use `closed-without-resolution`"),
            );
        }
    }
    out
}

// ---------------------------------------------------------------------------------
// V-012 / V-013
// ---------------------------------------------------------------------------------

/// V-012: at most one revision of a key is live.
#[must_use]
pub fn v012_one_live_head(ledger: &Ledger) -> Vec<Diagnostic> {
    const RULE: RuleId = RuleId(12);
    let mut out = Vec::new();
    for key in ledger.keys() {
        let live: Vec<u32> = ledger
            .revisions_of(key)
            .into_iter()
            .filter(|r| r.is_live())
            .map(|r| r.id.revision)
            .collect();
        if live.len() > 1 {
            let list = live
                .iter()
                .map(|r| format!("{key}/{r}"))
                .collect::<Vec<_>>()
                .join(", ");
            out.push(
                Diagnostic::error(
                    c::R001,
                    RULE,
                    Subject::Key(key.clone()),
                    format!("{key} has {} live revisions: {list}", live.len()),
                )
                .help("supersede or withdraw all but one"),
            );
        }
    }
    out
}

/// V-013: no two live normative records share a `topic` with overlapping scope (D-004b).
#[must_use]
pub fn v013_topic_exclusivity(ledger: &Ledger) -> Vec<Diagnostic> {
    const RULE: RuleId = RuleId(13);
    let parents = ledger.part_of_index();
    let candidates: Vec<&Record> = ordered(ledger)
        .into_iter()
        .filter(|r| r.kind.class() == Class::Normative && r.is_live() && r.topic.is_some())
        .collect();

    let mut out = Vec::new();
    for (i, a) in candidates.iter().enumerate() {
        for b in candidates.iter().skip(i + 1) {
            if a.topic != b.topic || !scopes_overlap(&a.scope, &b.scope, &parents) {
                continue;
            }
            let topic = a.topic.as_ref().expect("filtered to records with a topic");
            out.push(
                Diagnostic::error(
                    c::R002,
                    RULE,
                    subject(a),
                    format!(
                        "{} and {} are both live, share topic `{topic}`, and their scopes overlap",
                        a.id, b.id
                    ),
                )
                .note(Label::with_message(subject(b), "the other record")),
            );
        }
    }
    out
}

// ---------------------------------------------------------------------------------
// V-014 / V-015 / V-016
// ---------------------------------------------------------------------------------

/// V-014: the supersession graph is acyclic, and supersession does not cross kinds.
///
/// Raises `AKR-R011` for a cycle and `AKR-R017` for a cross-kind supersession.
#[must_use]
pub fn v014_supersession_acyclic(ledger: &Ledger) -> Vec<Diagnostic> {
    const RULE: RuleId = RuleId(14);
    let mut out = Vec::new();

    for record in ordered(ledger) {
        for reference in record.targets(Relation::Supersedes) {
            if let Some(target) = target_of(ledger, reference)
                && target.kind != record.kind
            {
                out.push(Diagnostic::error(
                    c::R017,
                    RULE,
                    slot_subject(record, SlotRef::Relation(Relation::Supersedes)),
                    format!("a {} may not supersede a {}", record.kind, target.kind),
                ));
            }
        }
    }

    let nodes: Vec<RevisionId> = ordered(ledger).into_iter().map(|r| r.id.clone()).collect();
    if let Some(cycle) = find_cycle(&nodes, |id| {
        ledger.get(id).map_or_else(Vec::new, |record| {
            record
                .targets(Relation::Supersedes)
                .iter()
                .filter_map(|t| target_of(ledger, t))
                .map(|t| t.id.clone())
                .collect()
        })
    }) {
        out.push(Diagnostic::error(
            c::R011,
            RULE,
            Subject::Revision(cycle[0].clone()),
            format!("supersession cycle: {}", cycle_text(&cycle)),
        ));
    }
    out
}

/// V-015: the `depends_on`, `derived_from`, `part_of`, `implements` and `blocks` graphs
/// are each acyclic.
#[must_use]
pub fn v015_structural_acyclic(ledger: &Ledger) -> Vec<Diagnostic> {
    const RULE: RuleId = RuleId(15);
    let mut out = Vec::new();
    let nodes: Vec<LogicalKey> = ledger.keys().into_iter().cloned().collect();

    for relation in STRUCTURAL_RELATIONS {
        if let Some(cycle) = find_cycle(&nodes, |key| {
            ledger.head(key).map_or_else(
                |_| Vec::new(),
                |record| {
                    record
                        .targets(*relation)
                        .iter()
                        .map(|t| t.key.clone())
                        .filter(|k| !ledger.revisions_of(k).is_empty())
                        .collect()
                },
            )
        }) {
            out.push(Diagnostic::error(
                c::R012,
                RULE,
                Subject::Key(cycle[0].clone()),
                format!("cycle in `{relation}`: {}", cycle_text(&cycle)),
            ));
        }
    }
    out
}

/// V-016: the `after` graph is acyclic.
#[must_use]
pub fn v016_after_acyclic(ledger: &Ledger) -> Vec<Diagnostic> {
    const RULE: RuleId = RuleId(16);
    let nodes: Vec<LogicalKey> = ledger.keys().into_iter().cloned().collect();
    find_cycle(&nodes, |key| {
        ledger.head(key).map_or_else(
            |_| Vec::new(),
            |record| {
                record
                    .targets(Relation::After)
                    .iter()
                    .map(|t| t.key.clone())
                    .filter(|k| !ledger.revisions_of(k).is_empty())
                    .collect()
            },
        )
    })
    .map(|cycle| {
        vec![Diagnostic::error(
            c::R013,
            RULE,
            Subject::Key(cycle[0].clone()),
            format!("cycle in `after`: {}", cycle_text(&cycle)),
        )]
    })
    .unwrap_or_default()
}

// ---------------------------------------------------------------------------------
// V-017
// ---------------------------------------------------------------------------------

/// V-017: a superseding planning record disposes of every unfinished child of the record
/// it supersedes (D-017).
///
/// An unfinished child is a record in a live planning state whose `part_of` **pins** the
/// superseded revision. Raises `AKR-R014` (missing), `AKR-R015` (`into` required or
/// forbidden) and `AKR-R016` (dispositioning a non-child).
#[must_use]
pub fn v017_disposition_complete(ledger: &Ledger) -> Vec<Diagnostic> {
    const RULE: RuleId = RuleId(17);
    let mut out = Vec::new();

    for record in ordered(ledger) {
        for disposition in &record.dispositions {
            if disposition.outcome.requires_into() && disposition.into.is_none() {
                out.push(Diagnostic::error(
                    c::R015,
                    RULE,
                    slot_subject(record, SlotRef::Disposition(disposition.target.clone())),
                    format!(
                        "`into` is required for outcome `{}`",
                        disposition.outcome.name()
                    ),
                ));
            }
            if disposition.outcome.forbids_into() && disposition.into.is_some() {
                out.push(Diagnostic::error(
                    c::R015,
                    RULE,
                    slot_subject(record, SlotRef::Disposition(disposition.target.clone())),
                    format!(
                        "`into` is forbidden for outcome `{}`",
                        disposition.outcome.name()
                    ),
                ));
            }
        }

        if record.kind.class() != Class::Planning {
            continue;
        }
        for reference in record.targets(Relation::Supersedes) {
            let Some(superseded) = target_of(ledger, reference) else {
                continue;
            };
            if superseded.kind.class() != Class::Planning {
                continue;
            }
            let children = children_of(ledger, &superseded.id);
            let missing: Vec<&Record> = children
                .iter()
                .copied()
                .filter(|child| {
                    !record
                        .dispositions
                        .iter()
                        .any(|d| d.target.key == child.id.key)
                })
                .collect();
            if !missing.is_empty() {
                let names = missing
                    .iter()
                    .map(|c| c.id.key.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                let mut diagnostic = Diagnostic::error(
                    c::R014,
                    RULE,
                    subject(record),
                    format!(
                        "{} supersedes {} but does not dispose of {names}",
                        record.id, superseded.id
                    ),
                )
                .help("add one `disposition` block per unfinished child (D-017)");
                for child in missing {
                    diagnostic = diagnostic.note(Label::with_message(
                        subject(child),
                        format!("{} and part_of the superseded plan", child.state),
                    ));
                }
                out.push(diagnostic);
            }
            let child_keys: BTreeSet<&LogicalKey> = children.iter().map(|c| &c.id.key).collect();
            for disposition in &record.dispositions {
                if !child_keys.contains(&disposition.target.key) {
                    out.push(Diagnostic::error(
                        c::R016,
                        RULE,
                        slot_subject(record, SlotRef::Disposition(disposition.target.clone())),
                        format!(
                            "{} is not `part_of` {}",
                            disposition.target.key, superseded.id
                        ),
                    ));
                }
            }
        }
    }
    out
}

/// Live planning records whose `part_of` pins the given revision.
fn children_of<'a>(ledger: &'a Ledger, parent: &RevisionId) -> Vec<&'a Record> {
    ordered(ledger)
        .into_iter()
        .filter(|candidate| {
            candidate.kind.class() == Class::Planning
                && candidate.is_live()
                && candidate.targets(Relation::PartOf).iter().any(|t| {
                    t.key == parent.key
                        && t.revision
                            .is_some_and(|revision| revision == parent.revision)
                })
        })
        .collect()
}

// ---------------------------------------------------------------------------------
// V-018 / V-019
// ---------------------------------------------------------------------------------

/// V-018: at most one live `plan_of_record` per milestone or track.
#[must_use]
pub fn v018_one_plan_of_record(ledger: &Ledger) -> Vec<Diagnostic> {
    const RULE: RuleId = RuleId(18);
    let mut plans: BTreeMap<LogicalKey, Vec<&Record>> = BTreeMap::new();
    for record in ordered(ledger).into_iter().filter(|r| r.is_live()) {
        for reference in record.targets(Relation::PlanOfRecord) {
            plans.entry(reference.key.clone()).or_default().push(record);
        }
    }
    plans
        .into_iter()
        .filter(|(_, holders)| holders.len() > 1)
        .map(|(target, holders)| {
            let list = holders
                .iter()
                .map(|h| h.id.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            Diagnostic::error(
                c::R018,
                RULE,
                Subject::Key(target.clone()),
                format!(
                    "{target} has {} live plans of record: {list}",
                    holders.len()
                ),
            )
            .help("model an alternative as a revision of the plan, not a second plan")
        })
        .collect()
}

/// V-019: a live record does not rely on an invalid terminal record.
///
/// The resolved counterpart of V-006: it catches a floating reference whose head became
/// terminal after it was written. A completed `depends_on` prerequisite is satisfied,
/// not invalid, and remains legal.
#[must_use]
pub fn v019_live_not_on_terminal(ledger: &Ledger) -> Vec<Diagnostic> {
    const RULE: RuleId = RuleId(19);
    let mut out = Vec::new();
    for record in ordered(ledger).into_iter().filter(|r| r.is_live()) {
        for relation in LIVENESS_RELATIONS {
            for reference in record.targets(*relation) {
                if reference.is_pinned() {
                    continue; // V-006 owns pinned references.
                }
                let Some(target) = target_of(ledger, reference) else {
                    continue;
                };
                if target.is_terminal() {
                    if *relation == Relation::DependsOn && target.state == State::Completed {
                        continue;
                    }
                    out.push(
                        Diagnostic::error(
                            c::R021,
                            RULE,
                            slot_subject(record, SlotRef::Relation(*relation)),
                            format!(
                                "{} is {} but `{relation}` resolves to {}, which is {}",
                                record.id, record.state, target.id, target.state
                            ),
                        )
                        .help(
                            "repoint the reference, revise this record, or use `after` for ordering",
                        ),
                    );
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------
// V-020
// ---------------------------------------------------------------------------------

/// V-020: a `completed` milestone or work record has every acceptance check satisfied.
///
/// A check is satisfied by evidence with `result pass`. The descendant-commit condition
/// of D-016 is applied only when [`LedgerFacts`](crate::model::LedgerFacts) carries both
/// the record's last content change and an ancestry that knows both commits; P5 fills
/// those. Without them the commit condition is not evaluated, and the rest of the rule
/// still is.
#[must_use]
pub fn v020_acceptance_satisfied(ledger: &Ledger) -> Vec<Diagnostic> {
    const RULE: RuleId = RuleId(20);
    let mut out = Vec::new();

    for record in ordered(ledger)
        .into_iter()
        .filter(|r| r.state == State::Completed)
    {
        let Some(acceptance) = &record.acceptance else {
            continue;
        };
        for check in &acceptance.checks {
            let mut satisfied = false;
            let mut reason = "no evidence is cited".to_owned();
            for reference in &check.verified_by {
                let Some(evidence) = target_of(ledger, reference) else {
                    reason = "the cited evidence does not resolve".to_owned();
                    continue;
                };
                let passes = evidence
                    .get(ContentSlot::Result)
                    .and_then(ContentValue::as_enum)
                    .and_then(|e| EvidenceResult::from_name(e.as_str()))
                    == Some(EvidenceResult::Pass);
                if !passes {
                    reason = format!("{} does not record `result pass`", evidence.id);
                    continue;
                }
                match commit_order(ledger, record, evidence) {
                    CommitOrder::Descends => {}
                    CommitOrder::Predates => {
                        reason = format!(
                            "{} predates the last content change to {}",
                            evidence.id, record.id
                        );
                        continue;
                    }
                    CommitOrder::Divergent => {
                        reason = format!(
                            "{} was observed at a commit that is not an ancestor of the last \
                             content change to {} \u{2014} the branch it was recorded on is not \
                             in this history (AKR-G012)",
                            evidence.id, record.id
                        );
                        continue;
                    }
                    CommitOrder::Unknown => {
                        reason = format!(
                            "{} was observed at a commit this repository does not have \
                             (AKR-G011)",
                            evidence.id
                        );
                        continue;
                    }
                }
                satisfied = true;
                break;
            }
            if !satisfied {
                out.push(
                    Diagnostic::error(
                        c::R022,
                        RULE,
                        slot_subject(record, SlotRef::Check(check.id.clone())),
                        format!(
                            "{} is `completed` but check `{}` is not satisfied: {reason}",
                            record.id, check.id
                        ),
                    )
                    .help("run the check and record the evidence, or leave the record active"),
                );
            }
        }
    }
    out
}

/// How the evidence's commit stands to the record's last content change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommitOrder {
    /// The evidence is current, or the facts needed to decide are absent and the rest of
    /// V-020 is still worth enforcing.
    Descends,
    /// The evidence is genuinely older: the last change descends from it.
    Predates,
    /// Neither commit descends from the other. A rewritten or diverged history, not an
    /// age.
    Divergent,
    /// The evidence names a commit this repository does not have at all.
    Unknown,
}

/// Whether the evidence's commit descends from the record's last content change.
///
/// [`CommitOrder::Descends`] when the facts needed to decide are absent: P1 has no git,
/// and the rest of V-020 is still worth enforcing.
///
/// The three failing answers are kept apart because they are not the same fault and do
/// not have the same repair. "Predates" is a claim about time and is only true when the
/// last change really does descend from the evidence. When a history is rewritten —
/// a rebase, a squashed re-upload — the evidence's commit survives on no branch, neither
/// commit reaches the other, and calling that "older" sends the reader looking for a
/// newer run of a check that was never stale. Lege-ecosystem carries 228 `AKR-G012`
/// warnings from one such re-upload, and every one of its eight `AKR-R022`s was reported
/// as an age.
///
/// When `record` carries a `legacy` source (D-028), the descendancy comparison itself is
/// waived — a historical port's own introduction commit says nothing about when the work
/// happened — but the evidence must still cite a commit this repository actually has,
/// whenever git facts were supplied at all. That containment check is not waived.
fn commit_order(ledger: &Ledger, record: &Record, evidence: &Record) -> CommitOrder {
    let observed = evidence
        .get(ContentSlot::ObservedAt)
        .and_then(ContentValue::as_commit);
    if record.sources.iter().any(|s| s.kind == SourceKind::Legacy) {
        return match observed {
            Some(commit)
                if ledger.facts.ancestry.has_facts() && !ledger.facts.ancestry.knows(commit) =>
            {
                CommitOrder::Unknown
            }
            _ => CommitOrder::Descends,
        };
    }
    let Some(last_change) = ledger.facts.last_change.get(&record.id) else {
        return CommitOrder::Descends;
    };
    if ledger.facts.last_change.get(&evidence.id) == Some(last_change) {
        // Evidence and the verified record were authored in the same prepared change.
        // Their commit could not be named by `observed_at` before Git created it.
        return CommitOrder::Descends;
    }
    let Some(observed) = observed else {
        return CommitOrder::Descends;
    };
    let ancestry = &ledger.facts.ancestry;
    match ancestry.is_descendant(observed, last_change) {
        Some(true) | None => CommitOrder::Descends,
        // Which of the two it is changes the words, never the verdict: the ancestry has
        // already said this evidence does not carry, and a workspace whose history was
        // rewritten should not newly fail because it can now be named accurately.
        Some(false) if ledger.facts.off_branch.contains(observed) => CommitOrder::Divergent,
        Some(false) => match ancestry.is_descendant(last_change, observed) {
            Some(true) => CommitOrder::Predates,
            _ => CommitOrder::Divergent,
        },
    }
}

// ---------------------------------------------------------------------------------
// V-021 / V-022 / V-023
// ---------------------------------------------------------------------------------

/// V-021: an `active` decision cites a requirement, policy, constraint or evidence.
#[must_use]
pub fn v021_decision_cites(ledger: &Ledger) -> Vec<Diagnostic> {
    const RULE: RuleId = RuleId(21);
    const CITEABLE: &[Kind] = &[
        Kind::Requirement,
        Kind::Policy,
        Kind::Constraint,
        Kind::Evidence,
    ];
    ordered(ledger)
        .into_iter()
        .filter(|r| r.kind == Kind::Decision && r.state == State::Active)
        .filter(|record| {
            ![
                Relation::Implements,
                Relation::DependsOn,
                Relation::SupportedBy,
            ]
            .iter()
            .flat_map(|relation| record.targets(*relation))
            .filter_map(|reference| target_of(ledger, reference))
            .any(|target| target.is_live() && CITEABLE.contains(&target.kind))
        })
        .map(|record| {
            Diagnostic::error(
                c::R031,
                RULE,
                subject(record),
                format!(
                    "active decision {} cites no requirement, policy, constraint, or evidence",
                    record.id
                ),
            )
            .help("a decision resting on nothing is a preference; cite what motivated it")
        })
        .collect()
}

/// V-022: a live observation has `observed_at` plus one of `method`, a `source` block, or
/// supporting evidence.
#[must_use]
pub fn v022_observation_provenance(ledger: &Ledger) -> Vec<Diagnostic> {
    const RULE: RuleId = RuleId(22);
    ordered(ledger)
        .into_iter()
        .filter(|r| r.kind == Kind::Observation && r.state == State::Verified)
        .filter(|r| {
            r.get(ContentSlot::Method).is_none()
                && r.sources.is_empty()
                && r.targets(Relation::SupportedBy).is_empty()
        })
        .map(|r| {
            Diagnostic::error(
                c::R032,
                RULE,
                subject(r),
                format!(
                    "verified observation {} has no `method`, `source`, or supporting evidence",
                    r.id
                ),
            )
            .help("the commit says when; this says how")
        })
        .collect()
}

/// V-023: a `contradicts` edge between two live records is resolved or acknowledged.
///
/// The relation is symmetric, so an acknowledgement on either side settles it, and a
/// mutually declared contradiction is reported once.
#[must_use]
pub fn v023_contradiction_dispositioned(ledger: &Ledger) -> Vec<Diagnostic> {
    const RULE: RuleId = RuleId(23);
    let mut out = Vec::new();
    for record in ordered(ledger).into_iter().filter(|r| r.is_live()) {
        for reference in record.targets(Relation::Contradicts) {
            let Some(other) = target_of(ledger, reference) else {
                continue;
            };
            if !other.is_live() || record.acknowledged || other.acknowledged {
                continue;
            }
            if other.id < record.id
                && other
                    .targets(Relation::Contradicts)
                    .iter()
                    .any(|t| t.key == record.id.key)
            {
                continue; // reported once, from the lower revision identifier
            }
            out.push(
                Diagnostic::error(
                    c::R041,
                    RULE,
                    slot_subject(record, SlotRef::Relation(Relation::Contradicts)),
                    format!(
                        "{} contradicts {}; both are live and it is not acknowledged",
                        record.id, other.id
                    ),
                )
                .note(Label::with_message(subject(other), "the other side"))
                .help("supersede one side, or set `acknowledged true` and say why"),
            );
        }
    }
    out
}

// ---------------------------------------------------------------------------------
// V-024
// ---------------------------------------------------------------------------------

/// V-024: every sealed revision's content hash matches `akr.lock`.
///
/// Does nothing unless [`LedgerFacts::lock_present`](crate::model::LedgerFacts) is set: a
/// project that has never been built has nothing to compare against, and inventing a
/// mismatch would be worse than silence. P3 supplies the facts.
///
/// Raises `AKR-R051` for a modified sealed revision and `AKR-R052` for a lock missing an
/// entry.
#[must_use]
pub fn v024_seals_match(ledger: &Ledger) -> Vec<Diagnostic> {
    const RULE: RuleId = RuleId(24);
    if !ledger.facts.lock_present {
        return Vec::new();
    }
    let mut out = Vec::new();
    for record in ordered(ledger).into_iter().filter(|r| r.is_sealed()) {
        let Some(fact) = ledger.facts.seals.get(&record.id) else {
            out.push(
                Diagnostic::error(
                    c::R052,
                    RULE,
                    subject(record),
                    format!("akr.lock has no seal entry for {}", record.id),
                )
                .help("run `akr build`"),
            );
            continue;
        };
        match (&fact.recorded, &fact.computed) {
            (Some(recorded), Some(computed)) if recorded != computed => {
                if lifecycle_only_seal_drift(ledger, record, fact, recorded) {
                    out.push(
                        Diagnostic::error(
                            c::R052,
                            RULE,
                            subject(record),
                            format!(
                                "{} moved from {} to {}; akr.lock still records the earlier lifecycle state",
                                record.id,
                                fact.recorded_state.expect("checked by lifecycle drift"),
                                record.state
                            ),
                        )
                        .help("run `akr build`"),
                    );
                } else if pin_follow_only_seal_drift(ledger, record, recorded) {
                    out.push(
                        Diagnostic::error(
                            c::R052,
                            RULE,
                            subject(record),
                            format!(
                                "{} followed a pin onto a successor; akr.lock still records the earlier revision",
                                record.id
                            ),
                        )
                        .help("run `akr build`"),
                    );
                } else {
                    out.push(
                        Diagnostic::error(
                            c::R051,
                            RULE,
                            subject(record),
                            format!(
                                "{} is {} and sealed; recorded {recorded}, computed {computed}",
                                record.id, record.state
                            ),
                        )
                        .help("create a new revision with `akr revise` instead"),
                    );
                }
            }
            (None, _) => out.push(Diagnostic::error(
                c::R052,
                RULE,
                subject(record),
                format!("akr.lock has no hash for {}", record.id),
            )),
            _ => {}
        }
    }
    out
}

/// Whether a mismatched seal is exactly one legal lifecycle transition.
///
/// Re-rendering the current record in the state stored by the lock proves that no other
/// hashed content changed. This lets supported operations retire a revision without
/// disguising a simultaneous body edit as ordinary lock staleness.
fn lifecycle_only_seal_drift(
    ledger: &Ledger,
    record: &Record,
    fact: &crate::model::SealFact,
    recorded_hash: &crate::model::ContentHash,
) -> bool {
    let Some(recorded_state) = fact.recorded_state else {
        return false;
    };
    if !record
        .kind
        .class()
        .transitions()
        .iter()
        .any(|transition| transition.from == recorded_state && transition.to == record.state)
    {
        return false;
    }
    let mut before = record.clone();
    before.state = recorded_state;
    crate::hash::content_hash(&crate::syntax::record_text(&before, &ledger.project.name))
        == *recorded_hash
}

/// Whether a mismatched seal is exactly the pin-follow `akr revise` writes onto live
/// referrers when it retires a head.
///
/// Reverting every non-historical pin from `@key/n` to the same-key revision that `n`
/// supersedes, then re-hashing, proves that no other hashed content changed. Historical
/// pins are left alone — they are supposed to stay on the retired revision.
fn pin_follow_only_seal_drift(
    ledger: &Ledger,
    record: &Record,
    recorded_hash: &crate::model::ContentHash,
) -> bool {
    let mut before = record.clone();
    if !before.rewrite_non_historical_pins(|reference| revert_followed_pin(ledger, reference)) {
        return false;
    }
    crate::hash::content_hash(&crate::syntax::record_text(&before, &ledger.project.name))
        == *recorded_hash
}

fn revert_followed_pin(ledger: &Ledger, reference: &mut Reference) -> bool {
    let Some(revision) = reference.revision else {
        return false;
    };
    let Some(previous) = predecessor_revision(ledger, &reference.key, revision) else {
        return false;
    };
    reference.revision = Some(previous);
    true
}

fn predecessor_revision(ledger: &Ledger, key: &LogicalKey, revision: u32) -> Option<u32> {
    ledger
        .get(&RevisionId::new(key.clone(), revision))?
        .targets(Relation::Supersedes)
        .iter()
        .filter(|target| &target.key == key)
        .filter_map(|target| target.revision)
        .filter(|previous| *previous < revision)
        .max()
}

// ---------------------------------------------------------------------------------
// V-025
// ---------------------------------------------------------------------------------

/// V-025: an evidence `artifact` does not live in disposable scratch.
///
/// Scratch is the one place in the workspace the protocol documents as disposable, and
/// `akr scratch prune` deletes from it on an ordinary handoff (D-036). An artefact cited
/// from there is a verified claim whose backing some later session removes without
/// knowing it was cited, and nothing notices, because a path is not a reference the
/// ledger resolves. An audit of Lege-ecosystem found 38 records citing scratch paths, most
/// of them already dangling
/// (`akr.papercut.collated-19-papercuts-from-evidence-intake`).
///
/// This is a rule rather than a check on one command because evidence reaches the ledger
/// by four routes — `akr evidence add`, `akr evidence add-many --from`, `akr propose
/// --kind evidence --from`, and `knowledge.propose` over MCP — and a guard on the first
/// of them leaves the other three open. Every write validates the resulting ledger before
/// it writes anything (`docs/07` §4), so a rule here is still a refusal at write time,
/// which is the point: a warning from a later build arrives after the record is in the
/// ledger, which is when nobody goes back and moves the file.
///
/// D-036's "scratch is never a ledger diagnostic" is about the directory's contents — how
/// much is sitting there, and how old. This is about a record. A top-level entry named in
/// `.agent/scratch/KEEP` is not disposable and is therefore a valid citation target; the
/// keep index is attached as a ledger input fact by both read and write pipelines.
///
/// Only live revisions are judged, for the reason V-023 gives about contradictions: a
/// sealed revision is a fact of history, not a live claim, and a rule that judges one is
/// a rule the sanctioned write path cannot satisfy. Superseding an evidence record leaves
/// the old revision in the file, so a rule reading every revision survives its own repair
/// — `knowledge.revise` was refused "because the superseded invalid revision still had to
/// validate" (`evidence-intake-project.papercut.correcting-an-invalid-scratch-backed-evidence`).
/// The same reading made every write in a workspace with historical scratch citations
/// fail on ~230 diagnostics about records nobody was touching, because every write
/// validates the whole resulting ledger (`docs/07` §4).
#[must_use]
pub fn v025_evidence_artifact_durable(ledger: &Ledger) -> Vec<Diagnostic> {
    const RULE: RuleId = RuleId(25);
    let mut out = Vec::new();
    for record in ordered(ledger)
        .into_iter()
        .filter(|r| r.kind == Kind::Evidence && r.is_live())
    {
        let Some(ContentValue::Text(artifact)) = record.get(ContentSlot::Artifact) else {
            continue;
        };
        let Some(entry) = crate::scratch::cited_entry(artifact) else {
            continue;
        };
        if !entry.is_empty() && ledger.facts.scratch_kept.contains(&entry) {
            continue;
        }
        // The first two repairs need the file to still be there. Once `akr scratch prune`
        // has taken it the citation cannot be honoured at all, and the only honest repair
        // left is to stop making it: `command`, `summary` and `observed_at` carry the
        // claim without it. A help line that names only the impossible repairs is what
        // sent one agent looking for a way out and finding none.
        let help = if entry.is_empty() {
            "move the artefact somewhere durable and cite that path, or drop the `artifact` \
             slot if it is already gone"
                .to_owned()
        } else {
            format!(
                "move the artefact somewhere durable and cite that path, keep it with \
                 `akr scratch keep {entry}` first, or drop the `artifact` slot if it is \
                 already gone"
            )
        };
        out.push(
            Diagnostic::error(
                c::T023,
                RULE,
                subject(record),
                format!(
                    "artifact {artifact:?} is under {}, which is disposable",
                    crate::scratch::SCRATCH_DIR
                ),
            )
            .help(help),
        );
    }
    out
}

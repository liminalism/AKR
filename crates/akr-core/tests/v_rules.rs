//! Exit criterion 2: every rule `V-001`..`V-025` has at least one passing and one
//! failing case, built entirely from model builders with no text format in sight.

use akr_core::diagnostics::{Code, Diagnostic, codes as c};
use akr_core::model::{
    Acceptance, CheckMethod, ContentHash, ContentSlot, ContentValue, Kind, Ledger, Outcome,
    Project, Record, RecordBuilder, RevisionId, SealFact, SourceKind, State, key,
};
use akr_core::validate;

// ---------------------------------------------------------------------------------
// harness
// ---------------------------------------------------------------------------------

fn ledger(records: Vec<Record>) -> Ledger {
    let mut ledger = Ledger::new(Project::new("fixtures", &["fx"]));
    ledger.extend(records);
    ledger
}

/// A record with every required slot filled, so a test exercises one rule and not V-008.
fn rec(key_text: &str, revision: u32, kind: Kind) -> RecordBuilder {
    RecordBuilder::new(key_text, revision, kind).filled()
}

fn codes_of(found: &[Diagnostic]) -> Vec<&'static str> {
    found.iter().map(|d| d.code.as_str()).collect()
}

#[track_caller]
fn assert_raises(found: &[Diagnostic], code: Code) {
    assert!(
        found.iter().any(|d| d.code == code),
        "expected {code}, got {:?}",
        codes_of(found)
    );
}

#[track_caller]
fn assert_clean(found: &[Diagnostic]) {
    assert!(
        found.is_empty(),
        "expected no diagnostics, got {:?}",
        codes_of(found)
    );
}

// ---------------------------------------------------------------------------------
// V-001
// ---------------------------------------------------------------------------------

#[test]
fn v001_fails_on_an_unknown_key() {
    let l = ledger(vec![
        rec("fx.work.dangling", 1, Kind::Work)
            .rel(Relation::DependsOn, "@fx.term.missing")
            .build(),
    ]);
    assert_raises(&validate::v001_references_resolve(&l), c::L001);
}

#[test]
fn v001_fails_on_a_missing_revision() {
    let l = ledger(vec![
        rec("fx.term.present", 1, Kind::Term).build(),
        rec("fx.work.pinned", 1, Kind::Work)
            .rel(Relation::DependsOn, "@fx.term.present/7")
            .build(),
    ]);
    assert_raises(&validate::v001_references_resolve(&l), c::L003);
}

#[test]
fn v001_fails_on_a_duplicate_revision_identifier() {
    let l = ledger(vec![
        rec("fx.term.twice", 1, Kind::Term).build(),
        rec("fx.term.twice", 1, Kind::Term).build(),
    ]);
    assert_raises(&validate::v001_references_resolve(&l), c::L041);
}

#[test]
fn v001_passes_when_every_reference_resolves() {
    let l = ledger(vec![
        rec("fx.term.present", 1, Kind::Term).build(),
        rec("fx.work.fine", 1, Kind::Work)
            .rel(Relation::DependsOn, "@fx.term.present")
            .rel(Relation::DependsOn, "@fx.term.present/1")
            .build(),
    ]);
    assert_clean(&validate::v001_references_resolve(&l));
}

// ---------------------------------------------------------------------------------
// V-002
// ---------------------------------------------------------------------------------

#[test]
fn v002_fails_on_an_undeclared_namespace() {
    let l = ledger(vec![rec("nope.term.stranger", 1, Kind::Term).build()]);
    assert_raises(&validate::v002_namespaces_declared(&l), c::L004);
}

#[test]
fn v002_passes_for_a_declared_namespace() {
    let l = ledger(vec![rec("fx.term.local", 1, Kind::Term).build()]);
    assert_clean(&validate::v002_namespaces_declared(&l));
}

// ---------------------------------------------------------------------------------
// V-003
// ---------------------------------------------------------------------------------

#[test]
fn v003_fails_when_a_key_is_split_across_files() {
    let l = ledger(vec![
        rec("fx.policy.split", 1, Kind::Policy)
            .state(State::Superseded)
            .file("a.akr")
            .build(),
        rec("fx.policy.split", 2, Kind::Policy)
            .rel(Relation::Supersedes, "@fx.policy.split/1")
            .file("b.akr")
            .build(),
    ]);
    assert_raises(&validate::v003_one_key_one_file(&l), c::L006);
}

#[test]
fn v003_passes_when_revisions_share_a_file() {
    let l = ledger(vec![
        rec("fx.policy.together", 1, Kind::Policy)
            .state(State::Superseded)
            .file("a.akr")
            .build(),
        rec("fx.policy.together", 2, Kind::Policy)
            .rel(Relation::Supersedes, "@fx.policy.together/1")
            .file("a.akr")
            .build(),
    ]);
    assert_clean(&validate::v003_one_key_one_file(&l));
}

// ---------------------------------------------------------------------------------
// V-004
// ---------------------------------------------------------------------------------

#[test]
fn v004_fails_on_an_unknown_anchor() {
    let l = ledger(vec![
        rec("fx.term.anchored", 1, Kind::Term).build(),
        rec("fx.work.cites", 1, Kind::Work)
            .rel(Relation::DependsOn, "@fx.term.anchored#nope")
            .build(),
    ]);
    assert_raises(&validate::v004_anchors_exist(&l), c::L011);
}

#[test]
fn v004_fails_distinctly_on_a_retired_anchor() {
    let l = ledger(vec![
        rec("fx.policy.anchors", 1, Kind::Policy)
            .state(State::Superseded)
            .claim("gone", "a claim revision 2 retires")
            .build(),
        rec("fx.policy.anchors", 2, Kind::Policy)
            .claim("kept", "a claim that survives")
            .retires(&["gone"])
            .rel(Relation::Supersedes, "@fx.policy.anchors/1")
            .build(),
        rec("fx.assessment.cites", 1, Kind::Assessment)
            .rel(Relation::SupportedBy, "@fx.policy.anchors#gone")
            .build(),
    ]);
    let found = validate::v004_anchors_exist(&l);
    assert_raises(&found, c::L012);
    assert!(
        !found.iter().any(|d| d.code == c::L011),
        "retired must not read as unknown"
    );
}

#[test]
fn v004_passes_for_an_existing_anchor() {
    let l = ledger(vec![
        rec("fx.term.anchored", 1, Kind::Term)
            .claim("here", "an anchor")
            .build(),
        rec("fx.work.cites", 1, Kind::Work)
            .rel(Relation::DependsOn, "@fx.term.anchored#here")
            .build(),
    ]);
    assert_clean(&validate::v004_anchors_exist(&l));
}

// ---------------------------------------------------------------------------------
// V-005
// ---------------------------------------------------------------------------------

#[test]
fn v005_fails_when_a_relation_target_is_out_of_range() {
    let l = ledger(vec![
        rec("fx.obs.something", 1, Kind::Observation).build(),
        rec("fx.work.mistargeted", 1, Kind::Work)
            .rel(Relation::Implements, "@fx.obs.something")
            .build(),
    ]);
    assert_raises(&validate::v005_targets_kind_correct(&l), c::L031);
}

#[test]
fn v005_fails_when_a_kind_is_out_of_domain() {
    let l = ledger(vec![
        rec("fx.req.something", 1, Kind::Requirement).build(),
        rec("fx.term.overreaching", 1, Kind::Term)
            .rel(Relation::Implements, "@fx.req.something")
            .build(),
    ]);
    assert_raises(&validate::v005_targets_kind_correct(&l), c::L032);
}

#[test]
fn v005_fails_when_a_restricted_slot_target_is_wrong() {
    let l = ledger(vec![
        rec("fx.obs.something", 1, Kind::Observation).build(),
        rec("fx.policy.excepting", 1, Kind::Policy)
            .content(
                ContentSlot::Exceptions,
                ContentValue::Refs(vec![akr_core::model::reference("@fx.obs.something")]),
            )
            .build(),
    ]);
    assert_raises(&validate::v005_targets_kind_correct(&l), c::L033);
}

#[test]
fn v005_passes_for_kind_correct_targets() {
    let l = ledger(vec![
        rec("fx.req.something", 1, Kind::Requirement).build(),
        rec("fx.work.correct", 1, Kind::Work)
            .rel(Relation::Implements, "@fx.req.something")
            .build(),
    ]);
    assert_clean(&validate::v005_targets_kind_correct(&l));
}

// ---------------------------------------------------------------------------------
// V-006
// ---------------------------------------------------------------------------------

#[test]
fn v006_fails_when_a_live_record_pins_a_terminal_target() {
    let l = ledger(vec![
        rec("fx.policy.retired", 1, Kind::Policy)
            .state(State::Withdrawn)
            .build(),
        rec("fx.work.builds-on-withdrawn", 1, Kind::Work)
            .state(State::Active)
            .rel(Relation::DependsOn, "@fx.policy.retired/1")
            .build(),
    ]);
    assert_raises(&validate::v006_historical_references(&l), c::L021);
}

#[test]
fn v006_passes_for_after_pointing_at_a_completed_milestone() {
    let l = ledger(vec![
        rec("fx.milestone.m1", 1, Kind::Milestone)
            .state(State::Completed)
            .build(),
        rec("fx.milestone.m2", 1, Kind::Milestone)
            .state(State::Active)
            .rel(Relation::After, "@fx.milestone.m1/1")
            .build(),
    ]);
    assert_clean(&validate::v006_historical_references(&l));
}

#[test]
fn v006_passes_for_depends_on_pointing_at_a_completed_milestone() {
    let l = ledger(vec![
        rec("fx.milestone.done", 1, Kind::Milestone)
            .state(State::Completed)
            .build(),
        rec("fx.work.next", 1, Kind::Work)
            .state(State::Ready)
            .rel(Relation::DependsOn, "@fx.milestone.done/1")
            .build(),
    ]);
    assert_clean(&validate::v006_historical_references(&l));
}

#[test]
fn v006_passes_for_part_of_a_superseded_plan_when_dispositioned() {
    let l = ledger(vec![
        rec("fx.work.plan", 1, Kind::Work)
            .state(State::Superseded)
            .build(),
        rec("fx.work.plan", 2, Kind::Work)
            .state(State::Active)
            .rel(Relation::Supersedes, "@fx.work.plan/1")
            .disposition("@fx.work.child", Outcome::IntentionallyDropped, None)
            .build(),
        rec("fx.work.child", 1, Kind::Work)
            .state(State::Ready)
            .rel(Relation::PartOf, "@fx.work.plan/1")
            .build(),
    ]);
    assert_clean(&validate::v006_historical_references(&l));
}

#[test]
fn v006_fails_for_part_of_a_superseded_plan_without_a_disposition() {
    let l = ledger(vec![
        rec("fx.work.plan", 1, Kind::Work)
            .state(State::Superseded)
            .build(),
        rec("fx.work.plan", 2, Kind::Work)
            .state(State::Active)
            .rel(Relation::Supersedes, "@fx.work.plan/1")
            .build(),
        rec("fx.work.orphan", 1, Kind::Work)
            .state(State::Ready)
            .rel(Relation::PartOf, "@fx.work.plan/1")
            .build(),
    ]);
    assert_raises(&validate::v006_historical_references(&l), c::L021);
}

// ---------------------------------------------------------------------------------
// V-007
// ---------------------------------------------------------------------------------

#[test]
fn v007_fails_on_a_state_from_another_class() {
    let l = ledger(vec![
        rec("fx.policy.wrong-state", 1, Kind::Policy)
            .state(State::Completed)
            .build(),
    ]);
    assert_raises(&validate::v007_state_legal(&l), c::T011);
}

#[test]
fn v007_passes_for_every_state_of_every_kind() {
    for kind in Kind::ALL {
        for state in kind.class().states() {
            let l = ledger(vec![rec("fx.a.b", 1, *kind).state(*state).build()]);
            assert_clean(&validate::v007_state_legal(&l));
        }
    }
}

// ---------------------------------------------------------------------------------
// V-008
// ---------------------------------------------------------------------------------

#[test]
fn v008_fails_on_a_missing_required_slot() {
    let l = ledger(vec![
        RecordBuilder::new("fx.term.bare", 1, Kind::Term)
            .all_scope()
            .build(),
    ]);
    assert_raises(&validate::v008_slots_present(&l), c::T001);
}

#[test]
fn v008_fails_on_a_slot_the_kind_does_not_define() {
    let l = ledger(vec![
        rec("fx.decision.confused", 1, Kind::Decision)
            .prose(ContentSlot::Rule, "policies have rules; decisions do not")
            .build(),
    ]);
    assert_raises(&validate::v008_slots_present(&l), c::T002);
}

#[test]
fn v008_fails_on_a_milestone_without_acceptance() {
    let mut record = rec("fx.milestone.vague", 1, Kind::Milestone).build();
    record.acceptance = None;
    assert_raises(
        &validate::v008_slots_present(&ledger(vec![record])),
        c::T006,
    );
}

#[test]
fn v008_fails_on_topic_outside_the_normative_class() {
    let l = ledger(vec![
        rec("fx.work.topical", 1, Kind::Work)
            .topic("something")
            .build(),
    ]);
    assert_raises(&validate::v008_slots_present(&l), c::T034);
}

#[test]
fn v008_fails_on_an_acceptance_block_where_it_is_not_permitted() {
    let mut record = rec("fx.term.overreaching", 1, Kind::Term).build();
    record.acceptance = Some(Acceptance::default());
    assert_raises(
        &validate::v008_slots_present(&ledger(vec![record])),
        c::T005,
    );
}

#[test]
fn v008_passes_for_a_filled_record_of_every_kind() {
    for kind in Kind::ALL {
        let l = ledger(vec![rec("fx.a.b", 1, *kind).build()]);
        assert_clean(&validate::v008_slots_present(&l));
    }
}

// ---------------------------------------------------------------------------------
// V-009 / V-010 / V-011
// ---------------------------------------------------------------------------------

#[test]
fn v009_fails_without_observed_at() {
    let l = ledger(vec![
        RecordBuilder::new("fx.obs.rumour", 1, Kind::Observation)
            .prose(ContentSlot::Statement, "something")
            .build(),
    ]);
    assert_raises(&validate::v009_observation_commit(&l), c::T021);
}

#[test]
fn v009_passes_with_observed_at() {
    let l = ledger(vec![rec("fx.obs.sourced", 1, Kind::Observation).build()]);
    assert_clean(&validate::v009_observation_commit(&l));
}

#[test]
fn v010_fails_without_a_result() {
    let l = ledger(vec![
        RecordBuilder::new("fx.evidence.partial", 1, Kind::Evidence)
            .enum_value(ContentSlot::Method, "command")
            .commit(
                ContentSlot::ObservedAt,
                "3f0a1c9d5b7e2648a0d4f1b8c36e9752ad014b6f",
            )
            .build(),
    ]);
    assert_raises(&validate::v010_evidence_slots(&l), c::T022);
}

#[test]
fn v010_passes_with_all_three() {
    let l = ledger(vec![rec("fx.evidence.complete", 1, Kind::Evidence).build()]);
    assert_clean(&validate::v010_evidence_slots(&l));
}

#[test]
fn v011_fails_when_a_resolved_question_records_no_answer() {
    let l = ledger(vec![
        rec("fx.question.unanswered", 1, Kind::Question)
            .state(State::Resolved)
            .build(),
    ]);
    assert_raises(&validate::v011_resolved_question(&l), c::T031);
}

#[test]
fn v011_passes_with_a_resolution_and_a_resolver() {
    let l = ledger(vec![
        rec("fx.question.answered", 1, Kind::Question)
            .state(State::Resolved)
            .prose(ContentSlot::Resolution, "the viewer owns text layout")
            .build(),
        rec("fx.decision.answering", 1, Kind::Decision)
            .state(State::Active)
            .rel(Relation::Resolves, "@fx.question.answered")
            .build(),
    ]);
    assert_clean(&validate::v011_resolved_question(&l));
}

// ---------------------------------------------------------------------------------
// V-012 / V-013
// ---------------------------------------------------------------------------------

#[test]
fn v012_fails_on_two_live_revisions() {
    let l = ledger(vec![
        rec("fx.policy.two-heads", 1, Kind::Policy)
            .state(State::Active)
            .build(),
        rec("fx.policy.two-heads", 2, Kind::Policy)
            .state(State::Active)
            .rel(Relation::Supersedes, "@fx.policy.two-heads/1")
            .build(),
    ]);
    assert_raises(&validate::v012_one_live_head(&l), c::R001);
}

#[test]
fn v012_passes_once_the_earlier_revision_is_superseded() {
    let l = ledger(vec![
        rec("fx.policy.one-head", 1, Kind::Policy)
            .state(State::Superseded)
            .build(),
        rec("fx.policy.one-head", 2, Kind::Policy)
            .state(State::Active)
            .rel(Relation::Supersedes, "@fx.policy.one-head/1")
            .build(),
    ]);
    assert_clean(&validate::v012_one_live_head(&l));
}

#[test]
fn v013_fails_on_a_shared_topic_with_overlapping_scope() {
    let l = ledger(vec![
        rec("fx.policy.loose", 1, Kind::Policy)
            .state(State::Active)
            .topic("tandem-work")
            .build(),
        rec("fx.policy.strict", 1, Kind::Policy)
            .state(State::Active)
            .topic("tandem-work")
            .scope(vec![])
            .path_scope("sim/**")
            .build(),
    ]);
    assert_raises(&validate::v013_topic_exclusivity(&l), c::R002);
}

#[test]
fn v013_passes_for_distinct_topics_and_for_disjoint_scopes() {
    let distinct = ledger(vec![
        rec("fx.policy.a", 1, Kind::Policy)
            .state(State::Active)
            .topic("one")
            .build(),
        rec("fx.policy.b", 1, Kind::Policy)
            .state(State::Active)
            .topic("two")
            .build(),
    ]);
    assert_clean(&validate::v013_topic_exclusivity(&distinct));

    let disjoint = ledger(vec![
        rec("fx.policy.a", 1, Kind::Policy)
            .state(State::Active)
            .topic("same")
            .scope(vec![])
            .path_scope("sim/**")
            .build(),
        rec("fx.policy.b", 1, Kind::Policy)
            .state(State::Active)
            .topic("same")
            .scope(vec![])
            .path_scope("lege/**")
            .build(),
    ]);
    assert_clean(&validate::v013_topic_exclusivity(&disjoint));
}

// ---------------------------------------------------------------------------------
// V-014 / V-015 / V-016
// ---------------------------------------------------------------------------------

#[test]
fn v014_fails_on_a_supersession_cycle() {
    let l = ledger(vec![
        rec("fx.decision.loop", 1, Kind::Decision)
            .state(State::Superseded)
            .rel(Relation::Supersedes, "@fx.decision.loop/2")
            .build(),
        rec("fx.decision.loop", 2, Kind::Decision)
            .state(State::Superseded)
            .rel(Relation::Supersedes, "@fx.decision.loop/1")
            .build(),
    ]);
    assert_raises(&validate::v014_supersession_acyclic(&l), c::R011);
}

#[test]
fn v014_fails_when_supersession_crosses_kinds() {
    let l = ledger(vec![
        rec("fx.obs.old", 1, Kind::Observation)
            .state(State::Superseded)
            .build(),
        rec("fx.decision.new", 1, Kind::Decision)
            .rel(Relation::Supersedes, "@fx.obs.old/1")
            .build(),
    ]);
    assert_raises(&validate::v014_supersession_acyclic(&l), c::R017);
}

#[test]
fn v014_passes_for_a_chain() {
    let l = ledger(vec![
        rec("fx.decision.chain", 1, Kind::Decision)
            .state(State::Superseded)
            .build(),
        rec("fx.decision.chain", 2, Kind::Decision)
            .state(State::Active)
            .rel(Relation::Supersedes, "@fx.decision.chain/1")
            .build(),
    ]);
    assert_clean(&validate::v014_supersession_acyclic(&l));
}

#[test]
fn v015_fails_on_a_dependency_cycle() {
    let l = ledger(vec![
        rec("fx.work.alpha", 1, Kind::Work)
            .rel(Relation::DependsOn, "@fx.work.beta")
            .build(),
        rec("fx.work.beta", 1, Kind::Work)
            .rel(Relation::DependsOn, "@fx.work.alpha")
            .build(),
    ]);
    assert_raises(&validate::v015_structural_acyclic(&l), c::R012);
}

#[test]
fn v015_passes_for_a_chain() {
    let l = ledger(vec![
        rec("fx.work.alpha", 1, Kind::Work)
            .rel(Relation::DependsOn, "@fx.work.beta")
            .build(),
        rec("fx.work.beta", 1, Kind::Work).build(),
    ]);
    assert_clean(&validate::v015_structural_acyclic(&l));
}

#[test]
fn v016_fails_on_an_ordering_cycle() {
    let l = ledger(vec![
        rec("fx.milestone.m1", 1, Kind::Milestone)
            .rel(Relation::After, "@fx.milestone.m2")
            .build(),
        rec("fx.milestone.m2", 1, Kind::Milestone)
            .rel(Relation::After, "@fx.milestone.m1")
            .build(),
    ]);
    assert_raises(&validate::v016_after_acyclic(&l), c::R013);
}

#[test]
fn v016_passes_for_an_ordered_plan() {
    let l = ledger(vec![
        rec("fx.milestone.m1", 1, Kind::Milestone).build(),
        rec("fx.milestone.m2", 1, Kind::Milestone)
            .rel(Relation::After, "@fx.milestone.m1")
            .build(),
    ]);
    assert_clean(&validate::v016_after_acyclic(&l));
}

// ---------------------------------------------------------------------------------
// V-017
// ---------------------------------------------------------------------------------

fn replan(with_disposition: bool) -> Ledger {
    let mut plan2 = rec("fx.work.plan", 2, Kind::Work)
        .state(State::Active)
        .rel(Relation::Supersedes, "@fx.work.plan/1");
    if with_disposition {
        plan2 = plan2.disposition("@fx.work.orphan", Outcome::IntentionallyDropped, None);
    }
    ledger(vec![
        rec("fx.work.plan", 1, Kind::Work)
            .state(State::Superseded)
            .build(),
        plan2.build(),
        rec("fx.work.orphan", 1, Kind::Work)
            .state(State::Ready)
            .rel(Relation::PartOf, "@fx.work.plan/1")
            .build(),
    ])
}

#[test]
fn v017_fails_when_a_replan_drops_a_child_silently() {
    assert_raises(
        &validate::v017_disposition_complete(&replan(false)),
        c::R014,
    );
}

#[test]
fn v017_passes_when_every_unfinished_child_is_dispositioned() {
    assert_clean(&validate::v017_disposition_complete(&replan(true)));
}

#[test]
fn v017_fails_when_into_is_required_or_forbidden() {
    let missing = ledger(vec![
        rec("fx.work.plan", 2, Kind::Work)
            .disposition("@fx.work.child", Outcome::CarriedForward, None)
            .build(),
    ]);
    assert_raises(&validate::v017_disposition_complete(&missing), c::R015);

    let surplus = ledger(vec![
        rec("fx.work.plan", 2, Kind::Work)
            .disposition(
                "@fx.work.child",
                Outcome::IntentionallyDropped,
                Some("@fx.track.t"),
            )
            .build(),
    ]);
    assert_raises(&validate::v017_disposition_complete(&surplus), c::R015);
}

#[test]
fn v017_fails_when_dispositioning_something_that_is_not_a_child() {
    let l = ledger(vec![
        rec("fx.work.plan", 1, Kind::Work)
            .state(State::Superseded)
            .build(),
        rec("fx.work.plan", 2, Kind::Work)
            .state(State::Active)
            .rel(Relation::Supersedes, "@fx.work.plan/1")
            .disposition("@fx.work.unrelated", Outcome::IntentionallyDropped, None)
            .build(),
    ]);
    assert_raises(&validate::v017_disposition_complete(&l), c::R016);
}

// ---------------------------------------------------------------------------------
// V-018 / V-019
// ---------------------------------------------------------------------------------

#[test]
fn v018_fails_on_two_live_plans_for_one_milestone() {
    let l = ledger(vec![
        rec("fx.milestone.target", 1, Kind::Milestone).build(),
        rec("fx.work.plan-a", 1, Kind::Work)
            .state(State::Active)
            .rel(Relation::PlanOfRecord, "@fx.milestone.target")
            .build(),
        rec("fx.work.plan-b", 1, Kind::Work)
            .state(State::Proposed)
            .rel(Relation::PlanOfRecord, "@fx.milestone.target")
            .build(),
    ]);
    assert_raises(&validate::v018_one_plan_of_record(&l), c::R018);
}

#[test]
fn v018_passes_with_one_live_plan() {
    let l = ledger(vec![
        rec("fx.milestone.target", 1, Kind::Milestone).build(),
        rec("fx.work.plan-a", 1, Kind::Work)
            .state(State::Active)
            .rel(Relation::PlanOfRecord, "@fx.milestone.target")
            .build(),
        rec("fx.work.plan-b", 1, Kind::Work)
            .state(State::Abandoned)
            .rel(Relation::PlanOfRecord, "@fx.milestone.target")
            .build(),
    ]);
    assert_clean(&validate::v018_one_plan_of_record(&l));
}

#[test]
fn v019_fails_when_a_floating_reference_resolves_to_a_terminal_head() {
    let l = ledger(vec![
        rec("fx.decision.replaced", 1, Kind::Decision)
            .state(State::Rejected)
            .build(),
        rec("fx.work.resting", 1, Kind::Work)
            .state(State::Active)
            .rel(Relation::DependsOn, "@fx.decision.replaced")
            .build(),
    ]);
    assert_raises(&validate::v019_live_not_on_terminal(&l), c::R021);
}

#[test]
fn v019_accepts_a_completed_dependency_but_not_an_abandoned_one() {
    let completed = ledger(vec![
        rec("fx.milestone.done", 1, Kind::Milestone)
            .state(State::Completed)
            .build(),
        rec("fx.work.next", 1, Kind::Work)
            .state(State::Ready)
            .rel(Relation::DependsOn, "@fx.milestone.done")
            .build(),
    ]);
    assert_clean(&validate::v019_live_not_on_terminal(&completed));

    let abandoned = ledger(vec![
        rec("fx.milestone.dropped", 1, Kind::Milestone)
            .state(State::Abandoned)
            .build(),
        rec("fx.work.next", 1, Kind::Work)
            .state(State::Ready)
            .rel(Relation::DependsOn, "@fx.milestone.dropped")
            .build(),
    ]);
    assert_raises(&validate::v019_live_not_on_terminal(&abandoned), c::R021);
}

#[test]
fn v019_passes_when_the_head_is_live_and_ignores_after_edges() {
    let l = ledger(vec![
        rec("fx.decision.current", 1, Kind::Decision)
            .state(State::Active)
            .build(),
        rec("fx.milestone.done", 1, Kind::Milestone)
            .state(State::Completed)
            .build(),
        rec("fx.work.resting", 1, Kind::Work)
            .state(State::Active)
            .rel(Relation::DependsOn, "@fx.decision.current")
            .rel(Relation::After, "@fx.milestone.done")
            .build(),
    ]);
    assert_clean(&validate::v019_live_not_on_terminal(&l));
}

// ---------------------------------------------------------------------------------
// V-020
// ---------------------------------------------------------------------------------

fn completed_milestone(evidence: &[&str]) -> Ledger {
    ledger(vec![
        rec("fx.evidence.green", 1, Kind::Evidence).build(),
        rec("fx.evidence.red", 1, Kind::Evidence)
            .enum_value(ContentSlot::Result, "fail")
            .build(),
        RecordBuilder::new("fx.milestone.claimed", 1, Kind::Milestone)
            .state(State::Completed)
            .check("the-check", CheckMethod::Command, evidence)
            .filled()
            .build(),
    ])
}

#[test]
fn v020_fails_when_a_check_has_no_passing_evidence() {
    assert_raises(
        &validate::v020_acceptance_satisfied(&completed_milestone(&[])),
        c::R022,
    );
    assert_raises(
        &validate::v020_acceptance_satisfied(&completed_milestone(&["@fx.evidence.red"])),
        c::R022,
    );
}

#[test]
fn v020_passes_when_every_check_is_satisfied() {
    assert_clean(&validate::v020_acceptance_satisfied(&completed_milestone(
        &["@fx.evidence.green"],
    )));
}

#[test]
fn v020_applies_the_descendant_rule_once_git_facts_are_present() {
    use akr_core::model::{Ancestry, Commit};
    let old = Commit::new("3f0a1c9d5b7e2648a0d4f1b8c36e9752ad014b6f").expect("commit");
    let new = Commit::new("7c41d0ba92e6f37518a3cd406b5e2f91d8074a63").expect("commit");

    let mut l = completed_milestone(&["@fx.evidence.green"]);
    // The evidence was observed at `old`; the milestone changed at `new`, which is a
    // descendant of it, so the evidence no longer counts (D-016).
    l.facts
        .last_change
        .insert(RevisionId::new(key("fx.milestone.claimed"), 1), new.clone());
    l.facts.ancestry = Ancestry::from_pairs([(new, old)]);
    assert_raises(&validate::v020_acceptance_satisfied(&l), c::R022);
}

#[test]
fn v020_accepts_evidence_co_committed_with_the_verified_record() {
    use akr_core::model::{Ancestry, Commit};
    let observed = Commit::new("3f0a1c9d5b7e2648a0d4f1b8c36e9752ad014b6f").expect("commit");
    let landed = Commit::new("7c41d0ba92e6f37518a3cd406b5e2f91d8074a63").expect("commit");

    let mut ledger = completed_milestone(&["@fx.evidence.green"]);
    ledger.facts.last_change.insert(
        RevisionId::new(key("fx.milestone.claimed"), 1),
        landed.clone(),
    );
    ledger
        .facts
        .last_change
        .insert(RevisionId::new(key("fx.evidence.green"), 1), landed.clone());
    ledger.facts.ancestry = Ancestry::from_pairs([(landed, observed)]);

    assert_clean(&validate::v020_acceptance_satisfied(&ledger));
}

// ---------------------------------------------------------------------------------
// D-028: legacy-sourced completion is exempt from the descendant-commit gate
// ---------------------------------------------------------------------------------

/// The same non-descendant setup as
/// [`v020_applies_the_descendant_rule_once_git_facts_are_present`], but on a milestone
/// that optionally carries a `legacy` source.
fn completed_milestone_with_old_evidence(legacy: bool) -> Ledger {
    use akr_core::model::{Ancestry, Commit};
    let old = Commit::new("3f0a1c9d5b7e2648a0d4f1b8c36e9752ad014b6f").expect("commit");
    let new = Commit::new("7c41d0ba92e6f37518a3cd406b5e2f91d8074a63").expect("commit");

    let mut milestone = RecordBuilder::new("fx.milestone.claimed", 1, Kind::Milestone)
        .state(State::Completed)
        .check("the-check", CheckMethod::Command, &["@fx.evidence.green"])
        .filled();
    if legacy {
        milestone = milestone.source(SourceKind::Legacy, Some("docs/legacy/PLAN.md"));
    }

    let mut l = ledger(vec![
        rec("fx.evidence.green", 1, Kind::Evidence).build(),
        milestone.build(),
    ]);
    l.facts
        .last_change
        .insert(RevisionId::new(key("fx.milestone.claimed"), 1), new.clone());
    l.facts.ancestry = Ancestry::from_pairs([(new, old)]);
    l
}

#[test]
fn d028_legacy_source_waives_the_descendant_gate() {
    // (a) completed record + old non-descendant passing evidence + legacy source: no R022.
    let l = completed_milestone_with_old_evidence(true);
    assert_clean(&validate::v020_acceptance_satisfied(&l));
}

#[test]
fn d028_without_a_legacy_source_the_gate_still_applies() {
    // (b) identical setup, no legacy source: R022 as before.
    let l = completed_milestone_with_old_evidence(false);
    assert_raises(&validate::v020_acceptance_satisfied(&l), c::R022);
}

#[test]
fn d028_legacy_source_still_requires_the_evidence_commit_to_exist() {
    // (c) legacy source, but the evidence cites a commit absent from the repository's
    // known ancestry: the descendant comparison is waived, containment is not, so this
    // still fails.
    use akr_core::model::{Ancestry, Commit};
    let known = Commit::new("7c41d0ba92e6f37518a3cd406b5e2f91d8074a63").expect("commit");
    let stranger = Commit::new("8db2c6e194f7a5013b6c0d2e9f47a1c8065de3b1").expect("commit");

    let milestone = RecordBuilder::new("fx.milestone.claimed", 1, Kind::Milestone)
        .state(State::Completed)
        .check("the-check", CheckMethod::Command, &["@fx.evidence.green"])
        .filled()
        .source(SourceKind::Legacy, Some("docs/legacy/PLAN.md"));

    let mut l = ledger(vec![
        rec("fx.evidence.green", 1, Kind::Evidence)
            .commit(ContentSlot::ObservedAt, stranger.as_str())
            .build(),
        milestone.build(),
    ]);
    l.facts.last_change.insert(
        RevisionId::new(key("fx.milestone.claimed"), 1),
        known.clone(),
    );
    // The ancestry has facts (it knows `known`), but never learned of `stranger` — the
    // repository never had it, exactly as `AKR-G011` would flag for an empirical record.
    l.facts.ancestry = Ancestry::from_pairs([(known.clone(), known)]);
    assert_raises(&validate::v020_acceptance_satisfied(&l), c::R022);
}

// ---------------------------------------------------------------------------------
// V-021 / V-022 / V-023
// ---------------------------------------------------------------------------------

#[test]
fn v021_fails_when_an_active_decision_cites_nothing() {
    let l = ledger(vec![
        rec("fx.decision.unmotivated", 1, Kind::Decision)
            .state(State::Active)
            .build(),
    ]);
    assert_raises(&validate::v021_decision_cites(&l), c::R031);
}

#[test]
fn v021_passes_when_it_cites_a_requirement() {
    let l = ledger(vec![
        rec("fx.req.something", 1, Kind::Requirement)
            .state(State::Active)
            .build(),
        rec("fx.decision.motivated", 1, Kind::Decision)
            .state(State::Active)
            .rel(Relation::Implements, "@fx.req.something")
            .build(),
    ]);
    assert_clean(&validate::v021_decision_cites(&l));
}

#[test]
fn v022_fails_without_provenance() {
    let l = ledger(vec![rec("fx.obs.unsourced", 1, Kind::Observation).build()]);
    assert_raises(&validate::v022_observation_provenance(&l), c::R032);
}

#[test]
fn v022_passes_with_a_method_or_a_source() {
    let with_method = ledger(vec![
        rec("fx.obs.measured", 1, Kind::Observation)
            .enum_value(ContentSlot::Method, "instrumented")
            .build(),
    ]);
    assert_clean(&validate::v022_observation_provenance(&with_method));

    let with_source = ledger(vec![
        rec("fx.obs.cited", 1, Kind::Observation)
            .source(SourceKind::External, Some("artifacts/run.log"))
            .build(),
    ]);
    assert_clean(&validate::v022_observation_provenance(&with_source));
}

#[test]
fn v023_fails_on_an_undispositioned_contradiction() {
    let l = ledger(vec![
        rec("fx.evidence.green", 1, Kind::Evidence).build(),
        rec("fx.obs.drift", 1, Kind::Observation)
            .rel(Relation::Contradicts, "@fx.evidence.green/1")
            .build(),
    ]);
    assert_raises(&validate::v023_contradiction_dispositioned(&l), c::R041);
}

#[test]
fn v023_passes_when_acknowledged_or_when_one_side_is_terminal() {
    let acknowledged = ledger(vec![
        rec("fx.evidence.green", 1, Kind::Evidence).build(),
        rec("fx.obs.drift", 1, Kind::Observation)
            .rel(Relation::Contradicts, "@fx.evidence.green/1")
            .acknowledged(true)
            .build(),
    ]);
    assert_clean(&validate::v023_contradiction_dispositioned(&acknowledged));

    let resolved = ledger(vec![
        rec("fx.evidence.green", 1, Kind::Evidence)
            .state(State::Superseded)
            .build(),
        rec("fx.obs.drift", 1, Kind::Observation)
            .rel(Relation::Contradicts, "@fx.evidence.green/1")
            .build(),
    ]);
    assert_clean(&validate::v023_contradiction_dispositioned(&resolved));
}

// ---------------------------------------------------------------------------------
// V-024
// ---------------------------------------------------------------------------------

fn sealed(recorded: Option<&str>, computed: Option<&str>, lock_present: bool) -> Ledger {
    let mut l = ledger(vec![
        rec("fx.policy.sealed", 1, Kind::Policy)
            .state(State::Active)
            .build(),
    ]);
    l.facts.lock_present = lock_present;
    if recorded.is_some() || computed.is_some() {
        l.facts.seals.insert(
            RevisionId::new(key("fx.policy.sealed"), 1),
            SealFact {
                recorded: recorded.map(|h| ContentHash(h.to_owned())),
                recorded_state: recorded.map(|_| State::Active),
                computed: computed.map(|h| ContentHash(h.to_owned())),
            },
        );
    }
    l
}

#[test]
fn v024_fails_when_a_sealed_revision_was_modified() {
    let l = sealed(Some("sha256:aaaa"), Some("sha256:bbbb"), true);
    assert_raises(&validate::v024_seals_match(&l), c::R051);
}

#[test]
fn v024_fails_when_the_lock_has_no_entry() {
    let l = sealed(None, None, true);
    assert_raises(&validate::v024_seals_match(&l), c::R052);
}

#[test]
fn v024_passes_when_hashes_agree_and_is_silent_without_a_lock() {
    let matching = sealed(Some("sha256:aaaa"), Some("sha256:aaaa"), true);
    assert_clean(&validate::v024_seals_match(&matching));

    // A project that has never been built has nothing to compare against.
    let no_lock = sealed(None, None, false);
    assert_clean(&validate::v024_seals_match(&no_lock));
}

// ---------------------------------------------------------------------------------
// V-025
// ---------------------------------------------------------------------------------

fn artifact(path: &str) -> Ledger {
    ledger(vec![
        rec("fx.evidence.benchmark", 1, Kind::Evidence)
            .state(State::Verified)
            .content(ContentSlot::Artifact, ContentValue::Text(path.to_owned()))
            .build(),
    ])
}

#[test]
fn v025_fails_on_an_artifact_under_the_scratch_directory() {
    // Every spelling of the same disposable place: bare, dot-relative, absolute, and
    // Windows-separated. A path is not a reference the ledger resolves, so the check is
    // textual and has to recognise all of them.
    for path in [
        ".agent/scratch/ocr-benchmark/out.txt",
        "./.agent/scratch/run/score.json",
        "/home/dev/project/.agent/scratch/run/score.json",
        r"C:\work\project\.agent\scratch\run\score.json",
        ".agent/scratch",
    ] {
        let l = artifact(path);
        let found = validate::v025_evidence_artifact_durable(&l);
        assert!(!found.is_empty(), "{path} was accepted");
        assert_raises(&found, c::T023);
    }
}

#[test]
fn v025_passes_on_a_durable_artifact_and_ignores_other_kinds() {
    for path in [
        "docs/benchmarks/2026-08-22.txt",
        ".agent/handoff/notes.md",
        "scratch/out.txt",
        "sources/.agent/scratchpad/out.txt",
    ] {
        assert_clean(&validate::v025_evidence_artifact_durable(&artifact(path)));
    }

    // The rule reads the `artifact` slot, not prose. A papercut describing the problem
    // names scratch paths in its statement and must not trip over itself.
    let prose = ledger(vec![
        rec("fx.papercut.scratch-citations", 1, Kind::Papercut)
            .prose(
                ContentSlot::Statement,
                "38 evidence records cite .agent/scratch/ocr-benchmark paths.",
            )
            .build(),
    ]);
    assert_clean(&validate::v025_evidence_artifact_durable(&prose));
}

#[test]
fn v025_passes_for_an_explicitly_kept_scratch_entry() {
    let mut kept = artifact(".agent/scratch/ocr-benchmark/out.txt");
    kept.facts.scratch_kept.insert("ocr-benchmark".to_owned());
    assert_clean(&validate::v025_evidence_artifact_durable(&kept));

    // Keeping a neighbour does not make the entire scratch tree durable.
    let mut other = artifact(".agent/scratch/other-run/out.txt");
    other.facts.scratch_kept.insert("ocr-benchmark".to_owned());
    assert_raises(&validate::v025_evidence_artifact_durable(&other), c::T023);
}

#[test]
fn v025_ignores_a_superseded_revision_so_the_repair_is_reachable() {
    // Superseding an evidence record leaves the offending revision in the file. A rule
    // that reads every revision therefore survives its own repair, which made this the
    // one rule the sanctioned write path could not satisfy: `revise` was refused because
    // the superseded revision still had to validate (D-037, amended).
    let repaired = ledger(vec![
        rec("fx.evidence.benchmark", 1, Kind::Evidence)
            .state(State::Superseded)
            .content(
                ContentSlot::Artifact,
                ContentValue::Text(".agent/scratch/run/out.txt".to_owned()),
            )
            .build(),
        rec("fx.evidence.benchmark", 2, Kind::Evidence)
            .state(State::Verified)
            .content(
                ContentSlot::Artifact,
                ContentValue::Text("docs/benchmarks/out.txt".to_owned()),
            )
            .build(),
    ]);
    assert_clean(&validate::v025_evidence_artifact_durable(&repaired));

    // A withdrawn revision is equally historical, and the live one is still judged.
    let withdrawn = ledger(vec![
        rec("fx.evidence.benchmark", 1, Kind::Evidence)
            .state(State::Withdrawn)
            .content(
                ContentSlot::Artifact,
                ContentValue::Text(".agent/scratch/run/out.txt".to_owned()),
            )
            .build(),
    ]);
    assert_clean(&validate::v025_evidence_artifact_durable(&withdrawn));
    assert_raises(
        &validate::v025_evidence_artifact_durable(&artifact(".agent/scratch/run/out.txt")),
        c::T023,
    );
}

// ---------------------------------------------------------------------------------
// the catalogue itself
// ---------------------------------------------------------------------------------

#[test]
fn validate_all_is_deterministic_under_shuffled_insertion_order() {
    let build = |reversed: bool| {
        let mut records = vec![
            rec("fx.work.alpha", 1, Kind::Work)
                .rel(Relation::DependsOn, "@fx.work.beta")
                .build(),
            rec("fx.work.beta", 1, Kind::Work)
                .rel(Relation::DependsOn, "@fx.work.alpha")
                .build(),
            rec("fx.decision.unmotivated", 1, Kind::Decision)
                .state(State::Active)
                .build(),
            rec("fx.obs.unsourced", 1, Kind::Observation).build(),
        ];
        if reversed {
            records.reverse();
        }
        validate::validate_all(&ledger(records))
    };
    assert_eq!(
        build(false),
        build(true),
        "diagnostics must not depend on insertion order"
    );
    assert!(!build(false).is_empty());
}

#[test]
fn a_clean_ledger_produces_no_diagnostics_at_all() {
    let l = ledger(vec![
        rec("fx.req.something", 1, Kind::Requirement)
            .state(State::Active)
            .build(),
        rec("fx.decision.motivated", 1, Kind::Decision)
            .state(State::Active)
            .rel(Relation::Implements, "@fx.req.something")
            .file("fx.akr")
            .build(),
        rec("fx.obs.measured", 1, Kind::Observation)
            .enum_value(ContentSlot::Method, "instrumented")
            .file("fx.akr")
            .build(),
    ]);
    assert_clean(&validate::validate_all(&l));
}

use akr_core::model::Relation;

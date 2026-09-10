//! The five write operations of `docs/07` §6, exercised over copies of both worked
//! examples in temporary directories.
//!
//! Every test works on a copy: the operations write to disk, and a test that mutated the
//! committed examples would be a test that broke the repository.

mod ops_support;

use akr_core::model::{
    Class, Commit, ContentSlot, ContentValue, Kind, Outcome as DispositionOutcome, Record,
    RecordBuilder, Reference, Relation, Segment, State, key,
};
use akr_core::ops::{self, DispositionRequest, Edits, ReviseMode, WriteContext, conventional_file};
use ops_support::Sandbox;

fn term(key_text: &str, title: &str) -> Record {
    let mut record = RecordBuilder::new(key_text, 1, Kind::Term)
        .title(title)
        .all_scope()
        .build();
    record.content.insert(
        ContentSlot::Definition,
        ContentValue::prose("A definition supplied by the caller, as `--from` would."),
    );
    record
}

// -------------------------------------------------------------------------------------
// propose
// -------------------------------------------------------------------------------------

#[test]
fn propose_creates_revision_one_in_the_conventional_file() {
    let sandbox = Sandbox::save_your_skin();
    let context = WriteContext::new(sandbox.akr_dir()).with_author("tester");
    let target = key("sys.term.audit-lane");

    let applied = ops::propose(
        &context,
        &target,
        Kind::Term,
        "Audit lane",
        Some(term("sys.term.audit-lane", "Audit lane")),
    )
    .expect("a well-formed proposal is accepted");

    assert_eq!(applied.operation, ops::Operation::Propose);
    assert_eq!(applied.changes.len(), 1);
    assert_eq!(applied.changes[0].kind, ops::ChangeKind::Created);
    assert_eq!(applied.files, vec![conventional_file(&target, Kind::Term)]);
    assert!(applied.lock_stale, "a new revision needs a new seal");

    // The record is on disk, in the state its class starts in, and the tree still parses.
    let reloaded = sandbox.ledger();
    let head = reloaded.head(&target).expect("the new head");
    assert_eq!(head.id.revision, 1);
    assert_eq!(
        head.state,
        State::Proposed,
        "a new record starts unaccepted (docs/07 §6)"
    );
    assert_eq!(head.author.as_deref(), Some("tester"));
    sandbox.assert_canonical();
}

#[test]
fn propose_refuses_an_existing_key() {
    let sandbox = Sandbox::save_your_skin();
    let context = WriteContext::new(sandbox.akr_dir());
    let target = key("sys.term.playable-day");

    let refused = ops::propose(&context, &target, Kind::Term, "again", None)
        .expect_err("the key already exists");
    assert_eq!(refused.code.as_str(), "AKR-L041");
    assert!(refused.help.is_some_and(|h| h.contains("akr revise")));
}

#[test]
fn propose_many_writes_every_record_in_one_atomic_pass() {
    let sandbox = Sandbox::save_your_skin();
    let context = WriteContext::new(sandbox.akr_dir()).with_author("tester");
    let records = vec![
        term("sys.term.batch-alpha", "Batch alpha"),
        term("sys.term.batch-beta", "Batch beta"),
    ];

    let applied = ops::propose_many(&context, &records).expect("the batch is valid");
    assert_eq!(applied.changes.len(), 2);
    assert_eq!(
        applied.files.len(),
        1,
        "both terms share one canonical file"
    );
    let ledger = sandbox.ledger();
    for key_text in ["sys.term.batch-alpha", "sys.term.batch-beta"] {
        assert!(
            ledger.head(&key(key_text)).is_ok(),
            "{key_text} was written"
        );
    }
    sandbox.assert_canonical();
}

#[test]
fn propose_many_rejects_a_duplicate_without_writing_anything() {
    let sandbox = Sandbox::save_your_skin();
    let context = WriteContext::new(sandbox.akr_dir());
    let before = sandbox.snapshot();
    let records = vec![
        term("sys.term.batch-duplicate", "First"),
        term("sys.term.batch-duplicate", "Second"),
    ];

    let refused = ops::propose_many(&context, &records).expect_err("duplicate batch key");
    assert_eq!(refused.code.as_str(), "AKR-L041");
    assert_eq!(before, sandbox.snapshot(), "a refused batch writes nothing");
}

#[test]
fn propose_refuses_a_record_that_would_not_validate() {
    let sandbox = Sandbox::save_your_skin();
    let context = WriteContext::new(sandbox.akr_dir());

    // No `definition`, which V-008 requires of a term. A bare proposal is refused rather
    // than written with a placeholder body.
    let refused = ops::propose(&context, &key("sys.term.bare"), Kind::Term, "Bare", None)
        .expect_err("a term with no definition does not validate");
    assert_eq!(refused.code.as_str(), "AKR-C031");
    assert!(
        refused
            .diagnostics
            .iter()
            .any(|d| d.code.as_str() == "AKR-T001")
    );
}

#[test]
fn propose_refuses_an_undeclared_namespace() {
    let sandbox = Sandbox::save_your_skin();
    let context = WriteContext::new(sandbox.akr_dir());
    let refused = ops::propose(
        &context,
        &key("nope.term.stranger"),
        Kind::Term,
        "Stranger",
        Some(term("nope.term.stranger", "Stranger")),
    )
    .expect_err("the namespace is not declared");
    assert!(
        refused
            .diagnostics
            .iter()
            .any(|d| d.code.as_str() == "AKR-L004")
    );
}

// -------------------------------------------------------------------------------------
// revise
// -------------------------------------------------------------------------------------

#[test]
fn revise_edits_a_proposed_head_in_place() {
    let sandbox = Sandbox::save_your_skin();
    let context = WriteContext::new(sandbox.akr_dir());
    // `sim.decision.timestep-4ms` is the example's one `proposed` normative head.
    let target = key("sim.decision.timestep-4ms");
    let before = sandbox.ledger().head(&target).expect("the head").id.clone();

    let applied = ops::revise(
        &context,
        &target,
        ReviseMode::Auto,
        &Edits {
            title: Some("Fix the simulator timestep at 4 ms (revised)".to_owned()),
            ..Edits::default()
        },
    )
    .expect("a proposed head is editable");

    assert_eq!(applied.changes[0].kind, ops::ChangeKind::Edited);
    let after = sandbox.ledger();
    let head = after.head(&target).expect("the head");
    assert_eq!(head.id, before, "a proposed head is edited, not duplicated");
    assert!(head.title.ends_with("(revised)"));
    assert_eq!(after.revisions_of(&target).len(), 1);
}

#[test]
fn revise_creates_a_new_revision_from_a_sealed_head() {
    let sandbox = Sandbox::save_your_skin();
    let context = WriteContext::new(sandbox.akr_dir());
    let target = key("sys.term.playable-day");

    let applied = ops::revise(
        &context,
        &target,
        ReviseMode::Auto,
        &Edits {
            title: Some("Playable day, restated".to_owned()),
            ..Edits::default()
        },
    )
    .expect("a sealed head produces a new revision");

    assert_eq!(
        applied.changes.len(),
        2,
        "the new revision and the retired one"
    );
    let after = sandbox.ledger();
    assert_eq!(after.revisions_of(&target).len(), 2);
    let head = after.head(&target).expect("the head");
    assert_eq!(head.id.revision, 2);
    assert_eq!(
        head.state,
        State::Active,
        "a revision keeps the state it inherits from the sealed head (D-043)"
    );
    assert!(
        head.targets(akr_core::model::Relation::Supersedes)
            .iter()
            .any(|t| t.revision == Some(1)),
        "the new revision must declare what it supersedes"
    );
    // Revision 1 is retired in the same write. Leaving it live would be two live heads,
    // and `docs/07` §4 refuses to write a ledger that does not validate.
    let first = after
        .get(&akr_core::model::RevisionId::new(target, 1))
        .expect("revision 1");
    assert_eq!(first.state, State::Superseded);
    sandbox.assert_canonical();
}

#[test]
fn revise_applies_an_explicit_state_to_the_new_revision_of_a_sealed_head() {
    let sandbox = Sandbox::save_your_skin();
    let context = WriteContext::new(sandbox.akr_dir());
    let target = key("sys.work.m3-audio-pass");

    ops::revise(
        &context,
        &target,
        ReviseMode::Auto,
        &Edits {
            state: Some(State::Active),
            ..Edits::default()
        },
    )
    .expect("the requested lifecycle state lands on the successor");

    let after = sandbox.ledger();
    let head = after.head(&target).expect("the successor head");
    assert_eq!(head.id.revision, 2);
    assert_eq!(head.state, State::Active);
    let first = after
        .get(&akr_core::model::RevisionId::new(target, 1))
        .expect("the retired ready revision");
    assert_eq!(first.state, State::Superseded);
}

fn policy_pinning_evidence(key_text: &str, evidence: &str, revision: u32) -> Record {
    let mut record = RecordBuilder::new(key_text, 1, Kind::Policy)
        .title("Pins a sealed evidence record")
        .all_scope()
        .build();
    record.state = State::Active;
    record.content.insert(
        ContentSlot::Rule,
        ContentValue::prose("The demo is what this policy stands on."),
    );
    record.relations.insert(
        Relation::SupportedBy,
        vec![Reference::pinned(key(evidence), revision)],
    );
    record.relations.insert(
        Relation::DerivedFrom,
        vec![Reference::pinned(key(evidence), revision)],
    );
    record
}

#[test]
fn revise_repoints_non_historical_pins_and_leaves_historical_ones() {
    let sandbox = Sandbox::save_your_skin();
    let context = WriteContext::new(sandbox.akr_dir()).with_author("tester");
    let evidence = key("sys.evidence.playable-day-demo");
    let pin = key("sys.policy.evidence-pin");

    ops::propose(
        &context,
        &pin,
        Kind::Policy,
        "Pins a sealed evidence record",
        Some(policy_pinning_evidence(
            "sys.policy.evidence-pin",
            "sys.evidence.playable-day-demo",
            1,
        )),
    )
    .expect("a well-formed pinning policy is accepted");

    let applied = ops::revise(
        &context,
        &evidence,
        ReviseMode::Auto,
        &Edits {
            title: Some("Recorded full-day session, restated".to_owned()),
            state: Some(State::Verified),
            ..Edits::default()
        },
    )
    .expect("the pin must follow in the same write rather than refuse with AKR-L021");

    assert!(
        applied
            .notes
            .iter()
            .any(|note| note.contains("repointed")
                && note.contains("sys.evidence.playable-day-demo/1")),
        "the write names the follow: {:?}",
        applied.notes
    );

    let after = sandbox.ledger();
    let referrer = after.head(&pin).expect("the pinning policy");
    assert_eq!(
        referrer.id.revision, 1,
        "the referrer is not itself revised"
    );
    assert_eq!(
        referrer.targets(Relation::SupportedBy),
        &[Reference::pinned(evidence.clone(), 2)],
        "the live non-historical pin follows onto the successor"
    );
    assert_eq!(
        referrer.targets(Relation::DerivedFrom),
        &[Reference::pinned(evidence, 1)],
        "the historical pin stays on the retired revision"
    );
}

#[test]
fn revise_refuses_an_in_place_edit_of_a_sealed_head() {
    let sandbox = Sandbox::save_your_skin();
    let context = WriteContext::new(sandbox.akr_dir());
    let refused = ops::revise(
        &context,
        &key("sys.term.playable-day"),
        ReviseMode::InPlace,
        &Edits {
            title: Some("nope".to_owned()),
            ..Edits::default()
        },
    )
    .expect_err("sealed bodies are immutable");
    assert_eq!(refused.code.as_str(), "AKR-C032");
}

#[test]
fn revise_refuses_an_unknown_key() {
    let sandbox = Sandbox::save_your_skin();
    let context = WriteContext::new(sandbox.akr_dir());
    let refused = ops::revise(
        &context,
        &key("sys.term.absent"),
        ReviseMode::Auto,
        &Edits::default(),
    )
    .expect_err("there is nothing to revise");
    assert_eq!(refused.code.as_str(), "AKR-L001");
}

// -------------------------------------------------------------------------------------
// supersede
// -------------------------------------------------------------------------------------

/// Proposes a plan and a child that **pins** revision 1 of it.
///
/// Neither committed example has a head with children pinned to it — in both, the
/// children pin the superseded revision, which is what the disposition already accounts
/// for. So the scenario that makes V-017 fire has to be built.
fn plan_with_a_child(context: &WriteContext) {
    let mut plan = RecordBuilder::new("sys.work.demo-plan", 1, Kind::Work)
        .title("Demo plan")
        .build();
    plan.content.insert(
        ContentSlot::Intent,
        ContentValue::prose("A plan with one child."),
    );
    ops::propose(
        context,
        &key("sys.work.demo-plan"),
        Kind::Work,
        "Demo plan",
        Some(plan),
    )
    .expect("the plan proposes");

    let mut child = RecordBuilder::new("sys.work.demo-child", 1, Kind::Work)
        .title("Demo child")
        .state(State::Ready)
        .rel(akr_core::model::Relation::PartOf, "@sys.work.demo-plan/1")
        .build();
    child.content.insert(
        ContentSlot::Intent,
        ContentValue::prose("A child pinned to revision 1."),
    );
    ops::propose(
        context,
        &key("sys.work.demo-child"),
        Kind::Work,
        "Demo child",
        Some(child),
    )
    .expect("the child proposes");
}

#[test]
fn supersede_lists_the_children_it_needs_a_disposition_for() {
    let sandbox = Sandbox::save_your_skin();
    let context = WriteContext::new(sandbox.akr_dir());
    plan_with_a_child(&context);
    let before = sandbox.snapshot();

    let refused = ops::supersede(&context, &key("sys.work.demo-plan"), &[])
        .expect_err("a replan may not drop a child silently");

    assert_eq!(refused.code.as_str(), "AKR-R014");
    assert_eq!(refused.unfinished_children.len(), 1);
    assert_eq!(
        refused.unfinished_children[0].key,
        key("sys.work.demo-child")
    );
    assert_eq!(refused.unfinished_children[0].state, State::Ready);
    assert!(refused.help.is_some_and(|h| h.contains("--disposition")));
    assert_eq!(before, sandbox.snapshot(), "a refusal writes nothing");
}

#[test]
fn supersede_writes_once_every_child_is_dispositioned() {
    let sandbox = Sandbox::save_your_skin();
    let context = WriteContext::new(sandbox.akr_dir());
    plan_with_a_child(&context);

    let applied = ops::supersede(
        &context,
        &key("sys.work.demo-plan"),
        &[DispositionRequest {
            child: key("sys.work.demo-child"),
            outcome: DispositionOutcome::CarriedForward,
            into: Some(key("sys.track.lighting")),
            note: Some("Standing work, not this plan's.".to_owned()),
        }],
    )
    .expect("a replan that accounts for its child is accepted");

    assert_eq!(
        applied.changes.len(),
        2,
        "the old head and the new revision"
    );
    assert!(applied.lock_stale);

    let after = sandbox.ledger();
    let target = key("sys.work.demo-plan");
    assert_eq!(after.revisions_of(&target).len(), 2);
    let head = after.head(&target).expect("the head");
    assert_eq!(head.id.revision, 2);
    assert_eq!(head.dispositions.len(), 1);
    assert_eq!(
        head.dispositions[0].outcome,
        DispositionOutcome::CarriedForward
    );
    let old = after
        .get(&akr_core::model::RevisionId::new(target, 1))
        .expect("the old head");
    assert_eq!(old.state, State::Superseded);
    sandbox.assert_canonical();
}

/// Superseding a plan whose children pin an *earlier* revision needs no disposition: the
/// replan that retired that revision already accounted for them.
#[test]
fn supersede_demands_nothing_when_no_child_pins_the_head() {
    let sandbox = Sandbox::save_your_skin();
    let context = WriteContext::new(sandbox.akr_dir());

    let applied = ops::supersede(&context, &key("sys.work.m3-plan"), &[])
        .expect("no child pins revision 2, so nothing is unaccounted for");
    assert_eq!(applied.changes.len(), 2);
    assert_eq!(
        sandbox
            .ledger()
            .head(&key("sys.work.m3-plan"))
            .expect("the head")
            .id
            .revision,
        3
    );
}

#[test]
fn supersede_with_links_an_already_proposed_replacement_key_atomically() {
    let sandbox = Sandbox::save_your_skin();
    let context = WriteContext::new(sandbox.akr_dir()).with_author("tester");
    let old_key = key("sys.term.legacy-session");
    let new_key = key("sys.term.playable-session");
    ops::propose(
        &context,
        &old_key,
        Kind::Term,
        "Legacy session",
        Some(term("sys.term.legacy-session", "Legacy session")),
    )
    .expect("the old term can be introduced without existing dependants");
    ops::revise(
        &context,
        &old_key,
        ReviseMode::InPlace,
        &Edits {
            state: Some(State::Active),
            ..Edits::default()
        },
    )
    .expect("the old term is accepted before replacement");
    ops::propose(
        &context,
        &new_key,
        Kind::Term,
        "Playable session",
        Some(term("sys.term.playable-session", "Playable session")),
    )
    .expect("the replacement is reviewed as a proposal first");

    let applied = ops::supersede_with(&context, &old_key, &new_key, &[])
        .expect("the graph transition is atomic");
    assert_eq!(applied.changes.len(), 2);

    let ledger = sandbox.ledger();
    assert_eq!(
        ledger.head(&old_key).expect("old head").state,
        State::Superseded
    );
    let replacement = ledger.head(&new_key).expect("replacement head");
    assert_eq!(replacement.state, State::Proposed);
    assert_eq!(
        replacement.targets(akr_core::model::Relation::Supersedes),
        &[Reference::pinned(old_key, 1)]
    );
}

// -------------------------------------------------------------------------------------
// complete
// -------------------------------------------------------------------------------------

#[test]
fn complete_refuses_the_check_no_evidence_satisfies() {
    // The sys-tandem M5 case: everything landed, and a designer still has to sign off.
    let sandbox = Sandbox::sys_tandem();
    let context = WriteContext::new(sandbox.akr_dir());

    let refused = ops::complete(&context, &key("tandem.milestone.m5-one-playable-day"), &[])
        .expect_err("code cannot self-certify a designer's judgement");

    assert_eq!(refused.code.as_str(), "AKR-R022");
    assert_eq!(refused.unsatisfied_checks.len(), 1);
    assert_eq!(
        refused.unsatisfied_checks[0].id,
        "three-seed-designer-signoff"
    );
}

#[test]
fn complete_refuses_a_kind_that_does_not_complete() {
    let sandbox = Sandbox::save_your_skin();
    let context = WriteContext::new(sandbox.akr_dir());
    let refused = ops::complete(&context, &key("sys.policy.tandem-work"), &[])
        .expect_err("policies do not complete");
    assert_eq!(refused.code.as_str(), "AKR-T011");
}

#[test]
fn complete_accepts_a_work_record_with_nothing_left_to_prove() {
    let sandbox = Sandbox::save_your_skin();
    let context = WriteContext::new(sandbox.akr_dir());
    let target = key("lege.work.extract-render-graph");

    let applied =
        ops::complete(&context, &target, &[]).expect("no acceptance block, nothing unmet");
    assert!(matches!(
        applied.changes[0].kind,
        ops::ChangeKind::StateChanged {
            to: State::Completed,
            ..
        }
    ));
    assert_eq!(
        sandbox.ledger().head(&target).expect("the head").state,
        State::Completed
    );
    sandbox.assert_canonical();
}

/// Attaching evidence satisfies the check — and then a *different* rule refuses.
///
/// Completing M3 makes it terminal, and `sys.work.m3-plan/2` still floats
/// `plan_of_record` at it, so V-019 fires. That is correct: a milestone cannot be done
/// while the plan that drives it is live. `docs/07` §6 does not mention the interaction;
/// see the P6 report.
#[test]
fn complete_attaches_evidence_and_then_meets_the_plan_of_record_rule() {
    let sandbox = Sandbox::save_your_skin();
    let context = WriteContext::new(sandbox.akr_dir());
    let before = sandbox.snapshot();

    let refused = ops::complete(
        &context,
        &key("sys.milestone.m3-playable-day"),
        &[(
            "no-placeholder-assets".to_owned(),
            Reference::head(key("sys.evidence.playable-day-demo")),
        )],
    )
    .expect_err("the live plan of record blocks completion");

    assert_eq!(refused.code.as_str(), "AKR-C031");
    assert!(
        refused
            .diagnostics
            .iter()
            .any(|d| d.code.as_str() == "AKR-R021"),
        "the plan of record is the obstacle"
    );
    assert!(
        !refused
            .diagnostics
            .iter()
            .any(|d| d.code.as_str() == "AKR-R022"),
        "the attached evidence did satisfy the acceptance check"
    );
    assert_eq!(before, sandbox.snapshot(), "a refusal writes nothing");
}

// -------------------------------------------------------------------------------------
// abandon
// -------------------------------------------------------------------------------------

#[test]
fn abandon_requires_a_reason() {
    let sandbox = Sandbox::save_your_skin();
    let context = WriteContext::new(sandbox.akr_dir());
    let refused = ops::abandon(&context, &key("lege.work.extract-render-graph"), "  ", &[])
        .expect_err("a silent abandonment is the failure D-017 exists to prevent");
    assert_eq!(refused.code.as_str(), "AKR-C031");
}

#[test]
fn abandon_demands_a_disposition_for_every_unfinished_child() {
    let sandbox = Sandbox::save_your_skin();
    let context = WriteContext::new(sandbox.akr_dir());
    plan_with_a_child(&context);

    let refused = ops::abandon(
        &context,
        &key("sys.work.demo-plan"),
        "no longer the plan",
        &[],
    )
    .expect_err("abandoning a plan with a live child is refused");
    assert_eq!(refused.code.as_str(), "AKR-R014");
    assert_eq!(refused.unfinished_children.len(), 1);
    assert_eq!(
        refused.unfinished_children[0].key,
        key("sys.work.demo-child")
    );
}

#[test]
fn abandon_records_the_reason_in_the_note_slot() {
    let sandbox = Sandbox::save_your_skin();
    let context = WriteContext::new(sandbox.akr_dir());
    let target = key("lege.work.extract-render-graph");

    let applied = ops::abandon(
        &context,
        &target,
        "superseded by the snapshot boundary",
        &[],
    )
    .expect("a childless work item abandons cleanly");
    assert!(matches!(
        applied.changes[0].kind,
        ops::ChangeKind::StateChanged {
            to: State::Abandoned,
            ..
        }
    ));

    let after = sandbox.ledger();
    let head = after.head(&target).expect("the head");
    assert_eq!(head.state, State::Abandoned);
    // D-026: the reason is a rendered slot, not a comment. A comment is excluded from the
    // seal hash and invisible to views; an abandonment reason is durable knowledge.
    assert_eq!(
        head.get(ContentSlot::Note),
        Some(&ContentValue::prose("superseded by the snapshot boundary")),
        "the reason must land in `note`"
    );
    let text = sandbox.read(&applied.files[0]);
    assert!(
        text.contains("note \"\"\""),
        "the note must be a prose slot:\n{text}"
    );
    sandbox.assert_canonical();
}

/// `note` is a planning-kind slot. Setting it on anything else is V-008's business.
#[test]
fn the_note_slot_belongs_to_planning_kinds_only() {
    for kind in Kind::ALL {
        let has_note = kind
            .content_slots()
            .iter()
            .any(|s| s.slot == ContentSlot::Note);
        assert_eq!(
            has_note,
            kind.class() == Class::Planning,
            "{kind}: note is planning-only (D-026)"
        );
    }
}

// -------------------------------------------------------------------------------------
// invariants across operations
// -------------------------------------------------------------------------------------

#[test]
fn every_successful_write_leaves_the_tree_canonical_and_valid() {
    let sandbox = Sandbox::sys_tandem();
    let context = WriteContext::new(sandbox.akr_dir());

    ops::propose(
        &context,
        &key("tandem.term.rota"),
        Kind::Term,
        "Rota",
        Some(term("tandem.term.rota", "Rota")),
    )
    .expect("proposal");
    ops::revise(
        &context,
        &key("tandem.term.rota"),
        ReviseMode::Auto,
        &Edits {
            title: Some("Rota, restated".to_owned()),
            ..Edits::default()
        },
    )
    .expect("revision");

    sandbox.assert_canonical();
    assert!(
        akr_core::validate::validate_all(&sandbox.ledger()).is_empty(),
        "the ledger must still validate after a sequence of writes"
    );
}

#[test]
fn a_repeated_state_change_is_idempotent() {
    let sandbox = Sandbox::save_your_skin();
    let context = WriteContext::new(sandbox.akr_dir());
    let target = key("lege.work.extract-render-graph");

    ops::complete(&context, &target, &[]).expect("first completion");
    let after_first = sandbox.snapshot();
    // Completing an already-completed record is a no-op on disk: the state slot is
    // already `completed`, so the canonical text does not change.
    let second = ops::complete(&context, &target, &[]);
    assert!(second.is_ok(), "completing twice is not an error");
    assert_eq!(
        after_first,
        sandbox.snapshot(),
        "the second write changed nothing"
    );
}

#[test]
fn the_conventional_file_follows_namespace_and_kind() {
    for (kind, expected) in [
        (Kind::Term, "records/sys/terms.akr"),
        (Kind::Policy, "records/sys/policies.akr"),
        (Kind::Evidence, "records/sys/evidence.akr"),
        (Kind::Work, "records/sys/work.akr"),
        (Kind::Question, "records/sys/questions.akr"),
    ] {
        assert_eq!(
            conventional_file(&key("sys.a.b"), kind).to_string_lossy(),
            expected
        );
    }
    // Every kind has a home, and planning kinds share none of it by accident.
    for kind in Kind::ALL {
        let path = conventional_file(&key("sys.a.b"), *kind);
        assert!(
            path.starts_with("records/sys"),
            "{kind}: {}",
            path.display()
        );
        if kind.class() == Class::Planning {
            assert!(path.to_string_lossy().ends_with(".akr"));
        }
    }
}

// -------------------------------------------------------------------------------------
// inherited diagnostics (D-039)
// -------------------------------------------------------------------------------------

/// An evidence record whose artefact is in disposable scratch, written straight to the
/// file the way a ledger authored before V-025 existed looks once `akr scratch prune` has
/// taken the entry: `AKR-T023`, and no way to make the cited path exist again.
fn stranded(slug: &str) -> String {
    format!(
        "\nrecord sys.evidence.{slug}/1 : evidence {{\n    \
         title \"A capture that scratch no longer holds\"\n    \
         state verified\n    \
         result pass\n    \
         method command\n    \
         observed_at git:e806b3f54a2d7091c5e13b8a26f490dc7b135e64\n    \
         command \"cargo test\"\n    \
         artifact \".agent/scratch/{slug}/log.txt\"\n}}\n"
    )
}

fn strand(sandbox: &Sandbox, slugs: &[&str]) {
    let path = sandbox.akr_dir().join("records/sys/evidence.akr");
    let mut text = std::fs::read_to_string(&path).expect("the evidence file is readable");
    for slug in slugs {
        text.push_str(&stranded(slug));
    }
    std::fs::write(&path, text).expect("the evidence file is writable");
}

#[test]
fn a_write_is_not_blocked_by_a_diagnostic_it_inherited() {
    let sandbox = Sandbox::save_your_skin();
    strand(&sandbox, &["pruned-capture"]);
    let context = WriteContext::new(sandbox.akr_dir()).with_author("tester");
    let target = key("sys.term.audit-lane");

    let applied = ops::propose(
        &context,
        &target,
        Kind::Term,
        "Audit lane",
        Some(term("sys.term.audit-lane", "Audit lane")),
    )
    .expect("an unrelated fault is not this write's to answer for");

    assert!(
        sandbox.ledger().head(&target).is_ok(),
        "the proposal reached disk"
    );
    let note = applied
        .notes
        .iter()
        .find(|note| note.contains("did not introduce"))
        .expect("the write says what it is leaving behind");
    assert!(note.contains("AKR-T023"), "and names the code: {note}");
    assert!(note.contains("1 diagnostic"), "and how many: {note}");
    assert!(
        applied.diagnostics.is_empty(),
        "the inherited errors are counted in the note, not re-rendered one by one"
    );
}

#[test]
fn a_write_is_refused_for_the_diagnostic_it_introduces() {
    let sandbox = Sandbox::save_your_skin();
    strand(&sandbox, &["pruned-capture"]);
    let before = sandbox.snapshot();
    let context = WriteContext::new(sandbox.akr_dir()).with_author("tester");

    let mut fresh = RecordBuilder::new("sys.evidence.fresh-capture", 1, Kind::Evidence)
        .title("A capture written into scratch today")
        .build();
    fresh.state = State::Verified;
    fresh.content.insert(
        ContentSlot::Result,
        ContentValue::Enum(Segment::new("pass").expect("a legal result")),
    );
    fresh.content.insert(
        ContentSlot::Method,
        ContentValue::Enum(Segment::new("command").expect("a legal method")),
    );
    fresh.content.insert(
        ContentSlot::ObservedAt,
        ContentValue::Commit(
            Commit::new("git:e806b3f54a2d7091c5e13b8a26f490dc7b135e64").expect("a legal commit"),
        ),
    );
    fresh.content.insert(
        ContentSlot::Artifact,
        ContentValue::Text(".agent/scratch/fresh-capture/log.txt".to_owned()),
    );

    let refusal = ops::propose(
        &context,
        &key("sys.evidence.fresh-capture"),
        Kind::Evidence,
        "A capture written into scratch today",
        Some(fresh),
    )
    .expect_err("citing scratch now is still refused, however broken the ledger already is");

    assert_eq!(
        refusal.diagnostics.len(),
        1,
        "only the new fault is charged to this write: {:?}",
        refusal
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    );
    assert!(refusal.diagnostics[0].message.contains("fresh-capture"));
    assert!(
        refusal.message.contains("would introduce"),
        "the refusal says whose fault it is: {}",
        refusal.message
    );
    assert_eq!(before, sandbox.snapshot(), "a refused write writes nothing");
}

#[test]
fn one_of_two_inherited_faults_can_be_repaired_at_a_time() {
    let sandbox = Sandbox::save_your_skin();
    strand(&sandbox, &["pruned-one", "pruned-two"]);
    let context = WriteContext::new(sandbox.akr_dir()).with_author("tester");
    let target = key("sys.evidence.pruned-one");

    let mut repaired = RecordBuilder::new("sys.evidence.pruned-one", 2, Kind::Evidence)
        .title("A capture that scratch no longer holds")
        .build();
    repaired.content.insert(
        ContentSlot::Result,
        ContentValue::Enum(Segment::new("pass").expect("a legal result")),
    );
    repaired.content.insert(
        ContentSlot::Method,
        ContentValue::Enum(Segment::new("command").expect("a legal method")),
    );
    repaired.content.insert(
        ContentSlot::ObservedAt,
        ContentValue::Commit(
            Commit::new("git:e806b3f54a2d7091c5e13b8a26f490dc7b135e64").expect("a legal commit"),
        ),
    );

    let applied = ops::revise(
        &context,
        &target,
        ReviseMode::Auto,
        &Edits {
            title: Some("A capture that scratch no longer holds".to_owned()),
            state: Some(State::Verified),
            replace_with: Some(Box::new(repaired)),
        },
    )
    .expect("dropping the dangling artefact is the repair, and it must be reachable");

    let note = applied
        .notes
        .iter()
        .find(|note| note.contains("did not introduce"))
        .expect("the other fault is still standing and still said aloud");
    assert!(
        note.contains("1 diagnostic"),
        "and the count has come down by one: {note}"
    );
    sandbox.assert_canonical();
}

//! Recording evidence, and attaching it to an acceptance check.
//!
//! The library operation behind `akr evidence add` and behind `akr complete --check`.
//! Two properties matter more than the mechanics:
//!
//! - **Evidence never declares what it verifies** (D-016). The request type has no field
//!   for it, and the only way to create the link is through the check.
//! - **A failed write changes nothing** (`docs/07-cli.md` §4). Every rejection is asserted
//!   to leave the ledger it was given untouched.

use akr_core::diagnostics::codes;
use akr_core::evidence::{AddEvidence, EvidenceError, acceptance_of, add, add_and_attach, attach};
use akr_core::model::{
    Ancestry, CheckMethod, Commit, ContentSlot, ContentValue, EvidenceResult, Kind, Ledger,
    Project, RecordBuilder, Reference, RevisionId, State, key, reference,
};
use akr_core::resolve::{BuildInputs, ResolvedModel, Verdict};

const C1: &str = "3f0a1c9d5b7e2648a0d4f1b8c36e9752ad014b6f";
const C2: &str = "7c41d0ba92e6f37518a3cd406b5e2f91d8074a63";
const C3: &str = "b2e58f1406c7a9d3e41b60258fa3d7c6195e0b48";

fn commit(hash: &str) -> Commit {
    Commit::new(hash).expect("a full hash")
}

fn id(text: &str, revision: u32) -> RevisionId {
    RevisionId::new(key(text), revision)
}

/// A milestone with one unsatisfied acceptance check.
fn ledger_with_milestone() -> Ledger {
    let mut ledger = Ledger::new(Project::new("sys", &["sys"]));
    let mut milestone = RecordBuilder::new("sys.milestone.m1", 1, Kind::Milestone)
        .title("M1 — the court speaks")
        .filled()
        .state(State::Active)
        .build();
    milestone.acceptance = Some(acceptance_of(&[("squelch-audit", CheckMethod::Command)]));
    ledger.insert(milestone);
    ledger
}

fn request(key_text: &str) -> AddEvidence {
    AddEvidence::new(
        key(key_text),
        "The squelch audit passes",
        EvidenceResult::Pass,
        CheckMethod::Command,
        commit(C3),
    )
    .command("cargo test -p sys_game_bridge --test squelch_audit")
    .artifact("artifacts/2026-08-03-squelch.log")
    .summary("Both previously ignored assertions now run and pass.")
}

// -------------------------------------------------------------------------------------
// Adding
// -------------------------------------------------------------------------------------

#[test]
fn adding_evidence_creates_a_verified_revision_one() {
    let ledger = ledger_with_milestone();
    let written = add(&ledger, &request("sys.evidence.squelch-audit")).expect("adds");

    assert_eq!(written.touched, vec![id("sys.evidence.squelch-audit", 1)]);
    let record = written
        .ledger
        .get(&id("sys.evidence.squelch-audit", 1))
        .expect("the record exists");
    assert_eq!(record.kind, Kind::Evidence);
    // Empirical kinds have no proposal state: an observation either was made or was not.
    assert_eq!(record.state, State::Verified);
    assert!(record.is_live());
    assert_eq!(
        record
            .get(ContentSlot::ObservedAt)
            .and_then(ContentValue::as_commit),
        Some(&commit(C3))
    );
}

#[test]
fn the_request_carries_every_evidence_slot_and_no_others() {
    let written = add(
        &ledger_with_milestone(),
        &request("sys.evidence.squelch-audit"),
    )
    .expect("adds");
    let record = written
        .ledger
        .get(&id("sys.evidence.squelch-audit", 1))
        .expect("exists");
    for slot in [
        ContentSlot::Result,
        ContentSlot::Method,
        ContentSlot::ObservedAt,
        ContentSlot::Command,
        ContentSlot::Artifact,
        ContentSlot::Summary,
    ] {
        assert!(record.get(slot).is_some(), "{slot:?} is recorded");
    }
    // The evidence describes only the observation. It has no relations at all.
    assert!(
        record.relations.is_empty(),
        "evidence never declares what it verifies (D-016)"
    );
}

#[test]
fn adding_an_existing_key_is_refused() {
    let ledger = ledger_with_milestone();
    let written = add(&ledger, &request("sys.evidence.squelch-audit")).expect("adds");
    let error =
        add(&written.ledger, &request("sys.evidence.squelch-audit")).expect_err("the key is taken");
    assert!(matches!(error, EvidenceError::KeyExists(_)));
    assert!(error.to_string().contains("akr revise"));
}

#[test]
fn a_failing_result_is_a_legitimate_record() {
    let ledger = ledger_with_milestone();
    let mut failing = request("sys.evidence.squelch-audit");
    failing.result = EvidenceResult::Fail;
    let written = add(&ledger, &failing).expect("a failure is worth recording");
    let record = written
        .ledger
        .get(&id("sys.evidence.squelch-audit", 1))
        .expect("exists");
    assert_eq!(
        record
            .get(ContentSlot::Result)
            .and_then(ContentValue::as_enum)
            .map(|e| e.as_str().to_owned()),
        Some("fail".to_owned())
    );
}

// -------------------------------------------------------------------------------------
// Attaching
// -------------------------------------------------------------------------------------

#[test]
fn attaching_writes_verified_by_on_the_check_not_on_the_evidence() {
    let ledger = ledger_with_milestone();
    let staged = add(&ledger, &request("sys.evidence.squelch-audit")).expect("adds");
    let written = attach(
        &staged.ledger,
        &key("sys.milestone.m1"),
        "squelch-audit",
        &reference("@sys.evidence.squelch-audit/1"),
        true,
    )
    .expect("attaches");

    let milestone = written
        .ledger
        .get(&id("sys.milestone.m1", 1))
        .expect("exists");
    let check = &milestone.acceptance.as_ref().expect("acceptance").checks[0];
    assert_eq!(
        check.verified_by,
        vec![reference("@sys.evidence.squelch-audit/1")]
    );

    let evidence = written
        .ledger
        .get(&id("sys.evidence.squelch-audit", 1))
        .expect("exists");
    assert!(
        evidence.relations.is_empty(),
        "the link runs one way only (D-016)"
    );
}

#[test]
fn attaching_pins_the_citation_by_default() {
    // D-009 guidance: pin when citing evidence. A floating citation would silently
    // re-point if the evidence key gained a revision, which a closed check must not do.
    let staged = add(
        &ledger_with_milestone(),
        &request("sys.evidence.squelch-audit"),
    )
    .expect("adds");
    let pinned = attach(
        &staged.ledger,
        &key("sys.milestone.m1"),
        "squelch-audit",
        &reference("@sys.evidence.squelch-audit"),
        true,
    )
    .expect("attaches");
    let check = &pinned
        .ledger
        .get(&id("sys.milestone.m1", 1))
        .expect("exists")
        .acceptance
        .as_ref()
        .expect("acceptance")
        .checks[0];
    assert!(check.verified_by[0].is_pinned());

    let floating = attach(
        &staged.ledger,
        &key("sys.milestone.m1"),
        "squelch-audit",
        &reference("@sys.evidence.squelch-audit"),
        false,
    )
    .expect("attaches");
    let check = &floating
        .ledger
        .get(&id("sys.milestone.m1", 1))
        .expect("exists")
        .acceptance
        .as_ref()
        .expect("acceptance")
        .checks[0];
    assert!(!check.verified_by[0].is_pinned());
}

#[test]
fn attaching_twice_does_not_duplicate_the_citation() {
    let staged = add(
        &ledger_with_milestone(),
        &request("sys.evidence.squelch-audit"),
    )
    .expect("adds");
    let once = attach(
        &staged.ledger,
        &key("sys.milestone.m1"),
        "squelch-audit",
        &reference("@sys.evidence.squelch-audit/1"),
        true,
    )
    .expect("attaches");
    let twice = attach(
        &once.ledger,
        &key("sys.milestone.m1"),
        "squelch-audit",
        &reference("@sys.evidence.squelch-audit/1"),
        true,
    )
    .expect("attaches again");
    let check = &twice
        .ledger
        .get(&id("sys.milestone.m1", 1))
        .expect("exists")
        .acceptance
        .as_ref()
        .expect("acceptance")
        .checks[0];
    assert_eq!(check.verified_by.len(), 1);
}

#[test]
fn attaching_to_an_unknown_check_is_refused() {
    let staged = add(
        &ledger_with_milestone(),
        &request("sys.evidence.squelch-audit"),
    )
    .expect("adds");
    let error = attach(
        &staged.ledger,
        &key("sys.milestone.m1"),
        "no-such-check",
        &reference("@sys.evidence.squelch-audit/1"),
        true,
    )
    .expect_err("no such check");
    assert!(matches!(error, EvidenceError::UnknownCheck { .. }));
}

#[test]
fn attaching_a_record_that_is_not_evidence_is_refused() {
    // V-005: `verified_by`'s range is `evidence` and nothing else.
    let mut ledger = ledger_with_milestone();
    ledger.insert(
        RecordBuilder::new("sys.obs.bridge", 1, Kind::Observation)
            .filled()
            .build(),
    );
    let error = attach(
        &ledger,
        &key("sys.milestone.m1"),
        "squelch-audit",
        &reference("@sys.obs.bridge/1"),
        true,
    )
    .expect_err("not evidence");
    assert!(matches!(error, EvidenceError::NotEvidence { kind, .. } if kind == Kind::Observation));
}

#[test]
fn attaching_an_unresolvable_reference_is_refused() {
    let error = attach(
        &ledger_with_milestone(),
        &key("sys.milestone.m1"),
        "squelch-audit",
        &reference("@sys.evidence.absent/1"),
        true,
    )
    .expect_err("does not resolve");
    assert!(matches!(error, EvidenceError::UnknownEvidence(_)));
}

#[test]
fn attaching_to_an_unknown_owner_is_refused() {
    let staged = add(
        &ledger_with_milestone(),
        &request("sys.evidence.squelch-audit"),
    )
    .expect("adds");
    let error = attach(
        &staged.ledger,
        &key("sys.milestone.absent"),
        "squelch-audit",
        &reference("@sys.evidence.squelch-audit/1"),
        true,
    )
    .expect_err("no such owner");
    assert!(matches!(error, EvidenceError::UnknownOwner(_)));
}

// -------------------------------------------------------------------------------------
// The write pipeline: validate the result, or change nothing
// -------------------------------------------------------------------------------------

#[test]
fn a_write_that_would_not_validate_changes_nothing() {
    // `docs/07-cli.md` §4: validation is of the *resulting* ledger, and a failure leaves
    // the working tree byte-identical.
    let mut ledger = ledger_with_milestone();
    // A resolved question with no `resolves` edge: V-011 fails, so any write to this
    // ledger fails with it, and nothing is applied.
    ledger.insert(
        RecordBuilder::new("sys.question.broken", 1, Kind::Question)
            .filled()
            .state(State::Resolved)
            .build(),
    );
    let before = ledger.clone();

    let error = add(&ledger, &request("sys.evidence.squelch-audit")).expect_err("invalid result");
    match &error {
        EvidenceError::WouldNotValidate(diagnostics) => {
            assert!(!diagnostics.is_empty());
            assert!(
                diagnostics
                    .iter()
                    .all(|d| d.severity == akr_core::diagnostics::Severity::Error)
            );
        }
        other => panic!("expected a validation failure, got {other:?}"),
    }
    assert!(error.to_string().contains("nothing was written"));
    assert_eq!(ledger, before, "the ledger it was given is untouched");
}

#[test]
fn add_and_attach_apply_together_or_not_at_all() {
    let ledger = ledger_with_milestone();
    let written = add_and_attach(
        &ledger,
        &request("sys.evidence.squelch-audit"),
        &key("sys.milestone.m1"),
        "squelch-audit",
    )
    .expect("both halves");
    assert_eq!(
        written.touched,
        vec![
            id("sys.evidence.squelch-audit", 1),
            id("sys.milestone.m1", 1)
        ]
    );
    let check = &written
        .ledger
        .get(&id("sys.milestone.m1", 1))
        .expect("exists")
        .acceptance
        .as_ref()
        .expect("acceptance")
        .checks[0];
    assert_eq!(check.verified_by.len(), 1);

    // A bad check identifier leaves no orphan evidence record behind.
    let failed = add_and_attach(
        &ledger,
        &request("sys.evidence.squelch-audit"),
        &key("sys.milestone.m1"),
        "no-such-check",
    )
    .expect_err("no such check");
    assert!(matches!(failed, EvidenceError::UnknownCheck { .. }));
    assert!(ledger.get(&id("sys.evidence.squelch-audit", 1)).is_none());
}

// -------------------------------------------------------------------------------------
// The verdict this all exists to move
// -------------------------------------------------------------------------------------

#[test]
fn attaching_evidence_turns_an_unsatisfied_check_into_a_satisfied_one() {
    let ledger = ledger_with_milestone();
    let inputs = BuildInputs::default();

    let before = ResolvedModel::build(&ledger, &inputs);
    let verdict = &before.checks_of(&id("sys.milestone.m1", 1))[0].verdict;
    assert_eq!(*verdict, Verdict::NoEvidence);

    let written = add_and_attach(
        &ledger,
        &request("sys.evidence.squelch-audit"),
        &key("sys.milestone.m1"),
        "squelch-audit",
    )
    .expect("attaches");
    let after = ResolvedModel::build(&written.ledger, &inputs);
    assert!(
        after.checks_of(&id("sys.milestone.m1", 1))[0]
            .verdict
            .is_satisfied()
    );
}

#[test]
fn evidence_older_than_the_last_content_change_does_not_satisfy() {
    // The condition that stops a passing test from before a redefinition closing the
    // milestone it no longer describes (D-016). With P5, the facts are real.
    let mut ledger = ledger_with_milestone();
    ledger.facts.ancestry =
        Ancestry::from_pairs(vec![(commit(C2), commit(C1)), (commit(C3), commit(C2))]);
    // The milestone was redefined at C3; the evidence was observed at C1.
    ledger
        .facts
        .last_change
        .insert(id("sys.milestone.m1", 1), commit(C3));

    let mut old = request("sys.evidence.squelch-audit");
    old.observed_at = commit(C1);
    let written =
        add_and_attach(&ledger, &old, &key("sys.milestone.m1"), "squelch-audit").expect("attaches");

    let model = ResolvedModel::build(&written.ledger, &BuildInputs::default());
    let verdict = &model.checks_of(&id("sys.milestone.m1", 1))[0].verdict;
    match verdict {
        Verdict::TooOld {
            by,
            observed_at,
            last_change,
        } => {
            assert_eq!(by, &id("sys.evidence.squelch-audit", 1));
            assert_eq!(observed_at, &commit(C1));
            assert_eq!(last_change, &commit(C3));
        }
        other => panic!("expected a too-old verdict, got {other:?}"),
    }
}

#[test]
fn committed_evidence_does_not_satisfy_an_uncommitted_redefinition() {
    // A revision written but not committed has no last-change commit. That used to read
    // as "no git facts" and take the evidence at its word, so re-citing evidence from
    // before a redefinition read as satisfied until the commit, and too old after it.
    let mut ledger = ledger_with_milestone();
    ledger.facts.ancestry =
        Ancestry::from_pairs(vec![(commit(C2), commit(C1)), (commit(C3), commit(C2))]);
    ledger.facts.uncommitted.insert(id("sys.milestone.m1", 1));

    let mut old = request("sys.evidence.squelch-audit");
    old.observed_at = commit(C3);
    let mut written =
        add_and_attach(&ledger, &old, &key("sys.milestone.m1"), "squelch-audit").expect("attaches");
    // The evidence itself is committed: it has a last change of its own.
    written
        .ledger
        .facts
        .last_change
        .insert(id("sys.evidence.squelch-audit", 1), commit(C3));

    let model = ResolvedModel::build(&written.ledger, &BuildInputs::default());
    assert_eq!(
        model.checks_of(&id("sys.milestone.m1", 1))[0].verdict,
        Verdict::TooOldUncommitted {
            by: id("sys.evidence.squelch-audit", 1),
            observed_at: commit(C3),
        }
    );

    // Uncommitted alongside the redefinition, the same evidence is the prepared-change
    // shape: the two will land in one commit, and it satisfies the check.
    written
        .ledger
        .facts
        .last_change
        .remove(&id("sys.evidence.squelch-audit", 1));
    written
        .ledger
        .facts
        .uncommitted
        .insert(id("sys.evidence.squelch-audit", 1));
    let model = ResolvedModel::build(&written.ledger, &BuildInputs::default());
    assert!(
        model.checks_of(&id("sys.milestone.m1", 1))[0]
            .verdict
            .is_satisfied()
    );
}

#[test]
fn co_committed_evidence_satisfies_the_prepared_change() {
    let mut ledger = ledger_with_milestone();
    ledger.facts.ancestry =
        Ancestry::from_pairs(vec![(commit(C2), commit(C1)), (commit(C3), commit(C2))]);
    ledger
        .facts
        .last_change
        .insert(id("sys.milestone.m1", 1), commit(C3));

    let mut evidence = request("sys.evidence.squelch-audit");
    evidence.observed_at = commit(C2);
    let mut written = add_and_attach(
        &ledger,
        &evidence,
        &key("sys.milestone.m1"),
        "squelch-audit",
    )
    .expect("attaches");
    written
        .ledger
        .facts
        .last_change
        .insert(id("sys.evidence.squelch-audit", 1), commit(C3));

    let model = ResolvedModel::build(&written.ledger, &BuildInputs::default());
    assert!(
        model.checks_of(&id("sys.milestone.m1", 1))[0]
            .verdict
            .is_satisfied()
    );
}

#[test]
fn completing_with_an_unsatisfied_check_is_still_akr_r022() {
    // The rule V-020 enforces, now that the facts behind it are real.
    let mut ledger = ledger_with_milestone();
    let records: Vec<_> = ledger
        .records()
        .iter()
        .map(|record| {
            let mut copy = record.clone();
            if copy.id == id("sys.milestone.m1", 1) {
                copy.state = State::Completed;
            }
            copy
        })
        .collect();
    ledger = Ledger::new(ledger.project.clone());
    ledger.extend(records);

    let model = ResolvedModel::build(&ledger, &BuildInputs::default());
    assert!(
        model.diagnostics.iter().any(|d| d.code == codes::R022),
        "a milestone completed on an unsatisfied check is AKR-R022"
    );
}

#[test]
fn a_reference_to_the_new_evidence_resolves() {
    let written = add_and_attach(
        &ledger_with_milestone(),
        &request("sys.evidence.squelch-audit"),
        &key("sys.milestone.m1"),
        "squelch-audit",
    )
    .expect("attaches");
    let resolved = written
        .ledger
        .resolve(&Reference::pinned(key("sys.evidence.squelch-audit"), 1))
        .expect("resolves")
        .expect("exists");
    assert_eq!(resolved.id, id("sys.evidence.squelch-audit", 1));
}

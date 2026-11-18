//! The resolved model: linking, heads, supersession chains, and the resolution log.
//!
//! These tests build ledgers with [`RecordBuilder`] rather than text, so they exercise
//! stages C and D in isolation from the parser. The text-driven end of the same code is
//! `tests/worked_example.rs` and `tests/fixture_corpus.rs`.

use akr_core::lock::Lock;
use akr_core::model::{
    Kind, Ledger, Outcome, Project, RecordBuilder, Relation, RevisionId, State, key,
};
use akr_core::resolve::{BuildInputs, RefSite, ResolvedModel, SourceFile, UNKNOWN_HASH, link};
use std::collections::BTreeMap;

fn id(text: &str, revision: u32) -> RevisionId {
    RevisionId::new(key(text), revision)
}

fn inputs() -> BuildInputs {
    BuildInputs {
        tool: "akr 0.1.0".to_owned(),
        grammar: "0.1".to_owned(),
        vocabulary: "0.1".to_owned(),
        ..BuildInputs::default()
    }
}

/// A plan superseded once, with a dispositioned child and a policy it implements.
fn planning_ledger() -> Ledger {
    let mut ledger = Ledger::new(Project::new("save-your-skin", &["sys"]));
    ledger.extend([
        RecordBuilder::new("sys.work.m3-plan", 1, Kind::Work)
            .filled()
            .state(State::Superseded)
            .file(".akr/records/sys/work.akr")
            .build(),
        RecordBuilder::new("sys.work.m3-plan", 2, Kind::Work)
            .filled()
            .state(State::Active)
            .file(".akr/records/sys/work.akr")
            .rel(Relation::Supersedes, "@sys.work.m3-plan/1")
            .rel(Relation::Implements, "@sys.policy.tandem-work")
            .rel(Relation::PlanOfRecord, "@sys.milestone.m3-playable-day")
            .disposition(
                "@sys.work.m3-lighting-pass",
                Outcome::CarriedForward,
                Some("@sys.track.lighting"),
            )
            .build(),
        RecordBuilder::new("sys.work.m3-lighting-pass", 1, Kind::Work)
            .filled()
            .state(State::Ready)
            .file(".akr/records/sys/work.akr")
            .rel(Relation::PartOf, "@sys.work.m3-plan/1")
            .build(),
        RecordBuilder::new("sys.policy.tandem-work", 1, Kind::Policy)
            .filled()
            .state(State::Active)
            .file(".akr/records/sys/policies.akr")
            .build(),
        RecordBuilder::new("sys.track.lighting", 1, Kind::Track)
            .filled()
            .state(State::Active)
            .file(".akr/records/sys/tracks.akr")
            .build(),
        RecordBuilder::new("sys.milestone.m3-playable-day", 1, Kind::Milestone)
            .filled()
            .state(State::Active)
            .file(".akr/records/sys/milestones.akr")
            .build(),
    ]);
    ledger
}

// -------------------------------------------------------------------------------------
// Heads
// -------------------------------------------------------------------------------------

#[test]
fn heads_come_from_the_ledger_not_a_second_algorithm() {
    let ledger = planning_ledger();
    let model = ResolvedModel::build(&ledger, &inputs());
    for key_ref in ledger.keys() {
        let expected = ledger.head(key_ref).expect("a head").id.clone();
        assert_eq!(model.heads.get(key_ref), Some(&expected));
    }
    assert!(model.head_errors.is_empty());
    assert!(model.is_head(&id("sys.work.m3-plan", 2)));
    assert!(!model.is_head(&id("sys.work.m3-plan", 1)));
}

#[test]
fn a_key_with_no_live_revision_still_has_a_head() {
    // Two-tier resolution (`docs/04` §3): finishing a milestone must not break every
    // reference to it.
    let mut ledger = Ledger::new(Project::new("p", &["sys"]));
    ledger.insert(
        RecordBuilder::new("sys.milestone.m1", 1, Kind::Milestone)
            .filled()
            .state(State::Completed)
            .build(),
    );
    let model = ResolvedModel::build(&ledger, &inputs());
    assert_eq!(
        model.heads.get(&key("sys.milestone.m1")),
        Some(&id("sys.milestone.m1", 1))
    );
    assert!(model.head_errors.is_empty());
}

#[test]
fn two_live_revisions_are_a_head_error_and_a_diagnostic() {
    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.extend([
        RecordBuilder::new("fx.policy.a", 1, Kind::Policy)
            .filled()
            .state(State::Active)
            .build(),
        RecordBuilder::new("fx.policy.a", 2, Kind::Policy)
            .filled()
            .state(State::Active)
            .build(),
    ]);
    let model = ResolvedModel::build(&ledger, &inputs());
    assert!(!model.heads.contains_key(&key("fx.policy.a")));
    assert!(model.head_errors.contains_key(&key("fx.policy.a")));
    assert!(
        model
            .diagnostics
            .iter()
            .any(|d| d.code == akr_core::diagnostics::codes::R001),
        "never a newest-wins tiebreak"
    );
}

// -------------------------------------------------------------------------------------
// Supersession chains
// -------------------------------------------------------------------------------------

#[test]
fn supersession_chains_run_oldest_first() {
    let ledger = planning_ledger();
    let model = ResolvedModel::build(&ledger, &inputs());
    assert_eq!(
        model.supersession.get(&key("sys.work.m3-plan")),
        Some(&vec![id("sys.work.m3-plan", 1), id("sys.work.m3-plan", 2)])
    );
}

#[test]
fn every_key_has_a_chain_containing_every_revision() {
    let ledger = planning_ledger();
    let model = ResolvedModel::build(&ledger, &inputs());
    for key_ref in ledger.keys() {
        let chain = model.supersession.get(key_ref).expect("a chain");
        assert_eq!(chain.len(), ledger.revisions_of(key_ref).len());
    }
}

#[test]
fn current_walks_forward_and_history_walks_back() {
    let ledger = planning_ledger();
    let model = ResolvedModel::build(&ledger, &inputs());
    assert_eq!(
        model.current(&id("sys.work.m3-plan", 1)),
        id("sys.work.m3-plan", 2)
    );
    assert_eq!(
        model.current(&id("sys.work.m3-plan", 2)),
        id("sys.work.m3-plan", 2)
    );
    assert_eq!(
        model.history(&id("sys.work.m3-plan", 2)),
        vec![id("sys.work.m3-plan", 2), id("sys.work.m3-plan", 1)]
    );
    assert_eq!(
        model.history(&id("sys.work.m3-plan", 1)),
        vec![id("sys.work.m3-plan", 1)]
    );
}

#[test]
fn a_supersession_cycle_does_not_hang() {
    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.extend([
        RecordBuilder::new("fx.policy.a", 1, Kind::Policy)
            .filled()
            .state(State::Superseded)
            .rel(Relation::Supersedes, "@fx.policy.a/2")
            .build(),
        RecordBuilder::new("fx.policy.a", 2, Kind::Policy)
            .filled()
            .state(State::Superseded)
            .rel(Relation::Supersedes, "@fx.policy.a/1")
            .build(),
    ]);
    let model = ResolvedModel::build(&ledger, &inputs());
    assert_eq!(model.supersession[&key("fx.policy.a")].len(), 2);
    assert!(
        model
            .diagnostics
            .iter()
            .any(|d| d.code == akr_core::diagnostics::codes::R011)
    );
}

// -------------------------------------------------------------------------------------
// Linking
// -------------------------------------------------------------------------------------

#[test]
fn every_reference_is_linked_and_labelled_with_its_slot() {
    let ledger = planning_ledger();
    let edges = link(&ledger);
    let sites: Vec<String> = edges
        .iter()
        .filter(|e| e.from == id("sys.work.m3-plan", 2))
        .map(|e| format!("{} -> {}", e.site.slot_name(), e.reference))
        .collect();
    assert_eq!(
        sites,
        [
            // Relation slots first, in `Relation` order — the vocabulary's declaration
            // order, not alphabetical — then dispositions.
            "supersedes -> @sys.work.m3-plan/1",
            "implements -> @sys.policy.tandem-work",
            "plan_of_record -> @sys.milestone.m3-playable-day",
            "disposition -> @sys.work.m3-lighting-pass",
            "into -> @sys.track.lighting",
        ],
        "§2.3 needs the slot, not just the relation"
    );
}

#[test]
fn disposition_and_into_are_slots_without_being_relations() {
    assert_eq!(RefSite::DispositionTarget.slot_name(), "disposition");
    assert_eq!(RefSite::DispositionInto.slot_name(), "into");
    assert_eq!(RefSite::DispositionTarget.relation(), None);
    assert_eq!(RefSite::Scope.slot_name(), "scope");
    assert_eq!(
        RefSite::Relation(Relation::DependsOn).relation(),
        Some(Relation::DependsOn)
    );
}

#[test]
fn linking_records_whether_a_reference_was_pinned() {
    let edges = link(&planning_ledger());
    let pinned = edges
        .iter()
        .find(|e| {
            e.reference.to_string() == "@sys.work.m3-plan/1" && e.site.slot_name() == "supersedes"
        })
        .expect("the supersedes edge");
    assert!(pinned.pinned);
    let floating = edges
        .iter()
        .find(|e| e.site.slot_name() == "implements")
        .expect("the implements edge");
    assert!(!floating.pinned);
    assert_eq!(floating.to, Some(id("sys.policy.tandem-work", 1)));
}

#[test]
fn linking_is_independent_of_insertion_order() {
    let ledger = planning_ledger();
    let baseline = link(&ledger);
    let mut reversed = Ledger::new(ledger.project.clone());
    let mut records = ledger.records().to_vec();
    records.reverse();
    reversed.extend(records);
    assert_eq!(link(&reversed), baseline);
}

// -------------------------------------------------------------------------------------
// The resolution log
// -------------------------------------------------------------------------------------

#[test]
fn only_floating_references_are_locked() {
    // §2.3: a pinned reference cannot change what it points at, so locking it is noise.
    let ledger = planning_ledger();
    let model = ResolvedModel::build(&ledger, &inputs());
    assert!(
        !model
            .resolutions
            .iter()
            .any(|r| r.slot == "supersedes" || r.slot == "part_of"),
        "the only pinned references in this ledger are supersedes and part_of"
    );
    // The log is keyed by (referring revision, slot, target key), so within one referring
    // revision the slots come out in name order.
    let slots: Vec<&str> = model.resolutions.iter().map(|r| r.slot.as_str()).collect();
    assert_eq!(
        slots,
        ["disposition", "implements", "into", "plan_of_record"]
    );
}

#[test]
fn resolutions_are_deduplicated_per_referring_revision_slot_and_target_key() {
    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.extend([
        RecordBuilder::new("fx.work.a", 1, Kind::Work)
            .filled()
            .rel(Relation::DependsOn, "@fx.policy.p")
            .rel(Relation::DependsOn, "@fx.policy.p")
            .build(),
        RecordBuilder::new("fx.policy.p", 1, Kind::Policy)
            .filled()
            .state(State::Active)
            .build(),
    ]);
    let model = ResolvedModel::build(&ledger, &inputs());
    assert_eq!(model.resolutions.len(), 1);
}

#[test]
fn a_revision_with_no_canonical_text_gets_the_unknown_hash() {
    let ledger = planning_ledger();
    let model = ResolvedModel::build(&ledger, &inputs());
    assert!(!model.missing_hashes.is_empty());
    assert!(model.content_hashes.is_empty());
    let lock = model.to_lock();
    assert!(lock.seals.iter().all(|s| s.hash.0 == UNKNOWN_HASH));
    assert!(lock.resolutions.iter().all(|r| r.hash.0 == UNKNOWN_HASH));
}

#[test]
fn canonical_text_produces_real_hashes() {
    let ledger = planning_ledger();
    let text = "record sys.policy.tandem-work/1 : policy {\n    title \"t\"\n}\n";
    let mut with_text = inputs();
    with_text
        .canonical_text
        .insert(id("sys.policy.tandem-work", 1), text.to_owned());
    let model = ResolvedModel::build(&ledger, &with_text);
    let hash = model
        .content_hash(&id("sys.policy.tandem-work", 1))
        .expect("hashed");
    assert_eq!(hash, &akr_core::hash::content_hash(text));
    assert!(
        !model
            .missing_hashes
            .contains(&id("sys.policy.tandem-work", 1))
    );
}

// -------------------------------------------------------------------------------------
// The lock the model implies
// -------------------------------------------------------------------------------------

#[test]
fn to_lock_seals_every_non_proposed_revision_and_no_others() {
    let mut ledger = planning_ledger();
    ledger.insert(
        RecordBuilder::new("sys.work.draft", 1, Kind::Work)
            .filled()
            .state(State::Proposed)
            .build(),
    );
    let lock = ResolvedModel::build(&ledger, &inputs()).to_lock();
    let sealed: Vec<String> = lock.seals.iter().map(|s| s.id.to_string()).collect();
    assert!(sealed.contains(&"sys.work.m3-plan/1".to_owned()));
    assert!(sealed.contains(&"sys.work.m3-plan/2".to_owned()));
    assert!(!sealed.contains(&"sys.work.draft/1".to_owned()));
    assert_eq!(sealed.len(), ledger.records().len() - 1);
}

#[test]
fn to_lock_carries_the_build_metadata_and_the_source_graph() {
    let ledger = planning_ledger();
    let mut with_sources = inputs();
    with_sources.sources = vec![SourceFile {
        path: ".akr/project.akr".to_owned(),
        hash: akr_core::hash::source_file_hash(b"akr 0.1\n"),
        records: 0,
        byte_len: 8,
    }];
    let model = ResolvedModel::build(&ledger, &with_sources);
    let lock = model.to_lock();
    assert_eq!(lock.project, "save-your-skin");
    assert_eq!(lock.build.tool, "akr 0.1.0");
    assert_eq!(lock.build.source_graph, model.source_graph);
    assert_eq!(lock.sources.len(), 1);
}

#[test]
fn to_lock_is_independent_of_insertion_order() {
    let ledger = planning_ledger();
    let baseline = ResolvedModel::build(&ledger, &inputs()).to_lock().render();
    let mut reversed = Ledger::new(ledger.project.clone());
    let mut records = ledger.records().to_vec();
    records.reverse();
    reversed.extend(records);
    assert_eq!(
        ResolvedModel::build(&reversed, &inputs())
            .to_lock()
            .render(),
        baseline
    );
}

#[test]
fn a_generated_lock_verifies_against_itself() {
    let ledger = planning_ledger();
    let lock = ResolvedModel::build(&ledger, &inputs()).to_lock();
    let parsed = Lock::parse(&lock.render()).expect("round trip");
    assert!(parsed.verify(&lock).is_empty());
}

#[test]
fn apply_facts_marks_the_lock_present_and_fills_every_seal() {
    let mut ledger = planning_ledger();
    let lock = ResolvedModel::build(&ledger, &inputs()).to_lock();
    let computed: BTreeMap<_, _> = ledger
        .records()
        .iter()
        .filter(|r| r.is_sealed())
        .map(|r| (r.id.clone(), akr_core::hash::content_hash("x")))
        .collect();
    lock.apply_facts(&mut ledger, &computed);
    assert!(ledger.facts.lock_present);
    assert_eq!(ledger.facts.seals.len(), computed.len());
    assert!(ledger.facts.seals.values().all(|f| f.recorded.is_some()));
}

// -------------------------------------------------------------------------------------
// Diagnostics
// -------------------------------------------------------------------------------------

#[test]
fn a_clean_ledger_produces_no_diagnostics_and_no_errors() {
    let ledger = planning_ledger();
    let model = ResolvedModel::build(&ledger, &inputs());
    assert!(
        model.diagnostics.is_empty(),
        "unexpected: {:?}",
        model
            .diagnostics
            .iter()
            .map(|d| (d.code.as_str(), d.message.clone()))
            .collect::<Vec<_>>()
    );
    assert!(!model.has_errors());
}

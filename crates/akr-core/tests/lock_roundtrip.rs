//! `akr.lock`: rendering, reading, ordering, verification, and the V-024 integration.
//!
//! Exit criterion 4 of `docs/13-implementation-roadmap.md` P3 lives here: editing a
//! sealed revision's text produces `AKR-R051` naming the key, the revision, and the
//! expected hash.

use akr_core::diagnostics::{Severity, Subject, codes};
use akr_core::hash::content_hash;
use akr_core::lock::{
    Build, LOCK_HEADER, Lock, Mismatch, ResolutionEntry, SealEntry, SourceEntry,
    currency_diagnostics,
};
use akr_core::model::{ContentHash, Kind, Ledger, Project, RecordBuilder, RevisionId, State, key};
use akr_core::validate::v024_seals_match;
use std::collections::BTreeMap;

fn id(text: &str, revision: u32) -> RevisionId {
    RevisionId::new(key(text), revision)
}

fn hash(text: &str) -> ContentHash {
    ContentHash(format!("sha256:{}", text.repeat(64 / text.len())))
}

fn sample() -> Lock {
    Lock {
        project: "save-your-skin".to_owned(),
        build: Build {
            tool: "akr 0.1.0".to_owned(),
            grammar: "0.1".to_owned(),
            vocabulary: "0.1".to_owned(),
            commit: Some(
                akr_core::model::Commit::new("e806b3f54a2d7091c5e13b8a26f490dc7b135e64")
                    .expect("valid commit"),
            ),
            source_graph: hash("a"),
            built_at: "2026-08-03T09:14:00Z".to_owned(),
        },
        sources: vec![
            SourceEntry {
                path: ".akr/records/sys/policies.akr".to_owned(),
                hash: hash("c"),
                records: 2,
            },
            SourceEntry {
                path: ".akr/project.akr".to_owned(),
                hash: hash("b"),
                records: 0,
            },
        ],
        resolutions: vec![
            ResolutionEntry {
                from: id("sys.work.m3-plan", 2),
                slot: "implements".to_owned(),
                to: id("sys.policy.tandem-work", 1),
                hash: hash("d"),
            },
            ResolutionEntry {
                from: id("sys.assessment.projection-gaps", 1),
                slot: "supported_by".to_owned(),
                to: id("sim.obs.projection-gaps", 1),
                hash: hash("e"),
            },
        ],
        seals: vec![
            SealEntry {
                id: id("sys.policy.tandem-work", 1),
                state: State::Active,
                hash: hash("d"),
            },
            SealEntry {
                id: id("lege.decision.renderer-boundary", 1),
                state: State::Superseded,
                hash: hash("f"),
            },
        ],
    }
}

// -------------------------------------------------------------------------------------
// Rendering and reading
// -------------------------------------------------------------------------------------

#[test]
fn render_then_parse_is_the_identity() {
    let lock = sample();
    let parsed = Lock::parse(&lock.render()).expect("parses");
    // Vectors come back sorted, so compare the rendering rather than the field order.
    assert_eq!(parsed.render(), lock.render());
    assert_eq!(parsed.build, lock.build);
}

#[test]
fn render_is_a_fixed_point() {
    // Two builds of the same sources produce byte-identical files (§4).
    let once = sample().render();
    let twice = Lock::parse(&once).expect("parses").render();
    assert_eq!(once, twice);
}

#[test]
fn render_starts_with_the_lock_header_and_ends_with_one_newline() {
    let rendered = sample().render();
    assert!(rendered.starts_with(&format!("{LOCK_HEADER}\nproject save-your-skin\n\n")));
    assert!(rendered.ends_with("}\n"));
    assert!(!rendered.ends_with("\n\n"));
    assert!(!rendered.contains('\r'), "LF only");
}

#[test]
fn render_orders_every_section() {
    // §4: build first; source by path; resolution by referring key, revision, slot, target;
    // seal by key then revision.
    let rendered = sample().render();
    let at = |needle: &str| rendered.find(needle).unwrap_or_else(|| panic!("{needle}"));
    assert!(at("build {") < at("source "));
    assert!(at("\nsource \".akr/project.akr\"") < at("\nsource \".akr/records/sys/policies.akr\""));
    assert!(
        at("\nresolution @sys.assessment.projection-gaps/1")
            < at("\nresolution @sys.work.m3-plan/2")
    );
    assert!(
        at("\nseal @lege.decision.renderer-boundary/1") < at("\nseal @sys.policy.tandem-work/1")
    );
    assert!(at("resolution ") < at("\nseal "));
}

#[test]
fn ordering_does_not_depend_on_input_order() {
    let mut reversed = sample();
    reversed.sources.reverse();
    reversed.resolutions.reverse();
    reversed.seals.reverse();
    assert_eq!(reversed.render(), sample().render());
}

#[test]
fn parse_rejects_a_bad_header() {
    let error = Lock::parse("akr 0.1\nproject p\n").expect_err("wrong header");
    assert!(error.message.contains("akr-lock 0.1"), "{}", error.message);
}

#[test]
fn parse_rejects_a_floating_reference() {
    // Every reference in a lock is pinned: the lock exists to record which revision.
    let text = format!(
        "{LOCK_HEADER}\nproject p\n\nseal @fx.policy.a {{\n    state active\n    hash \"sha256:aa\"\n}}\n"
    );
    let error = Lock::parse(&text).expect_err("floating reference");
    assert!(
        error.message.contains("must be pinned"),
        "{}",
        error.message
    );
}

#[test]
fn parse_reports_a_missing_slot_with_a_line_number() {
    let text = format!("{LOCK_HEADER}\nproject p\n\nsource \"a.akr\" {{\n    records 1\n}}\n");
    let error = Lock::parse(&text).expect_err("missing hash");
    assert!(error.message.contains("hash"), "{}", error.message);
    assert!(error.line > 0);
}

#[test]
fn parse_skips_comments_and_blank_lines() {
    let text = format!(
        "# generated; do not edit\n{LOCK_HEADER}\nproject p\n\n\nbuild {{\n    tool \"akr 0.1.0\"\n    grammar \"0.1\"\n    vocabulary \"0.1\"\n    source_graph \"sha256:aa\"\n}}\n"
    );
    let lock = Lock::parse(&text).expect("parses");
    assert_eq!(lock.project, "p");
    assert_eq!(lock.build.tool, "akr 0.1.0");
}

// -------------------------------------------------------------------------------------
// Verification
// -------------------------------------------------------------------------------------

#[test]
fn an_unchanged_lock_verifies_clean() {
    assert!(sample().verify(&sample()).is_empty());
}

#[test]
fn committing_identical_dirty_sources_does_not_invalidate_lock_currency() {
    let computed = sample();
    let mut recorded = computed.clone();
    recorded.build.commit = None;
    assert!(
        !recorded.verify(&computed).is_empty(),
        "exact provenance still differs"
    );
    assert!(recorded.currency_mismatches(&computed).is_empty());
    assert!(currency_diagnostics(&recorded, &computed, ".akr/akr.lock").is_empty());
    let mut changed = computed.clone();
    changed.sources[0].hash = hash("9");
    assert!(!currency_diagnostics(&recorded, &changed, ".akr/akr.lock").is_empty());
    assert!(
        !computed.currency_mismatches(&recorded).is_empty(),
        "losing known provenance remains a mismatch"
    );
}

#[test]
fn built_at_is_excluded_from_verification() {
    // §6: "Verification compares everything except build.built_at". It changes on every
    // build regardless of content; comparing it would make every lock permanently stale.
    let mut later = sample();
    later.build.built_at = "2027-01-01T00:00:00Z".to_owned();
    assert!(
        sample().verify(&later).is_empty(),
        "built_at must never cause a mismatch"
    );
}

#[test]
fn everything_else_in_build_is_compared() {
    for mutate in [
        (|l: &mut Lock| l.build.tool = "akr 0.2.0".to_owned()) as fn(&mut Lock),
        |l: &mut Lock| l.build.grammar = "0.2".to_owned(),
        |l: &mut Lock| l.build.vocabulary = "0.2".to_owned(),
        |l: &mut Lock| l.build.source_graph = hash("9"),
        |l: &mut Lock| l.build.commit = None,
    ] {
        let mut changed = sample();
        mutate(&mut changed);
        let mismatches = sample().verify(&changed);
        assert!(
            mismatches
                .iter()
                .any(|m| matches!(m, Mismatch::Build { .. })),
            "expected a build mismatch, got {mismatches:?}"
        );
    }
}

#[test]
fn a_changed_source_is_reported() {
    let mut changed = sample();
    changed.sources[0].hash = hash("9");
    let mismatches = sample().verify(&changed);
    assert_eq!(mismatches.len(), 1);
    assert!(matches!(&mismatches[0], Mismatch::Source { path, .. }
        if path == ".akr/records/sys/policies.akr"));
}

#[test]
fn an_added_and_a_removed_source_are_both_reported() {
    let mut fewer = sample();
    fewer.sources.pop();
    assert_eq!(sample().verify(&fewer).len(), 1, "removal");
    assert_eq!(fewer.verify(&sample()).len(), 1, "addition");
}

#[test]
fn a_repointed_resolution_is_reported() {
    let mut changed = sample();
    changed.resolutions[0].to = id("sys.policy.tandem-work", 2);
    let mismatches = sample().verify(&changed);
    assert!(matches!(&mismatches[0], Mismatch::Resolution { .. }));
}

#[test]
fn a_repointed_resolution_is_reported_even_at_the_same_revision() {
    // A `proposed` head edited in place keeps its revision number. The hash is what makes
    // that visible (§2.3).
    let mut changed = sample();
    changed.resolutions[0].hash = hash("9");
    assert_eq!(sample().verify(&changed).len(), 1);
}

#[test]
fn verification_is_deterministic() {
    let mut changed = sample();
    changed.sources[0].hash = hash("9");
    changed.seals[0].hash = hash("9");
    changed.build.tool = "akr 0.2.0".to_owned();
    let once = sample().verify(&changed);
    let twice = sample().verify(&changed);
    assert_eq!(once, twice);
}

// -------------------------------------------------------------------------------------
// Currency diagnostics: the other half of AKR-R052
// -------------------------------------------------------------------------------------

#[test]
fn stale_sources_raise_r052_against_the_lock_file() {
    let mut changed = sample();
    changed.sources[0].hash = hash("9");
    let diagnostics = currency_diagnostics(&sample(), &changed, ".akr/akr.lock");
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, codes::R052);
    assert_eq!(diagnostics[0].severity, Severity::Error);
    assert_eq!(
        diagnostics[0].primary.subject,
        Subject::File(".akr/akr.lock".to_owned())
    );
    assert!(
        diagnostics[0]
            .message
            .contains("does not match the sources")
    );
}

#[test]
fn seal_and_resolution_drift_do_not_duplicate_r052() {
    // AKR-R051 owns seal drift and a lock diff owns repointing. Reporting them here too
    // would double every diagnostic about a modified record.
    let mut changed = sample();
    changed.seals[0].hash = hash("9");
    changed.resolutions[0].to = id("sys.policy.tandem-work", 2);
    assert!(currency_diagnostics(&sample(), &changed, ".akr/akr.lock").is_empty());
}

// -------------------------------------------------------------------------------------
// V-024 integration — exit criterion 4
// -------------------------------------------------------------------------------------

const SEALED_TEXT: &str = "\
record fx.policy.sealed/1 : policy {
    title \"A sealed record\"
    state active
    scope [ all ]
    rule \"\"\"
        The original text.
        \"\"\"
}
";

fn sealed_ledger() -> Ledger {
    sealed_ledger_in(State::Active)
}

fn sealed_ledger_in(state: State) -> Ledger {
    let mut ledger = Ledger::new(Project::new("fixtures", &["fx"]));
    ledger.insert(
        RecordBuilder::new("fx.policy.sealed", 1, Kind::Policy)
            .filled()
            .state(state)
            .build(),
    );
    ledger
}

fn lock_for(text: &str) -> Lock {
    Lock {
        project: "fixtures".to_owned(),
        build: Build::default(),
        sources: Vec::new(),
        resolutions: Vec::new(),
        seals: vec![SealEntry {
            id: id("fx.policy.sealed", 1),
            state: State::Active,
            hash: content_hash(text),
        }],
    }
}

#[test]
fn an_unedited_sealed_revision_produces_nothing() {
    let mut ledger = sealed_ledger();
    let computed: BTreeMap<_, _> = [(id("fx.policy.sealed", 1), content_hash(SEALED_TEXT))].into();
    lock_for(SEALED_TEXT).apply_facts(&mut ledger, &computed);
    assert!(v024_seals_match(&ledger).is_empty());
}

#[test]
fn editing_a_sealed_revision_produces_r051_naming_key_revision_and_hashes() {
    let edited = SEALED_TEXT.replace("The original text.", "Text changed after sealing.");
    let mut ledger = sealed_ledger();
    let computed: BTreeMap<_, _> = [(id("fx.policy.sealed", 1), content_hash(&edited))].into();
    // The lock still records the hash of the text as it was sealed.
    lock_for(SEALED_TEXT).apply_facts(&mut ledger, &computed);

    let diagnostics = v024_seals_match(&ledger);
    assert_eq!(diagnostics.len(), 1);
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic.code, codes::R051);
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(
        diagnostic.primary.subject,
        Subject::Revision(id("fx.policy.sealed", 1))
    );
    assert!(
        diagnostic.message.contains("fx.policy.sealed/1"),
        "names the key and revision: {}",
        diagnostic.message
    );
    assert!(
        diagnostic.message.contains(&content_hash(SEALED_TEXT).0),
        "names the expected hash: {}",
        diagnostic.message
    );
    assert!(
        diagnostic.message.contains(&content_hash(&edited).0),
        "names the computed hash: {}",
        diagnostic.message
    );
    assert!(
        diagnostic
            .help
            .as_deref()
            .is_some_and(|h| h.contains("akr revise")),
        "points at the fix"
    );
}

#[test]
fn a_supported_state_transition_is_stale_lock_not_seal_tampering() {
    let active = sealed_ledger();
    let active_text = akr_core::syntax::record_text(
        active
            .get(&id("fx.policy.sealed", 1))
            .expect("sealed record"),
        &active.project.name,
    );
    let retired = active_text.replace("state active", "state superseded");
    let mut ledger = sealed_ledger_in(State::Superseded);
    let computed: BTreeMap<_, _> = [(id("fx.policy.sealed", 1), content_hash(&retired))].into();
    lock_for(&active_text).apply_facts(&mut ledger, &computed);

    let diagnostics = v024_seals_match(&ledger);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, codes::R052);
    assert!(diagnostics[0].message.contains("active to superseded"));
}

#[test]
fn a_state_transition_does_not_hide_a_simultaneous_body_edit() {
    let edited = SEALED_TEXT
        .replace("state active", "state superseded")
        .replace("The original text.", "Text changed while retiring.");
    let mut ledger = sealed_ledger_in(State::Superseded);
    let computed: BTreeMap<_, _> = [(id("fx.policy.sealed", 1), content_hash(&edited))].into();
    lock_for(SEALED_TEXT).apply_facts(&mut ledger, &computed);

    let diagnostics = v024_seals_match(&ledger);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, codes::R051);
}

#[test]
fn adding_a_comment_to_a_sealed_revision_produces_nothing() {
    // The exclusion of §3.3, end to end: a clarifying comment is not a modification.
    let commented = SEALED_TEXT.replace(
        "    state active\n",
        "    # still in force after the M3 replan\n    state active\n",
    );
    let mut ledger = sealed_ledger();
    let computed: BTreeMap<_, _> = [(id("fx.policy.sealed", 1), content_hash(&commented))].into();
    lock_for(SEALED_TEXT).apply_facts(&mut ledger, &computed);
    assert!(v024_seals_match(&ledger).is_empty());
}

#[test]
fn a_sealed_revision_missing_from_the_lock_produces_r052() {
    let mut ledger = sealed_ledger();
    let empty = Lock {
        project: "fixtures".to_owned(),
        ..Lock::default()
    };
    let computed: BTreeMap<_, _> = [(id("fx.policy.sealed", 1), content_hash(SEALED_TEXT))].into();
    empty.apply_facts(&mut ledger, &computed);
    let diagnostics = v024_seals_match(&ledger);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, codes::R052);
}

#[test]
fn without_a_lock_v024_says_nothing() {
    // A project that has never been built has nothing to compare against, and inventing a
    // mismatch would be worse than silence.
    let ledger = sealed_ledger();
    assert!(!ledger.facts.lock_present);
    assert!(v024_seals_match(&ledger).is_empty());
}

#[test]
fn a_revision_with_no_computed_hash_is_not_accused() {
    // A build that could not produce canonical text must not accuse anyone of editing a
    // sealed record.
    let mut ledger = sealed_ledger();
    lock_for(SEALED_TEXT).apply_facts(&mut ledger, &BTreeMap::new());
    assert!(v024_seals_match(&ledger).is_empty());
}

#[test]
fn proposed_revisions_are_not_sealed() {
    // `proposed` revisions are unsealed and freely editable; that is what makes `proposed`
    // worth having (D-015).
    let mut ledger = Ledger::new(Project::new("fixtures", &["fx"]));
    ledger.insert(
        RecordBuilder::new("fx.policy.draft", 1, Kind::Policy)
            .filled()
            .state(State::Proposed)
            .build(),
    );
    Lock {
        project: "fixtures".to_owned(),
        ..Lock::default()
    }
    .apply_facts(&mut ledger, &BTreeMap::new());
    assert!(v024_seals_match(&ledger).is_empty());
}

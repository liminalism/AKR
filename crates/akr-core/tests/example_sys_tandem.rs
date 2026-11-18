//! `examples/sys-tandem` end to end: the second worked example, on a real document.
//!
//! Where `worked_example.rs` proves the machinery on a corpus built to exercise it, this
//! one proves it on the SYS engine-and-simulator tandem roadmap of 2026-08-03, encoded
//! record for record. Two things it demonstrates that the synthetic example cannot:
//!
//! - **The acceptance gap.** The document's banner says "implementation landed; manual
//!   sign-off pending". The ledger computes exactly that: four milestones complete, one
//!   active with a single unsatisfied check. The test below proves the state is derived
//!   rather than asserted, by showing that completing M5 fails.
//! - **Freshness at scale.** One observation goes stale against the synthetic history and
//!   the doubt reaches four records along three relations, including a live policy.

use crate::support;

use akr_core::diagnostics::FileId;
use akr_core::freshness::derive;
use akr_core::graph::propagate_staleness;
use akr_core::model::{Commit, Date, Kind, Ledger, RevisionId, State, key};
use akr_core::resolve::{BuildInputs, ResolvedModel, Workspace, load_workspace};
use akr_core::syntax::{format, parse};
use akr_core::validate;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use support::{Step, SyntheticHistory};

fn example_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/sys-tandem")
}

fn workspace() -> &'static Workspace {
    static WORKSPACE: OnceLock<Workspace> = OnceLock::new();
    WORKSPACE.get_or_init(|| {
        let root = example_root();
        load_workspace(&root, &root.join(".akr")).expect("the tandem example loads")
    })
}

/// C5 and the frozen "today" of `MANIFEST.md` §3. Inputs, not clock readings.
fn inputs(base: &BuildInputs) -> BuildInputs {
    BuildInputs {
        tool: "akr 0.1.0".to_owned(),
        grammar: "0.1".to_owned(),
        vocabulary: "0.1".to_owned(),
        commit: Some(
            Commit::new("b7e3092d6a1f48c5039be2714da86f05c93e1b6d").expect("a frozen commit"),
        ),
        built_at: "2026-08-04T10:00:00Z".to_owned(),
        ..base.clone()
    }
}

fn id(key_text: &str, revision: u32) -> RevisionId {
    RevisionId::new(key(key_text), revision)
}

// -------------------------------------------------------------------------------------
// The ledger itself
// -------------------------------------------------------------------------------------

#[test]
fn the_example_resolves_with_no_diagnostics() {
    let workspace = workspace();
    assert!(
        workspace.diagnostics.is_empty(),
        "parse and lower: {:?}",
        workspace
            .diagnostics
            .iter()
            .map(|d| (d.code.as_str(), d.message.as_str()))
            .collect::<Vec<_>>()
    );
    let model = ResolvedModel::build(&workspace.ledger, &inputs(&workspace.inputs));
    assert!(
        model.diagnostics.is_empty(),
        "resolve: {:?}",
        model
            .diagnostics
            .iter()
            .map(|d| (d.code.as_str(), d.message.as_str()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn the_inventory_matches_the_manifest() {
    let workspace = workspace();
    let ledger = &workspace.ledger;
    assert_eq!(ledger.records().len(), 66, "revisions");
    assert_eq!(ledger.keys().len(), 63, "keys");

    // Three keys carry a second revision, and each is a distinct reason to revise.
    for key_text in [
        "tandem.milestone.m1-court-speaks",
        "tandem.assessment.central-fact",
        "tandem.work.defect-retirement-plan",
    ] {
        assert_eq!(ledger.revisions_of(&key(key_text)).len(), 2, "{key_text}");
    }

    // Every kind is exercised, which is what makes this a second *worked* example
    // rather than a second fixture.
    for kind in Kind::ALL {
        assert!(
            ledger.records().iter().any(|r| r.kind == *kind),
            "no {kind} record in the tandem example"
        );
    }
}

#[test]
fn every_source_file_is_canonically_formatted() {
    let workspace = workspace();
    for source in &workspace.inputs.sources {
        let path = example_root().join(&source.path);
        let text = std::fs::read_to_string(&path).expect("readable");
        let parsed = parse(&text, FileId(0));
        assert!(
            parsed.diagnostics.is_empty(),
            "{}: does not parse",
            source.path
        );
        assert_eq!(
            format(parsed.file.as_ref().expect("parses")),
            text,
            "{} is not canonical; run the formatter",
            source.path
        );
    }
}

// -------------------------------------------------------------------------------------
// The acceptance gap — what the document's banner says, computed
// -------------------------------------------------------------------------------------

#[test]
fn four_milestones_completed_and_one_is_not() {
    let workspace = workspace();
    let ledger = &workspace.ledger;
    let state_of = |key_text: &str| ledger.head(&key(key_text)).expect("a head").state;

    for key_text in [
        "tandem.milestone.m1-court-speaks",
        "tandem.milestone.m2-castle-works",
        "tandem.milestone.m3-audible-day",
        "tandem.milestone.m4-day-means-something",
    ] {
        assert_eq!(state_of(key_text), State::Completed, "{key_text}");
    }
    assert_eq!(
        state_of("tandem.milestone.m5-one-playable-day"),
        State::Active,
        "M5 cannot be complete while a designer has not signed off"
    );
}

/// The headline: the ledger *derives* the acceptance gap rather than asserting it.
///
/// The roadmap's banner claims "implementation landed; manual sign-off pending" in prose
/// nothing can check. Mark M5 completed and V-020 names the exact check that is missing.
#[test]
fn completing_m5_fails_on_the_one_unverified_check() {
    let workspace = workspace();
    let mut ledger = workspace.ledger.clone();
    let target = id("tandem.milestone.m5-one-playable-day", 1);

    let mut records: Vec<_> = ledger.records().to_vec();
    for record in &mut records {
        if record.id == target {
            record.state = State::Completed;
        }
    }
    let project = ledger.project.clone();
    ledger = akr_core::model::Ledger::new(project);
    ledger.extend(records);

    let found = validate::v020_acceptance_satisfied(&ledger);
    assert_eq!(
        found.len(),
        1,
        "exactly one check is unsatisfied, got {found:?}"
    );
    let diagnostic = &found[0];
    assert_eq!(diagnostic.code.as_str(), "AKR-R022");
    assert!(
        diagnostic.message.contains("three-seed-designer-signoff"),
        "the diagnostic must name the designer sign-off: {}",
        diagnostic.message
    );
}

/// M1's acceptance was revised, not merely met: revision 1 named two ignored assertions
/// and revision 2 names one, under a designer ruling. The roadmap records the ruling and
/// not the revision, which is why the milestone reads there as simply passed.
#[test]
fn m1_acceptance_was_narrowed_by_a_recorded_ruling() {
    let workspace = workspace();
    let ledger = &workspace.ledger;

    let first = ledger
        .get(&id("tandem.milestone.m1-court-speaks", 1))
        .expect("revision 1");
    let second = ledger
        .get(&id("tandem.milestone.m1-court-speaks", 2))
        .expect("revision 2");
    assert_eq!(first.state, State::Superseded);
    assert_eq!(second.state, State::Completed);

    let checks = |r: &akr_core::model::Record| -> Vec<String> {
        r.acceptance
            .as_ref()
            .expect("acceptance")
            .checks
            .iter()
            .map(|c| c.id.to_string())
            .collect()
    };
    assert!(checks(first).contains(&"squelch-audit-both-assertions".to_owned()));
    assert!(checks(second).contains(&"squelch-audit-calm-assertion".to_owned()));

    // The revision cites the ruling that narrowed it, and the ruling resolves the
    // question it was raised against.
    assert!(
        second
            .targets(akr_core::model::Relation::DerivedFrom)
            .iter()
            .any(|t| t.key == key("simulator.decision.wild-threshold-ignored")),
        "revision 2 must cite the ruling that narrowed its acceptance"
    );
    let ruling = ledger
        .head(&key("simulator.decision.wild-threshold-ignored"))
        .expect("the ruling");
    assert!(
        ruling
            .targets(akr_core::model::Relation::Resolves)
            .iter()
            .any(|t| t.key == key("simulator.question.wild-threshold"))
    );
}

// -------------------------------------------------------------------------------------
// Freshness — the P5 scenario, encoded as data
// -------------------------------------------------------------------------------------

/// The five commits of `MANIFEST.md` §2, materialised so the freshness computation has a
/// real repository to ask.
///
/// C3 is when the observations were made and C4 is when everything landed, which is the
/// whole point: the document asserts a state of the world in §1 and then reports, in §7 of
/// the same document, the work that changed it.
const STEPS: &[(&str, &[(&str, &str)])] = &[
    (
        "C1 skeleton",
        &[
            ("src/lib.rs", "// the simulator\n"),
            ("src/engine/mod.rs", "// simulator-side engine modules\n"),
            (
                "SYSEngine/crates/sys_game_bridge/src/lib.rs",
                "// the seam\n",
            ),
            (
                "SYSEngine/crates/sys_present/src/lib.rs",
                "// presentation\n",
            ),
            ("SYSEngine/crates/sys_sound/src/lib.rs", "// sound\n"),
            ("SYSEngine/docs/BUILD-STATUS.md", "# verified tree state\n"),
            ("AGENTS.md", "# rulings\n"),
        ],
    ),
    (
        "C2 engine docs",
        &[("SYSEngine/docs/RENDERING-NEXT.md", "# renderer boundary\n")],
    ),
    (
        "C3 the assessment and the rulings",
        &[
            (
                "SYSEngine/docs/2026-08-02-mid-engine-assessment.md",
                "# the gap register\n",
            ),
            ("AGENTS.md", "# rulings, revised\n"),
        ],
    ),
    (
        "C4 M1-M5 land",
        &[
            ("src/lib.rs", "// the simulator, with the day close\n"),
            (
                "SYSEngine/crates/sys_present/src/lib.rs",
                "// presentation, four more record families\n",
            ),
        ],
    ),
    (
        "C5 a bridge follow-up",
        &[(
            "SYSEngine/crates/sys_game_bridge/src/lib.rs",
            "// the seam, extended\n",
        )],
    ),
];

/// The manifest's invented hashes, paired with their position in §2's table.
const MAPPING: &[(&str, usize)] = &[
    ("a3f19d0c7b5e284610f8c2a94db3e0761fc58a29", 1),
    ("4e82b6d1a09f37c5e28d4bf160a739c8b2e05d17", 2),
    ("9d40e1b7c85a326f0b1de94738ca5602f81b7d4a", 3),
    ("5cb1a4f0d2e79836ba4c17e05d3f96428a7bc10e", 4),
    ("b7e3092d6a1f48c5039be2714da86f05c93e1b6d", 5),
];

fn history() -> SyntheticHistory {
    let steps: Vec<Step<'_>> = STEPS
        .iter()
        .map(|(message, writes)| Step::new(message, writes))
        .collect();
    SyntheticHistory::build("sys-tandem-history", &steps)
}

/// "Today" — 2026-08-04, the day after the roadmap (`MANIFEST.md` §2). An input.
fn today() -> Date {
    Date::new(2026, 8, 4).expect("a valid date")
}

/// The stale set, **derived** from the materialised history rather than declared.
///
/// C5 touched `SYSEngine/crates/sys_game_bridge/**` after the channel-coverage observation
/// was made at C3, and C4 touched `src/**` after the determinism observation. Both are
/// `watches` matches, so both observations are stale (`MANIFEST.md` §3).
fn stale_set(ledger: &Ledger, history: &SyntheticHistory) -> BTreeSet<RevisionId> {
    let queue = derive(ledger, &history.git(), history.head(), today()).expect("derives");
    assert!(
        queue.diagnostics.is_empty(),
        "the freshness inputs are sound: {:?}",
        queue
            .diagnostics
            .iter()
            .map(|d| (d.code.as_str(), d.message.clone()))
            .collect::<Vec<_>>()
    );
    let stale: BTreeSet<String> = queue.stale.iter().map(|s| s.id.to_string()).collect();
    assert_eq!(
        stale,
        [
            "engine.obs.channel-coverage/1",
            "simulator.obs.day-runs-deterministically/1",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect::<BTreeSet<_>>(),
        "MANIFEST §3: two stale observations"
    );
    queue.stale_set()
}

#[test]
fn staleness_reaches_the_policy_along_carrying_relations_only() {
    use akr_core::model::Relation;
    let workspace = workspace();
    let history = history();
    let ledger = history.remap(&workspace.ledger, MAPPING);
    let at_risk = propagate_staleness(&ledger, &stale_set(&ledger, &history));

    // `docs/02` §6 defines at_risk over **live** records, and the walk applies that: the
    // superseded `tandem.assessment.central-fact/1` rests on whatever it rested on, and
    // nobody is relying on it.
    let flagged: BTreeSet<String> = at_risk.iter().map(|r| r.id.to_string()).collect();
    let expected: BTreeSet<String> = [
        "engine.assessment.castle-not-court/1",
        "tandem.assessment.central-fact/2",
        "tandem.policy.tandem-work/1",
        "tandem.work.m5-plan/1",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    assert_eq!(flagged, expected, "the at-risk set");

    // The chain that matters: an operating rule two hops from a stale measurement.
    let policy = at_risk
        .iter()
        .find(|r| r.id == id("tandem.policy.tandem-work", 1))
        .expect("the tandem policy is at risk");
    assert_eq!(policy.depth, 2, "policy -> assessment -> observation");
    assert_eq!(policy.via, Relation::SupportedBy);
    assert_eq!(
        policy.path.last(),
        Some(&id("engine.obs.channel-coverage", 1)),
        "the path must end at the stale observation"
    );

    // D-024 propagates along `supported_by`, `depends_on` and `derived_from` and no others.
    assert!(!at_risk.is_empty());
    for flagged in &at_risk {
        assert!(
            matches!(
                flagged.via,
                Relation::SupportedBy | Relation::DependsOn | Relation::DerivedFrom
            ),
            "{} was flagged via {}, which does not carry staleness",
            flagged.id,
            flagged.via
        );
    }
    assert!(
        !at_risk
            .iter()
            .any(|r| r.id.key.to_string().starts_with("tandem.milestone.")),
        "milestones must not be flagged through ordering or containment"
    );
}

// -------------------------------------------------------------------------------------
// The lock
// -------------------------------------------------------------------------------------

#[test]
fn the_committed_lock_matches_what_the_build_produces() {
    let workspace = workspace();
    let model = ResolvedModel::build(&workspace.ledger, &inputs(&workspace.inputs));
    let committed = workspace
        .lock_text
        .as_ref()
        .expect("the example ships a lock");
    assert_eq!(
        model.to_lock().render(),
        *committed,
        "examples/sys-tandem/.akr/akr.lock is stale; regenerate it"
    );
}

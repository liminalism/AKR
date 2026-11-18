//! P6 exit criterion 3: a failure at any point leaves every source file byte-identical.
//!
//! `docs/07` §4 promises it in one sentence — "failure writes nothing... no partial write
//! and no `.bak` file" — and the only way to hold a promise like that is to check it on
//! every refusing path rather than on a representative one. Each case below hashes every
//! `.akr` file before and after, and compares.

// `ops::Refused` is a large `Err` variant by design — it carries the structured refusal
// data the CLI renders — and every closure here returns one. See the rationale on the
// same allow in `src/ops/mod.rs`.
#![allow(clippy::result_large_err)]

use crate::ops_support;

use akr_core::model::{
    ContentSlot, ContentValue, Kind, Outcome as DispositionOutcome, RecordBuilder, Reference, key,
};
use akr_core::ops::{self, DispositionRequest, Edits, ReviseMode, WriteContext};
use ops_support::Sandbox;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// A cheap content digest per file. Any change to any byte changes the map.
fn digests(sandbox: &Sandbox) -> BTreeMap<PathBuf, u64> {
    sandbox
        .snapshot()
        .into_iter()
        .map(|(path, text)| {
            let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
            for byte in text.as_bytes() {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(0x0100_0000_01b3);
            }
            (path, hash)
        })
        .collect()
}

/// Runs an operation expected to refuse, and asserts nothing moved.
#[track_caller]
fn refuses_without_writing(
    sandbox: &Sandbox,
    what: &str,
    operation: impl FnOnce(&WriteContext) -> ops::WriteResult,
) -> ops::Refused {
    let before = digests(sandbox);
    let files_before = sandbox.snapshot();
    let context = WriteContext::new(sandbox.akr_dir());

    let refused = match operation(&context) {
        Ok(applied) => panic!("{what}: expected a refusal, got {applied:?}"),
        Err(refused) => refused,
    };

    assert_eq!(before, digests(sandbox), "{what}: a source file changed");
    assert_eq!(files_before, sandbox.snapshot(), "{what}: contents changed");
    assert!(
        !leftover_temporaries(sandbox),
        "{what}: a temporary file was left behind"
    );
    refused
}

fn leftover_temporaries(sandbox: &Sandbox) -> bool {
    fn walk(dir: &std::path::Path) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                if walk(&path) {
                    return true;
                }
            } else if path.to_string_lossy().contains(".akr.tmp") {
                return true;
            }
        }
        false
    }
    walk(&sandbox.akr_dir())
}

// -------------------------------------------------------------------------------------

#[test]
fn propose_revise_and_validation_refusals_write_nothing() {
    let sandbox = Sandbox::save_your_skin();

    // propose refusals (Policy, not retired Term, so validation refusals actually run)
    let refused = refuses_without_writing(&sandbox, "existing key", |context| {
        ops::propose(
            context,
            &key("sys.policy.tandem-work"),
            Kind::Policy,
            "again",
            None,
        )
    });
    assert_eq!(refused.code.as_str(), "AKR-L041");

    let refused = refuses_without_writing(&sandbox, "missing required slot", |context| {
        ops::propose(context, &key("sys.policy.bare"), Kind::Policy, "Bare", None)
    });
    assert_eq!(refused.code.as_str(), "AKR-C031");
    assert!(
        refused
            .diagnostics
            .iter()
            .any(|d| d.code.as_str() == "AKR-T001")
    );

    let mut stranger = RecordBuilder::new("nope.policy.x", 1, Kind::Policy)
        .title("Stranger")
        .all_scope()
        .build();
    stranger.content.insert(
        ContentSlot::Rule,
        ContentValue::prose("Undeclared namespace."),
    );
    let refused = refuses_without_writing(&sandbox, "undeclared namespace", |context| {
        ops::propose(
            context,
            &key("nope.policy.x"),
            Kind::Policy,
            "Stranger",
            Some(stranger),
        )
    });
    assert!(
        refused
            .diagnostics
            .iter()
            .any(|d| d.code.as_str() == "AKR-L004")
    );

    // revise refusals
    refuses_without_writing(&sandbox, "unknown key", |context| {
        ops::revise(
            context,
            &key("sys.term.absent"),
            ReviseMode::Auto,
            &Edits::default(),
        )
    });
    refuses_without_writing(&sandbox, "sealed head, in place", |context| {
        ops::revise(
            context,
            &key("sys.term.playable-day"),
            ReviseMode::InPlace,
            &Edits {
                title: Some("no".to_owned()),
                ..Edits::default()
            },
        )
    });
    refuses_without_writing(&sandbox, "illegal state transition", |context| {
        ops::revise(
            context,
            &key("sim.decision.timestep-4ms"),
            ReviseMode::InPlace,
            &Edits {
                state: Some(akr_core::model::State::Completed),
                ..Edits::default()
            },
        )
    });

    // validation-step refusal after splice and canonical render
    let refused = refuses_without_writing(&sandbox, "result does not validate", |context| {
        ops::complete(
            context,
            &key("sys.milestone.m3-playable-day"),
            &[(
                "no-placeholder-assets".to_owned(),
                Reference::head(key("sys.evidence.playable-day-demo")),
            )],
        )
    });
    assert_eq!(refused.code.as_str(), "AKR-C031");
    assert!(
        !refused.diagnostics.is_empty(),
        "the refusal must say what was wrong"
    );
}

#[test]
fn every_supersede_refusal_writes_nothing() {
    let sandbox = Sandbox::save_your_skin();
    let context = WriteContext::new(sandbox.akr_dir());

    // A plan with a child pinned to its head, so the disposition demand has a target.
    let mut plan = akr_core::model::RecordBuilder::new("sys.work.atomic-plan", 1, Kind::Work)
        .title("Atomic plan")
        .build();
    plan.content.insert(
        akr_core::model::ContentSlot::Intent,
        akr_core::model::ContentValue::prose("A plan with one child."),
    );
    ops::propose(
        &context,
        &key("sys.work.atomic-plan"),
        Kind::Work,
        "Atomic plan",
        Some(plan),
    )
    .expect("the plan proposes");
    let mut child = akr_core::model::RecordBuilder::new("sys.work.atomic-child", 1, Kind::Work)
        .title("Atomic child")
        .state(akr_core::model::State::Ready)
        .rel(akr_core::model::Relation::PartOf, "@sys.work.atomic-plan/1")
        .build();
    child.content.insert(
        akr_core::model::ContentSlot::Intent,
        akr_core::model::ContentValue::prose("A child pinned to revision 1."),
    );
    ops::propose(
        &context,
        &key("sys.work.atomic-child"),
        Kind::Work,
        "Atomic child",
        Some(child),
    )
    .expect("the child proposes");

    let refused = refuses_without_writing(&sandbox, "missing disposition", |context| {
        ops::supersede(context, &key("sys.work.atomic-plan"), &[])
    });
    assert_eq!(refused.code.as_str(), "AKR-R014");

    refuses_without_writing(&sandbox, "unknown key", |context| {
        ops::supersede(context, &key("sys.work.absent"), &[])
    });

    // A disposition naming something that is not a child is refused by V-017 at
    // validation, after the in-memory splice — the deepest failure point there is.
    refuses_without_writing(&sandbox, "disposition of a non-child", |context| {
        ops::supersede(
            context,
            &key("sys.work.atomic-plan"),
            &[
                DispositionRequest {
                    child: key("sys.work.atomic-child"),
                    outcome: DispositionOutcome::IntentionallyDropped,
                    into: None,
                    note: None,
                },
                DispositionRequest {
                    child: key("sys.track.lighting"),
                    outcome: DispositionOutcome::IntentionallyDropped,
                    into: None,
                    note: None,
                },
            ],
        )
    });
}

#[test]
fn complete_and_abandon_refusals_write_nothing() {
    let sandbox = Sandbox::sys_tandem();

    let refused = refuses_without_writing(&sandbox, "unsatisfied check", |context| {
        ops::complete(context, &key("tandem.milestone.m5-one-playable-day"), &[])
    });
    assert_eq!(refused.code.as_str(), "AKR-R022");

    refuses_without_writing(&sandbox, "wrong kind", |context| {
        ops::complete(context, &key("tandem.policy.tandem-work"), &[])
    });
    refuses_without_writing(&sandbox, "unknown key", |context| {
        ops::complete(context, &key("tandem.milestone.absent"), &[])
    });

    refuses_without_writing(&sandbox, "no reason", |context| {
        ops::abandon(
            context,
            &key("simulator.work.playability-triage"),
            "   ",
            &[],
        )
    });
    refuses_without_writing(&sandbox, "wrong kind", |context| {
        ops::abandon(context, &key("tandem.term.bridge"), "why not", &[])
    });
    refuses_without_writing(&sandbox, "unknown key", |context| {
        ops::abandon(context, &key("tandem.work.absent"), "why not", &[])
    });
}

/// Sanity: the harness would notice a write. A successful operation changes the digests,
/// so the assertions above are not vacuously true.
#[test]
fn the_harness_detects_a_write() {
    let sandbox = Sandbox::save_your_skin();
    let context = WriteContext::new(sandbox.akr_dir());
    let before = digests(&sandbox);

    ops::complete(&context, &key("lege.work.extract-render-graph"), &[])
        .expect("a clean completion");

    assert_ne!(
        before,
        digests(&sandbox),
        "a successful write must change the tree"
    );
    assert!(
        !leftover_temporaries(&sandbox),
        "no temporary survives a success either"
    );
}

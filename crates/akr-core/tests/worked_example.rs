//! `examples/save-your-skin` end to end: load from disk, resolve, hash, lock, propagate.
//!
//! Exit criteria 2 and 5 of `docs/13-implementation-roadmap.md` P3:
//!
//! - the worked example passes `akr check` semantics with exit-0 equivalence
//!   (`MANIFEST.md` §9);
//! - content hashes are stable across a reformat.
//!
//! The example is also the best available cross-check on the resolver: its `akr.lock` was
//! written by hand from the specification, and every `(referring revision, slot, target)`
//! this code computes has to agree with it.

use akr_core::graph::propagate_staleness;
use akr_core::hash::content_hash;
use akr_core::lock::Lock;
use akr_core::model::RevisionId;
use akr_core::resolve::{BuildInputs, ResolvedModel, Workspace, load_workspace};
use akr_core::syntax::{format, parse};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

fn example_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/save-your-skin")
}

pub(crate) fn workspace() -> &'static Workspace {
    static WORKSPACE: OnceLock<Workspace> = OnceLock::new();
    WORKSPACE.get_or_init(|| {
        let root = example_root();
        load_workspace(&root, &root.join(".akr")).expect("the worked example loads")
    })
}

/// The build inputs the committed lock and the committed view banners were produced with.
///
/// `commit` is C5, the HEAD of the synthetic history in `MANIFEST.md` §4; `built_at` is
/// the frozen "today" of the same section. Both are *inputs*, not clock readings, so the
/// build stays a pure function of them (`docs/01-architecture.md` §4).
fn inputs(base: &BuildInputs) -> BuildInputs {
    BuildInputs {
        tool: "akr 0.1.0".to_owned(),
        grammar: "0.1".to_owned(),
        vocabulary: "0.1".to_owned(),
        commit: Some(
            akr_core::model::Commit::new("e806b3f54a2d7091c5e13b8a26f490dc7b135e64")
                .expect("a frozen commit"),
        ),
        built_at: "2026-08-03T09:14:00Z".to_owned(),
        ..base.clone()
    }
}

fn id(key: &str, revision: u32) -> RevisionId {
    RevisionId::new(akr_core::model::key(key), revision)
}

// -------------------------------------------------------------------------------------
// Exit criterion 2 — exit-0 equivalence
// -------------------------------------------------------------------------------------

#[test]
fn the_example_parses_and_lowers_without_diagnostics() {
    let workspace = workspace();
    assert!(
        workspace.diagnostics.is_empty(),
        "stages A and B must be clean:\n{}",
        workspace
            .diagnostics
            .iter()
            .map(|d| format!("  {} {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn the_example_resolves_clean() {
    // MANIFEST §9: `akr check` exits 0. The example is a valid ledger; every V-rule passes.
    let workspace = workspace();
    let model = ResolvedModel::build(&workspace.ledger, &inputs(&workspace.inputs));
    assert!(
        model.diagnostics.is_empty(),
        "expected a clean resolve, got:\n{}",
        model
            .diagnostics
            .iter()
            .map(|d| format!("  {} {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(!model.has_errors());
}

#[test]
fn the_example_has_the_inventory_the_manifest_freezes() {
    // MANIFEST §5: 40 keys, 42 revisions.
    let workspace = workspace();
    assert_eq!(workspace.ledger.records().len(), 42, "revisions");
    assert_eq!(workspace.ledger.keys().len(), 40, "keys");
    let model = ResolvedModel::build(&workspace.ledger, &inputs(&workspace.inputs));
    assert_eq!(model.heads.len(), 40, "every key resolves to a head");
    assert!(model.head_errors.is_empty());
}

#[test]
fn every_key_lives_in_one_file() {
    // V-003, and the thing that makes a key's whole history one diff (D-018).
    let workspace = workspace();
    for key in workspace.ledger.keys() {
        let files: BTreeSet<Option<String>> = workspace
            .ledger
            .revisions_of(key)
            .iter()
            .map(|r| r.file.clone())
            .collect();
        assert_eq!(files.len(), 1, "{key} is split across {files:?}");
    }
}

#[test]
fn the_two_multi_revision_keys_have_the_chains_the_manifest_describes() {
    let workspace = workspace();
    let model = ResolvedModel::build(&workspace.ledger, &inputs(&workspace.inputs));
    for key in ["lege.decision.renderer-boundary", "sys.work.m3-plan"] {
        let chain = model
            .supersession
            .get(&akr_core::model::key(key))
            .unwrap_or_else(|| panic!("{key} has a chain"));
        assert_eq!(chain, &vec![id(key, 1), id(key, 2)], "{key}");
        assert_eq!(model.current(&id(key, 1)), id(key, 2));
        assert!(model.is_head(&id(key, 2)));
    }
}

// -------------------------------------------------------------------------------------
// The lock: cross-checking the resolver against a hand-written specification artefact
// -------------------------------------------------------------------------------------

#[test]
fn the_committed_lock_parses() {
    let workspace = workspace();
    let text = workspace
        .lock_text
        .as_deref()
        .expect("the example has a lock");
    let lock = Lock::parse(text).expect("the committed lock parses");
    assert_eq!(lock.project, "save-your-skin");
    assert_eq!(lock.build.tool, "akr 0.1.0");
    assert_eq!(lock.build.built_at, "2026-08-03T09:14:00Z");
}

/// Set `AKR_REGENERATE=1` to rewrite the committed lock from the sources.
///
/// The same discipline as the roadmap snapshot: the artefact is build output, pinned by a
/// test, and regenerating it is a deliberate act that shows up as a diff.
fn regenerating() -> bool {
    std::env::var("AKR_REGENERATE").is_ok_and(|v| v == "1")
}

#[test]
fn the_committed_lock_is_reproduced_byte_for_byte() {
    // `akr.lock` is build output (D-014), so it is snapshot-tested exactly like the views.
    // Every digest in it is real: the source hashes are SHA-256 over raw disk bytes, the
    // seal hashes over canonical record text, and the source-graph hash over the sorted
    // pairs — which is why it agrees with the `source-graph:` line of all six view banners.
    let workspace = workspace();
    let model = ResolvedModel::build(&workspace.ledger, &inputs(&workspace.inputs));
    let rendered = model.to_lock().render();
    let path = example_root().join(".akr/akr.lock");

    if regenerating() {
        std::fs::write(&path, &rendered).expect("writable");
        return;
    }

    let committed = std::fs::read_to_string(&path).expect("the committed lock");
    if committed != rendered {
        let first = committed
            .lines()
            .zip(rendered.lines())
            .enumerate()
            .find(|(_, (a, b))| a != b)
            .map(|(n, (a, b))| (n + 1, a.to_owned(), b.to_owned()));
        panic!("akr.lock differs at {first:?}; rerun with AKR_REGENERATE=1 to update");
    }
}

#[test]
fn the_lock_source_graph_matches_every_view_banner() {
    // `docs/11-projections.md` §4: the source-graph hash is identical across all six views
    // of one build. It is the same value the lock records, which is what lets a reader
    // holding a view tell whether it matches the ledger in front of them.
    let workspace = workspace();
    let model = ResolvedModel::build(&workspace.ledger, &inputs(&workspace.inputs));
    let expected = format!("     source-graph: {}", model.source_graph);
    let generated = example_root().join("docs/generated");
    for view in [
        "ROADMAP.md",
        "CURRENT-STATE.md",
        "ACTIVE-WORK.md",
        "REVIEW-REQUIRED.md",
        "OPEN-QUESTIONS.md",
        "DECISION-HISTORY.md",
    ] {
        let text = std::fs::read_to_string(generated.join(view)).expect("a committed view");
        assert!(
            text.lines().nth(1) == Some(expected.as_str()),
            "{view} banner does not carry the lock's source-graph hash"
        );
    }
}

#[test]
fn the_computed_lock_agrees_with_the_committed_one() {
    // Structural agreement, asserted separately from the byte snapshot so that a failure
    // says *what* disagreed rather than only *where*.
    let workspace = workspace();
    let model = ResolvedModel::build(&workspace.ledger, &inputs(&workspace.inputs));
    let computed = model.to_lock();
    let committed = Lock::parse(workspace.lock_text.as_deref().expect("a lock")).expect("parses");

    let triples = |lock: &Lock| -> BTreeSet<String> {
        lock.resolutions
            .iter()
            .map(|r| format!("{} {} {}", r.from, r.slot, r.to))
            .collect()
    };
    assert_eq!(triples(&computed), triples(&committed), "resolutions");

    let seals = |lock: &Lock| -> BTreeSet<String> {
        lock.seals
            .iter()
            .map(|s| format!("{} {}", s.id, s.state))
            .collect()
    };
    assert_eq!(seals(&computed), seals(&committed), "seals");

    let paths =
        |lock: &Lock| -> BTreeSet<String> { lock.sources.iter().map(|s| s.path.clone()).collect() };
    assert_eq!(paths(&computed), paths(&committed), "sources");

    let records = |lock: &Lock| -> BTreeMap<String, u32> {
        lock.sources
            .iter()
            .map(|s| (s.path.clone(), s.records))
            .collect()
    };
    assert_eq!(records(&computed), records(&committed), "record counts");
}

#[test]
fn the_lock_seals_exactly_the_non_proposed_revisions() {
    // 42 revisions, 3 of them `proposed` (MANIFEST §5), so 39 seals.
    let workspace = workspace();
    let model = ResolvedModel::build(&workspace.ledger, &inputs(&workspace.inputs));
    let lock = model.to_lock();
    assert_eq!(lock.seals.len(), 39);
    let proposed = workspace
        .ledger
        .records()
        .iter()
        .filter(|r| !r.is_sealed())
        .count();
    assert_eq!(proposed, 3);
}

#[test]
fn a_generated_lock_round_trips_and_verifies() {
    let workspace = workspace();
    let model = ResolvedModel::build(&workspace.ledger, &inputs(&workspace.inputs));
    let lock = model.to_lock();
    let rendered = lock.render();
    let reparsed = Lock::parse(&rendered).expect("round trip");
    assert_eq!(reparsed.render(), rendered, "rendering is a fixed point");
    assert!(reparsed.verify(&lock).is_empty(), "verifies against itself");
}

// -------------------------------------------------------------------------------------
// Exit criterion 5 — hashes stable across a reformat
// -------------------------------------------------------------------------------------

#[test]
fn every_revision_gets_a_content_hash() {
    let workspace = workspace();
    let model = ResolvedModel::build(&workspace.ledger, &inputs(&workspace.inputs));
    assert_eq!(model.content_hashes.len(), 42);
    assert!(
        model.missing_hashes.is_empty(),
        "the formatter supplied canonical text for every revision"
    );
    assert!(
        model
            .content_hashes
            .values()
            .all(|h| h.0.len() == "sha256:".len() + 64)
    );
}

#[test]
fn content_hashes_survive_a_reformat() {
    // Take every record file, mangle its formatting in ways `akr fmt` undoes — extra
    // blank lines, doubled indentation, a comment inserted between slots — reformat, and
    // require every content hash to be unchanged. This is what makes `AKR-R051` mean
    // "somebody edited a sealed record" rather than "somebody ran the formatter".
    let workspace = workspace();
    let baseline = ResolvedModel::build(&workspace.ledger, &inputs(&workspace.inputs))
        .content_hashes
        .clone();
    assert!(!baseline.is_empty());

    let root = example_root();
    let mut compared = 0;
    for source in &workspace.inputs.sources {
        let path = root.join(&source.path);
        let original = std::fs::read_to_string(&path).expect("readable");
        let mangled = mangle(&original);
        assert_ne!(mangled, original, "{} was actually mangled", source.path);

        let parsed = parse(&mangled, akr_core::diagnostics::FileId(0));
        let Some(file) = parsed.file else { continue };
        assert!(
            parsed.diagnostics.is_empty(),
            "{}: mangled text must still parse:\n{:?}",
            source.path,
            parsed
                .diagnostics
                .iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
        );

        for at in 0..file.items.len() {
            let akr_core::syntax::cst::Item::Record(record) = &file.items[at] else {
                continue;
            };
            let Ok(key) = akr_core::model::LogicalKey::parse(&record.key) else {
                continue;
            };
            let revision = RevisionId::new(key, record.revision);
            let canonical = akr_core::resolve::canonical_record_text(&file, at).expect("a record");
            let Some(expected) = baseline.get(&revision) else {
                continue;
            };
            assert_eq!(
                &content_hash(&canonical),
                expected,
                "{revision} changed hash after a reformat"
            );
            compared += 1;
        }
    }
    assert_eq!(compared, 42, "every revision was compared");
}

/// Formatting damage that `akr fmt` is required to undo (D-012, D-006).
fn mangle(text: &str) -> String {
    let mut out = String::new();
    let mut in_prose = false;
    for line in text.lines() {
        if line.matches("\"\"\"").count() % 2 == 1 {
            in_prose = !in_prose;
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if in_prose {
            // Prose content is raw and load-bearing; leave it exactly alone.
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if line.trim().is_empty() {
            out.push_str("\n\n");
            continue;
        }
        if line.starts_with("    ") && !line.trim_start().starts_with('#') {
            out.push_str("        # inserted comment\n");
        }
        out.push_str(line);
        out.push_str("   \n");
    }
    out
}

#[test]
fn the_formatter_is_a_fixed_point_on_the_example() {
    // Every committed `.akr` file is canonical, which is what lets the content hash be
    // taken over it directly.
    let workspace = workspace();
    let root = example_root();
    for source in &workspace.inputs.sources {
        let text = std::fs::read_to_string(root.join(&source.path)).expect("readable");
        let parsed = parse(&text, akr_core::diagnostics::FileId(0));
        let file = parsed.file.expect("parses");
        assert_eq!(format(&file), text, "{} is not canonical", source.path);
    }
}

// -------------------------------------------------------------------------------------
// Propagation over the real corpus — MANIFEST §7, as amended
// -------------------------------------------------------------------------------------

#[test]
fn propagation_over_the_example_matches_the_manifest() {
    // MANIFEST §7: two stale records, four at risk, propagation depth 2. The stale set
    // itself is P5's to derive from `observed_at`, `watches` and `review_after`; here it
    // is supplied, which is exactly the interface P5 will use.
    let workspace = workspace();
    let stale: BTreeSet<RevisionId> = [
        id("sim.obs.projection-gaps", 1),
        id("sim.obs.timestep-drift", 1),
    ]
    .into();

    let at_risk = propagate_staleness(&workspace.ledger, &stale);
    let described: Vec<String> = at_risk
        .iter()
        .map(|r| format!("{} depth {} via {}", r.id, r.depth, r.via))
        .collect();
    assert_eq!(
        described,
        [
            "sim.work.rewrite-projection/1 depth 1 via depends_on",
            "sys.assessment.m3-readiness/1 depth 1 via supported_by",
            "sys.assessment.projection-gaps/1 depth 1 via supported_by",
            "sys.policy.tandem-work/1 depth 2 via supported_by",
        ]
    );
    assert_eq!(at_risk.len(), 4);
    assert_eq!(at_risk.iter().map(|r| r.depth).max(), Some(2));
}

#[test]
fn the_at_risk_policy_records_its_full_path() {
    let workspace = workspace();
    let stale: BTreeSet<RevisionId> = [id("sim.obs.projection-gaps", 1)].into();
    let at_risk = propagate_staleness(&workspace.ledger, &stale);
    let policy = at_risk
        .iter()
        .find(|r| r.id == id("sys.policy.tandem-work", 1))
        .expect("the policy is at risk");
    assert_eq!(
        policy
            .path
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        [
            "sys.assessment.projection-gaps/1",
            "sim.obs.projection-gaps/1"
        ]
    );
}

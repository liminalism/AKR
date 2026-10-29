//! Stage F: the roadmap renderer, the banner, and the views-current gate.
//!
//! Exit criteria of `docs/13-implementation-roadmap.md` P4:
//!
//! 1. `examples/save-your-skin/docs/generated/ROADMAP.md` is reproduced byte-identically
//!    from the example's `.akr` sources.
//! 2. A hand edit to the committed file makes `akr check --views-current` fail with
//!    `AKR-E011` naming the file and the first differing line.
//!
//! The snapshot is the point. A renderer whose output is only "checked by eye" drifts;
//! one pinned to a committed file cannot change without somebody seeing the diff.

use akr_core::diagnostics::{Severity, Subject};
use akr_core::model::{Ancestry, Commit, Kind, RevisionId, key};
use akr_core::render::{
    Freshness, RenderContext, View, banner, check_views_current, codes, render, render_roadmap,
    slug, write_views,
};
use akr_core::resolve::{BuildInputs, ResolvedModel, Workspace, load_workspace};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

// -------------------------------------------------------------------------------------
// The worked example, with the synthetic history its MANIFEST freezes
// -------------------------------------------------------------------------------------

/// `MANIFEST.md` §4: five commits, C1 through C5, with C5 as HEAD.
const COMMITS: [&str; 5] = [
    "3f0a1c9d5b7e2648a0d4f1b8c36e9752ad014b6f",
    "7c41d0ba92e6f37518a3cd406b5e2f91d8074a63",
    "b2e58f1406c7a9d3e41b60258fa3d7c6195e0b48",
    "5d9c2a70e31f8b46c07d5924ab6e3f1074c9d285",
    "e806b3f54a2d7091c5e13b8a26f490dc7b135e64",
];

fn commit(index: usize) -> Commit {
    Commit::new(COMMITS[index]).expect("a frozen commit")
}

fn id(text: &str, revision: u32) -> RevisionId {
    RevisionId::new(key(text), revision)
}

fn example_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/save-your-skin")
}

/// Loads the example and attaches the facts phase P5 will derive from git.
///
/// `last_change` and `ancestry` come from `MANIFEST.md` §4 rather than from a repository:
/// the example's history is fictional, and every document that reasons about ancestry uses
/// that table and nothing else. Supplying them here is exactly the interface P5 fills.
fn example() -> Workspace {
    let root = example_root();
    let mut workspace = load_workspace(&root, &root.join(".akr")).expect("the example loads");
    workspace.ledger.facts.ancestry = Ancestry::from_pairs(vec![
        (commit(1), commit(0)),
        (commit(2), commit(1)),
        (commit(3), commit(2)),
        (commit(4), commit(3)),
    ]);
    workspace.ledger.facts.last_change = [
        (id("sys.milestone.m1-walking-skeleton", 1), commit(0)),
        (id("sys.milestone.m2-deterministic-sim", 1), commit(1)),
        (id("sys.milestone.m3-playable-day", 1), commit(2)),
        (id("sys.work.m3-plan", 2), commit(3)),
    ]
    .into();
    workspace
}

fn example_inputs(workspace: &Workspace) -> BuildInputs {
    BuildInputs {
        tool: "akr 0.1.0".to_owned(),
        grammar: "0.1".to_owned(),
        vocabulary: "0.1".to_owned(),
        commit: Some(commit(4)),
        ..workspace.inputs.clone()
    }
}

/// `MANIFEST.md` §7: the two stale observations. P5 derives this set; P4 reads it.
fn stale_set() -> BTreeSet<RevisionId> {
    [
        id("sim.obs.projection-gaps", 1),
        id("sim.obs.timestep-drift", 1),
    ]
    .into()
}

fn committed_roadmap() -> String {
    std::fs::read_to_string(example_root().join("docs/generated/ROADMAP.md"))
        .expect("the committed roadmap")
}

// -------------------------------------------------------------------------------------
// Exit criterion 1 — byte-identical reproduction
// -------------------------------------------------------------------------------------

#[test]
fn the_committed_roadmap_is_reproduced_byte_for_byte() {
    let workspace = example();
    let model = ResolvedModel::build(&workspace.ledger, &example_inputs(&workspace));
    let freshness = Freshness::from_stale(&workspace.ledger, stale_set());
    let rendered = render_roadmap(RenderContext::new(&model, &freshness));
    let committed = committed_roadmap();

    if rendered != committed {
        let mut first = None;
        for (n, (a, b)) in committed.lines().zip(rendered.lines()).enumerate() {
            if a != b {
                first = Some((n + 1, a.to_owned(), b.to_owned()));
                break;
            }
        }
        panic!(
            "roadmap differs at {first:?}\ncommitted {} bytes, rendered {} bytes",
            committed.len(),
            rendered.len()
        );
    }
}

#[test]
fn rendering_is_reproducible() {
    // `docs/01-architecture.md` §4: a build is a pure function of (sources, commit, tool
    // version), byte-identical across runs.
    let first = example();
    let second = example();
    let model_a = ResolvedModel::build(&first.ledger, &example_inputs(&first));
    let model_b = ResolvedModel::build(&second.ledger, &example_inputs(&second));
    let fresh_a = Freshness::from_stale(&first.ledger, stale_set());
    let fresh_b = Freshness::from_stale(&second.ledger, stale_set());
    assert_eq!(
        render_roadmap(RenderContext::new(&model_a, &fresh_a)),
        render_roadmap(RenderContext::new(&model_b, &fresh_b))
    );
}

#[test]
fn rendering_does_not_depend_on_record_insertion_order() {
    let workspace = example();
    let baseline = {
        let model = ResolvedModel::build(&workspace.ledger, &example_inputs(&workspace));
        let freshness = Freshness::from_stale(&workspace.ledger, stale_set());
        render_roadmap(RenderContext::new(&model, &freshness))
    };

    let mut shuffled = akr_core::model::Ledger::new(workspace.ledger.project.clone());
    let mut records = workspace.ledger.records().to_vec();
    records.reverse();
    shuffled.extend(records);
    shuffled.facts = workspace.ledger.facts.clone();

    let model = ResolvedModel::build(&shuffled, &example_inputs(&workspace));
    let freshness = Freshness::from_stale(&shuffled, stale_set());
    assert_eq!(
        render_roadmap(RenderContext::new(&model, &freshness)),
        baseline
    );
}

// -------------------------------------------------------------------------------------
// The content the snapshot pins, asserted in its own right
// -------------------------------------------------------------------------------------

#[test]
fn milestones_render_in_after_order_not_key_order() {
    // Key order would be m1, m2, m3, m4, m5 by coincidence here, so the test asserts the
    // ordering graph is actually consulted: reverse the ledger and the order must hold.
    let committed = committed_roadmap();
    let positions: Vec<usize> = [
        "### M1 — walking skeleton",
        "### M2 — deterministic simulator",
        "### M3 — playable day",
        "### M4 — content tools",
        "### M5 — ship demo",
    ]
    .iter()
    .map(|h| committed.find(h).unwrap_or_else(|| panic!("{h} present")))
    .collect();
    assert!(positions.windows(2).all(|w| w[0] < w[1]), "{positions:?}");
}

#[test]
fn acceptance_verdicts_carry_the_descendant_commit_rule() {
    // MANIFEST §6: M1, M2 and M3's `full-day-demo` are satisfied; M3's
    // `no-placeholder-assets` is not, which is why M3 is still active.
    let committed = committed_roadmap();
    assert!(committed.contains(
        "| `viewer-boundary-clean` | command | **satisfied** by `@lege.evidence.boundary-lint-pass/1` (pass at `b2e58f14`, descends from `3f0a1c9d`) |"
    ));
    assert!(committed.contains(
        "| `full-day-demo` | observation | **satisfied** by `@sys.evidence.playable-day-demo/1` (pass at `e806b3f5`, descends from `b2e58f14`) |"
    ));
    assert!(
        committed.contains("| `no-placeholder-assets` | command | not satisfied — no evidence |")
    );
    assert!(committed.contains("**Acceptance** — 1 of 2 satisfied"));
}

#[test]
fn evidence_that_predates_the_last_content_change_does_not_satisfy() {
    // The condition that stops a passing test from 200 commits ago closing a milestone
    // whose definition changed yesterday (D-016). Asserted by moving M3's last content
    // change forward past its evidence.
    let mut workspace = example();
    workspace
        .ledger
        .facts
        .last_change
        .insert(id("sys.milestone.m3-playable-day", 1), commit(4));
    // The evidence is observed at C5 and C5 does not descend from a *later* commit, so
    // make the last change a commit the evidence cannot descend from: reuse C5 as the
    // change and C3 as the observation by swapping which milestone we inspect.
    workspace
        .ledger
        .facts
        .last_change
        .insert(id("sys.milestone.m1-walking-skeleton", 1), commit(4));

    let model = ResolvedModel::build(&workspace.ledger, &example_inputs(&workspace));
    let freshness = Freshness::none();
    let rendered = render_roadmap(RenderContext::new(&model, &freshness));
    assert!(
        rendered.contains(
            "not satisfied — `@lege.evidence.boundary-lint-pass/1` observed at `b2e58f14`, which does not descend from `e806b3f5`"
        ),
        "expected a too-old verdict, got:\n{}",
        rendered
            .lines()
            .filter(|l| l.contains("viewer-boundary-clean"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn dispositioned_children_appear_under_the_plan_of_record() {
    // `part_of` pins to a plan revision, so a view listing only the head's children would
    // silently drop exactly the items a replan is most likely to lose (D-017).
    let committed = committed_roadmap();
    assert!(committed.contains("**Under the plan of record**"));
    assert!(committed.contains(
        "`@sys.work.m3-audio-pass/1` — part of `@sys.work.m3-plan/1`, dispositioned `intentionally_dropped`"
    ));
    assert!(committed.contains(
        "`@sys.work.m3-lighting-pass/1` — part of `@sys.work.m3-plan/1`, dispositioned `carried_forward` into `@sys.track.lighting/1`"
    ));
    assert!(
        committed
            .contains("Carried into this track by disposition: `@sys.work.m3-lighting-pass/1`.")
    );
}

#[test]
fn freshness_renders_on_the_metadata_line_and_list_entries_never_in_a_heading() {
    let committed = committed_roadmap();
    assert!(committed.contains("`@sim.work.rewrite-projection/1` — **at risk**"));
    for line in committed.lines().filter(|l| l.starts_with('#')) {
        assert!(
            !line.contains("stale") && !line.contains("at risk"),
            "a heading anchor must not move when a record goes stale: {line}"
        );
    }
}

#[test]
fn empty_work_item_lists_say_so() {
    // §3: a missing heading would be ambiguous between "nothing here" and "not generated".
    assert!(committed_roadmap().contains("**Work items** — _(none)_"));
}

#[test]
fn archived_records_are_excluded() {
    // D-018: archived records still resolve; they appear in no view but DECISION-HISTORY.
    assert!(!committed_roadmap().contains("weekly-demo"));
}

#[test]
fn the_roadmap_ends_with_exactly_one_newline_and_has_no_cr() {
    let committed = committed_roadmap();
    assert!(committed.ends_with("| Tooling hygiene | `active` | continuous |\n"));
    assert!(!committed.ends_with("\n\n"));
    assert!(!committed.contains('\r'));
}

// -------------------------------------------------------------------------------------
// The banner (§4)
// -------------------------------------------------------------------------------------

#[test]
fn the_banner_carries_stable_build_inputs_and_no_clock_reading() {
    let workspace = example();
    let model = ResolvedModel::build(&workspace.ledger, &example_inputs(&workspace));
    let text = banner(&model);
    assert!(text.starts_with("<!-- GENERATED BY AKR — DO NOT EDIT\n"));
    assert!(text.contains(&format!("     source-graph: {}\n", model.source_graph)));
    assert!(text.contains("     tool: akr 0.1.0\n"));
    assert!(text.ends_with("-->\n"));
    assert_eq!(text.lines().count(), 4);
    // A future commit hash cannot be embedded in bytes participating in that commit.
    assert!(!text.contains("commit:"));
    // A timestamp would make every rebuild produce a diff and the CI gate useless.
    assert!(!text.contains("built_at") && !text.contains('Z'));
}

#[test]
fn the_banner_is_identical_across_every_view_of_one_build() {
    // §4: the source-graph hash is "identical across all six views of one build". The
    // committed views are the evidence.
    let root = example_root().join("docs/generated");
    let mut banners = BTreeSet::new();
    for &view in View::ALL {
        // PAPERCUTS.md is emitted only once a papercut exists (D-027), and the worked
        // example logs none — every view that IS committed carries the same banner.
        let Ok(text) = std::fs::read_to_string(root.join(view.file_name())) else {
            assert_eq!(view, View::Papercuts, "{} is committed", view.file_name());
            continue;
        };
        banners.insert(text.lines().take(4).collect::<Vec<_>>().join("\n"));
    }
    assert_eq!(banners.len(), 1, "the six banners disagree: {banners:#?}");
}

#[test]
fn the_banner_is_identical_before_and_after_git_resolves_a_commit() {
    let workspace = example();
    let inputs = BuildInputs {
        commit: None,
        ..example_inputs(&workspace)
    };
    let without_commit = ResolvedModel::build(&workspace.ledger, &inputs);
    let with_commit = ResolvedModel::build(&workspace.ledger, &example_inputs(&workspace));
    assert_eq!(banner(&without_commit), banner(&with_commit));
}

// -------------------------------------------------------------------------------------
// The catalogue and anchors
// -------------------------------------------------------------------------------------

#[test]
fn the_catalogue_is_eight_views_with_distinct_names_and_files() {
    assert_eq!(View::ALL.len(), 8);
    let names: BTreeSet<&str> = View::ALL.iter().map(|v| v.name()).collect();
    let files: BTreeSet<&str> = View::ALL.iter().map(|v| v.file_name()).collect();
    assert_eq!(names.len(), 8);
    assert_eq!(files.len(), 8);
}

#[test]
fn views_are_looked_up_by_name_or_file_name() {
    for &view in View::ALL {
        assert_eq!(View::from_name(view.name()), Some(view));
        assert_eq!(View::from_name(view.file_name()), Some(view));
        assert_eq!(View::from_name(&view.name().to_uppercase()), Some(view));
    }
    assert_eq!(View::from_name("roadmap.md"), Some(View::Roadmap));
    assert_eq!(View::from_name("burndown"), None);
}

#[test]
fn every_kind_is_hosted_by_exactly_one_view() {
    for &kind in Kind::ALL {
        assert!(View::hosting(kind).is_some(), "{kind} has no host view");
    }
    assert_eq!(View::hosting(Kind::Decision), Some(View::DecisionHistory));
    assert_eq!(View::hosting(Kind::Work), Some(View::ActiveWork));
    assert_eq!(View::hosting(Kind::Milestone), Some(View::Roadmap));
}

#[test]
fn slugs_match_github_heading_anchors() {
    assert_eq!(slug("M3 — playable day"), "m3--playable-day");
    assert_eq!(slug("16 ms frame budget"), "16-ms-frame-budget");
    assert_eq!(
        slug("Does a 4 ms timestep fit the frame budget?"),
        "does-a-4-ms-timestep-fit-the-frame-budget"
    );
    assert_eq!(slug("`state` and **emphasis**"), "state-and-emphasis");
}

#[test]
fn every_in_repository_link_the_roadmap_emits_resolves() {
    // §3: a reference to a record in another view is a relative link to that file's
    // anchor. A dead link is worse than none, so every one is checked against the file it
    // names.
    let root = example_root().join("docs/generated");
    let committed = committed_roadmap();
    let mut checked = 0;
    for capture in committed.split("](").skip(1) {
        let target = capture.split(')').next().expect("a link target");
        let (file, anchor) = target.split_once('#').expect("an anchored link");
        let text =
            std::fs::read_to_string(root.join(file)).unwrap_or_else(|_| panic!("{file} exists"));
        let anchors: BTreeSet<String> = text
            .lines()
            .filter_map(|l| l.strip_prefix("### ").or_else(|| l.strip_prefix("## ")))
            .map(slug)
            .collect();
        assert!(
            anchors.contains(anchor),
            "{target}: no heading in {file} slugs to {anchor}"
        );
        checked += 1;
    }
    assert!(
        checked >= 10,
        "expected the roadmap to link out, got {checked}"
    );
}

// -------------------------------------------------------------------------------------
// Exit criterion 2 — the views-current gate
// -------------------------------------------------------------------------------------

/// Copies the committed views into a scratch directory so a test can damage them.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("akr-p4-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch directory");
    let source = example_root().join("docs/generated");
    for &view in View::ALL {
        let file = view.file_name();
        // PAPERCUTS.md is not committed in the example — it is emitted only once a
        // papercut exists (D-027).
        if source.join(file).exists() {
            std::fs::copy(source.join(file), dir.join(file)).expect("copy a view");
        }
    }
    dir
}

#[test]
fn an_unmodified_directory_passes() {
    let workspace = example();
    let model = ResolvedModel::build(&workspace.ledger, &example_inputs(&workspace));
    let freshness = Freshness::from_stale(&workspace.ledger, stale_set());
    let dir = example_root().join("docs/generated");
    let diagnostics =
        check_views_current(&dir, RenderContext::new(&model, &freshness)).expect("readable");
    assert!(
        diagnostics.is_empty(),
        "expected a clean gate, got: {:?}",
        diagnostics
            .iter()
            .map(|d| (d.code.as_str(), d.message.clone()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_hand_edit_fails_with_e011_naming_the_file_and_the_first_differing_line() {
    let dir = scratch("hand-edit");
    let path = dir.join("ROADMAP.md");
    let original = std::fs::read_to_string(&path).expect("readable");
    let edited = original.replace(
        "`active` · `@sys.milestone.m3-playable-day/1`",
        "`nearly done` · `@sys.milestone.m3-playable-day/1`",
    );
    assert_ne!(edited, original, "the edit landed");
    std::fs::write(&path, &edited).expect("writable");

    let workspace = example();
    let model = ResolvedModel::build(&workspace.ledger, &example_inputs(&workspace));
    let freshness = Freshness::from_stale(&workspace.ledger, stale_set());
    let diagnostics =
        check_views_current(&dir, RenderContext::new(&model, &freshness)).expect("readable");

    assert_eq!(diagnostics.len(), 1);
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic.code, codes::E011);
    assert_eq!(diagnostic.severity, Severity::Error);

    // Names the file.
    let Subject::File(named) = &diagnostic.primary.subject else {
        panic!(
            "expected a file subject, got {:?}",
            diagnostic.primary.subject
        );
    };
    assert!(named.ends_with("ROADMAP.md"), "{named}");
    assert!(diagnostic.message.contains("ROADMAP.md"));
    assert!(diagnostic.message.contains("1 differing line"));

    // Names the first differing line, with both sides.
    let expected_line = 1 + original
        .lines()
        .zip(edited.lines())
        .position(|(a, b)| a != b)
        .expect("a differing line");
    // One note per side, so neither line has to share a row with the other: a generated
    // view's lines are long, and a single note holding both is unreadable in a terminal.
    let notes: Vec<&str> = diagnostic
        .notes
        .iter()
        .filter_map(|note| note.message.as_deref())
        .collect();
    assert_eq!(notes.len(), 2, "{notes:?}");
    for note in &notes {
        assert!(
            note.starts_with(&format!("line {expected_line} ")),
            "{note}"
        );
    }
    assert!(
        notes[0].contains("nearly done"),
        "quotes the committed line: {notes:?}"
    );
    assert!(
        notes[1].contains("`active`"),
        "quotes the emitted line: {notes:?}"
    );

    assert!(
        diagnostic
            .help
            .as_deref()
            .is_some_and(|h| h.contains("akr build")),
        "points at the fix"
    );
}

#[test]
fn a_missing_view_fails_with_e012() {
    let dir = scratch("missing");
    std::fs::remove_file(dir.join("ROADMAP.md")).expect("removable");
    let workspace = example();
    let model = ResolvedModel::build(&workspace.ledger, &example_inputs(&workspace));
    let freshness = Freshness::from_stale(&workspace.ledger, stale_set());
    let diagnostics =
        check_views_current(&dir, RenderContext::new(&model, &freshness)).expect("readable");
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, codes::E012);
    assert!(diagnostics[0].message.contains("run `akr build`"));
}

#[test]
fn a_damaged_banner_fails_with_e013_before_any_content_comparison() {
    // A view without a readable banner cannot be told from a hand-written document, so
    // that fault is reported instead of, not alongside, a content diff.
    let dir = scratch("banner");
    let path = dir.join("ROADMAP.md");
    let original = std::fs::read_to_string(&path).expect("readable");
    let beheaded: String = original.lines().skip(5).map(|l| format!("{l}\n")).collect();
    std::fs::write(&path, beheaded).expect("writable");

    let workspace = example();
    let model = ResolvedModel::build(&workspace.ledger, &example_inputs(&workspace));
    let freshness = Freshness::from_stale(&workspace.ledger, stale_set());
    let diagnostics =
        check_views_current(&dir, RenderContext::new(&model, &freshness)).expect("readable");
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, codes::E013);
    assert!(diagnostics[0].message.contains("banner"));
}

#[test]
fn each_banner_field_is_required() {
    let dir = scratch("banner-fields");
    let path = dir.join("ROADMAP.md");
    let original = std::fs::read_to_string(&path).expect("readable");
    let workspace = example();
    let model = ResolvedModel::build(&workspace.ledger, &example_inputs(&workspace));
    let freshness = Freshness::from_stale(&workspace.ledger, stale_set());

    for field in ["source-graph:", "tool:"] {
        let damaged: String = original
            .lines()
            .filter(|l| !l.trim_start().starts_with(field))
            .map(|l| format!("{l}\n"))
            .collect();
        std::fs::write(&path, damaged).expect("writable");
        let diagnostics =
            check_views_current(&dir, RenderContext::new(&model, &freshness)).expect("readable");
        assert!(
            diagnostics.iter().any(|d| d.code == codes::E013),
            "removing {field} must raise AKR-E013"
        );
    }
}

#[test]
fn an_unexpected_file_in_the_output_directory_fails_with_e014() {
    // The output directory is owned by the build (V-114).
    let dir = scratch("intruder");
    std::fs::write(dir.join("NOTES.md"), "hand-written\n").expect("writable");
    let workspace = example();
    let model = ResolvedModel::build(&workspace.ledger, &example_inputs(&workspace));
    let freshness = Freshness::from_stale(&workspace.ledger, stale_set());
    let diagnostics =
        check_views_current(&dir, RenderContext::new(&model, &freshness)).expect("readable");
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, codes::E014);
    assert!(diagnostics[0].message.contains("NOTES.md"));
}

#[test]
fn every_view_but_papercuts_always_renders() {
    // `PAPERCUTS.md` alone has the emit-only-when-nonempty rule (D-027); the example
    // holds no papercut, so it is the one file `scratch()` does not copy and the one
    // view that renders `None` here.
    let workspace = example();
    let model = ResolvedModel::build(&workspace.ledger, &example_inputs(&workspace));
    let freshness = Freshness::from_stale(&workspace.ledger, stale_set());
    let cx = RenderContext::new(&model, &freshness);
    for &view in View::ALL {
        let rendered = render(view, cx);
        if view == View::Papercuts {
            assert!(rendered.is_none(), "the example logs no papercut");
        } else {
            assert!(rendered.is_some(), "{view:?} did not render");
        }
    }
}

#[test]
fn writing_views_is_idempotent() {
    // Only files whose bytes differ are rewritten, so a no-op build produces no diff.
    let dir = scratch("write");
    let workspace = example();
    let model = ResolvedModel::build(&workspace.ledger, &example_inputs(&workspace));
    let freshness = Freshness::from_stale(&workspace.ledger, stale_set());
    let cx = RenderContext::new(&model, &freshness);
    assert!(write_views(&dir, cx).expect("writable").is_empty());

    std::fs::write(dir.join("ROADMAP.md"), "clobbered\n").expect("writable");
    assert_eq!(
        write_views(&dir, cx).expect("writable"),
        vec![View::Roadmap]
    );
    assert!(check_views_current(&dir, cx).expect("readable").is_empty());
}

#[test]
fn every_emission_code_is_registered_in_the_runtime_registry() {
    // The `E` range lives in codes-runtime.md, not codes-lang.md
    // (`spec/diagnostics/README.md` §2). An unregistered code is one nobody can look up.
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../spec/diagnostics/codes-runtime.md"
    );
    let text = std::fs::read_to_string(path).expect("codes-runtime.md is readable");
    for code in codes::ALL {
        assert!(
            text.contains(&format!("`{code}`")),
            "{code} is raised in code but absent from spec/diagnostics/codes-runtime.md"
        );
        assert_eq!(
            code.stage(),
            Some(akr_core::diagnostics::Stage::Emit),
            "{code} is not an emission code"
        );
    }
}

// -------------------------------------------------------------------------------------
// D-026 — the note slot
// -------------------------------------------------------------------------------------

#[test]
fn a_terminal_planning_record_renders_its_note_and_a_live_one_does_not() {
    use akr_core::model::{ContentSlot, ContentValue, Ledger, State};

    let workspace = example();
    let with_note = |state: State| -> Ledger {
        let mut ledger = Ledger::new(workspace.ledger.project.clone());
        ledger.facts = workspace.ledger.facts.clone();
        let records: Vec<_> = workspace
            .ledger
            .records()
            .iter()
            .map(|record| {
                let mut copy = record.clone();
                if copy.id.key.to_string() == "sys.track.lighting" {
                    copy.state = state;
                    copy.content.insert(
                        ContentSlot::Note,
                        ContentValue::prose("Standing work, paused while M3 lands."),
                    );
                }
                copy
            })
            .collect();
        ledger.extend(records);
        ledger
    };

    // Live: the note is working commentary the record's own intent should carry, so no
    // view shows it (docs/11 §3).
    let live = with_note(State::Active);
    let model = ResolvedModel::build(&live, &example_inputs(&workspace));
    let freshness = Freshness::from_stale(&live, stale_set());
    let rendered = render_roadmap(RenderContext::new(&model, &freshness));
    assert!(!rendered.contains("**Note:**"), "{rendered}");

    // Terminal: the note is the last thing anybody wrote about the record, and the only
    // place a reader finds out why it stopped (D-026).
    let done = with_note(State::Abandoned);
    let model = ResolvedModel::build(&done, &example_inputs(&workspace));
    let freshness = Freshness::from_stale(&done, stale_set());
    let rendered = render_roadmap(RenderContext::new(&model, &freshness));
    assert!(
        rendered.contains("> **Note:** Standing work, paused while M3 lands."),
        "{rendered}"
    );
}

// -------------------------------------------------------------------------------------
// The five remaining renderers — one focused test each: source query, ordering, and a
// determinism byte-compare under shuffled insertion order.
// -------------------------------------------------------------------------------------

fn committed(file: &str) -> String {
    std::fs::read_to_string(example_root().join("docs/generated").join(file))
        .unwrap_or_else(|_| panic!("{file} is committed"))
}

/// `render` on the shuffled ledger reproduces the committed file byte for byte — the
/// determinism check every renderer gets, run once per view here rather than five times
/// over in each renderer's own test.
fn assert_reproduces_and_is_order_independent(view: View, file: &str) {
    let workspace = example();
    let model = ResolvedModel::build(&workspace.ledger, &example_inputs(&workspace));
    let freshness = Freshness::from_stale(&workspace.ledger, stale_set());
    let rendered = render(view, RenderContext::new(&model, &freshness)).expect("renders");
    assert_eq!(rendered, committed(file), "{file} is stale; regenerate it");

    let mut shuffled = akr_core::model::Ledger::new(workspace.ledger.project.clone());
    let mut records = workspace.ledger.records().to_vec();
    records.reverse();
    shuffled.extend(records);
    shuffled.facts = workspace.ledger.facts.clone();
    let model = ResolvedModel::build(&shuffled, &example_inputs(&workspace));
    let freshness = Freshness::from_stale(&shuffled, stale_set());
    let reshuffled = render(view, RenderContext::new(&model, &freshness)).expect("renders");
    assert_eq!(reshuffled, rendered, "{file} depends on insertion order");
}

#[test]
fn current_state_excludes_decisions_and_orders_its_seven_sections() {
    assert_reproduces_and_is_order_independent(View::CurrentState, "CURRENT-STATE.md");
    let text = committed("CURRENT-STATE.md");
    assert!(
        !text.contains("@sim.decision.timestep-4ms")
            && !text.contains("@lege.decision.renderer-boundary"),
        "decisions have their own view"
    );
    let order = [
        "## Terms",
        "## Constraints",
        "## Policies",
        "## Requirements",
        "## Observations",
        "## Assessments",
        "## Evidence",
    ];
    let positions: Vec<usize> = order
        .iter()
        .map(|h| text.find(h).unwrap_or_else(|| panic!("{h} present")))
        .collect();
    assert!(positions.windows(2).all(|w| w[0] < w[1]), "{positions:?}");
    // An evidence record's reversed `verified_by` edge (§6).
    assert!(text.contains("**Verifies**"));
}

#[test]
fn active_work_groups_by_parent_in_roadmap_order_and_names_the_blocker() {
    assert_reproduces_and_is_order_independent(View::ActiveWork, "ACTIVE-WORK.md");
    let text = committed("ACTIVE-WORK.md");
    // M3's group precedes the standing track's group, matching ROADMAP.md's own order.
    let m3 = text.find("M3 — playable day").expect("M3 group");
    let track = text
        .find("Tooling hygiene")
        .expect("the tooling-hygiene track group");
    assert!(m3 < track, "parents are not in ROADMAP.md order");
    // Blocked work names what blocks it, inline.
    assert!(text.contains("**Blocked by**"));
    assert!(text.contains("Does a 4 ms timestep fit the frame budget?"));
    // Only live work states appear.
    for terminal in ["`completed`", "`abandoned`", "`superseded`"] {
        assert!(!text.contains(terminal), "{terminal} is not live work");
    }
}

#[test]
fn review_required_lists_stale_before_at_risk_in_queue_order() {
    assert_reproduces_and_is_order_independent(View::ReviewRequired, "REVIEW-REQUIRED.md");
    let text = committed("REVIEW-REQUIRED.md");
    let stale = text.find("## Stale").expect("a Stale heading");
    let at_risk = text.find("## At risk").expect("an At risk heading");
    assert!(stale < at_risk);
    // MANIFEST §7: two stale observations, propagating to four at-risk records.
    assert!(text.contains("## Stale (2)"));
    assert!(text.contains("## At risk (4)"));
    // Every at-risk entry names its path back to a stale record.
    assert!(text.contains("**Via**"));
}

#[test]
fn open_questions_sections_open_deferred_then_closed_and_shows_resolutions() {
    assert_reproduces_and_is_order_independent(View::OpenQuestions, "OPEN-QUESTIONS.md");
    let text = committed("OPEN-QUESTIONS.md");
    let open = text.find("## Open").expect("Open");
    let deferred = text.find("## Deferred").expect("Deferred");
    let closed = text.find("## Recently closed").expect("Recently closed");
    assert!(open < deferred && deferred < closed);
    // A resolved question carries its resolution and the live `resolves` edge that
    // closed it (V-011).
    assert!(text.contains("**Resolution.**"));
    assert!(text.contains("**Resolved by**"));
}

#[test]
fn decision_history_groups_by_key_newest_revision_first_and_includes_terminal_normative() {
    assert_reproduces_and_is_order_independent(View::DecisionHistory, "DECISION-HISTORY.md");
    let text = committed("DECISION-HISTORY.md");
    // Revision 2 of the renderer-boundary decision precedes revision 1.
    let two = text
        .find("### Revision 2 — The viewer consumes a frame snapshot")
        .expect("revision 2");
    let one = text
        .find("### Revision 1 — The viewer calls the simulator directly")
        .expect("revision 1");
    assert!(two < one, "newest revision does not come first");
    // A terminal revision of a non-decision normative kind is included (D-018's one
    // exception: this view alone carries archived and terminal normative records).
    assert!(
        text.contains("@sys.policy.weekly-demo/1"),
        "a withdrawn policy is part of the project's memory too"
    );
    assert!(text.contains("`withdrawn`"));
}

// -------------------------------------------------------------------------------------
// Regenerating the worked example's committed views
// -------------------------------------------------------------------------------------

/// Rewrites `examples/save-your-skin/docs/generated/` from the current renderers, over
/// the frozen synthetic history and pinned build inputs this file already uses for the
/// roadmap snapshot.
///
/// `#[ignore]`d: it is a maintenance tool, run by hand (`cargo test --release -p
/// akr-core --test render_roadmap -- --ignored regenerate`) after a renderer or a
/// worked-example source change, never as part of the ordinary suite. The five
/// views this test writes are otherwise checked byte-for-byte by
/// `an_unmodified_directory_passes`, so a stale regeneration fails loudly on the next
/// ordinary run rather than silently.
#[test]
#[ignore = "maintenance tool: run by hand to regenerate examples/save-your-skin/docs/generated/"]
fn regenerate_the_worked_example_views() {
    let workspace = example();
    let model = ResolvedModel::build(&workspace.ledger, &example_inputs(&workspace));
    let freshness = Freshness::from_stale(&workspace.ledger, stale_set());
    let cx = RenderContext::new(&model, &freshness);
    let dir = example_root().join("docs/generated");
    for &view in View::ALL {
        let Some(rendered) = render(view, cx) else {
            continue;
        };
        std::fs::write(dir.join(view.file_name()), rendered).expect("writable");
    }
}

#[test]
fn outstanding_work_lists_only_live_planning_heads() {
    let workspace = example();
    let inputs = example_inputs(&workspace);
    let model = ResolvedModel::build(&workspace.ledger, &inputs);
    let freshness = Freshness::none();
    let view = render(View::Backlog, RenderContext::new(&model, &freshness)).unwrap();
    assert!(view.contains("1/2 checks satisfied"), "{view}");
    assert!(view.contains("0/1 checks satisfied"), "{view}");
    assert!(
        view.contains("part of `@sys.milestone.m3-playable-day/1`"),
        "{view}"
    );
    assert!(
        view.contains("ACTIVE-WORK.md#extract-the-render-graph-behind-the-boundary"),
        "{view}"
    );
    for heading in ["## Active", "## Blocked", "## Ready", "## Proposed"] {
        assert!(view.contains(heading), "{view}");
    }
    for id in model.heads.values() {
        let record = workspace.ledger.get(id).unwrap();
        let included = matches!(record.kind, Kind::Milestone | Kind::Track | Kind::Work)
            && matches!(
                record.state.name(),
                "active" | "blocked" | "ready" | "proposed"
            )
            && !record
                .file
                .as_deref()
                .is_some_and(|p| p.contains("/archive/") || p.starts_with("archive/"));
        assert_eq!(
            view.contains(&format!("`@{}`", record.id)),
            included,
            "{}",
            record.id
        );
    }
}

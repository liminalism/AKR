//! P8's five exit criteria (`docs/13-implementation-roadmap.md` §3), through the binary.
//!
//! 1. A legacy document with no model available imports — one record per heading.
//! 2. Everything lands `proposed` with a `source { kind legacy }` block.
//! 3. `--lenient` changes exit status only; the warning list is identical without it.
//! 4. `akr complete` on the tracking record fails with `AKR-R022` while checks stand.
//! 5. Excerpts are byte-identical substrings of the source document.
//!
//! The extraction-level halves of 1 and 5 are in
//! `crates/akr-core/tests/import_extract.rs`; here the same properties are asserted on
//! what the write pipeline actually put on disk, because the formatter re-emits every
//! record and could paraphrase an excerpt without any extraction test noticing.

mod support;

use support::Example;

/// A corpus with one of everything: a document title, four durable-looking sections, a
/// status paragraph, a dead relative link and a live absolute one.
const CORPUS: &str = "\
# Old engine notes

## Determinism

The simulator must produce the same run from the same seed.

## Snapshot boundary

We decided to put a snapshot between the sim and the viewer.

M3 is about 60% done.

## Ambient occlusion

Ambient occlusion is ongoing; no milestone owns it. See [the plan](PLAN-v1.md).

## Nightly soak

A soak test runs every night on the build machine.
";

const DOCUMENT: &str = "docs/legacy/OLD-NOTES.md";

fn with_corpus(name: &str) -> Example {
    let example = Example::materialise(name);
    example.write_file(DOCUMENT, CORPUS);
    example
}

fn import(example: &Example, extra: &[&str]) -> support::Run {
    let mut args = vec!["--lenient", "import", DOCUMENT, "--namespace", "sys"];
    args.extend_from_slice(extra);
    example.run(&args)
}

// -------------------------------------------------------------------------------------
// exit criterion 1 — the deterministic floor, end to end
// -------------------------------------------------------------------------------------

#[test]
fn a_document_imports_with_one_record_per_heading() {
    let example = with_corpus("import-floor");
    let dry = import(&example, &["--dry-run"]);
    assert_eq!(dry.code, 0, "{}", dry.output());
    for line in [
        "4 durable claims",
        "would propose  sys.requirement.determinism",
        "would propose  sys.decision.snapshot-boundary",
        "would propose  sys.track.ambient-occlusion",
        "would propose  sys.policy.nightly-soak",
        "4 checks to @sys.work.old-notes-import",
        "nothing written (--dry-run)",
    ] {
        assert!(
            dry.stdout.contains(line),
            "missing {line:?}:\n{}",
            dry.stdout
        );
    }

    let before = example.sources();
    assert_eq!(before, example.sources(), "sources digest is stable");
    let real = import(&example, &[]);
    assert_eq!(real.code, 0, "{}", real.output());
    for line in [
        "created sys.requirement.determinism/1",
        "created sys.track.ambient-occlusion/1",
        "created sys.work.old-notes-import/1",
        "akr.lock is now stale",
    ] {
        assert!(
            real.stdout.contains(line),
            "missing {line:?}:\n{}",
            real.stdout
        );
    }
    assert_ne!(before, example.sources(), "the import wrote records");

    // The resulting ledger is valid: the strict check finds nothing to say.
    assert_eq!(example.run(&["build"]).code, 0);
    let check = example.run(&["check"]);
    assert_eq!(check.code, 0, "{}", check.output());
}

// -------------------------------------------------------------------------------------
// exit criteria 2 and 5 — proposed, provenanced, verbatim; asserted from disk
// -------------------------------------------------------------------------------------

#[test]
fn every_imported_record_is_proposed_with_legacy_provenance_and_verbatim_excerpt() {
    let example = with_corpus("import-provenance");
    assert_eq!(import(&example, &[]).code, 0);

    let staged =
        akr_core::ops::Staged::load(&example.root().join(".akr")).expect("the ledger loads");
    let imported: Vec<_> = staged
        .ledger
        .records()
        .iter()
        .filter(|r| {
            r.sources
                .iter()
                .any(|s| s.path.as_deref() == Some(DOCUMENT))
        })
        .collect();
    assert_eq!(imported.len(), 5, "four claims and the tracking record");

    for record in &imported {
        // AKR-M042: everything lands proposed. (No question in this corpus, so the
        // inquiry exception of docs/12 §3 does not soften the assertion.)
        assert_eq!(
            record.state,
            akr_core::model::State::Proposed,
            "{}",
            record.id
        );
        // AKR-M021: provenance is a legacy source block.
        let source = record
            .sources
            .iter()
            .find(|s| s.kind == akr_core::model::SourceKind::Legacy)
            .unwrap_or_else(|| panic!("{} has no legacy source", record.id));
        // Exit criterion 5, on what was actually written: the excerpt survived the
        // model round trip and the canonical formatter byte-identical.
        if let Some(excerpt) = &source.excerpt {
            assert!(
                CORPUS.contains(excerpt.as_str()),
                "{} paraphrased its excerpt: {excerpt:?}",
                record.id
            );
        }
    }
    // And the claims all carry one; only the tracking record's source block goes
    // without.
    let with_excerpts = imported
        .iter()
        .filter(|r| r.sources.iter().any(|s| s.excerpt.is_some()))
        .count();
    assert_eq!(with_excerpts, 4);
}

// -------------------------------------------------------------------------------------
// exit criterion 3 — --lenient changes exit status only
// -------------------------------------------------------------------------------------

#[test]
fn lenient_changes_the_exit_status_and_not_the_warning_list() {
    let example = with_corpus("import-lenient");
    let strict = example.run(&["import", DOCUMENT, "--namespace", "sys", "--dry-run"]);
    let lenient = example.run(&[
        "--lenient",
        "import",
        DOCUMENT,
        "--namespace",
        "sys",
        "--dry-run",
    ]);

    // The dead link is a warning either way; strict adds AKR-M041 and fails.
    assert_eq!(strict.code, 1, "{}", strict.output());
    assert_eq!(lenient.code, 0, "{}", lenient.output());
    let warnings = |run: &support::Run| -> Vec<String> {
        run.stdout
            .lines()
            .filter(|l| l.starts_with("warning["))
            .map(str::to_owned)
            .collect()
    };
    let strict_warnings = warnings(&strict);
    assert_eq!(strict_warnings, warnings(&lenient));
    assert!(
        strict_warnings.iter().any(|w| w.contains("AKR-M022")),
        "{strict_warnings:?}"
    );
    assert!(strict.stdout.contains("AKR-M041"), "{}", strict.stdout);
    assert!(!lenient.stdout.contains("AKR-M041"), "{}", lenient.stdout);

    // And without --dry-run, the strict invocation writes nothing at all.
    let before = example.sources();
    let refused = example.run(&["import", DOCUMENT, "--namespace", "sys"]);
    assert_eq!(refused.code, 1, "{}", refused.output());
    assert!(refused.stdout.contains("nothing written (AKR-M041)"));
    assert_eq!(
        before,
        example.sources(),
        "a refused import wrote something"
    );
}

// -------------------------------------------------------------------------------------
// exit criterion 4 — the tracking record cannot be closed early
// -------------------------------------------------------------------------------------

#[test]
fn completing_the_tracking_record_fails_while_any_check_is_unsatisfied() {
    let example = with_corpus("import-tracking");
    assert_eq!(import(&example, &[]).code, 0);

    let refused = example.run(&["complete", "sys.work.old-notes-import"]);
    assert_ne!(refused.code, 0, "{}", refused.output());
    assert!(
        refused.output().contains("AKR-R022"),
        "{}",
        refused.output()
    );
    assert!(
        refused.output().contains("determinism-claim"),
        "the refusal names the checks:\n{}",
        refused.output()
    );
}

// -------------------------------------------------------------------------------------
// the AKR-M faults
// -------------------------------------------------------------------------------------

#[test]
fn the_document_faults_are_diagnostics_not_environment_failures() {
    let example = Example::materialise("import-faults");

    let missing = example.run(&["import", "docs/legacy/ABSENT.md"]);
    assert_eq!(missing.code, 1, "{}", missing.output());
    assert!(
        missing.output().contains("AKR-M001"),
        "{}",
        missing.output()
    );

    example.write_file("docs/legacy/SLIDES.pdf", "%PDF-1.4\n");
    let format = example.run(&["import", "docs/legacy/SLIDES.pdf"]);
    assert_eq!(format.code, 1, "{}", format.output());
    assert!(format.output().contains("AKR-M002"), "{}", format.output());
}

#[test]
fn an_undeclared_namespace_is_m013_and_the_default_comes_from_the_path() {
    let example = with_corpus("import-namespace");
    // The document's first path segment is `docs`, which save-your-skin does not
    // declare — so the default fails, and says how to fix it.
    let defaulted = example.run(&["--lenient", "import", DOCUMENT]);
    assert_eq!(defaulted.code, 1, "{}", defaulted.output());
    assert!(
        defaulted.output().contains("AKR-M013"),
        "{}",
        defaulted.output()
    );
    assert!(
        defaulted.output().contains("--namespace"),
        "{}",
        defaulted.output()
    );

    let named = example.run(&["--lenient", "import", DOCUMENT, "--namespace", "nope"]);
    assert!(named.output().contains("AKR-M013"), "{}", named.output());
}

#[test]
fn a_colliding_key_is_m012_and_writes_nothing() {
    let example = with_corpus("import-collision");
    assert_eq!(import(&example, &[]).code, 0);
    let before = example.sources();
    let again = import(&example, &[]);
    assert_eq!(again.code, 1, "{}", again.output());
    assert!(again.output().contains("AKR-M012"), "{}", again.output());
    assert!(again.output().contains("akr revise"), "{}", again.output());
    assert_eq!(
        before,
        example.sources(),
        "a refused import wrote something"
    );
}

#[test]
fn a_document_with_nothing_durable_is_m011_and_writes_nothing() {
    let example = Example::materialise("import-empty");
    example.write_file("docs/legacy/EMPTY.md", "\n\n");
    let before = example.sources();
    let run = example.run(&[
        "--lenient",
        "import",
        "docs/legacy/EMPTY.md",
        "--namespace",
        "sys",
    ]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert!(run.output().contains("AKR-M011"), "{}", run.output());
    assert!(run.stdout.contains("nothing written"), "{}", run.stdout);
    assert_eq!(before, example.sources());
}

// -------------------------------------------------------------------------------------
// the audit, through `akr check`
// -------------------------------------------------------------------------------------

#[test]
fn deleting_the_document_after_import_turns_up_m022_on_check() {
    let example = with_corpus("import-audit");
    assert_eq!(import(&example, &[]).code, 0);
    assert_eq!(example.run(&["build"]).code, 0);
    assert_eq!(example.run(&["check"]).code, 0);

    std::fs::remove_file(example.root().join(DOCUMENT)).expect("the document goes");
    let strict = example.run(&["check"]);
    assert!(strict.output().contains("AKR-M022"), "{}", strict.output());
    assert_eq!(strict.code, 1, "strict makes the warning fatal (D-013)");
    let lenient = example.run(&["--lenient", "check"]);
    assert!(
        lenient.output().contains("AKR-M022"),
        "{}",
        lenient.output()
    );
    assert_eq!(lenient.code, 0, "{}", lenient.output());
}

#[test]
fn import_works_when_run_from_a_non_workspace_directory() {
    let example = with_corpus("import-out-of-tree");
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_akr"));
    let output = command
        .args([
            "--lenient",
            "--dir",
            example.root().to_string_lossy().as_ref(),
        ])
        .args(["import", "docs/legacy/OLD-NOTES.md", "--namespace", "sys"])
        .current_dir(std::env::temp_dir())
        .output()
        .expect("the akr binary runs");
    let status = output.status.code().unwrap_or(-1);
    let out = String::from_utf8_lossy(&output.stdout);
    let err = String::from_utf8_lossy(&output.stderr);
    assert_eq!(status, 0, "{out}{err}");
    assert!(
        out.contains("created sys.requirement.determinism/1"),
        "{out}"
    );
}

// -------------------------------------------------------------------------------------
// the other example
// -------------------------------------------------------------------------------------

#[test]
fn the_sys_tandem_legacy_roadmap_imports() {
    // The real pre-AKR document the sys-tandem example was distilled from, imported
    // under its own namespace. This is the command meeting the artefact P8 exists for.
    let example = Example::of(&support::SYS_TANDEM, "import-tandem");
    let source = std::fs::read_dir(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/sys-tandem/legacy"),
    )
    .expect("the legacy directory exists")
    .next()
    .expect("a legacy document")
    .expect("readable")
    .path();
    let text = std::fs::read_to_string(&source).expect("the document reads");
    example.write_file("legacy/roadmap.md", &text);

    let dry = example.run(&[
        "--lenient",
        "import",
        "legacy/roadmap.md",
        "--namespace",
        "tandem",
        "--dry-run",
    ]);
    assert_eq!(dry.code, 0, "{}", dry.output());
    assert!(dry.stdout.contains("durable claims"), "{}", dry.stdout);
    assert!(!dry.stdout.contains("0 durable claims"), "{}", dry.stdout);

    let real = example.run(&[
        "--lenient",
        "import",
        "legacy/roadmap.md",
        "--namespace",
        "tandem",
    ]);
    assert_eq!(real.code, 0, "{}", real.output());
    assert_eq!(example.run(&["build"]).code, 0);
    assert_eq!(example.run(&["--lenient", "check"]).code, 0);
}

// -------------------------------------------------------------------------------------
// A16 — ACTIVE-WORK hides import drafts nobody has dispositioned
// -------------------------------------------------------------------------------------

/// A report whose headings hit none of `classify`'s five keyword rules, so every one of
/// them falls through the catch-all to `work`. This is the real shape: the 2026-08 bulk
/// import that produced 125 live proposed work records turned headings like "Executive
/// verdict", "Wall times (median of 5 runs)" and "Peak RSS" into things the project
/// apparently intends to do.
const REPORT: &str = "\
# Decoder performance audit

## Wall times

The median of five runs, on the reference machine, at 4096x4096.

## Peak RSS

Resident set size at the high-water mark, sampled once a millisecond.
";

const REPORT_DOC: &str = "docs/legacy/AUDIT.md";

fn with_report(name: &str) -> Example {
    let example = Example::materialise(name);
    example.write_file(REPORT_DOC, REPORT);
    example
}

fn import_report(example: &Example) -> support::Run {
    example.run(&[
        "--lenient",
        "import",
        REPORT_DOC,
        "--namespace",
        "sys",
    ])
}

/// Until somebody dispositions a claim, its draft is not the project's plan and does not
/// belong in the view of what is being worked on.
#[test]
fn active_work_hides_an_undispositioned_import_draft() {
    let example = with_report("import-active-work-hidden");
    assert_eq!(import_report(&example).code, 0);
    assert_eq!(example.run(&["build"]).code, 0);

    let view = std::fs::read_to_string(example.root().join("docs/generated/ACTIVE-WORK.md"))
        .expect("ACTIVE-WORK.md");
    assert!(
        !view.contains("Wall times") && !view.contains("Peak RSS"),
        "undispositioned drafts must not appear in ACTIVE-WORK:\n{view}"
    );
    // The tracking record IS real work — "disposition every claim" — and stays visible.
    assert!(
        view.contains("audit-import"),
        "the tracking record itself is real work and must stay visible:\n{view}"
    );
}

/// THE REGRESSION THAT MATTERS, and the reason the obvious predicate was wrong.
///
/// Per D-015 only a *sealed* record needs a new revision, so a `proposed` record is edited
/// IN PLACE. A draft that has been read, rewritten and adopted as the project's live plan
/// is still revision 1, still `proposed`, and still cites only the document it came from —
/// so "proposed + only a legacy source + revision 1" would hide the single most useful
/// record in a ledger while leaving every phantom visible. That predicate was tested
/// against exactly such a record in a real workspace and would have buried it.
///
/// What separates the two is a fact the ledger already holds: whether the tracking check
/// is satisfied. Dispositioning a claim must bring its draft straight back into view.
#[test]
fn active_work_shows_a_draft_once_its_tracking_check_is_dispositioned() {
    let example = with_report("import-active-work-adopted");
    assert_eq!(import_report(&example).code, 0);
    example.git(&["add", "-A"]);
    example.git(&["commit", "--quiet", "-m", "import"]);

    let added = example.run(&[
        "evidence",
        "add",
        "sys.evidence.wall-times-dispositioned",
        "--result",
        "pass",
        "--method",
        "manual",
        "--summary",
        "The wall-times claim was read and adopted as the project's plan.",
    ]);
    assert_eq!(added.code, 0, "{}", added.output());

    // A `proposed` record is edited in place, which is the whole point: adoption leaves no
    // revision behind, only the disposition.
    let tracker = example.root().join(".akr/records/sys/work.akr");
    let text = std::fs::read_to_string(&tracker).expect("tracking record file");
    // Anchored on THIS claim's key: the checks are canonically sorted, so patching the
    // first one would disposition a different claim and prove nothing.
    let anchor = "promoted as sys.work.wall-times or declined with evidence";
    let patched = text.replacen(
        &format!("{anchor}\n                \"\"\"\n            method manual"),
        &format!(
            "{anchor}\n                \"\"\"\n            method manual\n            \
             verified_by [ @sys.evidence.wall-times-dispositioned/1 ]"
        ),
        1,
    );
    assert_ne!(patched, text, "the tracking check must have been patched");
    std::fs::write(&tracker, patched).expect("write tracking record");

    assert_eq!(example.run(&["build"]).code, 0);
    let view = std::fs::read_to_string(example.root().join("docs/generated/ACTIVE-WORK.md"))
        .expect("ACTIVE-WORK.md");
    assert!(
        view.contains("Wall times"),
        "a dispositioned draft is the project's plan and must be visible again:\n{view}"
    );
}

// -------------------------------------------------------------------------------------
// auto-save: the imported document is registered as a source
// -------------------------------------------------------------------------------------

#[test]
fn import_saves_the_document_and_cites_the_saved_copy() {
    let example = with_corpus("import-autosave");
    let run = import(&example, &[]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert!(
        run.stdout.contains("registered source"),
        "the write reports the saved copy:\n{}",
        run.stdout
    );

    // The stored copy is word for word, tracked by content hash.
    let hash = akr_core::source::hash_bytes(CORPUS.as_bytes());
    let catalog = example.read_file("sources/catalog.json");
    assert!(
        catalog.contains(&hash),
        "the catalog tracks the imported bytes:\n{catalog}"
    );
    let external: Vec<_> = std::fs::read_dir(example.root().join("sources/external"))
        .expect("sources/external")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect();
    assert_eq!(external.len(), 1, "one saved copy: {external:?}");
    let stored = std::fs::read_to_string(&external[0]).expect("the stored copy reads");
    assert_eq!(stored, CORPUS, "the saved copy is word for word");

    // Every imported record cites both the legacy path and the saved copy.
    let staged =
        akr_core::ops::Staged::load(&example.root().join(".akr")).expect("the ledger loads");
    let imported: Vec<_> = staged
        .ledger
        .records()
        .iter()
        .filter(|record| {
            record
                .sources
                .iter()
                .any(|source| source.path.as_deref() == Some(DOCUMENT))
        })
        .collect();
    assert_eq!(imported.len(), 5, "four claims and the tracking record");
    for record in &imported {
        let source = record
            .sources
            .iter()
            .find(|source| source.kind == akr_core::model::SourceKind::Legacy)
            .unwrap_or_else(|| panic!("{} has no legacy source", record.id));
        let document = source
            .document
            .as_deref()
            .unwrap_or_else(|| panic!("{} cites the path but no saved copy", record.id));
        assert!(
            catalog.contains(document),
            "{} cites {document:?}, which is not registered:\n{catalog}",
            record.id
        );
    }

    // The citations resolve: the strict check finds nothing to say.
    assert_eq!(example.run(&["build"]).code, 0);
    let check = example.run(&["check"]);
    assert_eq!(check.code, 0, "{}", check.output());
}

#[test]
fn import_warns_m023_for_a_live_link_with_no_saved_copy() {
    let example = Example::materialise("import-m023");
    example.write_file("docs/legacy/PLAN-v1.md", "# The plan\n\nIt exists.\n");
    example.write_file(DOCUMENT, CORPUS);
    let dry = example.run(&[
        "--lenient",
        "import",
        DOCUMENT,
        "--namespace",
        "sys",
        "--dry-run",
    ]);
    assert_eq!(dry.code, 0, "{}", dry.output());
    assert!(
        dry.output().contains("AKR-M023"),
        "a live unsaved link warns:\n{}",
        dry.output()
    );
    assert!(
        dry.output().contains("akr source add"),
        "the warning says how to save it:\n{}",
        dry.output()
    );

    // Registering the linked file silences the warning.
    let added = example.run(&["source", "add", "docs/legacy/PLAN-v1.md"]);
    assert_eq!(added.code, 0, "{}", added.output());
    let quiet = example.run(&[
        "--lenient",
        "import",
        DOCUMENT,
        "--namespace",
        "sys",
        "--dry-run",
    ]);
    assert_eq!(quiet.code, 0, "{}", quiet.output());
    assert!(
        !quiet.output().contains("AKR-M023"),
        "a saved link stops warning:\n{}",
        quiet.output()
    );
}

#[test]
fn identical_bytes_reuse_the_saved_copy() {
    let example = with_corpus("import-reuse");
    let first = import(&example, &[]);
    assert_eq!(first.code, 0, "{}", first.output());
    assert!(
        first.stdout.contains("registered source"),
        "the first import saves:\n{}",
        first.stdout
    );

    // Identical bytes under a different name and namespace: new keys, same source.
    example.write_file("docs/legacy/OLD-NOTES-COPY.md", CORPUS);
    let second = example.run(&[
        "--lenient",
        "import",
        "docs/legacy/OLD-NOTES-COPY.md",
        "--namespace",
        "sim",
    ]);
    assert_eq!(second.code, 0, "{}", second.output());
    assert!(
        second.stdout.contains("reusing saved source"),
        "identical bytes reuse the copy:\n{}",
        second.stdout
    );

    let external: Vec<_> = std::fs::read_dir(example.root().join("sources/external"))
        .expect("sources/external")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect();
    assert_eq!(external.len(), 1, "no duplicate copy: {external:?}");
}

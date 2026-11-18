//! The deterministic floor of P8 (`docs/13-implementation-roadmap.md` §3) and the
//! migration audit.
//!
//! Exit criterion 1 — one record per heading with no model available — and exit
//! criterion 5 — every excerpt a byte-identical substring of the source — are proven
//! here, where the corpus and the extraction can be compared without a workspace in
//! between. `crates/akr-cli/tests/import.rs` proves the same properties survive the
//! write pipeline.

use akr_core::diagnostics::codes::migration;
use akr_core::import::{Format, audit, classify, extract, slug_of};
use akr_core::model::{CheckMethod, Kind, Ledger, Project, RecordBuilder, SourceKind, State};

const CORPUS: &str = "\
# The legacy roadmap

## Determinism

The simulator must produce the same run from the same seed.
No exceptions were listed.

## Snapshot boundary

We decided to put a snapshot between the sim and the viewer.

M3 is about 60% done.

## Lighting

Lighting is ongoing; no milestone owns it.

## Weekly demo

The team builds a demo every Friday. See [the plan](PLAN-v1.md) and
[the readme](https://example.invalid/readme).

## Who owns audio?

Ask Dana about the audio pipeline.
";

// -------------------------------------------------------------------------------------
// exit criterion 1 — the deterministic floor
// -------------------------------------------------------------------------------------

#[test]
fn one_claim_per_heading_and_nothing_else() {
    let extraction = extract(CORPUS, Format::Markdown);
    let titles: Vec<&str> = extraction.claims.iter().map(|c| c.title.as_str()).collect();
    assert_eq!(
        titles,
        [
            "Determinism",
            "Snapshot boundary",
            "Lighting",
            "Weekly demo",
            "Who owns audio?"
        ]
    );
    // The status paragraph under "Snapshot boundary" is read and skipped, and the
    // document's own title proposes nothing: a bodiless level-1 heading is a name for
    // the pile, not a claim in it.
    assert_eq!(extraction.paragraphs_skipped, 1);
}

#[test]
fn the_kinds_follow_the_documented_rules() {
    let extraction = extract(CORPUS, Format::Markdown);
    let kinds: Vec<Kind> = extraction.claims.iter().map(|c| c.kind).collect();
    assert_eq!(
        kinds,
        [
            Kind::Policy,   // "must": requirement is retired (D-046)
            Kind::Decision, // "we decided"
            Kind::Policy,   // "ongoing": track is retired (D-046)
            Kind::Policy,   // "every"
            Kind::Question, // the title ends in ?
        ]
    );
    // The docs/12 §2 table rows, verbatim.
    assert_eq!(
        classify(
            "x",
            "The simulator must produce the same run from the same seed."
        ),
        Kind::Policy
    );
    assert_eq!(
        classify(
            "x",
            "We decided to put a snapshot between the sim and the viewer."
        ),
        Kind::Decision
    );
    assert_eq!(
        classify("x", "Lighting is ongoing; no milestone owns it."),
        Kind::Policy
    );
    // The floor never fabricates the kinds whose required slots a document cannot fill.
    assert_eq!(classify("x", "TODO: fix the projection pass."), Kind::Work);
}

#[test]
fn plain_text_proposes_one_claim_per_paragraph() {
    let text = "The first standing rule.\nIt spans lines.\n\nThe second thing.\n";
    let extraction = extract(text, Format::PlainText);
    assert_eq!(extraction.claims.len(), 2);
    assert_eq!(extraction.claims[0].title, "The first standing rule.");
    assert_eq!(
        extraction.claims[0].excerpt,
        "The first standing rule.\nIt spans lines."
    );
}

// -------------------------------------------------------------------------------------
// exit criterion 5 — verbatim excerpts
// -------------------------------------------------------------------------------------

#[test]
fn every_excerpt_is_a_byte_identical_substring_of_the_source() {
    for format in [Format::Markdown, Format::PlainText] {
        let extraction = extract(CORPUS, format);
        assert!(!extraction.claims.is_empty());
        for claim in &extraction.claims {
            assert!(
                CORPUS.contains(&claim.excerpt),
                "{:?} paraphrased {:?}",
                format,
                claim.excerpt
            );
        }
    }
}

#[test]
fn a_multi_line_excerpt_keeps_its_internal_newlines() {
    let extraction = extract(CORPUS, Format::Markdown);
    let determinism = &extraction.claims[0];
    assert_eq!(
        determinism.excerpt,
        "The simulator must produce the same run from the same seed.\nNo exceptions were listed."
    );
}

// -------------------------------------------------------------------------------------
// keys, slugs, links
// -------------------------------------------------------------------------------------

#[test]
fn slugs_are_valid_segments_and_unique() {
    assert_eq!(slug_of("Snapshot boundary"), "snapshot-boundary");
    assert_eq!(slug_of("Who owns audio?"), "who-owns-audio");
    assert_eq!(slug_of("3D & the *engine*!"), "d-the-engine");
    assert_eq!(slug_of("!!!"), "");

    let doubled = "## Same\n\na\n\n## Same\n\nb\n";
    let extraction = extract(doubled, Format::Markdown);
    assert_eq!(extraction.claims[0].slug, "same");
    assert_eq!(extraction.claims[1].slug, "same-2");
}

#[test]
fn relative_links_are_reported_and_absolute_ones_are_not() {
    let extraction = extract(CORPUS, Format::Markdown);
    let targets: Vec<&str> = extraction.links.iter().map(|l| l.target.as_str()).collect();
    assert_eq!(targets, ["PLAN-v1.md"]);
    let link = &extraction.links[0];
    assert!(link.line > 0 && link.column > 0);
    // The reported position points at the target itself.
    let line = CORPUS.lines().nth(link.line - 1).expect("the line exists");
    assert!(line[link.column - 1..].starts_with("PLAN-v1.md"), "{line}");
}

#[test]
fn an_empty_document_extracts_nothing() {
    let extraction = extract("\n\n   \n", Format::Markdown);
    assert!(extraction.claims.is_empty());
    assert_eq!(extraction.paragraphs_skipped, 0);
}

// -------------------------------------------------------------------------------------
// the audit — AKR-M022, AKR-M031, AKR-M032
// -------------------------------------------------------------------------------------

fn ledger(records: Vec<akr_core::model::Record>) -> Ledger {
    let mut ledger = Ledger::new(Project::new("fx", &["fx"]));
    ledger.extend(records);
    ledger
}

fn imported(key: &str, kind: Kind, path: &str) -> akr_core::model::Record {
    RecordBuilder::new(key, 1, kind)
        .filled()
        .source(SourceKind::Legacy, Some(path))
        .build()
}

fn tracker(key: &str, path: &str, state: State) -> akr_core::model::Record {
    RecordBuilder::new(key, 1, Kind::Work)
        .filled()
        .state(state)
        .source(SourceKind::Legacy, Some(path))
        .check("a-claim", CheckMethod::Manual, &[])
        .build()
}

#[test]
fn a_missing_document_is_m022_and_a_present_one_is_silent() {
    let ledger = ledger(vec![
        imported("fx.decision.a", Kind::Decision, "docs/legacy/GONE.md"),
        tracker(
            "fx.work.gone-import",
            "docs/legacy/GONE.md",
            State::Proposed,
        ),
    ]);
    let missing = audit(&ledger, "abcd1234", &|_| false);
    assert!(missing.iter().any(|d| d.code == migration::M022
        && d.severity == akr_core::diagnostics::Severity::Warning
        && d.message.contains("docs/legacy/GONE.md")
        && d.message.contains("abcd1234")));
    let present = audit(&ledger, "abcd1234", &|_| true);
    assert!(present.is_empty(), "{present:?}");
}

#[test]
fn bare_legacy_provenance_without_a_tracker_is_silent() {
    // The audit is anchored on tracking records, not on legacy provenance in general
    // (docs/12 §2; examples/sys-tandem/MANIFEST.md §8). Records that merely cite where
    // their knowledge came from — the blessed steady state of a mature ledger — are not
    // second-guessed. A missing tracker (AKR-M031) is guaranteed against at import time,
    // not re-derived here, because a mature ledger cannot tell an unfinished import from
    // a permanent citation.
    let ledger = ledger(vec![
        imported("fx.decision.a", Kind::Decision, "docs/legacy/LOOSE.md"),
        imported("fx.policy.b", Kind::Policy, "docs/legacy/LOOSE.md"),
    ]);
    // Silent whether the cited document is present or gone: with no migration under way,
    // there is nothing for the audit to say.
    assert!(audit(&ledger, "abcd1234", &|_| true).is_empty());
    assert!(audit(&ledger, "abcd1234", &|_| false).is_empty());
}

#[test]
fn a_work_record_without_acceptance_checks_is_not_a_tracker() {
    // The tracking record is a work record *with acceptance checks* (docs/12 §4). A bare
    // work record that merely cites the document is not one, so it does not turn the
    // document into an audited migration: even with the document gone, the audit stays
    // silent, exactly as it does for any other bare provenance citation.
    let ledger = ledger(vec![imported(
        "fx.work.a",
        Kind::Work,
        "docs/legacy/LOOSE.md",
    )]);
    assert!(audit(&ledger, "abcd1234", &|_| false).is_empty());
}

#[test]
fn archiving_before_completion_is_m032() {
    let path = "docs/legacy/archive/DONE.md";
    let unfinished = ledger(vec![
        imported("fx.decision.a", Kind::Decision, path),
        tracker("fx.work.done-import", path, State::Proposed),
    ]);
    let diagnostics = audit(&unfinished, "abcd1234", &|_| true);
    assert!(
        diagnostics.iter().any(|d| d.code == migration::M032
            && d.message.contains(path)
            && d.message.contains("proposed")),
        "{diagnostics:?}"
    );

    let finished = ledger(vec![
        imported("fx.decision.a", Kind::Decision, path),
        tracker("fx.work.done-import", path, State::Completed),
    ]);
    let diagnostics = audit(&finished, "abcd1234", &|_| true);
    assert!(
        !diagnostics.iter().any(|d| d.code == migration::M032),
        "{diagnostics:?}"
    );
}

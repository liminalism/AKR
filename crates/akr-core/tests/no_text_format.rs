//! Exit criterion 4: the crate compiles with no parser and no text-format dependency.
//!
//! P1 is semantics-first by design (`docs/13` §1): a parser written first tends to
//! define the model by accident. This test makes that a property of the build rather
//! than a promise in a document — if someone adds a runtime dependency to reach for a
//! serialisation library, it fails here.

/// The dependency list, kept to exactly what `docs/13` §4 sanctions.
///
/// One entry, added by P7. Stage E materialises the resolved model into a SQLite cache
/// with an FTS5 index, and neither a SQL engine nor a BM25 ranker is a reasonable thing to
/// own — which is why §4 named SQLite in the short list of permitted dependencies while
/// the SHA-256, JSON, glob and argument-parsing entries beside it were all written in
/// house instead.
///
/// The allowance is a list rather than a hole: a second entry fails this test, and the
/// conversation about whether it belongs happens here rather than in review.
const PERMITTED: &[&str] = &["rusqlite"];

#[test]
fn the_crate_has_no_runtime_dependencies_beyond_the_sanctioned_one() {
    let manifest = include_str!("../Cargo.toml");
    let after = manifest
        .split_once("\n[dependencies]")
        .expect("a [dependencies] section")
        .1;
    let body = after.split("\n[").next().expect("a section body");
    let entries: Vec<&str> = body
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter(|line| {
            let name = line.split(['=', ' ']).next().unwrap_or(line);
            !PERMITTED.contains(&name)
        })
        .collect();
    assert!(
        entries.is_empty(),
        "akr-core takes only the dependencies docs/13 §4 sanctions, found {entries:?}"
    );
}

#[test]
fn a_ledger_can_be_built_and_validated_with_no_text_anywhere() {
    use akr_core::model::{Kind, Ledger, Project, RecordBuilder, Relation, State};
    use akr_core::validate;

    let mut ledger = Ledger::new(Project::new("demo", &["demo"]));
    ledger.insert(
        RecordBuilder::new("demo.req.reproducible", 1, Kind::Requirement)
            .filled()
            .build(),
    );
    ledger.insert(
        RecordBuilder::new("demo.decision.fixed-step", 1, Kind::Decision)
            .state(State::Active)
            .rel(Relation::Implements, "@demo.req.reproducible")
            .filled()
            .build(),
    );
    assert!(validate::validate_all(&ledger).is_empty());
}

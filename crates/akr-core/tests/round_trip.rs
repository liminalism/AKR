//! Exit criterion 1 and the round-trip invariants of `docs/03` §7.

use akr_core::diagnostics::FileId;
use akr_core::syntax::{format, format_source, parse};

fn repo(path: &str) -> String {
    let full = format!("{}/../../{path}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("{path}: {e}"))
}

#[test]
fn the_exemplar_round_trips_byte_identically() {
    let source = repo("spec/exemplar.akr");
    let parsed = parse(&source, FileId(0));
    assert!(
        parsed.diagnostics.is_empty(),
        "exemplar must parse clean, got {:?}",
        parsed
            .diagnostics
            .iter()
            .map(|d| (d.code.as_str(), &d.message))
            .collect::<Vec<_>>()
    );
    let formatted = format(parsed.file.as_ref().expect("exemplar parses"));
    if formatted != source {
        for (n, (a, b)) in source.lines().zip(formatted.lines()).enumerate() {
            assert_eq!(a, b, "line {} differs", n + 1);
        }
        assert_eq!(
            source.len(),
            formatted.len(),
            "length differs after equal lines"
        );
    }
    assert_eq!(formatted, source, "spec/exemplar.akr must be canonical");
}

#[test]
fn formatting_is_idempotent_on_the_exemplar() {
    let source = repo("spec/exemplar.akr");
    let (once, _) = format_source(&source, FileId(0));
    let once = once.expect("formats");
    let (twice, _) = format_source(&once, FileId(0));
    assert_eq!(Some(once), twice);
}

#[test]
fn a_comment_above_a_top_level_block_is_not_duplicated_by_formatting() {
    // `Item::trivia()` returns a block's own trivia, and `emit_block` emits it itself
    // because a nested block never passes through `emit_item`. Emitting in both places
    // doubled the comment on every run — 1, 2, 4, 8 — so `akr fmt` was not idempotent for
    // the one construct every `akr init` writes (`defaults`), and any project.akr
    // documenting *why* a tree is declared untracked would have accumulated the reason.
    let source = "akr 0.1\nproject p\n\nnamespace p \"P.\"\n\n\
                  # Why this project keeps art out of git.\n\
                  untracked {\n    path \"art/**\"\n}\n\n\
                  # And a comment above defaults.\n\
                  defaults {\n    review_after_days 90\n}\n";
    let (once, _) = format_source(source, FileId(0));
    let once = once.expect("formats");
    let (twice, _) = format_source(&once, FileId(0));
    let twice = twice.expect("formats");
    assert_eq!(once, twice, "formatting is not idempotent");
    for comment in [
        "# Why this project keeps art out of git.",
        "# And a comment above defaults.",
    ] {
        assert_eq!(
            once.matches(comment).count(),
            1,
            "{comment:?} was duplicated by formatting:\n{once}"
        );
    }
}

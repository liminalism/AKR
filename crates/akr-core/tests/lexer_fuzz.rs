//! The §P2 fuzz posture: the lexer never panics and never loops forever.
//!
//! A seeded generator rather than a fuzzing dependency (`docs/13` §4). Two corpora: a
//! character soup drawn from the alphabet the lexer actually branches on, and every
//! prefix of every real fixture, which is where truncation bugs live.

use akr_core::diagnostics::FileId;
use akr_core::syntax::{lexer, parse};
use std::path::Path;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() as usize) % n
    }
}

/// Everything the lexer branches on, plus multi-byte characters and the delimiters most
/// likely to be left unbalanced.
const ALPHABET: &[&str] = &[
    "a", "z", "0", "9", "_", "-", ".", ":", "/", "@", "#", ",", "{", "}", "[", "]", " ", "\t",
    "\n", "\r", "\"", "\"\"\"", "\\", "\\u{", "git:", "record", "akr", "project", "true", "all",
    "ref", "path", "état", "知識", "🙂", "\u{feff}",
];

fn soup(rng: &mut Rng, length: usize) -> String {
    (0..length)
        .map(|_| ALPHABET[rng.below(ALPHABET.len())])
        .collect()
}

/// The lexer must consume every byte and emit at most one token per byte plus EOF.
/// A lexer that failed to advance would violate the bound before it could hang.
fn assert_bounded(text: &str) {
    let lexed = lexer::lex(text, FileId(0));
    assert!(
        lexed.tokens.len() <= text.len() + 1,
        "token count {} exceeds the input length {} — a branch is not consuming",
        lexed.tokens.len(),
        text.len()
    );
    assert!(matches!(lexed.tokens.last(), Some(t) if t.kind == lexer::TokenKind::Eof));
    let again = lexer::lex(text, FileId(0));
    assert_eq!(
        lexed.tokens.len(),
        again.tokens.len(),
        "lexing is not deterministic"
    );
}

#[test]
fn the_lexer_survives_character_soup() {
    run_character_soup(200);
}

#[test]
#[ignore = "exhaustive 4000-iteration fuzz: run manually or in nightly"]
fn the_lexer_survives_character_soup_exhaustive() {
    run_character_soup(4_000);
}

fn run_character_soup(iterations: usize) {
    let mut rng = Rng(0xfeed_face);
    for _ in 0..iterations {
        let length = rng.below(120);
        let text = soup(&mut rng, length);
        assert_bounded(&text);
        // Parsing must terminate too, and may report anything it likes.
        let _ = parse(&text, FileId(0));
    }
}

fn fixture_corpus() -> Vec<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut corpus: Vec<String> = Vec::new();
    for dir in [
        "fixtures/parse/ok",
        "fixtures/parse/err",
        "fixtures/format",
        "spec",
    ] {
        let Ok(entries) = std::fs::read_dir(root.join(dir)) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "akr")
                && let Ok(text) = std::fs::read_to_string(&path)
            {
                corpus.push(text);
            }
        }
    }
    assert!(
        corpus.len() >= 20,
        "expected a corpus, found {}",
        corpus.len()
    );
    corpus
}

#[test]
fn the_lexer_survives_every_prefix_of_every_fixture() {
    for text in &fixture_corpus() {
        let step = (text.len() / 16).max(1);
        for (idx, _) in text.char_indices().step_by(step) {
            assert_bounded(&text[..idx]);
        }
        for cut in text.len().saturating_sub(4)..=text.len() {
            if text.is_char_boundary(cut) {
                assert_bounded(&text[..cut]);
            }
        }
        let _ = parse(text, FileId(0));
    }
}

#[test]
#[ignore = "exhaustive prefix fuzz over every byte boundary of every fixture"]
fn the_lexer_survives_every_prefix_of_every_fixture_exhaustive() {
    for text in &fixture_corpus() {
        for cut in 0..text.len() {
            if text.is_char_boundary(cut) {
                assert_bounded(&text[..cut]);
            }
        }
        let _ = parse(text, FileId(0));
    }
}

#[test]
fn the_lexer_survives_pathological_repetition() {
    for text in [
        "\"".repeat(500),
        "\"\"\"".repeat(200),
        "{".repeat(500),
        "[".repeat(500),
        "@".repeat(500),
        "#".repeat(500),
        "\\".repeat(500),
        "git:".repeat(200),
        "\u{feff}".repeat(100),
        format!(
            "akr 0.1\nproject x\n{}",
            "record a.b/1 : term {".repeat(100)
        ),
    ] {
        assert_bounded(&text);
        let _ = parse(&text, FileId(0));
    }
}

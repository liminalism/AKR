//! Exit criterion 1 of `docs/13-implementation-roadmap.md` P6: every transcript under
//! `examples/save-your-skin/transcripts/` is reproduced by the real binary.
//!
//! A transcript is a shell session. Lines beginning with `$ ` are commands; everything
//! between one command and the next non-output line is that command's expected output;
//! lines beginning with `#` are commentary. This file parses that structure, runs every
//! `akr` command in a materialised copy of the worked example, and compares.
//!
//! # What is normalised, and why that is not cheating
//!
//! Exactly two classes of value are translated before comparison, both of them consequences
//! of the example's history being fictional:
//!
//! - the five commit hashes of `MANIFEST.md` §4, which a real repository cannot reproduce;
//! - the source-graph hash, which is a hash of record bytes that cite those commits.
//!
//! Nothing else is touched. Every word, every column, every count and every exit status is
//! compared byte for byte.
//!
//! # Blessing
//!
//! `AKR_BLESS=1 cargo test -p akr-cli --test transcripts` rewrites the expected blocks from
//! the current binary. The transcripts are the specification's illustration and the binary
//! is its implementation; when they disagree, whichever moves does so deliberately and the
//! diff is reviewed. Blessing is how the move is applied, not how it is decided.

mod support;

use std::path::Path;
use support::{Example, transcript_dir};

/// Commands a transcript names that this phase cannot run.
///
/// The write surface arrives with P6c and the index with P7; a transcript that shows them
/// is checked for structure but not executed. Listing them here rather than skipping
/// silently means the list shrinks visibly as the phases land.
const DEFERRED: &[&str] = &[
    "propose",
    "revise",
    "supersede",
    "complete",
    "abandon",
    "evidence",
    "search",
    "import",
];

/// One `$ command` and the output the transcript expects from it.
#[derive(Debug)]
struct Block {
    /// The `akr` argument list, without the program name.
    args: Vec<String>,
    /// The expected output, exactly as the transcript carries it.
    expected: String,
    /// The line the command sits on, for failure messages.
    line: usize,
    /// The expected exit status, when a `$ echo $?` follows.
    exit: Option<i32>,
    /// Byte offsets of the expected block within the transcript, for blessing.
    span: (usize, usize),
}

fn parse(text: &str) -> (Vec<Block>, Vec<(usize, String)>) {
    let mut blocks = Vec::new();
    let mut directives: Vec<(usize, String)> = Vec::new();
    let mut offset = 0usize;
    let lines: Vec<(usize, &str)> = text
        .lines()
        .map(|line| {
            let start = offset;
            offset += line.len() + 1;
            (start, line)
        })
        .collect();

    let mut index = 0;
    while index < lines.len() {
        let (_, line) = lines[index];
        let Some(command) = line.strip_prefix("$ ") else {
            index += 1;
            continue;
        };
        index += 1;
        let Some(args) = akr_args(command) else {
            directives.push((blocks.len(), command.to_owned()));
            continue;
        };

        // The blank line a transcript puts between a command and its output is shell
        // presentation, not output; so is the blank line before the next prompt.
        while index < lines.len() && lines[index].1.trim().is_empty() {
            index += 1;
        }
        let start = index;
        while index < lines.len()
            && !lines[index].1.starts_with("$ ")
            && !lines[index].1.starts_with('#')
        {
            index += 1;
        }
        let mut end = index;
        while end > start && lines[end - 1].1.trim().is_empty() {
            end -= 1;
        }

        let expected: String = lines[start..end]
            .iter()
            .map(|(_, line)| format!("{line}\n"))
            .collect();
        let span = (
            lines.get(start).map_or(text.len(), |(s, _)| *s),
            lines
                .get(end)
                .map_or(text.len(), |(s, _)| *s)
                .min(text.len()),
        );

        // `$ echo $?` immediately after a block states the expected status.
        let mut exit = None;
        let mut probe = index;
        while probe < lines.len() && lines[probe].1.trim().is_empty() {
            probe += 1;
        }
        if probe + 1 < lines.len() && lines[probe].1 == "$ echo $?" {
            exit = lines[probe + 1].1.trim().parse().ok();
        }

        blocks.push(Block {
            args,
            expected,
            line: lines[start.saturating_sub(1)].0,
            exit,
            span,
        });
    }
    (blocks, directives)
}

/// The argument list of an `akr` command line, or `None` for anything else.
fn akr_args(command: &str) -> Option<Vec<String>> {
    let mut words = shell_words(command);
    if words.first().map(String::as_str) != Some("akr") {
        return None;
    }
    words.remove(0);
    if words
        .first()
        .is_some_and(|w| DEFERRED.contains(&w.as_str()))
    {
        return None;
    }
    Some(words)
}

/// A minimal shell splitter: whitespace, single and double quotes, no expansion.
fn shell_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut any = false;
    for character in text.chars() {
        match (quote, character) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => current.push(c),
            (None, c @ ('"' | '\'')) => {
                quote = Some(c);
                any = true;
            }
            (None, c) if c.is_whitespace() => {
                if any || !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                    any = false;
                }
            }
            (None, c) => current.push(c),
        }
    }
    if any || !current.is_empty() {
        words.push(current);
    }
    words
}

fn transcripts() -> Vec<std::path::PathBuf> {
    let mut paths: Vec<_> = std::fs::read_dir(transcript_dir())
        .expect("the transcript directory exists")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.extension().is_some_and(|e| e == "txt"))
        .collect();
    paths.sort();
    paths
}

/// Replays one transcript, returning the rewritten text when blessing.
fn replay(example: &Example, path: &Path, bless: bool) -> Option<String> {
    let text = std::fs::read_to_string(path).expect("a readable transcript");
    let (blocks, directives) = parse(&text);
    assert!(
        !blocks.is_empty(),
        "{} names no runnable akr command",
        path.display()
    );

    let mut patches: Vec<((usize, usize), String)> = Vec::new();
    for (position, block) in blocks.iter().enumerate() {
        // Shell lines the transcript runs before this command — `sed` editing a view,
        // `git checkout` putting it back — are part of the scenario, not decoration.
        for (at, directive) in &directives {
            if *at == position {
                example.shell(directive);
            }
        }
        // Arguments quoting a manifest commit have to name the real one.
        let args: Vec<String> = block
            .args
            .iter()
            .map(|arg| example.denormalise(arg))
            .collect();
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        let run = example.run(&borrowed);
        let actual = example.normalise(&run.output());

        if bless {
            patches.push((block.span, actual.clone()));
            continue;
        }
        assert_eq!(
            actual,
            block.expected,
            "\n{}:{} `akr {}` does not reproduce the transcript",
            path.display(),
            block.line,
            block.args.join(" ")
        );
        if let Some(expected) = block.exit {
            assert_eq!(
                run.code,
                expected,
                "{}: `akr {}` exited {}, transcript says {expected}",
                path.display(),
                block.args.join(" "),
                run.code
            );
        }
    }

    if !bless {
        return None;
    }
    let mut out = text.clone();
    for ((start, end), replacement) in patches.into_iter().rev() {
        out.replace_range(start..end, &replacement);
    }
    Some(out)
}

#[test]
fn every_transcript_is_reproduced_by_the_binary() {
    let example = Example::materialise("transcripts");
    let bless = std::env::var("AKR_BLESS").is_ok();
    for path in transcripts() {
        if let Some(rewritten) = replay(&example, &path, bless) {
            std::fs::write(&path, rewritten).expect("bless");
        }
    }
    assert!(
        !bless,
        "AKR_BLESS rewrote the transcripts; rerun without it"
    );
}

// -------------------------------------------------------------------------------------
// Exit criterion 2 — a context bundle is byte-identical across runs
// -------------------------------------------------------------------------------------

#[test]
fn a_context_bundle_is_byte_identical_across_two_runs() {
    let example = Example::materialise("context-determinism");
    let args = [
        "context",
        "--goal",
        "sys.milestone.m3-playable-day",
        "--paths",
        "sim/src/project/**",
    ];
    let first = example.run(&args);
    let second = example.run(&args);
    assert_eq!(first.code, 0);
    assert_eq!(
        first.stdout, second.stdout,
        "two runs at the same commit must produce the same bundle, byte for byte"
    );
    assert!(first.stdout.starts_with("AKR CONTEXT BUNDLE\n"));
}

#[test]
fn the_json_bundle_is_byte_identical_across_two_runs() {
    let example = Example::materialise("context-json-determinism");
    let args = [
        "--format",
        "json",
        "context",
        "--goal",
        "sys.milestone.m3-playable-day",
    ];
    let first = example.run(&args);
    let second = example.run(&args);
    assert_eq!(first.stdout, second.stdout);
    assert!(first.stdout.contains("\"command\": \"context\""));
}

// -------------------------------------------------------------------------------------
// Exit criterion 5 — one representative case for each exit status
// -------------------------------------------------------------------------------------

#[test]
fn exit_zero_is_a_clean_check() {
    let example = Example::materialise("exit-0");
    let run = example.run(&["check"]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert!(run.stdout.ends_with("no diagnostics\n"));
}

#[test]
fn exit_one_is_a_diagnostic() {
    let example = Example::materialise("exit-1");
    // The opt-in freshness gate: the ledger is valid, so this is the honest way to make a
    // clean example produce an error without corrupting it.
    let run = example.run(&["check", "--review-clean"]);
    assert_eq!(run.code, 1, "{}", run.output());
    assert!(run.output().contains("AKR-G041"));
}

#[test]
fn exit_two_is_a_usage_error() {
    let example = Example::materialise("exit-2");
    let run = example.run(&["chekc"]);
    assert_eq!(run.code, 2, "{}", run.output());
    assert!(run.stderr.contains("AKR-C001"), "{}", run.stderr);
    assert!(
        run.stderr.contains("check"),
        "an unknown command suggests the nearest one: {}",
        run.stderr
    );
}

#[test]
fn exit_two_is_a_usage_error_after_the_workspace_loads() {
    // A malformed invocation that only surfaces once the workspace is read — a bad
    // flag value — is still a usage error, not a broken checkout (§3:
    // `AKR-C001`–`AKR-C005` exit 2).
    let example = Example::materialise("exit-2-loaded");
    let run = example.run(&[
        "evidence",
        "add",
        "sys.evidence.not-a-real-result",
        "--result",
        "nope",
        "--method",
        "command",
    ]);
    assert_eq!(run.code, 2, "{}", run.output());
    assert!(run.stderr.contains("AKR-C004"), "{}", run.stderr);
}

#[test]
fn exit_three_is_an_unusable_workspace() {
    // No `.akr/` anywhere above the directory, which is an environment fault rather than a
    // ledger fault (`docs/07-cli.md` §3).
    let dir = std::env::temp_dir().join(format!("akr-p6-no-workspace-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp directory");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_akr"))
        .args(["--dir", dir.to_str().expect("utf-8"), "check"])
        .current_dir(&dir)
        .output()
        .expect("the akr binary runs");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(output.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("AKR-C011"), "{stderr}");
}

// -------------------------------------------------------------------------------------
// The second worked example, exercised through the binary
// -------------------------------------------------------------------------------------

#[test]
fn the_shell_splitter_handles_quoted_globs() {
    assert_eq!(
        shell_words("akr context --goal g --paths \"sim/src/project/**\""),
        vec![
            "akr",
            "context",
            "--goal",
            "g",
            "--paths",
            "sim/src/project/**"
        ]
    );
    assert_eq!(shell_words("akr check").len(), 2);
}

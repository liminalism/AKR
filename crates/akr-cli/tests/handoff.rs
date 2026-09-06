//! `akr handoff` (D-040, D-041): moving work between agents without moving the judgement.
//!
//! The load-bearing claims here are negative — what a packet does *not* leak, what a parent
//! cannot substitute, and what a child is not silently told. So most of these run the
//! binary and assert on absence.

mod support;

use support::Example;

/// The id on the first line of a `create`-shaped output.
fn id_of(stdout: &str) -> String {
    stdout
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().last())
        .expect("the command names what it made")
        .to_owned()
}

const REQUEST: &str = "Review this project and optimise for performance, both memory and CPU.";

fn open_session(example: &Example, extra: &[&str]) -> String {
    let mut args = vec!["handoff", "session", "begin", "--request", REQUEST];
    args.extend_from_slice(extra);
    let run = example.run(&args);
    assert_eq!(run.code, 0, "{}", run.output());
    id_of(&run.stdout)
}

fn cut(example: &Example, mode: &str, extra: &[&str]) -> String {
    let mut args = vec!["handoff", mode];
    args.extend_from_slice(extra);
    let run = example.run(&args);
    assert_eq!(run.code, 0, "{}", run.output());
    id_of(&run.stdout)
}

#[test]
fn the_project_capsule_answers_what_every_agent_otherwise_asks() {
    let example = Example::materialise("handoff-capsule");
    let run = example.run(&["handoff", "capsule"]);
    assert_eq!(run.code, 0, "{}", run.output());
    for expected in ["PROJECT", "layout", "namespaces", "generated"] {
        assert!(run.stdout.contains(expected), "{}", run.stdout);
    }

    // The id is a digest over the derived content, so it is stable while the project is.
    let again = example.run(&["handoff", "capsule", "--refresh"]);
    assert_eq!(
        id_of(&run.stdout),
        id_of(&again.stdout),
        "{}",
        again.output()
    );

    // Boundaries are the one field nothing derives, so a refresh must not drop them.
    let set = example.run(&["handoff", "capsule", "--boundary", "no new dependencies"]);
    assert!(set.stdout.contains("no new dependencies"), "{}", set.stdout);
    let refreshed = example.run(&["handoff", "capsule", "--refresh"]);
    assert!(
        refreshed.stdout.contains("no new dependencies"),
        "a refresh dropped the part a person wrote:\n{}",
        refreshed.stdout
    );
}

#[test]
fn a_packet_inherits_the_session_rather_than_copying_it() {
    let example = Example::materialise("handoff-inherit");
    let session = open_session(&example, &["--baseline", "peak RSS 412 MB"]);
    let packet = cut(
        &example,
        "scout",
        &["--role", "perf-scout", "--task", "Inspect the sim crate."],
    );

    let opened = example.run(&["handoff", "open", &packet]);
    assert_eq!(opened.code, 0, "{}", opened.output());
    assert!(opened.stdout.contains(&session), "{}", opened.stdout);
    assert!(
        opened.stdout.contains("peak RSS 412 MB"),
        "{}",
        opened.stdout
    );
    assert!(opened.stdout.contains("PROJECT"), "{}", opened.stdout);

    // Inheritance is by reference: correcting the capsule corrects every packet cut from
    // it, which a copied capsule could not do.
    let corrected = example.run(&[
        "handoff",
        "session",
        "begin",
        "--request",
        REQUEST,
        "--baseline",
        "peak RSS 388 MB after the fix",
    ]);
    assert_eq!(corrected.code, 0, "{}", corrected.output());
    let second = cut(&example, "scout", &["--task", "Inspect the tone crate."]);
    let opened = example.run(&["handoff", "open", &second]);
    assert!(
        opened.stdout.contains("peak RSS 388 MB after the fix"),
        "{}",
        opened.stdout
    );
}

#[test]
fn the_session_request_is_carried_verbatim_and_a_narrowing_stays_visible() {
    let example = Example::materialise("handoff-verbatim");
    // Newlines and quotes are what a shell mangles, which is why --request-file exists.
    let request = "Optimise for \"performance\".\n\nBoth memory and CPU.";
    example.write_file(".agent/request.txt", request);
    let begun = example.run(&[
        "handoff",
        "session",
        "begin",
        "--request-file",
        ".agent/request.txt",
    ]);
    assert_eq!(begun.code, 0, "{}", begun.output());

    let packet = cut(
        &example,
        "worker",
        &["--task", "Rewrite the chroma inner loop."],
    );
    let opened = example.run(&["handoff", "open", &packet]);
    assert!(
        opened.stdout.contains("Both memory and CPU."),
        "the request came back changed:\n{}",
        opened.stdout
    );
    // A parent may narrow. It may not replace: from inside a child, a narrowed task and a
    // narrow one are otherwise indistinguishable.
    assert!(
        opened.stdout.contains("ORIGINAL REQUEST"),
        "{}",
        opened.stdout
    );
    assert!(
        opened.stdout.contains("YOUR ASSIGNMENT"),
        "{}",
        opened.stdout
    );
    assert!(
        opened.stdout.contains("Rewrite the chroma inner loop."),
        "{}",
        opened.stdout
    );
}

#[test]
fn disclosure_follows_the_mode() {
    let example = Example::materialise("handoff-disclosure");
    open_session(&example, &[]);
    let notes = [
        "--note-hypothesis",
        "the cost is all in chroma",
        "--note-examined",
        "src/chroma.rs",
    ];

    // A worker is continuing the parent's job, so repeating its reading is pure waste.
    let mut args = vec!["--task", "Continue the chroma work."];
    args.extend_from_slice(&notes);
    let worker = cut(&example, "worker", &args);
    let opened = example.run(&["handoff", "open", &worker]);
    assert!(
        opened.stdout.contains("the cost is all in chroma"),
        "a worker must see the parent's notes without asking:\n{}",
        opened.stdout
    );

    // The other three were sent to look, not to confirm.
    for mode in ["scout", "reviewer", "advisor"] {
        let mut args = vec!["--task", "Look at the pipeline."];
        args.extend_from_slice(&notes);
        let packet = cut(&example, mode, &args);
        let opened = example.run(&["handoff", "open", &packet]);
        assert!(
            !opened.stdout.contains("the cost is all in chroma"),
            "{mode} leaked a hypothesis on an ordinary open:\n{}",
            opened.stdout
        );
        assert!(
            opened.stdout.contains("withheld"),
            "{mode} must say the notes exist:\n{}",
            opened.stdout
        );

        let revealed = example.run(&["handoff", "reveal", &packet]);
        assert_eq!(revealed.code, 0, "{}", revealed.output());
        assert!(
            revealed.stdout.contains("the cost is all in chroma"),
            "{mode}: {}",
            revealed.stdout
        );
    }
}

#[test]
fn a_reveal_is_recorded_so_independence_is_knowable() {
    let example = Example::materialise("handoff-reveal-recorded");
    open_session(&example, &[]);
    let packet = cut(
        &example,
        "advisor",
        &["--note-hypothesis", "it is the allocator"],
    );

    let listed = example.run(&["handoff", "list"]);
    assert!(
        !listed.stdout.contains("[revealed]"),
        "a fresh packet has not been revealed:\n{}",
        listed.stdout
    );

    let first = example.run(&["--format", "json", "handoff", "reveal", &packet]);
    assert!(
        first.stdout.contains("\"first_reveal\": true"),
        "{}",
        first.stdout
    );
    let again = example.run(&["--format", "json", "handoff", "reveal", &packet]);
    assert!(
        again.stdout.contains("\"first_reveal\": false"),
        "the second reveal is not the first:\n{}",
        again.stdout
    );
    assert!(
        example
            .run(&["handoff", "list"])
            .stdout
            .contains("[revealed]"),
        "{}",
        listed.stdout
    );

    // Once revealed, an ordinary open shows them: withholding twice would only make the
    // child call reveal again for something it has already read.
    let opened = example.run(&["handoff", "open", &packet]);
    assert!(opened.stdout.contains("allocator"), "{}", opened.stdout);
}

#[test]
fn an_advisor_packet_with_no_task_inherits_the_request_whole() {
    let example = Example::materialise("handoff-advisor-inherits");
    open_session(&example, &[]);
    let packet = cut(&example, "advisor", &[]);
    let expanded = example.run(&["handoff", "expand", &packet, "task"]);
    assert!(expanded.stdout.contains(REQUEST), "{}", expanded.stdout);
}

#[test]
fn the_scope_defaults_to_the_whole_project() {
    let example = Example::materialise("handoff-scope");
    open_session(&example, &[]);
    let packet = cut(
        &example,
        "scout",
        &["--task", "Look around.", "--note-examined", "src/chroma.rs"],
    );

    let expanded = example.run(&["handoff", "expand", &packet, "scope"]);
    assert!(
        expanded.stdout.contains("**"),
        "a scope drawn from what the parent examined hands over its blind spot:\n{}",
        expanded.stdout
    );
    assert!(!expanded.stdout.contains("chroma"), "{}", expanded.stdout);

    let narrowed = cut(
        &example,
        "scout",
        &[
            "--task",
            "Look here.",
            "--scope",
            "sim/**",
            "--scope",
            "tests/**",
        ],
    );
    let expanded = example.run(&["handoff", "expand", &narrowed, "scope"]);
    assert!(expanded.stdout.contains("sim/**"), "{}", expanded.stdout);
    assert!(expanded.stdout.contains("tests/**"), "{}", expanded.stdout);
}

#[test]
fn a_fresh_session_is_exact_and_an_edited_tree_drifts() {
    let example = Example::materialise("handoff-drift");
    open_session(&example, &[]);
    let packet = cut(&example, "worker", &["--task", "Do the thing."]);

    // Writing the handoff store must not make the packet drift against itself.
    let verified = example.run(&["handoff", "verify", &packet]);
    assert_eq!(verified.code, 0, "{}", verified.output());
    assert!(verified.stdout.contains("exact"), "{}", verified.output());

    example.write_file("sim/moved-after-the-packet.txt", "changed");
    let verified = example.run(&["--format", "json", "handoff", "verify", &packet]);
    assert!(
        verified
            .stdout
            .contains("\"workspace_status\": \"drifted\""),
        "{}",
        verified.stdout
    );
    assert!(
        verified.stdout.contains("sim/moved-after-the-packet.txt"),
        "drift must name what moved:\n{}",
        verified.stdout
    );
    // Drift is a fact about the working tree, not a ledger contradiction (D-024).
    assert_eq!(verified.code, 0, "{}", verified.output());
    assert!(
        example
            .run(&["handoff", "open", &packet])
            .stdout
            .contains("drifted"),
        "the child is told at open time rather than left to compare by hand"
    );
}

#[test]
fn coverage_aggregates_and_names_what_nobody_opened() {
    let example = Example::materialise("handoff-coverage");
    open_session(&example, &[]);
    let first = cut(
        &example,
        "scout",
        &[
            "--role",
            "a",
            "--task",
            "Scout the loop.",
            "--scope",
            "sim/**",
        ],
    );
    let second = cut(
        &example,
        "scout",
        &[
            "--role",
            "b",
            "--task",
            "Scout the store.",
            "--scope",
            "sim/**",
        ],
    );

    let filed = example.run(&[
        "handoff",
        "result",
        &first,
        "--finding",
        "The plane is materialised per channel.",
        "--evidence",
        "sim/src/chroma.rs:120",
        "--read",
        "sim/src/chroma.rs:1-470",
        "--searched",
        "collect::<Vec",
        "--tested",
        "chroma benchmark",
        "--not-examined",
        "sim/src/localtone.rs",
    ]);
    assert_eq!(filed.code, 0, "{}", filed.output());

    let coverage = example.run(&["handoff", "coverage"]);
    assert_eq!(coverage.code, 0, "{}", coverage.output());
    assert!(
        coverage.stdout.contains("sim/src/chroma.rs"),
        "{}",
        coverage.stdout
    );
    assert!(
        coverage.stdout.contains("collect::<Vec"),
        "{}",
        coverage.stdout
    );
    // The half that stops the next wave checking the same doorway.
    assert!(
        coverage.stdout.contains("nobody has examined"),
        "{}",
        coverage.stdout
    );
    assert!(
        coverage.stdout.contains("sim/src/localtone.rs"),
        "{}",
        coverage.stdout
    );
    assert!(
        coverage.stdout.contains(&second),
        "an unanswered packet must be listed as outstanding:\n{}",
        coverage.stdout
    );

    // A path a child read is not reported as untouched, whoever else declined to open it.
    let filed = example.run(&[
        "handoff",
        "result",
        &second,
        "--read",
        "sim/src/localtone.rs",
    ]);
    assert_eq!(filed.code, 0, "{}", filed.output());
    let coverage = example.run(&["handoff", "coverage"]);
    assert!(
        !coverage.stdout.contains("nobody has examined"),
        "a path somebody read is not untouched:\n{}",
        coverage.stdout
    );
}

#[test]
fn a_result_without_coverage_says_so() {
    let example = Example::materialise("handoff-coverage-missing");
    open_session(&example, &[]);
    let packet = cut(&example, "worker", &["--task", "Do the thing."]);
    let filed = example.run(&["handoff", "result", &packet, "--finding", "It works."]);
    assert_eq!(filed.code, 0, "{}", filed.output());
    assert!(
        filed.stdout.contains("no coverage recorded"),
        "the one field that makes the next delegation possible:\n{}",
        filed.stdout
    );
}

#[test]
fn start_sends_a_child_to_its_packet() {
    let example = Example::materialise("handoff-start-notice");
    let started = example.run(&["start", "optimise the day loop"]);
    assert_eq!(started.code, 0, "{}", started.output());
    assert!(
        !started.stdout.contains("handoff      session"),
        "nothing to say before a session exists:\n{}",
        started.stdout
    );

    let session = open_session(&example, &[]);
    let started = example.run(&["start", "optimise the day loop"]);
    assert!(started.stdout.contains(&session), "{}", started.stdout);
    assert!(
        started.stdout.contains("akr handoff open"),
        "{}",
        started.stdout
    );
}

#[test]
fn the_failures_are_told_apart() {
    let example = Example::materialise("handoff-errors");

    // No session: the questions that need one say which one is missing.
    let coverage = example.run(&["handoff", "coverage"]);
    assert_eq!(coverage.code, 3, "{}", coverage.output());
    assert!(
        coverage.stderr.contains("AKR-C044"),
        "{}",
        coverage.output()
    );

    open_session(&example, &[]);
    let packet = cut(&example, "scout", &["--task", "Look."]);

    let missing = example.run(&["handoff", "open", "sc-000000000000"]);
    assert_eq!(missing.code, 3, "{}", missing.output());
    assert!(missing.stderr.contains("AKR-C043"), "{}", missing.output());

    // A mistyped section or mode is a malformed invocation, not an unusable workspace.
    let section = example.run(&["handoff", "expand", &packet, "hypotheses"]);
    assert_eq!(section.code, 2, "{}", section.output());
    assert!(section.stderr.contains("AKR-C004"), "{}", section.output());

    let mode = example.run(&["handoff", "create", "--mode", "oracle", "--task", "x"]);
    assert_eq!(mode.code, 2, "{}", mode.output());
    assert!(mode.stderr.contains("AKR-C004"), "{}", mode.output());
}

#[test]
fn handoff_state_stays_outside_the_ledger() {
    let example = Example::materialise("handoff-not-knowledge");
    open_session(&example, &[]);
    let packet = cut(&example, "worker", &["--task", "Do the thing."]);
    assert!(
        example
            .root()
            .join(format!(".agent/handoffs/{packet}.json"))
            .is_file(),
        "packets live where D-041 says"
    );

    // Nothing about a handoff is knowledge, so the ledger is untouched by making one.
    let checked = example.run(&["check"]);
    assert_eq!(checked.code, 0, "{}", checked.output());
    let found = example.run(&["search", "handoff packet"]);
    assert!(
        !found.stdout.contains(&packet),
        "a packet must be invisible to search:\n{}",
        found.stdout
    );

    let gone = example.run(&["handoff", "discard", &packet]);
    assert_eq!(gone.code, 0, "{}", gone.output());
    assert!(gone.stdout.contains("discarded"), "{}", gone.stdout);
    // Discarding what is not there is not an error: a cleanup step that fails on an
    // already-clean workspace is a step nobody runs.
    let again = example.run(&["handoff", "discard", &packet]);
    assert_eq!(again.code, 0, "{}", again.output());
}

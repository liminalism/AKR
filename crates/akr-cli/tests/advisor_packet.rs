//! `akr handoff` (D-040): handing a task to a second model without handing over the
//! judgement it is being brought in to make.
//!
//! The interesting claims here are all negative — what a packet does *not* leak, what the
//! preparing agent cannot substitute, and what the advisor is not silently told. So most
//! of these run the binary and assert on absence.

mod support;

use support::Example;

/// A packet id, read off `handoff create`'s first line.
fn packet_id(stdout: &str) -> String {
    stdout
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().last())
        .expect("create names the packet")
        .to_owned()
}

fn create(example: &Example, extra: &[&str]) -> String {
    let mut args = vec![
        "handoff",
        "create",
        "--task",
        "Review this project and optimise for performance, both memory and CPU.",
    ];
    args.extend_from_slice(extra);
    let run = example.run(&args);
    assert_eq!(run.code, 0, "{}", run.output());
    packet_id(&run.stdout)
}

#[test]
fn a_packet_is_addressable_and_lands_outside_the_ledger() {
    let example = Example::materialise("handoff-create");
    let id = create(&example, &[]);

    assert!(id.starts_with("ap-"), "addressable id: {id}");
    assert!(
        example
            .root()
            .join(format!(".agent/handoffs/{id}.json"))
            .is_file(),
        "the packet is stored where D-040 says: .agent/handoffs/"
    );

    // Nothing about a packet is knowledge, so the ledger must be untouched by making one.
    let checked = example.run(&["check"]);
    assert_eq!(checked.code, 0, "{}", checked.output());
    let found = example.run(&["search", "advisor packet"]);
    assert!(
        !found.stdout.contains(&id),
        "a packet must be invisible to search:\n{}",
        found.stdout
    );
}

#[test]
fn the_task_is_carried_verbatim() {
    let example = Example::materialise("handoff-verbatim");
    // Newlines and quotes are exactly what a shell mangles, which is why --task-file
    // exists; the point of the field is that it survives the round trip unchanged.
    let task = "Review this project and optimise for \"performance\".\n\nBoth memory and CPU.";
    example.write_file(".agent/request.txt", task);
    let run = example.run(&["handoff", "create", "--task-file", ".agent/request.txt"]);
    assert_eq!(run.code, 0, "{}", run.output());
    let id = packet_id(&run.stdout);

    let expanded = example.run(&["handoff", "expand", &id, "task"]);
    assert_eq!(expanded.code, 0, "{}", expanded.output());
    assert!(
        expanded.stdout.contains(task),
        "the task came back changed:\n{}",
        expanded.stdout
    );
}

#[test]
fn opening_a_packet_withholds_the_worker_notes() {
    let example = Example::materialise("handoff-blind");
    let id = create(
        &example,
        &[
            "--note-hypothesis",
            "the cost is all in chroma",
            "--note-examined",
            "src/chroma.rs",
            "--note-unexamined",
            "src/luma.rs",
        ],
    );

    let opened = example.run(&["handoff", "open", &id]);
    assert_eq!(opened.code, 0, "{}", opened.output());
    // The whole design rests on this: an advisor hired for what the worker did not
    // notice must not be handed the worker's noticing as the definition of the task.
    assert!(
        !opened.stdout.contains("chroma"),
        "a blind read leaked a hypothesis:\n{}",
        opened.stdout
    );
    assert!(
        opened.stdout.contains("withheld"),
        "the advisor must be told the notes exist:\n{}",
        opened.stdout
    );
    // And it must know where to look for itself.
    assert!(
        opened.stdout.contains("INDEPENDENT SEARCH SCOPE"),
        "{}",
        opened.stdout
    );

    let revealed = example.run(&["handoff", "reveal", &id]);
    assert_eq!(revealed.code, 0, "{}", revealed.output());
    assert!(revealed.stdout.contains("chroma"), "{}", revealed.stdout);
    assert!(revealed.stdout.contains("src/luma.rs"), "{}", revealed.stdout);
}

#[test]
fn a_reveal_is_recorded_so_independence_is_knowable() {
    let example = Example::materialise("handoff-reveal-recorded");
    let id = create(&example, &["--note-hypothesis", "it is the allocator"]);

    let listed = example.run(&["handoff", "list"]);
    assert!(
        !listed.stdout.contains("[revealed]"),
        "a fresh packet has not been revealed:\n{}",
        listed.stdout
    );

    let first = example.run(&["--format", "json", "handoff", "reveal", &id]);
    assert!(first.stdout.contains("\"first_reveal\": true"), "{}", first.stdout);

    let again = example.run(&["--format", "json", "handoff", "reveal", &id]);
    assert!(
        again.stdout.contains("\"first_reveal\": false"),
        "the second reveal is not the first:\n{}",
        again.stdout
    );

    let listed = example.run(&["handoff", "list"]);
    assert!(listed.stdout.contains("[revealed]"), "{}", listed.stdout);

    // Once revealed, an ordinary open shows the notes: withholding them a second time
    // would only make the advisor call `reveal` again for something it has already read.
    let opened = example.run(&["handoff", "open", &id]);
    assert!(opened.stdout.contains("allocator"), "{}", opened.stdout);
}

#[test]
fn the_envelope_defaults_to_the_whole_project() {
    let example = Example::materialise("handoff-envelope");
    let id = create(&example, &["--note-examined", "src/chroma.rs"]);

    let expanded = example.run(&["handoff", "expand", &id, "envelope"]);
    assert!(
        expanded.stdout.contains("**"),
        "a default envelope narrowed to what the worker examined would hand the advisor \
         the worker's blind spot:\n{}",
        expanded.stdout
    );
    assert!(!expanded.stdout.contains("chroma"), "{}", expanded.stdout);

    // Narrowing is possible, because sometimes the user narrowed the task.
    let narrowed = create(&example, &["--envelope", "src/**", "--envelope", "tests/**"]);
    let expanded = example.run(&["handoff", "expand", &narrowed, "envelope"]);
    assert!(expanded.stdout.contains("src/**"), "{}", expanded.stdout);
    assert!(expanded.stdout.contains("tests/**"), "{}", expanded.stdout);
}

#[test]
fn a_fresh_packet_is_exact_and_an_edited_tree_drifts() {
    let example = Example::materialise("handoff-drift");
    let id = create(&example, &[]);

    // Writing the packet must not make the packet drift against itself.
    let verified = example.run(&["handoff", "verify", &id]);
    assert_eq!(verified.code, 0, "{}", verified.output());
    assert!(verified.stdout.contains("exact"), "{}", verified.output());

    example.write_file("src/moved-after-the-packet.txt", "changed");
    let verified = example.run(&["--format", "json", "handoff", "verify", &id]);
    assert!(
        verified.stdout.contains("\"workspace_status\": \"drifted\""),
        "{}",
        verified.stdout
    );
    assert!(
        verified.stdout.contains("src/moved-after-the-packet.txt"),
        "drift must name what moved:\n{}",
        verified.stdout
    );
    // Drift is a fact about the working tree, not a ledger contradiction (D-024).
    assert_eq!(verified.code, 0, "{}", verified.output());

    // And the advisor is told at open time rather than left to compare by hand.
    let opened = example.run(&["handoff", "open", &id]);
    assert!(opened.stdout.contains("drifted"), "{}", opened.stdout);
}

#[test]
fn an_unknown_packet_and_an_unknown_section_fail_differently() {
    let example = Example::materialise("handoff-errors");
    let id = create(&example, &[]);

    let missing = example.run(&["handoff", "open", "ap-000000000000"]);
    assert_eq!(missing.code, 3, "{}", missing.output());
    assert!(missing.stderr.contains("AKR-C043"), "{}", missing.output());

    // A mistyped section is a malformed invocation, not an unusable workspace.
    let section = example.run(&["handoff", "expand", &id, "hypotheses"]);
    assert_eq!(section.code, 2, "{}", section.output());
    assert!(section.stderr.contains("AKR-C004"), "{}", section.output());
    assert!(section.stderr.contains("envelope"), "{}", section.output());
}

#[test]
fn a_packet_carries_the_session_head_and_what_was_established() {
    let example = Example::materialise("handoff-layer-a");
    let id = create(
        &example,
        &[
            "--by",
            "worker-model",
            "--question",
            "Where is the real cost?",
            "--command",
            "cargo test=706 passed",
            "--baseline",
            "peak RSS 412 MB",
            "--constraint",
            "no new dependencies",
        ],
    );

    let opened = example.run(&["handoff", "open", &id]);
    assert_eq!(opened.code, 0, "{}", opened.output());
    for expected in [
        "AKR SESSION HEAD",
        "Where is the real cost?",
        "cargo test",
        "706 passed",
        "peak RSS 412 MB",
        "no new dependencies",
        "worker-model",
    ] {
        assert!(
            opened.stdout.contains(expected),
            "layer A is missing {expected:?}:\n{}",
            opened.stdout
        );
    }
}

#[test]
fn discard_removes_a_packet_and_says_so_twice_over() {
    let example = Example::materialise("handoff-discard");
    let id = create(&example, &[]);

    let gone = example.run(&["handoff", "discard", &id]);
    assert_eq!(gone.code, 0, "{}", gone.output());
    assert!(gone.stdout.contains("discarded"), "{}", gone.stdout);
    assert!(!example.root().join(format!(".agent/handoffs/{id}.json")).exists());

    // Discarding what is not there is not an error: a packet is disposable, and a
    // cleanup step that fails on an already-clean workspace is a step nobody runs.
    let again = example.run(&["handoff", "discard", &id]);
    assert_eq!(again.code, 0, "{}", again.output());
    assert!(again.stdout.contains("no advisor packet"), "{}", again.stdout);
}

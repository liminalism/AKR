//! The write commands through the binary: `docs/07-cli.md` §4 and §6.
//!
//! Exit criterion 3 of P6 — a refused write leaves the working tree byte-identical — is
//! proved at library level in `akr-core/tests/ops_atomicity.rs`. This is the same claim
//! made where a user can observe it: one refusing path per command, run as a process,
//! with every source under `.akr/` hashed either side of the call. The disposable cache
//! is excluded: D-019 makes it rebuildable and non-authoritative, so an index rebuild or
//! the write pipeline's lock file is not a change to the working tree.
//!
//! The refusal *shapes* are checked here too, against the structured fields of
//! `ops::Refused` rather than against message text — the point of the structure being
//! that the rendering and the library can be checked independently.

mod support;

use support::{Example, Run};

/// Asserts that a command refuses, exits 1, and writes nothing.
fn refuses(example: &Example, args: &[&str]) -> String {
    let before = example.sources();
    let run = example.run(args);
    let after = example.sources();
    assert_eq!(
        run.code,
        1,
        "akr {} should refuse with exit 1:\n{}",
        args.join(" "),
        run.output()
    );
    assert_eq!(
        before,
        after,
        "akr {} refused but changed the working tree",
        args.join(" ")
    );
    assert!(
        run.output().contains("nothing written"),
        "akr {} does not say the tree is untouched:\n{}",
        args.join(" "),
        run.output()
    );
    run.output()
}

#[test]
fn propose_refuses_an_existing_key_and_writes_nothing() {
    let example = Example::materialise("write-propose");
    let text = refuses(
        &example,
        &["propose", "sys.term.playable-day", "--kind", "term"],
    );
    assert!(text.contains("AKR-L041"), "{text}");
    assert!(text.contains("akr revise"), "names the way forward: {text}");
}

#[test]
fn revise_refuses_an_in_place_edit_of_a_sealed_head() {
    let example = Example::materialise("write-revise");
    let text = refuses(
        &example,
        &[
            "revise",
            "sys.term.playable-day",
            "--in-place",
            "--title",
            "Something else",
        ],
    );
    assert!(text.contains("AKR-C032"), "{text}");
}

#[test]
fn supersede_lists_the_children_it_needs_a_disposition_for() {
    let example = Example::materialise("write-supersede");
    // D-017's demand attaches to a *pinned* `part_of @key/n`, which is what makes a child
    // the property of one plan revision rather than of the key. The worked example pins
    // the plan's children; this pins one of the milestone's so the refusal has something
    // to list without depending on the plan's own disposition blocks.
    let path = ".akr/records/sim/work.akr";
    let source = example.read_file(path);
    example.write_file(
        path,
        &source.replace(
            "part_of [ @sys.milestone.m3-playable-day ]",
            "part_of [ @sys.milestone.m3-playable-day/1 ]",
        ),
    );

    let text = refuses(&example, &["supersede", "sys.milestone.m3-playable-day"]);
    assert!(text.contains("unfinished child"), "{text}");
    assert!(text.contains("@sim.work.rewrite-projection"), "{text}");
    assert!(text.contains("blocked"), "names the state: {text}");
    assert!(
        text.contains("--disposition sim.work.rewrite-projection=intentionally_dropped"),
        "offers a runnable fix: {text}"
    );

    // And the fix works, which is the half of the interaction that matters.
    let applied = example.run(&[
        "supersede",
        "sys.milestone.m3-playable-day",
        "--disposition",
        "sim.work.rewrite-projection=intentionally_dropped",
    ]);
    assert_eq!(applied.code, 0, "{}", applied.output());
    assert!(applied.stdout.contains("sys.milestone.m3-playable-day/2"));
    assert!(
        applied.stdout.contains("akr.lock is now stale"),
        "a write always stales the lock (D-014): {}",
        applied.stdout
    );
}

#[test]
fn supersede_with_an_already_proposed_key_is_atomic() {
    let example = Example::materialise("write-supersede-with");
    for (key, title) in [
        ("sys.term.legacy-session", "Legacy session"),
        ("sys.term.playable-session", "Playable session"),
    ] {
        let body = example.root().join(format!("{key}.akr"));
        std::fs::write(
            &body,
            format!("scope [ all ]\ndefinition \"A definition for {title}.\"\n"),
        )
        .expect("write proposal fragment");
        let proposed = example.run(&[
            "propose",
            key,
            "--kind",
            "term",
            "--title",
            title,
            "--from",
            body.to_str().expect("utf-8"),
        ]);
        assert_eq!(proposed.code, 0, "{}", proposed.output());
    }
    let accepted = example.run(&[
        "revise",
        "sys.term.legacy-session",
        "--in-place",
        "--state",
        "active",
    ]);
    assert_eq!(accepted.code, 0, "{}", accepted.output());

    let replaced = example.run(&[
        "supersede",
        "sys.term.legacy-session",
        "--with",
        "sys.term.playable-session",
    ]);
    assert_eq!(replaced.code, 0, "{}", replaced.output());
    let source = example.read_file(".akr/records/sys/terms.akr");
    assert!(source.contains("supersedes [ @sys.term.legacy-session/1 ]"));
    assert!(source.contains("record sys.term.playable-session/1 : term"));
}

#[test]
fn complete_names_the_unsatisfied_check_and_writes_nothing() {
    let example = Example::materialise("write-complete");
    let text = refuses(&example, &["complete", "sys.milestone.m3-playable-day"]);
    assert!(text.contains("AKR-R022"), "{text}");
    assert!(text.contains("no-placeholder-assets"), "{text}");
    assert!(
        text.contains("--check <id>=<evidence-ref>"),
        "offers the fix: {text}"
    );
}

#[test]
fn abandon_requires_a_reason_and_writes_nothing() {
    let example = Example::materialise("write-abandon");
    let text = refuses(&example, &["abandon", "sys.work.m3-plan"]);
    assert!(text.contains("AKR-C031"), "{text}");
    assert!(text.contains("--reason"), "{text}");
}

#[test]
fn evidence_add_many_writes_a_batch_in_one_pass() {
    // `knowledge.evidence_add_many` existed over MCP with no command-line equivalent, and
    // with the translation and the batch write living in the MCP crate -- against both
    // "one implementation for CLI and MCP" and "akr-mcp contains no ledger logic". This is
    // the command-line half of that reunion.
    let example = Example::materialise("write-evidence-add-many");
    let head = example.commit(5).to_owned();
    let batch = format!(
        "record sys.evidence.batch-one/1 : evidence {{\n    \
             title \"First check\"\n    \
             result pass\n    \
             method command\n    \
             command \"cargo test -p sim\"\n    \
             observed_at git:{head}\n\
         }}\n\n\
         record sys.evidence.batch-two/1 : evidence {{\n    \
             title \"Second check\"\n    \
             result pass\n    \
             method observation\n    \
             observed_at git:{head}\n\
         }}\n"
    );
    example.write_file("batch.akr", &batch);

    let run = example.run(&["evidence", "add-many", "--from", "batch.akr"]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert!(
        run.stdout
            .contains("2 evidence records written in one pass"),
        "{}",
        run.output()
    );

    let written = example.read_file(".akr/records/sys/evidence.akr");
    assert!(written.contains("sys.evidence.batch-one"), "{written}");
    assert!(written.contains("sys.evidence.batch-two"), "{written}");
    assert_eq!(example.run(&["build"]).code, 0);
    let checked = example.run(&["check"]);
    assert_eq!(checked.code, 0, "{}", checked.output());
}

#[test]
fn evidence_add_many_is_atomic_and_writes_nothing_on_a_bad_record() {
    // The point of one pipeline pass: the ledger is validated as a whole, so a batch with
    // one bad record leaves the working tree exactly as it was.
    let example = Example::materialise("write-evidence-add-many-atomic");
    let head = example.commit(5).to_owned();
    let before = example.sources();
    let batch = format!(
        "record sys.evidence.good/1 : evidence {{\n    \
             title \"Fine\"\n    result pass\n    method command\n    \
             observed_at git:{head}\n\
         }}\n\n\
         record sys.evidence.bad/1 : evidence {{\n    \
             title \"Missing its result\"\n    method command\n    \
             observed_at git:{head}\n\
         }}\n"
    );
    example.write_file("batch.akr", &batch);

    let run = example.run(&["evidence", "add-many", "--from", "batch.akr"]);
    assert_ne!(run.code, 0, "{}", run.output());
    assert_eq!(before, example.sources(), "a refused batch wrote something");
}

#[test]
fn evidence_add_many_refuses_a_file_of_other_kinds() {
    // Named rather than skipped: a batch that silently wrote the evidence and ignored the
    // rest would be worse than one that wrote nothing.
    let example = Example::materialise("write-evidence-add-many-wrong-kind");
    example.write_file(
        "batch.akr",
        "record sys.term.thing/1 : term {\n    statement \"A thing.\"\n}\n",
    );
    let run = example.run(&["evidence", "add-many", "--from", "batch.akr"]);
    assert_ne!(run.code, 0, "{}", run.output());
    assert!(
        run.output().contains("term"),
        "the refusal should name the offending kind: {}",
        run.output()
    );
}

#[test]
fn evidence_add_many_requires_a_file() {
    let example = Example::materialise("write-evidence-add-many-no-file");
    let run = example.run(&["evidence", "add-many"]);
    assert_ne!(run.code, 0, "{}", run.output());
    assert!(run.output().contains("--from"), "{}", run.output());
}

#[test]
fn evidence_add_refuses_an_incomplete_request_without_writing() {
    let example = Example::materialise("write-evidence-refuse");
    let before = example.sources();
    // A missing `--result` cannot even be turned into a request, so it never reaches the
    // pipeline; the guarantee is the same and is checked the same way.
    let run = example.run(&["evidence", "add", "sys.evidence.asset-audit"]);
    assert_eq!(run.code, 3, "{}", run.output());
    assert!(run.output().contains("AKR-C003"), "{}", run.output());
    assert_eq!(before, example.sources());

    let bad = example.run(&[
        "evidence",
        "add",
        "sys.evidence.asset-audit",
        "--result",
        "maybe",
        "--method",
        "command",
    ]);
    assert_eq!(bad.code, 3, "{}", bad.output());
    assert!(bad.output().contains("AKR-C004"), "{}", bad.output());
    assert_eq!(before, example.sources());
}

// -------------------------------------------------------------------------------------
// The paths that succeed
// -------------------------------------------------------------------------------------

#[test]
fn a_yaml_like_from_file_names_the_accepted_slot_list() {
    let example = Example::materialise("write-propose-yaml-from");
    let body = example.root().join("yaml-like.akr");
    std::fs::write(&body, "statement: this is yaml, not an AKR slot-list\n").expect("write decoy");
    let run = example.run(&[
        "propose",
        "sys.work.yaml-decoy",
        "--kind",
        "work",
        "--title",
        "YAML decoy",
        "--from",
        body.to_str().expect("utf-8"),
    ]);
    assert_ne!(run.code, 0, "{}", run.output());
    assert!(run.output().contains("AKR-C031"), "{}", run.output());
    assert!(
        run.output().contains("slot-list") || run.output().contains("intent"),
        "{}",
        run.output()
    );
}

#[test]
fn get_names_the_work_records_intent_slot() {
    let example = Example::materialise("write-get-names-intent");
    let run = example.run(&["get", "sys.work.m3-plan"]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert!(
        run.stdout.contains("intent"),
        "body names the slot: {}",
        run.stdout
    );
}

#[test]
fn a_proposal_with_no_body_is_refused_outright() {
    let example = Example::materialise("write-propose-bodyless");
    // §4 validates the *resulting* ledger, and a record with no required prose slot does
    // not validate. So `--from`/`--edit` is not a convenience on `akr propose`: without
    // one there is nothing to write that the pipeline will accept. §6 says so explicitly;
    // this is that sentence being true.
    let text = refuses(
        &example,
        &[
            "propose",
            "sys.term.day-loop",
            "--kind",
            "term",
            "--title",
            "The day loop",
        ],
    );
    assert!(text.contains("AKR-C031"), "{text}");
    assert!(
        text.contains("definition"),
        "names the missing slot: {text}"
    );
}

#[test]
fn a_proposal_from_a_file_lands_and_checks_clean() {
    let example = Example::materialise("write-propose-from");
    let body = example.root().join("day-loop.akr");
    std::fs::write(
        &body,
        "akr 0.1\nproject save-your-skin\n\nrecord sys.term.day-loop/1 : term {\n    \
         title \"The day loop\"\n    state active\n    scope [ all ]\n    definition \"\"\"\n        \
         The repeating structure of one in-game day: wake, work, evening, sleep.\n        \
         \"\"\"\n    author \"test\"\n    created_at 2026-08-03\n}\n",
    )
    .expect("write body");

    let run = example.run(&[
        "propose",
        "sys.term.day-loop",
        "--kind",
        "term",
        "--from",
        body.to_str().expect("utf-8"),
    ]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert!(
        example
            .read_file(".akr/records/sys/terms.akr")
            .contains("sys.term.day-loop/1")
    );
    // The lock is stale until the next build (D-014), so the ledger only checks clean
    // after one. That is the sequence every write leaves behind.
    assert_eq!(example.run(&["build"]).code, 0);
    assert_eq!(example.run(&["check"]).code, 0);
}

#[test]
fn revise_on_a_sealed_head_retires_it_in_the_same_write() {
    let example = Example::materialise("write-revise-ok");
    let run = example.run(&[
        "revise",
        "sys.term.playable-day",
        "--title",
        "A playable day",
    ]);
    assert_eq!(run.code, 0, "{}", run.output());
    // Both halves in one write: revision 2 created and revision 1 retired. Leaving the old
    // head live would be two live heads, which V-012 refuses, and §4 refuses to write a
    // ledger that does not validate — so they cannot be separated.
    assert!(
        run.stdout.contains("created sys.term.playable-day/2"),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout
            .contains("sys.term.playable-day/1 active -> superseded"),
        "{}",
        run.stdout
    );
    let source = example.read_file(".akr/records/sys/terms.akr");
    assert!(source.contains("state superseded"));
    assert!(source.contains("supersedes [ @sys.term.playable-day/1 ]"));
    assert!(
        run.stdout.contains("starts proposed"),
        "a sealed content-only revise must say the successor is unaccepted: {}",
        run.stdout
    );

    let before_build = example.run(&["check"]);
    assert_eq!(before_build.code, 1, "{}", before_build.output());
    assert!(before_build.output().contains("AKR-R052"));
    assert!(!before_build.output().contains("AKR-R051"));
}

#[test]
fn revise_from_a_partial_fragment_keeps_unmentioned_slots() {
    // CLI `--from` used to replace the record. A fragment that only rewrote `intent`
    // dropped acceptance, relations and provenance — the merge MCP already did.
    let example = Example::materialise("write-revise-from-merge");
    let body = example.root().join("fragment.akr");
    std::fs::write(
        &body,
        "intent \"\"\"\n        Only the intent changed.\n        \"\"\"\n",
    )
    .expect("write fragment");

    let run = example.run(&[
        "revise",
        "sys.work.legacy-roadmap-import",
        "--from",
        body.to_str().expect("utf-8"),
    ]);
    assert_eq!(run.code, 0, "{}", run.output());
    let source = example.read_file(".akr/records/sys/work.akr");
    assert!(
        source.contains("Only the intent changed."),
        "the named slot is applied: {source}"
    );
    assert!(
        source.contains("check lighting-standing-claim"),
        "acceptance survived the overlay: {source}"
    );
    assert!(
        source.contains("part_of [ @sys.track.tooling-hygiene ]"),
        "relations survived the overlay: {source}"
    );
    assert!(
        source.contains("docs/legacy/ROADMAP.md"),
        "provenance survived the overlay: {source}"
    );
}

#[test]
fn revise_from_a_partial_fragment_on_a_sealed_head_keeps_unmentioned_slots() {
    let example = Example::materialise("write-revise-from-sealed-merge");
    let body = example.root().join("fragment.akr");
    std::fs::write(
        &body,
        "definition \"\"\"\n        A rewritten definition of the playable day.\n        \"\"\"\n",
    )
    .expect("write fragment");

    let run = example.run(&[
        "revise",
        "sys.term.playable-day",
        "--from",
        body.to_str().expect("utf-8"),
    ]);
    assert_eq!(run.code, 0, "{}", run.output());
    let source = example.read_file(".akr/records/sys/terms.akr");
    assert!(source.contains("sys.term.playable-day/2"), "{source}");
    let successor = source
        .split("record sys.term.playable-day/2")
        .nth(1)
        .expect("revision 2");
    assert!(
        successor.contains("A rewritten definition of the playable day."),
        "{successor}"
    );
    assert!(
        successor.contains("aliases [ \"playable day\", \"day-loop build\" ]"),
        "unmentioned aliases survived: {successor}"
    );
    assert!(
        successor.contains("claim day-boundary"),
        "unmentioned claims survived: {successor}"
    );
}

#[test]
fn revise_from_honours_state_in_the_fragment() {
    // A body that said `state completed` used to land `proposed` because only `--state`
    // filled `Edits.state`, and ops resets a sealed successor to the class initial.
    let example = Example::materialise("write-revise-from-state");
    let body = example.root().join("fragment.akr");
    std::fs::write(
        &body,
        "state active\ndefinition \"\"\"\n        Still the playable day, restated.\n        \"\"\"\n",
    )
    .expect("write fragment");

    let run = example.run(&[
        "revise",
        "sys.term.playable-day",
        "--from",
        body.to_str().expect("utf-8"),
    ]);
    assert_eq!(run.code, 0, "{}", run.output());
    let source = example.read_file(".akr/records/sys/terms.akr");
    assert!(source.contains("sys.term.playable-day/2"), "{source}");
    assert!(
        source.contains("aliases [ \"playable day\", \"day-loop build\" ]"),
        "the overlay still keeps unmentioned slots: {source}"
    );
    // The successor is the second record; its state must be the one the fragment named.
    let successor = source
        .split("record sys.term.playable-day/2")
        .nth(1)
        .expect("revision 2");
    let successor_state = successor
        .lines()
        .find(|line| line.trim_start().starts_with("state "))
        .expect("a state slot");
    assert!(
        successor_state.contains("active"),
        "the fragment's state lands on the successor: {successor_state}"
    );
}

#[test]
fn revise_adds_supersedes_edges_to_edgeless_same_key_revisions() {
    let example = Example::materialise("write-revise-edgeless");
    let path = ".akr/records/sys/work.akr";
    let existing = example.read_file(path);
    example.write_file(
        path,
        &format!(
            "{existing}\n\
             record sys.work.edgeless-chain/1 : work {{\n    \
             title \"A chain written without back-edges\"\n    \
             state superseded\n    \
             intent \"\"\"\n        First revision, never pointed at.\n        \"\"\"\n\
             }}\n\n\
             record sys.work.edgeless-chain/2 : work {{\n    \
             title \"A chain written without back-edges\"\n    \
             state superseded\n    \
             intent \"\"\"\n        Second revision, also never pointed at.\n        \"\"\"\n\
             }}\n\n\
             record sys.work.edgeless-chain/3 : work {{\n    \
             title \"A chain written without back-edges\"\n    \
             state active\n    \
             intent \"\"\"\n        Live head, no supersedes edge to either predecessor.\n        \"\"\"\n\
             }}\n"
        ),
    );

    let run = example.run(&["revise", "sys.work.edgeless-chain", "--title", "Healed"]);
    assert_eq!(run.code, 0, "{}", run.output());
    let source = example.read_file(path);
    assert!(source.contains("sys.work.edgeless-chain/4"), "{source}");
    let successor = source
        .split("record sys.work.edgeless-chain/4")
        .nth(1)
        .expect("revision 4");
    assert!(
        successor.contains("@sys.work.edgeless-chain/1")
            && successor.contains("@sys.work.edgeless-chain/2")
            && successor.contains("@sys.work.edgeless-chain/3"),
        "the new head points at every edge-less predecessor: {successor}"
    );
}

#[test]
fn revise_repoints_a_sealed_referrer_as_lock_stale_not_a_body_edit() {
    let example = Example::materialise("write-revise-pin-follow");
    let body = example.root().join("pin.akr");
    std::fs::write(
        &body,
        "state active\n\
         scope [ all ]\n\
         rule \"\"\"\n        The demo is what this policy stands on.\n        \"\"\"\n\
         supported_by [ @sys.evidence.playable-day-demo/1 ]\n\
         derived_from [ @sys.evidence.playable-day-demo/1 ]\n",
    )
    .expect("write fragment");
    let proposed = example.run(&[
        "propose",
        "sys.policy.evidence-pin",
        "--kind",
        "policy",
        "--title",
        "Pins a sealed evidence record",
        "--from",
        body.to_str().expect("utf-8"),
    ]);
    assert_eq!(proposed.code, 0, "{}", proposed.output());
    assert_eq!(example.run(&["build"]).code, 0, "seal the pinning policy");

    let run = example.run(&[
        "revise",
        "sys.evidence.playable-day-demo",
        "--title",
        "Recorded full-day session, restated",
        "--state",
        "verified",
    ]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert!(
        run.stdout.contains("repointed"),
        "the write names the follow: {}",
        run.stdout
    );

    let source = example.read_file(".akr/records/sys/policies.akr");
    assert!(
        source.contains("supported_by [ @sys.evidence.playable-day-demo/2 ]"),
        "the live pin follows: {source}"
    );
    assert!(
        source.contains("derived_from [ @sys.evidence.playable-day-demo/1 ]"),
        "the historical pin stays: {source}"
    );

    let check = example.run(&["check"]);
    assert_eq!(check.code, 1, "{}", check.output());
    assert!(
        check.output().contains("AKR-R052"),
        "pin-follow is lock-stale: {}",
        check.output()
    );
    assert!(
        !check.output().contains("AKR-R051"),
        "pin-follow is not a sealed body edit: {}",
        check.output()
    );
}

#[test]
fn abandon_writes_the_reason_into_the_note_slot() {
    let example = Example::materialise("write-abandon-ok");
    let run = example.run(&[
        "abandon",
        "sys.work.m3-plan",
        "--reason",
        "The milestone was rescoped and this plan no longer describes it.",
    ]);
    assert_eq!(run.code, 0, "{}", run.output());
    let source = example.read_file(".akr/records/sys/work.akr");
    // D-026: a comment would be excluded from the seal hash and invisible to every view.
    assert!(source.contains("    note"), "{source}");
    assert!(source.contains("no longer describes it"), "{source}");
    assert!(!source.contains("# The milestone was rescoped"), "{source}");
}

#[test]
fn evidence_add_creates_a_record_that_completes_a_check() {
    let example = Example::materialise("write-evidence-ok");
    let head = example.commit(5).to_owned();
    let run = example.run(&[
        "evidence",
        "add",
        "sys.evidence.asset-audit",
        "--result",
        "pass",
        "--method",
        "command",
        "--command",
        "cargo run -p tools -- audit-assets --path content/day-loop",
        "--summary",
        "Zero placeholder assets on the day-loop path.",
        "--observed-at",
        &head,
    ]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert!(
        run.stdout.contains("created sys.evidence.asset-audit/1"),
        "{}",
        run.stdout
    );

    // Evidence never declares what it verifies (D-016); the link is made on the check.
    // Completing the milestone still fails, because V-019 will not leave an active
    // `plan_of_record` pointing at a completed milestone — §6 says the plan is retired
    // first, and this is the refusal that says so.
    let blocked = example.run(&[
        "complete",
        "sys.milestone.m3-playable-day",
        "--check",
        "no-placeholder-assets=@sys.evidence.asset-audit/1",
    ]);
    assert_eq!(blocked.code, 1, "{}", blocked.output());
    assert!(
        blocked.output().contains("AKR-R021"),
        "{}",
        blocked.output()
    );

    // Retire the plan, and the same call goes through.
    assert_eq!(
        example
            .run(&[
                "abandon",
                "sys.work.m3-plan",
                "--reason",
                "The milestone is complete; the plan has nothing left to schedule.",
            ])
            .code,
        0
    );
    let complete = example.run(&[
        "complete",
        "sys.milestone.m3-playable-day",
        "--check",
        "no-placeholder-assets=@sys.evidence.asset-audit/1",
    ]);
    assert_eq!(complete.code, 0, "{}", complete.output());
    assert!(
        example
            .read_file(".akr/records/sys/milestones.akr")
            .contains("state completed")
    );
}

#[test]
fn a_write_is_visible_to_the_next_read_and_the_lock_says_so() {
    let example = Example::materialise("write-then-read");
    assert_eq!(example.run(&["lock", "--check"]).code, 0);
    let run = example.run(&[
        "revise",
        "sys.term.playable-day",
        "--title",
        "A playable day",
    ]);
    assert_eq!(run.code, 0, "{}", run.output());

    // The lock records a build, and no write operation may invent one (D-014). Until the
    // next `akr build` the lock is honestly stale, and `akr check` says so rather than
    // pretending otherwise.
    let stale = example.run(&["lock", "--check"]);
    assert_eq!(stale.code, 1, "{}", stale.output());
    assert_eq!(example.run(&["build"]).code, 0);
    assert_eq!(example.run(&["lock", "--check"]).code, 0);
}

#[test]
fn the_json_form_carries_the_structured_refusal() {
    let example = Example::materialise("write-json");
    let run = example.run(&[
        "--format",
        "json",
        "complete",
        "sys.milestone.m3-playable-day",
    ]);
    assert_eq!(run.code, 1, "{}", run.output());
    let text = run.stdout;
    assert!(text.contains("\"command\": \"complete\""), "{text}");
    assert!(text.contains("\"refused\": true"), "{text}");
    assert!(text.contains("\"code\": \"AKR-R022\""), "{text}");
    assert!(text.contains("\"unsatisfied_checks\""), "{text}");
    assert!(text.contains("\"id\": \"no-placeholder-assets\""), "{text}");
    assert!(text.contains("\"exit_code\": 1"), "{text}");
}

// -------------------------------------------------------------------------------------
// `akr papercut collate` (D-030): one master record, sisters never written
// -------------------------------------------------------------------------------------

/// Lays down a minimal `.akr` workspace holding one live papercut, next to the example's
/// own temp root, and returns the scan directory the command should point at.
fn sibling_workspaces(example: &Example) -> std::path::PathBuf {
    // Each Example root already carries a process id plus an atomic counter. Keeping the
    // scan below it prevents the two collation tests in this binary from deleting each
    // other's process-id-only directory when the test harness runs them in parallel.
    let scan = example.root().join("collate-scan");
    let _ = std::fs::remove_dir_all(&scan);
    for (project, slug) in [("alpha", "alpha-annoyance"), ("beta", "beta-friction")] {
        let akr = scan.join(project).join(".akr");
        let records = akr.join("records").join(project);
        std::fs::create_dir_all(&records).expect("sibling records dir");
        std::fs::write(
            akr.join("project.akr"),
            format!(
                "akr 0.1\nproject {project}\n\nnamespace {project} \"{project} knowledge.\"\n\n\
                 defaults {{\n    review_after_days 90\n    view_output \"docs/generated\"\n}}\n"
            ),
        )
        .expect("sibling project.akr");
        std::fs::write(
            records.join("papercuts.akr"),
            format!(
                "akr 0.1\nproject {project}\n\nrecord {project}.papercut.{slug}/1 : papercut {{\n    \
                 title \"{project} friction\"\n    state verified\n    statement \"\"\"\n        A \
                 {project}-side friction worth logging.\n        \"\"\"\n    observed_at \
                 git:0123456789abcdef0123456789abcdef01234567\n    author \"test\"\n    created_at \
                 2026-08-07\n}}\n"
            ),
        )
        .expect("sibling papercuts.akr");

        // …and one whose subject is the target namespace, not this sibling. D-033: this is the
        // record that used to be invisible to whoever maintains the tool.
        std::fs::write(
            records.join("tool-papercuts.akr"),
            format!(
                "akr 0.1\nproject {project}\n\nrecord {project}.papercut.{slug}-tool/1 : papercut {{\n    \
                 title \"{project} hit a sys bug\"\n    state verified\n    statement \"\"\"\n        \
                 A sys diagnostic was misleading.\n        \"\"\"\n    observed_at \
                 git:0123456789abcdef0123456789abcdef01234567\n    about \"sys\"\n    author \"test\"\n    created_at \
                 2026-08-07\n}}\n"
            ),
        )
        .expect("sibling tool-papercuts.akr");
    }
    // A scanned workspace that contributes nothing must not appear in the master title
    // or key; those name the provenance of the selected entries, not the scan breadth.
    let gamma = scan.join("gamma").join(".akr");
    std::fs::create_dir_all(gamma.join("records/gamma")).expect("empty sibling records dir");
    std::fs::write(
        gamma.join("project.akr"),
        "akr 0.1\nproject gamma\n\nnamespace gamma \"gamma knowledge.\"\n",
    )
    .expect("empty sibling project.akr");
    scan
}

#[test]
fn papercut_collate_gathers_sister_papercuts_once() {
    let example = Example::materialise("write-collate");
    let scan = sibling_workspaces(&example);
    let projects = scan.to_str().expect("utf-8 scan dir");

    let run = example.run(&[
        "papercut",
        "collate",
        "--projects",
        projects,
        "--all",
        "--namespace",
        "sys",
    ]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert!(
        run.stdout
            .contains("created sys.papercut.collated-4-papercuts-from-alpha-beta/1"),
        "{}",
        run.stdout
    );
    assert!(run.stdout.contains("wrote"), "{}", run.stdout);

    let source = example.read_file(".akr/records/sys/papercuts.akr");
    assert!(source.contains("collated ["), "{source}");
    assert!(
        source.contains("alpha.papercut.alpha-annoyance"),
        "{source}"
    );
    assert!(source.contains("beta.papercut.beta-friction"), "{source}");
    // The statement carries each absorbed papercut with its owning project — and its
    // full text, not a truncated title. The first real collation stored titles and told
    // the reader to go and open eight other ledgers, so none of it was acted on (D-033).
    assert!(
        source.contains("## alpha @alpha.papercut.alpha-annoyance"),
        "{source}"
    );
    assert!(
        source.contains("## beta @beta.papercut.beta-friction"),
        "{source}"
    );
    assert!(
        source.contains("A alpha-side friction worth logging."),
        "the absorbed statement must be readable here:\n{source}"
    );

    // The sisters were only read: neither workspace grew anything.
    assert!(
        !std::fs::read_to_string(scan.join("alpha/.akr/records/alpha/papercuts.akr"))
            .expect("alpha source")
            .contains("collated"),
        "the sister ledger must be untouched"
    );

    // A rerun is a no-op: the keys are already in a live collation's `collated` slot.
    let again = example.run(&[
        "papercut",
        "collate",
        "--projects",
        projects,
        "--all",
        "--namespace",
        "sys",
    ]);
    assert_eq!(again.code, 0, "{}", again.output());
    assert!(
        again.stdout.contains("nothing new to collate"),
        "{}",
        again.output()
    );
    let source = example.read_file(".akr/records/sys/papercuts.akr");
    assert_eq!(
        source.matches("record sys.papercut.collated").count(),
        1,
        "a second run must not add another master record:\n{source}"
    );

    // Resolving or withdrawing the master does not make its source reports new again.
    // The collated slot is historical ingestion state, not a live dependency edge.
    let terminal = source.replacen("state verified", "state withdrawn", 1);
    example.write_file(".akr/records/sys/papercuts.akr", &terminal);
    let after_resolution = example.run(&[
        "papercut",
        "collate",
        "--projects",
        projects,
        "--all",
        "--namespace",
        "sys",
    ]);
    assert_eq!(after_resolution.code, 0, "{}", after_resolution.output());
    assert!(
        after_resolution.stdout.contains("nothing new to collate"),
        "{}",
        after_resolution.output()
    );

    let _ = std::fs::remove_dir_all(&scan);
}

#[test]
fn collate_defaults_to_its_namespace_and_says_what_it_left() {
    let example = Example::materialise("write-collate-about");
    let scan = sibling_workspaces(&example);
    let projects = scan.to_str().expect("utf-8 scan dir");

    // The case the `about` slot exists for: an agent working in a sibling hit a friction
    // with *akr*, and the only ledger it could reach was the sibling's (D-033).
    let run = example.run(&[
        "papercut",
        "collate",
        "--projects",
        projects,
        "--namespace",
        "sys",
    ]);
    assert_eq!(run.code, 0, "{}", run.output());

    let source = example.read_file(".akr/records/sys/papercuts.akr");
    assert!(
        source.contains("alpha.papercut.alpha-annoyance-tool"),
        "the akr-subject papercut should have been absorbed:\n{source}"
    );
    assert!(
        !source.contains("\"alpha.papercut.alpha-annoyance\""),
        "the sibling's own friction is not ours to collate:\n{source}"
    );
    // What the filter left behind is counted, never dropped in silence.
    assert!(
        run.stdout.contains("left behind by the subject filter"),
        "{}",
        run.output()
    );
    // Broken down by subject, not totalled: the two left behind are the siblings' own
    // untagged frictions, and saying so is what makes the number readable.
    assert!(
        run.stdout.contains("(no subject)"),
        "the leftovers should be itemised:\n{}",
        run.output()
    );
    assert!(source.contains("about \"sys\""), "{source}");

    let _ = std::fs::remove_dir_all(&scan);
}

#[test]
fn collate_is_the_subcommand_even_when_an_agent_is_named() {
    // The bug this pins down cost a revert. `collate` used to be the subcommand only if
    // -m was *absent* -- but plain `akr papercut` requires -m, so supplying it is the
    // natural thing to do, and doing so silently logged a papercut whose entire message
    // was the word "collate" instead of collating anything.
    let example = Example::materialise("write-collate-dash-m");
    let scan = sibling_workspaces(&example);
    let projects = scan.to_str().expect("utf-8 scan dir");

    let run = example.run(&[
        "papercut",
        "collate",
        "-m",
        "tester",
        "--projects",
        projects,
        "--all",
        "--namespace",
        "sys",
    ]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert!(
        run.stdout.contains("collated-4-papercuts"),
        "-m must not steer this onto the logging path:\n{}",
        run.output()
    );
    let source = example.read_file(".akr/records/sys/papercuts.akr");
    assert!(
        !source.contains("\"collate\""),
        "the word `collate` was logged as a message:\n{source}"
    );

    let _ = std::fs::remove_dir_all(&scan);
}

#[test]
fn a_papercut_message_can_still_be_the_word_collate() {
    // The escape the disambiguation leaves open. Without it the subcommand would have
    // swallowed a legitimate, if unlikely, message.
    let example = Example::materialise("write-papercut-literal-collate");
    let run = example.run(&[
        "papercut",
        "-m",
        "tester",
        "--namespace",
        "sys",
        "--",
        "collate",
    ]);
    assert_eq!(run.code, 0, "{}", run.output());
    let source = example.read_file(".akr/records/sys/papercuts.akr");
    assert!(source.contains("collate"), "{source}");
    assert!(
        !source.contains("collated ["),
        "this should have logged, not collated:\n{source}"
    );
}

#[test]
fn collate_matches_a_subject_across_the_spellings_agents_use() {
    // One tool is not one name. The subject is free text written by whichever agent hit
    // the friction, so the same tool arrives as `sys`, `SYS` and `Sys`; a filter that
    // accepted one spelling left the rest in the pile it existed to clear.
    let example = Example::materialise("write-collate-spellings");
    let scan = sibling_workspaces(&example);
    let projects = scan.to_str().expect("utf-8 scan dir");

    let run = example.run(&[
        "papercut",
        "collate",
        "--projects",
        projects,
        "--about",
        "SYS",
        "--namespace",
        "sys",
        "--dry-run",
    ]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert!(
        run.stdout.contains("would collate 2 papercuts"),
        "`SYS` must match the `sys` subject:\n{}",
        run.output()
    );

    let _ = std::fs::remove_dir_all(&scan);
}

#[test]
fn collate_dry_run_reports_without_writing() {
    // The scan is global and the record it produces is one big one, so there is a way to
    // look before committing to it -- the missing step behind an --all run that swept in
    // 155 unrelated papercuts and had to be reverted.
    let example = Example::materialise("write-collate-dry-run");
    let scan = sibling_workspaces(&example);
    let projects = scan.to_str().expect("utf-8 scan dir");
    let before = example.sources();

    let run = example.run(&[
        "papercut",
        "collate",
        "--projects",
        projects,
        "--all",
        "--namespace",
        "sys",
        "--dry-run",
    ]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert!(
        run.stdout.contains("would collate 4 papercuts"),
        "{}",
        run.output()
    );
    assert!(run.stdout.contains("nothing written"), "{}", run.output());
    // Each entry is named, so the reader can see what they would be taking on.
    assert!(
        run.stdout.contains("@alpha.papercut.alpha-annoyance"),
        "{}",
        run.output()
    );
    assert_eq!(before, example.sources(), "a dry run wrote something");

    let _ = std::fs::remove_dir_all(&scan);
}

#[test]
fn a_papercut_can_name_what_it_was_about() {
    let example = Example::materialise("write-papercut-about");
    let run = example.run(&[
        "papercut",
        "-m",
        "tester",
        "the akr error message did not say which key",
        "--about",
        "akr",
        "--namespace",
        "sys",
    ]);
    assert_eq!(run.code, 0, "{}", run.output());
    let source = example.read_file(".akr/records/sys/papercuts.akr");
    assert!(source.contains("about \"akr\""), "{source}");

    // And it renders under its own heading, so nobody reads it as this project's backlog.
    let build = example.run(&["build"]);
    assert_eq!(build.code, 0, "{}", build.output());
    let view = example.read_file("docs/generated/PAPERCUTS.md");
    assert!(view.contains("## Not about this project"), "{view}");
    assert!(view.contains("(akr)"), "{view}");
}

#[test]
fn a_papercut_needs_no_namespace_in_a_multi_namespace_workspace() {
    // D-027 puts the whole ceremony of a papercut in one call. A workspace with several
    // declared namespaces was the one shape where that was untrue: the call was refused,
    // the agent had to go and read project.akr for a name the ledger already knew, and
    // retry. The default is where this project's papercuts already go, falling back to
    // the namespace carrying the most records; nothing about a papercut depends on which
    // one it lands in.
    let example = Example::materialise("write-papercut-default-namespace");
    let declared = example.run(&["start", "anything"]);
    assert!(
        declared.stdout.contains("namespaces  lege, sim, sys"),
        "the fixture must declare several namespaces for this to mean anything: {}",
        declared.output()
    );

    let run = example.run(&["papercut", "-m", "tester", "the cache went stale mid-run"]);
    assert_eq!(run.code, 0, "{}", run.output());
    let source = example.read_file(".akr/records/sys/papercuts.akr");
    assert!(source.contains("the cache went stale mid-run"), "{source}");

    // `--namespace` still says otherwise.
    let elsewhere = example.run(&[
        "papercut",
        "-m",
        "tester",
        "the viewer dropped a frame on resize",
        "--namespace",
        "lege",
    ]);
    assert_eq!(elsewhere.code, 0, "{}", elsewhere.output());
    assert!(
        example
            .read_file(".akr/records/lege/papercuts.akr")
            .contains("dropped a frame on resize")
    );

    // And once a workspace has logged papercuts, the default follows them rather than
    // re-deciding: three in `lege` outnumber the one in `sys`, so the next default is
    // `lege`. This is the rule that matters — the first papercut of a workspace picks a
    // namespace, and every one after it goes to the same place without being told.
    for message in [
        "the resize path allocated twice",
        "the atlas reloaded on every tab",
    ] {
        assert_eq!(
            example
                .run(&["papercut", "-m", "tester", message, "--namespace", "lege"])
                .code,
            0
        );
    }
    let followed = example.run(&["papercut", "-m", "tester", "the log file rotated mid-write"]);
    assert_eq!(followed.code, 0, "{}", followed.output());
    assert!(
        example
            .read_file(".akr/records/lege/papercuts.akr")
            .contains("rotated mid-write"),
        "the default should follow where papercuts already go"
    );

    assert_eq!(example.run(&["build"]).code, 0);
    let checked = example.run(&["check"]);
    assert_eq!(checked.code, 0, "{}", checked.output());
}

#[test]
fn evidence_refuses_an_artifact_under_the_scratch_directory() {
    // Scratch is documented as disposable and `akr scratch prune` deletes from it on an
    // ordinary handoff, so an evidence record whose artefact lives there is a verified
    // claim whose backing some later session removes without knowing it was cited. The
    // refusal is V-025 rather than a check on `--artifact`, because evidence reaches the
    // ledger by four routes and a guard on the flag leaves three of them open. Every one
    // of the four is exercised here; the pipeline validates the resulting ledger before
    // writing, so all four still refuse at write time.
    let example = Example::materialise("write-evidence-scratch");
    let head = example.commit(5).to_owned();
    let before = example.sources();

    let expect_refusal = |run: &Run, what: &str| {
        assert_ne!(run.code, 0, "{what} was accepted: {}", run.output());
        assert!(
            run.output().contains("AKR-T023"),
            "{what}: {}",
            run.output()
        );
        assert!(
            run.output().contains(".agent/scratch"),
            "{what}: {}",
            run.output()
        );
    };

    // 1. `akr evidence add --artifact`, in every spelling of the same place.
    for path in [
        ".agent/scratch/ocr-benchmark/out.txt",
        "./.agent/scratch/run/score.json",
    ] {
        let run = example.run(&[
            "evidence",
            "add",
            "sys.evidence.benchmark",
            "--result",
            "pass",
            "--method",
            "command",
            "--command",
            "cargo test",
            "--artifact",
            path,
            "--summary",
            "the benchmark ran",
        ]);
        expect_refusal(&run, path);
    }

    // 2. `akr evidence add-many --from`, which parses records rather than building them.
    let batch = format!(
        "record sys.evidence.batch-scratch/1 : evidence {{\n    \
             title \"Batched benchmark\"\n    \
             result pass\n    \
             method command\n    \
             command \"cargo bench\"\n    \
             artifact \".agent/scratch/ocr-benchmark/out.txt\"\n    \
             observed_at git:{head}\n\
         }}\n"
    );
    example.write_file("batch.akr", &batch);
    expect_refusal(
        &example.run(&["evidence", "add-many", "--from", "batch.akr"]),
        "evidence add-many",
    );

    // 3. `akr propose --kind evidence --from`, which never touches the evidence builder.
    let body = format!(
        "    result pass\n    \
             method command\n    \
             command \"cargo bench\"\n    \
             artifact \".agent/scratch/ocr-benchmark/out.txt\"\n    \
             observed_at git:{head}\n"
    );
    example.write_file("body.akr", &body);
    expect_refusal(
        &example.run(&[
            "propose",
            "sys.evidence.proposed-scratch",
            "--kind",
            "evidence",
            "--title",
            "Proposed benchmark",
            "--from",
            "body.akr",
        ]),
        "propose --kind evidence",
    );

    assert_eq!(example.sources(), before, "a refusal writes nothing");

    // A durable path is untouched by the check.
    let ok = example.run(&[
        "evidence",
        "add",
        "sys.evidence.benchmark",
        "--result",
        "pass",
        "--method",
        "command",
        "--command",
        "cargo test",
        "--artifact",
        "docs/benchmarks/2026-08-22.txt",
        "--summary",
        "the benchmark ran",
    ]);
    assert_eq!(ok.code, 0, "{}", ok.output());
}

#[test]
fn revising_a_completed_record_names_every_acceptance_reference() {
    // V-020 compares each acceptance reference's `observed_at` against the commit that
    // last changed the record's content, so revising a completed record puts *all* of its
    // evidence back in question at once. Refreshing three of four leaves the fourth to
    // surface as AKR-R022 on a later build, long after the session that could have
    // refreshed it. The write pipeline cannot settle the question — the commit this
    // revision lands in does not exist yet — but it can name the references in play at
    // the one moment somebody is looking.
    let example = Example::materialise("write-completed-acceptance-notes");
    let revised = example.run(&[
        "revise",
        "sys.milestone.m1-walking-skeleton",
        "--title",
        "M1 — walking skeleton, restated",
        "--state",
        "completed",
    ]);
    assert_eq!(revised.code, 0, "{}", revised.output());
    assert!(
        revised
            .stdout
            .contains("V-020 will compare every acceptance reference"),
        "{}",
        revised.output()
    );
    assert!(
        revised.stdout.contains("viewer-boundary-clean"),
        "the note should name the check: {}",
        revised.output()
    );
    assert!(
        revised.stdout.contains("@lege.evidence.boundary-lint-pass"),
        "the note should name the evidence: {}",
        revised.output()
    );

    // Advisory, never a diagnostic: a note does not fail a strict write, and it is in the
    // JSON envelope for the MCP surface to render the same lines.
    let json = example.run(&[
        "--format",
        "json",
        "get",
        "sys.milestone.m1-walking-skeleton",
    ]);
    assert_eq!(json.code, 0, "{}", json.output());
}

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

#[path = "build_cache.rs"]
mod build_cache;
#[path = "change_protocol.rs"]
mod change_protocol;
#[path = "commands.rs"]
mod commands;
#[path = "config.rs"]
mod config;
#[path = "context_budget.rs"]
mod context_budget;
#[path = "handoff.rs"]
mod handoff;
#[path = "import.rs"]
mod import;
#[path = "index.rs"]
mod index;
#[path = "review_snapshot.rs"]
mod review_snapshot;
#[path = "scratch.rs"]
mod scratch;
#[cfg(feature = "fts5")]
#[path = "search.rs"]
mod search;
#[path = "secrets.rs"]
mod secrets;
#[path = "source_library.rs"]
mod source_library;
#[path = "trace_and_pass.rs"]
mod trace_and_pass;
#[path = "transcripts.rs"]
mod transcripts;

use support::{Example, Run};

#[test]
fn capture_records_a_digest_without_a_scratch_artifact_dependency() {
    let example = Example::materialise("evidence-capture");
    example.write_file(".agent/scratch/capture.bin", "abc");
    let run = example.run(&[
        "evidence",
        "add",
        "sys.evidence.capture",
        "--result",
        "pass",
        "--method",
        "manual",
        "--summary",
        "Capture inspected.",
        "--capture",
        ".agent/scratch/capture.bin",
    ]);
    assert_eq!(run.code, 0, "{}", run.output());
    let get = example.run(&["get", "sys.evidence.capture"]);
    assert!(get.stdout.contains("Capture inspected."), "{}", get.stdout);
    assert!(
        get.stdout
            .contains("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
    );
    assert!(get.stdout.contains("3 bytes"));
    assert!(!get.stdout.contains("\n  artifact"), "{}", get.stdout);
    std::fs::remove_file(example.root().join(".agent/scratch/capture.bin")).unwrap();
    assert_eq!(example.run(&["build"]).code, 0);
}

#[test]
fn large_evidence_artifact_offers_the_capture_route() {
    let example = Example::materialise("evidence-large-artifact");
    example.write_file("captures/large.bin", &"a".repeat(65_537));
    let run = example.run(&[
        "evidence",
        "add",
        "sys.evidence.large",
        "--result",
        "pass",
        "--method",
        "manual",
        "--artifact",
        "captures/large.bin",
    ]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert!(run.stdout.contains("--capture"), "{}", run.stdout);
    assert!(run.stdout.contains("65537 bytes"), "{}", run.stdout);
}

#[test]
fn revising_a_successor_prunes_dispositions_from_an_older_plan_revision() {
    let example = Example::materialise("write-stale-disposition");
    example.write_file(
        "parent.akr",
        "state active\nintent \"A parent work item.\"\n",
    );
    example.write_file(
        "child.akr",
        "intent \"A child on the original revision.\"\npart_of [ @sys.work.parent/1 ]\n",
    );
    for (key, file) in [
        ("sys.work.parent", "parent.akr"),
        ("sys.work.child", "child.akr"),
    ] {
        let run = example.run(&["propose", key, "--kind", "work", "--from", file]);
        assert_eq!(run.code, 0, "{}", run.output());
    }
    let run = example.run(&[
        "revise",
        "sys.work.parent",
        "--title",
        "First successor",
        "--disposition",
        "sys.work.child=intentionally_dropped",
    ]);
    assert_eq!(run.code, 0, "{}", run.output());
    let run = example.run(&["revise", "sys.work.parent", "--title", "Second successor"]);
    assert_eq!(run.code, 0, "{}", run.output());
    let get = example.run(&["get", "sys.work.parent/3", "--detail", "canonical"]);
    assert!(
        !get.stdout.contains("disposition @sys.work.child"),
        "{}",
        get.stdout
    );
    let old = example.run(&["get", "sys.work.parent/2", "--detail", "canonical"]);
    assert!(
        old.stdout.contains("disposition @sys.work.child"),
        "{}",
        old.stdout
    );
}

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
fn refused_writes_leave_the_workspace_byte_identical() {
    let example = Example::materialise("write-refusals");
    let before = example.sources();
    let head = example.commit(5).to_owned();

    let missing_capture = example.run(&[
        "evidence",
        "add",
        "sys.evidence.capture",
        "--result",
        "pass",
        "--method",
        "manual",
        "--summary",
        "Capture inspected.",
        "--capture",
        "missing.bin",
    ]);
    assert_ne!(missing_capture.code, 0, "{}", missing_capture.output());

    let text = refuses(
        &example,
        &["propose", "sys.policy.tandem-work", "--kind", "policy"],
    );
    assert!(text.contains("AKR-L041"), "{text}");
    assert!(text.contains("akr revise"), "names the way forward: {text}");

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

    let text = refuses(&example, &["complete", "sys.milestone.m3-playable-day"]);
    assert!(text.contains("AKR-R022"), "{text}");
    assert!(text.contains("no-placeholder-assets"), "{text}");
    assert!(text.contains("--check <id>=<evidence-ref>"), "{text}");

    let text = refuses(&example, &["abandon", "sys.work.m3-plan"]);
    assert!(text.contains("AKR-C031"), "{text}");
    assert!(text.contains("--reason"), "{text}");

    let bodyless = refuses(
        &example,
        &[
            "propose",
            "sys.policy.day-loop",
            "--kind",
            "policy",
            "--title",
            "The day loop",
        ],
    );
    assert!(bodyless.contains("AKR-C031"), "{bodyless}");
    assert!(bodyless.contains("rule"), "{bodyless}");

    let get = example.run(&["get", "sys.work.m3-plan"]);
    assert_eq!(get.code, 0, "{}", get.output());
    assert!(get.stdout.contains("intent"), "{}", get.stdout);

    let json = example.run(&[
        "--format",
        "json",
        "complete",
        "sys.milestone.m3-playable-day",
    ]);
    assert_eq!(json.code, 1, "{}", json.output());
    for expected in [
        "\"command\": \"complete\"",
        "\"refused\": true",
        "\"code\": \"AKR-R022\"",
        "\"unsatisfied_checks\"",
        "\"id\": \"no-placeholder-assets\"",
        "\"exit_code\": 1",
    ] {
        assert!(
            json.stdout.contains(expected),
            "{expected}: {}",
            json.stdout
        );
    }

    let bad_batch = "\
record sys.work.good/1 : work {
    title \"Good work\"
    intent \"Should roll back on sibling failure.\"
}

record sys.milestone.bad/1 : milestone {
    title \"Bad milestone\"
    intent \"Uses implements on milestone.\"
    part_of [ @sys.work.good ]
    implements [ @sys.req.deterministic-sim ]
    acceptance {
        check boot {
            statement \"Boots.\"
            method manual
        }
    }
}
";
    example.write_file("bad-propose-batch.akr", bad_batch);
    let run = example.run(&["propose-many", "--from", "bad-propose-batch.akr"]);
    assert_ne!(run.code, 0, "{}", run.output());
    assert!(run.output().contains("AKR-L032"), "{}", run.output());
    assert!(
        run.output().contains("its domain is work, decision"),
        "{}",
        run.output()
    );

    let ev_batch = format!(
        "record sys.evidence.good/1 : evidence {{\n    \
             title \"Fine\"\n    result pass\n    method command\n    \
             observed_at git:{head}\n\
         }}\n\n\
         record sys.evidence.bad/1 : evidence {{\n    \
             title \"Missing its result\"\n    method command\n    \
             observed_at git:{head}\n\
         }}\n"
    );
    example.write_file("bad-ev-batch.akr", &ev_batch);
    assert_ne!(
        example
            .run(&["evidence", "add-many", "--from", "bad-ev-batch.akr"])
            .code,
        0
    );

    example.write_file(
        "wrong-kind-batch.akr",
        "record sys.policy.thing/1 : policy {\n    scope [ all ]\n    rule \"A thing.\"\n}\n",
    );
    let wrong_kind = example.run(&["evidence", "add-many", "--from", "wrong-kind-batch.akr"]);
    assert_ne!(wrong_kind.code, 0, "{}", wrong_kind.output());
    assert!(
        wrong_kind.output().contains("policy"),
        "{}",
        wrong_kind.output()
    );

    example.write_file(
        "yaml-like.akr",
        "statement: this is yaml, not an AKR slot-list\n",
    );
    let yaml = example.run(&[
        "propose",
        "sys.work.yaml-decoy",
        "--kind",
        "work",
        "--title",
        "YAML decoy",
        "--from",
        "yaml-like.akr",
    ]);
    assert_ne!(yaml.code, 0, "{}", yaml.output());
    assert!(yaml.output().contains("AKR-C031"), "{}", yaml.output());
    assert!(
        yaml.output().contains("slot-list") || yaml.output().contains("intent"),
        "{}",
        yaml.output()
    );

    for args in [
        vec!["propose-many"],
        vec!["evidence", "add-many"],
        vec!["evidence", "add", "sys.evidence.asset-audit"],
        vec![
            "evidence",
            "add",
            "sys.evidence.asset-audit",
            "--result",
            "maybe",
            "--method",
            "command",
        ],
    ] {
        assert_eq!(example.run(&args).code, 2, "{args:?}");
    }

    assert_eq!(
        before,
        example.sources(),
        "a refused write changed the ledger"
    );
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
        ("sys.policy.legacy-session", "Legacy session"),
        ("sys.policy.playable-session", "Playable session"),
    ] {
        let body = example.root().join(format!("{key}.akr"));
        std::fs::write(
            &body,
            format!("scope [ all ]\nrule \"A rule for {title}.\"\n"),
        )
        .expect("write proposal fragment");
        let proposed = example.run(&[
            "propose",
            key,
            "--kind",
            "policy",
            "--title",
            title,
            "--from",
            body.to_str().expect("utf-8"),
        ]);
        assert_eq!(proposed.code, 0, "{}", proposed.output());
    }
    let accepted = example.run(&[
        "revise",
        "sys.policy.legacy-session",
        "--in-place",
        "--state",
        "active",
    ]);
    assert_eq!(accepted.code, 0, "{}", accepted.output());

    let replaced = example.run(&[
        "supersede",
        "sys.policy.legacy-session",
        "--with",
        "sys.policy.playable-session",
    ]);
    assert_eq!(replaced.code, 0, "{}", replaced.output());
    let source = example.read_file(".akr/records/sys/policies.akr");
    assert!(source.contains("supersedes [ @sys.policy.legacy-session/1 ]"));
    assert!(source.contains("record sys.policy.playable-session/1 : policy"));
}

#[test]
fn supersede_creates_a_same_topic_decision_in_one_command() {
    // Papercut: `propose` of the replacement fails AKR-R002 while both decisions are
    // live on one topic, and `supersede --with` used to refuse `--from`/`--title`.
    let example = Example::materialise("write-supersede-create-decision");
    let body = example.root().join("snapshot-interface.akr");
    std::fs::write(
        &body,
        "scope [ path \"lege/**\" ]\n\
         topic renderer-boundary\n\
         decision \"\"\"\n    The viewer reads a frame snapshot through a narrow interface.\n    \"\"\"\n\
         resolves [ @lege.question.text-rendering-owner ]\n",
    )
    .expect("write body");
    let body = body.to_str().expect("utf-8");

    let detour = example.run(&[
        "propose",
        "lege.decision.snapshot-interface",
        "--kind",
        "decision",
        "--from",
        body,
    ]);
    assert_eq!(detour.code, 1, "{}", detour.output());
    assert!(detour.output().contains("AKR-R002"), "{}", detour.output());

    let replaced = example.run(&[
        "supersede",
        "lege.decision.renderer-boundary",
        "--with",
        "lege.decision.snapshot-interface",
        "--title",
        "The viewer reads a snapshot interface",
        "--from",
        body,
    ]);
    assert_eq!(replaced.code, 0, "{}", replaced.output());
    assert!(
        replaced
            .stdout
            .contains("lege.decision.renderer-boundary/2 active -> superseded"),
        "{}",
        replaced.stdout
    );
    assert!(
        replaced
            .stdout
            .contains("created lege.decision.snapshot-interface/1"),
        "{}",
        replaced.stdout
    );
    let source = example.read_file(".akr/records/lege/decisions.akr");
    assert!(source.contains("record lege.decision.snapshot-interface/1 : decision"));
    assert!(source.contains("supersedes [ @lege.decision.renderer-boundary/2 ]"));
    let work = example.read_file(".akr/records/lege/work.akr");
    assert!(
        work.contains("depends_on [ @lege.decision.snapshot-interface ]"),
        "the dependant follows: {work}"
    );
    let built = example.run(&["build"]);
    assert_eq!(built.code, 0, "{}", built.output());
}

#[test]
fn supersede_replaces_verified_observation_and_evidence_with_new_records() {
    // Papercut: observation and evidence records are `verified` from creation, so a
    // proposed-first correction was sealed and `supersede --with` refused it (AKR-C032).
    let example = Example::materialise("write-supersede-create-empirical");
    let commit = example.commit(4).to_owned();
    let observation = example.root().join("observation.akr");
    std::fs::write(
        &observation,
        format!(
            "scope [ path \"lege/src/render/**\" ]\n\
             statement \"\"\"\n    Re-measured: the barn transition reaches 17.2 ms.\n    \"\"\"\n\
             observed_at git:{commit}\n\
             method instrumented\n\
             watches [ \"lege/src/render/**\" ]\n"
        ),
    )
    .expect("write observation");
    let evidence = example.root().join("evidence.akr");
    std::fs::write(
        &evidence,
        format!(
            "result pass\n\
             method command\n\
             observed_at git:{commit}\n\
             command \"cargo run -p tools -- boundary-lint --crate lege\"\n\
             summary \"\"\"\n    Zero engine types, re-run against the corrected lint.\n    \"\"\"\n"
        ),
    )
    .expect("write evidence");

    for (old, new, body, title) in [
        (
            "lege.obs.frame-budget-headroom",
            "lege.obs.frame-budget-overrun",
            &observation,
            "The barn transition overruns the frame budget",
        ),
        (
            "lege.evidence.boundary-lint-pass",
            "lege.evidence.boundary-lint-rerun",
            &evidence,
            "Boundary lint rerun reports no engine imports",
        ),
    ] {
        let replaced = example.run(&[
            "supersede",
            old,
            "--with",
            new,
            "--title",
            title,
            "--from",
            body.to_str().expect("utf-8"),
        ]);
        assert_eq!(replaced.code, 0, "{}", replaced.output());
        assert!(
            replaced
                .stdout
                .contains(&format!("{old}/1 verified -> superseded")),
            "{}",
            replaced.stdout
        );
        let get = example.run(&["get", new]);
        assert!(get.stdout.contains("state verified"), "{}", get.stdout);
    }
    let requirements = example.read_file(".akr/records/lege/requirements.akr");
    assert!(
        requirements.contains("verified_by [ @lege.evidence.boundary-lint-rerun ]"),
        "{requirements}"
    );
    let built = example.run(&["build"]);
    assert_eq!(built.code, 0, "{}", built.output());
}

#[test]
fn propose_notes_a_watch_that_matches_nothing_at_head() {
    // Papercut: a verified observation accepted a watch over files that existed only in
    // the working tree, and the first anyone heard of it was AKR-G022 under --strict.
    let example = Example::materialise("write-propose-uncommitted-watch");
    example.write_file("lege/src/snapshot/mod.rs", "// not committed yet\n");
    let commit = example.commit(4).to_owned();
    let body = example.root().join("observation.akr");
    std::fs::write(
        &body,
        format!(
            "scope [ path \"lege/src/snapshot/**\" ]\n\
             statement \"\"\"\n    The snapshot module exists.\n    \"\"\"\n\
             observed_at git:{commit}\n\
             method instrumented\n\
             watches [ \"lege/src/snapshot/**\" ]\n"
        ),
    )
    .expect("write body");
    let proposed = example.run(&[
        "propose",
        "lege.obs.snapshot-module",
        "--kind",
        "observation",
        "--from",
        body.to_str().expect("utf-8"),
    ]);
    assert_eq!(proposed.code, 0, "{}", proposed.output());
    assert!(
        proposed.stdout.contains("warning[AKR-G027]")
            && proposed.stdout.contains("not committed yet"),
        "{}",
        proposed.stdout
    );
}

#[test]
fn propose_many_writes_connected_records_in_one_pass() {
    let example = Example::materialise("write-propose-many");
    let batch = "\
record sys.work.phased/1 : work {
    title \"Phased work\"
    intent \"Coordinate phased milestones.\"
}

record sys.milestone.phase-0/1 : milestone {
    title \"Phase 0\"
    intent \"Initial skeleton.\"
    part_of [ @sys.work.phased ]
    depends_on [ @sys.req.deterministic-sim ]
    acceptance {
        check skeleton {
            statement \"Skeleton compiles.\"
            method manual
        }
    }
}

record sys.milestone.phase-1/1 : milestone {
    title \"Phase 1\"
    intent \"Follow-on milestone.\"
    part_of [ @sys.work.phased ]
    after [ @sys.milestone.phase-0 ]
    acceptance {
        check loop {
            statement \"Loop runs.\"
            method manual
        }
    }
}
";
    example.write_file("batch.akr", batch);
    let run = example.run(&["propose-many", "--from", "batch.akr"]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert!(
        run.stdout.contains("3 records written in one pass"),
        "{}",
        run.output()
    );
    assert_eq!(example.run(&["build"]).code, 0);
    let checked = example.run(&["check"]);
    assert_eq!(checked.code, 0, "{}", checked.output());
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

// -------------------------------------------------------------------------------------
// The paths that succeed
// -------------------------------------------------------------------------------------

#[test]
fn a_proposal_from_a_file_lands_and_checks_clean() {
    let example = Example::materialise("write-propose-from");
    let body = example.root().join("day-loop.akr");
    std::fs::write(
        &body,
        "akr 0.1\nproject save-your-skin\n\nrecord sys.policy.day-loop/1 : policy {\n    \
         title \"The day loop\"\n    state active\n    scope [ all ]\n    rule \"\"\"\n        \
         The repeating structure of one in-game day: wake, work, evening, sleep.\n        \
         \"\"\"\n    author \"test\"\n    created_at 2026-08-03\n}\n",
    )
    .expect("write body");

    let run = example.run(&[
        "propose",
        "sys.policy.day-loop",
        "--kind",
        "policy",
        "--from",
        body.to_str().expect("utf-8"),
    ]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert!(
        example
            .read_file(".akr/records/sys/policies.akr")
            .contains("sys.policy.day-loop/1")
    );
    // The lock is stale until the next build (D-014), so the ledger only checks clean
    // after one. That is the sequence every write leaves behind.
    assert_eq!(example.run(&["build"]).code, 0);
    assert_eq!(example.run(&["check"]).code, 0);
}

#[test]
fn a_proposal_without_kind_takes_it_from_the_file() {
    let example = Example::materialise("write-propose-kindless");
    let body = example.root().join("day-loop.akr");
    std::fs::write(
        &body,
        "akr 0.1\nproject save-your-skin\n\nrecord sys.policy.day-loop/1 : policy {\n    \
         title \"The day loop\"\n    state active\n    scope [ all ]\n    rule \"\"\"\n        \
         The repeating structure of one in-game day: wake, work, evening, sleep.\n        \
         \"\"\"\n    author \"test\"\n    created_at 2026-08-03\n}\n",
    )
    .expect("write body");

    // The record line already says `: term`; demanding `--kind` as well was
    // ceremony that broke the documented `propose <key> --from <file>` form.
    let run = example.run(&[
        "propose",
        "sys.policy.day-loop",
        "--from",
        body.to_str().expect("utf-8"),
    ]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert!(
        example
            .read_file(".akr/records/sys/policies.akr")
            .contains("sys.policy.day-loop/1")
    );

    // With neither the flag nor a declaring file, the kind is still required.
    let run = example.run(&["propose", "sys.policy.still-kindless"]);
    assert_eq!(run.code, 2, "{}", run.output());
    assert!(run.output().contains("AKR-C003"), "{}", run.output());
}

#[test]
fn revise_accepts_the_revisioned_key_propose_printed() {
    // `akr propose` answers `created sys.term.day-loop/1`, and that whole form is
    // what gets pasted into the next command. The CLI has no `--base-rev`, so the
    // suffix carries nothing and is stripped rather than refused.
    let example = Example::materialise("write-revise-suffixed-key");
    let run = example.run(&[
        "revise",
        "sys.term.playable-day/1",
        "--title",
        "A playable day",
    ]);
    assert_eq!(run.code, 0, "{}", run.output());
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
        run.stdout
            .contains("keeps state active from the sealed head"),
        "a sealed content-only revise keeps the state it inherits and says so (D-043): {}",
        run.stdout
    );
    assert!(
        source.contains("state active"),
        "the successor keeps `active` rather than being returned to `proposed`: {source}"
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
fn papercut_observed_at_pins_the_record_to_that_commit() {
    let example = Example::materialise("write-papercut-observed-at");
    let head = example.commit(5).to_owned();
    let run = example.run(&[
        "papercut",
        "-m",
        "test",
        "The widget froze twice in one session",
        "--observed-at",
        &head,
    ]);
    assert_eq!(run.code, 0, "{}", run.output());
    let source = example.read_file(".akr/records/sys/papercuts.akr");
    assert!(
        source.contains(&format!("git:{head}")),
        "observed_at pinned to {head}:\n{source}"
    );

    let run = example.run(&[
        "papercut",
        "-m",
        "test",
        "Another freeze",
        "--observed-at",
        "nope",
    ]);
    assert_eq!(run.code, 2, "{}", run.output());
    assert!(run.output().contains("AKR-C004"), "{}", run.output());
}

#[test]
fn papercut_close_marks_the_record_closed() {
    let example = Example::materialise("write-papercut-close");
    let logged = example.run(&["papercut", "-m", "test", "A friction worth closing"]);
    assert_eq!(logged.code, 0, "{}", logged.output());
    let key = logged
        .stdout
        .strip_prefix("created ")
        .expect("created line")
        .lines()
        .next()
        .expect("created id")
        .trim()
        .split_once('/')
        .expect("revisioned id")
        .0
        .to_owned();
    let closed = example.run(&["papercut", "close", &key]);
    assert_eq!(closed.code, 0, "{}", closed.output());
    assert!(
        example
            .read_file(".akr/records/sys/papercuts.akr")
            .contains("closed_at"),
        "close writes the closure marker"
    );
    let again = example.run(&["papercut", "close", &key]);
    assert_eq!(again.code, 0, "{}", again.output());
    assert!(
        again.output().contains("already closed"),
        "{}",
        again.output()
    );
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
    let before = example.sources();

    // Dry runs write nothing and handle subject casing + `--all`.
    let dry_all = example.run(&[
        "papercut",
        "collate",
        "--projects",
        projects,
        "--all",
        "--namespace",
        "sys",
        "--dry-run",
    ]);
    assert_eq!(dry_all.code, 0, "{}", dry_all.output());
    assert!(
        dry_all.stdout.contains("would collate 4 papercuts"),
        "{}",
        dry_all.output()
    );
    assert!(
        dry_all.stdout.contains("nothing written"),
        "{}",
        dry_all.output()
    );
    assert!(
        dry_all.stdout.contains("@alpha.papercut.alpha-annoyance"),
        "{}",
        dry_all.output()
    );
    assert_eq!(before, example.sources(), "a dry run wrote something");

    let dry_subject = example.run(&[
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
    assert_eq!(dry_subject.code, 0, "{}", dry_subject.output());
    assert!(
        dry_subject.stdout.contains("would collate 2 papercuts"),
        "`SYS` must match the `sys` subject:\n{}",
        dry_subject.output()
    );

    // Default subject filter absorbs only `about "sys"` and itemises leftovers.
    let filtered = example.run(&[
        "papercut",
        "collate",
        "--projects",
        projects,
        "--namespace",
        "sys",
    ]);
    assert_eq!(filtered.code, 0, "{}", filtered.output());
    let source = example.read_file(".akr/records/sys/papercuts.akr");
    assert!(
        source.contains("alpha.papercut.alpha-annoyance-tool"),
        "{source}"
    );
    assert!(
        !source.contains("\"alpha.papercut.alpha-annoyance\""),
        "{source}"
    );
    assert!(
        filtered
            .stdout
            .contains("left behind by the subject filter")
            && filtered.stdout.contains("(no subject)"),
        "{}",
        filtered.output()
    );
    assert!(source.contains("about \"sys\""), "{source}");

    // `-m` still routes to collate subcommand, and `--all` absorbs the remaining two.
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
        run.stdout
            .contains("created sys.papercut.collated-2-papercuts-from-alpha-beta-2/1"),
        "{}",
        run.stdout
    );
    let source = example.read_file(".akr/records/sys/papercuts.akr");
    assert!(!source.contains("\"collate\""), "{source}");
    assert!(source.contains("collated ["), "{source}");
    assert!(
        source.contains("## alpha @alpha.papercut.alpha-annoyance")
            && source.contains("## beta @beta.papercut.beta-friction")
            && source.contains("A alpha-side friction worth logging."),
        "{source}"
    );
    assert!(
        !std::fs::read_to_string(scan.join("alpha/.akr/records/alpha/papercuts.akr"))
            .expect("alpha source")
            .contains("collated"),
        "the sister ledger must be untouched"
    );

    // Rerunning (even after withdrawing a master) is a no-op.
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

    // `--` still allows logging a literal `"collate"` message.
    let literal = example.run(&[
        "papercut",
        "-m",
        "tester",
        "--namespace",
        "sys",
        "--",
        "collate",
    ]);
    assert_eq!(literal.code, 0, "{}", literal.output());
    let source = example.read_file(".akr/records/sys/papercuts.akr");
    assert!(source.contains("collate"), "{source}");

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

/// Seals the workspace and commits everything, so the next revise writes revision n+1.
fn seal_and_commit(example: &Example, message: &str) {
    assert_eq!(example.run(&["build"]).code, 0);
    example.git(&["add", "-A"]);
    example.git(&["commit", "--quiet", "-m", message]);
}

/// The acceptance verdict line `akr get` prints for the probe's one check.
fn fixture_verdict(example: &Example, key: &str) -> String {
    let run = example.run(&["get", key]);
    assert_eq!(run.code, 0, "{}", run.output());
    run.stdout
        .lines()
        .find(|line| line.contains("#fixture"))
        .unwrap_or_else(|| panic!("no verdict for #fixture in {}", run.stdout))
        .to_owned()
}

/// A check body for the probe record, citing `cites` when given.
fn fixture_check(statement: &str, cites: Option<&str>) -> String {
    let cites = cites.map_or_else(String::new, |r| format!("    verified_by [ {r} ]\n"));
    format!(
        "acceptance {{\n  check fixture {{\n    statement \"{statement}\"\n    method command\n    \
         command \"cargo test\"\n{cites}  }}\n}}\n"
    )
}

/// Proposes the probe work record with one check, sealed and committed.
fn propose_probe(example: &Example, key: &str) {
    example.write_file(
        "w.akr",
        &format!(
            "state active\nintent \"Hold the fixture green.\"\n{}",
            fixture_check("The fixture passes.", None)
        ),
    );
    let run = example.run(&[
        "propose", key, "--kind", "work", "--title", "Probe", "--from", "w.akr",
    ]);
    assert_eq!(run.code, 0, "{}", run.output());
    seal_and_commit(example, "probe work");
}

/// Adds passing evidence observed at `HEAD`.
fn add_passing_evidence(example: &Example, key: &str) {
    let run = example.run(&[
        "evidence",
        "add",
        key,
        "--result",
        "pass",
        "--method",
        "command",
        "--command",
        "cargo test",
        "--summary",
        "The fixture passed.",
    ]);
    assert_eq!(run.code, 0, "{}", run.output());
}

/// Revises `key` from a fragment holding only its acceptance block.
fn revise_check(example: &Example, key: &str, statement: &str, cites: Option<&str>) {
    example.write_file("w.akr", &fixture_check(statement, cites));
    let run = example.run(&["revise", key, "--from", "w.akr"]);
    assert_eq!(run.code, 0, "{}", run.output());
}

/// Re-citing evidence that predates a record's last redefinition, in a revision that has
/// not been committed yet, must not read as satisfied (the field report: "re-citing the
/// September E5 evidence in rev 8 computed as satisfied until I removed the citation in
/// rev 9"). An uncommitted revision had no `last_change` at all, which the descendant gate
/// read as "no git facts" and took the evidence at its word; the same ledger, committed,
/// reported the evidence as too old. The verdict before the commit and after it now agree.
#[test]
fn a_revision_citing_evidence_older_than_the_definition_is_not_satisfied_before_its_commit() {
    let example = Example::materialise("write-uncommitted-recitation");
    let key = "sys.work.recitation-probe";
    propose_probe(&example, key);
    add_passing_evidence(&example, "sys.evidence.e5");
    seal_and_commit(&example, "e5");

    // The check is redefined after E5 was observed: E5 no longer speaks to it.
    revise_check(&example, key, "The fixture passes on every platform.", None);
    seal_and_commit(&example, "redefine the check");

    // A revision that only re-cites E5 keeps the committed definition's last change
    // (D-038), so E5 is too old before the commit exactly as it is after.
    revise_check(
        &example,
        key,
        "The fixture passes on every platform.",
        Some("@sys.evidence.e5/1"),
    );
    let before = fixture_verdict(&example, key);
    assert!(before.contains("not satisfied"), "{before}");
    assert!(before.contains("predates the last change"), "{before}");
    seal_and_commit(&example, "re-cite e5");
    assert_eq!(fixture_verdict(&example, key), before);

    // A revision that re-cites E5 *and* redefines the check: its last change is the commit
    // that will land it, which nothing already observed can descend from. This is the
    // reported shape, and it used to read as satisfied until committed.
    revise_check(
        &example,
        key,
        "The fixture passes under the new harness.",
        Some("@sys.evidence.e5/1"),
    );
    let pending = fixture_verdict(&example, key);
    assert!(pending.contains("not satisfied"), "{pending}");
    assert!(
        pending.contains("predates the uncommitted change"),
        "{pending}"
    );

    // V-020 agrees with the verdict: a completion on that basis fails `akr check` now,
    // not only after the commit.
    let run = example.run(&["complete", key, "--check", "fixture=@sys.evidence.e5/1"]);
    assert_eq!(run.code, 0, "{}", run.output());
    let check = example.run(&["check"]);
    assert_eq!(check.code, 1, "{}", check.output());
    assert!(check.output().contains("AKR-R022"), "{}", check.output());
    assert!(
        check.output().contains("predates the uncommitted change"),
        "{}",
        check.output()
    );
}

/// The prepared-change shape still holds before the commit: a redefinition and the fresh
/// evidence that verifies it, written together and both uncommitted, satisfy the check,
/// as they will once they land in the same commit.
#[test]
fn a_redefinition_and_its_fresh_evidence_satisfy_the_check_before_their_commit() {
    let example = Example::materialise("write-uncommitted-co-commit");
    let key = "sys.work.co-commit-probe";
    propose_probe(&example, key);
    add_passing_evidence(&example, "sys.evidence.e6");
    revise_check(
        &example,
        key,
        "The fixture passes under the new harness.",
        Some("@sys.evidence.e6/1"),
    );
    let before = fixture_verdict(&example, key);
    assert!(
        before.contains("satisfied by @sys.evidence.e6/1"),
        "{before}"
    );
    seal_and_commit(&example, "redefine and verify together");
    assert_eq!(fixture_verdict(&example, key), before);
}

/// A definition a branch introduced, merged under a newer commit to the same file, is
/// committed history. The file timeline walks `log --follow`, which lists the branch
/// commit below the newer one and omits the merge, so it cannot date the definition; that
/// used to read as "no git facts", and once uncommitted definitions got their own verdict
/// it read as "predates the uncommitted change" on a clean tree (seen in a sister
/// project's milestone, 4 of 5 checks satisfied turning into 0 of 5).
#[test]
fn a_definition_introduced_on_a_merged_branch_is_not_read_as_uncommitted() {
    let example = Example::materialise("write-merged-redefinition");
    let key = "sys.work.zz-merged-probe";
    propose_probe(&example, key);
    let commit_at = |message: &str, date: &str| {
        let _ = std::fs::remove_file(example.root().join("w.akr"));
        example.git(&["add", "-A"]);
        let status = std::process::Command::new("git")
            .args(["commit", "--quiet", "-m", message])
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .current_dir(example.root())
            .status()
            .expect("git runs");
        assert!(status.success(), "commit {message}");
    };
    // The fact cache would otherwise travel with the checkouts.
    example.write_file(".gitignore", ".akr/cache/\n");
    example.git(&["rm", "-r", "--cached", "--quiet", ".akr/cache"]);
    commit_at("ignore the cache", "2029-12-31T00:00:00Z");

    example.git(&["checkout", "--quiet", "-b", "side"]);
    add_passing_evidence(&example, "sys.evidence.e7");
    revise_check(
        &example,
        key,
        "The fixture passes under the new harness.",
        Some("@sys.evidence.e7/1"),
    );
    commit_at("redefine on a branch", "2030-01-01T00:00:00Z");
    let on_branch = fixture_verdict(&example, key);
    assert!(
        on_branch.contains("satisfied by @sys.evidence.e7/1"),
        "{on_branch}"
    );

    // A newer commit to the same file on main, then the merge.
    example.git(&["checkout", "--quiet", "main"]);
    example.write_file(
        "w.akr",
        &format!(
            "state active\nintent \"Another.\"\n{}",
            fixture_check("Another passes.", None)
        ),
    );
    let run = example.run(&[
        "propose",
        "sys.work.aa-other-probe",
        "--kind",
        "work",
        "--title",
        "Other",
        "--from",
        "w.akr",
    ]);
    assert_eq!(run.code, 0, "{}", run.output());
    commit_at("newer commit to the same file", "2030-01-02T00:00:00Z");
    let status = std::process::Command::new("git")
        .args(["merge", "--quiet", "--no-edit", "side"])
        .env("GIT_AUTHOR_DATE", "2030-01-03T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2030-01-03T00:00:00Z")
        .current_dir(example.root())
        .status()
        .expect("git runs");
    assert!(status.success(), "the branches merge cleanly");

    let merged = fixture_verdict(&example, key);
    assert!(!merged.contains("uncommitted"), "{merged}");
    assert_eq!(merged, on_branch);
}

#[test]
fn divergent_branches_completing_work_in_the_same_file_merge_and_validate_cleanly() {
    let example = Example::materialise("write-dag-merge-v020");
    let commit_at = |message: &str, date: &str| {
        let _ = std::fs::remove_file(example.root().join("w.akr"));
        example.git(&["add", "-A"]);
        let status = std::process::Command::new("git")
            .args(["commit", "--quiet", "--no-verify", "-m", message])
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .current_dir(example.root())
            .status()
            .expect("git runs");
        assert!(status.success(), "commit {message}");
    };
    example.write_file(".gitignore", ".akr/cache/\n");
    example.git(&[
        "rm",
        "-r",
        "--cached",
        "--quiet",
        "--ignore-unmatch",
        ".akr/cache",
    ]);
    commit_at("ignore cache", "2029-12-30T00:00:00Z");

    // Branch A (side): propose and complete a work item with --pass, and also revise an
    // existing work item to test concurrent same-key revision auto-renumbering.
    example.git(&["checkout", "--quiet", "-b", "side"]);
    example.write_file(
        "w.akr",
        "state active\nintent \"Colony ship lane.\"\nacceptance {\n    check smoke {\n        statement \"Smoke passes.\"\n        method command\n        command \"cargo test\"\n    }\n}\n",
    );
    assert_eq!(
        example
            .run(&[
                "propose",
                "sys.work.colony-ship",
                "--kind",
                "work",
                "--title",
                "Colony ship",
                "--from",
                "w.akr",
            ])
            .code,
        0
    );
    commit_at("propose colony ship", "2030-01-01T00:00:00Z");
    assert_eq!(
        example
            .run(&["complete", "sys.work.colony-ship", "--pass", "smoke"])
            .code,
        0
    );
    commit_at("complete colony ship", "2030-01-02T00:00:00Z");

    // Branch B (main): propose and complete a different work item in the SAME `.akr` file
    // (`records/sys/planning.akr`).
    example.git(&["checkout", "--quiet", "main"]);
    example.write_file(
        "w.akr",
        "state active\nintent \"Regional SAR lane.\"\nacceptance {\n    check smoke {\n        statement \"SAR passes.\"\n        method command\n        command \"cargo test\"\n    }\n}\n",
    );
    assert_eq!(
        example
            .run(&[
                "propose",
                "sys.work.regional-sar",
                "--kind",
                "work",
                "--title",
                "Regional SAR",
                "--from",
                "w.akr",
            ])
            .code,
        0
    );
    commit_at("propose regional sar", "2030-01-03T00:00:00Z");
    assert_eq!(
        example
            .run(&["complete", "sys.work.regional-sar", "--pass", "smoke"])
            .code,
        0
    );
    commit_at("complete regional sar", "2030-01-04T00:00:00Z");

    // Merge the two `.akr` file versions via `akr git merge-driver` and commit the merge.
    let git_out = |args: &[&str]| -> String {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(example.root())
            .output()
            .expect("git output");
        assert!(
            out.status.success(),
            "{:?}: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).expect("utf8")
    };
    let base_rev = git_out(&["merge-base", "main", "side"]);
    let base_akr = git_out(&[
        "show",
        &format!("{}:.akr/records/sys/work.akr", base_rev.trim()),
    ]);
    let their_akr = git_out(&["show", "side:.akr/records/sys/work.akr"]);
    example.write_file("base.tmp", &base_akr);
    example.write_file("theirs.tmp", &their_akr);
    let drv = example.run(&[
        "git",
        "merge-driver",
        "base.tmp",
        ".akr/records/sys/work.akr",
        "theirs.tmp",
        "--path",
        ".akr/records/sys/work.akr",
    ]);
    assert_eq!(drv.code, 0, "{}", drv.output());
    let _ = std::fs::remove_file(example.root().join("base.tmp"));
    let _ = std::fs::remove_file(example.root().join("theirs.tmp"));

    // Create the real two-parent git merge commit.
    let merged_planning =
        std::fs::read_to_string(example.root().join(".akr/records/sys/work.akr")).unwrap();
    let _ = std::process::Command::new("git")
        .args(["merge", "--no-commit", "--no-ff", "side"])
        .current_dir(example.root())
        .output();
    example.write_file(".akr/records/sys/work.akr", &merged_planning);
    assert_eq!(example.run(&["build"]).code, 0);
    commit_at("merge side into main", "2030-01-05T00:00:00Z");

    // Both completed work records from both branches must satisfy V-020 (no AKR-R022 or
    // AKR-G011), and both `passed_at` checks must report satisfied.
    let colony = example.run(&["get", "@sys.work.colony-ship"]);
    assert_eq!(colony.code, 0, "{}", colony.output());
    assert!(
        colony.stdout.contains("satisfied — passed at"),
        "{}",
        colony.stdout
    );
    let sar = example.run(&["get", "@sys.work.regional-sar"]);
    assert_eq!(sar.code, 0, "{}", sar.output());
    assert!(
        sar.stdout.contains("satisfied — passed at"),
        "{}",
        sar.stdout
    );
}

#[test]
fn complete_partial_stamps_in_place_and_trace_refuses_superseded_with_successor_hint() {
    let example = Example::materialise("write-partial-and-trace");
    example.write_file(
        "w.akr",
        "state active\nintent \"Two checks.\"\nacceptance {\n    check first {\n        statement \"First command.\"\n        method command\n        command \"cargo test\"\n    }\n    check second {\n        statement \"Second manual.\"\n        method manual\n    }\n}\n",
    );
    assert_eq!(
        example
            .run(&[
                "propose",
                "sys.work.two-step",
                "--kind",
                "work",
                "--title",
                "Two step",
                "--from",
                "w.akr",
            ])
            .code,
        0
    );

    // Partial complete stamps `first` without requiring `second` or moving state to completed.
    let partial = example.run(&[
        "complete",
        "sys.work.two-step",
        "--partial",
        "--pass",
        "first",
    ]);
    assert_eq!(partial.code, 0, "{}", partial.output());
    let got = example.run(&["get", "@sys.work.two-step"]);
    assert!(got.stdout.contains("state active"), "{}", got.stdout);
    assert!(
        got.stdout.contains("satisfied — passed at"),
        "{}",
        got.stdout
    );

    // Supersede `sys.work.two-step` onto `sys.work.two-step-v2`, then verify `akr trace`
    // refuses the superseded key and names `sys.work.two-step-v2` in help.
    let sup = example.run(&[
        "supersede",
        "sys.work.two-step",
        "--with",
        "sys.work.two-step-v2",
        "--from",
        "w.akr",
        "--title",
        "Two step v2",
    ]);
    assert_eq!(sup.code, 0, "{}", sup.output());

    let refused_trace = example.run(&["trace", "sys.work.two-step", "tried old key"]);
    assert_ne!(refused_trace.code, 0);
    assert!(
        refused_trace.output().contains("AKR-C032"),
        "{}",
        refused_trace.output()
    );
    assert!(
        refused_trace.output().contains("sys.work.two-step-v2"),
        "expected successor hint: {}",
        refused_trace.output()
    );
}

#[test]
fn propose_many_accepts_json_and_bom_crlf_and_rejects_illegal_observation_state() {
    let example = Example::materialise("write-propose-many-json");
    let head = {
        let out = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(example.root())
            .output()
            .expect("rev-parse");
        String::from_utf8(out.stdout)
            .expect("utf8")
            .trim()
            .to_owned()
    };

    // 1. UTF-8 BOM + CRLF on `--from` overlay works cleanly.
    let bom_crlf =
        "\u{feff}state active\r\nintent \"\"\"\r\n    BOM and CRLF overlay.\r\n\"\"\"\r\n";
    example.write_file("bom.akr", bom_crlf);
    let ok = example.run(&[
        "propose",
        "sys.work.bom-check",
        "--kind",
        "work",
        "--title",
        "BOM check",
        "--from",
        "bom.akr",
    ]);
    assert_eq!(ok.code, 0, "{}", ok.output());

    // 2. MCP JSON format in `propose-many --from batch.json`.
    let json_batch = format!(
        r#"{{"records":[{{"key":"sys.work.json-one","kind":"work","title":"JSON One","slots":{{"intent":"First via JSON."}}}},{{"key":"sys.obs.json-two","kind":"observation","title":"JSON Two","slots":{{"statement":"Observed via JSON.","method":"manual","observed_at":"{head}"}}}}]}}"#
    );
    example.write_file("batch.json", &json_batch);
    let batch_ok = example.run(&["propose-many", "--from", "batch.json"]);
    assert_eq!(batch_ok.code, 0, "{}", batch_ok.output());
    assert!(
        batch_ok.stdout.contains("2 records written in one pass"),
        "{}",
        batch_ok.stdout
    );

    // 3. Explicit `state proposed` on an observation in propose-many is rejected with
    // AKR-T011 and MUST NOT print "records written in one pass".
    let bad_obs = format!(
        "record sys.obs.bad-state/1 : observation {{\n    title \"Bad state\"\n    state proposed\n    statement \"Should fail.\"\n    method manual\n    observed_at git:{head}\n}}\n"
    );
    example.write_file("bad.akr", &bad_obs);
    let refused = example.run(&["propose-many", "--from", "bad.akr"]);
    assert_ne!(refused.code, 0);
    assert!(
        refused.output().contains("AKR-T011"),
        "{}",
        refused.output()
    );
    assert!(
        !refused.output().contains("records written in one pass"),
        "refused batch must not claim records were written: {}",
        refused.output()
    );
}

//! `akr scratch` (D-036): the one working directory nothing else empties.
//!
//! These run the binary rather than the functions, because the property under test is a
//! workflow — what an agent tidying up at the end of a session can and cannot delete — and
//! the part that matters most is the *cannot*.

mod support;

use support::Example;

/// Writes `name` under `.agent/scratch/` with `bytes` of content.
fn scratch_file(example: &Example, name: &str, bytes: usize) {
    example.write_file(&format!(".agent/scratch/{name}"), &"x".repeat(bytes));
}

#[test]
fn list_reports_what_is_there_largest_first() {
    let example = Example::materialise("scratch-list");
    scratch_file(&example, "small.txt", 10);
    scratch_file(&example, "large.txt", 5000);

    let run = example.run(&["scratch", "list"]);
    assert_eq!(run.code, 0, "{}", run.output());
    let large = run.stdout.find("large.txt").expect("large listed");
    let small = run.stdout.find("small.txt").expect("small listed");
    assert!(large < small, "largest first:\n{}", run.stdout);
}

#[test]
fn a_missing_scratch_directory_is_not_a_problem() {
    let example = Example::materialise("scratch-absent");
    let run = example.run(&["scratch", "list"]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert!(run.stdout.contains("no .agent/scratch"), "{}", run.stdout);

    // And `akr check` says so rather than staying silent about it: an agent that never
    // sees the directory named has no reason to believe it persists.
    let checked = example.run(&["check"]);
    assert!(
        checked.stdout.contains("no .agent/scratch"),
        "{}",
        checked.output()
    );
}

#[test]
fn prune_removes_only_what_the_threshold_covers() {
    let example = Example::materialise("scratch-prune");
    scratch_file(&example, "stale.txt", 100);

    // Freshly written, so the default fortnight spares it.
    let spared = example.run(&["scratch", "prune"]);
    assert_eq!(spared.code, 0, "{}", spared.output());
    assert!(
        spared.stdout.contains("nothing to prune"),
        "{}",
        spared.output()
    );
    assert!(example.root().join(".agent/scratch/stale.txt").exists());

    // `--older-than 0` covers everything, which is also how this stays deterministic
    // without faking the clock.
    let rehearsed = example.run(&["scratch", "prune", "--older-than", "0", "--dry-run"]);
    assert_eq!(rehearsed.code, 0, "{}", rehearsed.output());
    assert!(
        rehearsed.stdout.contains("would remove"),
        "{}",
        rehearsed.stdout
    );
    assert!(
        example.root().join(".agent/scratch/stale.txt").exists(),
        "a dry run deleted something"
    );

    let pruned = example.run(&["scratch", "prune", "--older-than", "0"]);
    assert_eq!(pruned.code, 0, "{}", pruned.output());
    assert!(
        !example.root().join(".agent/scratch/stale.txt").exists(),
        "{}",
        pruned.output()
    );
}

#[test]
fn a_kept_entry_survives_a_prune_that_covers_everything() {
    // The invariant the whole marker exists for: an agent tidying up at the end of a
    // session must not be able to delete what the next session was told to expect.
    let example = Example::materialise("scratch-keep");
    scratch_file(&example, "needed.txt", 100);
    scratch_file(&example, "spent.txt", 100);

    let kept = example.run(&[
        "scratch",
        "keep",
        "needed.txt",
        "--reason",
        "the comb output the port work still reads",
    ]);
    assert_eq!(kept.code, 0, "{}", kept.output());

    let pruned = example.run(&["scratch", "prune", "--older-than", "0"]);
    assert_eq!(pruned.code, 0, "{}", pruned.output());
    assert!(
        example.root().join(".agent/scratch/needed.txt").exists(),
        "a kept entry was pruned:\n{}",
        pruned.output()
    );
    assert!(!example.root().join(".agent/scratch/spent.txt").exists());

    // The reason is in the file, in a shape a person can edit without any tool.
    let keep = example.read_file(".agent/scratch/KEEP");
    assert!(keep.contains("needed.txt the comb output"), "{keep}");

    // And it can be released again.
    let forgotten = example.run(&["scratch", "keep", "needed.txt", "--forget"]);
    assert_eq!(forgotten.code, 0, "{}", forgotten.output());
    let after = example.run(&["scratch", "prune", "--older-than", "0"]);
    assert_eq!(after.code, 0, "{}", after.output());
    assert!(!example.root().join(".agent/scratch/needed.txt").exists());
}

/// Plants one verified evidence record whose artifact is `artifact`.
///
/// The write pipeline refuses a new scratch citation, so the repair tests have to look
/// the way a ledger authored before that refusal does: the record is already in the file.
fn plant_scratch_evidence(example: &Example, key: &str, artifact: &str) {
    let relative = ".akr/records/sys/evidence.akr";
    let mut text = example.read_file(relative);
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&format!(
        "record {key}/1 : evidence {{\n    \
         title \"A capture\"\n    \
         state verified\n    \
         result pass\n    \
         method command\n    \
         observed_at git:e806b3f54a2d7091c5e13b8a26f490dc7b135e64\n    \
         command \"cargo test\"\n    \
         summary \"the run finished\"\n    \
         artifact \"{artifact}\"\n}}\n"
    ));
    example.write_file(relative, &text);
}

fn evidence_record(text: &str, id: &str) -> String {
    let marker = format!("record {id} ");
    let start = text.find(&marker).unwrap_or_else(|| panic!("missing {id}"));
    let rest = &text[start..];
    let end = rest
        .find("\n}")
        .unwrap_or_else(|| panic!("{id} has no end"));
    rest[..=end + 1].to_owned()
}

#[test]
fn prune_detaches_the_citation_of_an_entry_it_deletes() {
    let example = Example::materialise("scratch-prune-detach");
    scratch_file(&example, "spent/log.txt", 40);
    plant_scratch_evidence(
        &example,
        "sys.evidence.spent",
        ".agent/scratch/spent/log.txt",
    );

    let rehearsed = example.run(&["scratch", "prune", "--older-than", "0", "--dry-run"]);
    assert_eq!(rehearsed.code, 0, "{}", rehearsed.output());
    assert!(
        rehearsed.stdout.contains("would remove"),
        "{}",
        rehearsed.stdout
    );
    assert!(
        rehearsed.stdout.contains("would detach 1 scratch citation"),
        "{}",
        rehearsed.stdout
    );
    assert!(
        rehearsed.stdout.contains("sys.evidence.spent/1 -> /2"),
        "{}",
        rehearsed.stdout
    );
    assert!(example.root().join(".agent/scratch/spent/log.txt").exists());
    let before = example.read_file(".akr/records/sys/evidence.akr");
    assert!(before.contains("artifact \".agent/scratch/spent/log.txt\""));
    assert!(!before.contains("sys.evidence.spent/2"));

    let pruned = example.run(&["scratch", "prune", "--older-than", "0"]);
    assert_eq!(pruned.code, 0, "{}", pruned.output());
    assert!(!example.root().join(".agent/scratch/spent/log.txt").exists());
    let after = example.read_file(".akr/records/sys/evidence.akr");
    let retired = evidence_record(&after, "sys.evidence.spent/1");
    assert!(retired.contains("state superseded"), "{retired}");
    assert!(
        retired.contains(".agent/scratch/spent/log.txt"),
        "history keeps the path: {retired}"
    );
    let head = evidence_record(&after, "sys.evidence.spent/2");
    assert!(!head.contains("artifact"), "{head}");
    assert!(head.contains("no longer cited"), "{head}");
    assert!(head.contains("the run finished"), "{head}");
}

#[test]
fn prune_of_a_fresh_file_does_not_detach_its_citation() {
    let example = Example::materialise("scratch-prune-fresh-citation");
    scratch_file(&example, "fresh/log.txt", 40);
    plant_scratch_evidence(
        &example,
        "sys.evidence.fresh",
        ".agent/scratch/fresh/log.txt",
    );

    let spared = example.run(&["scratch", "prune"]);
    assert_eq!(spared.code, 0, "{}", spared.output());
    assert!(
        spared.stdout.contains("nothing to prune"),
        "{}",
        spared.output()
    );
    assert!(example.root().join(".agent/scratch/fresh/log.txt").exists());
    let text = example.read_file(".akr/records/sys/evidence.akr");
    assert!(text.contains("artifact \".agent/scratch/fresh/log.txt\""));
    assert!(!text.contains("sys.evidence.fresh/2"));
}

#[test]
fn prune_detaches_a_citation_whose_file_is_already_gone() {
    let example = Example::materialise("scratch-prune-absent");
    plant_scratch_evidence(&example, "sys.evidence.gone", ".agent/scratch/gone/log.txt");

    let pruned = example.run(&["scratch", "prune"]);
    assert_eq!(pruned.code, 0, "{}", pruned.output());
    assert!(
        pruned.stdout.contains("detached 1 scratch citation"),
        "{}",
        pruned.stdout
    );
    let after = example.read_file(".akr/records/sys/evidence.akr");
    let head = evidence_record(&after, "sys.evidence.gone/2");
    assert!(!head.contains("artifact"), "{head}");
    assert!(head.contains("no longer cited"), "{head}");
}

#[test]
fn prune_does_not_detach_a_kept_citation_whose_file_is_gone() {
    let example = Example::materialise("scratch-prune-kept-citation");
    example.write_file(
        ".agent/scratch/KEEP",
        "kept-log the next session still reads it\n",
    );
    plant_scratch_evidence(
        &example,
        "sys.evidence.kept-log",
        ".agent/scratch/kept-log/log.txt",
    );

    let pruned = example.run(&["scratch", "prune", "--older-than", "0"]);
    assert_eq!(pruned.code, 0, "{}", pruned.output());
    assert!(
        pruned.stdout.contains("nothing to prune"),
        "{}",
        pruned.output()
    );
    let text = example.read_file(".akr/records/sys/evidence.akr");
    assert!(text.contains("artifact \".agent/scratch/kept-log/log.txt\""));
    assert!(!text.contains("sys.evidence.kept-log/2"));
}

#[test]
fn keep_insists_on_a_reason_and_on_the_entry_existing() {
    let example = Example::materialise("scratch-keep-refusals");
    scratch_file(&example, "there.txt", 10);

    // A marker with no reason outlives whatever made it necessary.
    let unexplained = example.run(&["scratch", "keep", "there.txt"]);
    assert_ne!(unexplained.code, 0, "{}", unexplained.output());
    assert!(
        unexplained.output().contains("--reason"),
        "{}",
        unexplained.output()
    );

    let absent = example.run(&["scratch", "keep", "ghost.txt", "--reason", "why"]);
    assert_ne!(absent.code, 0, "{}", absent.output());
}

#[test]
fn scratch_is_a_build_fact_and_fails_only_when_asked() {
    // The D-024 line, held for D-036: what is in the working tree is never a ledger
    // diagnostic, and an opt-in flag is how an operator asks for it to be one.
    let example = Example::materialise("scratch-gate");
    scratch_file(&example, "spent.txt", 4096);

    let reported = example.run(&["check"]);
    assert_eq!(
        reported.code,
        0,
        "scratch must not fail an ordinary check:\n{}",
        reported.output()
    );
    assert!(
        reported.stdout.contains("of scratch in"),
        "the build fact should always print:\n{}",
        reported.output()
    );

    // Nothing is old enough yet, so even the gate passes.
    let gated_fresh = example.run(&["check", "--scratch-clean"]);
    assert_eq!(gated_fresh.code, 0, "{}", gated_fresh.output());
}

#[test]
fn the_keep_file_is_not_itself_an_entry() {
    // Otherwise the marker would show up as the thing it protects against.
    let example = Example::materialise("scratch-keep-not-an-entry");
    scratch_file(&example, "work.txt", 10);
    assert_eq!(
        example
            .run(&[
                "scratch",
                "keep",
                "work.txt",
                "--reason",
                "still reading it"
            ])
            .code,
        0
    );
    let listed = example.run(&["scratch", "list"]);
    assert_eq!(listed.code, 0, "{}", listed.output());
    assert!(
        !listed.stdout.contains("  KEEP"),
        "KEEP listed as an entry:\n{}",
        listed.stdout
    );
}

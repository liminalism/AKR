//! The AKR ↔ git bridge, end to end through the binary.
//!
//! `docs/16-change-protocol.md`. The claims worth testing are all about *refusal*: what
//! the bridge declines to do is what makes it worth having. A tool that cheerfully
//! commits a code change with no work reference, or generates a message from a tree that
//! has since moved, would leave the ledger and the history disagreeing in exactly the way
//! the protocol exists to prevent.

mod support;

use support::Example;

/// Commits the materialised workspace, so `HEAD` and the index both hold the ledger.
///
/// The harness lays the example's `.akr/` on top of a replayed history without committing
/// it. That is right for the transcripts, and wrong here: a change protocol whose base
/// tree has no ledger in it would report every record as new.
fn baseline(example: &Example) {
    example.git(&["add", "-A"]);
    example.git(&["commit", "-m", "baseline"]);
}

/// Stages one implementation file and returns its path.
fn stage_code(example: &Example) -> &'static str {
    baseline(example);
    example.write_file("src/tone.rs", "// gate highlight chroma\n");
    example.git(&["add", "src/tone.rs"]);
    "src/tone.rs"
}

#[test]
fn diff_staged_reports_semantic_changes_not_textual_ones() {
    let example = Example::materialise("change-diff-staged");
    baseline(&example);
    let papercut = example.run(&[
        "papercut",
        "-m",
        "tester",
        "a friction worth recording",
        "--namespace",
        "sys",
    ]);
    assert_eq!(papercut.code, 0, "{}", papercut.output());
    example.git(&["add", ".akr"]);

    let run = example.run(&["diff", "--staged"]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert!(run.stdout.contains("records added"), "{}", run.output());
    assert!(run.stdout.contains("papercut"), "{}", run.output());
}

#[test]
fn a_reformat_is_not_a_semantic_change() {
    let example = Example::materialise("change-diff-reformat");
    baseline(&example);
    // Blank lines between records are formatting. A textual diff would call this a
    // change; a semantic one must not, which is the reason the delta parses both trees
    // instead of reading `git diff`.
    let path = ".akr/records/sys/policies.akr";
    let text = example.read_file(path);
    example.write_file(path, &format!("{text}\n\n"));
    example.git(&["add", path]);

    let run = example.run(&["diff", "--staged"]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert!(
        !run.stdout.contains("records added") && !run.stdout.contains("state transitions"),
        "a whitespace-only edit should move nothing:\n{}",
        run.output()
    );
}

#[test]
fn diff_staged_needs_the_flag() {
    let example = Example::materialise("change-diff-needs-flag");
    let run = example.run(&["diff"]);
    assert_eq!(run.code, 2, "{}", run.output());
    assert!(run.output().contains("--staged"), "{}", run.output());
}

#[test]
fn a_transaction_opens_shows_and_aborts() {
    let example = Example::materialise("change-transaction-lifecycle");
    let begin = example.run(&[
        "change",
        "begin",
        "--kind",
        "fix",
        "--scope",
        "tone",
        "--summary",
        "gate reconstructed highlight chroma",
    ]);
    assert_eq!(begin.code, 0, "{}", begin.output());
    assert!(
        begin.stdout.contains("opened change chg-"),
        "{}",
        begin.output()
    );

    let show = example.run(&["change", "show"]);
    assert!(
        show.stdout.contains("gate reconstructed highlight chroma"),
        "{}",
        show.output()
    );
    assert!(show.stdout.contains("prepared no"), "{}", show.output());
    let status = example.run(&["change", "status"]);
    assert_eq!(status.stdout, show.stdout, "{}", status.output());

    // Twice is a mistake, not an overwrite: silently replacing an open transaction would
    // discard whatever the previous one recorded.
    let again = example.run(&[
        "change",
        "begin",
        "--kind",
        "fix",
        "--summary",
        "something else",
    ]);
    assert_ne!(again.code, 0, "{}", again.output());
    assert!(
        again.output().contains("already open"),
        "{}",
        again.output()
    );

    let abort = example.run(&["change", "abort"]);
    assert_eq!(abort.code, 0, "{}", abort.output());
    assert!(
        example
            .run(&["change", "show"])
            .stdout
            .contains("no change transaction")
    );
}

#[test]
fn the_transaction_lives_in_the_git_directory_and_not_the_ledger() {
    let example = Example::materialise("change-transaction-location");
    example.run(&[
        "change",
        "begin",
        "--kind",
        "chore",
        "--summary",
        "a change",
    ]);
    assert!(
        example.root().join(".git/akr/current-change.akr").exists(),
        "the transaction belongs to the worktree, not the ledger"
    );
    // It must not be visible to AKR at all: it is scaffolding, and a record kind it is not.
    let search = example.run(&["search", "chg"]);
    assert!(
        !search.stdout.contains("current-change"),
        "{}",
        search.output()
    );
    let status = example.run(&["check"]);
    assert!(
        !status.output().contains("current-change"),
        "{}",
        status.output()
    );
}

#[test]
fn preparing_a_code_change_with_no_work_reference_is_refused() {
    let example = Example::materialise("change-untracked-refused");
    stage_code(&example);
    example.run(&[
        "change",
        "begin",
        "--kind",
        "fix",
        "--summary",
        "gate chroma",
    ]);

    let prepare = example.run(&["change", "prepare", "--staged"]);
    assert_ne!(prepare.code, 0, "{}", prepare.output());
    assert!(
        prepare.output().contains("names no work record"),
        "{}",
        prepare.output()
    );
}

#[test]
fn an_explicit_exemption_lets_maintenance_through() {
    let example = Example::materialise("change-untracked-exempt");
    stage_code(&example);
    example.run(&[
        "change",
        "begin",
        "--kind",
        "chore",
        "--scope",
        "ci",
        "--summary",
        "pin the build image",
        "--untracked-reason",
        "repository maintenance; no project behaviour changed",
    ]);
    let prepare = example.run(&["change", "prepare", "--staged"]);
    assert_eq!(prepare.code, 0, "{}", prepare.output());

    let message = example.run(&["git", "message"]);
    assert_eq!(message.code, 0, "{}", message.output());
    assert!(
        message.stdout.starts_with("chore(ci): pin the build image"),
        "{}",
        message.output()
    );
    assert!(
        message
            .stdout
            .contains("No AKR work record: repository maintenance"),
        "{}",
        message.output()
    );
    assert!(
        !message.stdout.contains("AKR-Work:"),
        "{}",
        message.output()
    );
}

#[test]
fn a_message_carries_the_trailers_that_are_the_durable_link() {
    let example = Example::materialise("change-message-trailers");
    stage_code(&example);
    example.run(&[
        "change",
        "begin",
        "--kind",
        "fix",
        "--scope",
        "tone",
        "--summary",
        "gate reconstructed highlight chroma by uncertainty",
        "--primary",
        "@sys.work.m3-plan/2",
    ]);
    let prepare = example.run(&["change", "prepare", "--staged"]);
    assert_eq!(prepare.code, 0, "{}", prepare.output());

    let message = example.run(&["git", "message"]);
    assert_eq!(message.code, 0, "{}", message.output());
    assert!(
        message.stdout.contains("AKR-Change: chg-"),
        "{}",
        message.output()
    );
    assert!(
        message.stdout.contains("AKR-Work: @sys.work.m3-plan/2"),
        "{}",
        message.output()
    );
    assert!(
        message.stdout.contains("AKR-Tree: "),
        "{}",
        message.output()
    );
    // The subject line is what shows in `git log --oneline`; it has to stay short.
    let subject = message.stdout.lines().next().expect("a subject");
    assert!(
        subject.len() <= 72,
        "{subject:?} is {} chars",
        subject.len()
    );
}

#[test]
fn a_message_is_byte_identical_across_runs() {
    let example = Example::materialise("change-message-deterministic");
    stage_code(&example);
    example.run(&[
        "change",
        "begin",
        "--kind",
        "fix",
        "--summary",
        "gate chroma",
        "--primary",
        "@sys.work.m3-plan/2",
    ]);
    example.run(&["change", "prepare", "--staged"]);
    let one = example.run(&["git", "message"]);
    let two = example.run(&["git", "message"]);
    assert_eq!(one.stdout, two.stdout);
}

#[test]
fn a_message_needs_a_prepared_transaction() {
    let example = Example::materialise("change-message-unprepared");
    stage_code(&example);
    example.run(&[
        "change",
        "begin",
        "--kind",
        "fix",
        "--summary",
        "gate chroma",
        "--primary",
        "@sys.work.m3-plan/2",
    ]);
    let message = example.run(&["git", "message"]);
    assert_ne!(message.code, 0, "{}", message.output());
    assert!(
        message.output().contains("has not been prepared"),
        "{}",
        message.output()
    );
}

#[test]
fn a_staged_tree_that_moved_invalidates_the_preparation() {
    let example = Example::materialise("change-tree-moved");
    stage_code(&example);
    example.run(&[
        "change",
        "begin",
        "--kind",
        "fix",
        "--summary",
        "gate chroma",
        "--primary",
        "@sys.work.m3-plan/2",
    ]);
    assert_eq!(example.run(&["change", "prepare", "--staged"]).code, 0);

    // Somebody staged another file after the preparation. The message would describe a
    // commit that is no longer the one about to be made.
    example.write_file("src/other.rs", "// something else\n");
    example.git(&["add", "src/other.rs"]);

    let message = example.run(&["git", "message"]);
    assert_ne!(message.code, 0, "{}", message.output());
    assert!(
        message.output().contains("staged tree moved"),
        "{}",
        message.output()
    );
}

#[test]
fn committing_through_the_bridge_leaves_the_trailers_in_history() {
    let example = Example::materialise("change-commit");
    stage_code(&example);
    example.run(&[
        "change",
        "begin",
        "--kind",
        "fix",
        "--scope",
        "tone",
        "--summary",
        "gate reconstructed highlight chroma",
        "--primary",
        "@sys.work.m3-plan/2",
    ]);
    assert_eq!(example.run(&["change", "prepare", "--staged"]).code, 0);

    let commit = example.run(&["git", "commit"]);
    assert_eq!(commit.code, 0, "{}", commit.output());

    let log = example.run(&["git", "log", "sys.work.m3-plan"]);
    assert_eq!(log.code, 0, "{}", log.output());
    assert!(
        log.stdout.contains("gate reconstructed highlight chroma"),
        "the commit must be findable from the record it advanced:\n{}",
        log.output()
    );
    // The transaction is scaffolding and goes; the trailers are what remain.
    assert!(
        example
            .run(&["change", "show"])
            .stdout
            .contains("no change transaction"),
        "a committed transaction should be cleared"
    );
    let check = example.run(&["check", "--views-current"]);
    assert_eq!(
        check.code,
        0,
        "a clean commit must not invalidate generated views:\n{}",
        check.output()
    );
}

#[test]
fn evidence_and_completed_work_can_land_in_the_same_commit() {
    let example = Example::materialise("change-co-committed-evidence");
    baseline(&example);
    example.write_file(
        "work.txt",
        r#"title "Verify the prepared change"
scope [ path "src/**" ]
intent "Prove evidence authored for a prepared tree remains current after Git advances."
acceptance {
    check prepared-tree {
        statement "The prepared implementation passed its command check."
        method command
        command "true"
    }
}
"#,
    );
    let proposed = example.run(&[
        "propose",
        "sys.work.prepared-tree",
        "--kind",
        "work",
        "--from",
        "work.txt",
    ]);
    assert_eq!(proposed.code, 0, "{}", proposed.output());
    let evidence = example.run(&[
        "evidence",
        "add",
        "sys.evidence.prepared-tree",
        "--result",
        "pass",
        "--method",
        "command",
        "--command",
        "true",
        "--summary",
        "The prepared tree passed.",
    ]);
    assert_eq!(evidence.code, 0, "{}", evidence.output());
    assert_eq!(
        example
            .run(&[
                "revise",
                "sys.work.prepared-tree",
                "--state",
                "ready",
                "--in-place",
            ])
            .code,
        0
    );
    assert_eq!(
        example
            .run(&["revise", "sys.work.prepared-tree", "--state", "active"])
            .code,
        0
    );
    let completed = example.run(&[
        "complete",
        "sys.work.prepared-tree",
        "--check",
        "prepared-tree=@sys.evidence.prepared-tree/1",
    ]);
    assert_eq!(completed.code, 0, "{}", completed.output());
    assert_eq!(example.run(&["build"]).code, 0);

    example.git(&["add", "-A"]);
    assert_eq!(
        example
            .run(&[
                "change",
                "begin",
                "--kind",
                "test",
                "--summary",
                "verify co-committed evidence",
                "--primary",
                "@sys.work.prepared-tree/2",
            ])
            .code,
        0
    );
    assert_eq!(example.run(&["change", "prepare", "--staged"]).code, 0);
    let commit = example.run(&["git", "commit"]);
    assert_eq!(commit.code, 0, "{}", commit.output());

    let check = example.run(&["check", "--views-current"]);
    assert_eq!(check.code, 0, "{}", check.output());
}

#[test]
fn hooks_are_thin_wrappers_around_the_binary() {
    let example = Example::materialise("change-hooks");
    let install = example.run(&["git", "install-hooks"]);
    assert_eq!(install.code, 0, "{}", install.output());
    let hook = example.read_file(".git/hooks/pre-commit");
    assert!(hook.contains("akr git-hook pre-commit"), "{hook}");
    // A hook that carried the checks itself would be a second implementation nobody
    // keeps in step with the first.
    assert!(hook.lines().count() <= 3, "{hook}");
}

#[test]
fn a_message_only_amend_passes_the_pre_commit_hook() {
    // The transaction closes at `akr git commit`, so every later commit in the worktree
    // meets a hook with no transaction to check — including `git commit --amend` that
    // only rewords a subject, which leaves the tree untouched and the AKR trailers
    // byte-identical. Refusing that made `--no-verify` the only way to fix a commit
    // subject, which is a bad habit to teach for a change the protocol has no opinion
    // about.
    let example = Example::materialise("change-hook-reword");
    let reword = example.run(&["git-hook", "pre-commit"]);
    assert_eq!(reword.code, 0, "{}", reword.output());
    assert!(reword.stdout.contains("reword"), "{}", reword.stdout);

    // An ordinary commit with no transaction is still refused: the exemption is the empty
    // index against HEAD, not the absent transaction.
    example.write_file("src/new.rs", "fn main() {}\n");
    example.git(&["add", "src/new.rs"]);
    let staged = example.run(&["git-hook", "pre-commit"]);
    assert!(
        staged.output().contains("AKR-C031"),
        "a staged change with no transaction is still refused: {}",
        staged.output()
    );
}

#[test]
fn a_ledger_only_change_is_not_a_code_change() {
    // `git ls-files --stage` lists the whole index, and reading it as "what is staged"
    // made a commit that writes only records claim every tracked file as its own and then
    // refuse itself for touching implementation it had never touched
    // (`akr.papercut.akr-change-prepare-staged-reports-code-n-files`).
    let example = Example::materialise("change-ledger-only");
    baseline(&example);
    let papercut = example.run(&[
        "papercut",
        "-m",
        "tester",
        "a friction worth recording",
        "--namespace",
        "sys",
    ]);
    assert_eq!(papercut.code, 0, "{}", papercut.output());
    example.git(&["add", ".akr"]);
    example.run(&["change", "begin", "--kind", "docs", "--summary", "log one"]);

    let prepare = example.run(&["change", "prepare", "--staged"]);
    assert_eq!(prepare.code, 0, "{}", prepare.output());
    assert!(
        prepare.output().contains("code             0 files"),
        "a change that writes only records touches no code: {}",
        prepare.output()
    );
    assert!(
        !prepare.output().contains("names no work record"),
        "and is not asked for a work record it does not need: {}",
        prepare.output()
    );
}

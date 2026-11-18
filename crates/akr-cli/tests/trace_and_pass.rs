//! AKR 0.7 end to end through the binary: the reasoning trace, a check that records its
//! own pass, retired kinds, and the evidence title (D-046, D-047, D-048, D-049).
//!
//! `docs/19-ledger-usage-and-reasoning-audit.md` measured what these replace: work
//! records that restate a diff, an evidence record per passing test run with its title
//! and summary identical, and trailers that named no work. Each test here is one of those
//! findings turned into a claim about the tool.

use crate::support;

use support::Example;

const WORK: &str = "lege.work.extract-render-graph";

fn baseline(example: &Example) {
    example.git(&["add", "-A"]);
    example.git(&["commit", "-m", "baseline"]);
}

fn head(example: &Example) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(example.root())
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("git runs");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// A work record with one command check, moved to `active`.
fn active_work_with_a_command_check(example: &Example, key: &str) {
    example.write_file(
        "work-body.akr",
        "intent \"\"\"\n    Keep the cache warm across restarts, because a cold start costs four seconds.\n\"\"\"\n\
         acceptance {\n    check suite {\n        statement \"The cache tests pass.\"\n        method command\n        command \"cargo test -p cache\"\n    }\n}\n",
    );
    let propose = example.run(&[
        "propose",
        key,
        "--kind",
        "work",
        "--title",
        "Warm cache",
        "--from",
        "work-body.akr",
    ]);
    assert_eq!(propose.code, 0, "{}", propose.output());
    for state in ["ready", "active"] {
        let revise = example.run(&["revise", key, "--state", state]);
        assert_eq!(revise.code, 0, "{}", revise.output());
    }
}

// -------------------------------------------------------------------------------------
// the trace
// -------------------------------------------------------------------------------------

#[test]
fn a_trace_entry_lands_in_place_and_get_shows_it() {
    let example = Example::materialise("trace-in-place");
    baseline(&example);
    let before = example.run(&["get", WORK, "--history"]);

    let trace = example.run(&[
        "trace",
        WORK,
        "split the graph out first: the viewer cannot be tested while it owns the passes",
    ]);
    assert_eq!(trace.code, 0, "{}", trace.output());
    assert!(trace.stdout.contains("edited"), "{}", trace.output());

    let after = example.run(&["get", WORK, "--history"]);
    assert!(
        after.stdout.contains("split the graph out first"),
        "{}",
        after.output()
    );
    assert!(
        after.stdout.contains("(new)"),
        "an uncommitted entry has no commit yet:\n{}",
        after.output()
    );
    // No revision was minted: the history block is what it was.
    let history = |text: &str| {
        text.split("\n  history\n")
            .nth(1)
            .unwrap_or_default()
            .to_owned()
    };
    assert_eq!(history(&before.stdout), history(&after.stdout));
}

#[test]
fn a_trace_entry_over_forty_words_is_refused_and_one_over_twenty_five_is_noted() {
    let example = Example::materialise("trace-bounds");
    let long = vec!["word"; 41].join(" ");
    let refused = example.run(&["trace", WORK, &long]);
    assert_eq!(refused.code, 1, "{}", refused.output());
    assert!(
        refused.output().contains("AKR-T024"),
        "{}",
        refused.output()
    );

    let wordy = vec!["word"; 30].join(" ");
    let noted = example.run(&["trace", WORK, &wordy]);
    assert_eq!(noted.code, 0, "{}", noted.output());
    assert!(noted.stdout.contains("30 words"), "{}", noted.output());

    let wrong_kind = example.run(&["trace", "sys.policy.tandem-work", "a reason"]);
    assert_eq!(wrong_kind.code, 1, "{}", wrong_kind.output());
    assert!(
        wrong_kind
            .output()
            .contains("only a work record carries a trace"),
        "{}",
        wrong_kind.output()
    );
}

#[test]
fn a_commit_carries_the_trace_it_adds_and_get_names_the_commit() {
    let example = Example::materialise("trace-commit");
    baseline(&example);

    let trace = example.run(&[
        "trace",
        WORK,
        "split the graph out first: the viewer cannot be tested while it owns the passes",
    ]);
    assert_eq!(trace.code, 0, "{}", trace.output());

    // A bare key, as an agent types it. `--why` is `akr trace` on the primary record.
    let begin = example.run(&[
        "change",
        "begin",
        "--kind",
        "refactor",
        "--summary",
        "extract the render graph",
        "--primary",
        WORK,
        "--why",
        "dropped the trait object: one implementor, and it hid the borrow",
    ]);
    assert_eq!(begin.code, 0, "{}", begin.output());
    assert!(begin.stdout.contains("traced"), "{}", begin.output());

    example.write_file("lege/src/render/graph.rs", "// the graph\n");
    example.git(&["add", "-A"]);
    let prepare = example.run(&["change", "prepare", "--staged"]);
    assert_eq!(prepare.code, 0, "{}", prepare.output());
    assert!(
        prepare.stdout.contains("2 trace entries"),
        "{}",
        prepare.output()
    );

    let message = example.run(&["git", "message"]);
    assert_eq!(message.code, 0, "{}", message.output());
    assert!(
        message.stdout.contains(&format!("AKR-Work: @{WORK}\n")),
        "the trailer is in the form the readers search for:\n{}",
        message.stdout
    );
    let why: Vec<&str> = message
        .stdout
        .lines()
        .filter_map(|line| line.strip_prefix("AKR-Why: "))
        .collect();
    assert_eq!(
        why,
        [
            "split the graph out first: the viewer cannot be tested while it owns the passes",
            "dropped the trait object: one implementor, and it hid the borrow",
        ],
        "{}",
        message.stdout
    );
    // The work record's intent is in the ledger. It is not copied into git as well.
    let intent = example.run(&["get", WORK]);
    let first_intent_line = intent
        .stdout
        .split("\n  intent\n")
        .nth(1)
        .and_then(|rest| rest.lines().next())
        .map(str::trim)
        .unwrap_or_default()
        .to_owned();
    assert!(!first_intent_line.is_empty(), "{}", intent.output());
    assert!(
        !message.stdout.contains(&first_intent_line),
        "the message restates the intent:\n{}",
        message.stdout
    );

    let commit = example.run(&["git", "commit"]);
    assert_eq!(commit.code, 0, "{}", commit.output());
    let landed = head(&example);

    // The link runs from the commit to the record, and back again by derivation: `get`
    // names the commit that introduced each entry without the ledger storing a hash.
    let get = example.run(&["get", WORK]);
    let trace_block = get
        .stdout
        .split("\n  trace\n")
        .nth(1)
        .unwrap_or_default()
        .to_owned();
    let lines: Vec<&str> = trace_block.lines().take(2).collect();
    assert_eq!(lines.len(), 2, "{}", get.output());
    for line in &lines {
        assert!(
            line.trim_start().starts_with(&landed[..8]),
            "each entry names {}:\n{}",
            &landed[..8],
            get.output()
        );
    }
    assert!(
        !example
            .read_file(".akr/records/lege/work.akr")
            .contains(&landed[..8]),
        "no commit id is written into the ledger (D-032)"
    );

    let log = example.run(&["git", "log", WORK]);
    assert!(log.stdout.contains(&landed[..8]), "{}", log.output());
}

#[test]
fn a_transaction_names_real_work_or_is_refused() {
    let example = Example::materialise("change-names-work");
    baseline(&example);

    // An unknown flag used to be ignored, so a misspelling opened a transaction that
    // named nothing.
    let typo = example.run(&["change", "begin", "--summary", "x", "--primry", WORK]);
    assert_eq!(typo.code, 2, "{}", typo.output());
    assert!(typo.output().contains("--primry"), "{}", typo.output());

    // A reason needs a record to live on.
    let orphan = example.run(&["change", "begin", "--summary", "x", "--why", "because"]);
    assert_ne!(orphan.code, 0, "{}", orphan.output());
    assert!(orphan.output().contains("--primary"), "{}", orphan.output());

    // A policy is not work. Every AKR-Work trailer in one audited project named a
    // record of another kind.
    let begin = example.run(&[
        "change",
        "begin",
        "--kind",
        "fix",
        "--summary",
        "tighten the rule",
        "--primary",
        "sys.policy.tandem-work",
    ]);
    assert_eq!(begin.code, 0, "{}", begin.output());
    example.write_file("src/rule.rs", "// rule\n");
    example.git(&["add", "-A"]);
    let prepare = example.run(&["change", "prepare", "--staged"]);
    assert_ne!(prepare.code, 0, "{}", prepare.output());
    assert!(
        prepare.output().contains("not a work record"),
        "{}",
        prepare.output()
    );
    example.run(&["change", "abort"]);

    let begin = example.run(&[
        "change",
        "begin",
        "--summary",
        "x",
        "--primary",
        "sys.work.no-such-thing",
    ]);
    assert_eq!(begin.code, 0, "{}", begin.output());
    let prepare = example.run(&["change", "prepare", "--staged"]);
    assert_ne!(prepare.code, 0, "{}", prepare.output());
    assert!(
        prepare.output().contains("does not resolve"),
        "{}",
        prepare.output()
    );
}

// -------------------------------------------------------------------------------------
// a check that records its own pass
// -------------------------------------------------------------------------------------

#[test]
fn a_command_check_completes_without_an_evidence_record() {
    let example = Example::materialise("pass-no-evidence");
    baseline(&example);
    let key = "sys.work.warm-cache";
    active_work_with_a_command_check(&example, key);
    let evidence_before = example.read_file(".akr/records/sys/evidence.akr");

    let bare = example.run(&["complete", key]);
    assert_eq!(bare.code, 1, "{}", bare.output());
    assert!(bare.output().contains("--pass"), "{}", bare.output());

    let complete = example.run(&["complete", key, "--pass", "suite"]);
    assert_eq!(complete.code, 0, "{}", complete.output());
    assert!(
        complete.stdout.contains("empty trace"),
        "completion without a reason is noted, never refused:\n{}",
        complete.output()
    );

    assert_eq!(
        example.read_file(".akr/records/sys/evidence.akr"),
        evidence_before,
        "a routine pass mints no evidence record"
    );
    let record = example.read_file(".akr/records/sys/work.akr");
    assert!(
        record.contains(&format!("passed_at git:{}", head(&example))),
        "{record}"
    );

    let get = example.run(&["get", key]);
    assert!(
        get.stdout.contains("satisfied — passed at"),
        "{}",
        get.output()
    );

    // And the ledger is sound once the lock is rebuilt: an in-place completion is lock
    // drift, never a broken seal.
    let build = example.run(&["build"]);
    assert_eq!(build.code, 0, "{}", build.output());
    let check = example.run(&["check"]);
    assert!(
        !check.output().contains("AKR-R022") && !check.output().contains("AKR-R051"),
        "{}",
        check.output()
    );
}

#[test]
fn a_pass_is_refused_for_a_check_that_is_not_a_command() {
    let example = Example::materialise("pass-not-command");
    let refused = example.run(&[
        "complete",
        "sys.milestone.m3-playable-day",
        "--pass",
        "full-day-demo",
    ]);
    assert_eq!(refused.code, 1, "{}", refused.output());
    assert!(
        refused.output().contains("only a `command` check"),
        "{}",
        refused.output()
    );
}

// -------------------------------------------------------------------------------------
// retired kinds, and the evidence title
// -------------------------------------------------------------------------------------

#[test]
fn a_retired_kind_takes_no_new_key_and_explain_says_where_it_went() {
    let example = Example::materialise("retired-kinds");
    for (kind, into) in [
        ("term", "policy"),
        ("requirement", "policy"),
        ("constraint", "policy"),
        ("assessment", "observation"),
        ("track", "milestone"),
    ] {
        let key = format!("sys.{kind}.brand-new");
        let refused = example.run(&["propose", &key, "--kind", kind, "--title", "New"]);
        assert_eq!(refused.code, 1, "{kind}: {}", refused.output());
        assert!(
            refused.output().contains("AKR-T008"),
            "{kind}: {}",
            refused.output()
        );
        assert!(
            refused.output().contains(&format!("`{into}`")),
            "{kind}: {}",
            refused.output()
        );

        let explain = example.run(&["explain", kind]);
        assert!(explain.stdout.contains("retired"), "{}", explain.output());
        assert!(explain.stdout.contains(into), "{}", explain.output());
    }

    // The records that exist are untouched: the example ledger still checks.
    let check = example.run(&["check"]);
    assert!(
        check.stdout.contains("live record(s) of retired kinds"),
        "{}",
        check.output()
    );
    assert!(!check.output().contains("AKR-T008"), "{}", check.output());
}

#[test]
fn explain_says_what_each_slot_is_for() {
    let example = Example::materialise("explain-guidance");
    let work = example.run(&["explain", "work"]);
    assert!(work.stdout.contains("purpose"), "{}", work.output());
    assert!(
        work.stdout.contains("Not a list of files or functions"),
        "{}",
        work.output()
    );
    assert!(work.stdout.contains("akr trace"), "{}", work.output());

    let decision = example.run(&["explain", "decision"]);
    assert!(
        decision.stdout.contains("alternatives rejected"),
        "{}",
        decision.output()
    );
}

#[test]
fn evidence_does_not_store_its_summary_twice() {
    let example = Example::materialise("evidence-title");

    // A short summary and no title: it is the title, and it is stored once.
    let short = example.run(&[
        "evidence",
        "add",
        "sys.evidence.cold-start",
        "--result",
        "pass",
        "--method",
        "command",
        "--summary",
        "Cold start 4.1 s, warm 0.3 s over ten runs",
    ]);
    assert_eq!(short.code, 0, "{}", short.output());

    // A long one: the title is its opening words and the summary keeps the whole.
    let long_summary = "Ran the full determinism sweep across 512 seeds on the release build and \
                        found no divergence in any of the per-tick digests";
    let long = example.run(&[
        "evidence",
        "add",
        "sys.evidence.sweep",
        "--result",
        "pass",
        "--method",
        "command",
        "--summary",
        long_summary,
    ]);
    assert_eq!(long.code, 0, "{}", long.output());

    // A title that the summary only repeats.
    let repeated = example.run(&[
        "evidence",
        "add",
        "sys.evidence.repeat",
        "--result",
        "pass",
        "--method",
        "manual",
        "--title",
        "Looked right on the device",
        "--summary",
        "Looked right on the device",
    ]);
    assert_eq!(repeated.code, 0, "{}", repeated.output());

    let text = example.read_file(".akr/records/sys/evidence.akr");
    let record = |key: &str| {
        let start = text.find(&format!("record {key}/1")).expect("the record");
        let end = text[start..]
            .find("\n}\n")
            .map_or(text.len(), |offset| start + offset);
        text[start..end].to_owned()
    };

    let short = record("sys.evidence.cold-start");
    assert!(
        short.contains("title \"Cold start 4.1 s, warm 0.3 s over ten runs\""),
        "{short}"
    );
    assert!(!short.contains("summary"), "{short}");

    let long = record("sys.evidence.sweep");
    assert!(long.contains("summary"), "{long}");
    assert!(long.contains("\u{2026}\""), "the title is cut: {long}");
    assert!(
        !long.contains(&format!("title \"{long_summary}\"")),
        "{long}"
    );

    let repeated = record("sys.evidence.repeat");
    assert!(
        repeated.contains("title \"Looked right on the device\""),
        "{repeated}"
    );
    assert!(!repeated.contains("summary"), "{repeated}");
}

// -------------------------------------------------------------------------------------
// advice arrives as notes
// -------------------------------------------------------------------------------------

#[test]
fn an_intent_that_lists_files_draws_a_note_and_is_still_written() {
    let example = Example::materialise("restatement-note");
    example.write_file(
        "inventory.akr",
        "intent \"\"\"\n    Add engine/practice.rs with commissions and manifests, and wire game::outcome::card into ui/outcome.rs.\n\"\"\"\n",
    );
    let propose = example.run(&[
        "propose",
        "sys.work.inventory",
        "--kind",
        "work",
        "--title",
        "Shared spine",
        "--from",
        "inventory.akr",
    ]);
    assert_eq!(
        propose.code,
        0,
        "advice never refuses: {}",
        propose.output()
    );
    assert!(
        propose.stdout.contains("names code"),
        "{}",
        propose.output()
    );

    example.write_file(
        "reason.akr",
        "intent \"\"\"\n    Give every profession one reducer, because three hand-rolled ones had drifted apart on rounding.\n\"\"\"\n",
    );
    let quiet = example.run(&[
        "propose",
        "sys.work.reasoned",
        "--kind",
        "work",
        "--title",
        "One reducer",
        "--from",
        "reason.akr",
    ]);
    assert_eq!(quiet.code, 0, "{}", quiet.output());
    assert!(!quiet.stdout.contains("names code"), "{}", quiet.output());
}

// -------------------------------------------------------------------------------------
// hooks by default (D-049)
// -------------------------------------------------------------------------------------

/// A fresh git repository in the temp directory, and a runner for `akr` and `git` in it.
fn fresh_repository(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("akr-p13-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp directory");
    let status = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&dir)
        .status()
        .expect("git runs");
    assert!(status.success());
    dir
}

fn akr_in(dir: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_akr"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("akr runs");
    assert!(output.status.success(), "{output:?}");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn init_installs_the_hooks_in_a_repository_unless_told_not_to() {
    let dir = fresh_repository("hooks");
    let out = akr_in(&dir, &["init", "--project", "demo"]);
    assert!(out.contains("git hooks"), "{out}");
    for hook in ["pre-commit", "commit-msg"] {
        let text = std::fs::read_to_string(dir.join(".git/hooks").join(hook)).expect("the hook");
        assert!(text.contains(&format!("akr git-hook {hook}")), "{text}");
    }
    let _ = std::fs::remove_dir_all(&dir);

    let dir = fresh_repository("no-hooks");
    let out = akr_in(&dir, &["init", "--project", "demo", "--no-hooks"]);
    assert!(!out.contains("git hooks"), "{out}");
    assert!(!dir.join(".git/hooks/pre-commit").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn init_keeps_a_hook_that_is_not_its_own() {
    // Installing by default must not replace a project's own pre-commit without a word.
    let dir = fresh_repository("foreign-hook");
    let hooks = dir.join(".git/hooks");
    std::fs::create_dir_all(&hooks).expect("hooks directory");
    std::fs::write(hooks.join("pre-commit"), "#!/bin/sh\nexec ./lint.sh\n").expect("a hook");

    let out = akr_in(&dir, &["init", "--project", "demo"]);
    assert!(out.contains("kept your existing pre-commit hook"), "{out}");
    assert_eq!(
        std::fs::read_to_string(hooks.join("pre-commit")).expect("still there"),
        "#!/bin/sh\nexec ./lint.sh\n"
    );
    assert!(
        std::fs::read_to_string(hooks.join("commit-msg"))
            .expect("the other one")
            .contains("akr git-hook commit-msg")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

//! The 0.7 write surface over MCP, held to the CLI (D-046, D-047, D-048).
//!
//! `docs/08-mcp.md` §1: one implementation. Each test does the same thing through
//! `knowledge.*` in one workspace and through `akr` in another, and requires the two
//! ledgers to be byte-identical afterwards — the only form of "the surfaces agree" that
//! cannot drift.

use crate::support::{Example, mcp_call};
use akr_core::json::Value;

const WORK: &str = "lege.work.extract-render-graph";

fn call(example: &Example, tool: &str, arguments: &str) -> (Value, bool) {
    mcp_call(example.root(), tool, arguments)
}

/// A work record with one command check, moved to `active`, written through the CLI in
/// both workspaces so that the step under test is the only difference between them.
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

/// A digest of every file under `.akr`: the scratch inputs a test wrote beside the
/// ledger are outside it, so two workspaces compare equal exactly when their ledgers do.
fn ledger(example: &Example) -> Vec<(String, String)> {
    example.sources()
}

#[test]
fn knowledge_trace_and_akr_trace_write_the_same_ledger() {
    let over_mcp = Example::materialise("mcp-trace");
    let over_cli = Example::materialise("mcp-trace-cli");
    let first = "split the graph out first: the viewer cannot be tested while it owns the passes";
    let second = "dropped the trait object: one implementor, and it hid the borrow";

    let (payload, is_error) = call(
        &over_mcp,
        "knowledge.trace",
        &format!(r#"{{"key":"{WORK}","entries":["{first}","{second}"]}}"#),
    );
    assert!(!is_error, "{}", payload.to_pretty());

    let run = over_cli.run(&["trace", WORK, first, second]);
    assert_eq!(run.code, 0, "{}", run.output());

    assert_eq!(ledger(&over_mcp), ledger(&over_cli));
    let written = over_mcp.read_file(".akr/records/lege/work.akr");
    assert!(
        written.contains(first) && written.contains(second),
        "the entries are in the ledger:\n{written}"
    );
}

#[test]
fn knowledge_trace_refuses_what_akr_trace_refuses() {
    let example = Example::materialise("mcp-trace-refusals");
    let before = example.sources();
    let long = vec!["word"; 41].join(" ");

    let (payload, is_error) = call(
        &example,
        "knowledge.trace",
        &format!(r#"{{"key":"{WORK}","entries":["{long}"]}}"#),
    );
    assert!(is_error, "{}", payload.to_pretty());
    assert!(
        payload.to_pretty().contains("AKR-T024"),
        "{}",
        payload.to_pretty()
    );

    let (payload, is_error) = call(
        &example,
        "knowledge.trace",
        r#"{"key":"sys.policy.tandem-work","entries":["a policy has a rationale"]}"#,
    );
    assert!(is_error, "{}", payload.to_pretty());
    assert!(
        payload.to_pretty().contains("AKR-T002"),
        "{}",
        payload.to_pretty()
    );

    let (payload, is_error) = call(
        &example,
        "knowledge.trace",
        &format!(r#"{{"key":"{WORK}"}}"#),
    );
    assert!(is_error, "{}", payload.to_pretty());

    assert_eq!(before, example.sources(), "a refusal writes nothing");
}

#[test]
fn a_recorded_pass_completes_the_same_way_on_both_surfaces() {
    let over_mcp = Example::materialise("mcp-pass");
    let over_cli = Example::materialise("mcp-pass-cli");
    let key = "sys.work.warm-cache";
    active_work_with_a_command_check(&over_mcp, key);
    active_work_with_a_command_check(&over_cli, key);
    assert_eq!(
        ledger(&over_mcp),
        ledger(&over_cli),
        "the same starting point"
    );

    let (payload, is_error) = call(
        &over_mcp,
        "knowledge.complete",
        &format!(r#"{{"key":"{key}","checks":{{"suite":{{"passed_at":"HEAD"}}}}}}"#),
    );
    assert!(!is_error, "{}", payload.to_pretty());
    assert!(
        payload.to_pretty().contains("empty trace"),
        "the advice arrives as a note on both surfaces: {}",
        payload.to_pretty()
    );

    let run = over_cli.run(&["complete", key, "--pass", "suite"]);
    assert_eq!(run.code, 0, "{}", run.output());

    assert_eq!(ledger(&over_mcp), ledger(&over_cli));
    let written = over_mcp.read_file(".akr/records/sys/work.akr");
    assert!(
        written.contains("passed_at git:"),
        "the check recorded its own pass:\n{written}"
    );
}

#[test]
fn a_pass_on_a_manual_check_and_a_new_retired_kind_are_refused_over_mcp() {
    let example = Example::materialise("mcp-pass-refusals");
    let before = example.sources();

    let (payload, is_error) = call(
        &example,
        "knowledge.complete",
        r#"{"key":"sys.milestone.m3-playable-day","checks":{"full-day-demo":{"passed_at":"HEAD"}}}"#,
    );
    assert!(is_error, "{}", payload.to_pretty());
    assert!(
        payload.to_pretty().contains("only a `command` check"),
        "{}",
        payload.to_pretty()
    );

    let (payload, is_error) = call(
        &example,
        "knowledge.propose",
        r#"{"key":"sys.term.brand-new","kind":"term","title":"New","scope":["all"],"slots":{"definition":"A word."}}"#,
    );
    assert!(is_error, "{}", payload.to_pretty());
    assert!(
        payload.to_pretty().contains("AKR-T008"),
        "{}",
        payload.to_pretty()
    );
    assert!(
        payload.to_pretty().contains("policy"),
        "{}",
        payload.to_pretty()
    );

    assert_eq!(before, example.sources(), "a refusal writes nothing");
}

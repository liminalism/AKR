//! Exit criterion 4 of P6: `knowledge.context` and `akr context` produce the same bundle
//! from the same request — and the same for every other read tool.
//!
//! Both surfaces are exercised as the processes they ship as. Testing the library functions
//! against each other would prove only that a call to one function equals a call to the same
//! function; running the two binaries proves that the *adapters* agree, which is where the
//! drift `docs/08-mcp.md` §1 warns about would actually appear.

mod support;

use akr_core::json::{Value, parse};
use support::{Example, mcp_binary};

/// One `tools/call`, over a fresh server process.
///
/// A fresh process per call is deliberate: §7 says a read tool's effect on the repository
/// is nil and that two calls against the same sources return byte-identical results. A
/// server that quietly cached between calls would pass a test that reused one process.
fn call(example: &Example, tool: &str, arguments: &str) -> Value {
    let request = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\
         \"params\":{{\"name\":\"{tool}\",\"arguments\":{arguments}}}}}\n"
    );
    let response = rpc(example, &request);
    response
        .first()
        .and_then(|value| value.get("result"))
        .and_then(|result| result.get("structuredContent"))
        .cloned()
        .unwrap_or(Value::Null)
}

/// Whether the last call returned an error result.
fn is_error(example: &Example, tool: &str, arguments: &str) -> bool {
    let request = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\
         \"params\":{{\"name\":\"{tool}\",\"arguments\":{arguments}}}}}\n"
    );
    rpc(example, &request)
        .first()
        .and_then(|value| value.get("result"))
        .and_then(|result| result.get("isError"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Feeds newline-delimited JSON-RPC to a server process and parses every response.
fn rpc(example: &Example, requests: &str) -> Vec<Value> {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let mut child = Command::new(mcp_binary())
        .arg("--dir")
        .arg(example.root())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("akr-mcp runs");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(requests.as_bytes())
        .expect("write");
    let output = child.wait_with_output().expect("akr-mcp exits");
    assert!(
        output.status.success(),
        "akr-mcp failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| parse(line).expect("a JSON-RPC response"))
        .collect()
}

/// The `result` object of an `akr ... --format json` run.
fn cli_result(example: &Example, args: &[&str]) -> Value {
    let mut full = vec!["--format", "json"];
    full.extend_from_slice(args);
    let run = example.run(&full);
    let document = parse(&run.stdout).unwrap_or_else(|error| {
        panic!(
            "akr {} produced no JSON: {error}\n{}",
            args.join(" "),
            run.output()
        )
    });
    document.get("result").cloned().unwrap_or(Value::Null)
}

fn without_search_overlays(value: Value) -> Value {
    match value {
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .filter(|(name, _)| {
                    !matches!(
                        name.as_str(),
                        "planning_candidates" | "recommended_context" | "has_more" | "next_offset"
                    )
                })
                .collect(),
        ),
        _ => value,
    }
}

// -------------------------------------------------------------------------------------
// Exit criterion 4
// -------------------------------------------------------------------------------------

#[test]
fn knowledge_context_and_akr_context_produce_the_same_bundle() {
    let example = Example::materialise("differential-context");
    let tool = call(
        &example,
        "knowledge.context",
        r#"{"goal":"sys.milestone.m3-playable-day","paths":["sim/src/project/**"]}"#,
    );
    let cli = cli_result(
        &example,
        &[
            "context",
            "--goal",
            "sys.milestone.m3-playable-day",
            "--paths",
            "sim/src/project/**",
        ],
    );
    assert_eq!(
        tool.to_pretty(),
        cli.to_pretty(),
        "the bundle must be identical, not merely equivalent"
    );
    assert!(tool.get("sections").is_some(), "{}", tool.to_pretty());
}

#[test]
fn knowledge_start_names_a_ledger_coverage_miss_and_the_recovery_path() {
    let example = Example::materialise("differential-start-coverage");
    example.write_file(
        "Sources/intake.md",
        "The moonstone blueprint cadence governs the opening.",
    );
    let tool = call(
        &example,
        "knowledge.start",
        r#"{"task":"moonstone blueprint cadence"}"#,
    );
    assert_eq!(
        tool.get("coverage")
            .and_then(|coverage| coverage.get("status"))
            .and_then(Value::as_str),
        Some("no_planning_match"),
        "{}",
        tool.to_pretty()
    );
    assert!(
        tool.get("coverage")
            .and_then(|coverage| coverage.get("next_steps"))
            .and_then(Value::as_array)
            .is_some_and(|steps| steps.len() >= 4),
        "{}",
        tool.to_pretty()
    );
    assert_eq!(
        tool.get("workspace_fallback")
            .and_then(|fallback| fallback.get("provenance"))
            .and_then(Value::as_str),
        Some("workspace text scan; non-authoritative and not AKR knowledge")
    );
    assert!(
        tool.get("workspace_fallback")
            .and_then(|fallback| fallback.get("hits"))
            .and_then(Value::as_array)
            .is_some_and(|hits| hits
                .iter()
                .any(|hit| hit.get("path").and_then(Value::as_str) == Some("Sources/intake.md"))),
        "{}",
        tool.to_pretty()
    );
}

/// Bulk evidence lands identically whichever surface asked for it.
///
/// This file exists to stop the two surfaces drifting, and `knowledge.evidence_add_many`
/// is what happens when a tool is added without a case here: it grew a whole
/// implementation inside `akr-mcp`, and no command-line equivalent at all, for as long as
/// nothing compared them. The comparison is the ledger bytes, which is the only equality
/// that matters -- the two calls take different input shapes on purpose.
#[test]
fn bulk_evidence_writes_the_same_ledger_from_either_surface() {
    let head = {
        let probe = Example::materialise("differential-evidence-head-probe");
        probe.commit(5).to_owned()
    };

    // Over MCP: a JSON array of evidence payloads.
    let over_mcp = Example::materialise("differential-evidence-mcp");
    let payload = format!(
        r#"{{"evidence":[{{"key":"sys.evidence.dual-one","title":"First check","result":"pass","method":"command","command":"cargo test -p sim","observed_at":"git:{head}"}},{{"key":"sys.evidence.dual-two","title":"Second check","result":"pass","method":"observation","observed_at":"git:{head}"}}]}}"#
    );
    let tool = call(&over_mcp, "knowledge.evidence_add_many", &payload);
    assert_eq!(
        tool.get("written").and_then(Value::as_integer),
        Some(2),
        "{}",
        tool.to_pretty()
    );
    let from_mcp = over_mcp.read_file(".akr/records/sys/evidence.akr");

    // From the command line: the same two records, as an AKR fragment.
    let over_cli = Example::materialise("differential-evidence-cli");
    let batch = format!(
        "record sys.evidence.dual-one/1 : evidence {{\n    \
             title \"First check\"\n    result pass\n    method command\n    \
             command \"cargo test -p sim\"\n    observed_at git:{head}\n\
         }}\n\n\
         record sys.evidence.dual-two/1 : evidence {{\n    \
             title \"Second check\"\n    result pass\n    method observation\n    \
             observed_at git:{head}\n\
         }}\n"
    );
    over_cli.write_file("batch.akr", &batch);
    let run = over_cli.run(&["evidence", "add-many", "--from", "batch.akr"]);
    assert_eq!(run.code, 0, "{}", run.output());
    let from_cli = over_cli.read_file(".akr/records/sys/evidence.akr");

    assert_eq!(
        from_mcp, from_cli,
        "the two surfaces must write byte-identical records"
    );
}

#[test]
fn knowledge_start_can_locate_a_user_supplied_plan_by_its_path() {
    let example = Example::materialise("differential-start-plan-path");
    example.write_file(
        "Sources/current-plan-80626.md",
        "# Delivery notes\n\nThe first action is intentionally described with unrelated vocabulary.\n",
    );
    let tool = call(
        &example,
        "knowledge.start",
        r#"{"task":"follow Sources/current-plan-80626.md"}"#,
    );
    assert!(
        tool.get("workspace_fallback")
            .and_then(|fallback| fallback.get("hits"))
            .and_then(Value::as_array)
            .is_some_and(|hits| hits.iter().any(|hit| {
                hit.get("path").and_then(Value::as_str) == Some("Sources/current-plan-80626.md")
            })),
        "{}",
        tool.to_pretty()
    );
}

#[cfg(feature = "fts5")]
#[test]
fn knowledge_start_and_akr_start_share_the_session_head() {
    let example = Example::materialise("differential-session-head");
    let tool = call(
        &example,
        "knowledge.start",
        r#"{"task":"rewrite projection","budget_tokens":1400}"#,
    );
    let cli = cli_result(
        &example,
        &["start", "rewrite projection", "--budget", "1400"],
    );
    assert_eq!(tool.to_pretty(), cli.to_pretty());
    assert_eq!(
        tool.get("handoff")
            .and_then(|handoff| handoff.get("snapshot"))
            .and_then(|snapshot| snapshot.get("origin"))
            .and_then(Value::as_str),
        Some("working_tree")
    );
}

#[test]
fn a_budget_reaches_the_same_assembly_through_both_surfaces() {
    let example = Example::materialise("differential-budget");
    let tool = call(
        &example,
        "knowledge.context",
        r#"{"goal":"sys.milestone.m3-playable-day","budget_tokens":900}"#,
    );
    let cli = cli_result(
        &example,
        &[
            "context",
            "--goal",
            "sys.milestone.m3-playable-day",
            "--budget",
            "900",
        ],
    );
    assert_eq!(tool.to_pretty(), cli.to_pretty());
}

// -------------------------------------------------------------------------------------
// Every read tool
// -------------------------------------------------------------------------------------

#[test]
fn knowledge_get_and_akr_get_agree() {
    let example = Example::materialise("differential-get");
    for reference in [
        "@sys.policy.tandem-work",
        "@sim.obs.projection-gaps",
        "@sys.work.m3-plan/1",
    ] {
        let tool = call(
            &example,
            "knowledge.get",
            &format!("{{\"ref\":\"{reference}\"}}"),
        );
        let cli = cli_result(&example, &["get", reference, "--relations"]);
        assert_eq!(tool.to_pretty(), cli.to_pretty(), "for {reference}");
    }
}

/// `explain` is the documented way to find out what a kind requires, and it answers from
/// the vocabulary tables rather than from the ledger. Routing it through the workspace
/// dispatcher reached the `unreachable!` that `run_standalone` already owns, so the tool
/// returned `AKR-X099` for every subject while the command line answered normally.
#[test]
fn knowledge_explain_and_akr_explain_agree() {
    let example = Example::materialise("differential-explain");
    for subject in ["decision", "constraint", "evidence", "AKR-C031"] {
        let tool = call(
            &example,
            "knowledge.explain",
            &format!("{{\"subject\":\"{subject}\"}}"),
        );
        assert!(
            !is_error(
                &example,
                "knowledge.explain",
                &format!("{{\"subject\":\"{subject}\"}}")
            ),
            "explaining {subject} must not fail"
        );
        let cli = cli_result(&example, &["explain", subject]);
        assert_eq!(tool.to_pretty(), cli.to_pretty(), "for {subject}");
    }
}

#[test]
fn knowledge_impact_and_akr_impact_agree_in_both_modes() {
    let example = Example::materialise("differential-impact");
    let tool = call(
        &example,
        "knowledge.impact",
        r#"{"ref":"@sim.obs.projection-gaps"}"#,
    );
    let cli = cli_result(&example, &["impact", "@sim.obs.projection-gaps"]);
    assert_eq!(tool.to_pretty(), cli.to_pretty());

    let range = format!("{}..{}", example.commit(2), example.commit(4));
    let tool = call(
        &example,
        "knowledge.impact",
        &format!("{{\"git_diff\":\"{range}\"}}"),
    );
    let cli = cli_result(&example, &["impact", "--git-diff", &range]);
    assert_eq!(tool.to_pretty(), cli.to_pretty());
}

#[test]
fn knowledge_validate_agrees_with_akr_check() {
    let example = Example::materialise("differential-validate");
    let tool = call(&example, "knowledge.validate", "{}");
    let cli = cli_result(&example, &["check"]);

    assert_eq!(tool.get("ok").and_then(Value::as_bool), Some(true));
    for field in ["records", "revisions", "stale", "at_risk"] {
        assert_eq!(
            tool.get("counts").and_then(|c| c.get(field)),
            cli.get(field),
            "counts.{field}"
        );
    }
    assert_eq!(
        tool.get("diagnostics").and_then(Value::as_array),
        Some(&[][..]),
        "a clean ledger has no diagnostics"
    );
    assert_eq!(
        tool.get("diagnostics_total").and_then(Value::as_integer),
        Some(0)
    );
    assert_eq!(tool.get("has_more").and_then(Value::as_bool), Some(false));
    assert_eq!(tool.get("next_offset"), Some(&Value::Null));
}

#[cfg(feature = "fts5")]
#[test]
fn knowledge_search_and_akr_search_agree() {
    let example = Example::materialise("differential-search");
    assert_eq!(example.run(&["build"]).code, 0);

    let tool = without_search_overlays(call(
        &example,
        "knowledge.search",
        r#"{"query":"projection"}"#,
    ));
    let cli = cli_result(&example, &["search", "projection"]);
    assert_eq!(tool.to_pretty(), cli.to_pretty());
    assert!(
        tool.get("results")
            .and_then(Value::as_array)
            .is_some_and(|results| !results.is_empty()),
        "two empty result sets agree trivially: {}",
        tool.to_pretty()
    );

    // Filters travel through the tool the same way they travel through the flags, which is
    // the part an agent would notice first if it drifted.
    let tool = call(
        &example,
        "knowledge.search",
        r#"{"query":"day","kinds":["milestone"],"limit":3}"#,
    );
    let cli = cli_result(
        &example,
        &["search", "day", "--kind", "milestone", "--limit", "3"],
    );
    assert_eq!(without_search_overlays(tool).to_pretty(), cli.to_pretty());
}

#[cfg(feature = "fts5")]
#[test]
fn knowledge_search_continuation_advances_through_ranked_results() {
    let example = Example::materialise("differential-search-pagination");
    assert_eq!(example.run(&["build"]).code, 0);

    let first = call(
        &example,
        "knowledge.search",
        r#"{"query":"projection","limit":1}"#,
    );
    let next = first
        .get("next_offset")
        .and_then(Value::as_integer)
        .expect("the first page has a continuation");
    let first_key = first
        .get("results")
        .and_then(Value::as_array)
        .and_then(|results| results.first())
        .and_then(|result| result.get("key"))
        .and_then(Value::as_str)
        .expect("the first page is useful");

    let second = call(
        &example,
        "knowledge.search",
        &format!(r#"{{"query":"projection","limit":1,"offset":{next}}}"#),
    );
    let second_key = second
        .get("results")
        .and_then(Value::as_array)
        .and_then(|results| results.first())
        .and_then(|result| result.get("key"))
        .and_then(Value::as_str)
        .expect("the continuation page is useful");
    assert_ne!(first_key, second_key, "the continuation repeated page one");
}

#[cfg(feature = "fts5")]
#[test]
fn knowledge_search_refreshes_a_missing_index() {
    let example = Example::materialise("differential-search-refresh");
    assert_eq!(example.run(&["build"]).code, 0);
    std::fs::remove_dir_all(example.root().join(".akr/cache")).expect("the cache goes");

    let tool = call(&example, "knowledge.search", r#"{"query":"projection"}"#);
    assert_eq!(
        tool.get("index_stale").and_then(Value::as_bool),
        Some(false)
    );
    assert!(
        tool.get("results")
            .and_then(Value::as_array)
            .is_some_and(|results| !results.is_empty()),
        "{}",
        tool.to_pretty()
    );
    let repeated = call(&example, "knowledge.search", r#"{"query":"projection"}"#);
    assert_eq!(tool.to_pretty(), repeated.to_pretty());
}

#[cfg(not(feature = "fts5"))]
#[test]
fn a_cache_without_a_ranker_rejects_mcp_search() {
    // P7 exit criterion 4 reaches the tool surface too: an agent must learn that search is
    // unavailable, not that the ledger is empty. CLI/MCP parity belongs to the FTS5 suite:
    // Cargo does not build this package's independently packaged CLI binary with matching
    // features, so consulting target/debug/akr here makes the result depend on build order.
    let example = Example::materialise("differential-search-degraded");

    let payload = call(&example, "knowledge.search", r#"{"query":"projection"}"#);
    let error = payload.get("error").expect("an error payload");
    assert_eq!(
        error.get("class").and_then(Value::as_str),
        Some("environment")
    );
}

#[test]
fn an_uninitialised_workspace_carries_the_remedy_on_both_surfaces() {
    // The friction `akr.papercut.root-cause-of-the-discoverability-miss-the-cli`: the CLI
    // tells a caller to run `akr init`, but the MCP surface used to drop that `help` line,
    // so an agent saw `AKR-C011` with no way forward. Both surfaces must now carry it, and
    // — §1's whole point — carry the *same* thing.
    let example = Example::materialise("differential-uninitialised");
    std::fs::remove_dir_all(example.root().join(".akr")).expect("the ledger goes");

    const REMEDY: &str = "run `akr init` to create one";

    // MCP: the error payload's first diagnostic carries the help.
    let payload = call(&example, "knowledge.search", r#"{"query":"projection"}"#);
    let error = payload.get("error").expect("an error payload");
    assert_eq!(
        error.get("class").and_then(Value::as_str),
        Some("environment")
    );
    let mcp_help = error
        .get("diagnostics")
        .and_then(Value::as_array)
        .and_then(|d| d.first())
        .and_then(|d| d.get("help"))
        .and_then(Value::as_str);
    assert_eq!(
        mcp_help,
        Some(REMEDY),
        "MCP dropped the remedy: {payload:?}"
    );

    // CLI: the same help, on the same diagnostic, in the JSON envelope.
    let run = example.run(&["--format", "json", "search", "projection"]);
    let document = parse(&run.stdout).expect("the CLI emits JSON");
    let cli_help = document
        .get("diagnostics")
        .and_then(Value::as_array)
        .and_then(|d| d.first())
        .and_then(|d| d.get("help"))
        .and_then(Value::as_str);
    assert_eq!(
        cli_help,
        Some(REMEDY),
        "CLI dropped the remedy: {}",
        run.stdout
    );
    assert_eq!(
        mcp_help, cli_help,
        "the two surfaces disagree on the remedy"
    );
}

#[test]
fn an_empty_ledger_explains_how_to_create_the_first_goal() {
    let example = Example::materialise("differential-empty-ledger");
    let records = example.root().join(".akr/records");
    std::fs::remove_dir_all(&records).expect("fixture records removed");
    std::fs::create_dir_all(&records).expect("empty records directory restored");
    let archive = example.root().join(".akr/archive");
    if archive.exists() {
        std::fs::remove_dir_all(&archive).expect("fixture archive removed");
    }

    const MESSAGE: &str = "knowledge ledger has no records yet";
    const REMEDY: &str = "create the first planning record with `knowledge.propose`, then use its key as the context goal";

    let payload = call(
        &example,
        "knowledge.context",
        r#"{"goal":"sys.milestone.first"}"#,
    );
    let error = payload.get("error").expect("an error payload");
    assert_eq!(error.get("summary").and_then(Value::as_str), Some(MESSAGE));
    let diagnostic = error
        .get("diagnostics")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .expect("one diagnostic");
    assert_eq!(diagnostic.get("help").and_then(Value::as_str), Some(REMEDY));
}

// -------------------------------------------------------------------------------------
// §7 — read/write separation and idempotency
// -------------------------------------------------------------------------------------

#[test]
fn read_tools_are_byte_identical_across_calls_and_touch_nothing() {
    let example = Example::materialise("differential-idempotent");
    let before = example.sources();
    let first = call(
        &example,
        "knowledge.context",
        r#"{"goal":"sys.milestone.m3-playable-day"}"#,
    );
    let second = call(
        &example,
        "knowledge.context",
        r#"{"goal":"sys.milestone.m3-playable-day"}"#,
    );
    assert_eq!(first.to_pretty(), second.to_pretty());
    assert_eq!(before, example.sources(), "a read tool wrote something");
}

/// The same agreement, on the other worked example.
///
/// `save-your-skin` is where every other test here lives, and a differential property that
/// held on exactly one ledger would be evidence about that ledger rather than about the
/// adapters. `sys-tandem` is shaped differently — three source roots, a superseded
/// assessment, five milestones — so a bundle that agrees across both surfaces on it too is
/// §1's invariant rather than a coincidence of one example's shape.
#[test]
fn both_surfaces_agree_on_the_other_example_too() {
    let example = Example::of(&support::SYS_TANDEM, "differential-sys-tandem");

    let tool = call(
        &example,
        "knowledge.context",
        r#"{"goal":"tandem.milestone.m5-one-playable-day"}"#,
    );
    let cli = cli_result(
        &example,
        &["context", "--goal", "tandem.milestone.m5-one-playable-day"],
    );
    assert_eq!(tool.to_pretty(), cli.to_pretty());
    assert!(tool.get("sections").is_some(), "{}", tool.to_pretty());

    // A key whose head is a supersession, which `save-your-skin` does not exercise through
    // `knowledge.get`: the tool has to agree with the CLI about *which* revision is head.
    for reference in [
        "@tandem.assessment.central-fact",
        "@simulator.question.wild-threshold",
        "@engine.req.no-debug-surfaces",
    ] {
        let tool = call(
            &example,
            "knowledge.get",
            &format!("{{\"ref\":\"{reference}\"}}"),
        );
        let cli = cli_result(&example, &["get", reference, "--relations"]);
        assert_eq!(tool.to_pretty(), cli.to_pretty(), "for {reference}");
        // Two surfaces that both failed would also "agree", so the record has to be here.
        assert!(
            tool.get("key").and_then(Value::as_str).is_some(),
            "no record came back for {reference}: {}",
            tool.to_pretty()
        );
    }

    let tool = call(
        &example,
        "knowledge.impact",
        r#"{"ref":"@tandem.assessment.central-fact"}"#,
    );
    let cli = cli_result(&example, &["impact", "@tandem.assessment.central-fact"]);
    assert_eq!(tool.to_pretty(), cli.to_pretty());
    assert!(
        tool.get("dependents")
            .and_then(Value::as_array)
            .is_some_and(|dependents| !dependents.is_empty()),
        "an impact query with nothing downstream would agree trivially: {}",
        tool.to_pretty()
    );
}

#[test]
fn every_declared_tool_has_a_schema_and_an_implementation() {
    let example = Example::materialise("differential-catalogue");
    let responses = rpc(
        &example,
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\",\"params\":{}}\n",
    );
    let tools = responses[0]
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(Value::as_array)
        .expect("a tool list");
    // Derived, never counted in prose: the registry is the only place the catalogue is
    // named, so adding a tool without declaring it here is impossible rather than merely
    // discouraged.
    assert_eq!(
        tools.len(),
        akr_mcp::schema::TOOLS.len(),
        "tools/list must report exactly the declared catalogue"
    );

    for tool in tools {
        let name = tool.get("name").and_then(Value::as_str).expect("a name");
        assert!(name.starts_with("knowledge."), "{name}");
        let schema = tool.get("inputSchema").expect("a schema");
        assert_eq!(
            schema.get("type").and_then(Value::as_str),
            Some("object"),
            "{name}"
        );
        assert!(schema.get("properties").is_some(), "{name}");

        // Every declared tool answers, and a tool with required arguments says so rather
        // than guessing. A tool listed but unimplemented would be worse than one absent:
        // the agent would plan around it.
        let required = schema
            .get("required")
            .and_then(Value::as_array)
            .expect("a required list");
        if !required.is_empty() {
            assert!(
                is_error(&example, name, "{}"),
                "{name} requires {required:?} but accepted an empty call"
            );
        }
    }

    // And a name outside the catalogue is refused rather than ignored.
    let payload = call(&example, "knowledge.query", "{}");
    assert_eq!(
        payload
            .get("error")
            .and_then(|e| e.get("class"))
            .and_then(Value::as_str),
        Some("usage")
    );
}

#[test]
fn a_notification_gets_no_response_and_a_bad_line_does_not_end_the_session() {
    let example = Example::materialise("differential-protocol");
    let responses = rpc(
        &example,
        "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n\
         not json at all\n\
         {\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"ping\",\"params\":{}}\n",
    );
    // Two responses: the parse error and the ping. The notification is silent.
    assert_eq!(responses.len(), 2, "{responses:?}");
    assert_eq!(
        responses[0]
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(Value::as_integer),
        Some(-32700)
    );
    assert_eq!(responses[1].get("id").and_then(Value::as_integer), Some(7));
}

#[test]
fn both_supported_protocol_versions_are_accepted() {
    let example = Example::materialise("differential-protocol");

    let responses = rpc(
        &example,
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\"}}\n\
         {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2026-07-28\"}}\n\
         {\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"server/discover\",\"params\":{\"protocolVersion\":\"2026-07-28\"}}\n\
         {\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-06-18\"}}\n\
         {\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-03-26\"}}\n\
         {\"jsonrpc\":\"2.0\",\"id\":6,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\"}}\n",
    );

    let legacy = responses[0]
        .get("result")
        .and_then(|r| r.get("protocolVersion"))
        .and_then(Value::as_str)
        .expect("legacy initialize result");
    assert_eq!(legacy, "2024-11-05");

    let current = responses[1]
        .get("result")
        .and_then(|r| r.get("protocolVersion"))
        .and_then(Value::as_str)
        .expect("current initialize result");
    assert_eq!(current, "2026-07-28");

    // A discovery client reaches this method *before* `initialize`, and falls back to the
    // legacy handshake only on a `-32601`. A success in the wrong shape therefore ends the
    // connection instead of downgrading it, so the fields below are asserted by name.
    let discover = responses[2].get("result").expect("discover result");
    assert_eq!(
        discover.get("resultType").and_then(Value::as_str),
        Some("complete"),
        "{discover:?}",
    );
    let supported: Vec<&str> = discover
        .get("supportedVersions")
        .and_then(Value::as_array)
        .expect("discover supportedVersions")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    for version in ["2026-07-28", "2025-11-25", "2025-06-18", "2024-11-05"] {
        assert!(
            supported.contains(&version),
            "{version} missing: {supported:?}"
        );
    }
    assert_eq!(
        discover.get("ttlMs").and_then(Value::as_integer),
        Some(0),
        "{discover:?}",
    );
    assert_eq!(
        discover.get("cacheScope").and_then(Value::as_str),
        Some("private"),
        "{discover:?}",
    );
    // The identity travels in `_meta`, not at the top level, and `protocolVersion` is not
    // part of a discovery result at all.
    assert!(discover.get("protocolVersion").is_none(), "{discover:?}");
    assert!(discover.get("serverInfo").is_none(), "{discover:?}");
    assert_eq!(
        discover
            .get("_meta")
            .and_then(|meta| meta.get("io.modelcontextprotocol/serverInfo"))
            .and_then(|info| info.get("name"))
            .and_then(Value::as_str),
        Some("akr-mcp"),
        "{discover:?}",
    );

    assert_eq!(
        responses[3]
            .get("result")
            .and_then(|result| result.get("protocolVersion"))
            .and_then(Value::as_str),
        Some("2025-06-18")
    );
    assert_eq!(
        responses[4]
            .get("result")
            .and_then(|result| result.get("protocolVersion"))
            .and_then(Value::as_str),
        Some("2025-03-26")
    );
    // Absent from the supported list, this was silently answered `2024-11-05`, which every
    // rmcp 3.x client offers first — so the whole session ran a generation behind.
    assert_eq!(
        responses[5]
            .get("result")
            .and_then(|result| result.get("protocolVersion"))
            .and_then(Value::as_str),
        Some("2025-11-25")
    );
}

/// A handoff reaches the same object from either surface, and `open` stays read-only.
///
/// Disclosure is the claim worth a differential test rather than a unit one: it only holds
/// if *both* adapters honour it, and the MCP surface deliberately has no reveal-on-open
/// shorthand where the command line does (D-040, D-041).
#[test]
fn a_handoff_opens_the_same_way_from_either_surface() {
    let example = Example::materialise("differential-handoff");
    call(
        &example,
        "knowledge.handoff_session_begin",
        r#"{"request":"Review this project and optimise for performance, both memory and CPU.","baselines":["peak RSS 412 MB"]}"#,
    );
    let created = call(
        &example,
        "knowledge.handoff_create",
        r#"{"mode":"scout","role":"perf-scout","task":"Inspect the sim crate.","by":"parent-model","worker_notes":{"hypotheses":["the cost is all in chroma"],"not_examined":["sim/src/localtone.rs"]}}"#,
    );
    let id = created
        .get("packet")
        .and_then(Value::as_str)
        .expect("create names the packet")
        .to_owned();
    assert!(id.starts_with("sc-"), "the id says what it is: {id}");
    assert_eq!(
        created
            .get("scope")
            .and_then(Value::as_array)
            .and_then(|globs| globs.first())
            .and_then(Value::as_str),
        Some("**"),
        "the scope defaults to the whole project: {}",
        created.to_pretty()
    );

    let tool = call(
        &example,
        "knowledge.handoff_open",
        &format!(r#"{{"packet":"{id}"}}"#),
    );
    let cli = cli_result(&example, &["handoff", "open", &id]);
    assert_eq!(tool.to_pretty(), cli.to_pretty());

    // The session capsule is inherited, not copied, and it comes through resolved.
    assert!(
        tool.get("request")
            .and_then(Value::as_str)
            .is_some_and(|request| request.contains("memory and CPU")),
        "{}",
        tool.to_pretty()
    );
    assert!(
        tool.get("project")
            .is_some_and(|project| !project.is_null()),
        "the project capsule reaches the child:\n{}",
        tool.to_pretty()
    );

    // Neither surface leaks the withheld layer on an ordinary open.
    assert_eq!(
        tool.get("worker_notes_revealed").and_then(Value::as_bool),
        Some(false),
        "{}",
        tool.to_pretty()
    );
    assert!(
        !tool.to_pretty().contains("the cost is all in chroma"),
        "a scout's open leaked a hypothesis:\n{}",
        tool.to_pretty()
    );

    // `knowledge.handoff_open` is declared read-only, so opening must not have stamped
    // the packet — otherwise the tool's own `readOnlyHint` would be false.
    let listed = call(&example, "knowledge.handoff_list", "{}");
    assert!(
        listed
            .get("packets")
            .and_then(Value::as_array)
            .is_some_and(|rows| rows
                .iter()
                .all(|row| row.get("revealed").and_then(Value::as_bool) == Some(false))),
        "{}",
        listed.to_pretty()
    );

    let revealed = call(
        &example,
        "knowledge.handoff_reveal",
        &format!(r#"{{"packet":"{id}"}}"#),
    );
    assert!(
        revealed.to_pretty().contains("the cost is all in chroma"),
        "{}",
        revealed.to_pretty()
    );

    // A result files and aggregates identically from either side.
    call(
        &example,
        "knowledge.handoff_result",
        &format!(
            r#"{{"packet":"{id}","findings":["planes are materialised per channel"],"read":["sim/src/chroma.rs:1-470"],"not_examined":["sim/src/localtone.rs"]}}"#
        ),
    );
    let tool = call(&example, "knowledge.handoff_coverage", "{}");
    let cli = cli_result(&example, &["handoff", "coverage"]);
    assert_eq!(tool.to_pretty(), cli.to_pretty());
    assert!(
        tool.to_pretty().contains("sim/src/localtone.rs"),
        "coverage must name what nobody opened:\n{}",
        tool.to_pretty()
    );
}

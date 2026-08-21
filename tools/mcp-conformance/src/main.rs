//! Drives `akr-mcp` with real MCP client libraries, several generations of them at once.
//!
//! `crates/akr-mcp/tests/conformance.rs` checks the envelope against tables we wrote. This
//! checks it against code we did not write, which is the only thing that can tell us a
//! client got stricter. That distinction is not academic: `resultType: "tool"` sat in this
//! server for a fortnight while Claude Code and Codex 0.147 called it happily, because
//! neither looked at the field. Codex 0.149 types results *by* it, and every tool call
//! stopped working on the day that client updated — with the server unchanged.
//!
//! Invoked by `scripts/verify-mcp-conformance.sh`, which builds the server and lays down a
//! throwaway workspace. Reads two variables, because a Cargo project outside the workspace
//! cannot use `CARGO_BIN_EXE_akr-mcp` the way the in-workspace tests do:
//!
//!   AKR_MCP_BIN        path to the akr-mcp binary under test
//!   AKR_MCP_WORKSPACE  path to an AKR workspace it may open

mod checks;
mod gen3;
mod legacy;
mod report;

use report::{Check, Observations, Outcome};
use serde_json::{Map, Value, json};
use std::path::PathBuf;
use std::process::ExitCode;

/// A `tools/call` to make: the tool, and the arguments to send it.
pub type CallSpec = (String, Map<String, Value>);

/// Calls with arguments a tool accepts, so the success envelope is exercised and not only
/// the refusal path that every empty-argument call reaches.
fn accepted_calls() -> Vec<CallSpec> {
    let mut start = Map::new();
    start.insert("task".into(), json!("mcp conformance"));
    vec![("knowledge.start".to_string(), start)]
}

fn env_path(name: &str) -> Result<PathBuf, String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Ok(PathBuf::from(value)),
        _ => Err(format!(
            "{name} is not set; run scripts/verify-mcp-conformance.sh"
        )),
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let (server, workspace) = match (env_path("AKR_MCP_BIN"), env_path("AKR_MCP_WORKSPACE")) {
        (Ok(server), Ok(workspace)) => (server, workspace),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    if !server.is_file() {
        eprintln!("error: no server binary at {}", server.display());
        return ExitCode::from(2);
    }

    println!("server    {}", server.display());
    println!("workspace {}", workspace.display());
    println!();

    let extra = accepted_calls();
    // Oldest first, so the report reads as a timeline of client strictness.
    let observations = vec![
        legacy::gen1::observe(&server, &workspace, &extra).await,
        legacy::gen2::observe(&server, &workspace, &extra).await,
        gen3::observe(&server, &workspace, &extra).await,
    ];

    let mut failed = 0usize;
    for obs in &observations {
        let checks = checks::evaluate(obs);
        report_generation(obs, &checks);
        failed += checks.iter().filter(|c| c.failed()).count();
    }

    println!();
    if failed == 0 {
        println!("PASS  every client generation accepted every response.");
        ExitCode::SUCCESS
    } else {
        println!("FAIL  {failed} check(s) failed: a client speaking correct MCP would");
        println!("      reject a response this server calls successful.");
        ExitCode::FAILURE
    }
}

fn report_generation(obs: &Observations, checks: &[Check]) {
    println!("=== {} ===", obs.generation);
    for check in checks {
        match &check.outcome {
            Outcome::Pass => println!("  pass  {}", check.name),
            Outcome::NotApplicable(why) => println!("  n/a   {} ({why})", check.name),
            Outcome::Fail(why) => println!("  FAIL  {}\n          {why}", check.name),
        }
    }
    let (vacuous, declared) = checks::vacuous_output_schemas(obs);
    if declared > 0 {
        println!(
            "  note  {vacuous} of {declared} declared outputSchemas constrain nothing \
             beyond \"is an object\", so validating against them cannot fail"
        );
    }
    println!();
}

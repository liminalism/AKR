//! The judgements, written once and applied to every generation.

use crate::report::{Check, NotObserved, Observations};
use serde_json::Value;

/// Every protocol version MCP has published, newest first.
///
/// Written out rather than read from the server, for the reason
/// `crates/akr-mcp/tests/conformance.rs` gives about its own copy: a check that derives its
/// expectations from the thing it is checking agrees with every bug in it. Verified against
/// the `schema/` directory of the modelcontextprotocol specification repository, which
/// publishes exactly these five.
pub const MCP_PROTOCOL_VERSIONS: &[&str] = &[
    "2026-07-28",
    "2025-11-25",
    "2025-06-18",
    "2025-03-26",
    "2024-11-05",
];

/// Judges one generation's observations.
pub fn evaluate(obs: &Observations) -> Vec<Check> {
    let mut checks = Vec::new();

    if let Some(why) = &obs.fatal {
        checks.push(Check::fail("connects", why.clone()));
        return checks;
    }
    checks.push(Check::pass("connects"));

    // Negotiation. The silent half of the failure mode: a version this server does not
    // offer is not refused, it is answered with the oldest one, and the session runs a
    // generation behind without anyone being told.
    for (requested, got) in &obs.negotiated {
        let name = format!("negotiates {requested}");
        match got {
            Ok(actual) if actual == requested => checks.push(Check::pass(name)),
            Ok(actual) => checks.push(Check::fail(
                name,
                format!("asked for {requested}, was answered {actual}"),
            )),
            Err(why) => checks.push(Check::fail(name, why.clone())),
        }
    }

    // Discovery, on the SDKs that have it.
    match &obs.discovery_lifecycle {
        Ok(version) => checks.push(Check::pass(format!(
            "connects via server/discover (at {version})"
        ))),
        Err(NotObserved::Unsupported(why)) => {
            checks.push(Check::skip("connects via server/discover", *why))
        }
        Err(NotObserved::Failed(why)) => {
            checks.push(Check::fail("connects via server/discover", why.clone()))
        }
    }
    match &obs.discovery {
        Ok(offered) => {
            let missing: Vec<&str> = MCP_PROTOCOL_VERSIONS
                .iter()
                .copied()
                .filter(|v| !offered.iter().any(|o| o == v))
                .collect();
            let invented: Vec<&str> = offered
                .iter()
                .map(String::as_str)
                .filter(|v| !MCP_PROTOCOL_VERSIONS.contains(v))
                .collect();
            if missing.is_empty() && invented.is_empty() {
                checks.push(Check::pass("discover offers every published version"));
            } else {
                checks.push(Check::fail(
                    "discover offers every published version",
                    format!("missing {missing:?}, not published {invented:?}"),
                ));
            }
        }
        Err(NotObserved::Unsupported(why)) => {
            checks.push(Check::skip("discover offers every published version", *why))
        }
        Err(NotObserved::Failed(why)) => checks.push(Check::fail(
            "discover offers every published version",
            why.clone(),
        )),
    }

    // The catalogue itself has to deserialise, schemas included.
    if obs.tools.is_empty() {
        checks.push(Check::fail("lists tools", "no tools listed"));
        return checks;
    }
    checks.push(Check::pass(format!("lists {} tools", obs.tools.len())));

    // Every call must produce a response the SDK can type. Refusals travel through the
    // same constructor as answers, so an envelope right only on the happy path is an
    // envelope that breaks the first time a tool says no.
    let undeserialisable: Vec<&crate::report::CallOutcome> =
        obs.calls.iter().filter(|c| c.result.is_err()).collect();
    if undeserialisable.is_empty() {
        checks.push(Check::pass(format!(
            "all {} tool results are typeable",
            obs.calls.len()
        )));
    } else {
        let first = undeserialisable[0];
        let why = first
            .result
            .as_ref()
            .err()
            .map(String::as_str)
            .unwrap_or("");
        checks.push(Check::fail(
            "all tool results are typeable",
            format!(
                "{} of {} rejected, first: {} -> {}",
                undeserialisable.len(),
                obs.calls.len(),
                first.tool,
                why
            ),
        ));
    }

    // The success envelope has to be reachable, not just the refusal one. If every call
    // came back `isError: true` the previous check would pass on refusals alone and prove
    // nothing about the shape a client actually reads most of the time.
    match obs
        .calls
        .iter()
        .filter_map(|c| c.result.as_ref().ok())
        .find(|ok| ok.is_error == Some(false))
    {
        Some(ok) if ok.content_blocks > 0 => {
            checks.push(Check::pass("an accepted call returns content"))
        }
        Some(_) => checks.push(Check::fail(
            "an accepted call returns content",
            "succeeded with an empty content array; a text-only client would show nothing",
        )),
        None => checks.push(Check::fail(
            "an accepted call returns content",
            "no call succeeded, so only the refusal envelope was exercised",
        )),
    }

    // Declared versus actual: does structuredContent satisfy the outputSchema this same
    // server advertised for that tool?
    let mut violations = Vec::new();
    let mut validated = 0usize;
    for call in &obs.calls {
        let Ok(ok) = &call.result else { continue };
        let Some(structured) = &ok.structured else {
            continue;
        };
        let Some(Some(schema)) = obs
            .tools
            .iter()
            .find(|(name, _)| *name == call.tool)
            .map(|(_, schema)| schema)
        else {
            continue;
        };
        validated += 1;
        if let Err(why) = validate(schema, structured) {
            violations.push(format!("{}: {}", call.tool, why));
        }
    }
    if violations.is_empty() {
        checks.push(Check::pass(format!(
            "{validated} payloads satisfy their declared outputSchema"
        )));
    } else {
        checks.push(Check::fail(
            "payloads satisfy their declared outputSchema",
            violations.join("; "),
        ));
    }

    checks
}

/// How many listed tools declare an `outputSchema` that constrains nothing.
///
/// Reported rather than asserted. `{"type":"object"}` admits any object, so validating
/// against it can never fail — a check that cannot fail is worse than no check, because it
/// reads as coverage. Naming the count keeps the gap visible instead of flattering it.
pub fn vacuous_output_schemas(obs: &Observations) -> (usize, usize) {
    let declared = obs.tools.iter().filter(|(_, s)| s.is_some()).count();
    let vacuous = obs
        .tools
        .iter()
        .filter_map(|(_, s)| s.as_ref())
        .filter(|s| is_vacuous(s))
        .count();
    (vacuous, declared)
}

/// True when a schema places no constraint beyond "is an object".
fn is_vacuous(schema: &Value) -> bool {
    let Some(map) = schema.as_object() else {
        return false;
    };
    map.keys()
        .all(|k| k == "type" || k == "$schema" || k == "description")
}

/// Validates one instance against one schema.
fn validate(schema: &Value, instance: &Value) -> Result<(), String> {
    let mut compiler = boon::Compiler::new();
    let mut schemas = boon::Schemas::new();
    let url = "mem://output.json";
    compiler
        .add_resource(url, schema.clone())
        .map_err(|e| format!("schema not loadable: {e}"))?;
    let key = compiler
        .compile(url, &mut schemas)
        .map_err(|e| format!("schema does not compile: {e}"))?;
    schemas
        .validate(instance, key)
        .map_err(|e| format!("payload does not satisfy it: {e}"))
}

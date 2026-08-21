//! The wire contract: every response this server emits must be a shape a client can type.
//!
//! `differential.rs` proves the adapters agree with the command line — that the *payload*
//! is right. This file proves the *envelope* is right, which is a different failure and
//! the one that has actually shipped. Three times a field has carried a value invented for
//! a vocabulary MCP had already closed: `supportedProtocols` on a discovery result,
//! `"tool"` for `resultType` on every tool this server has, and a protocol list missing
//! the version current clients offer first. None produced a JSON-RPC error. Each was a
//! *successful* response the client could not deserialise, and a malformed success is
//! worse than a refusal — the client reads it as a broken peer rather than an old one, so
//! it never falls back, and the host attaches with zero tools.
//!
//! The tables below are those closed sets, written out rather than derived from the code.
//! That is the whole point: a test that reads its expectations out of the thing it is
//! testing agrees with every bug in it. `docs/08-mcp.md` §1 asks the same of the
//! differential tests, for the same reason.

mod support;

use akr_core::json::{Value, parse};
use akr_mcp::schema::TOOLS;
use support::{Example, mcp_binary};

/// Every `resultType` MCP defines.
///
/// A result carrying anything else is not an extension a client ignores — the field is how
/// the client decides which result type to parse, so an unknown value leaves it with no
/// branch to take and the whole response is rejected.
const MCP_RESULT_TYPES: &[&str] = &["complete", "input_required", "task"];

/// Every protocol version MCP has published, newest first.
///
/// A version this server offers that is not here is one it invented; a version here that
/// this server omits is a client it silently answers a generation behind.
const MCP_PROTOCOL_VERSIONS: &[&str] = &[
    "2026-07-28",
    "2025-11-25",
    "2025-06-18",
    "2025-03-26",
    "2024-11-05",
];

/// The keys a `tools/call` result may carry.
const CALL_TOOL_RESULT_KEYS: &[&str] = &[
    "content",
    "structuredContent",
    "isError",
    "resultType",
    "_meta",
];

/// The keys a `server/discover` result may carry, and which of them are required.
const DISCOVER_REQUIRED_KEYS: &[&str] = &[
    "resultType",
    "supportedVersions",
    "capabilities",
    "ttlMs",
    "cacheScope",
];
const DISCOVER_OPTIONAL_KEYS: &[&str] = &["instructions", "_meta"];

/// Sends newline-delimited requests to one fresh server and returns the responses.
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

/// Asserts a `tools/call` result is one a client can deserialise.
///
/// Deliberately indifferent to whether the tool succeeded: a refusal travels through the
/// same constructor as an answer, so an envelope that is only right on the happy path is
/// an envelope that breaks every client the first time a tool says no.
fn assert_call_tool_envelope(tool: &str, result: &Value) {
    let Value::Object(fields) = result else {
        panic!("{tool}: result is not an object: {result:?}");
    };
    for (name, _) in fields {
        assert!(
            CALL_TOOL_RESULT_KEYS.contains(&name.as_str()),
            "{tool}: result carries `{name}`, which is not a key of a tool result: \
             {CALL_TOOL_RESULT_KEYS:?}",
        );
    }

    if let Some(kind) = result.get("resultType") {
        let kind = kind
            .as_str()
            .unwrap_or_else(|| panic!("{tool}: resultType is not a string: {kind:?}"));
        assert!(
            MCP_RESULT_TYPES.contains(&kind),
            "{tool}: resultType is {kind:?}, which MCP does not define. \
             A client matches on this field and has no branch for an invented value, \
             so the whole response is rejected. One of: {MCP_RESULT_TYPES:?}",
        );
    }

    let content = result
        .get("content")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{tool}: no `content` array: {result:?}"));
    assert!(
        !content.is_empty(),
        "{tool}: `content` is empty; a text-only client would show nothing",
    );
    for block in content {
        assert_eq!(
            block.get("type").and_then(Value::as_str),
            Some("text"),
            "{tool}: content block is not a text block: {block:?}",
        );
        assert!(
            block.get("text").and_then(Value::as_str).is_some(),
            "{tool}: text block carries no `text`: {block:?}",
        );
    }

    assert!(
        result.get("isError").and_then(Value::as_bool).is_some(),
        "{tool}: `isError` is missing or not a boolean: {result:?}",
    );
}

/// Every tool, refused, still returns a result a client can type.
///
/// Empty arguments are the point: they reach the refusal path of all of them without this
/// test needing to know one tool's arguments from another's, and refusals are §5's
/// successful responses carrying `isError`, not transport errors. If the envelope is wrong
/// it is wrong here, for every tool at once.
#[test]
fn every_tool_refusal_is_an_envelope_a_client_can_type() {
    let example = Example::materialise("conformance-refusals");

    let requests: String = TOOLS
        .iter()
        .enumerate()
        .map(|(index, tool)| {
            format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":{},\"method\":\"tools/call\",\
                 \"params\":{{\"name\":\"{}\",\"arguments\":{{}}}}}}\n",
                index + 1,
                tool.name
            )
        })
        .collect();

    let responses = rpc(&example, &requests);
    assert_eq!(
        responses.len(),
        TOOLS.len(),
        "one response per tool: {} of {}",
        responses.len(),
        TOOLS.len(),
    );

    for (tool, response) in TOOLS.iter().zip(&responses) {
        let result = response
            .get("result")
            .unwrap_or_else(|| panic!("{}: no result — {response:?}", tool.name));
        assert_call_tool_envelope(tool.name, result);
    }
}

/// A tool that answers returns the same envelope as one that refuses.
#[test]
fn a_successful_tool_result_is_an_envelope_a_client_can_type() {
    let example = Example::materialise("conformance-success");

    let responses = rpc(
        &example,
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":\
         {\"name\":\"knowledge.start\",\"arguments\":{\"task\":\"conformance\"}}}\n",
    );

    let result = responses[0].get("result").expect("a result");
    assert_call_tool_envelope("knowledge.start", result);
    assert_eq!(
        result.get("isError").and_then(Value::as_bool),
        Some(false),
        "knowledge.start refused a well-formed request: {result:?}",
    );
}

/// The discovery result carries the fields a discovery result has, and no others.
///
/// `protocolVersion` and a top-level `serverInfo` belong to an `initialize` result. This
/// server once returned one of those under the discovery method name; every client that
/// probes with `server/discover` refused the connection outright rather than falling back,
/// because a client only retreats to the legacy handshake on a `-32601`.
#[test]
fn the_discovery_result_is_a_discovery_result() {
    let example = Example::materialise("conformance-discover");

    let responses = rpc(
        &example,
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"server/discover\",\"params\":{}}\n",
    );
    let result = responses[0].get("result").expect("a discovery result");

    for key in DISCOVER_REQUIRED_KEYS {
        assert!(
            result.get(key).is_some(),
            "discovery result is missing `{key}`, which its schema requires: {result:?}",
        );
    }
    let Value::Object(fields) = result else {
        panic!("discovery result is not an object: {result:?}");
    };
    for (name, _) in fields {
        assert!(
            DISCOVER_REQUIRED_KEYS.contains(&name.as_str())
                || DISCOVER_OPTIONAL_KEYS.contains(&name.as_str()),
            "discovery result carries `{name}`, which is not one of its fields. \
             `protocolVersion` and `serverInfo` belong to an initialize result; the \
             identity travels in `_meta`.",
        );
    }

    assert_eq!(
        result.get("resultType").and_then(Value::as_str),
        Some("complete"),
        "{result:?}",
    );
    assert!(
        result
            .get("_meta")
            .and_then(|meta| meta.get("io.modelcontextprotocol/serverInfo"))
            .and_then(|info| info.get("name"))
            .and_then(Value::as_str)
            .is_some(),
        "the server identity is not in `_meta`: {result:?}",
    );
}

/// Every version offered is one MCP published, and the newest one it knows is offered.
///
/// The second half is the half that fails quietly: a version missing from the list is not
/// refused, it is silently answered with the oldest one, so a current client runs an entire
/// session a generation behind what both ends could speak.
#[test]
fn the_protocol_versions_offered_are_versions_mcp_published() {
    let example = Example::materialise("conformance-versions");

    let responses = rpc(
        &example,
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"server/discover\",\"params\":{}}\n",
    );
    let offered: Vec<&str> = responses[0]
        .get("result")
        .and_then(|result| result.get("supportedVersions"))
        .and_then(Value::as_array)
        .expect("supportedVersions")
        .iter()
        .filter_map(Value::as_str)
        .collect();

    for version in &offered {
        assert!(
            MCP_PROTOCOL_VERSIONS.contains(version),
            "{version} is not a protocol version MCP published: {MCP_PROTOCOL_VERSIONS:?}",
        );
    }
    for version in MCP_PROTOCOL_VERSIONS {
        assert!(
            offered.contains(version),
            "{version} is a published protocol version this server does not offer. \
             A client preferring it is answered the oldest version instead, silently. \
             Offered: {offered:?}",
        );
    }

    // And an `initialize` naming one of them is answered with that one, not a fallback.
    for version in MCP_PROTOCOL_VERSIONS {
        let example = Example::materialise("conformance-negotiate");
        let responses = rpc(
            &example,
            &format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\
                 \"params\":{{\"protocolVersion\":\"{version}\"}}}}\n"
            ),
        );
        assert_eq!(
            responses[0]
                .get("result")
                .and_then(|result| result.get("protocolVersion"))
                .and_then(Value::as_str),
            Some(*version),
            "initialize at {version} was answered with a different version",
        );
    }
}

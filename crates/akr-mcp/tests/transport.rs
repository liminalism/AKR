//! The installed shape of the MCP server: one subprocess, one long-lived stdio session.

mod support;

use akr_core::json::{Value, parse};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use support::Example;

#[test]
fn version_and_initialize_report_the_cargo_package_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_akr-mcp"))
        .arg("--version")
        .output()
        .expect("version runs");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("version is UTF-8"),
        format!("akr-mcp {}\n", env!("CARGO_PKG_VERSION"))
    );

    let server = akr_mcp::Server::new("/workspace-not-opened-by-this-test");
    let request = parse(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
    )
    .expect("request is JSON");
    let response = server.handle(&request).expect("request has an id");
    let version = response
        .get("result")
        .and_then(|result| result.get("serverInfo"))
        .and_then(|info| info.get("version"))
        .and_then(Value::as_str);
    assert_eq!(version, Some(env!("CARGO_PKG_VERSION")));
    assert_eq!(
        response
            .get("result")
            .and_then(|result| result.get("protocolVersion"))
            .and_then(Value::as_str),
        Some("2025-06-18")
    );
}

fn list_tool_names(server: &akr_mcp::Server) -> Vec<String> {
    let listed = server
        .handle(&parse(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).expect("list is JSON"))
        .expect("list has an id");
    listed
        .get("result")
        .and_then(|result| result.get("tools"))
        .and_then(Value::as_array)
        .expect("tools array")
        .iter()
        .map(|tool| {
            tool.get("name")
                .and_then(Value::as_str)
                .expect("tool name")
                .to_owned()
        })
        .collect()
}

#[test]
fn grok_initialize_advertises_host_safe_tool_names() {
    let server = akr_mcp::Server::new("/workspace-not-opened-by-this-test");
    let init = parse(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","clientInfo":{"name":"grok","version":"1.0.25"}}}"#,
    )
    .expect("request is JSON");
    let response = server.handle(&init).expect("initialize has an id");
    let instructions = response
        .get("result")
        .and_then(|result| result.get("instructions"))
        .and_then(Value::as_str)
        .expect("instructions");
    assert!(instructions.contains("knowledge_context"), "{instructions}");
    assert!(
        !instructions.contains("knowledge.context"),
        "{instructions}"
    );

    let names = list_tool_names(&server);
    assert!(names.contains(&"knowledge_search".to_owned()), "{names:?}");
    assert!(
        names.contains(&"knowledge_papercut".to_owned()),
        "{names:?}"
    );
    assert!(
        names
            .iter()
            .all(|name| akr_mcp::schema::is_host_safe_name(name)),
        "{names:?}"
    );
    assert!(names.iter().all(|name| !name.contains('.')), "{names:?}");
}

#[test]
fn default_initialize_still_advertises_dotted_names() {
    let server = akr_mcp::Server::new("/workspace-not-opened-by-this-test");
    let init = parse(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
    )
    .expect("request is JSON");
    server.handle(&init).expect("initialize has an id");
    let names = list_tool_names(&server);
    assert!(names.contains(&"knowledge.search".to_owned()), "{names:?}");
    assert!(names.iter().any(|name| name.contains('.')), "{names:?}");
}

#[test]
fn tools_call_accepts_the_host_safe_name() {
    let server = akr_mcp::Server::new("/workspace-not-opened-by-this-test");
    let call = parse(
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"knowledge_explain","arguments":{"subject":"AKR-C003"}}}"#,
    )
    .expect("request is JSON");
    let response = server.handle(&call).expect("call has an id");
    let is_error = response
        .get("result")
        .and_then(|result| result.get("isError"))
        .and_then(Value::as_bool);
    assert_eq!(is_error, Some(false), "{}", response.to_pretty());
}

fn call(
    input: &mut impl Write,
    output: &mut impl BufRead,
    id: usize,
    method: &str,
    params: &str,
) -> Value {
    writeln!(
        input,
        r#"{{"jsonrpc":"2.0","id":{id},"method":"{method}","params":{params}}}"#
    )
    .expect("request writes");
    input.flush().expect("request flushes");
    let mut line = String::new();
    output.read_line(&mut line).expect("response reads");
    assert!(!line.is_empty(), "server closed before response {id}");
    parse(line.trim()).expect("response is JSON")
}

fn content_length_call(input: &mut impl Write, output: &mut impl BufRead, body: &str) -> Value {
    write!(input, "Content-Length: {}\r\n\r\n{body}", body.len()).expect("frame writes");
    input.flush().expect("frame flushes");

    let mut length = None;
    loop {
        let mut line = String::new();
        output.read_line(&mut line).expect("response header reads");
        assert!(
            !line.is_empty(),
            "server closed before Content-Length response"
        );
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length:") {
            length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .expect("numeric content length"),
            );
        }
    }
    let mut body = vec![0; length.expect("Content-Length response header")];
    output.read_exact(&mut body).expect("response body reads");
    parse(std::str::from_utf8(&body).expect("response is UTF-8")).expect("response is JSON")
}

#[test]
fn a_real_subprocess_serves_content_length_initialize_and_tools_list() {
    let example = Example::materialise("mcp-content-length-subprocess");
    let mut child = Command::new(env!("CARGO_BIN_EXE_akr-mcp"))
        .arg("--dir")
        .arg(example.root())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("server starts");
    let mut input = child.stdin.take().expect("stdin");
    let mut output = BufReader::new(child.stdout.take().expect("stdout"));

    let initialized = content_length_call(
        &mut input,
        &mut output,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{}}}"#,
    );
    assert_eq!(
        initialized
            .get("result")
            .and_then(|result| result.get("protocolVersion"))
            .and_then(Value::as_str),
        Some("2025-06-18")
    );

    let listed = content_length_call(
        &mut input,
        &mut output,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
    );
    assert!(
        listed
            .get("result")
            .and_then(|result| result.get("tools"))
            .and_then(Value::as_array)
            .is_some_and(|tools| tools.iter().any(|tool| {
                tool.get("name").and_then(Value::as_str) == Some("knowledge.papercut")
            })),
        "{}",
        listed.to_pretty()
    );

    drop(input);
    let completed = child.wait_with_output().expect("server exits on EOF");
    assert!(completed.status.success());
    assert!(completed.stderr.is_empty(), "{:?}", completed.stderr);
}

#[test]
fn a_real_subprocess_survives_mixed_reads_writes_and_rejections() {
    let example = Example::materialise("mcp-subprocess-stress");
    let mut child = Command::new(env!("CARGO_BIN_EXE_akr-mcp"))
        .arg("--dir")
        .arg(example.root())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("server starts");
    let mut input = child.stdin.take().expect("stdin");
    let mut output = BufReader::new(child.stdout.take().expect("stdout"));

    let mut id = 1usize;
    let initialized = call(
        &mut input,
        &mut output,
        id,
        "initialize",
        r#"{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"stress","version":"1"}}"#,
    );
    assert!(initialized.get("result").is_some());

    for round in 0..25 {
        for (tool, arguments) in [
            ("knowledge.explain", r#"{"subject":"decision"}"#.to_owned()),
            (
                "knowledge.context",
                r#"{"goal":"@sys.milestone.m3-playable-day/1","budget_tokens":4000}"#.to_owned(),
            ),
            (
                "knowledge.get",
                r#"{"ref":"@sys.term.playable-day/1","detail":"summary"}"#.to_owned(),
            ),
            ("knowledge.validate", r#"{"limit":5}"#.to_owned()),
            ("knowledge.unknown", r#"{}"#.to_owned()),
            (
                "knowledge.papercut",
                format!(
                    r#"{{"agent":"stress","namespace":"sys","message":"subprocess stress report {round}"}}"#
                ),
            ),
        ] {
            id += 1;
            let params = format!(r#"{{"name":"{tool}","arguments":{arguments}}}"#);
            let response = call(&mut input, &mut output, id, "tools/call", &params);
            assert!(response.get("result").is_some(), "{}", response.to_pretty());
        }
    }

    drop(input);
    let completed = child.wait_with_output().expect("server exits on EOF");
    assert!(
        completed.status.success(),
        "{}",
        String::from_utf8_lossy(&completed.stderr)
    );
    assert!(
        completed.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&completed.stderr)
    );
}

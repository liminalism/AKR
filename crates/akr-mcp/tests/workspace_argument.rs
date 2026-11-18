//! D-045: a server started outside any workspace takes the ledger per call.
//!
//! A host session that opens in a directory holding several checkouts starts `akr-mcp`
//! there, one level above every `.akr/`. Without a per-call workspace each tool answers
//! `AKR-C011` and the agent falls back to the command line. These tests hold the four
//! halves of the fix: the argument is advertised only when the root has no ledger, it is
//! honoured for reads and writes, a malformed one is a usage error, and a server inside a
//! workspace keeps exactly the schemas it always had.

use std::path::{Path, PathBuf};

use crate::support::{Example, mcp_rpc};
use akr_core::json::Value;
use akr_mcp::schema::{TOOLS, input_schema};

/// A fresh directory with no ledger above it, removed on drop.
struct Parent(PathBuf);

impl Parent {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("akr-ws-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("parent directory");
        Self(path)
    }
}

impl Drop for Parent {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn rpc(dir: &Path, requests: &[String]) -> Vec<Value> {
    let mut input = String::new();
    for request in requests {
        input.push_str(request);
        input.push('\n');
    }
    mcp_rpc(dir, &input)
}

fn list(id: u32) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/list"}}"#)
}

fn call(id: u32, tool: &str, arguments: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"{tool}","arguments":{arguments}}}}}"#
    )
}

fn json_string(text: &str) -> String {
    Value::string(text).to_pretty().trim_end().to_owned()
}

fn result(responses: &[Value], id: i64) -> &Value {
    responses
        .iter()
        .find(|r| r.get("id").and_then(Value::as_integer) == Some(id))
        .and_then(|r| r.get("result"))
        .unwrap_or_else(|| panic!("no result for request {id}: {responses:?}"))
}

fn is_error(result: &Value) -> bool {
    result.get("isError").and_then(Value::as_bool) == Some(true)
}

fn first_diagnostic(result: &Value) -> &Value {
    result
        .get("structuredContent")
        .and_then(|s| s.get("error"))
        .and_then(|e| e.get("diagnostics"))
        .and_then(Value::as_array)
        .and_then(|d| d.first())
        .expect("a diagnostic")
}

#[test]
fn a_rootless_server_advertises_workspace_and_honours_it() {
    let example = Example::materialise("workspace-argument-read");
    let parent = Parent::new("read");
    // A second checkout one level down, so AKR-C011 lists what is below.
    let sibling = parent.0.join("other");
    std::fs::create_dir_all(sibling.join(".akr")).expect("sibling ledger");
    std::fs::write(sibling.join(".akr").join("project.akr"), "").expect("project file");

    let absolute = json_string(&example.root().to_string_lossy());
    let relative = json_string(&format!(
        "../{}",
        example.root().file_name().unwrap().to_string_lossy()
    ));
    let responses = rpc(
        &parent.0,
        &[
            list(1),
            call(2, "knowledge.search", r#"{"query":"projection"}"#),
            call(
                3,
                "knowledge.search",
                &format!(r#"{{"query":"projection","workspace":{absolute}}}"#),
            ),
            call(
                4,
                "knowledge_search",
                &format!(r#"{{"query":"projection","workspace":{relative}}}"#),
            ),
        ],
    );

    let tools = result(&responses, 1)
        .get("tools")
        .and_then(Value::as_array)
        .expect("tools");
    assert_eq!(tools.len(), TOOLS.len());
    for tool in tools {
        let property = tool
            .get("inputSchema")
            .and_then(|s| s.get("properties"))
            .and_then(|p| p.get("workspace"));
        assert!(property.is_some(), "{tool:?} lacks `workspace`");
    }

    let refused = result(&responses, 2);
    assert!(is_error(refused));
    let diagnostic = first_diagnostic(refused);
    assert_eq!(
        diagnostic.get("code").and_then(Value::as_str),
        Some("AKR-C011")
    );
    let help = diagnostic.get("help").and_then(Value::as_str).unwrap_or("");
    assert!(
        help.contains("other"),
        "help names the checkout below: {help}"
    );
    assert!(
        help.contains("`workspace`"),
        "help names the argument: {help}"
    );

    for id in [3, 4] {
        let answered = result(&responses, id);
        assert!(!is_error(answered), "call {id} failed: {answered:?}");
    }
}

#[test]
fn a_write_through_workspace_lands_in_that_workspace() {
    let example = Example::materialise("workspace-argument-write");
    let parent = Parent::new("write");
    let workspace = json_string(&example.root().to_string_lossy());
    let responses = rpc(
        &parent.0,
        &[call(
            1,
            "knowledge.papercut",
            &format!(
                r#"{{"message":"a friction written through the workspace argument","agent":"test","workspace":{workspace}}}"#
            ),
        )],
    );
    let written = result(&responses, 1);
    assert!(!is_error(written), "papercut failed: {written:?}");

    let records = example.root().join(".akr").join("records");
    let found = walk(&records).into_iter().any(|file| {
        std::fs::read_to_string(&file)
            .is_ok_and(|text| text.contains("written through the workspace argument"))
    });
    assert!(found, "the papercut is in the example's ledger");
    assert!(
        !parent.0.join(".akr").exists(),
        "nothing was written at the server's root"
    );
}

#[test]
fn a_malformed_workspace_is_a_usage_error() {
    let example = Example::materialise("workspace-argument-usage");
    let responses = rpc(
        example.root(),
        &[
            call(
                1,
                "knowledge.search",
                r#"{"query":"projection","workspace":7}"#,
            ),
            call(
                2,
                "knowledge.search",
                r#"{"query":"projection","workspace":"  "}"#,
            ),
        ],
    );
    for id in [1, 2] {
        let refused = result(&responses, id);
        assert!(is_error(refused));
        assert_eq!(
            first_diagnostic(refused)
                .get("code")
                .and_then(Value::as_str),
            Some("AKR-C004")
        );
    }
}

#[test]
fn a_server_inside_a_workspace_keeps_its_schemas_and_still_honours_the_argument() {
    // Schemas are a fixed cost in every session (D-034): a server whose root is a
    // workspace must advertise byte-for-byte what it advertised before D-045.
    let example = Example::materialise("workspace-argument-rooted");
    let other = Example::materialise("workspace-argument-other");
    let workspace = json_string(&other.root().to_string_lossy());
    let responses = rpc(
        example.root(),
        &[
            list(1),
            call(
                2,
                "knowledge.search",
                &format!(r#"{{"query":"projection","workspace":{workspace}}}"#),
            ),
        ],
    );
    let tools = result(&responses, 1)
        .get("tools")
        .and_then(Value::as_array)
        .expect("tools");
    for tool in tools {
        let name = tool.get("name").and_then(Value::as_str).expect("name");
        let advertised = tool.get("inputSchema").expect("schema");
        let expected = input_schema(name).expect("a schema");
        assert_eq!(advertised, &expected, "{name}");
    }
    assert!(!is_error(result(&responses, 2)));
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walk(&path));
            } else {
                out.push(path);
            }
        }
    }
    out
}

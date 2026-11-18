//! Local-only secrets share the CLI implementation and have explicit reveal semantics.
mod support;
use akr_core::json::{Value, parse};
use akr_mcp::{
    protocol::{Server, Surface},
    schema, tools,
};
use support::Example;

#[test]
fn mcp_secret_workflow_and_default_public_output() {
    let example = Example::materialise("mcp-secrets");
    let root = example.root();
    let value = "synthetic-mcp-private-marker\nD:\\developer\\key.env";
    std::fs::create_dir_all(root.join(".agent/scratch")).unwrap();
    std::fs::write(root.join(".agent/scratch/input.txt"), value).unwrap();
    let put = tools::call(
        root,
        "knowledge_secret_put",
        &Value::object(vec![("from", Value::string(".agent/scratch/input.txt"))]),
    )
    .unwrap_or_else(|e| panic!("{}", e.summary));
    assert!(!put.structured().to_pretty().contains(value));
    let reference = put
        .structured()
        .get("citation")
        .and_then(Value::as_str)
        .unwrap();
    for reveal in [None, Some(false), Some(true)] {
        let mut args = vec![("ref", Value::string(reference))];
        if let Some(reveal) = reveal {
            args.push(("reveal", Value::bool(reveal)));
        }
        let got = tools::call(root, "knowledge.secret_get", &Value::object(args))
            .unwrap_or_else(|e| panic!("{}", e.summary));
        assert_eq!(
            got.structured().get("value").and_then(Value::as_str),
            Some(if reveal == Some(true) { value } else { "[*]" })
        );
    }
    let wrong_type = tools::call(
        root,
        "knowledge.secret_get",
        &Value::object(vec![
            ("ref", Value::string(reference)),
            ("reveal", Value::string("true")),
        ]),
    );
    assert!(wrong_type.is_err());
    std::fs::write(
        root.join(".agent/scratch/document.txt"),
        format!("Path {reference}."),
    )
    .unwrap();
    let resolved = tools::call(
        root,
        "knowledge.secret_resolve",
        &Value::object(vec![
            ("from", Value::string(".agent/scratch/document.txt")),
            ("reveal", Value::bool(true)),
        ]),
    )
    .unwrap_or_else(|e| panic!("{}", e.summary));
    assert_eq!(
        resolved.structured().get("text").and_then(Value::as_str),
        Some(format!("Path {value}.").as_str())
    );
    std::fs::remove_dir_all(root.join(".akr/secrets")).unwrap();
    let missing = tools::call(
        root,
        "knowledge.secret_get",
        &Value::object(vec![
            ("ref", Value::string(reference)),
            ("reveal", Value::bool(true)),
        ]),
    )
    .unwrap_or_else(|e| panic!("{}", e.summary));
    assert_eq!(
        missing.structured().get("value").and_then(Value::as_str),
        Some("[*]")
    );
}

#[test]
fn catalogue_handshake_and_read_surface_teach_the_privacy_boundary() {
    let example = Example::materialise("mcp-secret-handshake");
    for host_safe in [false, true] {
        for root in [
            example.root().to_path_buf(),
            example.root().join("no-ledger"),
        ] {
            let server = Server::new(root).with_host_safe_tool_names(host_safe);
            let response = server.handle(&parse(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}"#).unwrap()).unwrap();
            let instructions = response
                .get("result")
                .unwrap()
                .get("instructions")
                .unwrap()
                .as_str()
                .unwrap();
            assert!(instructions.contains(if host_safe {
                "knowledge_secret_put"
            } else {
                "knowledge.secret_put"
            }));
            assert!(instructions.contains("[*]"));
            assert!(instructions.contains("remote agents"));
        }
    }
    for name in [
        "knowledge.secret_put",
        "knowledge.secret_get",
        "knowledge.secret_resolve",
    ] {
        let tool = schema::TOOLS.iter().find(|t| t.name == name).unwrap();
        assert_eq!(tool.writes, name.ends_with("put"));
        assert_eq!(Surface::Read.exposes(tool), !tool.writes);
        assert!(schema::input_schema(name).is_some());
    }
    assert!(akr_cli::init::AGENTS_TEMPLATE.contains("knowledge.secret_put"));
    assert!(include_str!("../../../scripts/agent-section.md").contains("knowledge.secret_put"));
}

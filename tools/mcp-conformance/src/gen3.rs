//! rmcp 3.x — the generation Codex 0.149 ships, and the one that rejected this server.
//!
//! It is the strict one: results are typed *by* `resultType`, discovery is probed before
//! `initialize`, and a value outside a closed set is not an unknown field to skip but a
//! response with no branch to take.

use crate::report::{CallOk, CallOutcome, NotObserved, Observations};
use rmcp3::ServiceExt;
use rmcp3::model::{
    CallToolRequestParams, ClientCapabilities, ClientInfo, Implementation, ProtocolVersion,
    RequestMetaObject,
};
use rmcp3::service::{ClientLifecycleMode, ClientServiceExt};
use rmcp3::transport::TokioChildProcess;
use serde_json::Value;
use std::path::Path;
use tokio::process::Command;

pub const LABEL: &str = "rmcp 3.x";

/// The SDK's constant for a version string.
///
/// Going through the constants rather than constructing one from text is deliberate: it
/// keeps this tool honest about testing versions a real client can actually ask for, and a
/// version the SDK has never heard of is not a case any client would produce.
fn protocol(version: &str) -> ProtocolVersion {
    match version {
        "2026-07-28" => ProtocolVersion::V_2026_07_28,
        "2025-11-25" => ProtocolVersion::V_2025_11_25,
        "2025-06-18" => ProtocolVersion::V_2025_06_18,
        "2025-03-26" => ProtocolVersion::V_2025_03_26,
        _ => ProtocolVersion::V_2024_11_05,
    }
}

fn client_info(version: &str) -> ClientInfo {
    let mut implementation = Implementation::default();
    implementation.name = "mcp-conformance".into();
    implementation.version = env!("CARGO_PKG_VERSION").into();

    let mut info = ClientInfo::default();
    info.protocol_version = protocol(version);
    info.capabilities = ClientCapabilities::default();
    info.client_info = implementation;
    info
}

fn command(server: &Path, workspace: &Path) -> Command {
    let mut cmd = Command::new(server);
    cmd.arg("--dir").arg(workspace);
    cmd
}

pub async fn observe(server: &Path, workspace: &Path, extra: &[crate::CallSpec]) -> Observations {
    let mut obs = Observations {
        generation: LABEL,
        fatal: None,
        negotiated: Vec::new(),
        discovery: Err(NotObserved::Failed("not attempted".into())),
        discovery_lifecycle: Err(NotObserved::Failed("not attempted".into())),
        tools: Vec::new(),
        calls: Vec::new(),
    };

    // One fresh connection per protocol version: the version is part of the handshake, so
    // reusing a session would only ever re-observe the first one.
    for version in crate::checks::MCP_PROTOCOL_VERSIONS {
        let transport = match TokioChildProcess::new(command(server, workspace)) {
            Ok(t) => t,
            Err(e) => {
                obs.negotiated
                    .push(((*version).to_string(), Err(format!("spawn failed: {e}"))));
                continue;
            }
        };
        let outcome = match client_info(version)
            .serve_with_lifecycle(transport, ClientLifecycleMode::Initialize)
            .await
        {
            Ok(service) => {
                let got = service
                    .peer_info()
                    .map(|info| info.protocol_version.as_str().to_string())
                    .ok_or_else(|| "server sent no peer info".to_string());
                let _ = service.cancel().await;
                got
            }
            Err(e) => Err(e.to_string()),
        };
        obs.negotiated.push(((*version).to_string(), outcome));
    }

    // The discovery-first lifecycle: `server/discover` before `initialize`, falling back
    // only on a -32601. This is the path that failed outright when the discovery result
    // was shaped like an initialize result.
    let preferred: Vec<ProtocolVersion> = ProtocolVersion::KNOWN_VERSIONS
        .iter()
        .rev()
        .cloned()
        .collect();
    match TokioChildProcess::new(command(server, workspace)) {
        Err(e) => {
            obs.discovery_lifecycle = Err(NotObserved::Failed(format!("spawn failed: {e}")));
        }
        Ok(transport) => {
            let lifecycle = ClientLifecycleMode::Auto {
                preferred_versions: preferred,
                legacy_version: Some(ProtocolVersion::V_2025_06_18),
            };
            match client_info("2026-07-28")
                .serve_with_lifecycle(transport, lifecycle)
                .await
            {
                Ok(service) => {
                    obs.discovery_lifecycle = Ok(service
                        .peer_info()
                        .map(|i| i.protocol_version.as_str().to_string())
                        .unwrap_or_else(|| "unknown".into()));
                    obs.discovery = match service.discover(RequestMetaObject::default()).await {
                        Ok(result) => Ok(result
                            .supported_versions
                            .iter()
                            .map(|v| v.as_str().to_string())
                            .collect()),
                        Err(e) => Err(NotObserved::Failed(e.to_string())),
                    };
                    let _ = service.cancel().await;
                }
                Err(e) => {
                    obs.discovery_lifecycle = Err(NotObserved::Failed(e.to_string()));
                    obs.discovery = Err(NotObserved::Failed("connection failed".into()));
                }
            }
        }
    }

    // The catalogue and the calls, over one ordinary session.
    let transport = match TokioChildProcess::new(command(server, workspace)) {
        Ok(t) => t,
        Err(e) => return Observations::fatal(LABEL, format!("spawn failed: {e}")),
    };
    let service = match client_info("2026-07-28").serve(transport).await {
        Ok(s) => s,
        Err(e) => return Observations::fatal(LABEL, e.to_string()),
    };

    match service.list_all_tools().await {
        Ok(tools) => {
            for tool in &tools {
                let schema = tool
                    .output_schema
                    .as_ref()
                    .map(|s| Value::Object((**s).clone()));
                obs.tools.push((tool.name.to_string(), schema));
            }
        }
        Err(e) => {
            let _ = service.cancel().await;
            return Observations::fatal(LABEL, format!("tools/list: {e}"));
        }
    }

    // Every tool in the catalogue, with empty arguments. That reaches all of their refusal
    // paths without this tool needing to know one tool's arguments from another's — and a
    // refusal is returned through the same constructor as an answer, so an envelope that
    // is only right on the happy path breaks the first time a tool says no.
    let mut specs: Vec<crate::CallSpec> = obs
        .tools
        .iter()
        .map(|(name, _)| (name.clone(), serde_json::Map::new()))
        .collect();
    specs.extend(extra.iter().cloned());

    for (tool, arguments) in &specs {
        let mut param = CallToolRequestParams::default();
        param.name = tool.clone().into();
        param.arguments = Some(arguments.clone());
        let result = match service.call_tool(param).await {
            Ok(r) => Ok(CallOk {
                is_error: r.is_error,
                content_blocks: r.content.len(),
                structured: r.structured_content.clone(),
            }),
            Err(e) => Err(e.to_string()),
        };
        obs.calls.push(CallOutcome {
            tool: tool.clone(),
            result,
        });
    }

    let _ = service.cancel().await;
    obs
}

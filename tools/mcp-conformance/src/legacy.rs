//! rmcp 1.x and 2.x — the generations that accepted this server while it was wrong.
//!
//! Neither has any concept of `resultType`; serde drops the field unread. That is exactly
//! why they belong in the matrix rather than being dropped as obsolete: they are the
//! control. When a defect fails on 3.x and passes here, the matrix has reproduced the real
//! event — Codex 0.147 calling this server happily on 13 August, Codex 0.149 refusing every
//! tool on the 21st, with the server unchanged between them.
//!
//! The two SDKs are API-compatible across everything used here, so one macro body serves
//! both. If a future pair diverges, split the module rather than adding conditionals.

macro_rules! legacy_generation {
    ($modname:ident, $sdk:ident, $label:literal) => {
        pub mod $modname {
            use crate::report::{CallOk, CallOutcome, NotObserved, Observations};
            use sdk::ServiceExt;
            use sdk::model::{
                CallToolRequestParams, ClientCapabilities, ClientInfo, Implementation,
                ProtocolVersion,
            };
            use sdk::transport::TokioChildProcess;
            use serde_json::Value;
            use std::path::Path;
            use tokio::process::Command;
            use $sdk as sdk;

            pub const LABEL: &str = $label;

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

            /// This SDK's constant for a version string, where it has one.
            ///
            /// An older SDK has never heard of a newer protocol version, and asking it to
            /// name one would be testing a request no client of this generation could make.
            fn protocol(version: &str) -> ProtocolVersion {
                match version {
                    "2025-06-18" => ProtocolVersion::V_2025_06_18,
                    "2025-03-26" => ProtocolVersion::V_2025_03_26,
                    _ => ProtocolVersion::V_2024_11_05,
                }
            }

            /// Versions a client of this generation can actually ask for.
            fn requestable() -> &'static [&'static str] {
                &["2025-06-18", "2025-03-26", "2024-11-05"]
            }

            fn command(server: &Path, workspace: &Path) -> Command {
                let mut cmd = Command::new(server);
                cmd.arg("--dir").arg(workspace);
                cmd
            }

            pub async fn observe(
                server: &Path,
                workspace: &Path,
                extra: &[crate::CallSpec],
            ) -> Observations {
                let mut obs = Observations {
                    generation: LABEL,
                    fatal: None,
                    negotiated: Vec::new(),
                    discovery: Err(NotObserved::Unsupported(
                        "this SDK predates server/discover",
                    )),
                    discovery_lifecycle: Err(NotObserved::Unsupported(
                        "this SDK predates server/discover",
                    )),
                    tools: Vec::new(),
                    calls: Vec::new(),
                };

                for version in requestable() {
                    let transport = match TokioChildProcess::new(command(server, workspace)) {
                        Ok(t) => t,
                        Err(e) => {
                            obs.negotiated
                                .push(((*version).to_string(), Err(format!("spawn failed: {e}"))));
                            continue;
                        }
                    };
                    let outcome = match client_info(version).serve(transport).await {
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

                let transport = match TokioChildProcess::new(command(server, workspace)) {
                    Ok(t) => t,
                    Err(e) => return Observations::fatal(LABEL, format!("spawn failed: {e}")),
                };
                let service = match client_info("2025-06-18").serve(transport).await {
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
        }
    };
}

legacy_generation!(gen1, rmcp1, "rmcp 1.x");
legacy_generation!(gen2, rmcp2, "rmcp 2.x");

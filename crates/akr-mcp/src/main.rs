//! The `akr-mcp` binary: a JSON-RPC server on stdio, bound to one workspace.
//!
//! ```text
//! akr-mcp [--dir <path>]
//! ```
//!
//! No network, no daemon, no state outside the workspace — the same three constraints the
//! `akr` binary lives under (`docs/07-cli.md` §1), because this is the same tool with a
//! different mouth.

use akr_mcp::protocol::Surface;
use akr_mcp::{Server, serve};
use std::io::{BufReader, stdin, stdout};
use std::path::PathBuf;

fn main() -> std::process::ExitCode {
    let mut root = PathBuf::from(".");
    let mut surface = Surface::Full;
    let mut accounting: Option<PathBuf> = None;
    let mut host_safe_tool_names = false;
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--dir" => match args.next() {
                Some(value) => root = PathBuf::from(value),
                None => {
                    eprintln!("error[AKR-C003]: --dir requires a value");
                    return std::process::ExitCode::from(2);
                }
            },
            "--surface" => match args.next().as_deref().map(Surface::from_name) {
                Some(Some(value)) => surface = value,
                _ => {
                    eprintln!("error[AKR-C004]: --surface takes `read` or `full`");
                    return std::process::ExitCode::from(2);
                }
            },
            "--tool-names" => match args.next().as_deref() {
                Some("underscore") => host_safe_tool_names = true,
                Some("dotted") => host_safe_tool_names = false,
                _ => {
                    eprintln!("error[AKR-C004]: --tool-names takes `dotted` or `underscore`");
                    return std::process::ExitCode::from(2);
                }
            },
            "--accounting" => match args.next() {
                Some(value) => accounting = Some(PathBuf::from(value)),
                None => {
                    eprintln!("error[AKR-C003]: --accounting requires a path");
                    return std::process::ExitCode::from(2);
                }
            },
            "--version" | "-V" => {
                println!("akr-mcp {}", akr_mcp::protocol::SERVER_VERSION);
                return std::process::ExitCode::SUCCESS;
            }
            "--help" | "-h" => {
                println!(
                    "akr-mcp — the AKR knowledge tools over MCP\n\n\
                     USAGE\n    akr-mcp [--dir <path>] [--surface read|full] \
                     [--tool-names dotted|underscore] [--accounting <path>]\n\n\
                     Speaks JSON-RPC 2.0 over stdio, one document per line. The MCP tools \
                     of docs/08-mcp.md §2 are listed by `tools/list`; every one of them \
                     calls the function `akr` calls.\n\n\
                     --surface read exposes only the tools that answer questions. Tool \
                     schemas are a fixed cost in every session that loads them, and an \
                     agent that will only ever read should not pay for the writers.\n\n\
                     --tool-names underscore advertises `knowledge_search` instead of \
                     `knowledge.search`. Grok Build is detected from initialize and gets \
                     this automatically; `tools/call` accepts both forms either way.\n\n\
                     --accounting appends one JSON line per call: sizes, estimated \
                     tokens, duration, whether the output budget withheld it. `akr mcp \
                     stats` aggregates it.\n"
                );
                return std::process::ExitCode::SUCCESS;
            }
            other => {
                eprintln!("error[AKR-C002]: unknown flag {other:?}");
                return std::process::ExitCode::from(2);
            }
        }
    }

    let mut server = Server::new(root)
        .with_surface(surface)
        .with_host_safe_tool_names(host_safe_tool_names);
    if let Some(path) = accounting {
        server = server.with_accounting(path);
    }
    match serve(&server, BufReader::new(stdin()), stdout()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!(
                "error[AKR-C042]: MCP stdio failed: {error}\nhelp: reinstall the current akr-mcp binary, then reconnect or restart the MCP client; a running server keeps the executable it started with"
            );
            std::process::ExitCode::from(3)
        }
    }
}

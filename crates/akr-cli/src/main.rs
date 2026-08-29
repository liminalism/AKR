//! The `akr` binary.
//!
//! One binary, no daemon, no server, no state outside the workspace
//! (`docs/07-cli.md` §1). This file does three things: parse arguments, run the command,
//! and turn what it produced into either text or the JSON envelope. Every exit path goes
//! through [`Exit`], so the four statuses of §3 are decided in one place.

use akr_cli::args::{self, Invocation};
use akr_cli::commands;
use akr_cli::session::{self, EnvError, Exit, envelope, wants_json};
use std::io::Write as _;

fn main() -> std::process::ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let code = run(&argv);
    std::process::ExitCode::from(code as u8)
}

fn run(argv: &[String]) -> Exit {
    // Usage errors happen before anything is read or written (§3).
    let invocation = match args::parse(argv) {
        Ok(invocation) => invocation,
        Err(error) => {
            eprintln!("error[{}]: {}", error.code, error.message);
            if let Some(help) = &error.help {
                eprintln!("help: {help}");
            }
            return Exit::Usage;
        }
    };
    let Invocation { global, command } = invocation;
    let json = wants_json(&global);

    // Diagnostics need the source map to carry a path and a location (§5), and the source
    // map lives on the session — so the JSON form is built here, while the session is
    // still in scope, rather than being carried around in `Output`.
    let (outcome, diagnostics) = match commands::run_standalone(&command) {
        // Standalone commands run without a workspace, so they have no source map and, by
        // construction, no ledger diagnostics.
        Some(outcome) => (outcome, Vec::new()),
        None => match session::Session::open_ledger(&global) {
            Ok(mut session) => {
                let outcome = commands::run(&mut session, &command);
                let diagnostics = outcome.as_ref().map_or_else(
                    |_| Vec::new(),
                    |output| {
                        commands::diagnostics_json(
                            &output.diagnostics,
                            &session.sources,
                            session.global.profile,
                        )
                    },
                );
                (outcome, diagnostics)
            }
            Err(error) => (Err(error), Vec::new()),
        },
    };

    match outcome {
        Ok(output) => {
            if json {
                let document = envelope(
                    &command.name(),
                    output.commit.as_deref(),
                    &output.source_graph,
                    output.exit,
                    diagnostics,
                    output.result.clone(),
                );
                print!("{}", document.to_pretty());
            } else {
                print!("{}", output.text);
            }
            let _ = std::io::stdout().flush();
            output.exit
        }
        Err(error) => {
            report_environment(&error, json, &command.name());
            Exit::Environment
        }
    }
}

/// An environment failure: exit status 3, and never a JSON `result`.
fn report_environment(error: &EnvError, json: bool, command: &str) {
    if json {
        // The diagnostic carries the same optional `help` field as every other §5
        // diagnostic; dropping it here would make the JSON surface strictly less helpful
        // than the text one and put the CLI out of step with the MCP payload.
        let mut fields = vec![
            ("code", akr_core::json::Value::string(error.code)),
            ("severity", akr_core::json::Value::string("error")),
            (
                "message",
                akr_core::json::Value::string(error.message.clone()),
            ),
        ];
        if let Some(help) = &error.help {
            fields.push(("help", akr_core::json::Value::string(help.clone())));
        }
        let document = envelope(
            command,
            None,
            "",
            Exit::Environment,
            vec![akr_core::json::Value::object(fields)],
            akr_core::json::Value::Object(Vec::new()),
        );
        print!("{}", document.to_pretty());
    } else {
        eprintln!("{error}");
        if let Some(help) = &error.help {
            eprintln!("help: {help}");
        }
    }
}

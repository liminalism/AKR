//! The `akr` command line, as a library.
//!
//! The binary in `main.rs` is a thin shell over this: parse arguments, run the command,
//! print. Everything else lives here so that `akr-mcp` can call *the same functions the
//! command line calls* rather than a parallel implementation of them.
//!
//! That is not a convenience. `docs/08-mcp.md` §1 makes it an invariant:
//!
//! > **One implementation.** `knowledge.context` and `akr context` call the same function
//! > with the same arguments and produce the same bundle. There is no MCP-specific
//! > assembly, ranking, or filtering. A behaviour that cannot be reproduced from the
//! > command line is a bug.
//!
//! An invariant of that shape is only worth stating if the code makes it structurally
//! true. Sharing a crate does; a differential test over two implementations would only
//! tell you when they had already drifted.

pub mod args;
pub mod change;
pub mod commands;
pub mod config;
pub mod fmt;
pub mod handoff;
pub mod import;
pub mod ingest;
pub mod init;
pub mod review_snapshot;
pub mod secrets;
pub mod session;
pub mod source;
pub mod write;

/// Result of running an `akr` command in-process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Execution {
    /// The §3 exit status.
    pub exit: session::Exit,
    /// Captured standard output.
    pub stdout: String,
    /// Captured standard error.
    pub stderr: String,
}

/// Parses `argv` (excluding the program name), executes the command, and captures output.
#[must_use]
pub fn execute(argv: &[String]) -> Execution {
    use std::fmt::Write as _;

    let mut stdout = String::new();
    let mut stderr = String::new();

    // Usage errors happen before anything is read or written (§3).
    let invocation = match args::parse(argv) {
        Ok(invocation) => invocation,
        Err(error) => {
            let _ = writeln!(stderr, "error[{}]: {}", error.code, error.message);
            if let Some(help) = &error.help {
                let _ = writeln!(stderr, "help: {help}");
            }
            return Execution {
                exit: session::Exit::Usage,
                stdout,
                stderr,
            };
        }
    };
    let args::Invocation { global, command } = invocation;
    let json = session::wants_json(&global);

    let (outcome, diagnostics) = match &command {
        args::Command::Init {
            project,
            namespaces,
            no_hooks,
        } if global.dir != std::path::Path::new(".") => (
            init::run_in(&global.dir, project.as_deref(), namespaces, !*no_hooks),
            Vec::new(),
        ),
        _ => match commands::run_standalone(&command) {
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
        },
    };

    let exit = match outcome {
        Ok(output) => {
            if json {
                let document = session::envelope(
                    &command.name(),
                    output.commit.as_deref(),
                    &output.source_graph,
                    output.exit,
                    diagnostics,
                    output.result.clone(),
                );
                stdout.push_str(&document.to_pretty());
            } else {
                stdout.push_str(&output.text);
            }
            output.exit
        }
        Err(error) => {
            let exit = error.exit();
            report_environment(
                &error,
                json,
                &command.name(),
                exit,
                &mut stdout,
                &mut stderr,
            );
            exit
        }
    };

    Execution {
        exit,
        stdout,
        stderr,
    }
}

fn report_environment(
    error: &session::EnvError,
    json: bool,
    command: &str,
    exit: session::Exit,
    stdout: &mut String,
    stderr: &mut String,
) {
    use std::fmt::Write as _;

    if json {
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
        let document = session::envelope(
            command,
            None,
            "",
            exit,
            vec![akr_core::json::Value::object(fields)],
            akr_core::json::Value::Object(Vec::new()),
        );
        stdout.push_str(&document.to_pretty());
    } else {
        let _ = writeln!(stderr, "{error}");
        if let Some(help) = &error.help {
            let _ = writeln!(stderr, "help: {help}");
        }
    }
}

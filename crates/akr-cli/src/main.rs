//! The `akr` binary.
//!
//! One binary, no daemon, no server, no state outside the workspace
//! (`docs/07-cli.md` §1). This file is a thin shell over [`akr_cli::execute`].

use std::io::Write as _;

fn main() -> std::process::ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let execution = akr_cli::execute(&argv);
    if !execution.stdout.is_empty() {
        print!("{}", execution.stdout);
        let _ = std::io::stdout().flush();
    }
    if !execution.stderr.is_empty() {
        eprint!("{}", execution.stderr);
        let _ = std::io::stderr().flush();
    }
    std::process::ExitCode::from(execution.exit as u8)
}

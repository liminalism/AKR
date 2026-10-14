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
pub mod session;
pub mod source;
pub mod write;

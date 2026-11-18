//! Detecting a server binary that is older than the workspace it is serving.
//!
//! # The friction this exists for
//!
//! Logged in AKR's own ledger on 2026-08-05 as
//! `akr.papercut.calling-the-knowledge-papercut-mcp-tool`, and hit again on 2026-08-08 by
//! a different agent: an `akr-mcp` on `PATH` that predates a vocabulary change rejects
//! every write with a diagnostic about a slot it has never heard of. The agent sees
//! `AKR-T002` about a record it did not author and reasonably concludes the *ledger* is
//! broken, when the truth is that one binary is stale and needs a restart.
//!
//! An MCP server is long-lived. Reinstalling the binary does not replace the process that
//! is already running, so the skew persists for the rest of the session — which is why
//! detecting it is worth a module rather than a comment in the install script.
//!
//! # What is compared
//!
//! `akr.lock` records the vocabulary version the workspace was last built with. This
//! server knows the version it was compiled against. If they differ, one of the two is
//! behind, and saying which turns a mystifying type error into a one-line remedy.

use akr_core::json::Value;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The vocabulary version this binary was built against.
pub const SERVER_VOCABULARY: &str = akr_cli::session::VOCABULARY_VERSION;

/// A version disagreement between this binary and the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skew {
    /// What the workspace's lock says it was built with.
    pub workspace: String,
    /// What this binary knows.
    pub server: String,
}

impl Skew {
    /// The one-line explanation, with the remedy.
    #[must_use]
    pub fn message(&self) -> String {
        format!(
            "this akr-mcp was built against vocabulary {server} and the workspace was \
             last built with vocabulary {workspace}. A record written by the newer of the \
             two will not type-check against the older, which usually shows up as an \
             AKR-T002 about a slot you did not write. Reinstall the server from the \
             current build and RECONNECT — a running MCP process keeps the binary it \
             started with, so reinstalling alone does not fix the session.",
            server = self.server,
            workspace = self.workspace,
        )
    }

    /// The diagnostic form, appended to a failing call's diagnostics array.
    #[must_use]
    pub fn diagnostic(&self) -> Value {
        Value::object(vec![
            ("code", Value::string("AKR-X042")),
            ("severity", Value::string("warning")),
            ("message", Value::string(self.message())),
            ("server_vocabulary", Value::string(self.server.clone())),
            (
                "workspace_vocabulary",
                Value::string(self.workspace.clone()),
            ),
            (
                "help",
                Value::string("scripts/setup-akr-mcp.sh reinstalls it; then reconnect the server"),
            ),
        ])
    }
}

/// Compares this binary against the workspace's lock.
///
/// Returns `None` when they agree, when there is no lock yet, or when the lock does not
/// name a vocabulary — an absent lock is an unbuilt workspace, not a skew, and guessing
/// would put a scary warning on every fresh checkout.
#[must_use]
pub fn detect(root: &Path) -> Option<Skew> {
    let lock = std::fs::read_to_string(root.join(".akr").join("akr.lock")).ok()?;
    let workspace = vocabulary_of(&lock)?;
    (workspace != SERVER_VOCABULARY).then(|| Skew {
        workspace,
        server: SERVER_VOCABULARY.to_owned(),
    })
}

/// The `vocabulary "x.y"` line of a lock's `build` block.
///
/// Read as text rather than through the lock parser on purpose: this runs on the failure
/// path, and a lock that the parser rejects is exactly the case where the answer matters
/// most. A regex-free scan cannot itself fail.
fn vocabulary_of(lock: &str) -> Option<String> {
    for line in lock.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("vocabulary ") {
            return Some(rest.trim().trim_matches('"').to_owned());
        }
    }
    None
}

/// The executable this server started from, as it was when it started.
///
/// A vocabulary skew is only one way a running server falls behind. A release that adds
/// a generated view without touching the vocabulary leaves the lock agreeing with a
/// server that still rejects the new view as unknown
/// (`lege-ecosystem.papercut.the-running-knowledge-validate-server-rejects`), so the
/// other half of the question is asked of the binary itself: has the file this process
/// was loaded from been replaced since?
#[derive(Debug, Clone)]
pub struct Executable {
    path: PathBuf,
    modified: Option<SystemTime>,
}

impl Executable {
    /// The running executable, or `None` when the platform cannot say where it is.
    #[must_use]
    pub fn current() -> Option<Self> {
        Self::at(std::env::current_exe().ok()?)
    }

    /// The executable at `path`, as it is now.
    #[must_use]
    pub fn at(path: PathBuf) -> Option<Self> {
        let modified = std::fs::metadata(&path).ok()?.modified().ok();
        Some(Self { path, modified })
    }

    /// The `AKR-X043` diagnostic when the file has been replaced or removed since
    /// [`Executable::current`] looked at it.
    #[must_use]
    pub fn replaced(&self) -> Option<Value> {
        let now = std::fs::metadata(&self.path)
            .ok()
            .and_then(|metadata| metadata.modified().ok());
        if now.is_some() && now == self.modified {
            return None;
        }
        Some(Value::object(vec![
            ("code", Value::string("AKR-X043")),
            ("severity", Value::string("warning")),
            (
                "message",
                Value::string(format!(
                    "{} was replaced after this akr-mcp started; this process is still the                      old binary, so a refusal here may be one the current release does not                      make. RECONNECT the MCP server, or run the same command with the akr CLI.",
                    self.path.display()
                )),
            ),
            (
                "help",
                Value::string("a running MCP process keeps the binary it started with"),
            ),
        ]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_replaced_executable_is_reported_and_an_untouched_one_is_not() {
        let path = std::env::temp_dir().join(format!("akr-mcp-skew-{}", std::process::id()));
        std::fs::write(&path, b"old").unwrap();
        let earlier = SystemTime::now() - std::time::Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(earlier)
            .unwrap();
        let started = Executable::at(path.clone()).unwrap();
        assert!(started.replaced().is_none());
        std::fs::write(&path, b"new").unwrap();
        let diagnostic = started.replaced().expect("a rewritten binary is reported");
        assert_eq!(
            diagnostic.get("code").and_then(Value::as_str),
            Some("AKR-X043")
        );
        std::fs::remove_file(&path).unwrap();
        assert!(started.replaced().is_some(), "a removed binary is reported");
    }

    const LOCK: &str = "akr-lock 0.1\nproject akr\n\nbuild {\n    tool \"akr 0.1.0\"\n    \
                        grammar \"0.1\"\n    vocabulary \"0.9\"\n}\n";

    #[test]
    fn a_differing_vocabulary_is_read_off_the_lock() {
        assert_eq!(vocabulary_of(LOCK).as_deref(), Some("0.9"));
    }

    #[test]
    fn a_lock_without_a_build_block_is_not_a_skew() {
        assert_eq!(vocabulary_of("akr-lock 0.1\nproject akr\n"), None);
    }

    #[test]
    fn the_message_names_the_remedy_and_the_reconnect() {
        let skew = Skew {
            workspace: "0.3".into(),
            server: "0.2".into(),
        };
        let message = skew.message();
        assert!(
            message.contains("0.3") && message.contains("0.2"),
            "{message}"
        );
        // The half everybody forgets: reinstalling does not restart the running process.
        assert!(message.contains("RECONNECT"), "{message}");
    }
}

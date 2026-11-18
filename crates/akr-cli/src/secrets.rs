//! Supported CLI/MCP workflow for the local-only secret category.

use crate::commands::Output;
use crate::session::{EnvError, Session};
use akr_core::{json::Value, secrets};
use std::path::Path;

/// Rejects private storage as an input to a public write or read workflow.
///
/// # Errors
/// Returns `AKR-C046` without file contents.
pub fn public_input(root: &Path, path: &Path) -> Result<(), EnvError> {
    let check = |path| {
        secrets::require_public_input(root, path).map_err(|_| {
            EnvError::new(
                "AKR-C046",
                "private secret storage cannot be used as public input",
            )
        })
    };
    check(path)?;
    if let Ok(actual) = std::fs::canonicalize(path) {
        check(&actual)?;
    }
    Ok(())
}

fn error(error: std::io::Error) -> EnvError {
    EnvError::new(
        "AKR-C045",
        format!("private secret operation failed: {error}"),
    )
}

/// Refuses an index that includes private entries, including files force-added to Git.
/// The guard inspects names only; it never reads or prints private file contents.
///
/// # Errors
/// Returns `AKR-C046` for tracked secrets, or a Git environment error.
pub fn ensure_untracked(session: &Session) -> Result<(), EnvError> {
    if let Some(repository) = &session.repository {
        let entries = repository
            .staged_entries()
            .map_err(|e| EnvError::new("AKR-G001", e.to_string()))?;
        if entries
            .iter()
            .any(|entry| secrets::is_private_path(&entry.path))
        {
            return Err(EnvError::new("AKR-C046", "private secret storage is in the Git index; refusing publication")
                .help("remove .akr/secrets from the index with git rm --cached; if values were already published, rotate them and remove them from history"));
        }
    }
    Ok(())
}

/// Stores a UTF-8 input file without echoing its name or its contents.
///
/// # Errors
/// Returns a storage or Git error, never a diagnostic containing the input contents.
pub fn put(session: &Session, from: &Path) -> Result<Output, EnvError> {
    ensure_untracked(session)?;
    let path = session.root.join(from);
    let value = std::fs::read_to_string(path).map_err(error)?;
    let reference = secrets::put(&session.root, &value).map_err(error)?;
    Ok(Output::plain(
        format!("{reference}\n"),
        Value::object(vec![
            ("category", Value::string("secret")),
            ("citation", Value::string(reference)),
            ("written", Value::bool(true)),
        ]),
    ))
}

/// Returns one value only when reveal was explicitly requested; otherwise returns `[*]`.
///
/// # Errors
/// Returns a storage or Git error on an explicit reveal of unusable private storage.
pub fn get(session: &Session, reference: &str, reveal: bool) -> Result<Output, EnvError> {
    let value = if reveal {
        ensure_untracked(session)?;
        secrets::get(&session.root, reference)
            .map_err(error)?
            .unwrap_or_else(|| secrets::REDACTED.to_owned())
    } else {
        secrets::REDACTED.to_owned()
    };
    Ok(Output::plain(
        format!("{value}\n"),
        Value::object(vec![("value", Value::string(value))]),
    ))
}

/// Resolves citations in a local input document, with public rendering as the default.
///
/// # Errors
/// Returns a storage or Git error; the input document is never included in diagnostics.
pub fn resolve(session: &Session, from: &Path, reveal: bool) -> Result<Output, EnvError> {
    public_input(&session.root, from)?;
    if reveal {
        ensure_untracked(session)?;
    }
    let text = std::fs::read_to_string(session.root.join(from)).map_err(error)?;
    let text = secrets::resolve(&session.root, &text, reveal).map_err(error)?;
    Ok(Output::plain(
        text.clone(),
        Value::object(vec![("text", Value::string(text))]),
    ))
}

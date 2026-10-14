//! Supported configuration reads and validated writes.
use crate::{
    commands::Output,
    session::{EnvError, Session},
};
use akr_core::{json::Value, ops::ProjectEdit};

/// Reads the project configuration without accessing private caches.
///
/// # Errors
/// Returns an environment error if `project.akr` cannot be read.
pub fn show(session: &Session) -> Result<Output, EnvError> {
    let text = std::fs::read_to_string(session.akr_dir.join("project.akr"))
        .map_err(|error| EnvError::new("AKR-C042", format!("cannot read project.akr: {error}")))?;
    Ok(Output::plain(
        &text,
        Value::object(vec![("configuration", Value::string(&text))]),
    ))
}

/// Applies a configuration edit through the core write pipeline.
///
/// # Errors
/// Returns an environment error if the result cannot be rendered.
pub fn edit(session: &Session, edit: &ProjectEdit) -> Result<Output, EnvError> {
    crate::write::render(
        session,
        akr_core::ops::configure(&crate::write::context_of(session), edit),
    )
}

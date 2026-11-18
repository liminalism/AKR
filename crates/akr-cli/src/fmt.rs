//! `akr fmt`: canonical formatting, and `--check`.
//!
//! Writes go through the same rule as every other write (`docs/07-cli.md` §4): parse,
//! re-emit, and replace the file only when the bytes differ, so a formatted workspace
//! produces no diff and no mtime churn.

use crate::commands::Output;
use crate::session::{EnvError, Exit, Session, report};
use akr_core::diagnostics::Diagnostic;
use akr_core::json::Value;
use std::path::{Path, PathBuf};

/// Formats, or checks formatting.
///
/// # Errors
/// [`EnvError`] for an unreadable or unwritable file.
pub fn run(session: &Session, check: bool, paths: &[PathBuf]) -> Result<Output, EnvError> {
    let targets: Vec<PathBuf> = if paths.is_empty() {
        session
            .inputs
            .sources
            .iter()
            .map(|source| session.root.join(&source.path))
            .chain(std::iter::once(session.akr_dir.join("akr.lock")))
            .filter(|path| path.is_file())
            .collect()
    } else {
        paths
            .iter()
            .map(|path| {
                if path.is_relative() {
                    session.root.join(path)
                } else {
                    path.clone()
                }
            })
            .collect()
    };

    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut changed = Vec::new();
    // One source map for the whole run: the file ids inside the diagnostics have to resolve
    // against the map `report` is later handed, and `session.sources` does not contain the
    // lock or any path named on the command line.
    let mut sources = akr_core::diagnostics::SourceMap::new();
    for path in &targets {
        let text = std::fs::read_to_string(path).map_err(|e| {
            EnvError::new("AKR-C042", format!("cannot read {}: {e}", path.display()))
        })?;
        let relative = relative(&session.root, path);
        let file = sources.add(&relative, &text);

        if check {
            diagnostics.extend(akr_core::syntax::check_formatted(&text, file, &relative));
            continue;
        }
        let (formatted, parse_diagnostics) = akr_core::syntax::format_source(&text, file);
        diagnostics.extend(parse_diagnostics);
        if let Some(formatted) = formatted
            && formatted != text
        {
            std::fs::write(path, &formatted).map_err(|e| {
                EnvError::new("AKR-C042", format!("cannot write {}: {e}", path.display()))
            })?;
            changed.push(relative);
        }
    }

    let fatal = !diagnostics.is_empty();
    let mut text = String::new();
    if check {
        if diagnostics.is_empty() {
            text.push_str(&format!(
                "{} files are canonically formatted\n",
                targets.len()
            ));
        } else {
            let (rendered, _) = report(&diagnostics, &sources, session.global.profile);
            text.push_str(&rendered);
            text.push_str(&format!("{} file(s) need formatting\n", diagnostics.len()));
        }
    } else if changed.is_empty() {
        text.push_str(&format!("{} files unchanged\n", targets.len()));
    } else {
        for path in &changed {
            text.push_str(&format!("formatted {path}\n"));
        }
    }

    Ok(Output {
        text,
        result: Value::object(vec![(
            "changed",
            Value::array(changed.iter().map(Value::string).collect()),
        )]),
        diagnostics,
        exit: if fatal { Exit::Diagnostics } else { Exit::Ok },
        commit: None,
        source_graph: String::new(),
    })
}

/// The path a diagnostic about a file should name.
#[must_use]
pub fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

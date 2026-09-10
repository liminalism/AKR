//! `akr check --views-current`: the CI gate of D-025.
//!
//! Stage F renders every view **in memory** and compares it against what is committed. A
//! difference is `AKR-E011`, a missing file `AKR-E012`, a damaged banner `AKR-E013`, and
//! an unexpected file in the output directory `AKR-E014`
//! (`docs/11-projections.md` §11).
//!
//! That comparison is what gives `sys.policy.no-hand-edited-views` actual force rather
//! than good intentions. A hand edit fails the build with the file and line named; a
//! ledger change without a rebuild fails the same way, with the same fix.

use super::codes;
use super::{RenderContext, View, render};
use crate::diagnostics::{Diagnostic, RuleId, Subject};
use std::collections::BTreeSet;
use std::path::Path;
use std::{fs, io};

/// V-112, V-113 and V-114 all belong to the projection rule range
/// (`docs/11-projections.md` §13).
const V112: RuleId = RuleId(112);
const V113: RuleId = RuleId(113);
const V114: RuleId = RuleId(114);

/// Compares every renderable view against the files in `dir`.
///
/// Returns diagnostics in a deterministic order: catalogue order for the per-view checks,
/// then path order for unexpected files.
///
/// [`render`] returns `None` for [`View::Papercuts`] alone, when the ledger holds no
/// papercut (D-027) — that file is expected to be absent, not reported as missing.
///
/// # Errors
/// Returns any I/O error from reading the output directory.
pub fn check_views_current(dir: &Path, cx: RenderContext<'_>) -> io::Result<Vec<Diagnostic>> {
    let mut out = Vec::new();
    let mut expected: BTreeSet<String> = BTreeSet::new();

    for &view in View::ALL {
        expected.insert(view.file_name().to_owned());
        let Some(rendered) = render(view, cx) else {
            continue;
        };
        let path = dir.join(view.file_name());
        let display = display_path(&path);

        let Ok(committed) = fs::read_to_string(&path) else {
            out.push(
                Diagnostic::error(
                    codes::E012,
                    V112,
                    Subject::File(display.clone()),
                    format!("{display} is missing; run `akr build`"),
                )
                .help("generated views are committed (D-025)"),
            );
            continue;
        };

        if let Some(problem) = banner_problem(&committed) {
            out.push(
                Diagnostic::error(
                    codes::E013,
                    V113,
                    Subject::File(display.clone()),
                    format!("{display} does not begin with a well-formed AKR banner: {problem}"),
                )
                .help("run `akr build`; the banner is written by the renderer, never by hand"),
            );
            continue;
        }

        if committed != rendered {
            let (line, committed_line, rendered_line) = first_difference(&committed, &rendered);
            let differing = differing_lines(&committed, &rendered);

            // A view whose ONLY difference is the banner's `tool:` line was rendered by an
            // older binary against the same ledger. Its content is current; the stamp is
            // not. That is a migration artifact which disappears the first time anyone
            // rebuilds, and it is not the failure D-025 exists to catch — so it is
            // AKR-E015 at warning severity, exempt from strict promotion, rather than
            // AKR-E011. Ten of fifteen workspaces in one 2026-09 sweep were in exactly
            // this state, every one of them reporting "no diagnostics" from a plain check.
            if tool_line_only(&committed, &rendered) {
                out.push(
                    Diagnostic::warning(
                        codes::E015,
                        V112,
                        Subject::File(display.clone()),
                        format!(
                            "{display} was rendered by a different `akr` version;                              its content matches the ledger but its banner does not"
                        ),
                    )
                    .note(crate::diagnostics::Label::with_message(
                        Subject::File(display.clone()),
                        format!("line {line} committed: {committed_line:?}"),
                    ))
                    .note(crate::diagnostics::Label::with_message(
                        Subject::File(display.clone()),
                        format!("line {line} emitted:   {rendered_line:?}"),
                    ))
                    .help("run `akr build` and commit the result; nothing else in this view differs"),
                );
                continue;
            }

            out.push(
                Diagnostic::error(
                    codes::E011,
                    V112,
                    Subject::File(display.clone()),
                    format!(
                        "{display} differs from the view this build would emit \
                         ({differing} differing line{})",
                        if differing == 1 { "" } else { "s" }
                    ),
                )
                .note(crate::diagnostics::Label::with_message(
                    Subject::File(display.clone()),
                    format!("line {line} committed: {committed_line:?}"),
                ))
                .note(crate::diagnostics::Label::with_message(
                    Subject::File(display.clone()),
                    format!("line {line} emitted:   {rendered_line:?}"),
                ))
                .help(
                    "run `akr build` and commit the result; see @sys.policy.no-hand-edited-views",
                ),
            );
        }
    }

    // The output directory is owned by the build (§11, V-114).
    if dir.is_dir() {
        let mut entries: Vec<_> = fs::read_dir(dir)?
            .collect::<io::Result<Vec<_>>>()?
            .into_iter()
            .map(|e| e.path())
            .collect();
        entries.sort();
        for entry in entries {
            let name = entry
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if entry.is_dir() || expected.contains(&name) {
                continue;
            }
            let display = display_path(&entry);
            out.push(
                Diagnostic::error(
                    codes::E014,
                    V114,
                    Subject::File(display.clone()),
                    format!(
                        "{display} is in the view output directory but is not a generated view"
                    ),
                )
                .help("the view output directory is owned by `akr build`; move the file elsewhere"),
            );
        }
    }

    Ok(out)
}

/// Writes every renderable view into `dir`, creating it if needed.
///
/// Only files whose bytes differ are rewritten, so an unchanged view keeps its mtime and
/// a no-op build produces no diff (`docs/06-compiler-pipeline.md` §13 step 10). Returns
/// the views that were actually written.
///
/// # Errors
/// Returns any I/O error from creating the directory or writing a file.
pub fn write_views(dir: &Path, cx: RenderContext<'_>) -> io::Result<Vec<View>> {
    fs::create_dir_all(dir)?;
    let mut written = Vec::new();
    for &view in View::ALL {
        let Some(rendered) = render(view, cx) else {
            continue;
        };
        let path = dir.join(view.file_name());
        if fs::read_to_string(&path).is_ok_and(|existing| existing == rendered) {
            continue;
        }
        fs::write(&path, &rendered)?;
        written.push(view);
    }
    Ok(written)
}

/// Why a file's banner is not well formed, or `None` when it is.
///
/// The banner must be on line 1 and carry all three fields (§4). A view whose banner was
/// edited, truncated or moved cannot be told from a hand-written document, which is the
/// whole reason the check exists.
fn banner_problem(text: &str) -> Option<&'static str> {
    let mut lines = text.lines();
    if lines.next() != Some("<!-- GENERATED BY AKR — DO NOT EDIT") {
        return Some("line 1 is not the banner opener");
    }
    let mut fields = [false; 2];
    for line in lines.by_ref().take(2) {
        let trimmed = line.trim_start();
        if trimmed.starts_with("source-graph: sha256:") {
            fields[0] = true;
        } else if trimmed.starts_with("tool: ") {
            fields[1] = true;
        }
    }
    if !fields[0] {
        return Some("no `source-graph:` field");
    }
    if !fields[1] {
        return Some("no `tool:` field");
    }
    if lines.next() != Some("-->") {
        return Some("the banner is not closed on line 5");
    }
    None
}

/// Whether the only difference between two renderings is the banner's `tool:` line.
///
/// The banner occupies the first four lines (§4), so a `tool:` field outside it is not a
/// banner field and does not qualify — a body line that happens to start with `tool: `
/// must still be a hard difference.
fn tool_line_only(committed: &str, rendered: &str) -> bool {
    let left: Vec<&str> = committed.lines().collect();
    let right: Vec<&str> = rendered.lines().collect();
    if left.len() != right.len() {
        return false;
    }
    let mut differing = left
        .iter()
        .zip(right.iter())
        .enumerate()
        .filter(|(_, (a, b))| a != b);
    let Some((index, (a, b))) = differing.next() else {
        return false;
    };
    differing.next().is_none()
        && index < 4
        && a.trim_start().starts_with("tool: ")
        && b.trim_start().starts_with("tool: ")
}

/// The 1-based line number of the first difference, with both sides' text.
///
/// The useful question is "was this a hand edit or a stale build?", and the answer is
/// usually visible in one line of context (§11).
fn first_difference(committed: &str, rendered: &str) -> (usize, String, String) {
    let mut left = committed.lines();
    let mut right = rendered.lines();
    let mut line = 0usize;
    loop {
        line += 1;
        match (left.next(), right.next()) {
            (None, None) => return (line, String::new(), String::new()),
            (a, b) if a == b => {}
            (a, b) => {
                return (
                    line,
                    a.unwrap_or("<end of file>").to_owned(),
                    b.unwrap_or("<end of file>").to_owned(),
                );
            }
        }
    }
}

/// How many lines differ, counting a length difference as a difference per line.
fn differing_lines(committed: &str, rendered: &str) -> usize {
    let left: Vec<&str> = committed.lines().collect();
    let right: Vec<&str> = rendered.lines().collect();
    let common = left.len().min(right.len());
    let mismatched = (0..common).filter(|&i| left[i] != right[i]).count();
    mismatched + left.len().abs_diff(right.len())
}

/// A path for a diagnostic subject: repo-relative where possible, forward slashes.
/// A path as a diagnostic should name it: forward slashes, and relative to the working
/// directory when it is under it, so `docs/generated/ROADMAP.md` rather than an absolute
/// path that says nothing a reader needs.
fn display_path(path: &Path) -> String {
    // On Windows a canonicalised workspace path carries the `\\?\` verbatim prefix while
    // `current_dir` does not, so the prefixes must be compared like with like or the
    // diagnostic degrades to an absolute path.
    let relative = std::env::current_dir().ok().and_then(|cwd| {
        path.strip_prefix(&cwd)
            .ok()
            .map(Path::to_path_buf)
            .or_else(|| {
                let canonical = cwd.canonicalize().ok()?;
                path.strip_prefix(canonical).ok().map(Path::to_path_buf)
            })
    });
    let path = relative.as_deref().unwrap_or(path);
    path.to_string_lossy().replace('\\', "/")
}

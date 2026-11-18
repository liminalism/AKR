//! `akr init`: scaffolding a workspace.
//!
//! Creates `.akr/project.akr`, `.akr/records/`, `.akr/archive/`, the `AGENTS.md` protocol
//! text, and the two `.gitignore` entries. Never overwrites: an existing `.akr/` is
//! `AKR-C013` (`docs/07-cli.md` §6).

use crate::commands::Output;
use crate::session::{EnvError, TOOL_VERSION};
use akr_core::json::Value;
use akr_core::model::Segment;
use std::path::Path;

/// The agent brief: `scripts/agent-section.md`, embedded (D-049).
///
/// There used to be a second, older text here, and a third copy in `docs/08-mcp.md`. The
/// setup scripts install the file; `akr init` wrote this constant; the two drifted, and
/// the one a new workspace got never mentioned the commit protocol at all. One file, and
/// every installer reads it.
///
/// Protocol only — no philosophy, no data model, no examples — because an agent reads it
/// every session and every extra line competes with the task.
pub const AGENTS_TEMPLATE: &str = include_str!("../../../scripts/agent-section.md");

/// The marker the setup scripts and `akr init` open the brief with.
pub const AGENTS_BEGIN: &str = "<!-- AKR_START -->";
/// The marker that closes it.
pub const AGENTS_END: &str = "<!-- AKR_END -->";

/// The brief between its markers, with `\n` line endings whatever the checkout used.
///
/// The markers are what let `scripts/setup-akr-mcp` refresh the block in place later, so
/// a workspace scaffolded by `init` is updated by the same rewrite as one set up by hand.
#[must_use]
pub fn agents_section() -> String {
    format!(
        "{AGENTS_BEGIN}\n{}\n{AGENTS_END}\n",
        AGENTS_TEMPLATE.replace("\r\n", "\n").trim_end()
    )
}

/// The paths a workspace must not track: the rebuildable index cache, the two
/// disposable agent subtrees (D-036, D-040), and local-only secret entries.
pub(crate) const GITIGNORE_ENTRIES: &[&str] = &[
    ".akr/cache/",
    ".agent/scratch/",
    ".agent/handoffs/",
    ".akr/secrets/",
];

/// The entries `init` would write that `root`'s `.gitignore` does not have.
///
/// This list is project setup AKR authors on the project's behalf, and it is written once
/// at `init` and never reconciled: a workspace scaffolded before `.agent/handoffs/` was
/// added keeps the two-entry block forever. Checked on 2026-09-08, four of five sampled
/// workspaces were missing it, one of them carrying 49 untracked handoff packets as a
/// result — permanently noisy in `git status` and one `git add -A` from being committed
/// by accident. Nobody re-runs `init` on a workspace that already works, so the
/// reconciliation has to happen where every session already looks.
pub(crate) fn missing_gitignore_entries(root: &std::path::Path) -> Vec<&'static str> {
    let Ok(text) = std::fs::read_to_string(root.join(".gitignore")) else {
        return Vec::new();
    };
    let present: std::collections::BTreeSet<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    GITIGNORE_ENTRIES
        .iter()
        .filter(|entry| !present.contains(*entry))
        .copied()
        .collect()
}

/// Scaffolds a workspace in the current directory.
///
/// # Errors
/// [`EnvError`] `AKR-C013` when `.akr/` exists, `AKR-C023` for a malformed project name,
/// and `AKR-C042` for an unwritable directory.
pub fn run(project: Option<&str>, namespaces: &[String], hooks: bool) -> Result<Output, EnvError> {
    let root = std::env::current_dir().map_err(|e| {
        EnvError::new(
            "AKR-C042",
            format!("cannot read the current directory: {e}"),
        )
    })?;
    run_in(&root, project, namespaces, hooks)
}

/// Scaffolds a workspace in a named directory, so tests need no `chdir`.
///
/// # Errors
/// As [`run`].
pub fn run_in(
    root: &Path,
    project: Option<&str>,
    namespaces: &[String],
    hooks: bool,
) -> Result<Output, EnvError> {
    let akr = root.join(".akr");
    if akr.exists() {
        return Err(EnvError::new(
            "AKR-C013",
            format!("{}/.akr already exists", root.display()),
        )
        .help("`akr init` never overwrites"));
    }

    let name = project
        .map(ToOwned::to_owned)
        .or_else(|| {
            root.file_name()
                .map(|n| n.to_string_lossy().replace('_', "-").to_lowercase())
        })
        .unwrap_or_else(|| "project".to_owned());
    if Segment::new(&name).is_err() {
        return Err(EnvError::new(
            "AKR-C023",
            format!("project name {name:?} is not in key-segment form"),
        )
        .help("lowercase ASCII, digits and internal hyphens (D-005)"));
    }
    for namespace in namespaces {
        if Segment::new(namespace).is_err() {
            return Err(EnvError::new(
                "AKR-C023",
                format!("namespace {namespace:?} is not in key-segment form"),
            ));
        }
    }

    let mut created = Vec::new();
    let mkdir = |path: &Path| -> Result<(), EnvError> {
        std::fs::create_dir_all(path).map_err(|e| {
            EnvError::new("AKR-C042", format!("cannot create {}: {e}", path.display()))
        })
    };
    mkdir(&akr.join("records"))?;
    mkdir(&akr.join("archive"))?;
    created.push(".akr/records/".to_owned());
    created.push(".akr/archive/".to_owned());

    let mut project_text = format!("akr 0.1\nproject {name}\n");
    if namespaces.is_empty() {
        project_text.push_str(&format!("\nnamespace {name} \"Project-wide knowledge.\"\n"));
    } else {
        project_text.push('\n');
        for namespace in namespaces {
            project_text.push_str(&format!("namespace {namespace} \"TODO: describe.\"\n"));
        }
    }
    project_text.push_str(
        "\ndefaults {\n    review_after_days 90\n    view_output \"docs/generated\"\n}\n",
    );
    write(&akr.join("project.akr"), &project_text)?;
    created.push(".akr/project.akr".to_owned());

    let agents = root.join("AGENTS.md");
    if agents.exists() {
        let existing = std::fs::read_to_string(&agents).unwrap_or_default();
        // Either marker, or the heading every earlier brief opened with: the file
        // already carries a brief, and replacing it is the setup scripts' job.
        if !existing.contains(AGENTS_BEGIN)
            && !existing.contains("## Project knowledge (AKR)")
            && !existing.contains("## AKR (project knowledge)")
        {
            let mut merged = existing;
            if !merged.ends_with('\n') {
                merged.push('\n');
            }
            merged.push('\n');
            merged.push_str(&agents_section());
            write(&agents, &merged)?;
            created.push("AGENTS.md (appended)".to_owned());
        }
    } else {
        write(
            &agents,
            &format!("# Agent protocol\n\n{}", agents_section()),
        )?;
        created.push("AGENTS.md".to_owned());
    }

    let gitignore = root.join(".gitignore");
    let existing = std::fs::read_to_string(&gitignore).unwrap_or_default();
    let missing: Vec<&str> = GITIGNORE_ENTRIES
        .iter()
        .copied()
        .filter(|entry| !existing.lines().any(|line| line.trim() == *entry))
        .collect();
    if !missing.is_empty() {
        let mut merged = existing;
        if !merged.is_empty() && !merged.ends_with('\n') {
            merged.push('\n');
        }
        merged.push_str(
            "\n# AKR: rebuildable cache, disposable agent notes and local-only secrets.\n",
        );
        for entry in &missing {
            merged.push_str(&format!("{entry}\n"));
        }
        write(&gitignore, &merged)?;
        created.push(format!(".gitignore ({} entries)", missing.len()));
    }

    // D-049: the hooks that notice a commit made outside the change protocol. Absent
    // hooks were the default for as long as installing them was a separate command, and
    // the measured result was that the protocol was not followed at all. A directory
    // that is not a repository simply has none to install.
    let mut hook_lines = String::new();
    if hooks && let Ok(repository) = akr_core::git::Repository::open(root) {
        match crate::change::write_hooks(&repository, false) {
            Ok(report) => {
                if !report.written.is_empty() {
                    created.push(format!("git hooks ({})", report.written.join(", ")));
                }
                for hook in &report.kept {
                    hook_lines.push_str(&format!(
                        "kept your existing {hook} hook; `akr git install-hooks` replaces it\n"
                    ));
                }
            }
            Err(error) => hook_lines.push_str(&format!(
                "git hooks were not installed: {}; run `akr git install-hooks` later\n",
                error.message
            )),
        }
    }

    let mut text = String::new();
    for item in &created {
        text.push_str(&format!("created {item}\n"));
    }
    text.push_str(&hook_lines);
    text.push_str(&format!(
        "\nakr {TOOL_VERSION} — next: write a record, then run `akr check`. Commit through \
         `akr change begin`, `akr change prepare --staged`, `akr git commit`\n"
    ));
    Ok(Output {
        text,
        result: Value::object(vec![
            ("project", Value::string(name)),
            (
                "created",
                Value::array(created.iter().map(Value::string).collect()),
            ),
        ]),
        diagnostics: Vec::new(),
        exit: crate::session::Exit::Ok,
        commit: None,
        source_graph: String::new(),
    })
}

fn write(path: &Path, text: &str) -> Result<(), EnvError> {
    std::fs::write(path, text)
        .map_err(|e| EnvError::new("AKR-C042", format!("cannot write {}: {e}", path.display())))
}

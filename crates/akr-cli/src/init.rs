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

/// The `AGENTS.md` section `docs/08-mcp.md` §8 fixes, quoted there in full.
///
/// Deliberately protocol only — no philosophy, no data model, no examples — because an
/// agent reads it every session and every extra line competes with the task. Everything
/// beyond this is reachable through the tools themselves.
pub const AGENTS_TEMPLATE: &str = r#"## Project knowledge (AKR)

Durable project knowledge lives in `.akr/` as typed records, not in Markdown.
`docs/generated/` is build output. Follow this protocol.

**Before starting any task**
1. If you know the exact planning key, call `knowledge.context` with that key and the
   paths you expect to touch.
2. Otherwise call `knowledge.start` with the task and paths. Read its session head, pick
   a live candidate (or an explicitly relevant proposal), then call `knowledge.context`
   with that exact key.
3. Read context bundles in full. Contradictions and staleness warnings are always
   included and are never noise.

**While working**
- Look things up with `knowledge.get`; find them with `knowledge.search`.
  Search ranks results; it never grants authority. A record's standing comes from its
  state, its scope, and its relations.
- Scratch notes go in `.agent/scratch/`. Nobody reviews them and nothing depends on them
  — but **nothing empties it either**. It is a gitignored directory inside the repository,
  not the OS temp directory and not `target/`, so it survives every session and grows
  until somebody deletes it by hand. Before handing work back, run `akr scratch prune`,
  and `akr scratch keep <name> --reason "<why>"` for anything the next session needs.
  `akr check` reports the total; `akr check --scratch-clean` fails when anything prunable
  is left.

**When something becomes durable**
- New knowledge: `knowledge.propose`. Observations need `observed_at` and, if they can
  go out of date, `watches`.
- Changed knowledge: `knowledge.revise`. Never edit a `.akr` file directly, and never
  edit a record that is not `proposed`.
- Replacing a plan: `knowledge.supersede`, with a disposition for every unfinished
  child. The tool will list them; answer each one.
- Finishing work: record what you observed with `knowledge.evidence_add`, then
  `knowledge.complete` with evidence for every acceptance check. Evidence records
  state what was observed; they never state what they verify.
- Unsure what a kind requires? `akr explain <kind>` prints its schema.

**Papercuts**
- When you hit a small friction while working — a tool call that missed and had to be
  retried, a confusing or undocumented setup step, a flaky command, a stale cache, a
  misleading error, a non-obvious gotcha — log it with `knowledge.papercut` (or
  `akr papercut -m <agent> "message"`). One or two sentences: what you were doing,
  what got in the way (a guess at the cause/fix is a bonus). Do this proactively, in
  the moment, even though none of these are blocking — logged together they show where
  the project needs sanding down. This is distinct from durable records (knowledge) and
  from `.agent/scratch/` (working notes).

**Never**
- Never edit `docs/generated/` — it is regenerated and CI checks it.
- Never read `.akr/cache/` — it is a private cache.
- Never delete a record. Move it to a terminal state instead.

**Before handing back**
- `knowledge.validate`. If it reports diagnostics, fix them or say so explicitly.
"#;

/// The paths a workspace must not track: the rebuildable index cache, and the two
/// disposable agent subtrees (D-036, D-040).
const GITIGNORE_ENTRIES: &[&str] = &[".akr/cache/", ".agent/scratch/", ".agent/handoffs/"];

/// Scaffolds a workspace in the current directory.
///
/// # Errors
/// [`EnvError`] `AKR-C013` when `.akr/` exists, `AKR-C023` for a malformed project name,
/// and `AKR-C042` for an unwritable directory.
pub fn run(project: Option<&str>, namespaces: &[String]) -> Result<Output, EnvError> {
    let root = std::env::current_dir().map_err(|e| {
        EnvError::new(
            "AKR-C042",
            format!("cannot read the current directory: {e}"),
        )
    })?;
    run_in(&root, project, namespaces)
}

/// Scaffolds a workspace in a named directory, so tests need no `chdir`.
///
/// # Errors
/// As [`run`].
pub fn run_in(
    root: &Path,
    project: Option<&str>,
    namespaces: &[String],
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
        if !existing.contains("## Project knowledge (AKR)") {
            let mut merged = existing;
            if !merged.ends_with('\n') {
                merged.push('\n');
            }
            merged.push('\n');
            merged.push_str(AGENTS_TEMPLATE);
            write(&agents, &merged)?;
            created.push("AGENTS.md (appended)".to_owned());
        }
    } else {
        write(&agents, &format!("# Agent protocol\n\n{AGENTS_TEMPLATE}"))?;
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
        merged.push_str("\n# AKR: a rebuildable cache and disposable agent notes.\n");
        for entry in &missing {
            merged.push_str(&format!("{entry}\n"));
        }
        write(&gitignore, &merged)?;
        created.push(format!(".gitignore ({} entries)", missing.len()));
    }

    let mut text = String::new();
    for item in &created {
        text.push_str(&format!("created {item}\n"));
    }
    text.push_str(&format!(
        "\nakr {TOOL_VERSION} — next: write a record, then run `akr check`\n"
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

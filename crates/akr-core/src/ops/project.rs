//! Project configuration uses the same locked validation/commit boundary as records.
use super::*;
use crate::diagnostics::{FileId, Span};
use crate::model::{Glob, Segment};
use crate::syntax::{
    cst::{BodyItem, Item, Value},
    parse,
};

/// A project configuration edit, applied after acquiring the workspace write lock.
#[derive(Debug, Clone)]
pub enum ProjectEdit {
    /// Replace with a complete project file; records and project renames are refused.
    Replace(String),
    /// Add a deliberately untracked path, leaving other declarations untouched.
    AddUntracked(String),
    /// Remove an exact declaration, leaving other paths and comments untouched.
    RemoveUntracked(String),
}

/// Edits `project.akr`, validates the resulting ledger, then atomically writes it.
///
/// # Errors
/// Returns a refusal without writing if parsing, validation, or concurrency checks fail.
pub fn configure(context: &WriteContext, edit: &ProjectEdit) -> WriteResult {
    let operation = Operation::Configure;
    let (mut staged, _lock) = load_locked(context, operation)?;
    let path = PathBuf::from("project.akr");
    let original = staged.texts.get(&path).cloned();
    let mut tree = match edit {
        ProjectEdit::Replace(text) => {
            let parsed = parse(text, FileId(0));
            if !parsed.diagnostics.is_empty() || parsed.file.is_none() {
                let mut refusal = refused("configuration does not parse");
                refusal.diagnostics = parsed
                    .diagnostics
                    .into_iter()
                    .map(|mut diagnostic| {
                        // The caller's source map holds the old workspace, not this input.
                        diagnostic.primary.span = None;
                        diagnostic
                    })
                    .collect();
                return Err(refusal);
            }
            parsed.file.expect("checked above")
        }
        ProjectEdit::AddUntracked(_) | ProjectEdit::RemoveUntracked(_) => staged
            .trees
            .get(&path)
            .cloned()
            .ok_or_else(|| refused("current configuration does not parse"))?,
    };
    if tree.keyword != "akr" || tree.project != staged.ledger.project.name {
        return Err(refused(
            "configuration must retain the current project name and AKR header",
        ));
    }
    match edit {
        ProjectEdit::AddUntracked(glob) => {
            validate_path(glob)?;
            let already = tree.items.iter().any(|item| matches!(item,
                Item::Block(block) if block.name == "untracked" && block.body.iter().any(|item| path_is(item, glob))));
            if !already {
                let mut block = match parse(
                    "akr 0.1\nproject config\nuntracked { path \"placeholder\" }\n",
                    FileId(0),
                )
                .file
                .expect("static project")
                .items
                .remove(0)
                {
                    Item::Block(block) => block,
                    _ => unreachable!("static block"),
                };
                let BodyItem::Slot(slot) = &mut block.body[0] else {
                    unreachable!("static slot")
                };
                slot.value = Value::Str(
                    glob.clone(),
                    Span {
                        file: FileId(0),
                        start: 0,
                        end: 0,
                    },
                );
                if let Some(Item::Block(existing)) = tree.items.iter_mut().find(|item| {
                    matches!(item,
                    Item::Block(block) if block.name == "untracked")
                }) {
                    existing.body.extend(block.body);
                } else {
                    tree.items.push(Item::Block(block));
                }
            }
        }
        ProjectEdit::RemoveUntracked(glob) => {
            validate_path(glob)?;
            for item in &mut tree.items {
                if let Item::Block(block) = item
                    && block.name == "untracked"
                {
                    block.body.retain(|item| !path_is(item, glob));
                }
            }
        }
        ProjectEdit::Replace(_) => {}
    }
    let mut namespaces = BTreeSet::new();
    for item in &tree.items {
        match item {
            Item::Namespace(namespace) => {
                if Segment::new(&namespace.name).is_err() || !namespaces.insert(&namespace.name) {
                    return Err(refused(
                        "configuration needs valid, unique namespace declarations",
                    ));
                }
            }
            Item::Block(block) if block.name == "untracked" && block.head.is_none() => {
                for item in &block.body {
                    match item {
                        BodyItem::Slot(slot) if slot.name == "path" => match &slot.value {
                            Value::Str(path, _) => validate_path(path)?,
                            _ => return Err(refused("untracked path must be a quoted glob")),
                        },
                        _ => return Err(refused("untracked accepts only path declarations")),
                    }
                }
            }
            Item::Block(block) if block.name == "defaults" && block.head.is_none() => {
                let mut keys = BTreeSet::new();
                for item in &block.body {
                    let BodyItem::Slot(slot) = item else {
                        return Err(refused("defaults accepts only setting slots"));
                    };
                    if !keys.insert(&slot.name) {
                        return Err(refused("defaults settings must be unique"));
                    }
                    let valid = match (slot.name.as_str(), &slot.value) {
                        ("review_after_days", Value::Scalar(days, _)) => {
                            days.parse::<u64>().is_ok()
                        }
                        ("view_output", Value::Str(path, _)) => !path.trim().is_empty(),
                        _ => false,
                    };
                    if !valid {
                        return Err(refused(
                            "defaults accepts review_after_days (nonnegative integer) and view_output (nonempty quoted path)",
                        ));
                    }
                }
            }
            _ => {
                return Err(refused(
                    "configuration accepts namespaces, defaults and untracked blocks; author records through propose/revise",
                ));
            }
        }
    }
    if namespaces.is_empty() {
        return Err(refused("configuration needs at least one namespace"));
    }
    let previous_untracked = staged.ledger.project.untracked.clone();
    staged.set_tree(&path, &tree);
    staged.reparse();
    let repository = context.akr_dir.parent().map(crate::git::Repository::open);
    let mut coverage_note = None;
    let repository = match repository {
        Some(Ok(repository)) => Some(repository),
        Some(Err(crate::git::GitError::NotARepository(_))) | None => {
            coverage_note = Some(
                "untracked declarations were not checked against Git: no repository is available"
                    .to_owned(),
            );
            None
        }
        Some(Err(error)) => {
            return Err(refused(&format!(
                "cannot verify untracked declarations: {error}"
            )));
        }
    };
    let head = match repository.as_ref().map(crate::git::Repository::head) {
        Some(Ok(head)) => Some(head),
        Some(Err(crate::git::GitError::UnknownRevision(_))) => {
            coverage_note = Some("untracked declarations were not checked against Git: make an initial commit, then run `akr check`".to_owned());
            None
        }
        Some(Err(error)) => {
            return Err(refused(&format!(
                "cannot verify untracked declarations: {error}"
            )));
        }
        None => None,
    };
    if let (Some(repository), Some(head)) = (repository, head) {
        let tracked = repository
            .run_ls_tree(&head)
            .map_err(|error| refused(&format!("cannot verify untracked declarations: {error}")))?;
        for glob in &staged.ledger.project.untracked {
            if !previous_untracked.contains(glob)
                && let Some(tracked_path) = tracked
                    .iter()
                    .find(|path| crate::freshness::declaration_covers_path(glob, path))
            {
                return Err(refused(&format!(
                    "untracked declaration {:?} matches tracked path {tracked_path:?}; narrow the declaration to assets git does not hold",
                    glob.as_str()
                )));
            }
        }
    }
    let mut applied = finish_staged(
        context,
        &mut staged,
        operation,
        BTreeMap::from([(path, original)]),
        Vec::new(),
        &[],
    )?;
    if let Some(note) = coverage_note {
        applied.notes.push(note);
    }
    Ok(applied)
}

fn path_is(item: &BodyItem, path: &str) -> bool {
    matches!(item, BodyItem::Slot(slot) if slot.name == "path" && matches!(&slot.value, Value::Str(value, _) if value == path))
}

fn validate_path(path: &str) -> Result<(), Refused> {
    crate::freshness::validate_glob(&Glob::new(path))
        .map_err(|error| refused(&format!("invalid untracked path: {error}")))
}

fn refused(message: &str) -> Refused {
    Refused::new(Operation::Configure, cli::C031, format!("{message}; nothing was written"))
        .with_help("use `akr config show` for the current configuration; `akr config set --from <file>` accepts a complete project file")
}

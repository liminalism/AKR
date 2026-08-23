//! In-memory staging and the atomic write of `docs/07` §4.
//!
//! Nothing reaches the disk until the whole resulting ledger has validated. That is what
//! makes the guarantee testable: on any failing path, every source file is byte-identical
//! afterwards, because no path writes before step 5.

use crate::diagnostics::{Diagnostic, FileId, Severity};
use crate::model::Ledger;
use crate::syntax::cst;
use crate::syntax::{format, lower, parse};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

/// A workspace read into memory: file texts, their trees, and the ledger they lower to.
#[derive(Debug)]
pub struct Staged {
    /// Absolute path of the `.akr` directory.
    pub akr_dir: PathBuf,
    /// The project name from `project.akr`.
    pub project: String,
    /// File text, keyed by path relative to the `.akr` directory.
    pub texts: BTreeMap<PathBuf, String>,
    /// Parsed trees, same keys.
    pub trees: BTreeMap<PathBuf, cst::File>,
    /// The ledger the texts lower to.
    pub ledger: Ledger,
    /// Diagnostics from reading: parse and lower.
    pub diagnostics: Vec<Diagnostic>,
}

/// Why a workspace could not be read at all.
#[derive(Debug)]
pub enum LoadError {
    /// The `.akr` directory or a file under it could not be read.
    Io(PathBuf, io::Error),
    /// `project.akr` is absent (`AKR-C012`).
    NoProject(PathBuf),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(path, error) => write!(f, "{}: {error}", path.display()),
            Self::NoProject(path) => write!(f, "{}/project.akr is missing", path.display()),
        }
    }
}

impl std::error::Error for LoadError {}

fn collect(dir: &Path, base: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect(&path, base, out)?;
        } else if path.extension().is_some_and(|e| e == "akr") {
            out.push(path.strip_prefix(base).unwrap_or(&path).to_path_buf());
        }
    }
    Ok(())
}

impl Staged {
    /// Reads every `.akr` source under `akr_dir` and lowers it.
    ///
    /// # Errors
    /// Returns [`LoadError`] if the directory cannot be walked or `project.akr` is absent.
    pub fn load(akr_dir: &Path) -> Result<Self, LoadError> {
        let mut relative = Vec::new();
        collect(akr_dir, akr_dir, &mut relative)
            .map_err(|e| LoadError::Io(akr_dir.to_path_buf(), e))?;
        relative.sort();
        if !relative.iter().any(|p| p == Path::new("project.akr")) {
            return Err(LoadError::NoProject(akr_dir.to_path_buf()));
        }

        let mut texts = BTreeMap::new();
        for path in &relative {
            let full = akr_dir.join(path);
            let text = fs::read_to_string(&full).map_err(|e| LoadError::Io(full, e))?;
            texts.insert(path.clone(), text);
        }
        let mut staged = Self {
            akr_dir: akr_dir.to_path_buf(),
            project: String::new(),
            texts,
            trees: BTreeMap::new(),
            ledger: Ledger::default(),
            diagnostics: Vec::new(),
        };
        staged.reparse();
        Ok(staged)
    }

    /// Re-parses and re-lowers every staged text, rebuilding the tree map and the ledger.
    pub fn reparse(&mut self) {
        self.trees.clear();
        self.diagnostics.clear();
        let mut pairs: Vec<(String, cst::File)> = Vec::new();
        for (index, (path, text)) in self.texts.iter().enumerate() {
            let parsed = parse(text, FileId(u32::try_from(index).unwrap_or(u32::MAX)));
            self.diagnostics.extend(parsed.diagnostics);
            if let Some(file) = parsed.file {
                if self.project.is_empty() {
                    self.project.clone_from(&file.project);
                }
                self.trees.insert(path.clone(), file.clone());
                pairs.push((path.to_string_lossy().into_owned(), file));
            }
        }
        let (mut ledger, diagnostics) = lower::lower_all(&pairs);
        self.diagnostics.extend(diagnostics);
        if let Some(root) = self.akr_dir.parent() {
            ledger.facts.scratch_kept = crate::scratch::kept_entries(root);
        }
        self.ledger = ledger;
    }

    /// Replaces a file's text and its tree with a modified tree's canonical rendering.
    ///
    /// Both maps are updated, so a second edit to the same file within one operation sees
    /// the first. Updating only the text would silently discard every edit but the last.
    pub fn set_tree(&mut self, path: &Path, file: &cst::File) {
        self.texts.insert(path.to_path_buf(), format(file));
        self.trees.insert(path.to_path_buf(), file.clone());
    }

    /// Every diagnostic of effective error severity, under the given profile.
    #[must_use]
    pub fn errors(diagnostics: &[Diagnostic], strict: bool) -> Vec<Diagnostic> {
        diagnostics
            .iter()
            .filter(|d| strict || d.severity == Severity::Error)
            .cloned()
            .collect()
    }

    /// Writes the given files, atomically per file.
    ///
    /// Every temporary is written and fsynced before any rename, so the window in which a
    /// multi-file operation is half-applied is one `rename` call wide. `docs/07` §4
    /// promises per-file atomicity; this is as close to multi-file atomicity as a
    /// filesystem gives without a journal.
    ///
    /// # Errors
    /// Returns the first I/O error. Temporaries are removed on failure.
    pub fn commit(&self, paths: &[PathBuf]) -> io::Result<()> {
        let mut temporaries: Vec<(PathBuf, PathBuf)> = Vec::new();
        let result = (|| -> io::Result<()> {
            for (index, relative) in paths.iter().enumerate() {
                let target = self.akr_dir.join(relative);
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                let temporary =
                    target.with_extension(format!("akr.tmp{}{index}", std::process::id()));
                let text = self.texts.get(relative).map_or("", String::as_str);
                let mut file = fs::File::create(&temporary)?;
                file.write_all(text.as_bytes())?;
                file.sync_all()?;
                temporaries.push((temporary, target));
            }
            for (temporary, target) in &temporaries {
                fs::rename(temporary, target)?;
            }
            Ok(())
        })();
        if result.is_err() {
            for (temporary, _) in &temporaries {
                let _ = fs::remove_file(temporary);
            }
        }
        result
    }
}

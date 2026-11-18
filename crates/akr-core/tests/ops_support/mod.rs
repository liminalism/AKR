//! A sandbox: a copy of a worked example in a temporary directory.
//!
//! The write operations write. A test that ran them against the committed examples would
//! be a test that broke the repository, so every one of them works on a copy.

// Shared by two test binaries, each of which compiles it separately and uses a different
// subset: `ops_write` wants the ledger and the canonicality assertion, `ops_atomicity`
// wants only the snapshot. Splitting the module to satisfy the lint would give each
// binary its own copy of the same code.
#![allow(dead_code)]

use akr_core::diagnostics::FileId;
use akr_core::model::Ledger;
use akr_core::syntax::{format, lower, parse};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A temporary copy of an example's `.akr` directory, removed when the test ends.
pub struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    /// A copy of `examples/save-your-skin`.
    #[must_use]
    pub fn save_your_skin() -> Self {
        Self::of("save-your-skin")
    }

    /// A copy of `examples/sys-tandem`.
    #[must_use]
    pub fn sys_tandem() -> Self {
        Self::of("sys-tandem")
    }

    fn of(example: &str) -> Self {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(example)
            .join(".akr");
        let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root =
            std::env::temp_dir().join(format!("akr-ops-{example}-{}-{unique}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        copy_akr_dir(&source, &root.join(".akr")).expect("the example copies");
        Self { root }
    }

    /// The sandbox's `.akr` directory.
    #[must_use]
    pub fn akr_dir(&self) -> PathBuf {
        self.root.join(".akr")
    }

    /// Reads a file relative to the `.akr` directory.
    #[must_use]
    pub fn read(&self, relative: &Path) -> String {
        std::fs::read_to_string(self.akr_dir().join(relative)).expect("readable")
    }

    /// Every `.akr` file's text, keyed by path relative to the `.akr` directory.
    #[must_use]
    pub fn snapshot(&self) -> BTreeMap<PathBuf, String> {
        let mut out = BTreeMap::new();
        let base = self.akr_dir();
        let mut paths = Vec::new();
        walk(&base, &base, &mut paths);
        paths.sort();
        for relative in paths {
            let text = std::fs::read_to_string(base.join(&relative)).expect("readable");
            out.insert(relative, text);
        }
        out
    }

    /// The ledger the sandbox currently lowers to.
    #[must_use]
    pub fn ledger(&self) -> Ledger {
        let mut pairs = Vec::new();
        for (path, text) in self.snapshot() {
            let parsed = parse(&text, FileId(0));
            if let Some(file) = parsed.file {
                pairs.push((path.to_string_lossy().into_owned(), file));
            }
        }
        lower::lower_all(&pairs).0
    }

    /// Asserts every source file is canonically formatted — `akr fmt` on a freshly
    /// written ledger is a no-op (`docs/07` §4).
    pub fn assert_canonical(&self) {
        for (path, text) in self.snapshot() {
            let parsed = parse(&text, FileId(0));
            assert!(
                parsed.diagnostics.is_empty(),
                "{}: does not parse after a write",
                path.display()
            );
            assert_eq!(
                format(parsed.file.as_ref().expect("parses")),
                text,
                "{} is not canonical after a write",
                path.display()
            );
        }
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn walk(dir: &Path, base: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, base, out);
        } else if path.extension().is_some_and(|e| e == "akr") {
            out.push(path.strip_prefix(base).unwrap_or(&path).to_path_buf());
        }
    }
}

/// Copies `.akr/` excluding top-level `cache` and `secrets` directories so tests never
/// duplicate the 241 KB `cache/index.sqlite` that `ops` never reads.
fn copy_akr_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if entry.path().is_dir()
            && (name.eq_ignore_ascii_case("cache") || name.eq_ignore_ascii_case("secrets"))
        {
            continue;
        }
        let target = to.join(name);
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

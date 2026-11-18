//! Local-only secret entries and opaque inline citations.
//!
//! This category deliberately does not enter `Ledger`: its bytes must never participate
//! in locks, indexes, generated views or ordinary agent context. Public text carries
//! `{{secret:<32 lowercase hex digits>}}`; only an explicit local reveal reads values.

use crate::json::Value;
use std::collections::hash_map::RandomState;
use std::fs::{self, OpenOptions};
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Repository-relative private category. Never a public record directory.
pub const DIRECTORY: &str = ".akr/secrets";
/// What a reader without private data sees.
pub const REDACTED: &str = "[*]";
const PREFIX: &str = "{{secret:";

/// Whether a Git path belongs to the private category (including the directory itself).
#[must_use]
pub fn is_private_path(path: &str) -> bool {
    let path = path.replace('\\', "/");
    let parts: Vec<_> = path.split('/').collect();
    parts
        .windows(2)
        .any(|p| p[0].eq_ignore_ascii_case(".akr") && p[1].eq_ignore_ascii_case("secrets"))
}

/// Prevents an ordinary input workflow from copying private storage into public state.
/// Canonical paths catch aliases, parent traversals and links into the private category.
///
/// # Errors
/// Returns a permission error without disclosing the file name or contents.
pub fn require_public_input(root: &Path, path: &Path) -> io::Result<()> {
    let private = root.join(DIRECTORY);
    let candidate = root.join(path);
    let canonical_private = fs::canonicalize(&private).unwrap_or(private.clone());
    let canonical_candidate = fs::canonicalize(&candidate).unwrap_or(candidate.clone());
    let contains_private_category = |path: &Path| {
        let parts: Vec<_> = path
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase())
            .collect();
        parts
            .windows(2)
            .any(|p| p[0] == ".akr" && p[1] == "secrets")
    };
    if candidate.starts_with(&private)
        || canonical_candidate.starts_with(&canonical_private)
        || contains_private_category(&candidate)
        || contains_private_category(&canonical_candidate)
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "private secret storage cannot be used as public input",
        ));
    }
    Ok(())
}

fn id(reference: &str) -> io::Result<&str> {
    let id = reference
        .strip_prefix(PREFIX)
        .and_then(|s| s.strip_suffix("}}"))
        .unwrap_or(reference);
    if id.len() == 32
        && id
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        Ok(id)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected an opaque secret citation or 32 lowercase hex digits",
        ))
    }
}

fn citation(id: &str) -> String {
    format!("{PREFIX}{id}}}}}")
}

fn reject_link(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            let linked = meta.file_type().is_symlink();
            #[cfg(windows)]
            let linked = {
                use std::os::windows::fs::MetadataExt;
                linked || meta.file_attributes() & 0x400 != 0 // all reparse points, including junctions
            };
            if linked {
                Err(io::Error::other(
                    "private secret storage must not use symbolic links or reparse points",
                ))
            } else {
                Ok(())
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

fn directory(root: &Path, create: bool) -> io::Result<PathBuf> {
    reject_link(&root.join(".akr"))?;
    let dir = root.join(DIRECTORY);
    reject_link(&dir)?;
    if create {
        fs::create_dir_all(&dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        }
        // The root ignore protects a fresh workspace; this protects existing workspaces
        // immediately, even before their generated setup guidance has been refreshed.
        let ignore = dir.join(".gitignore");
        reject_link(&ignore)?;
        let current = match fs::read_to_string(&ignore) {
            Ok(current) => current,
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e),
        };
        if current != "*\n" {
            // Repair empty, interrupted or negated ignore files before writing a value.
            // Atomic replacement leaves the old file intact if the repair fails.
            let nonce = RandomState::new().build_hasher().finish();
            let temp = dir.join(format!(".gitignore.{nonce:016x}.tmp"));
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            let result = file.write_all(b"*\n").and_then(|()| file.sync_all());
            drop(file);
            let result = result.and_then(|()| fs::rename(&temp, &ignore));
            if result.is_err() {
                let _ = fs::remove_file(&temp);
            }
            result?;
        }
    }
    Ok(dir)
}

/// Creates an immutable private entry and returns only its citation, never its value.
/// The id is randomly seeded and independent of the value; it is not an access token.
///
/// # Errors
/// Returns an I/O error; error messages never include the value.
pub fn put(root: &Path, value: &str) -> io::Result<String> {
    if value.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a secret value must not be empty",
        ));
    }
    let dir = directory(root, true)?;
    let body = Value::object(vec![
        ("category", Value::string("secret")),
        ("version", Value::integer(1)),
        ("value", Value::string(value)),
    ])
    .to_pretty();
    loop {
        // RandomState obtains per-instance random seeds from the standard library.
        // Identity must not be a hash of the value (which could disclose small values).
        let next = || RandomState::new().build_hasher().finish();
        let opaque = format!("{:016x}{:016x}", next(), next());
        let path = dir.join(format!("{opaque}.json"));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = match options.open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        if let Err(e) = file
            .write_all(body.as_bytes())
            .and_then(|()| file.sync_all())
        {
            let _ = fs::remove_file(&path);
            return Err(e);
        }
        return Ok(citation(&opaque));
    }
}

/// Explicitly reads one private value. An absent entry is normal for public clones.
///
/// # Errors
/// Rejects malformed identifiers, symlinks and corrupt storage without echoing contents.
pub fn get(root: &Path, reference: &str) -> io::Result<Option<String>> {
    let opaque = id(reference)?;
    let path = directory(root, false)?.join(format!("{opaque}.json"));
    reject_link(&path)?;
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let corrupt = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid private secret entry (contents withheld)",
        )
    };
    let entry = crate::json::parse(&text).map_err(|_| corrupt())?;
    if entry.get("category").and_then(Value::as_str) != Some("secret")
        || entry.get("version").and_then(Value::as_integer) != Some(1)
    {
        return Err(corrupt());
    }
    entry
        .get("value")
        .and_then(Value::as_str)
        .map(|s| Some(s.to_owned()))
        .ok_or_else(corrupt)
}

/// Substitutes inline citations in a single pass. Inserted values are never re-expanded.
/// Missing private data always renders as `[*]`; ordinary callers should use `redact`.
///
/// # Errors
/// Propagates a storage error only when `reveal` explicitly requests local values.
pub fn resolve(root: &Path, text: &str, reveal: bool) -> io::Result<String> {
    substitute(text, |reference| {
        if reveal {
            Ok(get(root, reference)?.unwrap_or_else(|| REDACTED.to_owned()))
        } else {
            Ok(REDACTED.to_owned())
        }
    })
}

fn substitute(
    text: &str,
    mut replacement: impl FnMut(&str) -> io::Result<String>,
) -> io::Result<String> {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find(PREFIX) {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        if let Some(end) = rest.find("}}").map(|n| n + 2) {
            let reference = &rest[..end];
            if id(reference).is_ok() {
                out.push_str(&replacement(reference)?);
                rest = &rest[end..];
                continue;
            }
        }
        out.push_str(PREFIX);
        rest = &rest[PREFIX.len()..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Public rendering. Never opens the filesystem, even when private entries exist.
#[must_use]
pub fn redact(text: &str) -> String {
    substitute(text, |_| Ok(REDACTED.to_owned())).expect("redaction cannot fail")
}

/// Redacts citations recursively in a transport payload without accessing private data.
pub fn redact_json(value: &mut Value) {
    match value {
        Value::String(text) => *text = redact(text),
        Value::Array(items) => items.iter_mut().for_each(redact_json),
        Value::Object(fields) => fields.iter_mut().for_each(|(_, value)| redact_json(value)),
        _ => (),
    }
}

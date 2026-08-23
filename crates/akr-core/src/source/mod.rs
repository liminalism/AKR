//! Immutable external source library (`sources/`).
//!
//! AKR's authoritative knowledge lives in `.akr/`; exact outside advice,
//! audits and reports live in `sources/external/` as immutable, content-hashed
//! files. This module manages the catalog and the file-system invariants.
//!
//! The catalog is `sources/catalog.json` — a deterministic list of
//! [`SourceDocument`] entries. Registered bytes are immutable while present,
//! but a document may be finalized into retained cited fragments or metadata
//! only. The older entry remains when a source is superseded.
//!
//! [`chunk`] turns those exact bytes into retrieval units. Chunking is derived and
//! rebuildable: it can only affect how well search finds a passage, never what the
//! project believes about it.

pub mod chunk;

pub use chunk::{ChunkKind, PARSER_VERSION, SourceChunk, chunk_markdown};

use crate::hash::Sha256;
use std::path::{Path, PathBuf};

/// Origin of a registered source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceOrigin {
    /// Advice, audits and reports from outside the project.
    External,
    /// Reference material the project keeps but does not maintain.
    InternalReference,
}

/// What source material remains available after registration or finalization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceAvailability {
    /// The complete registered document is available at [`SourceDocument::path`].
    Full,
    /// Only exact cited ranges and their retained context are available.
    CitedOnly,
    /// The source's identity remains, but no source bytes are retained.
    MetadataOnly,
}

impl SourceAvailability {
    /// The catalog's spelling of this availability.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::CitedOnly => "cited-only",
            Self::MetadataOnly => "metadata-only",
        }
    }

    /// Parses the catalog's spelling; `None` for anything else.
    // Inherent rather than `FromStr`: these parse the *catalog's* spelling and return
    // `Option`, where the trait demands an associated error type and a blanket `parse()`
    // that would invite parsing arbitrary strings into them.
    #[allow(clippy::should_implement_trait)]
    #[must_use]
    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "full" => Some(Self::Full),
            "cited-only" => Some(Self::CitedOnly),
            "metadata-only" => Some(Self::MetadataOnly),
            _ => None,
        }
    }
}

impl SourceOrigin {
    /// The catalog's spelling of this origin.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::External => "external",
            Self::InternalReference => "internal-reference",
        }
    }

    /// Parses the catalog's spelling, accepting `internal` for
    /// [`Self::InternalReference`]; `None` for anything else.
    // Inherent rather than `FromStr`: these parse the *catalog's* spelling and return
    // `Option`, where the trait demands an associated error type and a blanket `parse()`
    // that would invite parsing arbitrary strings into them.
    #[allow(clippy::should_implement_trait)]
    #[must_use]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "external" => Some(Self::External),
            "internal-reference" | "internal" => Some(Self::InternalReference),
            _ => None,
        }
    }
}

/// A registered source document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDocument {
    /// Stable identifier, e.g. `2026-08-05-jp2lam-audit`.
    pub id: String,
    /// Human title.
    pub title: String,
    /// Origin.
    pub origin: SourceOrigin,
    /// Media type, `text/markdown` for now.
    pub media_type: String,
    /// Repo-relative path, e.g. `sources/external/2026-08-05-foo--a1b2c3d4.md`.
    pub path: String,
    /// `sha256:` + 64 hex.
    pub content_hash: String,
    /// Byte length of the stored file.
    pub byte_len: u64,
    /// Date string `YYYY-MM-DD` from wall clock at add time, or caller-supplied.
    pub added_at: String,
    /// Optional observed git commit or URL.
    pub observed_at: Option<String>,
    /// Optional scope globs.
    pub scope: Option<String>,
    /// Previous version this supersedes, if any.
    pub supersedes: Option<String>,
    /// Whether the full document, cited fragments, or only metadata remains.
    pub availability: SourceAvailability,
    /// Content-addressed fragments retained after finalization.
    pub fragments: Vec<RetainedFragment>,
}

/// A content-addressed source fragment retained for one or more exact citations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetainedFragment {
    /// Exact ranges in the original document that this fragment supports.
    pub cited_ranges: Vec<crate::model::SourceRange>,
    /// The captured semantic context containing those ranges.
    pub captured_range: crate::model::SourceRange,
    /// Hash of the captured bytes.
    pub content_hash: String,
    /// Hash-derived fragment identifier, also used as the blob name.
    pub blob: String,
}

/// Verification diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceDiagnostic {
    /// Stored bytes do not match their hash.
    HashMismatch {
        /// The registered document's id.
        id: String,
        /// Its path, as the catalog records it.
        path: String,
        /// The hash the catalog registered.
        expected: String,
        /// The hash the bytes on disk actually have.
        found: String,
    },
    /// Catalog references a missing file.
    MissingFile {
        /// The registered document's id.
        id: String,
        /// The path that is not there.
        path: String,
    },
    /// Catalog itself unreadable.
    CatalogError(
        /// Why it could not be read or parsed.
        String,
    ),
    /// A cited-only catalog entry has lost a retained fragment.
    MissingFragment {
        /// The registered document's id.
        id: String,
        /// The content-addressed blob that is missing.
        blob: String,
    },
    /// A retained fragment no longer matches its content hash.
    FragmentHashMismatch {
        /// The registered document's id.
        id: String,
        /// The content-addressed blob that drifted.
        blob: String,
        /// The hash the fragment was retained under.
        expected: String,
        /// The hash its bytes now have.
        found: String,
    },
}

impl std::fmt::Display for SourceDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HashMismatch {
                id,
                path,
                expected,
                found,
            } => write!(
                f,
                "AKR-S021 registered source bytes do not match their content hash\nsource: {id}\npath: {path}\nexpected: {expected}\nfound: {found}\nhelp: restore the original bytes or register a superseding source version"
            ),
            Self::MissingFile { id, path } => write!(
                f,
                "AKR-S021 registered source file is missing\nsource: {id}\npath: {path}"
            ),
            Self::CatalogError(msg) => write!(f, "AKR-S021 catalog error: {msg}"),
            Self::MissingFragment { id, blob } => write!(
                f,
                "AKR-S021 retained source fragment is missing\nsource: {id}\nfragment: {blob}"
            ),
            Self::FragmentHashMismatch {
                id,
                blob,
                expected,
                found,
            } => write!(
                f,
                "AKR-S021 retained source fragment hash mismatch\nsource: {id}\nfragment: {blob}\nexpected: {expected}\nfound: {found}"
            ),
        }
    }
}

/// Catalog file location: `<workspace-root>/sources/catalog.json`.
///
/// `workspace_root` is the directory containing `.akr/`.
#[must_use]
pub fn catalog_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join("sources").join("catalog.json")
}

/// External sources directory.
#[must_use]
pub fn external_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join("sources").join("external")
}

/// Canonical fragment path for a content hash.
#[must_use]
pub fn fragment_path(workspace_root: &Path, blob: &str) -> PathBuf {
    let hash = blob.strip_prefix("sha256:").unwrap_or(blob);
    let prefix = hash.get(..2).unwrap_or("00");
    workspace_root
        .join(".akr")
        .join("source-fragments")
        .join("sha256")
        .join(prefix)
        .join(format!("{hash}.blob"))
}

/// Computes `sha256:` + hex for bytes.
#[must_use]
pub fn hash_bytes(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("sha256:{}", h.finish().to_hex())
}

/// Loads the catalog, or returns empty if absent.
pub fn load_catalog(workspace_root: &Path) -> Result<Vec<SourceDocument>, SourceDiagnostic> {
    let path = catalog_path(workspace_root);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| {
        SourceDiagnostic::CatalogError(format!("cannot read {}: {e}", path.display()))
    })?;
    parse_catalog(&raw)
}

/// Parses catalog JSON.
pub fn parse_catalog(json: &str) -> Result<Vec<SourceDocument>, SourceDiagnostic> {
    let v: serde_json_value::Value = parse_json(json)?;
    let arr = v
        .as_array()
        .ok_or_else(|| SourceDiagnostic::CatalogError("catalog root must be array".into()))?;
    let mut out = Vec::new();
    for item in arr {
        out.push(parse_document(item)?);
    }
    Ok(out)
}

fn parse_document(v: &serde_json_value::Value) -> Result<SourceDocument, SourceDiagnostic> {
    let obj = v
        .as_object()
        .ok_or_else(|| SourceDiagnostic::CatalogError("catalog entry must be object".into()))?;
    let get_str = |k: &str, required: bool| -> Result<Option<String>, SourceDiagnostic> {
        match obj.get(k) {
            Some(serde_json_value::Value::String(s)) => Ok(Some(s.clone())),
            Some(serde_json_value::Value::Null) | None if !required => Ok(None),
            None if !required => Ok(None),
            Some(other) => Err(SourceDiagnostic::CatalogError(format!(
                "field {k:?} must be string, got {other:?}"
            ))),
            None => Err(SourceDiagnostic::CatalogError(format!(
                "missing field {k:?}"
            ))),
        }
    };
    let id = get_str("id", true)?.unwrap();
    let title = get_str("title", true)?.unwrap_or_else(|| id.clone());
    let origin = get_str("origin", true)?.unwrap();
    let origin = SourceOrigin::from_str(&origin).ok_or_else(|| {
        SourceDiagnostic::CatalogError(format!(
            "origin {origin:?} is not external|internal-reference"
        ))
    })?;
    let media_type = get_str("media_type", true)?.unwrap();
    let path = get_str("path", false)?.unwrap_or_default();
    let content_hash = get_str("content_hash", true)?.unwrap();
    let byte_len = match obj.get("byte_len") {
        Some(serde_json_value::Value::Number(n)) => n.as_u64().unwrap_or(0),
        _ => 0,
    };
    let added_at = get_str("added_at", false)?.unwrap_or_default();
    let observed_at = get_str("observed_at", false)?;
    let scope = get_str("scope", false)?;
    let supersedes = get_str("supersedes", false)?;
    let availability = match get_str("availability", false)? {
        None => SourceAvailability::Full,
        Some(value) => SourceAvailability::from_str(&value).ok_or_else(|| {
            SourceDiagnostic::CatalogError(format!(
                "source {id:?} has an unknown availability; expected full|cited-only|metadata-only"
            ))
        })?,
    };
    let fragments = parse_fragments(obj.get("fragments"), &id)?;
    Ok(SourceDocument {
        id,
        title,
        origin,
        media_type,
        path,
        content_hash,
        byte_len,
        added_at,
        observed_at,
        scope,
        supersedes,
        availability,
        fragments,
    })
}

fn parse_fragments(
    value: Option<&serde_json_value::Value>,
    id: &str,
) -> Result<Vec<RetainedFragment>, SourceDiagnostic> {
    let Some(serde_json_value::Value::Array(items)) = value else {
        return Ok(Vec::new());
    };
    let mut fragments = Vec::new();
    for item in items {
        let obj = item.as_object().ok_or_else(|| {
            SourceDiagnostic::CatalogError(format!("source {id:?} fragment must be an object"))
        })?;
        let text = |key: &str| {
            obj.get(key)
                .and_then(|value| match value {
                    serde_json_value::Value::String(value) => Some(value.clone()),
                    _ => None,
                })
                .ok_or_else(|| SourceDiagnostic::CatalogError(format!("fragment missing {key:?}")))
        };
        let range = |value: &serde_json_value::Value| -> Result<crate::model::SourceRange, SourceDiagnostic> {
            let obj = value.as_object().ok_or_else(|| {
                SourceDiagnostic::CatalogError(format!("source {id:?} fragment range must be an object"))
            })?;
            let number = |key: &str| {
                obj.get(key)
                    .and_then(|value| match value {
                        serde_json_value::Value::Number(value) => value.as_u64(),
                        _ => None,
                    })
                    .ok_or_else(|| SourceDiagnostic::CatalogError(format!("fragment range missing {key:?}")))
            };
            Ok(crate::model::SourceRange {
                start_byte: number("start_byte")?,
                end_byte: number("end_byte")?,
                start_line: u32::try_from(number("start_line")?).unwrap_or(u32::MAX),
                end_line: u32::try_from(number("end_line")?).unwrap_or(u32::MAX),
                excerpt_hash: obj
                    .get("excerpt_hash")
                    .and_then(|value| match value {
                        serde_json_value::Value::String(value) => Some(value.clone()),
                        _ => None,
                    }),
            })
        };
        let captured_range = range(obj.get("captured_range").ok_or_else(|| {
            SourceDiagnostic::CatalogError(format!("source {id:?} fragment missing captured_range"))
        })?)?;
        let cited_values = obj
            .get("cited_ranges")
            .and_then(serde_json_value::Value::as_array)
            .ok_or_else(|| {
                SourceDiagnostic::CatalogError(format!(
                    "source {id:?} fragment missing cited_ranges"
                ))
            })?;
        fragments.push(RetainedFragment {
            cited_ranges: cited_values.iter().map(range).collect::<Result<_, _>>()?,
            captured_range,
            content_hash: text("content_hash")?,
            blob: text("blob")?,
        });
    }
    Ok(fragments)
}

/// Serializes catalog deterministically (sorted by id).
#[must_use]
pub fn serialize_catalog(docs: &[SourceDocument]) -> String {
    let mut sorted = docs.to_vec();
    sorted.sort_by(|a, b| a.id.cmp(&b.id));
    let mut parts = Vec::new();
    for d in &sorted {
        let mut obj = serde_json_value::Map::new();
        obj.insert("id".into(), serde_json_value::Value::String(d.id.clone()));
        obj.insert(
            "title".into(),
            serde_json_value::Value::String(d.title.clone()),
        );
        obj.insert(
            "origin".into(),
            serde_json_value::Value::String(d.origin.as_str().to_owned()),
        );
        obj.insert(
            "media_type".into(),
            serde_json_value::Value::String(d.media_type.clone()),
        );
        obj.insert(
            "path".into(),
            serde_json_value::Value::String(d.path.clone()),
        );
        obj.insert(
            "content_hash".into(),
            serde_json_value::Value::String(d.content_hash.clone()),
        );
        obj.insert(
            "byte_len".into(),
            serde_json_value::Value::Number(serde_json_value::Number::from(d.byte_len)),
        );
        obj.insert(
            "added_at".into(),
            serde_json_value::Value::String(d.added_at.clone()),
        );
        if let Some(v) = &d.observed_at {
            obj.insert(
                "observed_at".into(),
                serde_json_value::Value::String(v.clone()),
            );
        }
        if let Some(v) = &d.scope {
            obj.insert("scope".into(), serde_json_value::Value::String(v.clone()));
        }
        if let Some(v) = &d.supersedes {
            obj.insert(
                "supersedes".into(),
                serde_json_value::Value::String(v.clone()),
            );
        }
        obj.insert(
            "availability".into(),
            serde_json_value::Value::String(d.availability.as_str().to_owned()),
        );
        if !d.fragments.is_empty() {
            let fragments = d
                .fragments
                .iter()
                .map(|fragment| {
                    let mut item = serde_json_value::Map::new();
                    item.insert(
                        "cited_ranges".into(),
                        serde_json_value::Value::Array(
                            fragment.cited_ranges.iter().map(range_json).collect(),
                        ),
                    );
                    item.insert(
                        "captured_range".into(),
                        range_json(&fragment.captured_range),
                    );
                    item.insert(
                        "content_hash".into(),
                        serde_json_value::Value::String(fragment.content_hash.clone()),
                    );
                    item.insert(
                        "blob".into(),
                        serde_json_value::Value::String(fragment.blob.clone()),
                    );
                    serde_json_value::Value::Object(item)
                })
                .collect();
            obj.insert(
                "fragments".into(),
                serde_json_value::Value::Array(fragments),
            );
        }
        parts.push(serde_json_value::Value::Object(obj));
    }
    let v = serde_json_value::Value::Array(parts);
    // pretty with 2 spaces, deterministic
    serde_json_value::to_string_pretty(&v).unwrap_or_else(|_| "[]".into()) + "\n"
}

fn range_json(range: &crate::model::SourceRange) -> serde_json_value::Value {
    let mut obj = serde_json_value::Map::new();
    obj.insert(
        "start_byte".into(),
        serde_json_value::Value::Number(serde_json_value::Number::from(range.start_byte)),
    );
    obj.insert(
        "end_byte".into(),
        serde_json_value::Value::Number(serde_json_value::Number::from(range.end_byte)),
    );
    obj.insert(
        "start_line".into(),
        serde_json_value::Value::Number(serde_json_value::Number::from(u64::from(
            range.start_line,
        ))),
    );
    obj.insert(
        "end_line".into(),
        serde_json_value::Value::Number(serde_json_value::Number::from(u64::from(range.end_line))),
    );
    if let Some(hash) = &range.excerpt_hash {
        obj.insert(
            "excerpt_hash".into(),
            serde_json_value::Value::String(hash.clone()),
        );
    }
    serde_json_value::Value::Object(obj)
}

/// Saves catalog atomically (write temp then rename).
pub fn save_catalog(
    workspace_root: &Path,
    docs: &[SourceDocument],
) -> Result<(), SourceDiagnostic> {
    let path = catalog_path(workspace_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            SourceDiagnostic::CatalogError(format!("cannot create {}: {e}", parent.display()))
        })?;
    }
    let raw = serialize_catalog(docs);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, raw.as_bytes()).map_err(|e| {
        SourceDiagnostic::CatalogError(format!("cannot write {}: {e}", tmp.display()))
    })?;
    std::fs::rename(&tmp, &path).map_err(|e| {
        SourceDiagnostic::CatalogError(format!(
            "cannot rename {} -> {}: {e}",
            tmp.display(),
            path.display()
        ))
    })?;
    Ok(())
}

/// A registered document together with its exact bytes.
#[derive(Debug, Clone)]
pub struct LoadedSource {
    /// The catalog entry.
    pub document: SourceDocument,
    /// The file's contents, verified against `document.content_hash`.
    pub text: String,
}

/// Whether a newer catalog entry supersedes this one.
#[must_use]
pub fn is_superseded(doc: &SourceDocument, catalog: &[SourceDocument]) -> bool {
    catalog
        .iter()
        .any(|other| other.supersedes.as_deref() == Some(doc.id.as_str()))
}

/// A digest over the corpus's identity: sorted `(id, content_hash)` pairs.
///
/// This is the source library's half of the cache generation pair (D-031). It moves when
/// a document is registered or superseded and at no other time, which is what lets a
/// record write leave the chunk tables alone and a source registration leave the record
/// tables alone.
#[must_use]
pub fn corpus_hash(docs: &[SourceDocument]) -> String {
    let mut sorted: Vec<&SourceDocument> = docs.iter().collect();
    sorted.sort_by(|a, b| a.id.cmp(&b.id));
    let mut hasher = Sha256::new();
    for doc in sorted {
        hasher.update(doc.id.as_bytes());
        hasher.update(b"\0");
        hasher.update(doc.content_hash.as_bytes());
        hasher.update(b"\0");
    }
    hasher.update(chunk::PARSER_VERSION.to_string().as_bytes());
    format!("sha256:{}", hasher.finish().to_hex())
}

/// Reads every catalog entry's bytes, verifying each hash on the way through.
///
/// Serving a passage from a file that no longer matches its registration would defeat the
/// whole point of registering it, so a mismatch is an error here rather than a warning.
///
/// # Errors
/// [`SourceDiagnostic::HashMismatch`] or [`SourceDiagnostic::MissingFile`] for the first
/// entry that fails, and [`SourceDiagnostic::CatalogError`] for an unreadable catalog.
pub fn load_corpus(workspace_root: &Path) -> Result<Vec<LoadedSource>, SourceDiagnostic> {
    let catalog = load_catalog(workspace_root)?;
    let mut out = Vec::with_capacity(catalog.len());
    for document in catalog {
        if document.availability != SourceAvailability::Full {
            continue;
        }
        let file = workspace_root.join(&document.path);
        let bytes = std::fs::read(&file).map_err(|_| SourceDiagnostic::MissingFile {
            id: document.id.clone(),
            path: document.path.clone(),
        })?;
        let found = hash_bytes(&bytes);
        if found != document.content_hash {
            return Err(SourceDiagnostic::HashMismatch {
                id: document.id.clone(),
                path: document.path.clone(),
                expected: document.content_hash.clone(),
                found,
            });
        }
        out.push(LoadedSource {
            document,
            text: String::from_utf8_lossy(&bytes).into_owned(),
        });
    }
    Ok(out)
}

/// Reads and verifies a retained fragment blob.
pub fn read_fragment(
    workspace_root: &Path,
    document: &SourceDocument,
    fragment: &RetainedFragment,
) -> Result<Vec<u8>, SourceDiagnostic> {
    let path = fragment_path(workspace_root, &fragment.blob);
    let bytes = std::fs::read(&path).map_err(|_| SourceDiagnostic::MissingFragment {
        id: document.id.clone(),
        blob: fragment.blob.clone(),
    })?;
    let found = hash_bytes(&bytes);
    if found != fragment.content_hash || found != fragment.blob {
        return Err(SourceDiagnostic::FragmentHashMismatch {
            id: document.id.clone(),
            blob: fragment.blob.clone(),
            expected: fragment.content_hash.clone(),
            found,
        });
    }
    Ok(bytes)
}

fn contains_range(outer: &crate::model::SourceRange, inner: &crate::model::SourceRange) -> bool {
    outer.start_byte <= inner.start_byte && outer.end_byte >= inner.end_byte
}

/// Turns a one-based, inclusive line range into the exact byte locator a citation needs.
///
/// A `source` block wants all four coordinates or none (`AKR-T012`): bytes are what
/// resolves, lines are what a reader opens the file at, and a half-written range would
/// point somewhere nobody chose. That rule is right for the stored record and wrong as a
/// demand on the author, who reads a document by line and would otherwise have to count
/// bytes by hand. This closes that gap: give it the lines, it returns the range the
/// language asks for, hashed over the bytes it selected so the citation verifies itself.
///
/// The range covers whole lines, and includes the newline that ends the last one, which
/// is the convention [`check_citation`] measures lines against.
///
/// # Errors
/// A message naming the document when it is not registered, retains no full text, has
/// drifted from its content hash, or does not have the lines asked for.
pub fn locate_lines(
    workspace_root: &Path,
    document_id: &str,
    start_line: u32,
    end_line: u32,
) -> Result<crate::model::SourceRange, String> {
    if start_line == 0 || end_line < start_line {
        return Err(format!(
            "line range {start_line}-{end_line} is not a range; lines are one-based and \
             the end must not precede the start"
        ));
    }
    let catalog = load_catalog(workspace_root).map_err(|e| e.to_string())?;
    let document = catalog
        .iter()
        .find(|document| document.id == document_id)
        .ok_or_else(|| format!("source {document_id:?} is not registered"))?;
    if document.availability != SourceAvailability::Full {
        return Err(format!(
            "source {document_id:?} retains {} text, so a line range cannot be located in \
             it; cite it by start_byte and end_byte from an existing retained range",
            document.availability.as_str()
        ));
    }
    let bytes = std::fs::read(workspace_root.join(&document.path))
        .map_err(|_| format!("source {document_id:?} file is missing"))?;
    if hash_bytes(&bytes) != document.content_hash {
        return Err(format!(
            "source {document_id:?} bytes do not match their content hash"
        ));
    }

    let mut line_starts: Vec<usize> = vec![0];
    for (offset, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            line_starts.push(offset + 1);
        }
    }
    // A file ending in a newline leaves a phantom empty line after it: the offset is a
    // legal place to start reading and not a line anybody can cite.
    let mut lines = line_starts.len();
    if bytes.last() == Some(&b'\n') {
        lines -= 1;
    }
    if lines == 0 || end_line as usize > lines {
        return Err(format!(
            "source {document_id:?} has {lines} lines; line range {start_line}-{end_line} \
             is out of bounds"
        ));
    }

    let start = line_starts[start_line as usize - 1];
    let requested_end = if (end_line as usize) < lines {
        line_starts[end_line as usize]
    } else {
        bytes.len()
    };
    // Citation validation does not count trailing empty lines as part of a passage. A
    // line-only authoring call used to store the requested blank `end_line` while its
    // computed bytes described the preceding passage, so the write succeeded and the
    // immediately following check raised AKR-S022. Normalize both coordinates together
    // before the record exists. A citation consisting only of a blank line retains that
    // line because `last_line_of` cannot move before `start_line`.
    let end_line = last_line_of(start_line, &bytes[start..requested_end]);
    let end = if (end_line as usize) < lines {
        line_starts[end_line as usize]
    } else {
        bytes.len()
    };
    Ok(crate::model::SourceRange {
        start_byte: start as u64,
        end_byte: end as u64,
        start_line,
        end_line,
        excerpt_hash: Some(hash_bytes(&bytes[start..end])),
    })
}

/// Resolves one citation from either the full source or retained fragments.
pub fn resolve_citation(
    workspace_root: &Path,
    source: &crate::model::Source,
) -> Result<Option<Vec<u8>>, String> {
    let Some(document_id) = &source.document else {
        return Ok(None);
    };
    let catalog = load_catalog(workspace_root).map_err(|e| e.to_string())?;
    let document = catalog
        .iter()
        .find(|document| &document.id == document_id)
        .ok_or_else(|| format!("cites source {document_id:?}, which is not registered"))?;
    let Some(range) = &source.range else {
        return Ok(None);
    };
    match document.availability {
        SourceAvailability::Full => {
            let bytes = std::fs::read(workspace_root.join(&document.path))
                .map_err(|_| format!("source {:?} file is missing", document.id))?;
            if hash_bytes(&bytes) != document.content_hash {
                return Err(format!(
                    "source {:?} bytes do not match their content hash",
                    document.id
                ));
            }
            let start = usize::try_from(range.start_byte).unwrap_or(usize::MAX);
            let end = usize::try_from(range.end_byte).unwrap_or(usize::MAX);
            let selected = bytes
                .get(start..end)
                .ok_or_else(|| format!("source {:?} citation is out of bounds", document.id))?;
            Ok(Some(selected.to_vec()))
        }
        SourceAvailability::CitedOnly => {
            for fragment in &document.fragments {
                if !fragment.cited_ranges.iter().any(|cited| cited == range)
                    || !contains_range(&fragment.captured_range, range)
                {
                    continue;
                }
                let captured =
                    read_fragment(workspace_root, document, fragment).map_err(|e| e.to_string())?;
                let relative_start = usize::try_from(
                    range
                        .start_byte
                        .saturating_sub(fragment.captured_range.start_byte),
                )
                .unwrap_or(usize::MAX);
                let relative_end = usize::try_from(
                    range
                        .end_byte
                        .saturating_sub(fragment.captured_range.start_byte),
                )
                .unwrap_or(usize::MAX);
                let selected = captured.get(relative_start..relative_end).ok_or_else(|| {
                    format!("retained fragment for {:?} is incomplete", document.id)
                })?;
                return Ok(Some(selected.to_vec()));
            }
            Err(format!(
                "source {:?} exact citation was not retained",
                document.id
            ))
        }
        SourceAvailability::MetadataOnly => Err(format!(
            "source {:?} retains metadata only; exact citation text is unavailable",
            document.id
        )),
    }
}

/// A record's citation into the source library, checked against the library.
///
/// Four things can be wrong with a citation and they fail differently, so they are
/// reported separately rather than as one "bad source" verdict: the reader's next action
/// is different for each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CitationProblem {
    /// The named document is not in the catalog.
    UnknownDocument {
        /// The document id the citation named.
        document: String,
    },
    /// The byte range runs past the end of the document.
    RangeOutOfBounds {
        /// The document id the citation named.
        document: String,
        /// The end the citation asked for.
        end_byte: u64,
        /// How many bytes the document actually has.
        byte_len: u64,
    },
    /// The range does not begin and end on character boundaries.
    RangeNotOnBoundary {
        /// The document id the citation named.
        document: String,
        /// The first byte, which may be mid-character.
        start: u64,
        /// One past the last byte, which may be mid-character.
        end: u64,
    },
    /// The recorded excerpt hash disagrees with the bytes in that range.
    ExcerptHashMismatch {
        /// The document id the citation named.
        document: String,
        /// The hash the record carries.
        expected: String,
        /// The hash the cited bytes actually have.
        found: String,
    },
    /// The line range does not describe the same passage as the byte range.
    LinesDisagree {
        /// The document id the citation named.
        document: String,
        /// The `(start, end)` lines the record carries.
        recorded: (u32, u32),
        /// The `(start, end)` lines the byte range really covers.
        actual: (u32, u32),
    },
    /// The catalog retains metadata or fragments that do not cover this citation.
    TextUnavailable {
        /// The document id the citation named.
        document: String,
        /// What the catalog retains instead, and why that is not enough.
        reason: String,
    },
}

impl std::fmt::Display for CitationProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownDocument { document } => write!(
                f,
                "cites source {document:?}, which is not registered; \
                 run `akr source list` or register it with `akr source add`"
            ),
            Self::RangeOutOfBounds {
                document,
                end_byte,
                byte_len,
            } => write!(
                f,
                "cites bytes up to {end_byte} of source {document:?}, which is {byte_len} bytes long"
            ),
            Self::RangeNotOnBoundary {
                document,
                start,
                end,
            } => write!(
                f,
                "cites bytes {start}..{end} of source {document:?}, which do not fall on \
                 character boundaries"
            ),
            Self::ExcerptHashMismatch {
                document,
                expected,
                found,
            } => write!(
                f,
                "the excerpt hash of the citation into {document:?} does not match the \
                 bytes in its range\nexpected: {expected}\nfound: {found}"
            ),
            Self::LinesDisagree {
                document,
                recorded,
                actual,
            } => write!(
                f,
                "the citation into {document:?} records lines {}-{} for a byte range that \
                 covers lines {}-{}",
                recorded.0, recorded.1, actual.0, actual.1
            ),
            Self::TextUnavailable { document, reason } => {
                write!(
                    f,
                    "source {document:?} citation text is unavailable: {reason}"
                )
            }
        }
    }
}

/// Checks one citation against a loaded corpus.
///
/// Returns nothing for a `source` block that names no document: a loose `path` is still
/// legitimate provenance (a legacy migration writes one), and only a citation into the
/// registered library makes a promise this can check.
#[must_use]
pub fn check_citation(
    source: &crate::model::Source,
    corpus: &[LoadedSource],
) -> Vec<CitationProblem> {
    let Some(document) = &source.document else {
        return Vec::new();
    };
    let Some(loaded) = corpus.iter().find(|item| &item.document.id == document) else {
        return vec![CitationProblem::UnknownDocument {
            document: document.clone(),
        }];
    };
    let Some(range) = &source.range else {
        return Vec::new();
    };

    let text = &loaded.text;
    let byte_len = text.len() as u64;
    if range.end_byte > byte_len || range.start_byte > range.end_byte {
        return vec![CitationProblem::RangeOutOfBounds {
            document: document.clone(),
            end_byte: range.end_byte,
            byte_len,
        }];
    }
    let start = usize::try_from(range.start_byte).unwrap_or(usize::MAX);
    let end = usize::try_from(range.end_byte).unwrap_or(usize::MAX);
    let Some(slice) = text.get(start..end) else {
        return vec![CitationProblem::RangeNotOnBoundary {
            document: document.clone(),
            start: range.start_byte,
            end: range.end_byte,
        }];
    };

    let mut problems = Vec::new();
    if let Some(expected) = &range.excerpt_hash {
        let found = hash_bytes(slice.as_bytes());
        if &found != expected {
            problems.push(CitationProblem::ExcerptHashMismatch {
                document: document.clone(),
                expected: expected.clone(),
                found,
            });
        }
    }

    // Lines are the human half of the locator, and a citation whose lines and bytes
    // disagree is worse than one with no lines at all: a reader would open the file at
    // the wrong place and believe they had found the passage.
    let start_line = u32::try_from(text[..start].matches('\n').count() + 1).unwrap_or(u32::MAX);
    let end_line = last_line_of(start_line, slice.as_bytes());
    if (range.start_line, range.end_line) != (start_line, end_line) {
        problems.push(CitationProblem::LinesDisagree {
            document: document.clone(),
            recorded: (range.start_line, range.end_line),
            actual: (start_line, end_line),
        });
    }
    problems
}

/// The last line a cited slice covers, counting from `start_line`.
///
/// Trailing newlines are trimmed before the count, and that is the whole of the rule:
/// a range that stops at the end of line 40 is written `40`, not `41`, and one that
/// swallows a blank line after it is still `40`. Both citation paths — the full document
/// and a retained fragment — go through this, because they did not, and the fragment path
/// counted every newline in the slice. A range captured by `akr source finalize --context
/// block` ends on a chunk boundary, so it almost always ends in one or two newlines, and
/// every retained citation on a cited-only source came back `AKR-S022` for line ranges
/// one or two lines past the ones the record stored
/// (`akr.papercut.collated-19-papercuts-from-evidence-intake`).
fn last_line_of(start_line: u32, slice: &[u8]) -> u32 {
    let mut end = slice.len();
    while end > 0 && slice[end - 1] == b'\n' {
        end -= 1;
    }
    let counted = slice[..end].iter().filter(|byte| **byte == b'\n').count();
    start_line.saturating_add(u32::try_from(counted).unwrap_or(u32::MAX))
}

/// Checks one citation against the catalog and its full or retained bytes.
#[must_use]
pub fn check_citation_at(
    workspace_root: &Path,
    source: &crate::model::Source,
) -> Vec<CitationProblem> {
    let Some(document) = &source.document else {
        return Vec::new();
    };
    let catalog = match load_catalog(workspace_root) {
        Ok(catalog) => catalog,
        Err(error) => {
            return vec![CitationProblem::TextUnavailable {
                document: document.clone(),
                reason: error.to_string(),
            }];
        }
    };
    let Some(entry) = catalog.iter().find(|entry| &entry.id == document) else {
        return vec![CitationProblem::UnknownDocument {
            document: document.clone(),
        }];
    };
    if entry.availability == SourceAvailability::Full {
        return match load_corpus(workspace_root) {
            Ok(corpus) => check_citation(source, &corpus),
            Err(error) => vec![CitationProblem::TextUnavailable {
                document: document.clone(),
                reason: error.to_string(),
            }],
        };
    }
    let Some(range) = &source.range else {
        return Vec::new();
    };
    let Some(fragment) = entry.fragments.iter().find(|fragment| {
        fragment.cited_ranges.iter().any(|cited| cited == range)
            && contains_range(&fragment.captured_range, range)
    }) else {
        return vec![CitationProblem::TextUnavailable {
            document: document.clone(),
            reason: if entry.availability == SourceAvailability::MetadataOnly {
                "the document is metadata-only".into()
            } else {
                "the exact range was not retained".into()
            },
        }];
    };
    let bytes = match read_fragment(workspace_root, entry, fragment) {
        Ok(bytes) => bytes,
        Err(error) => {
            return vec![CitationProblem::TextUnavailable {
                document: document.clone(),
                reason: error.to_string(),
            }];
        }
    };
    let start = usize::try_from(
        range
            .start_byte
            .saturating_sub(fragment.captured_range.start_byte),
    )
    .unwrap_or(usize::MAX);
    let end = usize::try_from(
        range
            .end_byte
            .saturating_sub(fragment.captured_range.start_byte),
    )
    .unwrap_or(usize::MAX);
    let Some(slice) = bytes.get(start..end) else {
        return vec![CitationProblem::TextUnavailable {
            document: document.clone(),
            reason: "retained fragment does not contain the requested bytes".into(),
        }];
    };
    let mut problems = Vec::new();
    if std::str::from_utf8(&bytes[..start]).is_err() || std::str::from_utf8(&bytes[..end]).is_err()
    {
        problems.push(CitationProblem::RangeNotOnBoundary {
            document: document.clone(),
            start: range.start_byte,
            end: range.end_byte,
        });
        return problems;
    }
    if let Some(expected) = &range.excerpt_hash {
        let found = hash_bytes(slice);
        if &found != expected {
            problems.push(CitationProblem::ExcerptHashMismatch {
                document: document.clone(),
                expected: expected.clone(),
                found,
            });
        }
    }
    let start_line = fragment.captured_range.start_line.saturating_add(
        u32::try_from(bytes[..start].iter().filter(|byte| **byte == b'\n').count())
            .unwrap_or(u32::MAX),
    );
    let end_line = last_line_of(start_line, slice);
    if (range.start_line, range.end_line) != (start_line, end_line) {
        problems.push(CitationProblem::LinesDisagree {
            document: document.clone(),
            recorded: (range.start_line, range.end_line),
            actual: (start_line, end_line),
        });
    }
    problems
}

/// Verifies every catalog entry's file hash.
#[must_use]
pub fn verify_catalog(workspace_root: &Path) -> Vec<SourceDiagnostic> {
    let docs = match load_catalog(workspace_root) {
        Ok(d) => d,
        Err(e) => return vec![e],
    };
    let mut diags = Vec::new();
    for doc in &docs {
        if doc.availability == SourceAvailability::CitedOnly {
            for fragment in &doc.fragments {
                if !fragment_path(workspace_root, &fragment.blob).is_file() {
                    diags.push(SourceDiagnostic::MissingFragment {
                        id: doc.id.clone(),
                        blob: fragment.blob.clone(),
                    });
                } else if let Err(error) = read_fragment(workspace_root, doc, fragment) {
                    diags.push(error);
                }
            }
            continue;
        }
        if doc.availability == SourceAvailability::MetadataOnly {
            continue;
        }
        let file = workspace_root.join(&doc.path);
        if !file.is_file() {
            diags.push(SourceDiagnostic::MissingFile {
                id: doc.id.clone(),
                path: doc.path.clone(),
            });
            continue;
        }
        match std::fs::read(&file) {
            Ok(bytes) => {
                let found = hash_bytes(&bytes);
                if found != doc.content_hash {
                    diags.push(SourceDiagnostic::HashMismatch {
                        id: doc.id.clone(),
                        path: doc.path.clone(),
                        expected: doc.content_hash.clone(),
                        found,
                    });
                } else if bytes.len() as u64 != doc.byte_len {
                    // byte_len mismatch is also a hash mismatch in practice, but report plainly
                    diags.push(SourceDiagnostic::HashMismatch {
                        id: doc.id.clone(),
                        path: doc.path.clone(),
                        expected: doc.content_hash.clone(),
                        found: found.clone(),
                    });
                }
            }
            Err(e) => diags.push(SourceDiagnostic::CatalogError(format!(
                "cannot read {}: {e}",
                file.display()
            ))),
        }
    }
    diags
}

// Minimal JSON value without adding serde dependency. We vendor a tiny parser
// using akr_core::json? Not suitable. Instead we depend on `serde_json` via
// `akr-core` Cargo — keep dependency minimal by using `serde_json` if available
// or fall back to manual. To avoid adding dep, we shell out to akr_core::json::Value
// round-trip? For now we implement with akr_core::json::Value parser and emit
// with manual pretty printing — but catalog read/write needs real JSON. The simplest
// dep-free path: use `serde_json` if present, else hand-parse with akr_core::json.
// We choose to use `akr_core::json` for reading and manual emit for writing above
// already handles writing without serde. For reading we need a small shim.
mod serde_json_value {
    // Lightweight JSON value that we build from akr_core::json::Value
    use std::collections::BTreeMap;
    #[derive(Debug, Clone, PartialEq)]
    pub enum Value {
        Null,
        Bool(bool),
        Number(Number),
        String(String),
        Array(Vec<Value>),
        Object(Map),
    }
    pub type Map = BTreeMap<String, Value>;

    #[derive(Debug, Clone, PartialEq)]
    pub struct Number(u64, String);
    impl Number {
        pub fn from(n: u64) -> Self {
            Self(n, n.to_string())
        }
        pub fn as_u64(&self) -> Option<u64> {
            Some(self.0)
        }
    }

    pub fn to_string_pretty(v: &Value) -> Result<String, String> {
        Ok(pretty(v, 0))
    }

    fn pretty(v: &Value, indent: usize) -> String {
        match v {
            Value::Null => "null".into(),
            Value::Bool(b) => {
                if *b {
                    "true".into()
                } else {
                    "false".into()
                }
            }
            Value::Number(n) => n.1.clone(),
            Value::String(s) => format!("\"{}\"", escape(s)),
            Value::Array(a) => {
                if a.is_empty() {
                    return "[]".into();
                }
                let mut s = String::from("[\n");
                for (i, item) in a.iter().enumerate() {
                    s.push_str(&"  ".repeat(indent + 1));
                    s.push_str(&pretty(item, indent + 1));
                    if i + 1 < a.len() {
                        s.push(',');
                    }
                    s.push('\n');
                }
                s.push_str(&"  ".repeat(indent));
                s.push(']');
                s
            }
            Value::Object(m) => {
                if m.is_empty() {
                    return "{}".into();
                }
                let mut s = String::from("{\n");
                let mut first = true;
                for (k, v) in m {
                    if !first {
                        s.push_str(",\n");
                    }
                    first = false;
                    s.push_str(&"  ".repeat(indent + 1));
                    s.push_str(&format!("\"{}\": {}", escape(k), pretty(v, indent + 1)));
                }
                s.push('\n');
                s.push_str(&"  ".repeat(indent));
                s.push('}');
                s
            }
        }
    }

    fn escape(s: &str) -> String {
        let mut out = String::new();
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
                _ => out.push(c),
            }
        }
        out
    }

    impl Value {
        pub fn as_array(&self) -> Option<&Vec<Value>> {
            if let Self::Array(a) = self {
                Some(a)
            } else {
                None
            }
        }
        pub fn as_object(&self) -> Option<&Map> {
            if let Self::Object(m) = self {
                Some(m)
            } else {
                None
            }
        }
    }
}

fn parse_json(raw: &str) -> Result<serde_json_value::Value, SourceDiagnostic> {
    // Delegate to akr_core::json::parse then convert
    let v = crate::json::parse(raw)
        .map_err(|e| SourceDiagnostic::CatalogError(format!("invalid catalog JSON: {e}")))?;
    Ok(convert(v))
}

fn convert(v: crate::json::Value) -> serde_json_value::Value {
    match v {
        crate::json::Value::Null => serde_json_value::Value::Null,
        crate::json::Value::Bool(b) => serde_json_value::Value::Bool(b),
        crate::json::Value::Integer(n) => {
            let u = u64::try_from(n).unwrap_or(0);
            serde_json_value::Value::Number(serde_json_value::Number::from(u))
        }
        crate::json::Value::String(s) => serde_json_value::Value::String(s),
        crate::json::Value::Array(a) => {
            serde_json_value::Value::Array(a.into_iter().map(convert).collect())
        }
        crate::json::Value::Object(m) => {
            let mut map = std::collections::BTreeMap::new();
            for (k, v) in m {
                map.insert(k, convert(v));
            }
            serde_json_value::Value::Object(map)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Source, SourceKind, SourceRange};
    use std::sync::atomic::{AtomicU32, Ordering};

    fn temp_root(name: &str) -> std::path::PathBuf {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "akr-source-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("temp root");
        path
    }

    #[test]
    fn cited_only_fragments_resolve_without_the_original_file() {
        let root = temp_root("cited-only");
        let text = "prefix\nadopt this recommendation\nsuffix\n";
        let selected = "adopt this recommendation";
        let range = SourceRange {
            start_byte: 7,
            end_byte: 7 + selected.len() as u64,
            start_line: 2,
            end_line: 2,
            excerpt_hash: Some(hash_bytes(selected.as_bytes())),
        };
        let captured = SourceRange {
            start_byte: 0,
            end_byte: text.len() as u64,
            start_line: 1,
            end_line: 3,
            excerpt_hash: Some(hash_bytes(text.as_bytes())),
        };
        let blob = hash_bytes(text.as_bytes());
        let document = SourceDocument {
            id: "audit".into(),
            title: "Audit".into(),
            origin: SourceOrigin::External,
            media_type: "text/markdown".into(),
            path: "sources/external/audit.md".into(),
            content_hash: hash_bytes(text.as_bytes()),
            byte_len: text.len() as u64,
            added_at: "2026-08-08".into(),
            observed_at: None,
            scope: None,
            supersedes: None,
            availability: SourceAvailability::CitedOnly,
            fragments: vec![RetainedFragment {
                cited_ranges: vec![range.clone()],
                captured_range: captured,
                content_hash: blob.clone(),
                blob,
            }],
        };
        save_catalog(&root, &[document]).expect("catalog");
        let fragment = fragment_path(&root, &hash_bytes(text.as_bytes()));
        std::fs::create_dir_all(fragment.parent().expect("fragment parent")).expect("parent");
        std::fs::write(&fragment, text).expect("fragment");

        let citation = Source {
            kind: SourceKind::External,
            role: None,
            path: None,
            url: None,
            excerpt: None,
            document: Some("audit".into()),
            range: Some(range),
            use_note: None,
        };
        assert!(check_citation_at(&root, &citation).is_empty());
        assert_eq!(
            resolve_citation(&root, &citation).expect("resolve"),
            Some(selected.as_bytes().to_vec())
        );
        assert!(verify_catalog(&root).is_empty());
        let _ = std::fs::remove_dir_all(root);
    }
}

//! `akr source *` — immutable source library in `sources/`.
//!
//! Registered bytes under `sources/` are content-hashed and immutable while
//! present. `akr source verify` (and `akr check`) report `AKR-S021` on
//! mismatch; `akr source finalize` is the controlled path to retained
//! fragments or metadata-only lineage.

use crate::commands::Output;
use crate::session::{EnvError, Exit, Session};
use akr_core::json::Value;
use akr_core::model::SourceRange;
use akr_core::source::{
    self, RetainedFragment, SourceAvailability, SourceDiagnostic, SourceDocument, SourceOrigin,
};
use std::path::{Path, PathBuf};

/// `akr source add <path> ...`
pub fn add(
    session: &Session,
    path: &Path,
    id: Option<&str>,
    title: Option<&str>,
    origin: Option<&str>,
    observed_at: Option<&str>,
    scope: Option<&str>,
) -> Result<Output, EnvError> {
    let workspace_root = session.root.clone();
    let origin = origin.unwrap_or("external");
    let origin_ty = SourceOrigin::from_str(origin).ok_or_else(|| {
        EnvError::new(
            "AKR-C004",
            format!("origin {origin:?} is not external|internal-reference"),
        )
    })?;

    let bytes = std::fs::read(path)
        .map_err(|e| EnvError::new("AKR-C042", format!("cannot read {}: {e}", path.display())))?;
    let content_hash = source::hash_bytes(&bytes);
    let byte_len = bytes.len() as u64;

    let stem = id
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| derive_id(path, &bytes));
    let safe_id = sanitize_id(&stem);
    if safe_id.is_empty() {
        return Err(EnvError::new("AKR-C004", "source id must be non-empty"));
    }

    // Deduplicate by id.
    let mut catalog = source::load_catalog(&workspace_root).map_err(to_env)?;
    if catalog.iter().any(|d| d.id == safe_id) {
        return Err(EnvError::new(
            "AKR-C042",
            format!("source {safe_id:?} already registered; use `akr source supersede`"),
        ));
    }

    let short = short_hash(&content_hash);
    let file_name = format!("{safe_id}--{short}.md");
    let rel_path = PathBuf::from("sources").join("external").join(&file_name);
    let dest = workspace_root.join(&rel_path);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            EnvError::new(
                "AKR-C042",
                format!("cannot create {}: {e}", parent.display()),
            )
        })?;
    }
    if dest.exists() {
        return Err(EnvError::new(
            "AKR-C042",
            format!("{} already exists", dest.display()),
        ));
    }
    std::fs::write(&dest, &bytes)
        .map_err(|e| EnvError::new("AKR-C042", format!("cannot write {}: {e}", dest.display())))?;

    let title = title.unwrap_or(&safe_id).to_owned();
    let added_at = today_iso();
    let doc = SourceDocument {
        id: safe_id.clone(),
        title,
        origin: origin_ty,
        media_type: "text/markdown".into(),
        path: rel_path.to_string_lossy().replace('\\', "/"),
        content_hash: content_hash.clone(),
        byte_len,
        added_at,
        observed_at: observed_at.map(ToOwned::to_owned),
        scope: scope.map(ToOwned::to_owned),
        supersedes: None,
        availability: SourceAvailability::Full,
        fragments: Vec::new(),
    };
    catalog.push(doc.clone());
    source::save_catalog(&workspace_root, &catalog).map_err(to_env)?;

    let text = format!(
        "registered source {safe_id}\n  {content_hash}\n  {}\n",
        doc.path
    );
    let result = Value::object(vec![
        ("id", Value::string(safe_id)),
        ("content_hash", Value::string(content_hash)),
        ("path", Value::string(doc.path)),
        ("byte_len", Value::integer(byte_len as i64)),
    ]);
    Ok(Output::plain(text, result))
}

/// Ensures `bytes` are registered as a source, returning `(id, was_new)`.
///
/// `akr import` calls this for the imported document, so the excerpts it writes stay
/// verifiable after the original moves or is archived. Identical bytes already
/// registered as `full` are reused by content hash; otherwise a word-for-word copy is
/// stored under `sources/external/` like [`add`], with `origin internal-reference`
/// (legacy material the project preserves, not outside advice) and a `supersedes` link
/// to the previous version of the same file, when there is one.
pub fn ensure_registered(
    workspace_root: &Path,
    original: &Path,
    bytes: &[u8],
) -> Result<(String, bool), EnvError> {
    let content_hash = source::hash_bytes(bytes);
    let byte_len = bytes.len() as u64;
    let mut catalog = source::load_catalog(workspace_root).map_err(to_env)?;

    // Identical bytes already saved: reuse, so re-importing does not duplicate.
    // Only `full` entries count — a finalized copy no longer has the text an
    // excerpt would need to verify against.
    if let Some(existing) = catalog
        .iter()
        .find(|d| d.content_hash == content_hash && d.availability == SourceAvailability::Full)
    {
        return Ok((existing.id.clone(), false));
    }

    let short = short_hash(&content_hash);
    let file_stem = original
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("imported-document");
    let stem_base = sanitize_id(file_stem);
    let stem_base = if stem_base.is_empty() {
        "imported-document".to_owned()
    } else {
        stem_base
    };
    let mut safe_id = sanitize_id(&format!("{stem_base}--{short}"));
    if safe_id.is_empty() {
        safe_id = format!("imported-document--{short}");
    }
    // A derived id that already names different bytes (a short-hash collision, or a
    // previous version under the same name) gets a version suffix, never an overwrite.
    if catalog.iter().any(|d| d.id == safe_id) {
        let mut n = 2usize;
        while catalog.iter().any(|d| d.id == format!("{safe_id}-v{n}")) {
            n += 1;
        }
        safe_id = format!("{safe_id}-v{n}");
    }

    // Link to the previous version of the same file, when there is one: the head of
    // the same-stem chain, i.e. the candidate no other entry supersedes.
    let supersedes = {
        let prefix = format!("{stem_base}-");
        let candidates: Vec<&SourceDocument> = catalog
            .iter()
            .filter(|d| d.id == stem_base || d.id.starts_with(&prefix))
            .collect();
        let mut heads: Vec<&SourceDocument> = candidates
            .iter()
            .filter(|c| {
                !catalog
                    .iter()
                    .any(|d| d.supersedes.as_deref() == Some(&c.id))
            })
            .copied()
            .collect();
        if heads.is_empty() {
            heads = candidates;
        }
        heads
            .into_iter()
            .max_by(|a, b| (&a.added_at, &a.id).cmp(&(&b.added_at, &b.id)))
            .filter(|c| c.content_hash != content_hash)
            .map(|c| c.id.clone())
    };

    let file_name = format!("{safe_id}--{short}.md");
    let rel_path = PathBuf::from("sources").join("external").join(&file_name);
    let dest = workspace_root.join(&rel_path);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            EnvError::new(
                "AKR-C042",
                format!("cannot create {}: {e}", parent.display()),
            )
        })?;
    }
    if dest.exists() {
        return Err(EnvError::new(
            "AKR-C042",
            format!("{} already exists", dest.display()),
        ));
    }
    std::fs::write(&dest, bytes)
        .map_err(|e| EnvError::new("AKR-C042", format!("cannot write {}: {e}", dest.display())))?;

    let title = original
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&safe_id)
        .to_owned();
    catalog.push(SourceDocument {
        id: safe_id.clone(),
        title,
        origin: SourceOrigin::InternalReference,
        media_type: "text/markdown".into(),
        path: rel_path.to_string_lossy().replace('\\', "/"),
        content_hash,
        byte_len,
        added_at: today_iso(),
        observed_at: None,
        scope: None,
        supersedes,
        availability: SourceAvailability::Full,
        fragments: Vec::new(),
    });
    source::save_catalog(workspace_root, &catalog).map_err(to_env)?;
    Ok((safe_id, true))
}

/// `akr source list`
pub fn list(session: &Session, all: bool) -> Result<Output, EnvError> {
    let catalog = source::load_catalog(&session.root).map_err(to_env)?;
    let mut text = String::new();
    if catalog.is_empty() {
        text.push_str("no sources registered\n");
    } else {
        for doc in &catalog {
            if !all && is_superseded(doc, &catalog) {
                continue;
            }
            text.push_str(&format!(
                "{}  {}  {}  {}  {}\n",
                doc.id,
                doc.origin.as_str(),
                doc.availability.as_str(),
                doc.content_hash,
                doc.path
            ));
        }
    }
    let result = Value::object(vec![
        (
            "sources",
            Value::array(catalog.iter().map(doc_to_json).collect()),
        ),
        ("count", Value::integer(catalog.len() as i64)),
    ]);
    Ok(Output::plain(text, result))
}

/// How much of a source `akr source get` and `knowledge.source_get` return.
///
/// The default is a section rather than the whole document, and that is a token decision
/// rather than a convenience one: an advisor report is tens of thousands of tokens, and a
/// tool whose easiest call returns all of it will be called that way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Detail {
    /// The matching chunk only.
    Snippet,
    /// The chunk plus its immediate neighbours (the default).
    #[default]
    Section,
    /// The complete registered document.
    Whole,
}

impl Detail {
    /// Parses the `--detail` argument.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "snippet" => Some(Self::Snippet),
            "section" => Some(Self::Section),
            "whole" => Some(Self::Whole),
            _ => None,
        }
    }

    /// The name used in output and over MCP.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Snippet => "snippet",
            Self::Section => "section",
            Self::Whole => "whole",
        }
    }
}

/// Brings the chunk index up to date before a query reads it.
///
/// `akr search` has done this since P7 through `ensure_search_index`, and the source
/// surface did not: `akr source add` writes the file and the catalog, and only a `build`
/// chunked it, so the obvious next call — search for the passage you just registered —
/// returned `AKR-I031` or nothing at all, with no step in the add's own output to say why
/// (`akr.papercut.collated-19-papercuts-from-evidence-intake`, from
/// Evidence-intake-project). The sync is a no-op when the corpus hash has not moved, so
/// this costs an unchanged library nothing, and `--no-rebuild` still refuses to write.
#[cfg(feature = "fts5")]
fn ensure_source_index(session: &Session) -> Result<(), EnvError> {
    crate::commands::source_index_build(session).map(|_| ())
}

/// `akr source search <query>` — BM25 over the chunk index.
///
/// Every result carries the words `non-authoritative`. `docs/15-external-sources.md` §8:
/// a source hit says where a passage is, and never that the project agreed with it. That
/// label is the difference between an agent citing the report as advice and citing it as
/// the plan of record.
#[cfg(feature = "fts5")]
pub fn search(
    session: &Session,
    query: &str,
    mode: akr_core::store::QueryMode,
    documents: &[String],
    all_versions: bool,
    limit: Option<usize>,
) -> Result<Output, EnvError> {
    let path = akr_core::store::sources_cache_path(&session.akr_dir);
    ensure_source_index(session)?;
    let request = akr_core::store::SourceRequest {
        query: query.to_owned(),
        mode,
        documents: documents.to_vec(),
        all_versions,
        limit,
    };
    let hits = akr_core::store::search_sources(&path, &request)
        .map_err(|e| EnvError::new(e.code.as_str(), e.message))?;

    let mut text = String::new();
    if hits.is_empty() {
        text.push_str("no matches in the source library\n");
    } else {
        for hit in &hits {
            text.push_str(&format!(
                "{:.2}  source:{}\n      external · non-authoritative\n      {}\n      lines {}-{}  chunk {}\n      {}\n\n",
                hit.score,
                hit.document_id,
                if hit.heading.is_empty() {
                    "(no heading)"
                } else {
                    hit.heading.as_str()
                },
                hit.start_line,
                hit.end_line,
                hit.chunk_id,
                hit.snippet,
            ));
        }
    }
    let result = Value::object(vec![
        ("standing", Value::string("non_authoritative")),
        (
            "results",
            Value::array(hits.iter().map(hit_to_json).collect()),
        ),
        ("count", Value::integer(hits.len() as i64)),
    ]);
    Ok(Output::plain(text, result))
}

/// `akr source get --chunk <chunk-id> [--neighbors n]`.
#[cfg(feature = "fts5")]
pub fn get_chunk(session: &Session, chunk_id: &str, neighbors: usize) -> Result<Output, EnvError> {
    let path = akr_core::store::sources_cache_path(&session.akr_dir);
    ensure_source_index(session)?;
    let chunks = akr_core::store::get_chunk(&path, chunk_id, neighbors)
        .map_err(|e| EnvError::new(e.code.as_str(), e.message))?;
    let mut text = String::new();
    for chunk in &chunks {
        text.push_str(&format!(
            "source:{} · non-authoritative · lines {}-{}\n",
            chunk.hit.document_id, chunk.hit.start_line, chunk.hit.end_line
        ));
        text.push_str(&chunk.text);
        if !chunk.text.ends_with('\n') {
            text.push('\n');
        }
        text.push('\n');
    }
    let result = Value::object(vec![
        ("standing", Value::string("non_authoritative")),
        (
            "chunks",
            Value::array(
                chunks
                    .iter()
                    .map(|chunk| {
                        let mut fields = match hit_to_json(&chunk.hit) {
                            Value::Object(fields) => fields,
                            other => vec![("hit".into(), other)],
                        };
                        fields.push(("text".into(), Value::string(chunk.text.clone())));
                        Value::Object(fields)
                    })
                    .collect(),
            ),
        ),
    ]);
    Ok(Output::plain(text, result))
}

#[cfg(feature = "fts5")]
fn hit_to_json(hit: &akr_core::store::SourceHit) -> Value {
    Value::object(vec![
        ("document", Value::string(hit.document_id.clone())),
        ("title", Value::string(hit.document_title.clone())),
        ("path", Value::string(hit.document_path.clone())),
        ("chunk", Value::string(hit.chunk_id.clone())),
        ("heading", Value::string(hit.heading.clone())),
        ("kind", Value::string(hit.kind.clone())),
        ("start_byte", Value::integer(hit.start_byte as i64)),
        ("end_byte", Value::integer(hit.end_byte as i64)),
        ("start_line", Value::integer(i64::from(hit.start_line))),
        ("end_line", Value::integer(i64::from(hit.end_line))),
        ("score", Value::string(format!("{:.4}", hit.score))),
        ("snippet", Value::string(hit.snippet.clone())),
        ("standing", Value::string("non_authoritative")),
    ])
}

/// `akr source get <id>`
pub fn get(
    session: &Session,
    id: &str,
    // `--whole` asks for the entire document, which is also what asking for neither a
    // section nor a line range gets you. It stays in the signature because the flag is
    // documented and a caller may pass it; it selects nothing extra.
    _whole: bool,
    lines: Option<&str>,
    section: Option<&str>,
) -> Result<Output, EnvError> {
    let catalog = source::load_catalog(&session.root).map_err(to_env)?;
    let doc = catalog
        .iter()
        .find(|d| d.id == id)
        .ok_or_else(|| EnvError::new("AKR-C042", format!("source {id:?} not found")))?;
    if doc.availability != SourceAvailability::Full {
        if doc.availability == SourceAvailability::MetadataOnly {
            return Err(EnvError::new(
                "AKR-S022",
                format!("source {id:?} retains metadata only; no source text is available"),
            ));
        }
        let mut output = String::new();
        for fragment in &doc.fragments {
            let bytes = source::read_fragment(&session.root, doc, fragment).map_err(to_env)?;
            output.push_str(&String::from_utf8_lossy(&bytes));
            if !output.ends_with('\n') {
                output.push('\n');
            }
        }
        return Ok(Output::plain(
            output.clone(),
            Value::object(vec![
                ("id", Value::string(doc.id.clone())),
                (
                    "availability",
                    Value::string(doc.availability.as_str().to_owned()),
                ),
                ("text", Value::string(output)),
            ]),
        ));
    }
    let file = session.root.join(&doc.path);
    let bytes = std::fs::read(&file)
        .map_err(|e| EnvError::new("AKR-C042", format!("cannot read {}: {e}", file.display())))?;
    let text_raw = String::from_utf8_lossy(&bytes).into_owned();

    // Verify hash before serving (defensive).
    let found = source::hash_bytes(&bytes);
    if found != doc.content_hash {
        return Err(EnvError::new(
            "AKR-S021",
            format!(
                "registered source bytes do not match their content hash\nsource: {}\nexpected: {}\nfound: {}",
                doc.id, doc.content_hash, found
            ),
        ));
    }

    let output = if let Some(sec) = section {
        extract_section(&text_raw, sec)
            .ok_or_else(|| EnvError::new("AKR-C004", format!("section {sec:?} not found")))?
    } else if let Some(range) = lines {
        extract_lines(&text_raw, range)?
    } else {
        // Neither a section nor a line range: the whole document, which is also what
        // `--whole` asks for. The two used to be separate arms with identical bodies.
        text_raw.clone()
    };

    let mut fields = vec![
        ("id".into(), Value::string(doc.id.clone())),
        ("path".into(), Value::string(doc.path.clone())),
        (
            "content_hash".into(),
            Value::string(doc.content_hash.clone()),
        ),
        ("text".into(), Value::string(output.clone())),
    ];

    // A record cites bytes, but a reader asks for lines, and the gap between the two was
    // left to the caller to close by hand. When the lines were named, hand back the exact
    // locator for them so the citation can be written straight from what was read.
    let mut text = output.clone();
    if let Some(range) = lines {
        let (start, end) = parse_range(range)?;
        let cited = source::locate_lines(&session.root, &doc.id, start, end)
            .map_err(|e| EnvError::new("AKR-S022", e))?;
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&format!(
            "\ncite {}: start_byte {} end_byte {} start_line {} end_line {}\n",
            doc.id, cited.start_byte, cited.end_byte, cited.start_line, cited.end_line
        ));
        let mut locator = vec![
            (
                "start_byte".into(),
                Value::integer(i64::try_from(cited.start_byte).unwrap_or(i64::MAX)),
            ),
            (
                "end_byte".into(),
                Value::integer(i64::try_from(cited.end_byte).unwrap_or(i64::MAX)),
            ),
            (
                "start_line".into(),
                Value::integer(i64::from(cited.start_line)),
            ),
            ("end_line".into(), Value::integer(i64::from(cited.end_line))),
        ];
        if let Some(hash) = &cited.excerpt_hash {
            text.push_str(&format!("      excerpt_hash {hash}\n"));
            locator.push(("excerpt_hash".into(), Value::string(hash.clone())));
        }
        fields.push(("citation".into(), Value::Object(locator)));
    }

    Ok(Output::plain(text, Value::Object(fields)))
}

/// `akr source verify`
pub fn verify(session: &Session) -> Result<Output, EnvError> {
    let diags = source::verify_catalog(&session.root);
    if diags.is_empty() {
        return Ok(Output::plain(
            "all sources verified\n",
            Value::object(vec![("ok", Value::bool(true))]),
        ));
    }
    let mut text = String::new();
    for d in &diags {
        text.push_str(&format!("{d}\n\n"));
    }
    let result = Value::object(vec![
        ("ok", Value::bool(false)),
        (
            "diagnostics",
            Value::array(diags.iter().map(|d| Value::string(d.to_string())).collect()),
        ),
    ]);
    // Exit 1, the ledger-diagnostics status: a source whose bytes no longer match its
    // registration is a fact about the workspace's content, not a broken checkout. A
    // verification that printed `AKR-S021` and exited 0 would pass every CI job that
    // runs it, which is the one thing this command exists to prevent.
    Ok(Output::plain(text, result).with_diagnostics(
        diags.iter().map(source_diagnostic).collect(),
        Exit::Diagnostics,
    ))
}

/// The [`akr_core::diagnostics::Diagnostic`] form of a catalog failure.
fn source_diagnostic(diagnostic: &SourceDiagnostic) -> akr_core::diagnostics::Diagnostic {
    akr_core::diagnostics::Diagnostic {
        code: akr_core::diagnostics::Code::new("AKR-S021"),
        severity: akr_core::diagnostics::Severity::Error,
        rule: None,
        message: diagnostic.to_string(),
        primary: akr_core::diagnostics::Label::new(akr_core::diagnostics::Subject::Ledger),
        notes: Vec::new(),
        help: None,
    }
}

/// `akr source supersede <old-id> <new-path>`
pub fn supersede(
    session: &Session,
    old_id: &str,
    new_path: &Path,
    new_id: Option<&str>,
) -> Result<Output, EnvError> {
    let workspace_root = session.root.clone();
    let mut catalog = source::load_catalog(&workspace_root).map_err(to_env)?;
    let old = catalog
        .iter()
        .find(|d| d.id == old_id)
        .ok_or_else(|| EnvError::new("AKR-C042", format!("source {old_id:?} not found")))?
        .clone();

    let bytes = std::fs::read(new_path).map_err(|e| {
        EnvError::new(
            "AKR-C042",
            format!("cannot read {}: {e}", new_path.display()),
        )
    })?;
    let content_hash = source::hash_bytes(&bytes);
    let byte_len = bytes.len() as u64;

    let new_id_owned = new_id
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("{old_id}--v2"));
    let safe_new_id = sanitize_id(&new_id_owned);
    if catalog.iter().any(|d| d.id == safe_new_id) {
        return Err(EnvError::new(
            "AKR-C042",
            format!("source {safe_new_id:?} already exists"),
        ));
    }
    let short = short_hash(&content_hash);
    let file_name = format!("{safe_new_id}--{short}.md");
    let rel_path = PathBuf::from("sources").join("external").join(&file_name);
    let dest = workspace_root.join(&rel_path);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            EnvError::new(
                "AKR-C042",
                format!("cannot create {}: {e}", parent.display()),
            )
        })?;
    }
    std::fs::write(&dest, &bytes)
        .map_err(|e| EnvError::new("AKR-C042", format!("cannot write {}: {e}", dest.display())))?;

    let doc = SourceDocument {
        id: safe_new_id.clone(),
        title: old.title.clone(),
        origin: old.origin.clone(),
        media_type: "text/markdown".into(),
        path: rel_path.to_string_lossy().replace('\\', "/"),
        content_hash: content_hash.clone(),
        byte_len,
        added_at: today_iso(),
        observed_at: None,
        scope: old.scope.clone(),
        supersedes: Some(old_id.to_owned()),
        availability: SourceAvailability::Full,
        fragments: Vec::new(),
    };
    catalog.push(doc.clone());
    source::save_catalog(&workspace_root, &catalog).map_err(to_env)?;

    let text = format!(
        "superseded {old_id} with {safe_new_id}\n  {content_hash}\n  {}\n",
        doc.path
    );
    let result = Value::object(vec![
        ("old_id", Value::string(old_id.to_owned())),
        ("new_id", Value::string(safe_new_id)),
        ("content_hash", Value::string(content_hash)),
        ("path", Value::string(doc.path)),
    ]);
    Ok(Output::plain(text, result))
}

fn sanitize_id(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
            out.push(c);
        } else {
            // Everything else becomes a hyphen — whitespace and separators included.
            out.push('-');
        }
    }
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    out.trim_matches('-').to_owned()
}

fn derive_id(path: &Path, bytes: &[u8]) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("source");
    let short = short_hash(&source::hash_bytes(bytes));
    format!("{stem}--{short}")
}

fn short_hash(content_hash: &str) -> String {
    let h = content_hash.strip_prefix("sha256:").unwrap_or(content_hash);
    h[..8.min(h.len())].to_owned()
}

fn today_iso() -> String {
    // Use SystemTime like session::current_date, but output ISO.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = (secs / 86_400) as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02}", year, m, d)
}

fn is_superseded(doc: &SourceDocument, catalog: &[SourceDocument]) -> bool {
    catalog
        .iter()
        .any(|d| d.supersedes.as_deref() == Some(&doc.id))
}

fn doc_to_json(d: &SourceDocument) -> Value {
    Value::object(vec![
        ("id", Value::string(d.id.clone())),
        ("title", Value::string(d.title.clone())),
        ("origin", Value::string(d.origin.as_str().to_owned())),
        ("path", Value::string(d.path.clone())),
        (
            "availability",
            Value::string(d.availability.as_str().to_owned()),
        ),
        ("content_hash", Value::string(d.content_hash.clone())),
        ("byte_len", Value::integer(d.byte_len as i64)),
        ("added_at", Value::string(d.added_at.clone())),
        (
            "retained_fragments",
            Value::integer(d.fragments.len() as i64),
        ),
    ])
}

fn source_references(session: &Session, id: &str) -> Vec<(String, Option<SourceRange>)> {
    session
        .ledger
        .records()
        .iter()
        .flat_map(|record| {
            record
                .sources
                .iter()
                .filter(|&source| source.document.as_deref() == Some(id))
                .map(|source| (record.id.to_string(), source.range.clone()))
        })
        .collect()
}

/// `akr source status <id>`.
pub fn status(session: &Session, id: &str) -> Result<Output, EnvError> {
    let catalog = source::load_catalog(&session.root).map_err(to_env)?;
    let doc = catalog
        .iter()
        .find(|doc| doc.id == id)
        .ok_or_else(|| EnvError::new("AKR-C042", format!("source {id:?} not found")))?;
    let references = source_references(session, id);
    let exact = references
        .iter()
        .filter(|(_, range)| range.is_some())
        .count();
    let lineage = references.len() - exact;
    let captured_bytes: usize = doc
        .fragments
        .iter()
        .map(|fragment| {
            fragment
                .captured_range
                .end_byte
                .saturating_sub(fragment.captured_range.start_byte) as usize
        })
        .sum();
    let text = format!(
        "{}\n\navailability     {}\nfull bytes        {}\nexact references  {}\nlineage refs      {}\nretained fragments {}\nretained bytes    {}\n",
        doc.id,
        doc.availability.as_str(),
        doc.byte_len,
        exact,
        lineage,
        doc.fragments.len(),
        captured_bytes,
    );
    Ok(Output::plain(
        text,
        Value::object(vec![
            ("source", doc_to_json(doc)),
            ("exact_references", Value::integer(exact as i64)),
            ("lineage_references", Value::integer(lineage as i64)),
        ]),
    ))
}

/// `akr source dependents <id>`.
pub fn dependents(session: &Session, id: &str) -> Result<Output, EnvError> {
    let catalog = source::load_catalog(&session.root).map_err(to_env)?;
    if !catalog.iter().any(|doc| doc.id == id) {
        return Err(EnvError::new(
            "AKR-C042",
            format!("source {id:?} not found"),
        ));
    }
    let references = source_references(session, id);
    let mut text = format!("{id}\n\n");
    if references.is_empty() {
        text.push_str("no record references\n");
    } else {
        for (record, range) in &references {
            match range {
                Some(range) => text.push_str(&format!(
                    "EXACT  {record}  lines {}-{}  bytes {}..{}\n",
                    range.start_line, range.end_line, range.start_byte, range.end_byte
                )),
                None => text.push_str(&format!("LINEAGE  {record}\n")),
            }
        }
    }
    Ok(Output::plain(
        text,
        Value::object(vec![
            ("source", Value::string(id.to_owned())),
            (
                "dependents",
                Value::array(
                    references
                        .iter()
                        .map(|(record, range)| {
                            Value::object(vec![
                                ("record", Value::string(record.clone())),
                                (
                                    "mode",
                                    Value::string(if range.is_some() {
                                        "exact"
                                    } else {
                                        "lineage"
                                    }),
                                ),
                            ])
                        })
                        .collect(),
                ),
            ),
        ]),
    ))
}

/// Finalizes a full source into cited fragments or metadata-only lineage.
pub fn finalize(
    session: &Session,
    id: &str,
    retain: &str,
    context: &str,
    remove_file: bool,
    dry_run: bool,
) -> Result<Output, EnvError> {
    let mut catalog = source::load_catalog(&session.root).map_err(to_env)?;
    let index = catalog
        .iter()
        .position(|doc| doc.id == id)
        .ok_or_else(|| EnvError::new("AKR-C042", format!("source {id:?} not found")))?;
    let document = catalog[index].clone();
    if document.availability != SourceAvailability::Full {
        return Err(EnvError::new(
            "AKR-S031",
            format!(
                "source {id:?} is already {}",
                document.availability.as_str()
            ),
        ));
    }
    let references = source_references(session, id);
    let exact: Vec<_> = references
        .iter()
        .filter_map(|(_, range)| range.clone())
        .collect();
    if retain == "metadata" && !exact.is_empty() {
        return Err(EnvError::new(
            "AKR-S031",
            format!(
                "source cannot become metadata-only: {} exact record citations require source bytes",
                exact.len()
            ),
        ));
    }
    let loaded = source::load_corpus(&session.root)
        .map_err(|error| EnvError::new("AKR-S021", error.to_string()))?
        .into_iter()
        .find(|item| item.document.id == id)
        .ok_or_else(|| EnvError::new("AKR-S021", format!("source {id:?} is not readable")))?;
    let bytes = loaded.text.as_bytes();
    let mut fragments = Vec::<RetainedFragment>::new();
    if retain == "cited" {
        for range in exact {
            let captured = captured_range(bytes, &range, context, &loaded.text);
            let start = usize::try_from(captured.start_byte).unwrap_or(usize::MAX);
            let end = usize::try_from(captured.end_byte).unwrap_or(usize::MAX);
            let captured_bytes = bytes.get(start..end).ok_or_else(|| {
                EnvError::new("AKR-S022", format!("citation into {id:?} is out of bounds"))
            })?;
            let blob = source::hash_bytes(captured_bytes);
            if let Some(existing) = fragments
                .iter_mut()
                .find(|fragment| fragment.blob == blob && fragment.captured_range == captured)
            {
                if !existing.cited_ranges.contains(&range) {
                    existing.cited_ranges.push(range);
                }
            } else {
                fragments.push(RetainedFragment {
                    cited_ranges: vec![range],
                    captured_range: captured,
                    content_hash: blob.clone(),
                    blob,
                });
            }
        }
    }
    let retained_bytes: usize = fragments
        .iter()
        .map(|fragment| {
            fragment
                .captured_range
                .end_byte
                .saturating_sub(fragment.captured_range.start_byte) as usize
        })
        .sum();
    let plan = format!(
        "Source finalization plan\n\nfull document       {} bytes\nexact references   {}\ncaptured fragments  {}\nretained bytes      {}\nrecords rewritten   0\nsource file removed {}\n",
        document.byte_len,
        references
            .iter()
            .filter(|(_, range)| range.is_some())
            .count(),
        fragments.len(),
        retained_bytes,
        if remove_file { "yes" } else { "no" },
    );
    if dry_run {
        return Ok(Output::plain(
            plan,
            Value::object(vec![
                ("dry_run", Value::bool(true)),
                ("fragments", Value::integer(fragments.len() as i64)),
            ]),
        ));
    }
    for fragment in &fragments {
        let path = source::fragment_path(&session.root, &fragment.blob);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                EnvError::new(
                    "AKR-C042",
                    format!("cannot create {}: {error}", parent.display()),
                )
            })?;
        }
        let start = fragment.captured_range.start_byte as usize;
        let end = fragment.captured_range.end_byte as usize;
        let captured = bytes.get(start..end).ok_or_else(|| {
            EnvError::new("AKR-S022", format!("fragment for {id:?} is out of bounds"))
        })?;
        if !path.exists() {
            std::fs::write(&path, captured).map_err(|error| {
                EnvError::new(
                    "AKR-C042",
                    format!("cannot write {}: {error}", path.display()),
                )
            })?;
        }
    }
    catalog[index].availability = if retain == "metadata" {
        SourceAvailability::MetadataOnly
    } else {
        SourceAvailability::CitedOnly
    };
    catalog[index].fragments = fragments;
    source::save_catalog(&session.root, &catalog).map_err(to_env)?;
    if remove_file {
        let path = session.root.join(&document.path);
        std::fs::remove_file(&path).map_err(|error| {
            EnvError::new(
                "AKR-C042",
                format!("cannot remove {}: {error}", path.display()),
            )
        })?;
    }
    Ok(Output::plain(
        format!(
            "{}finalized {} as {}\n",
            plan,
            id,
            catalog[index].availability.as_str()
        ),
        Value::object(vec![
            ("id", Value::string(id.to_owned())),
            (
                "availability",
                Value::string(catalog[index].availability.as_str().to_owned()),
            ),
            (
                "fragments",
                Value::integer(catalog[index].fragments.len() as i64),
            ),
        ]),
    ))
}

fn captured_range(bytes: &[u8], range: &SourceRange, context: &str, text: &str) -> SourceRange {
    if context == "exact" {
        return range.clone();
    }
    let chunks = akr_core::source::chunk_markdown(text);
    chunks
        .iter()
        .find(|chunk| chunk.start_byte <= range.start_byte && chunk.end_byte >= range.end_byte)
        .map(|chunk| SourceRange {
            start_byte: chunk.start_byte,
            end_byte: chunk.end_byte,
            start_line: chunk.start_line,
            end_line: chunk.end_line,
            excerpt_hash: Some(source::hash_bytes(
                &bytes[chunk.start_byte as usize..chunk.end_byte as usize],
            )),
        })
        .unwrap_or_else(|| range.clone())
}

fn extract_lines(text: &str, range: &str) -> Result<String, EnvError> {
    let (start, end) = parse_range(range)?;
    let lines: Vec<&str> = text.lines().collect();
    if start < 1 || end < start || end as usize > lines.len() {
        return Err(EnvError::new(
            "AKR-C004",
            format!("line range {range:?} out of bounds (1..{})", lines.len()),
        ));
    }
    let slice = &lines[(start as usize - 1)..(end as usize)];
    Ok(slice.join("\n") + "\n")
}

fn parse_range(s: &str) -> Result<(u32, u32), EnvError> {
    let mut parts = s.split(':');
    let a = parts
        .next()
        .ok_or_else(|| EnvError::new("AKR-C004", format!("invalid range {s:?}")))?;
    let b = parts
        .next()
        .ok_or_else(|| EnvError::new("AKR-C004", format!("invalid range {s:?}")))?;
    if parts.next().is_some() {
        return Err(EnvError::new("AKR-C004", format!("invalid range {s:?}")));
    }
    let start: u32 = a
        .parse()
        .map_err(|_| EnvError::new("AKR-C004", format!("invalid range {s:?}")))?;
    let end: u32 = b
        .parse()
        .map_err(|_| EnvError::new("AKR-C004", format!("invalid range {s:?}")))?;
    Ok((start, end))
}

fn extract_section(text: &str, heading: &str) -> Option<String> {
    let needle = heading.trim();
    let lines: Vec<&str> = text.lines().collect();
    let mut start: Option<usize> = None;
    let mut level: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        if let Some((lv, title)) = parse_heading(line)
            && title.trim() == needle
        {
            start = Some(i);
            level = Some(lv);
            break;
        }
    }
    let start = start?;
    let lv = level.unwrap_or(1);
    let mut end = lines.len();
    for (i, line) in lines.iter().enumerate().skip(start + 1) {
        if let Some((l, _)) = parse_heading(line)
            && l <= lv
        {
            end = i;
            break;
        }
    }
    Some(lines[start..end].join("\n") + "\n")
}

fn parse_heading(line: &str) -> Option<(usize, String)> {
    let trimmed = line.trim();
    let mut lvl = 0;
    for c in trimmed.chars() {
        if c == '#' {
            lvl += 1;
        } else {
            break;
        }
    }
    if lvl == 0 || lvl > 6 {
        return None;
    }
    if !trimmed[lvl..].starts_with(' ') && !trimmed[lvl..].is_empty() {
        return None;
    }
    Some((lvl, trimmed[lvl..].trim().to_owned()))
}

fn to_env(d: SourceDiagnostic) -> EnvError {
    match d {
        SourceDiagnostic::HashMismatch { .. } => EnvError::new("AKR-S021", d.to_string()),
        SourceDiagnostic::MissingFile { .. } => EnvError::new("AKR-S021", d.to_string()),
        SourceDiagnostic::MissingFragment { .. } => EnvError::new("AKR-S021", d.to_string()),
        SourceDiagnostic::FragmentHashMismatch { .. } => EnvError::new("AKR-S021", d.to_string()),
        SourceDiagnostic::CatalogError(msg) => EnvError::new("AKR-C042", msg),
    }
}

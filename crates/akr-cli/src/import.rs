//! `akr import` (`docs/07-cli.md` §6, `docs/12-migration.md`).
//!
//! The command is three phases, and only the last one writes:
//!
//! 1. **Read and extract** — `akr_core::import::extract`, the deterministic floor: one
//!    draft claim per heading, verbatim excerpts. `AKR-M001`/`AKR-M002` end it here.
//! 2. **Plan** — keys are proposed from `--namespace` (or the document's location) and
//!    checked against the ledger (`AKR-M012`, `AKR-M013`); the document's own dead links
//!    become `AKR-M022` warnings, and live links with no saved copy in `sources/`
//!    become `AKR-M023`; an empty extraction is `AKR-M011`.
//! 3. **Write** — the document is saved into the source library first (word for word,
//!    content-hashed, reused by hash on re-import), then one `akr_core::ops::import`
//!    call writes every drafted record, the tracking record and its checks in a single
//!    validated, atomic write, each `source` block carrying both the legacy `path` and
//!    the saved copy's `document` id.
//!
//! `--lenient` is the one place warnings are downgraded (D-013), and it changes the exit
//! status only: the warning list is identical with and without it. Without it, any
//! warning is `AKR-M041` and phase 3 never runs — that holds for `--dry-run` too, so the
//! recommended first invocation shows exactly what the writing one will decide.

use crate::args::Profile;
use crate::commands::Output;
use crate::session::{EnvError, Exit, Session};
use akr_core::diagnostics::{Diagnostic, Label, Severity, Subject, codes::migration};
use akr_core::import::{DraftClaim, Extraction, Format, extract, slug_of};
use akr_core::json::Value;
use akr_core::model::{LogicalKey, Segment};
use akr_core::ops::{ImportRequest, ImportedRecord};
use std::path::{Path, PathBuf};

/// Runs `akr import`.
///
/// # Errors
/// [`EnvError`] only for a malformed `--tracking` key. Everything about the document or
/// the ledger — missing source, bad format, collisions — is a diagnostic with exit 1:
/// the invocation was fine, the tool looked, and this is what it found.
pub fn run(
    session: &Session,
    path: &Path,
    namespace: Option<&str>,
    tracking: Option<&str>,
    dry_run: bool,
) -> Result<Output, EnvError> {
    let source = resolve_source(session, path);
    let document = source.document.clone();
    let strict = session.global.profile == Profile::Strict;

    // Phase 1 — read.
    if !source.absolute.exists() {
        return Ok(diagnostics_output(
            session,
            vec![error(
                migration::M001,
                format!("import source {document} does not exist"),
            )],
        ));
    }
    let extension = source
        .absolute
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default();
    let Some(format) = Format::from_extension(&extension) else {
        return Ok(diagnostics_output(
            session,
            vec![
                error(
                    migration::M002,
                    format!("{document}: {extension:?} is not an importable format"),
                )
                .with_help("0.1 imports Markdown (.md) and plain text (.txt) only"),
            ],
        ));
    };
    let text = std::fs::read_to_string(&source.absolute)
        .map_err(|e| EnvError::new("AKR-C011", format!("cannot read {document}: {e}")))?;
    let extraction = extract(&text, format);

    // Phase 2 — plan.
    let namespace = namespace.map_or_else(|| namespace_of(&document), str::to_owned);
    // The catalog answers both "which links are already saved?" (M023) and "is the
    // document itself already saved?" (the plan note). Unreadable means unknown,
    // which warns rather than fails — the write itself re-reads it strictly.
    let catalog = akr_core::source::load_catalog(&session.root).unwrap_or_default();
    let mut warnings = link_warnings(session, &source, &extraction, &catalog);
    if extraction.claims.is_empty() {
        warnings.push(warning(
            migration::M011,
            format!("{document}: no durable claim extracted"),
        ));
    }

    let mut errors = Vec::new();
    let declared =
        Segment::new(&namespace).is_ok_and(|ns| session.ledger.project.namespaces.contains(&ns));
    if !declared {
        errors.push(
            error(
                migration::M013,
                format!("namespace {namespace} is not declared in project.akr"),
            )
            .with_help("declare it in project.akr, or rerun with --namespace <ns>"),
        );
    }

    let tracking = match tracking {
        Some(text) => crate::write::parse_key(text)?,
        None => {
            let stem = source
                .absolute
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let slug = some_slug(&slug_of(&stem), "document");
            parse_or_die(&format!("{namespace}.work.{slug}-import"))
        }
    };

    let mut records = Vec::new();
    if declared {
        for claim in &extraction.claims {
            let key = parse_or_die(&format!("{namespace}.{}.{}", claim.kind.name(), claim.slug));
            if !session.ledger.revisions_of(&key).is_empty() {
                errors.push(
                    error(
                        migration::M012,
                        format!(
                            "{key} already exists; imported records may not overwrite \
                             ledger records"
                        ),
                    )
                    .with_help(&format!(
                        "add the source block to the existing record instead: \
                         `akr revise {key}`"
                    )),
                );
                continue;
            }
            records.push(ImportedRecord {
                key,
                kind: claim.kind,
                title: claim.title.clone(),
                body: claim.excerpt.clone(),
                excerpt: claim.excerpt.clone(),
                check_id: check_id_of(claim),
            });
        }
    }

    if !errors.is_empty() {
        errors.splice(0..0, warnings);
        return Ok(diagnostics_output(session, errors));
    }

    let mut text_out = plan_text(&document, &extraction, &records, &tracking, dry_run);

    // The strict gate (AKR-M041): warnings end the import before anything is written,
    // dry or not, so both invocations decide identically.
    let gated = strict && !warnings.is_empty();
    if gated {
        warnings.push(error(
            migration::M041,
            format!(
                "import produced {} warning{}; rerun with --lenient after reviewing them",
                warnings.len(),
                if warnings.len() == 1 { "" } else { "s" }
            ),
        ));
    }

    if dry_run || gated || records.is_empty() {
        for diagnostic in &warnings {
            text_out.push_str(&akr_core::diagnostics::render(diagnostic, &session.sources));
        }
        text_out.push_str(&save_preview_note(&document, &text, &catalog));
        let reason = if dry_run {
            "--dry-run"
        } else if gated {
            "AKR-M041"
        } else {
            "no durable claims"
        };
        text_out.push_str(&format!("nothing written ({reason})\n"));
        let exit = if gated { Exit::Diagnostics } else { Exit::Ok };
        return Ok(
            Output::plain(text_out, plan_json(&records, &tracking, false))
                .with_diagnostics(warnings, exit),
        );
    }

    // Phase 3 — write. The source copy lands first, so the records never cite an id
    // the catalog does not have; a refused ledger write leaves at most an orphan
    // source, which registers no records and harms nothing.
    let (source_id, was_new) =
        crate::source::ensure_registered(&session.root, &source.absolute, text.as_bytes())?;
    let request = ImportRequest {
        document: source.document,
        records,
        tracking,
        source_document: Some(source_id.clone()),
    };
    let mut output = crate::write::render(
        session,
        akr_core::ops::import(&crate::write::context_of(session), &request),
    )?;
    output.text = {
        let mut combined = text_out;
        for diagnostic in &warnings {
            combined.push_str(&akr_core::diagnostics::render(diagnostic, &session.sources));
        }
        if was_new {
            combined.push_str(&format!("registered source {source_id}\n"));
        } else {
            combined.push_str(&format!("reusing saved source {source_id}\n"));
        }
        combined.push_str(&output.text);
        combined
    };
    warnings.extend(std::mem::take(&mut output.diagnostics));
    output.diagnostics = warnings;
    Ok(output)
}

// -------------------------------------------------------------------------------------
// planning helpers
// -------------------------------------------------------------------------------------

/// The transcript form of `docs/12` §7: what the import proposes, before any warning.
fn plan_text(
    document: &str,
    extraction: &Extraction,
    records: &[ImportedRecord],
    tracking: &LogicalKey,
    dry_run: bool,
) -> String {
    let mut out = format!(
        "{document} — {} durable claim{}, {} paragraph{} skipped\n\n",
        extraction.claims.len(),
        if extraction.claims.len() == 1 {
            ""
        } else {
            "s"
        },
        extraction.paragraphs_skipped,
        if extraction.paragraphs_skipped == 1 {
            ""
        } else {
            "s"
        },
    );
    let verb = if dry_run {
        "would propose"
    } else {
        "proposing    "
    };
    let width = records
        .iter()
        .map(|r| r.key.to_string().len())
        .max()
        .unwrap_or(0);
    for (index, record) in records.iter().enumerate() {
        out.push_str(&format!(
            "  {verb}  {:<width$}   {:<11} (claim {})\n",
            record.key.to_string(),
            record.kind.name(),
            index + 1,
        ));
    }
    if !records.is_empty() {
        let verb = if dry_run {
            "would add    "
        } else {
            "adding       "
        };
        out.push_str(&format!(
            "  {verb}  {} check{} to @{tracking}\n\n",
            records.len(),
            if records.len() == 1 { "" } else { "s" },
        ));
    }
    out
}

fn plan_json(records: &[ImportedRecord], tracking: &LogicalKey, written: bool) -> Value {
    Value::object(vec![
        ("operation", Value::string("import")),
        (
            "proposals",
            Value::array(
                records
                    .iter()
                    .map(|r| {
                        Value::object(vec![
                            ("key", Value::string(r.key.to_string())),
                            ("kind", Value::string(r.kind.name())),
                            ("title", Value::string(r.title.clone())),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("tracking", Value::string(tracking.to_string())),
        ("written", Value::bool(written)),
    ])
}

struct SourceLocation {
    absolute: PathBuf,
    document: String,
    in_workspace: bool,
}

fn resolve_source(session: &Session, path: &Path) -> SourceLocation {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        session.root.join(path)
    };
    let root = &session.root;
    let normalised = normalise(&absolute);
    let document = normalised
        .strip_prefix(root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| normalised.to_string_lossy().replace('\\', "/"));
    SourceLocation {
        in_workspace: normalised.starts_with(root),
        absolute,
        document,
    }
}

/// `AKR-M022` for every relative link in the document that resolves to nothing, and
/// `AKR-M023` for every one that resolves to a file with no saved copy in `sources/`.
///
/// A live plain path is tomorrow's dead link: the M023 warning names it while the bytes
/// are still there to save with `akr source add`.
fn link_warnings(
    session: &Session,
    source: &SourceLocation,
    extraction: &Extraction,
    catalog: &[akr_core::source::SourceDocument],
) -> Vec<Diagnostic> {
    let base = source.absolute.parent().unwrap_or(Path::new(""));
    let head = session
        .commit
        .as_ref()
        .map_or_else(|| "HEAD".to_owned(), |c| c.as_str()[..8].to_owned());
    let mut out = Vec::new();
    for link in &extraction.links {
        let target = normalise(&base.join(&link.target));
        let check = if source.in_workspace || target.is_absolute() {
            target.clone()
        } else {
            session.root.join(&target)
        };
        if !check.exists() {
            out.push(warning(
                migration::M022,
                format!(
                    "source path \"{}\" does not exist at {head} ({document}:{}:{})",
                    target.display(),
                    link.line,
                    link.column,
                    document = source.document
                ),
            ));
            continue;
        }
        // Live but unsaved: an old-style link with no immutable copy behind it.
        let saved = std::fs::read(&check).is_ok_and(|bytes| {
            let hash = akr_core::source::hash_bytes(&bytes);
            catalog.iter().any(|d| {
                d.content_hash == hash
                    && d.availability == akr_core::source::SourceAvailability::Full
            })
        });
        if !saved {
            out.push(
                warning(
                    migration::M023,
                    format!(
                        "source path \"{}\" has no saved copy in sources/ \
                         ({document}:{}:{})",
                        target.display(),
                        link.line,
                        link.column,
                        document = source.document
                    ),
                )
                .with_help("register it with `akr source add` before it moves"),
            );
        }
    }
    out
}

/// The plan note about the document's own saved copy, for runs that write nothing.
///
/// A dry run (or a gated or empty one) must not register anything, but it can still
/// say whether the copy is already there or would be created.
fn save_preview_note(
    document: &str,
    text: &str,
    catalog: &[akr_core::source::SourceDocument],
) -> String {
    let hash = akr_core::source::hash_bytes(text.as_bytes());
    if let Some(existing) = catalog.iter().find(|d| {
        d.content_hash == hash && d.availability == akr_core::source::SourceAvailability::Full
    }) {
        format!("saved copy already registered as source {}\n", existing.id)
    } else {
        format!("would save {document} as a source on import\n")
    }
}

/// Lexical `..`/`.` resolution, for paths that may not exist.
fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// The default namespace: the document's first path segment (`docs/12` §3).
fn namespace_of(document: &str) -> String {
    let first = Path::new(document)
        .components()
        .next()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .unwrap_or_default();
    some_slug(&slug_of(&first), "docs")
}

fn check_id_of(claim: &DraftClaim) -> Segment {
    Segment::new(&format!("{}-claim", claim.slug))
        .unwrap_or_else(|_| Segment::new("imported-claim").expect("a valid literal segment"))
}

fn some_slug(slug: &str, fallback: &str) -> String {
    if slug.is_empty() {
        fallback.to_owned()
    } else {
        slug.to_owned()
    }
}

/// A key built from validated parts; the parts make failure unreachable.
fn parse_or_die(text: &str) -> LogicalKey {
    LogicalKey::parse(text).expect("namespace, kind name and slug are each valid segments")
}

// -------------------------------------------------------------------------------------
// diagnostics
// -------------------------------------------------------------------------------------

fn error(code: akr_core::diagnostics::Code, message: String) -> Diagnostic {
    diagnostic(code, Severity::Error, message)
}

fn warning(code: akr_core::diagnostics::Code, message: String) -> Diagnostic {
    diagnostic(code, Severity::Warning, message)
}

fn diagnostic(
    code: akr_core::diagnostics::Code,
    severity: Severity,
    message: String,
) -> Diagnostic {
    Diagnostic {
        code,
        severity,
        rule: None,
        message,
        primary: Label::new(Subject::Ledger),
        notes: Vec::new(),
        help: None,
    }
}

trait WithHelp {
    fn with_help(self, help: &str) -> Self;
}

impl WithHelp for Diagnostic {
    fn with_help(mut self, help: &str) -> Self {
        self.help = Some(help.to_owned());
        self
    }
}

/// Errors rendered and exit 1: the tool did its job, and this is what it found.
fn diagnostics_output(session: &Session, diagnostics: Vec<Diagnostic>) -> Output {
    let mut text = String::new();
    for diagnostic in &diagnostics {
        text.push_str(&akr_core::diagnostics::render(diagnostic, &session.sources));
    }
    Output::plain(text, Value::Object(Vec::new())).with_diagnostics(diagnostics, Exit::Diagnostics)
}

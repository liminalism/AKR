//! Turning a `knowledge.propose` or `knowledge.revise` payload into a record.
//!
//! # Why this emits AKR source rather than building a `Record`
//!
//! Every content slot has a type — `observed_at` is a commit, `watches` is an array of
//! globs, `review_after` is a date, `method` is an enum member — and that table already
//! exists, in the lowering pass of `akr_core::syntax`. Building a `Record` field by field
//! from JSON would be a second copy of it, and the two would drift the first time a slot
//! was added.
//!
//! So this writes the record out in AKR's own syntax and hands it to the parser. The
//! grammar stays the single source of truth for what a slot means, and a payload that
//! names a slot the kind does not have fails with the same `AKR-T***` diagnostic an
//! author would get — which is what §5's `schema` class is for.

use std::path::Path;

use akr_core::json::Value;
use akr_core::model::{Class, ContentSlot, Kind, LogicalKey, Record, Relation};

use crate::errors::ToolError;

/// Everything a payload does not carry: who the record is, and where the workspace is.
///
/// `root` is only needed to resolve a citation given by line into the byte range the
/// language stores; see [`source_range`].
pub struct Heading<'a> {
    /// The workspace root, for reading a cited source document.
    pub root: &'a Path,
    /// The project name the file declares.
    pub project: &'a str,
    /// The record's key.
    pub key: &'a LogicalKey,
    /// The revision being written.
    pub revision: u32,
    /// The record's kind.
    pub kind: Kind,
    /// The one-line title.
    pub title: &'a str,
    /// The lifecycle state, when the caller named one.
    pub state: Option<&'a str>,
}

/// Renders a tool payload as AKR source for one record.
///
/// # Errors
/// [`ToolError`] of class `usage` when a field is the wrong JSON shape — an object where
/// an array was needed, a slot name that is not a slot at all.
pub fn to_source(heading: &Heading, payload: &Value) -> Result<String, ToolError> {
    let Heading {
        root,
        project,
        key,
        revision,
        kind,
        title,
        state,
    } = *heading;
    let mut out = format!("akr 0.1\nproject {project}\n\n");
    out.push_str(&format!("record {key}/{revision} : {} {{\n", kind.name()));
    out.push_str(&format!("    title {}\n", quote(title)));
    if let Some(state) = state {
        out.push_str(&format!("    state {state}\n"));
    }

    if let Some(scope) = payload.get("scope") {
        out.push_str(&format!("    scope [ {} ]\n", scope_terms(scope)?));
    }
    if payload.get("topic").is_some() && kind.class() != Class::Normative {
        return Err(ToolError::new(
            "AKR-C004",
            format!(
                "`topic` is only valid on normative kinds; kind `{}` does not accept it",
                kind.name()
            ),
        ));
    }
    if let Some(topic) = payload.get("topic").and_then(Value::as_str) {
        out.push_str(&format!("    topic {topic}\n"));
    }

    // Slots in the kind's declared order, so the emitted text is close to canonical before
    // the formatter ever sees it. The formatter is authoritative; this only keeps the
    // intermediate readable when a diagnostic points into it.
    if let Some(Value::Object(slots)) = payload.get("slots") {
        for spec in kind.content_slots() {
            let name = spec.slot.name();
            if let Some((_, value)) = slots.iter().find(|(k, _)| k == name) {
                out.push_str(&slot_line(spec.slot, value)?);
            }
        }
        for (name, _) in slots {
            if ContentSlot::from_name(name)
                .is_none_or(|slot| !kind.content_slots().iter().any(|spec| spec.slot == slot))
            {
                return Err(ToolError::new(
                    "AKR-T002",
                    format!(
                        "`{name}` is not a slot of kind `{}`; valid slots: {}",
                        kind.name(),
                        kind.content_slots()
                            .iter()
                            .map(|spec| format!(
                                "{} ({})",
                                spec.slot.name(),
                                spec.slot.value_type()
                            ))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ));
            }
        }
    }

    if let Some(Value::Array(claims)) = payload.get("claims") {
        for claim in claims {
            let anchor = claim
                .get("anchor")
                .and_then(Value::as_str)
                .ok_or_else(|| ToolError::new("AKR-C004", "each claim needs an `anchor`"))?;
            let text = claim
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| ToolError::new("AKR-C004", "each claim needs a `text`"))?;
            out.push_str(&format!("    claim {anchor} {{\n"));
            out.push_str(&format!("        text {}\n", prose(text, 8)));
            out.push_str("    }\n");
        }
    }
    if let Some(Value::Array(checks)) = payload.get("acceptance") {
        out.push_str("    acceptance {\n");
        for check in checks {
            out.push_str(&check_block(check)?);
        }
        out.push_str("    }\n");
    }

    if let Some(Value::Array(retired)) = payload.get("retired_claims") {
        let anchors: Vec<String> = retired
            .iter()
            .filter_map(Value::as_str)
            .map(ToOwned::to_owned)
            .collect();
        if !anchors.is_empty() {
            out.push_str(&format!("    retired_claims [ {} ]\n", anchors.join(", ")));
        }
    }

    if let Some(Value::Object(relations)) = payload.get("relations") {
        for (name, targets) in relations {
            let relation = Relation::from_name(name)
                .ok_or_else(|| ToolError::new("AKR-C004", format!("`{name}` is not a relation")))?;
            let refs: Vec<String> = targets
                .as_array()
                .unwrap_or_default()
                .iter()
                .filter_map(Value::as_str)
                .map(reference)
                .collect();
            if !refs.is_empty() {
                out.push_str(&format!(
                    "    {} [ {} ]\n",
                    relation.name(),
                    refs.join(", ")
                ));
            }
        }
    }

    // V-023 refuses a live `contradicts` edge between two live records unless one of them
    // says the contradiction is knowingly tolerated, and `acknowledged true` is how it
    // says so (D-023). The marker is a common slot rather than a content slot, so it never
    // appeared in `slots`, and no other field carried it: an agent that hit `AKR-R041` on
    // a `contradicts` it meant to keep had no way through this surface to acknowledge it,
    // and dropped the relation instead — which is exactly the contradiction going
    // unrecorded that the rule exists to prevent
    // (`akr.papercut.collated-19-papercuts-from-evidence-intake`, from
    // Evidence-intake-project).
    if payload
        .get("acknowledged")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        out.push_str("    acknowledged true\n");
    }

    if let Some(Value::Array(sources)) = payload.get("sources") {
        for source in sources {
            let Value::Object(fields) = source else {
                return Err(ToolError::new(
                    "AKR-C004",
                    "each source entry must be an object",
                ));
            };
            let kind = fields
                .iter()
                .find_map(|(name, value)| (name == "kind").then_some(value))
                .and_then(Value::as_str)
                .ok_or_else(|| ToolError::new("AKR-C004", "each source needs a `kind`"))?;
            if !matches!(kind, "legacy" | "external" | "internal") {
                return Err(ToolError::new(
                    "AKR-C004",
                    format!(
                        "`source.kind` must be one of legacy, external, internal, found {kind:?}"
                    ),
                ));
            }
            out.push_str("    source {\n");
            out.push_str(&format!("        kind {kind}\n"));
            if let Some(role) = source_str(fields, "role") {
                if !matches!(
                    role,
                    "origin" | "rationale" | "evidence" | "constraint" | "example"
                ) {
                    return Err(ToolError::new(
                        "AKR-C004",
                        format!("unknown source role {role:?}"),
                    ));
                }
                out.push_str(&format!("        role {role}\n"));
            }
            if let Some(path) = fields
                .iter()
                .find_map(|(name, value)| (name == "path").then_some(value))
                .and_then(Value::as_str)
            {
                out.push_str(&format!("        path {}\n", quote(path)));
            }
            if let Some(url) = fields
                .iter()
                .find_map(|(name, value)| (name == "url").then_some(value))
                .and_then(Value::as_str)
            {
                out.push_str(&format!("        url {}\n", quote(url)));
            }
            if let Some(excerpt) = fields
                .iter()
                .find_map(|(name, value)| (name == "excerpt").then_some(value))
                .and_then(Value::as_str)
            {
                out.push_str(&format!("        excerpt {}\n", prose(excerpt, 8)));
            }
            if let Some(document) = source_str(fields, "document") {
                out.push_str(&format!("        document {}\n", quote(document)));
            }
            let range = source_range(root, fields)?;
            if let Some(range) = &range {
                out.push_str(&format!("        start_byte {}\n", range.start_byte));
                out.push_str(&format!("        end_byte {}\n", range.end_byte));
                out.push_str(&format!("        start_line {}\n", range.start_line));
                out.push_str(&format!("        end_line {}\n", range.end_line));
            }
            if let Some(hash) = source_str(fields, "excerpt_hash")
                .map(ToOwned::to_owned)
                .or_else(|| range.and_then(|range| range.excerpt_hash))
            {
                out.push_str(&format!("        excerpt_hash {}\n", quote(&hash)));
            }
            if let Some(use_note) = source_str(fields, "use") {
                out.push_str(&format!("        use {}\n", prose(use_note, 8)));
            }
            out.push_str("    }\n");
        }
    }

    if let Some(author) = payload.get("author").and_then(Value::as_str) {
        out.push_str(&format!("    author {}\n", quote(author)));
    }
    if let Some(created) = payload.get("created_at").and_then(Value::as_str) {
        out.push_str(&format!("    created_at {created}\n"));
    }
    out.push_str("}\n");
    Ok(out)
}

/// The exact locator for one `source` block, completing a line-only citation.
///
/// The language wants all four coordinates or none (`AKR-T012`), because bytes are what
/// resolves and lines are what a reader opens the file at. Demanding all four of the
/// caller was a different thing: an author reads a registered document by line, and had
/// to count bytes by hand to cite it. So lines alone are now enough when the citation
/// names a `document` — the byte range is read off the registered bytes themselves, and
/// carries the excerpt hash of exactly what it selected.
///
/// Bytes alone, or bytes and lines together, are passed through untouched: an author who
/// has the offsets already is not second-guessed.
fn source_range(
    root: &Path,
    fields: &[(String, Value)],
) -> Result<Option<akr_core::model::SourceRange>, ToolError> {
    let start_byte = source_integer(fields, "start_byte")?;
    let end_byte = source_integer(fields, "end_byte")?;
    let start_line = source_integer(fields, "start_line")?;
    let end_line = source_integer(fields, "end_line")?;

    if let (None, None, Some(start_line), Some(end_line)) =
        (start_byte, end_byte, start_line, end_line)
    {
        let document = source_str(fields, "document").ok_or_else(|| {
            ToolError::new(
                "AKR-C004",
                "a source citing lines needs `document` too: without a registered \
                 document there is nothing to read the byte offsets from. Give \
                 start_byte and end_byte instead, or name the document.",
            )
        })?;
        let lines = |value: i64| u32::try_from(value).unwrap_or(u32::MAX);
        return akr_core::source::locate_lines(root, document, lines(start_line), lines(end_line))
            .map(Some)
            .map_err(|error| ToolError::new("AKR-C004", error));
    }

    match (start_byte, end_byte, start_line, end_line) {
        (Some(start_byte), Some(end_byte), Some(start_line), Some(end_line)) => {
            Ok(Some(akr_core::model::SourceRange {
                start_byte: u64::try_from(start_byte).unwrap_or(0),
                end_byte: u64::try_from(end_byte).unwrap_or(0),
                start_line: u32::try_from(start_line).unwrap_or(0),
                end_line: u32::try_from(end_line).unwrap_or(0),
                excerpt_hash: source_str(fields, "excerpt_hash").map(ToOwned::to_owned),
            }))
        }
        (None, None, None, None) => Ok(None),
        _ => Err(ToolError::new(
            "AKR-C004",
            "a source range needs start_byte and end_byte together with start_line and \
             end_line, or start_line and end_line alone with a `document` to read the \
             offsets from, or none of them",
        )),
    }
}

fn source_str<'a>(fields: &'a [(String, Value)], name: &str) -> Option<&'a str> {
    fields
        .iter()
        .find_map(|(field, value)| (field == name).then_some(value))
        .and_then(Value::as_str)
}

fn source_integer(fields: &[(String, Value)], name: &str) -> Result<Option<i64>, ToolError> {
    let Some(value) = fields
        .iter()
        .find_map(|(field, value)| (field == name).then_some(value))
    else {
        return Ok(None);
    };
    let value = value
        .as_integer()
        .ok_or_else(|| ToolError::new("AKR-C004", format!("source.{name} must be an integer")))?;
    if value < 0 {
        return Err(ToolError::new(
            "AKR-C004",
            format!("source.{name} must not be negative"),
        ));
    }
    Ok(Some(value))
}

/// Parses the emitted source back into a record.
///
/// # Errors
/// [`ToolError`] of class `schema` when the payload does not describe a legal record.
pub fn parse(source: &str, key: &LogicalKey) -> Result<Record, ToolError> {
    let mut sources = akr_core::diagnostics::SourceMap::new();
    let file = sources.add("<tool payload>", source);
    let parsed = akr_core::syntax::parse(source, file);
    let Some(tree) = parsed.file else {
        return Err(schema_error(&parsed.diagnostics));
    };
    let (ledger, lowered) =
        akr_core::syntax::lower::lower_all(&[("<tool payload>".to_owned(), tree)]);
    let all: Vec<_> = parsed.diagnostics.into_iter().chain(lowered).collect();
    if all
        .iter()
        .any(|d| d.severity == akr_core::diagnostics::Severity::Error)
    {
        return Err(schema_error(&all));
    }
    ledger
        .records()
        .iter()
        .find(|record| record.id.key == *key)
        .cloned()
        .ok_or_else(|| ToolError::new("AKR-T002", "the payload describes no record"))
}

fn schema_error(diagnostics: &[akr_core::diagnostics::Diagnostic]) -> ToolError {
    let first = diagnostics
        .iter()
        .find(|d| d.severity == akr_core::diagnostics::Severity::Error);
    let (code, message) = first.map_or(
        (
            "AKR-T002",
            "the payload does not describe a record".to_owned(),
        ),
        |d| (d.code.as_str(), d.message.clone()),
    );
    ToolError::new(code, message).with_diagnostics(
        diagnostics
            .iter()
            .map(|d| {
                Value::object(vec![
                    ("code", Value::string(d.code.as_str())),
                    ("severity", Value::string("error")),
                    ("message", Value::string(d.message.clone())),
                ])
            })
            .collect(),
    )
}

// -------------------------------------------------------------------------------------
// slot rendering
// -------------------------------------------------------------------------------------

/// One `    <slot> <value>` line, typed as the lowering pass types it.
fn slot_line(slot: ContentSlot, value: &Value) -> Result<String, ToolError> {
    let name = slot.name();
    let rendered = match slot {
        // Commits and dates are bare tokens, never quoted strings.
        ContentSlot::ObservedAt | ContentSlot::AsOf => {
            let text = string(value, name)?;
            let hash = text.strip_prefix("git:").unwrap_or(&text);
            format!("git:{hash}")
        }
        ContentSlot::ReviewAfter | ContentSlot::Target => string(value, name)?,
        // Enum members are bare words.
        ContentSlot::Method | ContentSlot::Result | ContentSlot::Confidence => string(value, name)?,
        ContentSlot::Watches | ContentSlot::Aliases | ContentSlot::Collated => {
            let items: Vec<String> = array(value, name)?
                .iter()
                .filter_map(Value::as_str)
                .map(quote)
                .collect();
            format!("[ {} ]", items.join(", "))
        }
        ContentSlot::Exceptions => {
            let items: Vec<String> = array(value, name)?
                .iter()
                .filter_map(Value::as_str)
                .map(reference)
                .collect();
            format!("[ {} ]", items.join(", "))
        }
        _ => {
            let text = string(value, name)?;
            if text.contains('\n') || text.len() > 60 {
                prose(&text, 4)
            } else {
                quote(&text)
            }
        }
    };
    Ok(format!("    {name} {rendered}\n"))
}

/// One `    check <id> { ... }` block, for the `acceptance` array (D-016).
///
/// This is the milestone gap the CLI's `--from` workaround exists for: `knowledge.propose`
/// had no way to author an acceptance block, so a milestone (V-008 requires one) could not
/// be created over MCP at all.
fn check_block(value: &Value) -> Result<String, ToolError> {
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::new("AKR-C004", "each acceptance check needs an `id`"))?;
    let statement = value
        .get("statement")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::new("AKR-C004", "each acceptance check needs a `statement`"))?;
    let method = value
        .get("method")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::new("AKR-C004", "each acceptance check needs a `method`"))?;

    let mut out = format!("        check {id} {{\n");
    out.push_str(&format!("            statement {}\n", prose(statement, 12)));
    out.push_str(&format!("            method {method}\n"));
    if let Some(command) = value.get("command").and_then(Value::as_str) {
        out.push_str(&format!("            command {}\n", quote(command)));
    }
    if let Some(Value::Array(refs)) = value.get("verified_by") {
        let items: Vec<String> = refs
            .iter()
            .filter_map(Value::as_str)
            .map(reference)
            .collect();
        if !items.is_empty() {
            out.push_str(&format!(
                "            verified_by [ {} ]\n",
                items.join(", ")
            ));
        }
    }
    out.push_str("        }\n");
    Ok(out)
}

fn string(value: &Value, slot: &str) -> Result<String, ToolError> {
    value
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| ToolError::new("AKR-C004", format!("slot `{slot}` expects a string")))
}

fn array<'a>(value: &'a Value, slot: &str) -> Result<&'a [Value], ToolError> {
    value
        .as_array()
        .ok_or_else(|| ToolError::new("AKR-C004", format!("slot `{slot}` expects an array")))
}

fn scope_terms(value: &Value) -> Result<String, ToolError> {
    let items = value
        .as_array()
        .ok_or_else(|| ToolError::new("AKR-C004", "`scope` expects an array"))?;
    let mut terms = Vec::new();
    for item in items {
        // Both the object form of §3 (`{"form": "all"}`) and the plain string form an
        // agent will reach for first.
        let term = match item {
            Value::String(text) if text == "all" => "all".to_owned(),
            Value::String(text) if text.starts_with('@') => format!("ref {text}"),
            Value::String(text) if text.starts_with("path ") || text.starts_with("ref ") => {
                return Err(ToolError::new(
                    "AKR-C004",
                    format!(
                        "scope value {text:?} uses rendered AKR syntax; pass a bare glob such as \"src/**\" or a reference such as \"@key\""
                    ),
                ));
            }
            Value::String(text) => format!("path {}", quote(text)),
            Value::Object(_) => match item.get("form").and_then(Value::as_str) {
                Some("all") => "all".to_owned(),
                Some("path") => {
                    let glob = item.get("glob").and_then(Value::as_str).ok_or_else(|| {
                        ToolError::new(
                            "AKR-C004",
                            "a path scope object needs `glob`, for example {\"form\":\"path\",\"glob\":\"src/**\"}",
                        )
                    })?;
                    format!("path {}", quote(glob))
                }
                Some("ref") => {
                    let target = item.get("ref").and_then(Value::as_str).ok_or_else(|| {
                        ToolError::new(
                            "AKR-C004",
                            "a ref scope object needs `ref`, for example {\"form\":\"ref\",\"ref\":\"@key\"}",
                        )
                    })?;
                    format!("ref {}", reference(target))
                }
                _ => {
                    return Err(ToolError::new(
                        "AKR-C004",
                        "unknown scope form; use \"all\", a bare glob such as \"src/**\", a reference such as \"@key\", or an object with form all|path|ref",
                    ));
                }
            },
            _ => return Err(ToolError::new("AKR-C004", "unknown scope term")),
        };
        terms.push(term);
    }
    Ok(terms.join(", "))
}

fn reference(text: &str) -> String {
    if text.starts_with('@') {
        text.to_owned()
    } else {
        format!("@{text}")
    }
}

fn quote(text: &str) -> String {
    let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// A triple-quoted prose block, indented to `indent`.
fn prose(text: &str, indent: usize) -> String {
    let pad = " ".repeat(indent + 4);
    let mut out = String::from("\"\"\"\n");
    for line in text.lines() {
        out.push_str(&format!("{pad}{}\n", line.trim_end()));
    }
    out.push_str(&format!("{}\"\"\"", " ".repeat(indent + 4)));
    out
}

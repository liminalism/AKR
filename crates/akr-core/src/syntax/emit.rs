//! Model to source: renders a [`Record`] as canonical AKR text.
//!
//! The inverse of [`super::lower`], and the piece the write operations of
//! [`crate::ops`] are built on. Everything the MCP surface writes goes through here too.
//!
//! # Canonicality is delegated, not duplicated
//!
//! This module emits *valid* text and then runs it through the parser and the formatter,
//! which are the arbiters of canonical form (D-012). Re-implementing slot ordering here
//! would give the project two answers to the same question, and they would drift.

use super::cst::{self, escape};
use super::{format, parse};
use crate::diagnostics::FileId;
use crate::model::{
    Acceptance, Check, Claim, ContentValue, Disposition, Record, ScopeTerm, Source, SourceKind,
};
use std::fmt::Write as _;

/// Renders a record as canonical source, without a file header.
///
/// The result always ends with `}` and a newline, and re-parsing it yields a record equal
/// to the input — `tests/emit.rs` holds that round trip.
#[must_use]
pub fn record_text(record: &Record, project: &str) -> String {
    let raw = format!("akr 0.1\nproject {project}\n\n{}", raw_record(record));
    let parsed = parse(&raw, FileId(0));
    let Some(file) = &parsed.file else {
        // Emission produced something unparseable, which is a bug here rather than in the
        // caller's data. Return the raw text so the caller's validation reports it.
        return raw_record(record);
    };
    let canonical = format(file);
    canonical
        .find("record ")
        .map_or_else(|| raw_record(record), |at| canonical[at..].to_owned())
}

/// Renders a record as a CST node, ready to splice into a parsed file.
///
/// Returns `None` only if emission produced text the parser rejects, which would be a bug
/// in this module.
#[must_use]
pub fn record_node(record: &Record, project: &str) -> Option<cst::Record> {
    let raw = format!(
        "akr 0.1\nproject {project}\n\n{}",
        record_text(record, project)
    );
    let parsed = parse(&raw, FileId(0));
    if !parsed.diagnostics.is_empty() {
        return None;
    }
    parsed.file?.items.into_iter().find_map(|item| match item {
        cst::Item::Record(record) => Some(record),
        _ => None,
    })
}

// -------------------------------------------------------------------------------------
// raw emission
// -------------------------------------------------------------------------------------

fn raw_record(record: &Record) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "record {}/{} : {} {{",
        record.id.key, record.id.revision, record.kind
    );
    let _ = writeln!(out, "    title \"{}\"", escape(&record.title));
    let _ = writeln!(out, "    state {}", record.state);
    if !record.scope.is_empty() {
        let terms: Vec<String> = record.scope.iter().map(scope_term).collect();
        let _ = writeln!(out, "    scope [ {} ]", terms.join(", "));
    }
    if let Some(topic) = &record.topic {
        let _ = writeln!(out, "    topic {topic}");
    }
    for spec in record.kind.content_slots() {
        if let Some(value) = record.content.get(&spec.slot) {
            emit_value(&mut out, 1, spec.slot.name(), value);
        }
    }
    for claim in &record.claims {
        emit_claim(&mut out, claim);
    }
    if !record.retired_claims.is_empty() {
        let names: Vec<String> = record
            .retired_claims
            .iter()
            .map(ToString::to_string)
            .collect();
        let _ = writeln!(out, "    retired_claims [ {} ]", names.join(", "));
    }
    if let Some(acceptance) = &record.acceptance {
        emit_acceptance(&mut out, acceptance);
    }
    for disposition in &record.dispositions {
        emit_disposition(&mut out, disposition);
    }
    for (relation, targets) in &record.relations {
        if targets.is_empty() {
            continue;
        }
        let refs: Vec<String> = targets.iter().map(ToString::to_string).collect();
        let _ = writeln!(out, "    {relation} [ {} ]", refs.join(", "));
    }
    if record.acknowledged {
        let _ = writeln!(out, "    acknowledged true");
    }
    if let Some(author) = &record.author {
        let _ = writeln!(out, "    author \"{}\"", escape(author));
    }
    if let Some(created_at) = record.created_at {
        let _ = writeln!(out, "    created_at {created_at}");
    }
    for source in &record.sources {
        emit_source(&mut out, source);
    }
    out.push_str("}\n");
    out
}

fn scope_term(term: &ScopeTerm) -> String {
    match term {
        ScopeTerm::All => "all".to_owned(),
        ScopeTerm::Ref(reference) => format!("ref {reference}"),
        ScopeTerm::Path(glob) => format!("path \"{}\"", escape(glob.as_str())),
    }
}

fn indent(level: usize) -> String {
    " ".repeat(level * 4)
}

fn emit_prose(out: &mut String, level: usize, name: &str, text: &str) {
    let pad = indent(level);
    let inner = indent(level + 1);
    let _ = writeln!(out, "{pad}{name} \"\"\"");
    for line in text.lines() {
        if line.is_empty() {
            out.push('\n');
        } else {
            let _ = writeln!(out, "{inner}{line}");
        }
    }
    let _ = writeln!(out, "{inner}\"\"\"");
}

fn emit_value(out: &mut String, level: usize, name: &str, value: &ContentValue) {
    let pad = indent(level);
    match value {
        ContentValue::Prose(text) => emit_prose(out, level, name, text),
        ContentValue::Text(text) => {
            let _ = writeln!(out, "{pad}{name} \"{}\"", escape(text));
        }
        ContentValue::Date(date) => {
            let _ = writeln!(out, "{pad}{name} {date}");
        }
        ContentValue::Commit(commit) => {
            let _ = writeln!(out, "{pad}{name} {commit}");
        }
        ContentValue::Enum(member) => {
            let _ = writeln!(out, "{pad}{name} {member}");
        }
        ContentValue::Strings(items) => {
            let rendered: Vec<String> =
                items.iter().map(|s| format!("\"{}\"", escape(s))).collect();
            let _ = writeln!(out, "{pad}{name} [ {} ]", rendered.join(", "));
        }
        ContentValue::Globs(items) => {
            let rendered: Vec<String> = items
                .iter()
                .map(|g| format!("\"{}\"", escape(g.as_str())))
                .collect();
            let _ = writeln!(out, "{pad}{name} [ {} ]", rendered.join(", "));
        }
        ContentValue::Refs(items) => {
            let rendered: Vec<String> = items.iter().map(ToString::to_string).collect();
            let _ = writeln!(out, "{pad}{name} [ {} ]", rendered.join(", "));
        }
    }
}

fn emit_claim(out: &mut String, claim: &Claim) {
    let _ = writeln!(out, "    claim {} {{", claim.anchor);
    emit_prose(out, 2, "text", &claim.text);
    if !claim.supported_by.is_empty() {
        let refs: Vec<String> = claim.supported_by.iter().map(ToString::to_string).collect();
        let _ = writeln!(out, "        supported_by [ {} ]", refs.join(", "));
    }
    out.push_str("    }\n");
}

fn emit_acceptance(out: &mut String, acceptance: &Acceptance) {
    out.push_str("    acceptance {\n");
    for check in &acceptance.checks {
        emit_check(out, check);
    }
    out.push_str("    }\n");
}

fn emit_check(out: &mut String, check: &Check) {
    let _ = writeln!(out, "        check {} {{", check.id);
    emit_prose(out, 3, "statement", &check.statement);
    let _ = writeln!(out, "            method {}", check.method.name());
    if let Some(command) = &check.command {
        let _ = writeln!(out, "            command \"{}\"", escape(command));
    }
    if !check.verified_by.is_empty() {
        let refs: Vec<String> = check.verified_by.iter().map(ToString::to_string).collect();
        let _ = writeln!(out, "            verified_by [ {} ]", refs.join(", "));
    }
    if let Some(commit) = &check.passed_at {
        let _ = writeln!(out, "            passed_at {commit}");
    }
    out.push_str("        }\n");
}

fn emit_disposition(out: &mut String, disposition: &Disposition) {
    let _ = writeln!(out, "    disposition {} {{", disposition.target);
    let _ = writeln!(out, "        outcome {}", disposition.outcome.name());
    if let Some(into) = &disposition.into {
        let _ = writeln!(out, "        into {into}");
    }
    if let Some(note) = &disposition.note {
        emit_prose(out, 2, "note", note);
    }
    out.push_str("    }\n");
}

fn emit_source(out: &mut String, source: &Source) {
    out.push_str("    source {\n");
    let kind = match source.kind {
        SourceKind::Legacy => "legacy",
        SourceKind::External => "external",
        SourceKind::Internal => "internal",
    };
    let _ = writeln!(out, "        kind {kind}");
    if let Some(role) = source.role {
        let _ = writeln!(out, "        role {}", role.as_str());
    }
    if let Some(document) = &source.document {
        let _ = writeln!(out, "        document \"{}\"", escape(document));
    }
    if let Some(path) = &source.path {
        let _ = writeln!(out, "        path \"{}\"", escape(path));
    }
    if let Some(url) = &source.url {
        let _ = writeln!(out, "        url \"{}\"", escape(url));
    }
    if let Some(range) = &source.range {
        let _ = writeln!(out, "        start_byte {}", range.start_byte);
        let _ = writeln!(out, "        end_byte {}", range.end_byte);
        let _ = writeln!(out, "        start_line {}", range.start_line);
        let _ = writeln!(out, "        end_line {}", range.end_line);
        if let Some(hash) = &range.excerpt_hash {
            let _ = writeln!(out, "        excerpt_hash \"{}\"", escape(hash));
        }
    }
    if let Some(excerpt) = &source.excerpt {
        emit_prose(out, 2, "excerpt", excerpt);
    }
    if let Some(use_note) = &source.use_note {
        emit_prose(out, 2, "use", use_note);
    }
    out.push_str("    }\n");
}

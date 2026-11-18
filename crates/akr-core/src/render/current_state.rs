//! `CURRENT-STATE.md` — what the project believes right now.
//!
//! Normative specification: `docs/11-projections.md` §6.
//!
//! **Source query.** Live normative records (`term`, `requirement`, `policy`,
//! `constraint`) and live empirical records (`observation`, `evidence`, `assessment`).
//! `decision` is excluded — it has its own view.
//!
//! **Section order.** Terms, Constraints, Policies, Requirements, Observations,
//! Assessments, Evidence. Within each, by key.

use super::common::{is_archived, one_line, required_prose};
use super::{RenderContext, banner};
use crate::model::{ContentSlot, ContentValue, Kind, Record, Relation};

/// Renders `CURRENT-STATE.md`.
#[must_use]
pub fn render_current_state(cx: RenderContext<'_>) -> String {
    let mut blocks: Vec<String> = Vec::new();
    blocks.push(banner(cx.model).trim_end().to_owned());
    blocks.push(format!("# Current state — {}", cx.ledger().project.name));
    blocks.push(
        "What the project believes right now: live terms, constraints, policies and \
         requirements, and what has been found or assessed. Decisions have their own \
         view, `DECISION-HISTORY.md`."
            .to_owned(),
    );

    const SECTIONS: &[(&str, Kind)] = &[
        ("Terms", Kind::Term),
        ("Constraints", Kind::Constraint),
        ("Policies", Kind::Policy),
        ("Requirements", Kind::Requirement),
        ("Observations", Kind::Observation),
        ("Assessments", Kind::Assessment),
        ("Evidence", Kind::Evidence),
    ];
    for (heading, kind) in SECTIONS {
        let records = heads_of_kind(cx, *kind);
        // D-046: a retired kind has a section only in a ledger that still holds such
        // records. A project that never wrote a `term` should not be told it has none.
        if records.is_empty() && kind.is_retired() {
            continue;
        }
        blocks.push(format!("## {heading}"));
        if records.is_empty() {
            blocks.push("_(none)_".to_owned());
            continue;
        }
        for record in records {
            blocks.extend(record_blocks(cx, record));
        }
    }

    let mut out = blocks.join("\n\n");
    out.push('\n');
    out
}

fn record_blocks<'a>(cx: RenderContext<'a>, record: &'a Record) -> Vec<String> {
    let mut blocks = vec![format!("### {}", record.title), metadata_line(record)];
    if let Some(body) = required_prose(record) {
        blocks.push(body);
    }
    if !record.claims.is_empty() {
        blocks.push(claims_list(record));
    }
    if let Some(line) = relations_line(cx, record) {
        blocks.push(line);
    }
    if record.kind == Kind::Evidence {
        blocks.extend(verifies_block(cx, record));
    }
    if let Some(quote) = super::review_required::freshness_quote(cx, &record.id) {
        blocks.push(quote);
    }
    blocks
}

/// `` `state` · `@key/rev` [· scope `...`] [· topic `...`] [· marker] ``
fn metadata_line(record: &Record) -> String {
    let mut parts = vec![
        format!("`{}`", record.state.name()),
        format!("`@{}`", record.id),
    ];
    if !record.scope.is_empty() {
        parts.push(format!(
            "scope {}",
            record
                .scope
                .iter()
                .map(|term| format!("`{term}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if let Some(topic) = &record.topic {
        parts.push(format!("topic `{topic}`"));
    }
    parts.join(" · ")
}

/// Claims as a definition list: `` - `#anchor` — text ``.
fn claims_list(record: &Record) -> String {
    record
        .claims
        .iter()
        .map(|claim| format!("- `#{}` — {}", claim.anchor, one_line(&claim.text)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The relation slots, as a compact one-line list: `**relation** \`@id\`[, \`@id\`] ·
/// ...`. `exceptions` — a content slot, not a relation — joins the same line, first,
/// because a policy naming exceptions is unreadable without them.
fn relations_line(cx: RenderContext<'_>, record: &Record) -> Option<String> {
    let ledger = cx.ledger();
    let mut parts: Vec<String> = Vec::new();

    if let Some(ContentValue::Refs(references)) = record.get(ContentSlot::Exceptions) {
        let targets: Vec<String> = references
            .iter()
            .filter_map(|r| ledger.resolve(r).ok().flatten())
            .map(|t| format!("`@{}`", t.id))
            .collect();
        if !targets.is_empty() {
            parts.push(format!("**exceptions** {}", targets.join(", ")));
        }
    }

    for (relation, references) in &record.relations {
        if *relation == Relation::Contradicts {
            continue; // rendered on the freshness / build side, not per-record here
        }
        let targets: Vec<String> = references
            .iter()
            .filter_map(|r| ledger.resolve(r).ok().flatten())
            .map(|t| format!("`@{}`", t.id))
            .collect();
        if !targets.is_empty() {
            parts.push(format!("**{}** {}", relation.name(), targets.join(", ")));
        }
    }

    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// An evidence record's **Verifies** list: `verified_by` runs one way (D-016), so this
/// reverses it for the reader, who is usually holding the evidence and wondering what it
/// closed.
fn verifies_block(cx: RenderContext<'_>, evidence: &Record) -> Vec<String> {
    let ledger = cx.ledger();
    let mut lines: Vec<String> = Vec::new();
    for record in crate::graph::sorted_records(ledger) {
        let Some(acceptance) = &record.acceptance else {
            continue;
        };
        for check in &acceptance.checks {
            let cites = check.verified_by.iter().any(|r| {
                ledger
                    .resolve(r)
                    .ok()
                    .flatten()
                    .is_some_and(|target| target.id == evidence.id)
            });
            if cites {
                lines.push(format!(
                    "- `{}` `@{}` — check `{}`",
                    record.state.name(),
                    record.id,
                    check.id
                ));
            }
        }
    }
    if lines.is_empty() {
        return Vec::new();
    }
    vec!["**Verifies**".to_owned(), lines.join("\n")]
}

/// The head revision of every live record of one kind, excluding archived ones, sorted by
/// key. `decision` is never passed here — `CURRENT-STATE.md` excludes it (§6).
fn heads_of_kind<'a>(cx: RenderContext<'a>, kind: Kind) -> Vec<&'a Record> {
    let ledger = cx.ledger();
    let mut out: Vec<&Record> = cx
        .model
        .heads
        .values()
        .filter_map(|id| ledger.get(id))
        .filter(|record| record.kind == kind && record.is_live() && !is_archived(record))
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

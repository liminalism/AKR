//! `PAPERCUTS.md` — small frictions, newest first (D-027).
//!
//! **Source query.** The head revision of every `papercut` record, excluding archived
//! ones. Terminal papercuts (withdrawn, superseded, disproven) are excluded: a papercut
//! someone has explicitly retired has been dealt with.
//!
//! **Order.** `created_at` descending, then key descending — newest first, because the
//! reader of this file is asking "what has been hurting lately". Both are total: the
//! tooling always stamps `created_at`, and the key breaks ties deterministically.
//!
//! **Emission.** [`render_papercuts`] returns `None` when the ledger holds no papercut,
//! so a project that never logs one never grows the file, and the frozen worked example
//! stays byte-identical.

use super::common::is_archived;
use super::{RenderContext, banner};
use crate::model::{ContentSlot, ContentValue, Kind, Record};

/// Renders `PAPERCUTS.md`, or `None` when there is nothing to render.
#[must_use]
pub fn render_papercuts(cx: RenderContext<'_>) -> Option<String> {
    let ledger = cx.ledger();
    let mut papercuts: Vec<&Record> = cx
        .model
        .heads
        .values()
        .filter_map(|id| ledger.get(id))
        .filter(|record| record.kind == Kind::Papercut && record.is_live() && !is_archived(record))
        .collect();
    if papercuts.is_empty() {
        return None;
    }
    papercuts.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.id.cmp(&a.id))
    });

    let mut blocks: Vec<String> = Vec::new();
    blocks.push(banner(cx.model).trim_end().to_owned());
    blocks.push(format!("# Papercuts — {}", ledger.project.name));
    blocks.push(
        "Small frictions hit while working, logged in the moment (D-027). None of these \
         blocked anything; together they show where the project needs sanding down. \
         Newest first."
            .to_owned(),
    );

    // Split by subject, because these are two different reading tasks. "What has been
    // hurting in *this* project" is what the file is for; a friction with a tool, logged
    // here because this is where the agent happened to be, is somebody else's backlog and
    // would otherwise be read as ours (D-033).
    let (ours, elsewhere): (Vec<&Record>, Vec<&Record>) = papercuts
        .iter()
        .partition(|record| about_of(record).is_none());

    if !ours.is_empty() {
        blocks.push(items(&ours, false));
    }
    if !elsewhere.is_empty() {
        blocks.push("## Not about this project".to_owned());
        blocks.push(
            "Frictions with something else — a tool, a harness — hit while working here. \
             They are logged where they were hit; `akr papercut collate --about <subject>` \
             is how the project that owns the subject gathers them."
                .to_owned(),
        );
        blocks.push(items(&elsewhere, true));
    }

    let mut out = blocks.join("\n\n");
    out.push('\n');
    Some(out)
}

/// One bullet per papercut, with the subject shown when there is one.
fn items(records: &[&Record], show_subject: bool) -> String {
    records
        .iter()
        .map(|record| {
            let date = record
                .created_at
                .map_or_else(String::new, |d| format!("{d} "));
            let closed = match record.get(ContentSlot::ClosedAt) {
                Some(ContentValue::Date(date)) => format!("[closed {date}] "),
                _ => String::new(),
            };
            let author = record
                .author
                .as_deref()
                .map_or_else(String::new, |a| format!("[{a}] "));
            let subject = if show_subject {
                about_of(record).map_or_else(String::new, |about| format!("({about}) "))
            } else {
                String::new()
            };
            format!(
                "- {date}{closed}{author}{subject}{}  `@{}`",
                line_for(record),
                record.id
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The `about` subject of a papercut, if it declared one.
fn about_of(record: &Record) -> Option<String> {
    match record.get(ContentSlot::About) {
        Some(ContentValue::Text(text) | ContentValue::Prose(text)) => Some(text.clone()),
        _ => None,
    }
}

/// What one bullet says about a record.
///
/// A collation is summarised rather than flattened. Its statement is every absorbed
/// papercut in full — which is right for the record, and wrong for a bullet list, where
/// it renders as one paragraph a screen wide that nobody reads. The view says how many
/// and from where; the record says what they were.
fn line_for(record: &Record) -> String {
    match record.get(ContentSlot::Collated) {
        Some(ContentValue::Strings(keys)) if !keys.is_empty() => {
            let mut projects: Vec<&str> = keys
                .iter()
                .filter_map(|key| key.split('.').next())
                .collect();
            projects.sort_unstable();
            projects.dedup();
            format!(
                "{} — {} papercut{} collated from {} project{}; open the record for the \
                 statements",
                record.title,
                keys.len(),
                if keys.len() == 1 { "" } else { "s" },
                projects.len(),
                if projects.len() == 1 { "" } else { "s" },
            )
        }
        _ => statement_line(record),
    }
}

/// The statement as one line: prose newlines become spaces, because the bullet list is
/// the view's whole layout and a multi-line entry would break it.
fn statement_line(record: &Record) -> String {
    match record.get(ContentSlot::Statement) {
        Some(ContentValue::Prose(text) | ContentValue::Text(text)) => text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
        _ => record.title.clone(),
    }
}

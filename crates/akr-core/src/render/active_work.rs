//! `ACTIVE-WORK.md` — what is being worked on, and what is stuck.
//!
//! Normative specification: `docs/11-projections.md` §7.
//!
//! **Source query.** Live `work` records — `proposed`, `ready`, `active`, `blocked`.
//!
//! **Section order.** Grouped by `part_of` parent, parents in `ROADMAP.md` order,
//! unparented work last under "Unparented". Within a group: by state in the order
//! `active`, `blocked`, `ready`, `proposed`, then by key.

use super::common::{is_archived, link, note_block, prose};
use super::roadmap::{parent_order, sorted_work};
use super::{RenderContext, banner};
use crate::model::{ContentSlot, Kind, Ledger, Record, Relation, RevisionId};
use crate::resolve::Verdict;
use std::collections::BTreeMap;

/// Renders `ACTIVE-WORK.md`.
#[must_use]
pub fn render_active_work(cx: RenderContext<'_>) -> String {
    let ledger = cx.ledger();
    let mut blocks: Vec<String> = Vec::new();
    blocks.push(banner(cx.model).trim_end().to_owned());
    blocks.push(format!("# Active work — {}", ledger.project.name));
    blocks.push(
        "Live work, grouped by parent in `ROADMAP.md` order. Blocked work names its \
         blocker inline."
            .to_owned(),
    );

    let live = live_work(ledger, cx);
    let mut by_parent: BTreeMap<RevisionId, Vec<&Record>> = BTreeMap::new();
    let mut unparented: Vec<&Record> = Vec::new();
    for item in &live {
        match direct_parent(cx, item) {
            Some(parent) => by_parent.entry(parent).or_default().push(item),
            None => unparented.push(*item),
        }
    }

    // Group headings in ROADMAP.md order first (milestones, then tracks); any parent
    // that is itself a `work` record — nested work, which ROADMAP.md does not list —
    // follows, by key; unparented work is always last.
    let mut ordered_parents: Vec<RevisionId> = parent_order(cx.model)
        .into_iter()
        .filter(|id| by_parent.contains_key(id))
        .collect();
    let mut other_parents: Vec<RevisionId> = by_parent
        .keys()
        .filter(|id| !ordered_parents.contains(id))
        .cloned()
        .collect();
    other_parents.sort();
    ordered_parents.extend(other_parents);

    for parent_id in &ordered_parents {
        let Some(parent) = ledger.get(parent_id) else {
            continue;
        };
        let items = sorted_work(by_parent.remove(parent_id).unwrap_or_default());
        blocks.push(format!("## {} `@{}`", link(parent), parent.id));
        for item in &items {
            blocks.extend(work_blocks(cx, item));
        }
    }

    blocks.push("## Unparented".to_owned());
    if unparented.is_empty() {
        blocks.push("_(none)_".to_owned());
    } else {
        for item in &sorted_work(unparented) {
            blocks.extend(work_blocks(cx, item));
        }
    }

    let mut out = blocks.join("\n\n");
    out.push('\n');
    out
}

fn work_blocks<'a>(cx: RenderContext<'a>, record: &'a Record) -> Vec<String> {
    let mut blocks = vec![format!("### {}", record.title), metadata_line(cx, record)];
    if let Some(intent) = prose(record, ContentSlot::Intent) {
        blocks.push(intent);
    }
    if let Some(note) = note_block(record) {
        blocks.push(note);
    }
    if let Some(line) = plan_of_record_line(cx, record) {
        blocks.push(line);
    }
    blocks.extend(disposition_blocks(record, cx.ledger()));
    if let Some(block) = blocked_by_block(cx, record) {
        blocks.push(block);
    }
    blocks.extend(acceptance_blocks(cx, record));
    if let Some(quote) = super::review_required::freshness_quote(cx, &record.id) {
        blocks.push(quote);
    }
    blocks
}

/// `` `state` · `@key/rev` [· part of `@parent`] [· marker] ``
fn metadata_line(cx: RenderContext<'_>, record: &Record) -> String {
    let mut parts = vec![
        format!("`{}`", record.state.name()),
        format!("`@{}`", record.id),
    ];
    if let Some(parent) = record
        .targets(Relation::PartOf)
        .first()
        .and_then(|r| cx.ledger().resolve(r).ok().flatten())
    {
        parts.push(format!("part of `@{}`", parent.id));
    }
    if let Some(marker) = cx.freshness.marker(&record.id) {
        parts.push(marker.to_owned());
    }
    parts.join(" · ")
}

/// `**Plan of record for** <link> \`@id\`` when this work item designates one.
fn plan_of_record_line(cx: RenderContext<'_>, record: &Record) -> Option<String> {
    let target = record
        .targets(Relation::PlanOfRecord)
        .first()
        .and_then(|r| cx.ledger().resolve(r).ok().flatten())?;
    Some(format!(
        "**Plan of record for** {} `@{}`",
        link(target),
        target.id
    ))
}

/// A plan's own `disposition` blocks, in full — the record of what happened to the
/// previous plan's unfinished children (D-017), rendered nowhere else a casual reader
/// will see it.
fn disposition_blocks(record: &Record, ledger: &Ledger) -> Vec<String> {
    if record.dispositions.is_empty() {
        return Vec::new();
    }
    let mut lines = vec!["**Dispositions**".to_owned()];
    let mut items = Vec::new();
    for disposition in &record.dispositions {
        let Some(target) = ledger.resolve(&disposition.target).ok().flatten() else {
            continue;
        };
        let mut line = format!("- `@{}` — `{}`", target.id, disposition.outcome.name());
        if let Some(into) = &disposition.into
            && let Some(into_target) = ledger.resolve(into).ok().flatten()
        {
            line.push_str(&format!(" into `@{}`", into_target.id));
        }
        if let Some(note) = &disposition.note {
            line.push_str(&format!(" — {}", super::common::one_line(note)));
        }
        items.push(line);
    }
    if items.is_empty() {
        return Vec::new();
    }
    lines.push(items.join("\n"));
    lines
}

/// The live `blocks` edges holding this record, each naming the blocker: "blocked"
/// without "by what" is the least useful status in software.
fn blocked_by_block(cx: RenderContext<'_>, record: &Record) -> Option<String> {
    if record.state != crate::model::State::Blocked {
        return None;
    }
    let ledger = cx.ledger();
    let mut lines: Vec<String> = Vec::new();
    for other in crate::graph::sorted_records(ledger) {
        if !other.is_live() {
            continue;
        }
        let blocks_this = other.targets(Relation::Blocks).iter().any(|r| {
            ledger
                .resolve(r)
                .ok()
                .flatten()
                .is_some_and(|t| t.id == record.id)
        });
        if blocks_this {
            lines.push(format!(
                "- `{}` {} `@{}`",
                other.state.name(),
                link(other),
                other.id
            ));
        }
    }
    if lines.is_empty() {
        return None;
    }
    Some(format!("**Blocked by**\n{}", lines.join("\n")))
}

/// The acceptance heading and table, mirroring `ROADMAP.md`'s rendering, for a work
/// record that carries checks.
fn acceptance_blocks(cx: RenderContext<'_>, record: &Record) -> Vec<String> {
    let verdicts = cx.model.checks_of(&record.id);
    if verdicts.is_empty() {
        return Vec::new();
    }
    let satisfied = verdicts.iter().filter(|v| v.verdict.is_satisfied()).count();
    let mut table = String::from("| Check | Method | Verdict |\n| --- | --- | --- |");
    for entry in &verdicts {
        let method = record
            .acceptance
            .as_ref()
            .and_then(|a| a.checks.iter().find(|c| c.id == entry.check))
            .map_or("manual", |c| c.method.name());
        table.push_str(&format!(
            "\n| `{}` | {method} | {} |",
            entry.check,
            verdict_text(&entry.verdict)
        ));
    }
    vec![
        format!(
            "**Acceptance** — {satisfied} of {} satisfied",
            verdicts.len()
        ),
        table,
    ]
}

fn verdict_text(verdict: &Verdict) -> String {
    match verdict {
        Verdict::Satisfied { by, .. } => format!("**satisfied** by `@{by}`"),
        Verdict::NoEvidence => "not satisfied — no evidence".to_owned(),
        Verdict::Unresolved => "not satisfied — the cited evidence does not resolve".to_owned(),
        Verdict::Failing { by, result } => format!(
            "not satisfied — `@{by}` reports `{}`",
            result.map_or("no result", crate::model::EvidenceResult::name)
        ),
        Verdict::TooOld { by, .. } => format!("not satisfied — `@{by}` predates the last change"),
    }
}

/// The direct `part_of` parent, resolved, ignoring the target's kind: `ROADMAP.md`
/// order covers milestones and tracks; a `work` parent (nested work) is grouped too, by
/// key, since the source query does not exclude it (`docs/11-projections.md` §7).
fn direct_parent(cx: RenderContext<'_>, record: &Record) -> Option<RevisionId> {
    record
        .targets(Relation::PartOf)
        .first()
        .and_then(|r| cx.ledger().resolve(r).ok().flatten())
        .map(|t| t.id.clone())
}

fn live_work<'a>(ledger: &'a Ledger, cx: RenderContext<'a>) -> Vec<&'a Record> {
    let undispositioned = undispositioned_import_drafts(cx);
    ledger
        .records()
        .iter()
        .filter(|r| r.kind == Kind::Work && r.is_live() && !is_archived(r))
        .filter(|r| !undispositioned.contains(&r.id.key.to_string()))
        .collect()
}

/// The keys of import drafts nobody has dispositioned yet.
///
/// `akr import`'s `classify` has a catch-all of `work`, so every heading of a discursive
/// document becomes a live `proposed` work record. One 2026-08 bulk import turned five
/// reports into 125 of them and a 1291-line `ACTIVE-WORK.md`, where "Executive verdict",
/// "Wall times (median of 5 runs)" and "Peak RSS" each read as something the project
/// intends to do.
///
/// The signal is one the ledger already holds and no other predicate can substitute for.
/// D-022 has `akr import` write a tracking record whose acceptance enumerates the claims,
/// one check each, reading *"<title>" is dispositioned: promoted as `<key>` or declined
/// with evidence*. A draft is undispositioned **exactly while its tracking check is
/// unsatisfied**, computed by the same acceptance machinery every other view uses, and it
/// becomes false the moment somebody actually dispositions the claim.
///
/// The tempting predicate — hide a `proposed` work record with only a `legacy` source at
/// revision 1 — is WRONG, and expensively so. Per D-015 only a *sealed* record needs a new
/// revision, so **a `proposed` record is edited in place**: a draft that has been read,
/// rewritten and adopted as the project's live plan is still revision 1, still `proposed`,
/// and still cites only the document it came from. That predicate was tested against
/// `lege-ecosystem.work.jp2lam-jpeg-2000-decoder-optimization-plan` — the single live
/// account of that whole effort — and would have hidden it while leaving all 125 phantoms
/// visible. Acceptance blocks and relations do not rescue it either: that record has
/// neither, and genuine drafts often acquire a `part_of` edge from the importer itself.
/// The general lesson is worth keeping: a state machine that edits in place cannot be
/// asked "has anyone touched this?", and a predicate that assumes it can will be
/// confidently wrong.
///
/// Fails OPEN. A statement that does not parse, or a tracker that cannot be identified,
/// hides nothing — showing a draft is noise, hiding an adopted plan is loss, and those
/// are not symmetric.
fn undispositioned_import_drafts(cx: RenderContext<'_>) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    for record in cx.ledger().records() {
        // A tracking record is D-022's shape: live work, a non-empty acceptance block, and
        // a legacy source naming the document under migration. `import::audit` identifies
        // it the same way.
        if record.kind != Kind::Work || !record.is_live() {
            continue;
        }
        if !record
            .acceptance
            .as_ref()
            .is_some_and(|a| !a.checks.is_empty())
        {
            continue;
        }
        if !record
            .sources
            .iter()
            .any(|s| s.kind == crate::model::SourceKind::Legacy)
        {
            continue;
        }
        for entry in cx.model.checks_of(&record.id) {
            if entry.verdict.is_satisfied() {
                continue;
            }
            let Some(check) = record
                .acceptance
                .as_ref()
                .and_then(|a| a.checks.iter().find(|c| c.id == entry.check))
            else {
                continue;
            };
            if let Some(key) = promoted_key(&check.statement) {
                out.insert(key);
            }
        }
    }
    out
}

/// The key a tracking check names, from *promoted as `<key>` or declined*.
///
/// The statement is the only link the ledger holds between a tracking check and the draft
/// it tracks, so it is parsed rather than inferred. Anything that does not match this
/// exact shape yields `None` and hides nothing.
fn promoted_key(statement: &str) -> Option<String> {
    let rest = statement.split_once("promoted as ")?.1;
    let key = rest.split_once(" or declined")?.0.trim();
    (!key.is_empty() && key.contains('.')).then(|| key.to_owned())
}

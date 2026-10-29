//! `ROADMAP.md` — where the project is going.
//!
//! Normative specification: `docs/11-projections.md` §5, over the universal rendering
//! rules of §3 and the banner of §4.
//!
//! **Source query.** The head revision of every `milestone` record, then of every `track`
//! record, excluding archived ones. Completed milestones are included: a roadmap that
//! hides finished work cannot show what was finished.
//!
//! **Section order.** (1) Milestones, in `after` topological order with key as tiebreak.
//! (2) Tracks, by key. (3) Two summary tables.
//!
//! # Output shape
//!
//! The file is a sequence of *blocks* joined by exactly one blank line, with a single
//! trailing newline. That is the whole layout rule, and expressing it as a join rather
//! than as scattered `push('\n')` calls is what keeps the output stable enough to pin
//! with a byte-for-byte snapshot.

use super::common::{is_archived, link, note_block, prose};
use super::{RenderContext, banner};
use crate::model::{
    ContentSlot, ContentValue, Disposition, Kind, Ledger, Record, Reference, Relation, RevisionId,
    State,
};
use crate::resolve::{ResolvedModel, Verdict};
use std::collections::BTreeSet;

/// Renders `ROADMAP.md`.
#[must_use]
pub fn render_roadmap(cx: RenderContext<'_>) -> String {
    let mut blocks: Vec<String> = Vec::new();
    blocks.push(banner(cx.model).trim_end().to_owned());
    blocks.push(format!("# Roadmap — {}", cx.ledger().project.name));
    blocks.push(
        "Live milestones in `after` order, then standing tracks. Acceptance verdicts are computed\n\
         from evidence under the descendant-commit rule (D-016)."
            .to_owned(),
    );

    let milestones = milestone_order(cx.model);
    let tracks = heads_of_kind(cx.model, Kind::Track);

    blocks.push("## Milestones".to_owned());
    for milestone in &milestones {
        blocks.extend(milestone_blocks(cx, milestone));
    }

    blocks.push("## Tracks".to_owned());
    blocks.push("Standing work that no milestone contains.".to_owned());
    for track in &tracks {
        blocks.extend(track_blocks(cx, track));
    }

    blocks.push("## Summary".to_owned());
    blocks.push(milestone_summary(cx, &milestones));
    blocks.push(track_summary(cx, &tracks));

    let mut out = blocks.join("\n\n");
    out.push('\n');
    out
}

// -------------------------------------------------------------------------------------
// Milestones
// -------------------------------------------------------------------------------------

fn milestone_blocks<'a>(cx: RenderContext<'a>, record: &'a Record) -> Vec<String> {
    let mut blocks = vec![format!("### {}", record.title), metadata_line(cx, record)];
    if let Some(intent) = prose(record, ContentSlot::Intent) {
        blocks.push(intent);
    }
    if let Some(note) = note_block(record) {
        blocks.push(note);
    }

    if let Some(plan) = plan_of_record(cx.model, record) {
        blocks.push(format!("**Plan of record:** {} `@{}`", link(plan), plan.id));
    }

    blocks.extend(acceptance_blocks(cx, record));

    if let Some(line) = depends_on_line(cx, record) {
        blocks.push(line);
    }

    let children = children_of(cx, &record.id);
    blocks.extend(work_item_blocks(cx, &children, "**Work items**", None));

    if let Some(plan) = plan_of_record(cx.model, record) {
        let listed: BTreeSet<&RevisionId> = children.iter().map(|c| &c.id).collect();
        let under_plan: Vec<&Record> = live_work(cx.ledger())
            .into_iter()
            .filter(|w| !listed.contains(&w.id))
            .filter(|w| parent_key(cx.model, w).is_some_and(|k| k == plan.id.key))
            .collect();
        let under_plan = sorted_work(under_plan);
        if !under_plan.is_empty() {
            blocks.extend(work_item_blocks(
                cx,
                &under_plan,
                "**Under the plan of record**",
                Some(plan),
            ));
        }
    }

    blocks
}

/// Milestones (in `after` topological order), then tracks (by key) — `ROADMAP.md`'s own
/// section order, reused by `active_work` to group work items in the same order their
/// parents appear there (`docs/11-projections.md` §7).
#[must_use]
pub(super) fn parent_order(model: &ResolvedModel<'_>) -> Vec<RevisionId> {
    milestone_order(model)
        .into_iter()
        .chain(heads_of_kind(model, Kind::Track))
        .map(|r| r.id.clone())
        .collect()
}

/// Milestones in `after` topological order, with the record key as tiebreak.
///
/// `after` points backwards — `M2 after [M1]` — so the ordering graph runs from
/// prerequisite to dependent and a topological sort puts M1 first. Kahn's algorithm over
/// a sorted ready set gives the same order every run (`docs/06-compiler-pipeline.md` §11).
/// A cycle is V-016's problem (`AKR-R013`); rendering falls back to key order rather than
/// refusing to produce a roadmap while that diagnostic is being read.
fn milestone_order<'a>(model: &'a ResolvedModel<'a>) -> Vec<&'a Record> {
    let milestones = heads_of_kind(model, Kind::Milestone);
    let ids: BTreeSet<&RevisionId> = milestones.iter().map(|m| &m.id).collect();

    let mut graph = crate::graph::DiGraph::new();
    for milestone in &milestones {
        graph.add_node(milestone.id.clone());
        for reference in milestone.targets(Relation::After) {
            if let Some(target) = model.ledger().resolve(reference).ok().flatten()
                && ids.contains(&target.id)
            {
                graph.add_edge(target.id.clone(), milestone.id.clone());
            }
        }
    }

    let Some(order) = graph.topological_order() else {
        return milestones;
    };
    order
        .into_iter()
        .filter_map(|id| milestones.iter().copied().find(|m| m.id == id))
        .collect()
}

// -------------------------------------------------------------------------------------
// Tracks
// -------------------------------------------------------------------------------------

fn track_blocks<'a>(cx: RenderContext<'a>, record: &'a Record) -> Vec<String> {
    let mut blocks = vec![format!("### {}", record.title), metadata_line(cx, record)];
    if let Some(intent) = prose(record, ContentSlot::Intent) {
        blocks.push(intent);
    }
    if let Some(note) = note_block(record) {
        blocks.push(note);
    }
    if let Some(line) = depends_on_line(cx, record) {
        blocks.push(line);
    }
    let children = children_of(cx, &record.id);
    blocks.extend(work_item_blocks(cx, &children, "**Work items**", None));

    // Work carried into this track by a disposition (D-017). It is not `part_of` the
    // track, so nothing above would show it, and it is exactly the work a replan is most
    // likely to lose.
    let carried = carried_into(cx.ledger(), &record.id);
    if !carried.is_empty() {
        blocks.push(format!(
            "Carried into this track by disposition: {}.",
            carried
                .iter()
                .map(|id| format!("`@{id}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    blocks
}

/// Revisions dispositioned `into` this record, sorted by key.
fn carried_into(ledger: &Ledger, target: &RevisionId) -> Vec<RevisionId> {
    let mut out = BTreeSet::new();
    for record in ledger.records() {
        for disposition in &record.dispositions {
            let Some(into) = &disposition.into else {
                continue;
            };
            let resolves_here = ledger
                .resolve(into)
                .ok()
                .flatten()
                .is_some_and(|r| &r.id == target);
            if !resolves_here {
                continue;
            }
            if let Some(child) = ledger.resolve(&disposition.target).ok().flatten() {
                out.insert(child.id.clone());
            }
        }
    }
    out.into_iter().collect()
}

// -------------------------------------------------------------------------------------
// Shared pieces
// -------------------------------------------------------------------------------------

/// The metadata line beneath a heading: state, key, and the facts that qualify them.
///
/// Freshness goes here and never in the heading (§3): a heading anchor that changed when
/// a record went stale would break every link into it from every other view, on a build
/// that changed no record at all.
fn metadata_line(cx: RenderContext<'_>, record: &Record) -> String {
    let mut parts = vec![
        format!("`{}`", record.state.name()),
        format!("`@{}`", record.id),
    ];
    if !record.scope.is_empty() && record.kind == Kind::Track {
        // Each term in its own backticks, as `docs/11-projections.md` §6 renders them:
        // a scope list is a list of literals, and one span around the whole list would
        // read as a single value.
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
    if let Some(ContentValue::Date(target)) = record.get(ContentSlot::Target) {
        parts.push(format!("target {target}"));
    }
    if let Some(ContentValue::Text(cadence)) = record.get(ContentSlot::Cadence) {
        parts.push(format!("cadence {cadence:?}"));
    }
    let after: Vec<String> = record
        .targets(Relation::After)
        .iter()
        .filter_map(|r| cx.ledger().resolve(r).ok().flatten())
        .map(|t| format!("`@{}`", t.id))
        .collect();
    if !after.is_empty() {
        parts.push(format!("after {}", after.join(", ")));
    }
    if let Some(marker) = cx.freshness.marker(&record.id) {
        parts.push(marker.to_owned());
    }
    parts.join(" · ")
}

/// The acceptance heading and table, or nothing when the record has no acceptance block.
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
            render_verdict(&entry.verdict)
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

/// The four verdict renderings of `docs/09-context-assembly.md` §4 step 7.
///
/// The "evidence too old" row is the one that earns its keep: an agent that cannot see
/// *why* a check is unsatisfied will assume the check is wrong.
fn render_verdict(verdict: &Verdict) -> String {
    match verdict {
        Verdict::Satisfied {
            by,
            observed_at,
            last_change: Some(last_change),
        } => format!(
            "**satisfied** by `@{by}` (pass at `{}`, descends from `{}`)",
            short(observed_at.as_str()),
            short(last_change.as_str())
        ),
        Verdict::Satisfied {
            by, observed_at, ..
        } => format!(
            "**satisfied** by `@{by}` (pass at `{}`)",
            short(observed_at.as_str())
        ),
        Verdict::NoEvidence => "not satisfied — no evidence".to_owned(),
        Verdict::Unresolved => "not satisfied — the cited evidence does not resolve".to_owned(),
        Verdict::Failing { by, result } => format!(
            "not satisfied — `@{by}` reports `{}`",
            result.map_or("no result", crate::model::EvidenceResult::name)
        ),
        Verdict::TooOld {
            by,
            observed_at,
            last_change,
        } => format!(
            "not satisfied — `@{by}` observed at `{}`, which does not descend from `{}`",
            short(observed_at.as_str()),
            short(last_change.as_str())
        ),
        Verdict::TooOldUncommitted { by, observed_at } => format!(
            "not satisfied — `@{by}` observed at `{}`, which does not descend from the uncommitted change",
            short(observed_at.as_str())
        ),
    }
}

/// A commit abbreviated for prose. Never for identity: references are always full 40 hex
/// (D-008); this is a label beside one.
fn short(commit: &str) -> &str {
    &commit[..8.min(commit.len())]
}

/// `**Depends on** <link> \`@id\`[, ...]`, or nothing when the record depends on nothing.
fn depends_on_line(cx: RenderContext<'_>, record: &Record) -> Option<String> {
    let links: Vec<String> = record
        .targets(Relation::DependsOn)
        .iter()
        .filter_map(|r| cx.ledger().resolve(r).ok().flatten())
        .map(|target| format!("{} `@{}`", link(target), target.id))
        .collect();
    (!links.is_empty()).then(|| format!("**Depends on** {}", links.join(", ")))
}

/// The work-items heading and list, as one or two blocks.
///
/// An empty list prints `— _(none)_` on the heading line (§3): a missing heading would be
/// ambiguous between "nothing here" and "not generated".
fn work_item_blocks(
    cx: RenderContext<'_>,
    items: &[&Record],
    heading: &str,
    plan: Option<&Record>,
) -> Vec<String> {
    if items.is_empty() {
        return vec![format!("{heading} — _(none)_")];
    }
    let list = items
        .iter()
        .map(|item| work_item_line(cx, item, plan))
        .collect::<Vec<_>>()
        .join("\n");
    vec![heading.to_owned(), list]
}

fn work_item_line(cx: RenderContext<'_>, item: &Record, plan: Option<&Record>) -> String {
    let mut line = format!("- `{}` {} `@{}`", item.state.name(), link(item), item.id);
    if let Some(plan) = plan {
        let parent = item
            .targets(Relation::PartOf)
            .first()
            .and_then(|r| cx.ledger().resolve(r).ok().flatten())
            .map(|p| p.id.clone());
        if let Some(parent) = parent {
            line.push_str(&format!(" — part of `@{parent}`"));
        }
        if let Some(disposition) = disposition_for(plan, &item.id) {
            line.push_str(&format!(", dispositioned `{}`", disposition.outcome.name()));
            if let Some(into) = &disposition.into
                && let Some(target) = cx.ledger().resolve(into).ok().flatten()
            {
                line.push_str(&format!(" into `@{}`", target.id));
            }
        }
    }
    if let Some(marker) = cx.freshness.marker(&item.id) {
        // A list entry under a plan already uses an em dash for the disposition clause, so
        // the marker joins with a comma there and with an em dash where it is the only
        // qualifier.
        let separator = if plan.is_some() { ", " } else { " — " };
        line.push_str(separator);
        line.push_str(marker);
    }
    line
}

pub(super) fn disposition_for<'a>(plan: &'a Record, child: &RevisionId) -> Option<&'a Disposition> {
    plan.dispositions.iter().find(|d| d.target.key == child.key)
}

// -------------------------------------------------------------------------------------
// Summary tables
// -------------------------------------------------------------------------------------

fn milestone_summary(cx: RenderContext<'_>, milestones: &[&Record]) -> String {
    let mut table =
        String::from("| Milestone | State | Target | Acceptance |\n| --- | --- | --- | --- |");
    for milestone in milestones {
        let verdicts = cx.model.checks_of(&milestone.id);
        let satisfied = verdicts.iter().filter(|v| v.verdict.is_satisfied()).count();
        let target = match milestone.get(ContentSlot::Target) {
            Some(ContentValue::Date(date)) => date.to_string(),
            _ => "—".to_owned(),
        };
        table.push_str(&format!(
            "\n| {} | `{}` | {target} | {satisfied} / {} |",
            milestone.title,
            milestone.state.name(),
            verdicts.len()
        ));
    }
    table
}

fn track_summary(cx: RenderContext<'_>, tracks: &[&Record]) -> String {
    let _ = cx;
    let mut table = String::from("| Track | State | Cadence |\n| --- | --- | --- |");
    for track in tracks {
        let cadence = match track.get(ContentSlot::Cadence) {
            Some(ContentValue::Text(text)) => text.clone(),
            _ => "—".to_owned(),
        };
        table.push_str(&format!(
            "\n| {} | `{}` | {cadence} |",
            track.title,
            track.state.name()
        ));
    }
    table
}

// -------------------------------------------------------------------------------------
// Selection helpers
// -------------------------------------------------------------------------------------

/// The head revision of every record of one kind, excluding archived ones (D-018), sorted
/// by key.
fn heads_of_kind<'a>(model: &'a ResolvedModel<'a>, kind: Kind) -> Vec<&'a Record> {
    let ledger = model.ledger();
    let mut out: Vec<&Record> = model
        .heads
        .values()
        .filter_map(|id| ledger.get(id))
        .filter(|record| record.kind == kind && !is_archived(record))
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// The live plan of record for a milestone or track: the live `work` record whose
/// `plan_of_record` resolves to it. At most one exists (V-018).
fn plan_of_record<'a>(model: &'a ResolvedModel<'a>, target: &Record) -> Option<&'a Record> {
    let ledger = model.ledger();
    let mut candidates: Vec<&Record> = ledger
        .records()
        .iter()
        .filter(|r| r.is_live())
        .filter(|r| {
            r.targets(Relation::PlanOfRecord)
                .iter()
                .filter_map(|reference| ledger.resolve(reference).ok().flatten())
                .any(|t| t.id == target.id)
        })
        .collect();
    candidates.sort_by(|a, b| a.id.cmp(&b.id));
    candidates.first().copied()
}

/// Live work records whose `part_of` resolves to this revision, in render order.
fn children_of<'a>(cx: RenderContext<'a>, parent: &RevisionId) -> Vec<&'a Record> {
    let items: Vec<&Record> = live_work(cx.ledger())
        .into_iter()
        .filter(|w| {
            w.targets(Relation::PartOf)
                .iter()
                .filter_map(|r| cx.ledger().resolve(r).ok().flatten())
                .any(|t| &t.id == parent)
        })
        .collect();
    sorted_work(items)
}

pub(super) fn live_work(ledger: &Ledger) -> Vec<&Record> {
    ledger
        .records()
        .iter()
        .filter(|r| r.kind == Kind::Work && r.is_live() && !is_archived(r))
        .collect()
}

/// The key a work record is `part_of`, resolved.
pub(super) fn parent_key(
    model: &ResolvedModel<'_>,
    record: &Record,
) -> Option<crate::model::LogicalKey> {
    record
        .targets(Relation::PartOf)
        .first()
        .and_then(|r: &Reference| model.ledger().resolve(r).ok().flatten())
        .map(|t| t.id.key.clone())
}

/// Work in lifecycle order — `active`, `blocked`, `ready`, `proposed` — then by key
/// (`docs/09-context-assembly.md` §4 step 3).
pub(super) fn sorted_work(mut items: Vec<&Record>) -> Vec<&Record> {
    items.sort_by(|a, b| (state_rank(a.state), &a.id).cmp(&(state_rank(b.state), &b.id)));
    items
}

pub(super) const fn state_rank(state: State) -> u8 {
    match state {
        State::Active => 0,
        State::Blocked => 1,
        State::Ready => 2,
        State::Proposed => 3,
        _ => 4,
    }
}

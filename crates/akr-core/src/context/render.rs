//! Rendering a bundle: the text form of `docs/09-context-assembly.md` §7, and the JSON
//! form of `docs/08-mcp.md` §3.
//!
//! Every section header is printed even when the section is empty, and an empty section
//! prints `(none)`. A missing section header would be ambiguous between "nothing here"
//! and "not computed"; the whole point of a deterministic bundle is that the reader can
//! tell.

use super::{Bundle, Entry, Section, body_of, observed_commit};
use crate::model::{ContentSlot, ContentValue, Record, Relation, RevisionId, ScopeTerm};
use crate::render::Freshness;
use crate::resolve::{ResolvedModel, Verdict};

/// The width of the section rules.
const RULE: usize = 77;

/// Renders the text form.
#[must_use]
pub fn render_text(bundle: &Bundle, model: &ResolvedModel<'_>, freshness: &Freshness) -> String {
    let ledger = model.ledger();
    let mut out = String::new();

    let title = ledger
        .get(&bundle.goal)
        .map_or_else(String::new, |r| r.title.clone());
    out.push_str("AKR CONTEXT BUNDLE\n");
    out.push_str(&format!("goal        {} — {title}\n", bundle.goal));
    out.push_str(&format!(
        "commit      {}\n",
        bundle.commit.as_deref().unwrap_or("(none)")
    ));
    if bundle.paths.is_empty() {
        out.push_str("paths       (none)\n");
    } else {
        out.push_str(&format!(
            "paths       {}\n",
            bundle
                .paths
                .iter()
                .map(|g| g.as_str().to_owned())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    // `tool_version` is the lock's `build.tool` string, which already reads `akr 0.1.0`
    // (`spec/akr-lock.md` §2). §7's `generated akr <version>` wants one `akr`, not two.
    let tool = bundle
        .tool_version
        .strip_prefix("akr ")
        .unwrap_or(&bundle.tool_version);
    out.push_str(&format!(
        "generated   akr {tool}, source-graph\n            {}\n",
        bundle.source_graph
    ));

    for &section in Section::ALL {
        out.push('\n');
        out.push_str(&heading(section));
        out.push('\n');
        let body = match section {
            Section::Goal => render_goal(bundle, model, freshness),
            Section::Milestone => render_milestone(bundle, model),
            Section::Acceptance => render_acceptance(bundle, model),
            Section::Contradictions => render_contradictions(bundle, model),
            Section::Staleness => render_staleness(bundle, freshness),
            _ => render_records(bundle, model, freshness, section),
        };
        if body.trim().is_empty() {
            out.push_str("(none)\n");
        } else {
            out.push_str(&body);
        }
    }

    out.push('\n');
    out.push_str(&format!("── EXCLUDED {}\n", "─".repeat(RULE - 12)));
    out.push_str(&format!(
        "  superseded revisions  {:>3}\n",
        bundle.excluded.superseded.len()
    ));
    out.push_str(&format!(
        "  archived              {:>3}\n",
        bundle.excluded.archived.len()
    ));
    out.push_str(&format!(
        "  terminal              {:>3}\n",
        bundle.excluded.terminal.len()
    ));
    out.push_str(&format!(
        "  out of scope          {:>3}\n",
        bundle.excluded.out_of_scope.len()
    ));

    out.push('\n');
    out.push_str(&format!(
        "{} records, {} sections, ~{} tokens. {}\n",
        bundle.len(),
        Section::ALL.len(),
        bundle.estimated_tokens,
        if bundle.truncated.is_empty() {
            "No prose truncated.".to_owned()
        } else {
            format!("Prose truncated in {} records.", bundle.truncated.len())
        }
    ));
    let withheld = bundle.withheld_count();
    if withheld > 0 || !bundle.truncated.is_empty() {
        out.push_str(&wrap(
            &format!(
                "{withheld} entries withheld whole to fit the budget, each named in its \
                 section. `akr get <key>` (knowledge.get) reads any withheld or truncated \
                 record in full; a larger --budget or a narrower --paths keeps them in the \
                 bundle."
            ),
            "",
            0,
        ));
    }
    out
}

/// What no budget reduces, rendered: the goal, the milestone, the plan of record,
/// acceptance and contradictions. The budget's floor is measured on this.
pub(super) fn mandatory_text(
    bundle: &Bundle,
    model: &ResolvedModel<'_>,
    freshness: &Freshness,
) -> String {
    let mut out = render_goal(bundle, model, freshness);
    out.push_str(&render_milestone(bundle, model));
    out.push_str(&render_records(
        bundle,
        model,
        freshness,
        Section::PlanOfRecord,
    ));
    out.push_str(&render_acceptance(bundle, model));
    out.push_str(&render_contradictions(bundle, model));
    out
}

/// The records a section's budget withheld, by key, and how to read them.
///
/// Consecutive keys that share everything but their last segment fold into one brace
/// group, `@ns.kind.{a/1, b/2}`. A section is sorted by kind, so the fold is what keeps a
/// list of a hundred withheld names from costing a large share of what withholding the
/// records saved.
fn withheld_lines(bundle: &Bundle, section: Section) -> String {
    let Some(ids) = bundle.withheld.get(&section).filter(|ids| !ids.is_empty()) else {
        return String::new();
    };
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for id in ids {
        let key = id.key.to_string();
        let (prefix, last) = key.rsplit_once('.').unwrap_or(("", key.as_str()));
        let item = format!("{last}/{}", id.revision);
        match groups.last_mut() {
            Some((current, items)) if current == prefix => items.push(item),
            _ => groups.push((prefix.to_owned(), vec![item])),
        }
    }
    let mut out = String::from("\n");
    out.push_str(&wrap(
        &format!(
            "WITHHELD ({}) to fit the budget, from the low-ranked end of this section; \
             `akr get <key>` reads one in full:",
            ids.len()
        ),
        "  ",
        2,
    ));
    for (prefix, items) in groups {
        if prefix.is_empty() {
            for item in items {
                out.push_str(&format!("    @{item}\n"));
            }
        } else if items.len() == 1 {
            out.push_str(&format!("    @{prefix}.{}\n", items[0]));
        } else {
            let mut group = wrap(&items.join(", "), &format!("    @{prefix}.{{"), 6);
            group.pop();
            group.push_str("}\n");
            out.push_str(&group);
        }
    }
    out
}

fn heading(section: Section) -> String {
    let text = format!("── {}. {} ", section.number(), section.title());
    let width = text.chars().count();
    format!("{text}{}", "─".repeat(RULE.saturating_sub(width)))
}

fn render_goal(bundle: &Bundle, model: &ResolvedModel<'_>, freshness: &Freshness) -> String {
    let ledger = model.ledger();
    let Some(record) = ledger.get(&bundle.goal) else {
        return String::new();
    };
    let mut out = format!("\n{}\n", header_line(record, None, None));
    if let Some(body) = body_of(record) {
        out.push('\n');
        out.push_str(&indent(body, 2));
    }
    let relations = relation_lines(record, model, freshness, 2);
    if !relations.is_empty() {
        out.push('\n');
        out.push_str(&relations);
    }
    out
}

fn render_milestone(bundle: &Bundle, model: &ResolvedModel<'_>) -> String {
    let entries = bundle.section(Section::Milestone);
    if entries.is_empty() {
        let kind = model
            .ledger()
            .get(&bundle.goal)
            .map_or("record", |r| r.kind.name());
        return format!("\nThe goal is itself a {kind}. No containing milestone or track.\n");
    }
    let mut out = String::new();
    for entry in entries {
        let Some(record) = model.ledger().get(entry.id()) else {
            continue;
        };
        out.push_str(&format!("\n{}\n", header_line(record, None, None)));
        if let Some(body) = body_of(record) {
            out.push_str(&indent(body, 2));
        }
    }
    out
}

fn render_records(
    bundle: &Bundle,
    model: &ResolvedModel<'_>,
    freshness: &Freshness,
    section: Section,
) -> String {
    let ledger = model.ledger();
    let mut out = String::new();
    for entry in bundle.section(section) {
        let Some(record) = ledger.get(entry.id()) else {
            continue;
        };
        let Entry::Record { via, depth, .. } = entry;
        let marker = freshness.marker(&record.id).map(|m| match m {
            "**stale**" => "[STALE]".to_owned(),
            _ => format!(
                "[AT RISK depth {}]",
                freshness.at_risk(&record.id).map_or(0, |r| r.depth)
            ),
        });
        // A normative record's scope is why it is in the bundle at all, so it stands in
        // for the "what brought this in" qualifier the other sections carry.
        let qualifier = if section == Section::Normative {
            scope_text(record)
        } else {
            context_of(model, record, *via, *depth)
        };
        out.push_str(&format!(
            "\n{}\n",
            header_line(record, marker.as_deref(), qualifier)
        ));
        let truncated = bundle.truncated.contains(&record.id);
        if truncated {
            out.push_str("  (prose truncated to fit the budget)\n");
        } else if let Some(body) = body_of(record) {
            out.push_str(&indent(body, 2));
        }
        // D-026: operator commentary on a planning record. Informational only, and shown
        // because the reason somebody abandoned or re-scoped a plan is exactly what the
        // next agent to touch it needs and cannot reconstruct.
        if let Some(ContentValue::Prose(note) | ContentValue::Text(note)) =
            record.get(ContentSlot::Note)
        {
            out.push_str(&format!("  note\n{}", indent(note, 4)));
        }
        if section == Section::WorkItems {
            // Step 3: "Blocked items are rendered with the live `blocks` edges that hold
            // them, so the reason is adjacent to the fact."
            out.push_str(&blocked_by_lines(record, model));
            if let Some(line) = disposition_line(model, record) {
                out.push_str(&line);
            }
        }
        if section == Section::PlanOfRecord {
            out.push_str(&dispositions_of(record, model));
        }
        if section == Section::Normative && !truncated {
            out.push_str(&claim_lines(record, 2));
        }
        let relations = relation_lines(record, model, freshness, 2);
        if !relations.is_empty() {
            out.push_str(&relations);
        }
        if section == Section::WorkItems || section == Section::Observations {
            out.push_str(&risk_lines(record, freshness, 2));
        }
    }
    out.push_str(&withheld_lines(bundle, section));
    out
}

/// The trailing qualifier on a header line: what brought the record in.
fn context_of(
    model: &ResolvedModel<'_>,
    record: &Record,
    via: Option<Relation>,
    depth: usize,
) -> Option<String> {
    match via {
        Some(Relation::PartOf) => {
            let parent = record
                .targets(Relation::PartOf)
                .first()
                .and_then(|reference| model.ledger().resolve(reference).ok().flatten())?;
            Some(format!("part_of @{}", parent.id))
        }
        Some(Relation::Blocks) => Some("blocker".to_owned()),
        Some(relation) if depth > 0 => Some(format!("via {relation}, depth {depth}")),
        _ => None,
    }
}

/// `@key/rev  kind  state  <qualifier>  <marker>`
fn header_line(record: &Record, marker: Option<&str>, context: Option<String>) -> String {
    let mut line = format!(
        "@{:<34} {:<11} {:<9}",
        record.id.to_string(),
        record.kind.name(),
        record.state.name()
    );
    if let Some(context) = context {
        line.push_str(&context);
    }
    let mut line = line.trim_end().to_owned();
    if let Some(marker) = marker {
        line.push_str(&format!("  {marker}"));
    }
    if let Some(commit) = observed_commit(record) {
        line.push_str(&format!("  observed_at {commit}"));
    }
    if let Some(ContentValue::Date(target)) = record.get(ContentSlot::Target) {
        line.push_str(&format!("  target {target}"));
    }
    line
}

/// `scope all` or `scope path "…"`, as a normative record declares it.
///
/// Scope is why a normative record is in the bundle at all, so it is shown rather than
/// left for the reader to infer from the section heading.
fn scope_text(record: &Record) -> Option<String> {
    if record.scope.is_empty() {
        return None;
    }
    let terms: Vec<String> = record
        .scope
        .iter()
        .map(|term| match term {
            ScopeTerm::All => "all".to_owned(),
            ScopeTerm::Path(glob) => format!("path {:?}", glob.as_str()),
            ScopeTerm::Ref(reference) => format!("ref {reference}"),
        })
        .collect();
    Some(format!("scope {}", terms.join(", ")))
}

/// The live `blocks` edges holding a blocked work item.
fn blocked_by_lines(record: &Record, model: &ResolvedModel<'_>) -> String {
    let ledger = model.ledger();
    let mut out = String::new();
    for other in crate::graph::sorted_records(ledger) {
        if !other.is_live() {
            continue;
        }
        for reference in other.targets(Relation::Blocks) {
            if ledger
                .resolve(reference)
                .ok()
                .flatten()
                .is_some_and(|target| target.id == record.id)
            {
                out.push_str(&format!(
                    "  BLOCKED BY  @{}  ({})\n",
                    other.id,
                    other.state.name()
                ));
            }
        }
    }
    out
}

/// A plan's own `disposition` blocks: what happened to the previous plan's children.
fn dispositions_of(record: &Record, model: &ResolvedModel<'_>) -> String {
    let ledger = model.ledger();
    let mut out = String::new();
    for disposition in &record.dispositions {
        let Some(target) = ledger.resolve(&disposition.target).ok().flatten() else {
            continue;
        };
        let mut line = format!(
            "  disposition @{:<27} {}",
            target.id.key.to_string(),
            disposition.outcome.name()
        );
        if let Some(into) = &disposition.into
            && let Some(target) = ledger.resolve(into).ok().flatten()
        {
            line.push_str(&format!(" -> @{}", target.id));
        }
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// A normative record's addressable claims, one per line under their anchor.
fn claim_lines(record: &Record, indent_by: usize) -> String {
    let pad = " ".repeat(indent_by);
    let mut out = String::new();
    let width = record
        .claims
        .iter()
        .map(|claim| claim.anchor.to_string().chars().count())
        .max()
        .map_or(0, |longest| longest + 2);
    for claim in &record.claims {
        let head = format!("{pad}#{:<width$}", claim.anchor.to_string());
        out.push_str(&wrap(&claim.text, &head, head.chars().count()));
    }
    out
}

/// Why a record is at risk, on the record itself.
fn risk_lines(record: &Record, freshness: &Freshness, indent_by: usize) -> String {
    let Some(entry) = freshness.at_risk(&record.id) else {
        return String::new();
    };
    let pad = " ".repeat(indent_by);
    let mut out = format!("{pad}AT RISK     via {} -> @{}\n", entry.via, entry.path[0]);
    for id in entry.path.iter().skip(1) {
        out.push_str(&format!("{pad}                             -> @{id}\n"));
    }
    out
}

/// Wraps `text` under a first-line prefix, continuation lines indented to `hang`.
fn wrap(text: &str, first: &str, hang: usize) -> String {
    let pad = " ".repeat(hang);
    let mut out = String::new();
    let mut line = first.to_owned();
    let mut width = first.chars().count();
    for word in text.split_whitespace() {
        if width > hang && width + 1 + word.chars().count() > 78 {
            out.push_str(line.trim_end());
            out.push('\n');
            line = pad.clone();
            width = hang;
        }
        line.push_str(word);
        line.push(' ');
        width += word.chars().count() + 1;
    }
    out.push_str(line.trim_end());
    out.push('\n');
    out
}

/// The relation slots of a record, resolved, one per line. Never truncated (V-123).
///
/// A target that is terminal is named as such, and a target that is stale or at risk
/// carries its marker: the whole reason to show a relation is that the reader has to know
/// what this record rests on, and "it rests on something questionable" is the part that
/// changes what they do next.
fn relation_lines(
    record: &Record,
    model: &ResolvedModel<'_>,
    freshness: &Freshness,
    indent_by: usize,
) -> String {
    let ledger = model.ledger();
    let pad = " ".repeat(indent_by);
    let mut out = String::new();
    for (relation, references) in &record.relations {
        if *relation == Relation::Contradicts {
            continue; // step 10 owns these
        }
        let targets: Vec<String> = references
            .iter()
            .map(|reference| match ledger.resolve(reference).ok().flatten() {
                Some(target) => {
                    let state = if target.is_live() {
                        String::new()
                    } else {
                        format!("  ({})", target.state.name())
                    };
                    let marker = if freshness.is_stale(&target.id) {
                        "  [STALE]".to_owned()
                    } else if let Some(risk) = freshness.at_risk(&target.id) {
                        format!("  [AT RISK depth {}]", risk.depth)
                    } else {
                        String::new()
                    };
                    format!("@{}{state}{marker}", target.id)
                }
                None => reference.to_string(),
            })
            .collect();
        if targets.is_empty() {
            continue;
        }
        out.push_str(&format!(
            "{pad}{:<14}{}\n",
            relation.name(),
            targets.join(", ")
        ));
    }
    // `exceptions` is a content slot rather than a relation, but it names records and a
    // policy is unreadable without it: the rule says "except on the tracks listed under
    // exceptions".
    if let Some(ContentValue::Refs(references)) = record.get(ContentSlot::Exceptions) {
        let targets: Vec<String> = references
            .iter()
            .map(|reference| match ledger.resolve(reference).ok().flatten() {
                Some(target) => format!("@{}", target.id),
                None => reference.to_string(),
            })
            .collect();
        if !targets.is_empty() {
            out.push_str(&format!(
                "{pad}{:<14}{}\n",
                "exceptions",
                targets.join(", ")
            ));
        }
    }
    for term in &record.scope {
        if let ScopeTerm::Ref(reference) = term
            && let Some(target) = ledger.resolve(reference).ok().flatten()
        {
            out.push_str(&format!("{pad}{:<14}@{}\n", "scope ref", target.id));
        }
    }
    out
}

/// The disposition governing a work item, where a superseding plan carries one.
fn disposition_line(model: &ResolvedModel<'_>, record: &Record) -> Option<String> {
    let ledger = model.ledger();
    for other in crate::graph::sorted_records(ledger) {
        for disposition in &other.dispositions {
            let target = ledger.resolve(&disposition.target).ok().flatten()?;
            if target.id != record.id {
                continue;
            }
            let mut line = format!(
                "  DISPOSITIONED by @{}: {}",
                other.id,
                disposition.outcome.name()
            );
            if let Some(into) = &disposition.into
                && let Some(target) = ledger.resolve(into).ok().flatten()
            {
                line.push_str(&format!(" into @{}", target.id));
            }
            line.push('\n');
            if let Some(note) = &disposition.note {
                line.push_str(&indent(note, 4));
            }
            return Some(line);
        }
    }
    None
}

fn render_acceptance(bundle: &Bundle, model: &ResolvedModel<'_>) -> String {
    let mut out = String::new();
    let mut owner: Option<RevisionId> = None;
    for verdict in &bundle.acceptance {
        if owner.as_ref() != Some(&verdict.owner) {
            let satisfied = bundle
                .acceptance
                .iter()
                .filter(|v| v.owner == verdict.owner && v.verdict.is_satisfied())
                .count();
            let total = bundle
                .acceptance
                .iter()
                .filter(|v| v.owner == verdict.owner)
                .count();
            out.push_str(&format!(
                "\n@{}  —  {satisfied} of {total} satisfied\n\n",
                verdict.owner
            ));
            owner = Some(verdict.owner.clone());
        }
        let mark = if verdict.verdict.is_satisfied() {
            "[x]"
        } else {
            "[ ]"
        };
        let method = model
            .ledger()
            .get(&verdict.owner)
            .and_then(|r| r.acceptance.as_ref())
            .and_then(|a| a.checks.iter().find(|c| c.id == verdict.check))
            .map_or("manual", |c| c.method.name());
        let check = model
            .ledger()
            .get(&verdict.owner)
            .and_then(|r| r.acceptance.as_ref())
            .and_then(|a| a.checks.iter().find(|c| c.id == verdict.check));
        out.push_str(&format!(
            "  {mark} {:<26} method {method}\n",
            verdict.check.as_str()
        ));
        if let Some(check) = check {
            out.push_str(&indent(&check.statement, 6));
            if let Some(command) = &check.command {
                out.push_str(&format!("      command {command:?}\n"));
            }
        }
        out.push_str(&format!("      {}\n", verdict_text(&verdict.verdict)));
    }
    out
}

fn verdict_text(verdict: &Verdict) -> String {
    match verdict {
        Verdict::Satisfied {
            by,
            observed_at,
            last_change: Some(last_change),
        } => format!(
            "SATISFIED by @{by}\n        result pass, observed_at {}, which descends from {}",
            &observed_at.as_str()[..8],
            &last_change.as_str()[..8]
        ),
        Verdict::Satisfied {
            by, observed_at, ..
        } => format!(
            "SATISFIED by @{by} (pass at {})",
            &observed_at.as_str()[..8]
        ),
        Verdict::NoEvidence => "NOT SATISFIED — no evidence".to_owned(),
        Verdict::Unresolved => "NOT SATISFIED — the cited evidence does not resolve".to_owned(),
        Verdict::Failing { by, result } => format!(
            "NOT SATISFIED — @{by} reports {}",
            result.map_or("no result", crate::model::EvidenceResult::name)
        ),
        Verdict::TooOld {
            by,
            observed_at,
            last_change,
        } => format!(
            "NOT SATISFIED — @{by} observed at {}, which does not descend from {}",
            &observed_at.as_str()[..8],
            &last_change.as_str()[..8]
        ),
        Verdict::TooOldUncommitted { by, observed_at } => format!(
            "NOT SATISFIED — @{by} observed at {}, which does not descend from the uncommitted change",
            &observed_at.as_str()[..8]
        ),
    }
}

fn render_contradictions(bundle: &Bundle, model: &ResolvedModel<'_>) -> String {
    let ledger = model.ledger();
    let mut out = String::new();
    for pair in &bundle.contradictions {
        out.push_str(&format!("\n@{}  <->  @{}\n", pair.left, pair.right));
        out.push_str(&format!("  declared by  @{}\n", pair.declared_by));
        out.push_str(&format!(
            "  status       {}\n",
            if pair.acknowledged {
                "ACKNOWLEDGED (acknowledged true)"
            } else {
                "UNRESOLVED"
            }
        ));
        for endpoint in [&pair.left, &pair.right] {
            if let Some(record) = ledger.get(endpoint)
                && !record.is_live()
            {
                out.push_str(&format!(
                    "  note         @{} is {}\n",
                    record.id,
                    record.state.name()
                ));
            }
        }
    }
    out
}

fn render_staleness(bundle: &Bundle, freshness: &Freshness) -> String {
    let entries = bundle.section(Section::Staleness);
    let stale: Vec<&Entry> = entries
        .iter()
        .filter(|e| freshness.is_stale(e.id()))
        .collect();
    let at_risk: Vec<&Entry> = entries
        .iter()
        .filter(|e| !freshness.is_stale(e.id()))
        .collect();

    let mut out = String::new();
    out.push_str(&format!("\nSTALE ({})\n", stale.len()));
    for entry in &stale {
        out.push_str(&format!("  @{}\n", entry.id()));
        // Step 11: "Each entry names the cause." A staleness list without causes tells the
        // reader what to distrust but not what to do about it.
        if let Some(cause) = freshness.cause(entry.id()) {
            out.push_str(&format!("      {}\n", cause_text(cause)));
        }
    }
    if stale.is_empty() {
        out.push_str("  (none)\n");
    }
    out.push_str(&format!("\nAT RISK ({})\n", at_risk.len()));
    let width = at_risk
        .iter()
        .map(|e| e.id().to_string().len() + 1)
        .max()
        .map_or(0, |longest| longest + 3);
    for entry in &at_risk {
        let Some(flag) = freshness.at_risk(entry.id()) else {
            continue;
        };
        out.push_str(&format!(
            "  {:<width$}depth {}\n",
            format!("@{}", entry.id()),
            flag.depth
        ));
        for (index, id) in flag.path.iter().enumerate() {
            if index == 0 {
                out.push_str(&format!("      {:<12} -> @{id}\n", flag.via.to_string()));
            } else {
                out.push_str(&format!("      {:<12} -> @{id}\n", ""));
            }
        }
    }
    if at_risk.is_empty() {
        out.push_str("  (none)\n");
    }
    out.push_str(
        "\n  These are warnings, not diagnostics. No record above has been changed and\n  \
         none has been declared false (D-003, D-024).\n",
    );
    out
}

/// Why a record is stale, in one line.
fn cause_text(cause: &crate::freshness::StaleCause) -> String {
    match cause {
        crate::freshness::StaleCause::Watch { glob, commit, .. } => format!(
            "watches {:?} matched by {}",
            glob.as_str(),
            &commit.as_str()[..8]
        ),
        crate::freshness::StaleCause::ReviewAfter { date } => {
            format!("review_after {date} passed")
        }
    }
}

fn indent(text: &str, by: usize) -> String {
    let pad = " ".repeat(by);
    let mut out = String::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            out.push('\n');
        } else {
            out.push_str(&format!("{pad}{line}\n"));
        }
    }
    out
}

// -------------------------------------------------------------------------------------
// JSON
// -------------------------------------------------------------------------------------

/// Renders the JSON form of `docs/08-mcp.md` §3.
///
/// Sections appear in the fixed order, always, whether or not they are empty — the same
/// contract as the text form and for the same reason.
#[must_use]
pub fn render_json(bundle: &Bundle, model: &ResolvedModel<'_>) -> crate::json::Value {
    use crate::json::Value;
    let ledger = model.ledger();

    let goal_title = ledger
        .get(&bundle.goal)
        .map_or_else(String::new, |r| r.title.clone());

    let sections: Vec<Value> = Section::ALL
        .iter()
        .map(|section| {
            let records: Vec<Value> = bundle
                .section(*section)
                .iter()
                .map(|entry| {
                    let Entry::Record { id, via, depth } = entry;
                    let mut fields = vec![
                        ("key", Value::string(id.key.to_string())),
                        ("rev", Value::integer(i64::from(id.revision))),
                    ];
                    if let Some(record) = ledger.get(id) {
                        fields.push(("kind", Value::string(record.kind.name())));
                        fields.push(("state", Value::string(record.state.name())));
                        fields.push(("title", Value::string(record.title.clone())));
                    }
                    if let Some(via) = via {
                        fields.push(("via", Value::string(via.name())));
                    }
                    if *depth > 0 {
                        fields.push(("depth", Value::integer(*depth as i64)));
                    }
                    Value::object(fields)
                })
                .collect();
            Value::object(vec![
                ("id", Value::string(section.id())),
                ("records", Value::array(records)),
            ])
        })
        .collect();

    let acceptance: Vec<Value> = bundle
        .acceptance
        .iter()
        .map(|verdict| {
            Value::object(vec![
                ("owner", Value::string(verdict.owner.to_string())),
                ("check", Value::string(verdict.check.as_str())),
                ("satisfied", Value::bool(verdict.verdict.is_satisfied())),
                (
                    "verdict",
                    Value::string(verdict_text(&verdict.verdict).replace('\n', " ")),
                ),
            ])
        })
        .collect();

    let contradictions: Vec<Value> = bundle
        .contradictions
        .iter()
        .map(|pair| {
            Value::object(vec![
                ("left", Value::string(pair.left.to_string())),
                ("right", Value::string(pair.right.to_string())),
                ("declared_by", Value::string(pair.declared_by.to_string())),
                ("acknowledged", Value::bool(pair.acknowledged)),
            ])
        })
        .collect();

    Value::object(vec![
        (
            "goal",
            Value::object(vec![
                ("key", Value::string(bundle.goal.key.to_string())),
                ("rev", Value::integer(i64::from(bundle.goal.revision))),
                ("title", Value::string(goal_title)),
                // The goal's intent is what the bundle exists to serve. A structured
                // reader that saw only key, rev and title had to fall back to one
                // `knowledge.get` per record to learn what it had been asked to do.
                (
                    "body",
                    ledger
                        .get(&bundle.goal)
                        .and_then(body_of)
                        .map_or(Value::Null, |body| Value::string(body.trim_end())),
                ),
            ]),
        ),
        (
            "commit",
            bundle
                .commit
                .as_ref()
                .map_or(Value::Null, |c| Value::string(c.clone())),
        ),
        (
            "paths",
            Value::array(
                bundle
                    .paths
                    .iter()
                    .map(|g| Value::string(g.as_str()))
                    .collect(),
            ),
        ),
        ("sections", Value::array(sections)),
        ("acceptance", Value::array(acceptance)),
        ("contradictions", Value::array(contradictions)),
        (
            "excluded",
            Value::object(vec![
                (
                    "superseded",
                    Value::integer(bundle.excluded.superseded.len() as i64),
                ),
                (
                    "archived",
                    Value::integer(bundle.excluded.archived.len() as i64),
                ),
                (
                    "terminal",
                    Value::integer(bundle.excluded.terminal.len() as i64),
                ),
                (
                    "out_of_scope",
                    Value::integer(bundle.excluded.out_of_scope.len() as i64),
                ),
            ]),
        ),
        (
            "truncated_prose",
            Value::array(
                bundle
                    .truncated
                    .iter()
                    .map(|id| Value::string(id.to_string()))
                    .collect(),
            ),
        ),
        (
            "withheld",
            Value::array(
                bundle
                    .withheld
                    .iter()
                    .map(|(section, ids)| {
                        Value::object(vec![
                            ("section", Value::string(section.id())),
                            (
                                "records",
                                Value::array(
                                    ids.iter().map(|id| Value::string(id.to_string())).collect(),
                                ),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "estimated_tokens",
            Value::integer(bundle.estimated_tokens as i64),
        ),
    ])
}

//! Compact index of outstanding planning heads.
use super::common::{is_archived, link};
use super::{RenderContext, banner};
use crate::model::{Kind, Relation, State};

pub(super) fn render(cx: RenderContext<'_>) -> String {
    let mut blocks = vec![banner(cx.model).trim_end().to_owned(),
        format!("# Outstanding work — {}", cx.ledger().project.name),
        "Live planning heads only. Proposed normative records are recorded claims, not a work queue. Acceptance counts reflect current evidence; details and blockers are in the linked views.".to_owned()];
    let visible_work: std::collections::BTreeSet<_> =
        super::active_work::live_work(cx.ledger(), cx)
            .into_iter()
            .map(|r| &r.id)
            .collect();
    for (state, heading) in [
        (State::Active, "Active"),
        (State::Blocked, "Blocked"),
        (State::Ready, "Ready"),
        (State::Proposed, "Proposed"),
    ] {
        blocks.push(format!("## {heading}"));
        let mut records: Vec<_> = cx
            .model
            .heads
            .values()
            .filter_map(|id| cx.ledger().get(id))
            .filter(|r| {
                matches!(r.kind, Kind::Milestone | Kind::Track | Kind::Work)
                    && r.state == state
                    && !is_archived(r)
                    && (r.kind != Kind::Work || visible_work.contains(&r.id))
            })
            .collect();
        records.sort_by(|a, b| a.id.cmp(&b.id));
        if records.is_empty() {
            blocks.push("_(none)_".to_owned());
            continue;
        }
        let lines: Vec<_> = records
            .iter()
            .map(|r| {
                let checks = cx.model.checks_of(&r.id);
                let satisfied = checks.iter().filter(|v| v.verdict.is_satisfied()).count();
                let parent = r
                    .targets(Relation::PartOf)
                    .first()
                    .and_then(|p| cx.ledger().resolve(p).ok().flatten())
                    .map(|p| format!(" · part of `@{}`", p.id))
                    .unwrap_or_default();
                let acceptance = if checks.is_empty() {
                    "no acceptance checks".to_owned()
                } else {
                    format!("{satisfied}/{} checks satisfied", checks.len())
                };
                format!(
                    "- {} `@{}` · {} · {acceptance}{parent}",
                    link(r),
                    r.id,
                    r.kind.name()
                )
            })
            .collect();
        blocks.push(lines.join("\n"));
    }
    format!("{}\n", blocks.join("\n\n"))
}

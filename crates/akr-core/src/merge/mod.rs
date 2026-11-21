//! Semantic three-way merging for `.akr` record files.
//!
//! Git's textual 3-way merge does not understand AKR's record structure:
//!
//! - Two branches appending distinct records to the same `.akr/records/<ns>/<kind>.akr`
//!   file collide on the same trailing lines.
//! - Two branches appending different `trace` entries to the same work record conflict
//!   line-by-line even though traces are an ordered append-only log.
//! - Two branches concurrently minting revision `n+1` of the same key (or concurrently
//!   editing a `proposed` revision) produce conflicting or duplicate `(key, revision)`
//!   blocks that violate V-012 ("at most one live head per key").
//!
//! This module merges `.akr` record files semantically:
//!
//! 1. Disjoint records merge cleanly in canonical order.
//! 2. Concurrent `trace` additions to the same revision concatenate in deterministic
//!    order (`base`, then `ours` additions, then `theirs` additions, deduplicated).
//! 3. Concurrent non-trivial edits or duplicate revision numbers for the same `(key, rev)`
//!    are resolved automatically by keeping `ours` at its revision number and renumbering
//!    `theirs` to `max_rev + 1` with `supersedes [ @key/<ours_rev> ]` (and retiring `ours`
//!    to `superseded` when both sides are live).
//! 4. Output is formatted canonically through the parser and formatter.

use crate::diagnostics::FileId;
use crate::model::{
    ContentSlot, ContentValue, Ledger, LogicalKey, Record, Reference, Relation, RevisionId, State,
};
use crate::syntax::{self, emit};
use std::collections::{BTreeMap, BTreeSet};

/// One revision renumbered during a semantic merge to preserve revision uniqueness and
/// single-live-head invariants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Renumbered {
    /// The logical key that had a concurrent revision collision.
    pub key: LogicalKey,
    /// The revision number `theirs` originally carried.
    pub from_revision: u32,
    /// The new revision number assigned to `theirs`.
    pub to_revision: u32,
    /// The revision number kept from `ours` that the renumbered revision now supersedes.
    pub supersedes_revision: u32,
}

/// The result of merging a `.akr` record file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeOutcome {
    /// The canonically formatted merged `.akr` file text.
    pub content: String,
    /// Revisions automatically renumbered to resolve concurrent same-key collisions.
    pub renumbered: Vec<Renumbered>,
    /// Revisions whose `trace` arrays were merged cleanly across both branches.
    pub merged_traces: Vec<RevisionId>,
}

/// Why a semantic merge could not proceed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeError {
    /// One of the three input versions failed to parse as valid AKR syntax.
    Parse {
        /// Which input failed (`base`, `ours`, or `theirs`).
        side: &'static str,
        /// The first syntax or lowering diagnostic message.
        message: String,
    },
    /// No input version declared a `project <name>` header.
    MissingProject,
}

impl std::fmt::Display for MergeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse { side, message } => {
                write!(f, "cannot parse `{side}` as AKR records: {message}")
            }
            Self::MissingProject => {
                write!(
                    f,
                    "none of base, ours, or theirs declared a `project` header"
                )
            }
        }
    }
}

impl std::error::Error for MergeError {}

/// Parses one side of a 3-way merge. An empty file yields an empty ledger and no project
/// name (the normal case for `base` on an add/add conflict).
fn parse_side(
    side: &'static str,
    text: &str,
) -> Result<(Option<String>, BTreeMap<RevisionId, Record>), MergeError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok((None, BTreeMap::new()));
    }
    let parsed = syntax::parse(text, FileId(0));
    if let Some(err) = parsed
        .diagnostics
        .iter()
        .find(|d| d.severity == crate::diagnostics::Severity::Error)
    {
        return Err(MergeError::Parse {
            side,
            message: err.message.clone(),
        });
    }
    let Some(file) = parsed.file else {
        return Err(MergeError::Parse {
            side,
            message: "empty parse tree".to_owned(),
        });
    };
    let project = (!file.project.is_empty()).then(|| file.project.clone());
    let (ledger, lowered_diags) = syntax::lower_all(&[(side.to_owned(), file)]);
    if let Some(err) = lowered_diags
        .iter()
        .find(|d| d.severity == crate::diagnostics::Severity::Error)
    {
        return Err(MergeError::Parse {
            side,
            message: err.message.clone(),
        });
    }
    let mut map = BTreeMap::new();
    for mut record in ledger.records().to_vec() {
        // Strip source provenance fields so semantic equality compares content only.
        record.file = None;
        map.insert(record.id.clone(), record);
    }
    Ok((project, map))
}

/// Semantically merges three versions of a `.akr` record file (`base`, `ours`, `theirs`).
///
/// # Errors
/// Returns [`MergeError::Parse`] if any non-empty input has syntax errors, or
/// [`MergeError::MissingProject`] if none of the inputs declares a project header.
pub fn merge_record_file(base: &str, ours: &str, theirs: &str) -> Result<MergeOutcome, MergeError> {
    let (base_proj, base_map) = parse_side("base", base)?;
    let (ours_proj, ours_map) = parse_side("ours", ours)?;
    let (theirs_proj, theirs_map) = parse_side("theirs", theirs)?;

    let project = ours_proj
        .or(theirs_proj)
        .or(base_proj)
        .ok_or(MergeError::MissingProject)?;

    // Track the highest revision number seen for every key across all three inputs.
    let mut max_rev: BTreeMap<LogicalKey, u32> = BTreeMap::new();
    for id in base_map
        .keys()
        .chain(ours_map.keys())
        .chain(theirs_map.keys())
    {
        let entry = max_rev.entry(id.key.clone()).or_insert(0);
        if id.revision > *entry {
            *entry = id.revision;
        }
    }

    let mut all_ids: BTreeSet<RevisionId> = BTreeSet::new();
    all_ids.extend(ours_map.keys().cloned());
    all_ids.extend(theirs_map.keys().cloned());

    let mut merged: BTreeMap<RevisionId, Record> = BTreeMap::new();
    // Track which merged records originated from `theirs` so same-file pin rewrites only
    // adjust `theirs`'s references when a collision is renumbered.
    let mut from_theirs: BTreeSet<RevisionId> = BTreeSet::new();
    let mut renumbered: Vec<Renumbered> = Vec::new();
    let mut merged_traces: Vec<RevisionId> = Vec::new();

    for id in all_ids {
        let b = base_map.get(&id);
        let o = ours_map.get(&id);
        let t = theirs_map.get(&id);

        match (b, o, t) {
            // Only in ours.
            (None, Some(ours_rec), None) => {
                merged.insert(id, ours_rec.clone());
            }
            // Only in theirs.
            (None, None, Some(theirs_rec)) => {
                merged.insert(id.clone(), theirs_rec.clone());
                from_theirs.insert(id);
            }
            // Present in base and ours, removed in theirs: respect deletion if unchanged
            // in ours; otherwise keep ours's modification.
            (Some(base_rec), Some(ours_rec), None) => {
                if !records_equal(base_rec, ours_rec) {
                    merged.insert(id, ours_rec.clone());
                }
            }
            // Present in base and theirs, removed in ours: symmetric.
            (Some(base_rec), None, Some(theirs_rec)) => {
                if !records_equal(base_rec, theirs_rec) {
                    merged.insert(id.clone(), theirs_rec.clone());
                    from_theirs.insert(id);
                }
            }
            // Present in both ours and theirs.
            (base_opt, Some(ours_rec), Some(theirs_rec)) => {
                if records_equal(ours_rec, theirs_rec) {
                    merged.insert(id, ours_rec.clone());
                    continue;
                }
                if let Some(base_rec) = base_opt {
                    if records_equal(base_rec, ours_rec) {
                        merged.insert(id.clone(), theirs_rec.clone());
                        from_theirs.insert(id);
                        continue;
                    }
                    if records_equal(base_rec, theirs_rec) {
                        merged.insert(id, ours_rec.clone());
                        continue;
                    }
                    // Both sides modified `id` relative to `base`. Try a clean trace merge
                    // first (including trace-only on one side + non-trace bookkeeping on the
                    // other).
                    if let Some(combined) =
                        try_merge_trace_or_bookkeeping(base_rec, ours_rec, theirs_rec)
                    {
                        merged.insert(id.clone(), combined);
                        merged_traces.push(id);
                        continue;
                    }
                }

                // True concurrent edit or duplicate revision minting for the same `(key, rev)`.
                // Auto-renumber `theirs` to `max_rev + 1` superseding `ours`.
                let next_rev = {
                    let entry = max_rev.entry(id.key.clone()).or_insert(id.revision);
                    *entry += 1;
                    *entry
                };
                let mut kept_ours = ours_rec.clone();
                let mut bumped_theirs = theirs_rec.clone();
                bumped_theirs.id = RevisionId::new(id.key.clone(), next_rev);

                let mut supersedes = bumped_theirs.targets(Relation::Supersedes).to_vec();
                supersedes.push(Reference::pinned(id.key.clone(), id.revision));
                supersedes.sort();
                supersedes.dedup();
                bumped_theirs
                    .relations
                    .insert(Relation::Supersedes, supersedes);

                if kept_ours.is_live() && bumped_theirs.is_live() {
                    kept_ours.state = State::Superseded;
                }

                let new_id = bumped_theirs.id.clone();
                merged.insert(id.clone(), kept_ours);
                merged.insert(new_id.clone(), bumped_theirs);
                from_theirs.insert(new_id);
                renumbered.push(Renumbered {
                    key: id.key.clone(),
                    from_revision: id.revision,
                    to_revision: next_rev,
                    supersedes_revision: id.revision,
                });
            }
            (None | Some(_), None, None) => {}
        }
    }

    // Rewrite same-file non-historical pins in `from_theirs` records that referenced a
    // revision we just renumbered.
    if !renumbered.is_empty() {
        let remap: BTreeMap<(LogicalKey, u32), u32> = renumbered
            .iter()
            .map(|r| ((r.key.clone(), r.from_revision), r.to_revision))
            .collect();
        for (id, record) in &mut merged {
            if !from_theirs.contains(id) {
                continue;
            }
            record.rewrite_non_historical_pins(|reference| {
                let Some(rev) = reference.revision else {
                    return false;
                };
                if let Some(&new_rev) = remap.get(&(reference.key.clone(), rev)) {
                    reference.revision = Some(new_rev);
                    true
                } else {
                    false
                }
            });
        }
    }

    // Ensure no key ends up with two live heads (e.g. ours minted key/2 live while theirs
    // minted key/2 superseded -> key/3 live). Keep the highest live revision as the live
    // head and retire earlier live revisions into its `supersedes` list.
    enforce_single_live_head(&mut merged);

    let content = render_merged_file(&project, &merged);
    Ok(MergeOutcome {
        content,
        renumbered,
        merged_traces,
    })
}

/// Ensures at most one live revision exists per key in `merged`.
fn enforce_single_live_head(merged: &mut BTreeMap<RevisionId, Record>) {
    let mut live_by_key: BTreeMap<LogicalKey, Vec<u32>> = BTreeMap::new();
    for (id, record) in merged.iter() {
        if record.is_live() {
            live_by_key
                .entry(id.key.clone())
                .or_default()
                .push(id.revision);
        }
    }
    for (key, mut revs) in live_by_key {
        if revs.len() <= 1 {
            continue;
        }
        revs.sort();
        let Some(&highest) = revs.last() else {
            continue;
        };
        let earlier = &revs[..revs.len() - 1];
        for &rev in earlier {
            if let Some(rec) = merged.get_mut(&RevisionId::new(key.clone(), rev)) {
                rec.state = State::Superseded;
            }
        }
        if let Some(head) = merged.get_mut(&RevisionId::new(key.clone(), highest)) {
            let mut supersedes = head.targets(Relation::Supersedes).to_vec();
            for &rev in earlier {
                supersedes.push(Reference::pinned(key.clone(), rev));
            }
            supersedes.sort();
            supersedes.dedup();
            head.relations.insert(Relation::Supersedes, supersedes);
        }
    }
}

/// Compares two records semantically (ignoring `file`).
fn records_equal(a: &Record, b: &Record) -> bool {
    let mut a = a.clone();
    let mut b = b.clone();
    a.file = None;
    b.file = None;
    a == b
}

/// Strips `ContentSlot::Trace` from a record for trace-only comparison.
fn without_trace(record: &Record) -> Record {
    let mut copy = record.clone();
    copy.file = None;
    copy.content.remove(&ContentSlot::Trace);
    copy
}

fn trace_entries(record: &Record) -> Vec<String> {
    match record.get(ContentSlot::Trace) {
        Some(ContentValue::Strings(entries)) => entries.clone(),
        _ => Vec::new(),
    }
}

/// Merges two traces relative to `base`: starts with `base`, appends new entries from
/// `ours`, then appends new entries from `theirs` not already present.
fn merge_trace_lists(base: &[String], ours: &[String], theirs: &[String]) -> Vec<String> {
    let mut out = base.to_vec();
    for entry in ours {
        if !out.contains(entry) {
            out.push(entry.clone());
        }
    }
    for entry in theirs {
        if !out.contains(entry) {
            out.push(entry.clone());
        }
    }
    out
}

/// Attempts a clean merge when at least one side only modified `trace` (or both sides only
/// modified `trace`).
fn try_merge_trace_or_bookkeeping(base: &Record, ours: &Record, theirs: &Record) -> Option<Record> {
    let base_nt = without_trace(base);
    let ours_nt = without_trace(ours);
    let theirs_nt = without_trace(theirs);

    let merged_trace = merge_trace_lists(
        &trace_entries(base),
        &trace_entries(ours),
        &trace_entries(theirs),
    );

    if base_nt == ours_nt && base_nt == theirs_nt {
        let mut combined = ours.clone();
        if !merged_trace.is_empty() {
            combined
                .content
                .insert(ContentSlot::Trace, ContentValue::Strings(merged_trace));
        }
        return Some(combined);
    }
    // Ours only changed trace; theirs changed something else (e.g. completed or passed_at).
    if base_nt == ours_nt {
        let mut combined = theirs.clone();
        if !merged_trace.is_empty() {
            combined
                .content
                .insert(ContentSlot::Trace, ContentValue::Strings(merged_trace));
        }
        return Some(combined);
    }
    // Theirs only changed trace; ours changed something else.
    if base_nt == theirs_nt {
        let mut combined = ours.clone();
        if !merged_trace.is_empty() {
            combined
                .content
                .insert(ContentSlot::Trace, ContentValue::Strings(merged_trace));
        }
        return Some(combined);
    }
    None
}

/// Renders a merged set of records into canonical AKR file text.
fn render_merged_file(project: &str, records: &BTreeMap<RevisionId, Record>) -> String {
    let mut raw = format!("akr 0.1\nproject {project}\n");
    for record in records.values() {
        raw.push('\n');
        raw.push_str(&emit::record_text(record, project));
    }
    let parsed = syntax::parse(&raw, FileId(0));
    match parsed.file {
        Some(file) if parsed.diagnostics.is_empty() => syntax::format(&file),
        _ => raw,
    }
}

/// Convenience helper for tests and callers that want to parse a merged file back into a
/// [`Ledger`].
#[must_use]
pub fn parse_merged_ledger(text: &str) -> Option<Ledger> {
    let parsed = syntax::parse(text, FileId(0));
    let file = parsed.file?;
    Some(syntax::lower_all(&[("merged.akr".to_owned(), file)]).0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "\
akr 0.1
project demo

record demo.work.alpha/1 : work {
    title \"Alpha\"
    state active
    intent \"\"\"
        Ship alpha.
        \"\"\"
    trace [ \"Started alpha\" ]
}
";

    #[test]
    fn disjoint_records_appended_on_both_branches_merge_cleanly() {
        let ours = format!(
            "{BASE}\nrecord demo.work.beta/1 : work {{\n    title \"Beta\"\n    state active\n    intent \"\"\"\n        Ship beta.\n        \"\"\"\n}}\n"
        );
        let theirs = format!(
            "{BASE}\nrecord demo.work.gamma/1 : work {{\n    title \"Gamma\"\n    state active\n    intent \"\"\"\n        Ship gamma.\n        \"\"\"\n}}\n"
        );

        let outcome = merge_record_file(BASE, &ours, &theirs).expect("clean merge");
        assert!(outcome.renumbered.is_empty());
        assert!(outcome.merged_traces.is_empty());
        let ledger = parse_merged_ledger(&outcome.content).expect("valid merged ledger");
        assert_eq!(ledger.records().len(), 3);
        assert!(
            ledger
                .head(&LogicalKey::parse("demo.work.alpha").unwrap())
                .is_ok()
        );
        assert!(
            ledger
                .head(&LogicalKey::parse("demo.work.beta").unwrap())
                .is_ok()
        );
        assert!(
            ledger
                .head(&LogicalKey::parse("demo.work.gamma").unwrap())
                .is_ok()
        );
    }

    #[test]
    fn concurrent_trace_appends_concatenate_without_renumbering() {
        let ours = BASE.replace(
            "trace [ \"Started alpha\" ]",
            "trace [ \"Started alpha\", \"Ours step\" ]",
        );
        let theirs = BASE.replace(
            "trace [ \"Started alpha\" ]",
            "trace [ \"Started alpha\", \"Theirs step\" ]",
        );

        let outcome = merge_record_file(BASE, &ours, &theirs).expect("trace merge");
        assert!(outcome.renumbered.is_empty());
        assert_eq!(outcome.merged_traces.len(), 1);
        let ledger = parse_merged_ledger(&outcome.content).expect("valid merged ledger");
        let rec = ledger
            .head(&LogicalKey::parse("demo.work.alpha").unwrap())
            .expect("head");
        assert_eq!(
            trace_entries(rec),
            vec![
                "Started alpha".to_owned(),
                "Ours step".to_owned(),
                "Theirs step".to_owned()
            ]
        );
    }

    #[test]
    fn concurrent_same_key_revision_auto_renumbers_theirs_and_supersedes_ours() {
        let ours = "\
akr 0.1
project demo

record demo.work.alpha/1 : work {
    title \"Alpha\"
    state superseded
    intent \"\"\"
        Ship alpha.
        \"\"\"
}

record demo.work.alpha/2 : work {
    title \"Alpha ours\"
    state active
    intent \"\"\"
        Ours plan.
        \"\"\"
    supersedes [ @demo.work.alpha/1 ]
}
";
        let theirs = "\
akr 0.1
project demo

record demo.work.alpha/1 : work {
    title \"Alpha\"
    state superseded
    intent \"\"\"
        Ship alpha.
        \"\"\"
}

record demo.work.alpha/2 : work {
    title \"Alpha theirs\"
    state active
    intent \"\"\"
        Theirs plan.
        \"\"\"
    supersedes [ @demo.work.alpha/1 ]
}
";
        let outcome = merge_record_file(BASE, ours, theirs).expect("auto-renumber merge");
        assert_eq!(outcome.renumbered.len(), 1);
        assert_eq!(outcome.renumbered[0].from_revision, 2);
        assert_eq!(outcome.renumbered[0].to_revision, 3);
        assert_eq!(outcome.renumbered[0].supersedes_revision, 2);

        let ledger = parse_merged_ledger(&outcome.content).expect("valid merged ledger");
        let key = LogicalKey::parse("demo.work.alpha").unwrap();
        let revs = ledger.revisions_of(&key);
        assert_eq!(revs.len(), 3);
        assert_eq!(revs[0].state, State::Superseded);
        assert_eq!(revs[1].state, State::Superseded);
        assert_eq!(revs[1].title, "Alpha ours");
        assert_eq!(revs[2].id.revision, 3);
        assert_eq!(revs[2].state, State::Active);
        assert_eq!(revs[2].title, "Alpha theirs");
        assert!(
            revs[2]
                .targets(Relation::Supersedes)
                .contains(&Reference::pinned(key.clone(), 2))
        );
        // Single live head invariant preserved.
        assert_eq!(ledger.head(&key).unwrap().id.revision, 3);
    }

    #[test]
    fn add_add_conflict_with_empty_base_merges_cleanly() {
        let ours = "\
akr 0.1
project demo

record demo.work.one/1 : work {
    title \"One\"
    state active
    intent \"\"\"
        One.
        \"\"\"
}
";
        let theirs = "\
akr 0.1
project demo

record demo.work.two/1 : work {
    title \"Two\"
    state active
    intent \"\"\"
        Two.
        \"\"\"
}
";
        let outcome = merge_record_file("", ours, theirs).expect("add/add merge");
        let ledger = parse_merged_ledger(&outcome.content).expect("valid merged ledger");
        assert_eq!(ledger.records().len(), 2);
    }
}

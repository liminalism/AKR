//! The compact session head prepended by `akr start` and `knowledge.start`.
//!
//! This is a projection, never authority: records remain authoritative for intent and
//! Git remains authoritative for snapshots. The projection exists to make the first
//! read of a session useful without making the agent search for a chronologically recent
//! planning key.
//!
//! It is also the orientation layer an advisor packet embeds verbatim
//! ([`super::packet`]), so that a second model arriving cold does not have to rediscover
//! the ledger's shape before it can look at the code.

use crate::session::{EnvError, Session};
use akr_core::change::{self, SemanticDelta};
use akr_core::diagnostics::{Diagnostic, FileId, Severity};
use akr_core::freshness::ReviewQueue;
use akr_core::json::Value;
use akr_core::model::{Class, Kind, Ledger, LogicalKey, Record, Relation, RevisionId, State};
use akr_core::resolve::{BuildInputs, ResolvedModel, Verdict};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const DEFAULT_BUDGET: usize = 1_400;
const MIN_LIST_ITEMS: usize = 4;

/// An assembled handoff plus the committed ledger used only when the working ledger was
/// invalid and task orientation must avoid its disposable index.
pub struct Handoff {
    /// Human-readable compact rendering.
    pub text: String,
    /// Structured MCP/JSON rendering.
    pub value: Value,
    /// A validated HEAD ledger when the working-tree ledger could not be trusted.
    pub fallback_ledger: Option<Ledger>,
}

struct Snapshot {
    ledger: Ledger,
    source_graph: String,
    origin: &'static str,
    excluded_diagnostics: usize,
    overlay: SemanticDelta,
}

/// The age at which an unkept scratch entry is worth mentioning, in days.
///
/// The same fourteen `akr scratch prune` defaults to: a session head that flagged entries
/// the command would not remove would be pointing at work nobody can act on.
const SCRATCH_AGE_DAYS: u64 = 14;

/// Assemble the handoff from a validated working ledger or a separately parsed HEAD.
///
/// # Errors
/// Returns an environment error only when neither snapshot can be trusted or Git facts
/// required by the command are unavailable.
pub fn assemble(session: &Session, budget: Option<usize>) -> Result<Handoff, EnvError> {
    let current_model = session.resolve();
    let current_diagnostics = session.diagnostics(&current_model);
    let current_errors = current_diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .count();
    let repository = session.repository.as_ref();
    let head = repository.and_then(|repository| repository.head().ok());
    let snapshot = if let (Some(repository), Some(head)) = (repository, head.as_ref()) {
        let head_files = change::akr_files_at(repository, head)
            .map_err(|error| EnvError::new("AKR-G001", error.to_string()))?;
        let (head_ledger, head_diagnostics, head_graph) = ledger_from_files(&head_files);
        let head_errors = head_diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == Severity::Error)
            .count();
        if current_errors == 0 {
            Snapshot {
                overlay: change::delta(Some(&head_ledger), &session.ledger, &[]),
                ledger: session.ledger.clone(),
                source_graph: session.source_graph(),
                origin: "working_tree",
                excluded_diagnostics: 0,
            }
        } else if head_errors == 0 {
            Snapshot {
                ledger: head_ledger,
                source_graph: head_graph,
                origin: "head_fallback",
                excluded_diagnostics: current_errors,
                overlay: SemanticDelta::default(),
            }
        } else {
            return Err(EnvError::new(
                "AKR-C031",
                format!(
                    "neither working-tree knowledge ({current_errors} errors) nor HEAD knowledge ({head_errors} errors) validates"
                ),
            )
            .help("run `akr check` for the working-tree diagnostics and repair the ledger before starting work"));
        }
    } else if current_errors == 0 {
        Snapshot {
            ledger: session.ledger.clone(),
            source_graph: session.source_graph(),
            origin: "working_tree_no_git",
            excluded_diagnostics: 0,
            overlay: SemanticDelta::default(),
        }
    } else {
        return Err(EnvError::new(
            "AKR-C031",
            format!("working-tree knowledge has {current_errors} errors and no committed snapshot is available"),
        )
        .help("run `akr check` and repair the ledger before starting work"));
    };

    let inputs = BuildInputs {
        commit: head.clone(),
        ..BuildInputs::default()
    };
    let model = ResolvedModel::build(&snapshot.ledger, &inputs);
    let queue = match (repository, head.as_ref()) {
        (Some(repository), Some(head)) => {
            akr_core::freshness::derive(&snapshot.ledger, repository, head, session.today)
                .unwrap_or_default()
        }
        _ => ReviewQueue::default(),
    };
    let (latest, linked) = match repository {
        Some(repository) if head.is_some() => repository
            .session_head()
            .map(|(latest, linked)| (Some(latest), linked))
            .map_err(|error| EnvError::new("AKR-G001", error.to_string()))?,
        _ => (None, None),
    };
    let changed_paths = repository
        .and_then(|repository| repository.working_tree_changes().ok())
        .unwrap_or_default();
    let open_change = repository.and_then(|repository| change::load(repository).ok().flatten());
    let item_limit = (budget.unwrap_or(DEFAULT_BUDGET) / 90).max(MIN_LIST_ITEMS);

    let planning = live_planning(&snapshot.ledger);
    let focus = linked
        .as_ref()
        .map(|commit| focus_records(&snapshot.ledger, &commit.work))
        .unwrap_or_default();
    let related = related_records(&snapshot.ledger, &focus, 3);
    let attention = attention_records(&snapshot.ledger, &model, &queue);

    let planning_omitted = planning.len().saturating_sub(item_limit);
    let related_limit = item_limit
        .saturating_sub(planning.len().min(item_limit) / 2)
        .max(2);
    let related_omitted = related.len().saturating_sub(related_limit);
    // Every other list here is bounded by `item_limit`, and the text view reports the
    // overlay as counts alone; the JSON view enumerated it in full, so an uncommitted
    // workspace could spend the whole budget describing its own dirt and crowd out the
    // records the caller asked for.
    let overlay_omitted = snapshot.overlay.added.len().saturating_sub(item_limit)
        + snapshot.overlay.revised.len().saturating_sub(item_limit)
        + snapshot
            .overlay
            .transitions
            .len()
            .saturating_sub(item_limit);

    let namespaces: Vec<String> = snapshot
        .ledger
        .project
        .namespaces
        .iter()
        .map(ToString::to_string)
        .collect();
    let mut text = String::from("AKR SESSION HEAD\n");
    text.push_str(&format!(
        "snapshot    {} — {}\nledger      {} ({})\n",
        latest.as_ref().map_or_else(
            || "(no Git commit)".to_owned(),
            |latest| format!("{} {}", short(latest.commit.as_str()), latest.subject)
        ),
        snapshot.origin,
        snapshot.source_graph,
        match snapshot.origin {
            "working_tree" => "validated overlay",
            "working_tree_no_git" => "validated ledger",
            _ => "validated HEAD fallback",
        }
    ));
    if !namespaces.is_empty() {
        text.push_str(&format!("namespaces  {}\n", namespaces.join(", ")));
    }
    if snapshot.excluded_diagnostics > 0 {
        text.push_str(&format!(
            "warning     excluded invalid working ledger ({} errors)\n",
            snapshot.excluded_diagnostics
        ));
    }
    if let Some(linked) = &linked {
        text.push_str(&format!(
            "recent AKR  {} — {}\n",
            short(linked.commit.as_str()),
            linked.subject
        ));
    } else {
        text.push_str("recent AKR  (no reachable AKR-Work commit)\n");
    }
    if !focus.is_empty() {
        text.push_str("focus\n");
        for record in &focus {
            text.push_str(&record_line(record, "  "));
        }
    }
    text.push_str(&format!(
        "outstanding  {} live planning heads\n",
        planning.len()
    ));
    for record in planning.iter().take(item_limit) {
        text.push_str(&record_line(record, "  "));
    }
    if planning_omitted > 0 {
        text.push_str(&format!(
            "  … {planning_omitted} more; use knowledge.search by state\n"
        ));
    }
    if !attention.is_empty() {
        text.push_str("attention\n");
        for item in attention.iter().take(item_limit) {
            text.push_str(&format!("  {item}\n"));
        }
    }
    if !snapshot.overlay.added.is_empty()
        || !snapshot.overlay.revised.is_empty()
        || !snapshot.overlay.transitions.is_empty()
        || !changed_paths.is_empty()
        || open_change.is_some()
    {
        text.push_str(&format!(
            "overlay      {} added, {} revised, {} transitions, {} dirty paths\n",
            snapshot.overlay.added.len(),
            snapshot.overlay.revised.len(),
            snapshot.overlay.transitions.len(),
            changed_paths.len()
        ));
        if let Some(change) = &open_change {
            text.push_str(&format!(
                "change       {} — {} ({})\n",
                change.id,
                change.summary,
                if change.prepared_tree.is_some() {
                    "prepared"
                } else {
                    "open"
                }
            ));
        }
    }

    // Scratch, at orientation. `knowledge.start` is the one call an agent always makes
    // before working, so it is where "this directory persists and it is yours" has to be
    // said — a line in `akr check` alone is read at the end, if at all (D-036).
    let scratch = akr_core::scratch::scan(&session.root, std::time::SystemTime::now());
    let scratch_prunable: Vec<_> = scratch.prunable(SCRATCH_AGE_DAYS).collect();
    if !scratch_prunable.is_empty() {
        text.push_str(&format!(
            "scratch      {} in {} prunable {} — `akr scratch prune`, or keep what the \
             next session needs\n",
            akr_core::scratch::human_bytes(scratch_prunable.iter().map(|entry| entry.bytes).sum()),
            scratch_prunable.len(),
            if scratch_prunable.len() == 1 {
                "entry"
            } else {
                "entries"
            }
        ));
    }
    text.push('\n');

    let value = Value::object(vec![
        (
            "snapshot",
            Value::object(vec![
                ("origin", Value::string(snapshot.origin)),
                (
                    "commit",
                    latest
                        .as_ref()
                        .map_or(Value::Null, |latest| Value::string(latest.commit.as_str())),
                ),
                (
                    "subject",
                    latest
                        .as_ref()
                        .map_or(Value::Null, |latest| Value::string(latest.subject.clone())),
                ),
                (
                    "ledger_revision",
                    Value::string(snapshot.source_graph.clone()),
                ),
                (
                    "excluded_working_diagnostics",
                    Value::integer(snapshot.excluded_diagnostics as i64),
                ),
            ]),
        ),
        (
            "namespaces",
            Value::array(namespaces.iter().cloned().map(Value::string).collect()),
        ),
        (
            "recent_focus",
            linked.as_ref().map_or(Value::Null, |linked| {
                Value::object(vec![
                    ("commit", Value::string(linked.commit.as_str())),
                    ("subject", Value::string(linked.subject.clone())),
                    (
                        "work",
                        Value::array(focus.iter().map(record_json).collect()),
                    ),
                    (
                        "related",
                        Value::array(
                            related
                                .iter()
                                .take(related_limit)
                                .map(record_json)
                                .collect(),
                        ),
                    ),
                ])
            }),
        ),
        (
            "outstanding",
            Value::object(vec![
                ("counts", state_counts_json(&planning)),
                (
                    "branches",
                    Value::array(planning.iter().take(item_limit).map(record_json).collect()),
                ),
            ]),
        ),
        (
            "attention",
            Value::array(
                attention
                    .iter()
                    .take(item_limit)
                    .cloned()
                    .map(Value::string)
                    .collect(),
            ),
        ),
        (
            "overlay",
            Value::object(vec![
                ("added", refs_json(&snapshot.overlay.added, item_limit)),
                ("revised", refs_json(&snapshot.overlay.revised, item_limit)),
                (
                    "transitions",
                    Value::array(
                        snapshot
                            .overlay
                            .transitions
                            .iter()
                            .take(item_limit)
                            .map(|transition| {
                                Value::object(vec![
                                    ("ref", Value::string(format!("@{}", transition.id))),
                                    (
                                        "from",
                                        transition.from.map_or(Value::Null, |state| {
                                            Value::string(state.name())
                                        }),
                                    ),
                                    ("to", Value::string(transition.to.name())),
                                ])
                            })
                            .collect(),
                    ),
                ),
                ("dirty_paths", Value::integer(changed_paths.len() as i64)),
                (
                    "change",
                    open_change.as_ref().map_or(Value::Null, |change| {
                        Value::object(vec![
                            ("id", Value::string(change.id.clone())),
                            ("summary", Value::string(change.summary.clone())),
                            ("prepared", Value::bool(change.prepared_tree.is_some())),
                        ])
                    }),
                ),
            ]),
        ),
        (
            "omitted",
            Value::object(vec![
                ("planning", Value::integer(planning_omitted as i64)),
                ("related", Value::integer(related_omitted as i64)),
                ("overlay", Value::integer(overlay_omitted as i64)),
            ]),
        ),
    ]);

    Ok(Handoff {
        text,
        value,
        fallback_ledger: (snapshot.origin == "head_fallback").then_some(snapshot.ledger),
    })
}

fn ledger_from_files(files: &[(String, String)]) -> (Ledger, Vec<Diagnostic>, String) {
    let mut parsed = Vec::new();
    let mut diagnostics = Vec::new();
    let mut hashed = Vec::new();
    for (index, (path, text)) in files.iter().enumerate() {
        let file = FileId(u32::try_from(index).unwrap_or(0));
        let syntax = akr_core::syntax::parse(text, file);
        diagnostics.extend(syntax.diagnostics);
        if let Some(tree) = syntax.file {
            parsed.push((path.clone(), tree));
        }
        hashed.push((
            path.clone(),
            akr_core::hash::source_file_hash(text.as_bytes()),
        ));
    }
    let (ledger, lowered) = akr_core::syntax::lower_all(&parsed);
    diagnostics.extend(lowered);
    diagnostics.extend(akr_core::validate::validate_all(&ledger));
    let graph =
        akr_core::hash::source_graph_hash(hashed.iter().map(|(path, hash)| (path.as_str(), hash)))
            .to_string();
    (ledger, diagnostics, graph)
}

fn live_planning(ledger: &Ledger) -> Vec<&Record> {
    let mut records: Vec<&Record> = ledger
        .records()
        .iter()
        .filter(|record| record.kind.class() == Class::Planning)
        .filter(|record| {
            ledger
                .head(&record.id.key)
                .is_ok_and(|head| head.id == record.id)
        })
        .filter(|record| record.is_live())
        .collect();
    records.sort_by(|left, right| {
        state_rank(left.state)
            .cmp(&state_rank(right.state))
            .then_with(|| left.id.cmp(&right.id))
    });
    records
}

fn state_rank(state: State) -> u8 {
    match state {
        State::Active => 0,
        State::Blocked => 1,
        State::Ready => 2,
        State::Proposed => 3,
        _ => 4,
    }
}

fn focus_records<'a>(ledger: &'a Ledger, references: &[String]) -> Vec<&'a Record> {
    let mut out = Vec::new();
    for reference in references {
        let key = reference
            .trim()
            .trim_start_matches('@')
            .split_once('/')
            .map_or_else(|| reference.trim().trim_start_matches('@'), |(key, _)| key);
        let Ok(key) = LogicalKey::parse(key) else {
            continue;
        };
        if let Ok(record) = ledger.head(&key)
            && !out
                .iter()
                .any(|existing: &&Record| existing.id == record.id)
        {
            out.push(record);
        }
    }
    out.sort_by_key(|record| record.id.clone());
    out
}

fn related_records<'a>(ledger: &'a Ledger, focus: &[&Record], depth: usize) -> Vec<&'a Record> {
    let focus_ids: BTreeSet<RevisionId> = focus.iter().map(|record| record.id.clone()).collect();
    let mut seen = focus_ids.clone();
    let mut queue: VecDeque<(RevisionId, usize)> =
        focus_ids.into_iter().map(|id| (id, 0)).collect();
    while let Some((id, at)) = queue.pop_front() {
        if at >= depth {
            continue;
        }
        let Some(record) = ledger.get(&id) else {
            continue;
        };
        let mut neighbours = Vec::new();
        for (_, reference) in record.references() {
            if let Ok(Some(target)) = ledger.resolve(reference) {
                neighbours.push(target.id.clone());
            }
        }
        for candidate in ledger.records() {
            if candidate.references().iter().any(|(_, reference)| {
                ledger
                    .resolve(reference)
                    .ok()
                    .flatten()
                    .is_some_and(|target| target.id == id)
            }) {
                neighbours.push(candidate.id.clone());
            }
        }
        neighbours.sort();
        neighbours.dedup();
        for neighbour in neighbours {
            if seen.insert(neighbour.clone()) {
                queue.push_back((neighbour, at + 1));
            }
        }
    }
    let mut records: Vec<&Record> = seen
        .into_iter()
        .filter(|id| !focus.iter().any(|record| record.id == *id))
        .filter_map(|id| ledger.get(&id))
        .collect();
    records.sort_by_key(|record| record.id.clone());
    records
}

fn attention_records(
    ledger: &Ledger,
    model: &ResolvedModel<'_>,
    queue: &ReviewQueue,
) -> Vec<String> {
    let mut out = Vec::new();
    for record in live_planning(ledger) {
        if record.state == State::Blocked {
            out.push(format!("blocked @{} — {}", record.id, record.title));
        }
    }
    for record in ledger.records() {
        if record.kind == Kind::Question
            && ledger
                .head(&record.id.key)
                .is_ok_and(|head| head.id == record.id)
            && matches!(record.state, State::Open | State::Deferred)
        {
            out.push(format!(
                "{} @{} — {}",
                record.state, record.id, record.title
            ));
        }
    }
    for verdict in &model.acceptance {
        if !verdict.verdict.is_satisfied()
            && ledger.get(&verdict.owner).is_some_and(Record::is_live)
        {
            out.push(format!(
                "unsatisfied @{}#{} — {}",
                verdict.owner,
                verdict.check,
                verdict_name(&verdict.verdict)
            ));
        }
    }
    out.extend(
        queue
            .stale
            .iter()
            .map(|entry| format!("stale @{}", entry.id)),
    );
    out.extend(
        queue
            .at_risk
            .iter()
            .map(|entry| format!("at-risk @{} depth {}", entry.id, entry.depth)),
    );
    out.sort();
    out.dedup();
    out
}

fn verdict_name(verdict: &Verdict) -> &'static str {
    match verdict {
        Verdict::Satisfied { .. } => "satisfied",
        Verdict::NoEvidence => "no evidence",
        Verdict::Unresolved => "unresolved evidence",
        Verdict::Failing { .. } => "failing evidence",
        Verdict::TooOld { .. } => "evidence too old",
    }
}

fn record_line(record: &Record, prefix: &str) -> String {
    format!(
        "{prefix}{:<8} @{} — {}\n",
        record.state.name(),
        record.id,
        record.title
    )
}

fn record_json(record: &&Record) -> Value {
    let parent = record
        .targets(Relation::PartOf)
        .first()
        .map_or(Value::Null, |reference| {
            Value::string(format!("@{reference}"))
        });
    Value::object(vec![
        ("ref", Value::string(format!("@{}", record.id))),
        ("kind", Value::string(record.kind.name())),
        ("state", Value::string(record.state.name())),
        ("title", Value::string(record.title.clone())),
        ("parent", parent),
    ])
}

fn state_counts_json(records: &[&Record]) -> Value {
    let mut counts: BTreeMap<&str, i64> = BTreeMap::new();
    for record in records {
        *counts.entry(record.state.name()).or_default() += 1;
    }
    Value::Object(
        counts
            .into_iter()
            .map(|(state, count)| (state.to_owned(), Value::integer(count)))
            .collect(),
    )
}

fn refs_json(ids: &[RevisionId], limit: usize) -> Value {
    Value::array(
        ids.iter()
            .take(limit)
            .map(|id| Value::string(format!("@{id}")))
            .collect(),
    )
}

fn short(hash: &str) -> &str {
    hash.get(..8).unwrap_or(hash)
}

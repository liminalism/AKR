//! Owned, immutable data for human-facing AKR review clients.
//!
//! The command layer usually renders one question at a time. A desktop reviewer needs
//! the opposite shape: load the workspace once, then navigate records, history, edges,
//! freshness and Git provenance without issuing a tool call per row. This module is that
//! boundary. It deliberately exposes ordinary owned Rust values rather than `Session`, a
//! borrowed [`akr_core::resolve::ResolvedModel`], MCP JSON, or the private SQLite cache.

use crate::args::{Format, Global, Profile};
use crate::session::{EnvError, Session};
use akr_core::diagnostics::Severity;
use akr_core::freshness::StaleCause;
use akr_core::model::{ContentValue, Record, Relation, SourceKind};
use akr_core::resolve::Verdict;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A coarse stage of a snapshot load.
///
/// A load is one opaque call that can take seconds — most of it waiting on git — so a
/// client with a window needs to be able to say what it is waiting for. These are stage
/// boundaries, not a percentage: the phases are ordered and each is reported once, which
/// is enough for a determinate progress bar and honest about the fact that the cost
/// inside a phase is not knowable in advance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReviewPhase {
    /// Reading and parsing the workspace.
    Opening,
    /// Resolving revisions to their heads.
    Resolving,
    /// Asking git which records are stale or at risk.
    Freshness,
    /// Projecting records into owned review values.
    Projecting,
    /// Collecting diagnostics and counts.
    Summarising,
}

impl ReviewPhase {
    /// Every phase, in the order a load reports them.
    pub const ALL: [Self; 5] = [
        Self::Opening,
        Self::Resolving,
        Self::Freshness,
        Self::Projecting,
        Self::Summarising,
    ];

    /// A short present-tense description, for a status line.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Opening => "reading ledger",
            Self::Resolving => "resolving revisions",
            Self::Freshness => "checking git freshness",
            Self::Projecting => "projecting records",
            Self::Summarising => "collecting diagnostics",
        }
    }

    /// Which step this is, one-based.
    #[must_use]
    pub fn step(self) -> usize {
        Self::ALL
            .iter()
            .position(|phase| *phase == self)
            .unwrap_or(0)
            + 1
    }
}

/// Inputs which make a snapshot deterministic.
#[derive(Debug, Clone, Default)]
pub struct ReviewOptions {
    /// Resolve Git facts at this commit rather than HEAD.
    pub at: Option<akr_core::model::Commit>,
    /// Compare `review_after` against this date rather than the current date.
    pub today: Option<akr_core::model::Date>,
}

/// One complete workspace generation for a review client.
#[derive(Debug, Clone)]
pub struct ReviewSnapshot {
    /// Repository/workspace root.
    pub workspace: PathBuf,
    /// Project name from `project.akr`.
    pub project: String,
    /// Exact hash of the loaded AKR source bytes.
    pub source_graph: String,
    /// Git history boundary, where available.
    pub head: Option<String>,
    /// Declared namespaces, in canonical order.
    pub namespaces: Vec<String>,
    /// Resolved head records, ordered by key.
    pub records: Vec<ReviewRecord>,
    /// Parse, resolve, validation and Git-input diagnostics.
    pub diagnostics: Vec<ReviewDiagnostic>,
    /// Review-queue summary.
    pub counts: ReviewCounts,
}

/// Counts used by the review dashboard.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReviewCounts {
    /// Resolved record heads.
    pub records: usize,
    /// All revisions, including history.
    pub revisions: usize,
    /// Stale empirical heads.
    pub stale: usize,
    /// Live records whose support chain reaches stale knowledge.
    pub at_risk: usize,
    /// Diagnostics of effective severity error or warning.
    pub diagnostics: usize,
    /// Live planning heads.
    pub live_planning: usize,
    /// Live questions.
    pub open_questions: usize,
}

/// One resolved record head and everything the inspector needs.
#[derive(Debug, Clone)]
pub struct ReviewRecord {
    /// Stable `key/revision` identifier.
    pub id: String,
    /// Logical key without revision.
    pub key: String,
    /// First key segment.
    pub namespace: String,
    /// Revision number.
    pub revision: u32,
    /// Human title.
    pub title: String,
    /// Record kind.
    pub kind: String,
    /// Record class.
    pub class: String,
    /// Lifecycle state.
    pub state: String,
    /// Canonical source text for this head, when parsing supplied it.
    pub body: String,
    /// Kind-specific content slots.
    pub slots: Vec<ReviewField>,
    /// Addressable claims.
    pub claims: Vec<ReviewClaim>,
    /// Acceptance checks and resolved verdicts.
    pub acceptance: Vec<ReviewCheck>,
    /// Both directions of the typed relation graph.
    pub relations: Vec<ReviewRelation>,
    /// `part_of` parent key, used for the planning tree.
    pub parent: Option<String>,
    /// Milestone/track keys for which this work is plan of record.
    pub plan_for: Vec<String>,
    /// Freshness or propagation detail.
    pub freshness: ReviewFreshness,
    /// Source/provenance locators.
    pub provenance: Vec<ReviewProvenance>,
    /// Every revision, newest first.
    pub history: Vec<ReviewRevision>,
    /// Definitional last-change commit.
    pub defined_at: Option<String>,
    /// Evidence/observation commit carried by the record.
    pub observed_at: Option<String>,
}

/// A name/value content slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewField {
    /// Slot name.
    pub name: String,
    /// Human-readable value.
    pub value: String,
}

/// One addressable claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewClaim {
    /// Claim anchor.
    pub anchor: String,
    /// Claim prose.
    pub text: String,
    /// Supporting references.
    pub supported_by: Vec<String>,
}

/// One acceptance check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewCheck {
    /// Check identifier.
    pub id: String,
    /// Observable outcome.
    pub statement: String,
    /// `manual`, `command`, or `observation`.
    pub method: String,
    /// Exact command when present.
    pub command: Option<String>,
    /// Whether current evidence satisfies the check.
    pub satisfied: bool,
    /// Actionable verdict summary.
    pub verdict: String,
}

/// One inbound or outbound typed edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewRelation {
    /// Relation name.
    pub relation: String,
    /// `inbound` or `outbound`.
    pub direction: String,
    /// Resolved stable target/source revision when available.
    pub record: String,
}

/// Human review status for one head.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReviewFreshness {
    /// `current`, `stale`, or `at_risk`.
    pub status: String,
    /// Cause suitable for the inspector.
    pub cause: Option<String>,
    /// Related commit, where the cause names one.
    pub commit: Option<String>,
    /// Related path, where the cause names one.
    pub path: Option<String>,
    /// Propagation route for at-risk records.
    pub chain: Vec<String>,
}

/// Provenance locator from a source block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewProvenance {
    /// `legacy`, `external`, or `internal`.
    pub kind: String,
    /// Optional semantic role.
    pub role: Option<String>,
    /// Repository path.
    pub path: Option<String>,
    /// External URL.
    pub url: Option<String>,
    /// Registered source document identifier.
    pub document: Option<String>,
    /// Exact byte range, rendered as `start..end`.
    pub range: Option<String>,
    /// What the project retained from the source.
    pub use_note: Option<String>,
}

/// One entry in revision history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewRevision {
    /// Stable revision identifier.
    pub id: String,
    /// Lifecycle state.
    pub state: String,
    /// Title at that revision.
    pub title: String,
    /// Definitional last-change commit, when known.
    pub defined_at: Option<String>,
}

/// A diagnostic detached from source-map lifetimes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewDiagnostic {
    /// Stable AKR code.
    pub code: String,
    /// `error` or `warning`.
    pub severity: String,
    /// One-line explanation.
    pub message: String,
    /// Suggested recovery.
    pub help: Option<String>,
}

/// A snapshot load failure.
#[derive(Debug, Clone)]
pub struct ReviewLoadError(pub EnvError);

impl std::fmt::Display for ReviewLoadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for ReviewLoadError {}

impl ReviewSnapshot {
    /// Loads one exact, immutable workspace generation.
    ///
    /// # Errors
    /// [`ReviewLoadError`] when the workspace or required Git history is unavailable.
    pub fn load(root: &Path, options: ReviewOptions) -> Result<Self, ReviewLoadError> {
        Self::load_reporting(root, options, &|_| {})
    }

    /// Loads a snapshot, reporting each phase as it begins.
    ///
    /// `progress` is called on the loading thread, once per [`ReviewPhase`], in order. It
    /// must not block: a client that draws from it should hand the phase to its event loop
    /// rather than paint from here.
    ///
    /// # Errors
    /// [`ReviewLoadError`] when the workspace or required Git history is unavailable.
    pub fn load_reporting(
        root: &Path,
        options: ReviewOptions,
        progress: &(dyn Fn(ReviewPhase) + Sync),
    ) -> Result<Self, ReviewLoadError> {
        progress(ReviewPhase::Opening);
        let global = Global {
            dir: root.to_path_buf(),
            profile: Profile::Strict,
            format: Format::Json,
            at: options.at,
            today: options.today,
            ..Global::default()
        };
        let mut session = Session::open(&global).map_err(ReviewLoadError)?;
        session.attach_lock();
        progress(ReviewPhase::Resolving);
        let model = session.resolve();
        progress(ReviewPhase::Freshness);
        let queue = session.checked_review_queue();
        progress(ReviewPhase::Projecting);

        let stale: BTreeMap<_, _> = queue.stale.iter().map(|item| (&item.id, item)).collect();
        let at_risk: BTreeMap<_, _> = queue.at_risk.iter().map(|item| (&item.id, item)).collect();
        let mut inbound: BTreeMap<String, Vec<ReviewRelation>> = BTreeMap::new();
        for source in session.ledger.records() {
            if !model.is_head(&source.id) {
                continue;
            }
            for (relation, targets) in &source.relations {
                for target in targets {
                    if let Ok(Some(record)) = session.ledger.resolve(target) {
                        inbound
                            .entry(record.id.to_string())
                            .or_default()
                            .push(ReviewRelation {
                                relation: relation.name().to_owned(),
                                direction: "inbound".to_owned(),
                                record: source.id.to_string(),
                            });
                    }
                }
            }
        }

        let mut records = Vec::new();
        for record in session.ledger.records() {
            if !model.is_head(&record.id) {
                continue;
            }
            records.push(project_record(
                record,
                &session,
                &model,
                stale.get(&record.id).copied(),
                at_risk.get(&record.id).copied(),
                inbound.remove(&record.id.to_string()).unwrap_or_default(),
            ));
        }
        records.sort_by(|left, right| left.key.cmp(&right.key));

        progress(ReviewPhase::Summarising);
        let mut diagnostics = session.diagnostics(&model);
        diagnostics.extend(queue.diagnostics.clone());
        diagnostics.sort_by_key(akr_core::diagnostics::Diagnostic::sort_key);
        let diagnostics: Vec<_> = diagnostics
            .into_iter()
            .map(|diagnostic| ReviewDiagnostic {
                code: diagnostic.code.as_str().to_owned(),
                severity: match diagnostic.severity {
                    Severity::Error => "error",
                    Severity::Warning => "warning",
                }
                .to_owned(),
                message: diagnostic.message,
                help: diagnostic.help,
            })
            .collect();

        let revisions = session.ledger.records().len();
        let counts = ReviewCounts {
            records: records.len(),
            revisions,
            stale: queue.stale.len(),
            at_risk: queue.at_risk.len(),
            diagnostics: diagnostics.len(),
            live_planning: session
                .ledger
                .records()
                .iter()
                .filter(|record| model.is_head(&record.id))
                .filter(|record| record.kind.class() == akr_core::model::Class::Planning)
                .filter(|record| record.is_live())
                .count(),
            open_questions: session
                .ledger
                .records()
                .iter()
                .filter(|record| model.is_head(&record.id))
                .filter(|record| record.kind == akr_core::model::Kind::Question)
                .filter(|record| record.is_live())
                .count(),
        };
        Ok(Self {
            workspace: session.root.clone(),
            project: session.ledger.project.name.clone(),
            source_graph: session.source_graph(),
            head: session.commit.as_ref().map(ToString::to_string),
            namespaces: session
                .ledger
                .project
                .namespaces
                .iter()
                .map(ToString::to_string)
                .collect(),
            records,
            diagnostics,
            counts,
        })
    }
}

fn project_record(
    record: &Record,
    session: &Session,
    model: &akr_core::resolve::ResolvedModel<'_>,
    stale: Option<&akr_core::freshness::Stale>,
    at_risk: Option<&akr_core::graph::AtRisk>,
    mut relations: Vec<ReviewRelation>,
) -> ReviewRecord {
    for (relation, targets) in &record.relations {
        for target in targets {
            let resolved = session
                .ledger
                .resolve(target)
                .ok()
                .flatten()
                .map_or_else(|| target.to_string(), |target| target.id.to_string());
            relations.push(ReviewRelation {
                relation: relation.name().to_owned(),
                direction: "outbound".to_owned(),
                record: resolved,
            });
        }
    }
    relations.sort_by(|left, right| {
        (&left.direction, &left.relation, &left.record).cmp(&(
            &right.direction,
            &right.relation,
            &right.record,
        ))
    });

    let checks = record
        .acceptance
        .as_ref()
        .map(|acceptance| {
            acceptance
                .checks
                .iter()
                .map(|check| {
                    let verdict = model
                        .checks_of(&record.id)
                        .into_iter()
                        .find(|verdict| verdict.check == check.id)
                        .map(|verdict| &verdict.verdict);
                    ReviewCheck {
                        id: check.id.to_string(),
                        statement: check.statement.clone(),
                        method: check.method.name().to_owned(),
                        command: check.command.clone(),
                        satisfied: verdict.is_some_and(Verdict::is_satisfied),
                        verdict: verdict.map_or_else(|| "unresolved".to_owned(), verdict_text),
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let freshness = if let Some(stale) = stale {
        match &stale.cause {
            StaleCause::Watch { glob, commit, path } => ReviewFreshness {
                status: "stale".to_owned(),
                cause: Some(format!("watched path matched {glob}")),
                commit: Some(commit.to_string()),
                path: Some(path.clone()),
                chain: Vec::new(),
            },
            StaleCause::ReviewAfter { date } => ReviewFreshness {
                status: "stale".to_owned(),
                cause: Some(format!("review date {date} passed")),
                ..ReviewFreshness::default()
            },
        }
    } else if let Some(at_risk) = at_risk {
        ReviewFreshness {
            status: "at_risk".to_owned(),
            cause: Some(format!(
                "{} hop(s) via {}",
                at_risk.depth,
                at_risk.via.name()
            )),
            chain: at_risk.path.iter().map(ToString::to_string).collect(),
            ..ReviewFreshness::default()
        }
    } else {
        ReviewFreshness {
            status: "current".to_owned(),
            ..ReviewFreshness::default()
        }
    };

    let parent = record
        .targets(Relation::PartOf)
        .first()
        .map(|target| target.key.to_string());
    let plan_for = record
        .targets(Relation::PlanOfRecord)
        .iter()
        .map(|target| target.key.to_string())
        .collect();
    let history = session
        .ledger
        .revisions_of(&record.id.key)
        .into_iter()
        .rev()
        .map(|revision| ReviewRevision {
            id: revision.id.to_string(),
            state: revision.state.name().to_owned(),
            title: revision.title.clone(),
            defined_at: session
                .ledger
                .facts
                .last_change
                .get(&revision.id)
                .map(ToString::to_string),
        })
        .collect();

    ReviewRecord {
        id: record.id.to_string(),
        key: record.id.key.to_string(),
        namespace: record
            .id
            .key
            .to_string()
            .split('.')
            .next()
            .unwrap_or_default()
            .to_owned(),
        revision: record.id.revision,
        title: record.title.clone(),
        kind: record.kind.name().to_owned(),
        class: record.kind.class().name().to_owned(),
        state: record.state.name().to_owned(),
        body: session
            .inputs
            .canonical_text
            .get(&record.id)
            .cloned()
            .unwrap_or_default(),
        slots: record
            .content
            .iter()
            .map(|(name, value)| ReviewField {
                name: name.name().to_owned(),
                value: content_text(value),
            })
            .collect(),
        claims: record
            .claims
            .iter()
            .map(|claim| ReviewClaim {
                anchor: claim.anchor.to_string(),
                text: claim.text.clone(),
                supported_by: claim.supported_by.iter().map(ToString::to_string).collect(),
            })
            .collect(),
        acceptance: checks,
        relations,
        parent,
        plan_for,
        freshness,
        provenance: record
            .sources
            .iter()
            .map(|source| ReviewProvenance {
                kind: match source.kind {
                    SourceKind::Legacy => "legacy",
                    SourceKind::External => "external",
                    SourceKind::Internal => "internal",
                }
                .to_owned(),
                role: source.role.map(|role| role.as_str().to_owned()),
                path: source.path.clone(),
                url: source.url.clone(),
                document: source.document.clone(),
                range: source
                    .range
                    .as_ref()
                    .map(|range| format!("{}..{}", range.start_byte, range.end_byte)),
                use_note: source.use_note.clone(),
            })
            .collect(),
        history,
        defined_at: session
            .ledger
            .facts
            .last_change
            .get(&record.id)
            .map(ToString::to_string),
        observed_at: akr_core::freshness::observed_at(record).map(ToString::to_string),
    }
}

fn content_text(value: &ContentValue) -> String {
    match value {
        ContentValue::Text(value) | ContentValue::Prose(value) => value.clone(),
        ContentValue::Date(value) => value.to_string(),
        ContentValue::Commit(value) => value.to_string(),
        ContentValue::Enum(value) => value.to_string(),
        ContentValue::Strings(values) => values.join(", "),
        ContentValue::Globs(values) => values
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", "),
        ContentValue::Refs(values) => values
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", "),
    }
}

fn verdict_text(verdict: &Verdict) -> String {
    match verdict {
        Verdict::Satisfied { by, .. } => format!("satisfied by {by}"),
        Verdict::NoEvidence => "no evidence".to_owned(),
        Verdict::Unresolved => "evidence does not resolve".to_owned(),
        Verdict::Failing { by, .. } => format!("failing evidence {by}"),
        Verdict::TooOld {
            by, last_change, ..
        } => {
            format!("evidence {by} predates {last_change}")
        }
    }
}

//! Deterministic context assembly: what an agent needs to know before it touches
//! something.
//!
//! `docs/09-context-assembly.md` is normative. This module implements §3 (the request),
//! §4 (the eleven ordered steps), §5 (exclusions), §6 (budgeting) and §7 (the bundle
//! format).
//!
//! # The principle
//!
//! > **Membership in a context bundle is computed from the graph. Search reorders what
//! > the graph already authorised, and nothing else.**
//!
//! Nothing in this module consults a ranker, an index, or a model. A record enters a
//! bundle because it is the goal, because a named relation reaches it, because its
//! declared scope overlaps the request, or because it contradicts something already in —
//! and for no other reason. Two agents asking the same question at the same commit get
//! the same bundle, byte for byte, which is what makes divergent behaviour between two
//! sessions explicable.

mod render;

pub use render::{render_json, render_text};

use crate::freshness::{observed_at, watches};
use crate::graph::sorted_records;
use crate::model::{
    Class, ContentSlot, ContentValue, Glob, Kind, Ledger, Record, Relation, RevisionId, ScopeTerm,
    scopes_overlap,
};
use crate::render::Freshness;
use crate::resolve::{CheckVerdict, ResolvedModel};
use std::collections::{BTreeMap, BTreeSet};

/// How far step 6 follows `depends_on` and `implements` by default.
pub const DEFAULT_DEPTH: usize = 3;

/// Why a bundle could not be assembled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextError {
    /// `--goal` does not resolve to a record (`AKR-X001`).
    GoalUnresolved(String),
    /// The goal is in a terminal state (`AKR-X002`).
    GoalTerminal {
        /// The goal.
        id: RevisionId,
        /// Its state.
        state: String,
    },
    /// The goal's kind cannot anchor a bundle (`AKR-X003`).
    GoalKind {
        /// The goal.
        id: RevisionId,
        /// Its kind.
        kind: Kind,
    },
    /// A pinned goal does not name the current head (`AKR-X004`).
    GoalRevision {
        /// The requested revision.
        requested: RevisionId,
        /// The current head.
        head: RevisionId,
    },
    /// A context goal may not select a claim or check anchor (`AKR-X005`).
    GoalAnchor(String),
    /// A `--paths` glob is not in the D-008 subset (`AKR-X011`).
    BadPath {
        /// The glob.
        glob: String,
        /// Why it was rejected.
        reason: String,
    },
    /// The budget cannot hold the mandatory sections (`AKR-X021`).
    BudgetTooSmall {
        /// What was asked for.
        budget: usize,
        /// What the mandatory sections need.
        required: usize,
    },
}

impl std::fmt::Display for ContextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GoalUnresolved(text) => write!(f, "--goal {text} does not resolve to a record"),
            Self::GoalTerminal { id, state } => {
                write!(f, "--goal {id} is in terminal state {state}")
            }
            Self::GoalKind { id, kind } => write!(
                f,
                "--goal {} is a {kind}; a bundle anchors on a milestone, work or track record",
                id.key
            ),
            Self::GoalRevision { requested, head } => write!(
                f,
                "--goal {requested} is not the current head, which is {head}"
            ),
            Self::GoalAnchor(goal) => write!(
                f,
                "--goal {goal} selects an anchor; a bundle anchors on a planning record"
            ),
            Self::BadPath { glob, reason } => write!(f, "--paths {glob}: {reason}"),
            Self::BudgetTooSmall { budget, required } => write!(
                f,
                "budget of {budget} tokens cannot hold the mandatory sections ({required} tokens)"
            ),
        }
    }
}

impl std::error::Error for ContextError {}

/// A bundle request (`docs/09-context-assembly.md` §3).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Request {
    /// The anchor: a live `milestone`, `work` or `track` record.
    pub goal: String,
    /// The files the caller expects to touch.
    pub paths: Vec<Glob>,
    /// An approximate token ceiling.
    pub budget: Option<usize>,
    /// How far step 6 follows dependencies.
    pub depth: usize,
}

impl Request {
    /// A request for one goal, with the default dependency depth.
    #[must_use]
    pub fn new(goal: impl Into<String>) -> Self {
        Self {
            goal: goal.into(),
            paths: Vec::new(),
            budget: None,
            depth: DEFAULT_DEPTH,
        }
    }

    /// Adds a path filter.
    ///
    /// # Panics
    /// Never; the glob is validated when the bundle is assembled.
    #[must_use]
    pub fn path(mut self, glob: &str) -> Self {
        self.paths.push(Glob::new(glob));
        self
    }

    /// Sets the token budget.
    #[must_use]
    pub fn budget(mut self, budget: usize) -> Self {
        self.budget = Some(budget);
        self
    }

    /// The paths as scope terms, for the D-010 overlap test.
    fn scope(&self) -> Vec<ScopeTerm> {
        self.paths.iter().cloned().map(ScopeTerm::Path).collect()
    }
}

/// The eleven sections, in the order §4 fixes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Section {
    /// 1. The anchor.
    Goal,
    /// 2. What larger thing it serves.
    Milestone,
    /// 3. The work under it.
    WorkItems,
    /// 4. The authoritative plan.
    PlanOfRecord,
    /// 5. What governs the code in question.
    Normative,
    /// 6. What it rests on, and what is holding it.
    Dependencies,
    /// 7. What "done" means, and whether it is.
    Acceptance,
    /// 8. What has been found out.
    Observations,
    /// 9. What is not yet known.
    Questions,
    /// 10. What is disputed.
    Contradictions,
    /// 11. What should not be trusted without re-checking.
    Staleness,
}

impl Section {
    /// Every section, in bundle order.
    pub const ALL: &'static [Section] = &[
        Section::Goal,
        Section::Milestone,
        Section::WorkItems,
        Section::PlanOfRecord,
        Section::Normative,
        Section::Dependencies,
        Section::Acceptance,
        Section::Observations,
        Section::Questions,
        Section::Contradictions,
        Section::Staleness,
    ];

    /// The heading as the text form renders it.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Goal => "GOAL",
            Self::Milestone => "MILESTONE",
            Self::WorkItems => "WORK ITEMS",
            Self::PlanOfRecord => "PLAN OF RECORD",
            Self::Normative => "NORMATIVE (in scope)",
            Self::Dependencies => "DEPENDENCIES AND BLOCKERS",
            Self::Acceptance => "ACCEPTANCE",
            Self::Observations => "OBSERVATIONS",
            Self::Questions => "OPEN QUESTIONS",
            Self::Contradictions => "CONTRADICTIONS",
            Self::Staleness => "STALENESS",
        }
    }

    /// The identifier the JSON form uses.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Goal => "goal",
            Self::Milestone => "milestone",
            Self::WorkItems => "work-items",
            Self::PlanOfRecord => "plan-of-record",
            Self::Normative => "normative",
            Self::Dependencies => "dependencies",
            Self::Acceptance => "acceptance",
            Self::Observations => "observations",
            Self::Questions => "questions",
            Self::Contradictions => "contradictions",
            Self::Staleness => "staleness",
        }
    }

    /// The 1-based number the text form prints.
    #[must_use]
    pub fn number(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0) + 1
    }
}

/// Why a record entered a bundle. Carried so that every member can be justified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// A record, with the relation that reached it and the depth if any.
    Record {
        /// Which revision.
        id: RevisionId,
        /// How it got in.
        via: Option<Relation>,
        /// Propagation distance for step 6.
        depth: usize,
    },
}

impl Entry {
    /// The revision this entry names.
    #[must_use]
    pub const fn id(&self) -> &RevisionId {
        match self {
            Self::Record { id, .. } => id,
        }
    }
}

/// A declared contradiction, always surfaced (V-121).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Contradiction {
    /// The lexicographically smaller endpoint.
    pub left: RevisionId,
    /// The larger.
    pub right: RevisionId,
    /// Which side declared it.
    pub declared_by: RevisionId,
    /// Whether it is knowingly tolerated (D-023).
    pub acknowledged: bool,
}

/// How many records were left out, and why (§5).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Excluded {
    /// Non-head revisions of keys whose head is in the bundle or out of it.
    pub superseded: Vec<RevisionId>,
    /// Records under `.akr/archive/`.
    pub archived: Vec<RevisionId>,
    /// Records in a terminal state that are not archived.
    pub terminal: Vec<RevisionId>,
    /// Live records the request's scope does not reach.
    pub out_of_scope: Vec<RevisionId>,
}

/// An assembled bundle.
#[derive(Debug, Clone)]
pub struct Bundle {
    /// The goal.
    pub goal: RevisionId,
    /// The commit the model resolved against, if any.
    pub commit: Option<String>,
    /// The tool version.
    pub tool_version: String,
    /// The source-graph hash.
    pub source_graph: String,
    /// The request's path filters.
    pub paths: Vec<Glob>,
    /// Members, by section, each already in the section's sort order.
    pub sections: BTreeMap<Section, Vec<Entry>>,
    /// Acceptance verdicts for the selected planning records.
    pub acceptance: Vec<CheckVerdict>,
    /// Declared contradictions touching the bundle.
    pub contradictions: Vec<Contradiction>,
    /// What was left out.
    pub excluded: Excluded,
    /// Records whose prose the budget truncated (`AKR-X022`).
    pub truncated: Vec<RevisionId>,
    /// Records the budget withheld whole, by the section they were selected into, each
    /// list in that section's order. The selection reached them; only their entries were
    /// left out, and the bundle names every one so the reader can fetch it.
    pub withheld: BTreeMap<Section, Vec<RevisionId>>,
    /// The approximate token count of the delivered bundle, both forms
    /// ([`delivered_tokens`]).
    pub estimated_tokens: usize,
}

impl Bundle {
    /// One section's entries, or an empty slice.
    #[must_use]
    pub fn section(&self, section: Section) -> &[Entry] {
        self.sections.get(&section).map_or(&[], Vec::as_slice)
    }

    /// Every revision the bundle contains, sorted and deduplicated.
    #[must_use]
    pub fn members(&self) -> BTreeSet<&RevisionId> {
        self.sections.values().flatten().map(Entry::id).collect()
    }

    /// How many records the bundle holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.members().len()
    }

    /// Whether the bundle holds nothing. Never true in practice: the goal is always in.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.members().is_empty()
    }

    /// How many entries the budget withheld, across sections.
    #[must_use]
    pub fn withheld_count(&self) -> usize {
        self.withheld.values().map(Vec::len).sum()
    }
}

// -------------------------------------------------------------------------------------
// Assembly
// -------------------------------------------------------------------------------------

/// Assembles a bundle (`docs/09-context-assembly.md` §4).
///
/// # Errors
/// [`ContextError`] for an unresolvable, terminal or wrongly-kinded goal, a malformed
/// path filter, or a budget too small for the mandatory sections.
pub fn assemble(
    model: &ResolvedModel<'_>,
    freshness: &Freshness,
    request: &Request,
) -> Result<Bundle, ContextError> {
    let ledger = model.ledger();

    for glob in &request.paths {
        if let Err(reason) = crate::freshness::validate_glob(glob) {
            return Err(ContextError::BadPath {
                glob: glob.as_str().to_owned(),
                reason: reason.to_string(),
            });
        }
    }

    // -- step 1: the goal ------------------------------------------------------------
    let goal_ref = crate::model::Reference::parse(request.goal.trim_start_matches('@'))
        .map_err(|_| ContextError::GoalUnresolved(request.goal.clone()))?;
    if goal_ref.anchor.is_some() {
        return Err(ContextError::GoalAnchor(request.goal.clone()));
    }
    let goal = ledger
        .head(&goal_ref.key)
        .map_err(|_| ContextError::GoalUnresolved(request.goal.clone()))?;
    if let Some(revision) = goal_ref.revision
        && revision != goal.id.revision
    {
        return Err(ContextError::GoalRevision {
            requested: RevisionId::new(goal_ref.key, revision),
            head: goal.id.clone(),
        });
    }
    if !goal.is_live() {
        return Err(ContextError::GoalTerminal {
            id: goal.id.clone(),
            state: goal.state.name().to_owned(),
        });
    }
    if !matches!(goal.kind, Kind::Milestone | Kind::Work | Kind::Track) {
        return Err(ContextError::GoalKind {
            id: goal.id.clone(),
            kind: goal.kind,
        });
    }

    let mut sections: BTreeMap<Section, Vec<Entry>> = BTreeMap::new();
    let mut selected: BTreeSet<RevisionId> = BTreeSet::new();
    let push = |sections: &mut BTreeMap<Section, Vec<Entry>>,
                selected: &mut BTreeSet<RevisionId>,
                section: Section,
                id: &RevisionId,
                via: Option<Relation>,
                depth: usize| {
        if !selected.insert(id.clone()) {
            return false;
        }
        sections.entry(section).or_default().push(Entry::Record {
            id: id.clone(),
            via,
            depth,
        });
        true
    };

    push(
        &mut sections,
        &mut selected,
        Section::Goal,
        &goal.id,
        None,
        0,
    );

    // -- step 2: the containing milestone or track ------------------------------------
    let ancestors = part_of_ancestors(model, &goal.id);
    for (distance, id) in &ancestors {
        let Some(record) = ledger.get(id) else {
            continue;
        };
        if matches!(record.kind, Kind::Milestone | Kind::Track) && record.is_live() {
            push(
                &mut sections,
                &mut selected,
                Section::Milestone,
                id,
                Some(Relation::PartOf),
                *distance,
            );
        }
    }

    // -- step 4 first: the plan of record decides part of step 3 ----------------------
    let plan_targets: Vec<RevisionId> = std::iter::once(goal.id.clone())
        .chain(ancestors.iter().map(|(_, id)| id.clone()))
        .collect();
    let plan = plan_of_record(model, &plan_targets);

    // -- step 3: work items ------------------------------------------------------------
    let mut work: Vec<&Record> = Vec::new();
    for record in live_work(ledger) {
        let under_goal = part_of_chain(model, record)
            .iter()
            .any(|parent| parent == &goal.id);
        let under_plan = plan.as_ref().is_some_and(|plan| {
            part_of_chain(model, record)
                .iter()
                .any(|parent| parent.key == plan.id.key)
        });
        let in_paths = !request.paths.is_empty()
            && scopes_overlap(&record.scope, &request.scope(), &ledger.part_of_index());
        if under_goal || under_plan || in_paths {
            work.push(record);
        }
    }
    work.sort_by(|a, b| (state_rank(a), &a.id).cmp(&(state_rank(b), &b.id)));
    for record in work {
        push(
            &mut sections,
            &mut selected,
            Section::WorkItems,
            &record.id,
            Some(Relation::PartOf),
            1,
        );
    }

    // -- step 4: the plan of record, in full ------------------------------------------
    if let Some(plan) = &plan {
        // It may already be in step 3; the plan section names it either way.
        selected.insert(plan.id.clone());
        sections
            .entry(Section::PlanOfRecord)
            .or_default()
            .push(Entry::Record {
                id: plan.id.clone(),
                via: Some(Relation::PlanOfRecord),
                depth: 0,
            });
    }

    // -- step 5: normative records in scope --------------------------------------------
    let organisational: Vec<RevisionId> = plan_targets.clone();
    let mut normative: Vec<&Record> = ledger
        .records()
        .iter()
        .filter(|r| r.kind.class() == Class::Normative && r.is_live() && !is_archived(r))
        .filter(|r| model.is_head(&r.id))
        .filter(|r| in_scope(model, r, &organisational, request))
        .collect();
    normative.sort_by(|a, b| (normative_rank(a.kind), &a.id).cmp(&(normative_rank(b.kind), &b.id)));
    for record in normative {
        push(
            &mut sections,
            &mut selected,
            Section::Normative,
            &record.id,
            None,
            0,
        );
    }

    // -- step 6: dependencies and blockers ----------------------------------------------
    let mut frontier: Vec<RevisionId> = selected.iter().cloned().collect();
    for depth in 1..=request.depth {
        let mut next: BTreeSet<RevisionId> = BTreeSet::new();
        // Blockers first: a dependency is something to read, a blocker is a reason to stop.
        for record in sorted_records(ledger) {
            if !record.is_live() || is_archived(record) {
                continue;
            }
            for reference in record.targets(Relation::Blocks) {
                let Some(target) = ledger.resolve(reference).ok().flatten() else {
                    continue;
                };
                if frontier.contains(&target.id)
                    && push(
                        &mut sections,
                        &mut selected,
                        Section::Dependencies,
                        &record.id,
                        Some(Relation::Blocks),
                        depth,
                    )
                {
                    next.insert(record.id.clone());
                }
            }
        }
        for source in &frontier {
            let Some(record) = ledger.get(source) else {
                continue;
            };
            for relation in [Relation::DependsOn, Relation::Implements] {
                for reference in record.targets(relation) {
                    let Some(target) = ledger.resolve(reference).ok().flatten() else {
                        continue;
                    };
                    // Empirical targets are step 8's, which knows how to order and mark
                    // them.
                    if target.kind.class() == Class::Empirical
                        || !target.is_live()
                        || is_archived(target)
                        || !model.is_head(&target.id)
                    {
                        continue;
                    }
                    if push(
                        &mut sections,
                        &mut selected,
                        Section::Dependencies,
                        &target.id,
                        Some(relation),
                        depth,
                    ) {
                        next.insert(target.id.clone());
                    }
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next.into_iter().collect();
    }
    if let Some(entries) = sections.get_mut(&Section::Dependencies) {
        entries.sort_by_key(|entry| match entry {
            Entry::Record { id, via, depth } => (
                usize::from(*via != Some(Relation::Blocks)),
                *depth,
                id.clone(),
            ),
        });
    }

    // -- step 7: acceptance --------------------------------------------------------------
    let mut acceptance: Vec<CheckVerdict> = model
        .acceptance
        .iter()
        .filter(|verdict| selected.contains(&verdict.owner))
        .cloned()
        .collect();
    acceptance.sort_by(|a, b| (&a.owner, &a.check).cmp(&(&b.owner, &b.check)));

    // -- step 8: observations -------------------------------------------------------------
    //
    // Run to a fixpoint. "The target of a … edge from any already-selected record"
    // includes records selected *by this step*: `sys.assessment.m3-readiness/1` arrives
    // by `scope ref` on the goal and carries `supported_by @sim.obs.timestep-drift/1`,
    // and the observation is only reachable through it. One pass would drop it, which is
    // the difference between the bundle showing a contradiction and hiding it.
    let mut empirical: Vec<&Record> = Vec::new();
    let mut chosen: BTreeSet<RevisionId> = BTreeSet::new();
    loop {
        let mut added = false;
        for record in ledger.records() {
            if chosen.contains(&record.id)
                || record.kind.class() != Class::Empirical
                || !record.is_live()
                || is_archived(record)
                || !model.is_head(&record.id)
            {
                continue;
            }
            let by_path = !request.paths.is_empty()
                && (scopes_overlap(&record.scope, &request.scope(), &ledger.part_of_index())
                    || watches(record)
                        .iter()
                        .any(|glob| glob_overlaps(glob, &request.paths)));
            let by_ref_scope = record.scope.iter().any(|term| match term {
                ScopeTerm::Ref(reference) => ledger
                    .resolve(reference)
                    .ok()
                    .flatten()
                    .is_some_and(|t| organisational.contains(&t.id)),
                _ => false,
            });
            let by_edge = sorted_records(ledger).into_iter().any(|other| {
                (selected.contains(&other.id) || chosen.contains(&other.id))
                    && ([
                        Relation::SupportedBy,
                        Relation::VerifiedBy,
                        Relation::DerivedFrom,
                        Relation::DependsOn,
                    ]
                    .iter()
                    .any(|relation| {
                        other
                            .targets(*relation)
                            .iter()
                            .filter_map(|r| ledger.resolve(r).ok().flatten())
                            .any(|t| t.id == record.id)
                    }) || other.acceptance.as_ref().is_some_and(|acceptance| {
                        acceptance.checks.iter().any(|check| {
                            check
                                .verified_by
                                .iter()
                                .filter_map(|r| ledger.resolve(r).ok().flatten())
                                .any(|t| t.id == record.id)
                        })
                    }) || other.claims.iter().any(|claim| {
                        claim
                            .supported_by
                            .iter()
                            .filter_map(|r| ledger.resolve(r).ok().flatten())
                            .any(|t| t.id == record.id)
                    }))
            });
            if by_path || by_ref_scope || by_edge {
                chosen.insert(record.id.clone());
                empirical.push(record);
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    empirical.sort_by(|a, b| {
        let stale = |r: &Record| usize::from(!freshness.is_stale(&r.id));
        (stale(a), &a.id).cmp(&(stale(b), &b.id))
    });
    for record in empirical {
        push(
            &mut sections,
            &mut selected,
            Section::Observations,
            &record.id,
            None,
            0,
        );
    }

    // -- step 9: open questions --------------------------------------------------------
    let mut questions: Vec<&Record> = ledger
        .records()
        .iter()
        .filter(|r| r.kind == Kind::Question && r.is_live() && !is_archived(r))
        .filter(|r| model.is_head(&r.id))
        .filter(|r| {
            let blocks = r
                .targets(Relation::Blocks)
                .iter()
                .filter_map(|reference| ledger.resolve(reference).ok().flatten())
                .any(|t| selected.contains(&t.id));
            let part_of = r
                .targets(Relation::PartOf)
                .iter()
                .filter_map(|reference| ledger.resolve(reference).ok().flatten())
                .any(|t| selected.contains(&t.id));
            blocks || part_of
        })
        .collect();
    questions.sort_by(|a, b| {
        let open = |r: &Record| usize::from(r.state != crate::model::State::Open);
        (open(a), &a.id).cmp(&(open(b), &b.id))
    });
    for record in questions {
        // A question that blocks a selected record appears twice on purpose: in step 6 as
        // a reason to stop, and here as an open question. `docs/09-context-assembly.md`
        // §8 lists `sim.question.timestep-vs-budget/1` under both sections, so questions
        // join acceptance, contradictions and staleness as cross-cutting.
        selected.insert(record.id.clone());
        let entries = sections.entry(Section::Questions).or_default();
        if !entries.iter().any(|entry| match entry {
            Entry::Record { id, .. } => id == &record.id,
        }) {
            entries.push(Entry::Record {
                id: record.id.clone(),
                via: None,
                depth: 0,
            });
        }
    }

    // -- step 10: contradictions, always ------------------------------------------------
    let contradictions = contradictions_touching(ledger, &selected);
    for pair in &contradictions {
        for endpoint in [&pair.left, &pair.right] {
            if !selected.contains(endpoint)
                && ledger.get(endpoint).is_some()
                && push(
                    &mut sections,
                    &mut selected,
                    Section::Contradictions,
                    endpoint,
                    Some(Relation::Contradicts),
                    0,
                )
            {
                // Surfaced even when terminal or archived: exclusion rules do not apply
                // to step 10 (V-121).
            }
        }
    }

    // -- step 11: staleness ---------------------------------------------------------------
    let mut flagged: Vec<(usize, RevisionId)> = Vec::new();
    for id in &selected {
        if freshness.is_stale(id) {
            flagged.push((0, id.clone()));
        } else if let Some(entry) = freshness.at_risk(id) {
            flagged.push((entry.depth, id.clone()));
        }
    }
    flagged.sort();
    for (depth, id) in flagged {
        sections
            .entry(Section::Staleness)
            .or_default()
            .push(Entry::Record {
                id,
                via: None,
                depth,
            });
    }

    // -- exclusions -----------------------------------------------------------------------
    let excluded = tally_exclusions(model, &selected);

    let mut bundle = Bundle {
        goal: goal.id.clone(),
        commit: model.commit.as_ref().map(|c| c.as_str().to_owned()),
        tool_version: model.tool_version.clone(),
        source_graph: model.source_graph.0.clone(),
        paths: request.paths.clone(),
        sections,
        acceptance,
        contradictions,
        excluded,
        truncated: Vec::new(),
        withheld: BTreeMap::new(),
        estimated_tokens: 0,
    };

    apply_budget(&mut bundle, model, freshness, request)?;
    Ok(bundle)
}

// -------------------------------------------------------------------------------------
// Budgeting (§6)
// -------------------------------------------------------------------------------------

/// Tokens held back from the budget for the `budget` accounting object the command adds
/// after assembly, so that the object reporting the promise does not itself break it.
const BUDGET_RESERVE: usize = 64;

/// How many observations survive whole before any other record is withheld (§6 step 6).
const KEPT_OBSERVATIONS: usize = 10;

/// The token estimate every context budget is measured with: four characters a token.
///
/// That is what tokenizers average on English prose and the rule of thumb hosts apply
/// when they decide a tool result is too large to keep, so a budget measured this way is
/// a budget the host agrees with. It overcounts runs of indentation a little, which errs
/// toward a smaller bundle rather than a refused one.
///
/// It replaced a words-plus-punctuation count that read a dotted key, a JSON field or a
/// box-drawing rule as a single token. On a 2,700-record ledger that undercounted the
/// delivered bundle about two and a half times, so a bundle "within" a 12,000-token
/// budget arrived at more than 100,000 characters and was refused by the host.
#[must_use]
pub fn estimate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

/// What a bundle costs to deliver: its text form and its JSON form, which MCP sends
/// together and which `--budget` is stated in.
#[must_use]
pub fn delivered_tokens(
    bundle: &Bundle,
    model: &ResolvedModel<'_>,
    freshness: &Freshness,
) -> usize {
    estimate_tokens(&render_text(bundle, model, freshness))
        + estimate_tokens(&render_json(bundle, model).to_pretty())
}

/// One step of reduction, in the order the budget spends them.
#[derive(Debug, Clone)]
enum Reduction {
    /// Drop a record's prose, keeping its header, scope and relations.
    Truncate(RevisionId),
    /// Withhold a record's entry from a section whole, naming it in the section's
    /// `withheld` list so the reader can fetch it.
    Withhold(Section, RevisionId),
}

/// Applies the token budget.
///
/// The budget is measured against the *rendered* bundle — both forms, with the same
/// estimator the command reports — not against the records selected, which is a far
/// smaller number and the reason a budgeted bundle used to arrive several times over.
///
/// Reduction is a fixed sequence (§6), each section spent from its tail, which is the
/// least-ranked end of every section's sort order:
///
/// 1. prose of observations, normative records, dependencies and questions;
/// 2. observations beyond the ten that lead the section, withheld whole;
/// 3. normative records, then dependencies (deepest first, blockers last), withheld whole;
/// 4. prose of work items;
/// 5. questions, the leading observations, and work items, withheld whole.
///
/// The bundle takes the shortest prefix of that sequence that fits. What never reduces:
/// the goal — its body is the intent the whole bundle serves — the milestone, the plan of
/// record and its dispositions, acceptance verdicts, contradictions, staleness warnings,
/// and the key and revision of every record the selection reached. A withheld record is
/// named in its section, so the graph the reader is shown is incomplete in a way it says,
/// never in a way it hides (V-123).
fn apply_budget(
    bundle: &mut Bundle,
    model: &ResolvedModel<'_>,
    freshness: &Freshness,
    request: &Request,
) -> Result<(), ContextError> {
    let Some(budget) = request.budget else {
        bundle.estimated_tokens = delivered_tokens(bundle, model, freshness);
        return Ok(());
    };
    let mandatory = mandatory_tokens(bundle, model, freshness);
    if mandatory > budget {
        return Err(ContextError::BudgetTooSmall {
            budget,
            required: mandatory,
        });
    }
    let target = budget.saturating_sub(BUDGET_RESERVE).max(mandatory);

    let base = bundle.clone();
    let measure = |reduced: &Bundle| delivered_tokens(reduced, model, freshness);
    let initial = measure(&base);
    if initial <= target {
        bundle.estimated_tokens = initial;
        return Ok(());
    }

    let reductions = reductions_for(&base);
    let full = reduce(&base, &reductions);
    let full_cost = measure(&full);
    if full_cost > target {
        // Irreducible above the request. Deliver the smallest honest bundle; the command
        // reports both figures.
        *bundle = full;
        bundle.estimated_tokens = full_cost;
        return Ok(());
    }

    // The cost falls as the prefix grows, so the shortest prefix that fits is found by
    // bisection: a handful of renders rather than one per record.
    let (mut low, mut high) = (0usize, reductions.len());
    let mut best = (full, full_cost);
    while low + 1 < high {
        let middle = low + (high - low) / 2;
        let candidate = reduce(&base, &reductions[..middle]);
        let cost = measure(&candidate);
        if cost <= target {
            high = middle;
            best = (candidate, cost);
        } else {
            low = middle;
        }
    }
    let (reduced, cost) = best;
    *bundle = reduced;
    bundle.estimated_tokens = cost;
    Ok(())
}

/// The reduction sequence for a bundle, in spending order.
fn reductions_for(bundle: &Bundle) -> Vec<Reduction> {
    let protected: BTreeSet<&RevisionId> = std::iter::once(&bundle.goal)
        .chain(bundle.section(Section::Milestone).iter().map(Entry::id))
        .chain(bundle.section(Section::PlanOfRecord).iter().map(Entry::id))
        .collect();
    let tail = |section: Section, skip: usize| -> Vec<RevisionId> {
        bundle
            .section(section)
            .iter()
            .skip(skip)
            .rev()
            .map(Entry::id)
            .filter(|id| !protected.contains(id))
            .cloned()
            .collect()
    };
    let head = |section: Section, take: usize| -> Vec<RevisionId> {
        bundle
            .section(section)
            .iter()
            .take(take)
            .rev()
            .map(Entry::id)
            .filter(|id| !protected.contains(id))
            .cloned()
            .collect()
    };

    let mut out = Vec::new();
    let mut truncated: BTreeSet<RevisionId> = BTreeSet::new();
    let mut truncate = |out: &mut Vec<Reduction>, ids: Vec<RevisionId>| {
        for id in ids {
            if truncated.insert(id.clone()) {
                out.push(Reduction::Truncate(id));
            }
        }
    };
    let withhold = |out: &mut Vec<Reduction>, section: Section, ids: Vec<RevisionId>| {
        out.extend(ids.into_iter().map(|id| Reduction::Withhold(section, id)));
    };

    for section in [
        Section::Observations,
        Section::Normative,
        Section::Dependencies,
        Section::Questions,
    ] {
        truncate(&mut out, tail(section, 0));
    }
    withhold(
        &mut out,
        Section::Observations,
        tail(Section::Observations, KEPT_OBSERVATIONS),
    );
    withhold(&mut out, Section::Normative, tail(Section::Normative, 0));
    withhold(
        &mut out,
        Section::Dependencies,
        tail(Section::Dependencies, 0),
    );
    truncate(&mut out, tail(Section::WorkItems, 0));
    withhold(&mut out, Section::Questions, tail(Section::Questions, 0));
    withhold(
        &mut out,
        Section::Observations,
        head(Section::Observations, KEPT_OBSERVATIONS),
    );
    withhold(&mut out, Section::WorkItems, tail(Section::WorkItems, 0));
    out
}

/// `bundle` with `reductions` applied.
fn reduce(bundle: &Bundle, reductions: &[Reduction]) -> Bundle {
    let mut out = bundle.clone();
    let mut truncated: BTreeSet<RevisionId> = bundle.truncated.iter().cloned().collect();
    let mut withheld: BTreeMap<Section, BTreeSet<RevisionId>> = BTreeMap::new();
    for reduction in reductions {
        match reduction {
            Reduction::Truncate(id) => {
                truncated.insert(id.clone());
            }
            Reduction::Withhold(section, id) => {
                withheld.entry(*section).or_default().insert(id.clone());
            }
        }
    }
    for (section, ids) in &withheld {
        let Some(entries) = out.sections.get_mut(section) else {
            continue;
        };
        // In the section's own order, so the list reads like the section it came from.
        let list = out.withheld.entry(*section).or_default();
        list.extend(
            entries
                .iter()
                .map(Entry::id)
                .filter(|id| ids.contains(*id))
                .cloned(),
        );
        entries.retain(|entry| !ids.contains(entry.id()));
    }
    out.withheld.retain(|_, ids| !ids.is_empty());
    // A record withheld everywhere has no prose left to truncate; one still shown in
    // another section (a blocking question is in two) keeps its truncation marker there.
    // Staleness, acceptance and contradictions name records without rendering their
    // prose, so an entry there does not count as showing it.
    let members: BTreeSet<RevisionId> = out
        .sections
        .iter()
        .filter(|(section, _)| {
            !matches!(
                section,
                Section::Staleness | Section::Acceptance | Section::Contradictions
            )
        })
        .flat_map(|(_, entries)| entries.iter().map(Entry::id).cloned())
        .collect();
    out.truncated = truncated
        .into_iter()
        .filter(|id| members.contains(id))
        .collect();
    out
}

/// What the bundle must hold whatever the budget: the goal in full, the milestone and the
/// plan of record, every acceptance verdict and every contradiction — measured as
/// rendered, in both forms. Everything else can be truncated or withheld by name.
fn mandatory_tokens(bundle: &Bundle, model: &ResolvedModel<'_>, freshness: &Freshness) -> usize {
    let text = render::mandatory_text(bundle, model, freshness);
    // The JSON form repeats keys and titles; twice the text is a close bound for it.
    estimate_tokens(&text) * 2
}

// -------------------------------------------------------------------------------------
// Selection helpers
// -------------------------------------------------------------------------------------

/// Whether a record lives under `.akr/archive/` (D-018).
fn is_archived(record: &Record) -> bool {
    record
        .file
        .as_deref()
        .is_some_and(|path| path.contains("/archive/") || path.starts_with("archive/"))
}

fn live_work(ledger: &Ledger) -> Vec<&Record> {
    let mut out: Vec<&Record> = ledger
        .records()
        .iter()
        .filter(|r| r.kind == Kind::Work && r.is_live() && !is_archived(r))
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

fn state_rank(record: &Record) -> u8 {
    match record.state {
        crate::model::State::Active => 0,
        crate::model::State::Blocked => 1,
        crate::model::State::Ready => 2,
        crate::model::State::Proposed => 3,
        _ => 4,
    }
}

/// Definitions first, then the limits the project did not choose, then the rules it did,
/// then what it must deliver, then what it decided (§4 step 5).
const fn normative_rank(kind: Kind) -> u8 {
    match kind {
        Kind::Term => 0,
        Kind::Constraint => 1,
        Kind::Policy => 2,
        Kind::Requirement => 3,
        Kind::Decision => 4,
        _ => 5,
    }
}

/// The `part_of` chain of a record, resolved, nearest first. Cycle-safe.
fn part_of_chain(model: &ResolvedModel<'_>, record: &Record) -> Vec<RevisionId> {
    let ledger = model.ledger();
    let mut out = Vec::new();
    let mut cursor = record;
    let mut guard = 0usize;
    while let Some(parent) = cursor
        .targets(Relation::PartOf)
        .first()
        .and_then(|reference| ledger.resolve(reference).ok().flatten())
    {
        out.push(parent.id.clone());
        cursor = parent;
        guard += 1;
        if guard > ledger.records().len() {
            break;
        }
    }
    out
}

/// The `part_of` ancestors of a revision, with their distance.
fn part_of_ancestors(model: &ResolvedModel<'_>, id: &RevisionId) -> Vec<(usize, RevisionId)> {
    let Some(record) = model.ledger().get(id) else {
        return Vec::new();
    };
    part_of_chain(model, record)
        .into_iter()
        .enumerate()
        .map(|(index, parent)| (index + 1, parent))
        .collect()
}

/// The live plan of record for any of the given targets (V-018: at most one each).
fn plan_of_record<'a>(model: &'a ResolvedModel<'a>, targets: &[RevisionId]) -> Option<&'a Record> {
    let ledger = model.ledger();
    let mut candidates: Vec<&Record> = ledger
        .records()
        .iter()
        .filter(|r| r.is_live() && !is_archived(r))
        .filter(|r| {
            r.targets(Relation::PlanOfRecord)
                .iter()
                .filter_map(|reference| ledger.resolve(reference).ok().flatten())
                .any(|t| targets.contains(&t.id))
        })
        .collect();
    candidates.sort_by(|a, b| a.id.cmp(&b.id));
    candidates.first().copied()
}

/// The D-010 scope test for step 5: a `ref` term naming the goal or a step-2 record, a
/// `path` term overlapping the request, or `scope [ all ]`.
fn in_scope(
    model: &ResolvedModel<'_>,
    record: &Record,
    organisational: &[RevisionId],
    request: &Request,
) -> bool {
    let ledger = model.ledger();
    if record.scope.contains(&ScopeTerm::All) {
        return true;
    }
    let by_ref = record.scope.iter().any(|term| match term {
        ScopeTerm::Ref(reference) => ledger
            .resolve(reference)
            .ok()
            .flatten()
            .is_some_and(|t| organisational.contains(&t.id)),
        _ => false,
    });
    if by_ref {
        return true;
    }
    !request.paths.is_empty()
        && scopes_overlap(&record.scope, &request.scope(), &ledger.part_of_index())
}

/// Whether a watch glob overlaps any requested path, by the conservative D-010 prefix
/// test.
fn glob_overlaps(glob: &Glob, paths: &[Glob]) -> bool {
    paths
        .iter()
        .any(|path| crate::model::glob_prefixes_comparable(glob, path))
}

/// Every `contradicts` edge with an endpoint among the selected records (V-121).
fn contradictions_touching(ledger: &Ledger, selected: &BTreeSet<RevisionId>) -> Vec<Contradiction> {
    let mut out: BTreeSet<Contradiction> = BTreeSet::new();
    for record in sorted_records(ledger) {
        for reference in record.targets(Relation::Contradicts) {
            let Some(target) = ledger.resolve(reference).ok().flatten() else {
                continue;
            };
            if !selected.contains(&record.id) && !selected.contains(&target.id) {
                continue;
            }
            let (left, right) = if record.id <= target.id {
                (record.id.clone(), target.id.clone())
            } else {
                (target.id.clone(), record.id.clone())
            };
            out.insert(Contradiction {
                left,
                right,
                declared_by: record.id.clone(),
                acknowledged: record.acknowledged || target.acknowledged,
            });
        }
    }
    out.into_iter().collect()
}

/// Counts what was left out, by reason (§5).
fn tally_exclusions(model: &ResolvedModel<'_>, selected: &BTreeSet<RevisionId>) -> Excluded {
    let ledger = model.ledger();
    let mut excluded = Excluded::default();
    for record in sorted_records(ledger) {
        if selected.contains(&record.id) {
            continue;
        }
        if !model.is_head(&record.id) {
            excluded.superseded.push(record.id.clone());
        } else if is_archived(record) {
            excluded.archived.push(record.id.clone());
        } else if record.is_terminal() {
            excluded.terminal.push(record.id.clone());
        } else {
            excluded.out_of_scope.push(record.id.clone());
        }
    }
    excluded
}

/// A record's required prose slot, for rendering.
#[must_use]
pub fn body_of(record: &Record) -> Option<&str> {
    named_body_of(record).map(|(_, text)| text)
}

/// The required prose slot and its text, so a renderer can name the slot.
#[must_use]
pub fn named_body_of(record: &Record) -> Option<(ContentSlot, &str)> {
    for slot in [
        ContentSlot::Intent,
        ContentSlot::Statement,
        ContentSlot::Rule,
        ContentSlot::Decision,
        ContentSlot::Definition,
        ContentSlot::Question,
        ContentSlot::Summary,
    ] {
        if let Some(ContentValue::Prose(text) | ContentValue::Text(text)) = record.get(slot) {
            return Some((slot, text));
        }
    }
    None
}

/// The `observed_at` of a record, for rendering.
#[must_use]
pub fn observed_commit(record: &Record) -> Option<String> {
    observed_at(record).map(|c| c.as_str()[..8].to_owned())
}

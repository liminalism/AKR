//! Stage F: rendering the resolved model into generated Markdown views.
//!
//! `docs/11-projections.md` is normative for the catalogue, the source queries, the
//! section ordering, the rendering rules, and the banner. This module implements it.
//!
//! All seven views render. Six always return `Some`; [`View::Papercuts`] returns `None`
//! when the ledger holds no papercut (D-027), so a project that never logs one never
//! grows the file.
//!
//! # Determinism
//!
//! Headings come from the required `title` slot, never derived from prose. Every list is
//! sorted by the view's declared sort key with the record key as the final tiebreak. The
//! banner's three variable fields are all build inputs, and none of them is a clock
//! reading — a timestamp would make every rebuild produce a diff and the CI gate useless.

mod active_work;
mod common;
mod current_state;
mod decision_history;
mod open_questions;
mod papercuts;
mod review_required;
mod roadmap;
mod views_current;

pub use active_work::render_active_work;
pub use current_state::render_current_state;
pub use decision_history::render_decision_history;
pub use open_questions::render_open_questions;
pub use papercuts::render_papercuts;
pub use review_required::render_review_required;
pub use roadmap::render_roadmap;
pub use views_current::{check_views_current, write_views};

/// The emission diagnostics this module raises.
///
/// Registered in `spec/diagnostics/codes-runtime.md`, not `codes-lang.md`: the `E` range
/// belongs to the runtime half of the design set (`spec/diagnostics/README.md` §2). They
/// live here rather than in [`crate::diagnostics::codes`] because that constant set is
/// asserted to be language-stage only, and widening it would make
/// `tests/codes_registry.rs` check the wrong registry.
///
/// When P5 and P6 add the `G`, `C`, `X` and `I` ranges, the natural home is one shared
/// `diagnostics::runtime` module; until there is a second range to share it with, one
/// module owning one range is the smaller thing.
pub mod codes {
    use crate::diagnostics::Code;

    /// View output directory not writable.
    pub const E001: Code = Code::new("AKR-E001");
    /// View output path escapes the repository.
    pub const E002: Code = Code::new("AKR-E002");
    /// Unknown view.
    pub const E003: Code = Code::new("AKR-E003");
    /// Generated view is out of date — the D-025 CI gate.
    pub const E011: Code = Code::new("AKR-E011");
    /// Generated view missing.
    pub const E012: Code = Code::new("AKR-E012");
    /// Generated view banner malformed.
    pub const E013: Code = Code::new("AKR-E013");
    /// Unexpected file in the view output directory.
    pub const E014: Code = Code::new("AKR-E014");
    /// Generated view differs only in the banner's `tool:` version line.
    pub const E015: Code = Code::new("AKR-E015");
    /// Generated view is behind ledger changes that are not committed yet.
    pub const E016: Code = Code::new("AKR-E016");
    /// Record required by a view is absent from the resolved model.
    pub const E021: Code = Code::new("AKR-E021");
    /// Duplicate heading anchor in a view.
    pub const E022: Code = Code::new("AKR-E022");

    /// Every emission code this crate can raise.
    pub const ALL: &[Code] =
        &[E001, E002, E003, E011, E012, E013, E014, E015, E016, E021, E022];
}

use crate::graph::{AtRisk, propagate_staleness};
use crate::model::{Ledger, RevisionId};
use crate::resolve::ResolvedModel;
use std::collections::{BTreeMap, BTreeSet};

/// The seven generated views (`docs/11-projections.md` §2), in the order stage F renders
/// them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum View {
    /// Where the project is going.
    Roadmap,
    /// What the project believes right now.
    CurrentState,
    /// What is being worked on, and what is stuck.
    ActiveWork,
    /// What should not be trusted without re-checking.
    ReviewRequired,
    /// What is not yet known.
    OpenQuestions,
    /// What was decided, and what was retired.
    DecisionHistory,
    /// Small frictions, newest first — emitted once the ledger holds one (D-027).
    Papercuts,
}

impl View {
    /// The catalogue, in render order. Closed for 0.1.
    pub const ALL: &'static [View] = &[
        View::Roadmap,
        View::CurrentState,
        View::ActiveWork,
        View::ReviewRequired,
        View::OpenQuestions,
        View::DecisionHistory,
        View::Papercuts,
    ];

    /// The file the view is written to.
    #[must_use]
    pub const fn file_name(self) -> &'static str {
        match self {
            Self::Roadmap => "ROADMAP.md",
            Self::CurrentState => "CURRENT-STATE.md",
            Self::ActiveWork => "ACTIVE-WORK.md",
            Self::ReviewRequired => "REVIEW-REQUIRED.md",
            Self::OpenQuestions => "OPEN-QUESTIONS.md",
            Self::DecisionHistory => "DECISION-HISTORY.md",
            Self::Papercuts => "PAPERCUTS.md",
        }
    }

    /// The catalogue name `akr view <name>` accepts.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Roadmap => "roadmap",
            Self::CurrentState => "current-state",
            Self::ActiveWork => "active-work",
            Self::ReviewRequired => "review-required",
            Self::OpenQuestions => "open-questions",
            Self::DecisionHistory => "decision-history",
            Self::Papercuts => "papercuts",
        }
    }

    /// Looks up a view by catalogue name or file name, case-insensitively and with or
    /// without the `.md` suffix (`docs/07-cli.md`, `akr view`).
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        let lower = name.to_ascii_lowercase();
        let stem = lower.strip_suffix(".md").unwrap_or(&lower);
        Self::ALL
            .iter()
            .copied()
            .find(|v| v.name() == stem || v.file_name().eq_ignore_ascii_case(name))
    }

    /// Which view renders a record of this kind, for cross-view links
    /// (`docs/11-projections.md` §3: "a reference to a record in another view is a
    /// relative link to that file's anchor").
    ///
    /// `decision` goes to `DECISION-HISTORY.md` rather than `CURRENT-STATE.md`, which
    /// excludes decisions because they have their own view (§6).
    #[must_use]
    pub const fn hosting(kind: crate::model::Kind) -> Option<Self> {
        use crate::model::Kind;
        Some(match kind {
            Kind::Milestone | Kind::Track => Self::Roadmap,
            Kind::Work => Self::ActiveWork,
            Kind::Decision => Self::DecisionHistory,
            Kind::Question => Self::OpenQuestions,
            Kind::Papercut => Self::Papercuts,
            Kind::Term
            | Kind::Requirement
            | Kind::Policy
            | Kind::Constraint
            | Kind::Observation
            | Kind::Evidence
            | Kind::Assessment => Self::CurrentState,
        })
    }
}

/// The derived freshness a view needs: which records are stale, and what rests on them.
///
/// # Seam
///
/// Deriving the stale set from `observed_at`, `watches` and `review_after` is phase P5
/// (`docs/10-freshness-and-git.md` §3). A renderer only ever *reads* the answer, so this
/// carries it rather than computing it: [`Freshness::from_stale`] takes the set and does
/// the propagation, and [`Freshness::none`] is the honest answer before P5 exists.
#[derive(Debug, Clone, Default)]
pub struct Freshness {
    stale: BTreeSet<RevisionId>,
    at_risk: BTreeMap<RevisionId, AtRisk>,
    causes: BTreeMap<RevisionId, crate::freshness::StaleCause>,
}

impl Freshness {
    /// No record is stale and nothing is at risk.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// Propagates from a stale set (D-024) and keeps both halves for rendering.
    #[must_use]
    pub fn from_stale(ledger: &Ledger, stale: BTreeSet<RevisionId>) -> Self {
        let at_risk = propagate_staleness(ledger, &stale)
            .into_iter()
            .map(|entry| (entry.id.clone(), entry))
            .collect();
        Self {
            stale,
            at_risk,
            causes: BTreeMap::new(),
        }
    }

    /// Whether a revision is stale in its own right.
    #[must_use]
    pub fn is_stale(&self, id: &RevisionId) -> bool {
        self.stale.contains(id)
    }

    /// Attaches the causes a review queue computed, so a bundle can say *why* stale.
    ///
    /// Optional because the projections renderer does not need them and the roadmap
    /// snapshot tests construct a `Freshness` from a bare set.
    #[must_use]
    pub fn with_causes(
        mut self,
        causes: BTreeMap<RevisionId, crate::freshness::StaleCause>,
    ) -> Self {
        self.causes = causes;
        self
    }

    /// Why a revision is stale, when the caller supplied causes.
    #[must_use]
    pub fn cause(&self, id: &RevisionId) -> Option<&crate::freshness::StaleCause> {
        self.causes.get(id)
    }

    /// The at-risk entry for a revision, if it has one.
    #[must_use]
    pub fn at_risk(&self, id: &RevisionId) -> Option<&AtRisk> {
        self.at_risk.get(id)
    }

    /// The marker a metadata line or list entry carries, if any
    /// (`docs/11-projections.md` §3).
    #[must_use]
    pub fn marker(&self, id: &RevisionId) -> Option<&'static str> {
        if self.is_stale(id) {
            Some("**stale**")
        } else if self.at_risk.contains_key(id) {
            Some("**at risk**")
        } else {
            None
        }
    }

    /// Stale revisions in review-queue order (`docs/10-freshness-and-git.md` §7): cause
    /// `watch` before cause `review_after`, then the matching commit or date, then key.
    ///
    /// `REVIEW-REQUIRED.md` is the committed half of `akr review-queue`, and reuses this
    /// ordering so the two never disagree about which record comes first.
    #[must_use]
    pub fn stale_in_order(&self) -> Vec<RevisionId> {
        let mut out: Vec<RevisionId> = self.stale.iter().cloned().collect();
        out.sort_by(|a, b| {
            stale_order_key(self.causes.get(a), a).cmp(&stale_order_key(self.causes.get(b), b))
        });
        out
    }

    /// At-risk entries by propagation depth, then key — the order
    /// [`crate::graph::propagate_staleness`] already computes.
    #[must_use]
    pub fn at_risk_in_order(&self) -> Vec<&AtRisk> {
        let mut out: Vec<&AtRisk> = self.at_risk.values().collect();
        out.sort_by(|a, b| (a.depth, &a.id).cmp(&(b.depth, &b.id)));
        out
    }
}

/// The sort key [`Freshness::stale_in_order`] uses: cause `watch` (0) before
/// `review_after` (1), then the commit or date the cause names, then the key — the same
/// ordering `docs/10-freshness-and-git.md` §7 defines, computed here from what `Freshness`
/// carries rather than from the private `ReviewQueue` entry.
fn stale_order_key(
    cause: Option<&crate::freshness::StaleCause>,
    id: &RevisionId,
) -> (u8, String, String) {
    match cause {
        Some(crate::freshness::StaleCause::Watch { commit, .. }) => {
            (0, commit.as_str().to_owned(), id.to_string())
        }
        Some(crate::freshness::StaleCause::ReviewAfter { date }) => {
            (1, date.to_string(), id.to_string())
        }
        None => (2, String::new(), id.to_string()),
    }
}

/// Everything a view needs: the resolved model, and the freshness derived over it.
#[derive(Debug, Clone, Copy)]
pub struct RenderContext<'a> {
    /// The resolved model.
    pub model: &'a ResolvedModel<'a>,
    /// Stale and at-risk flags.
    pub freshness: &'a Freshness,
}

impl<'a> RenderContext<'a> {
    /// A context over a model, with the given freshness.
    #[must_use]
    pub fn new(model: &'a ResolvedModel<'a>, freshness: &'a Freshness) -> Self {
        Self { model, freshness }
    }

    /// The ledger behind the model.
    #[must_use]
    pub fn ledger(&self) -> &'a Ledger {
        self.model.ledger()
    }
}

/// Renders one view, or `None` for `PAPERCUTS.md` when the ledger holds no papercut
/// (D-027) — the only view with an emit-only-when-nonempty rule.
#[must_use]
pub fn render(view: View, context: RenderContext<'_>) -> Option<String> {
    match view {
        View::Roadmap => Some(render_roadmap(context)),
        View::CurrentState => Some(render_current_state(context)),
        View::ActiveWork => Some(render_active_work(context)),
        View::ReviewRequired => Some(render_review_required(context)),
        View::OpenQuestions => Some(render_open_questions(context)),
        View::DecisionHistory => Some(render_decision_history(context)),
        View::Papercuts => render_papercuts(context),
    }
}

/// The banner every generated file opens with, on line 1 (D-025).
///
/// ```text
/// <!-- GENERATED BY AKR — DO NOT EDIT
///      source-graph: sha256:<64 hex>
///      tool: akr <version>
/// -->
/// ```
///
/// Both fields are inputs to the build. **No timestamp appears**, deliberately: a
/// wall-clock field would make every rebuild produce a diff, which would make the
/// `--views-current` gate useless and train everyone to ignore view changes.
///
/// A commit hash is deliberately absent. These bytes participate in the commit, so its
/// hash cannot be known while rendering them. The source-graph is stable across that
/// boundary and commits map back to it through their `AKR-Graph` trailer.
#[must_use]
pub fn banner(model: &ResolvedModel<'_>) -> String {
    format!(
        "<!-- GENERATED BY AKR — DO NOT EDIT\n     source-graph: {}\n     tool: {}\n-->\n",
        model.source_graph, model.tool_version
    )
}

/// A GitHub-flavoured heading anchor, for links between and within views.
///
/// Lowercase; backticks, emphasis and link syntax removed; anything that is not
/// alphanumeric, a space, a hyphen or an underscore dropped; spaces to hyphens. Headings
/// come from the required `title` slot (§3), so this is a pure function of a title.
#[must_use]
pub fn slug(heading: &str) -> String {
    let mut text = heading.replace(['`', '*'], "");
    // Strip `[label](target)` down to `label`.
    while let Some(open) = text.find("](") {
        if let Some(close) = text[open..].find(')') {
            text.replace_range(open..=open + close, "");
            if let Some(bracket) = text[..open].rfind('[') {
                text.remove(bracket);
            }
        } else {
            break;
        }
    }
    text.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        .collect::<String>()
        .trim()
        .replace(' ', "-")
}

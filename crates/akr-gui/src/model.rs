use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Immutable, presentation-ready data supplied by the AKR review projection.
///
/// The fields intentionally use owned UI values. The `akr-cli` adapter should
/// map its public `ReviewSnapshot` into this structure without leaking parser
/// or SQLite implementation details into the GUI.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReviewSnapshot {
    pub workspace: PathBuf,
    pub project: String,
    pub source_graph: String,
    pub head: Option<String>,
    pub counts: ReviewCounts,
    pub diagnostics: Vec<Diagnostic>,
    pub records: Vec<Record>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReviewCounts {
    pub records: usize,
    pub revisions: usize,
    pub stale: usize,
    pub at_risk: usize,
    pub diagnostics: usize,
    pub live_planning: usize,
    pub open_questions: usize,
}

impl ReviewSnapshot {
    pub fn record(&self, key: &str) -> Option<&Record> {
        self.records.iter().find(|record| record.key == key)
    }

    pub fn sorted_records(&self) -> Vec<&Record> {
        let mut records = self.records.iter().collect::<Vec<_>>();
        records.sort_by(|left, right| left.key.cmp(&right.key));
        records
    }

    /// Every record whose state is `proposed`, sorted by key.
    ///
    /// This is the outstanding list the review workbench shows: entries that
    /// have been recorded but not yet accepted, completed, or withdrawn. It
    /// is derived from record state alone, so what it shows never depends on
    /// anyone's summary of what is left.
    pub fn outstanding_records(&self) -> Vec<&Record> {
        let mut records = self
            .records
            .iter()
            .filter(|record| record.state == "proposed")
            .collect::<Vec<_>>();
        records.sort_by(|left, right| left.key.cmp(&right.key));
        records
    }

    pub fn outstanding_count(&self) -> usize {
        self.records
            .iter()
            .filter(|record| record.state == "proposed")
            .count()
    }

    /// The parent relation forms the planning hierarchy. Unparented records
    /// remain visible under `Unparented`, rather than being silently dropped.
    pub fn planning_roots(&self) -> Vec<TreeNode> {
        let keys = self
            .records
            .iter()
            .filter(|record| matches!(record.kind.as_str(), "track" | "milestone" | "work"))
            .map(|record| record.key.as_str())
            .collect::<BTreeSet<_>>();
        let mut children = BTreeMap::<String, Vec<String>>::new();
        let mut roots = Vec::new();
        for record in &self.records {
            if !matches!(record.kind.as_str(), "track" | "milestone" | "work") {
                continue;
            }
            let parent = record
                .relations
                .iter()
                .find(|relation| relation.kind == "part_of")
                .map(|relation| relation.target.as_str());
            match parent.filter(|parent| keys.contains(parent)) {
                Some(parent) => children
                    .entry(parent.to_owned())
                    .or_default()
                    .push(record.key.clone()),
                None => roots.push(record.key.clone()),
            }
        }
        for descendants in children.values_mut() {
            descendants.sort();
        }
        roots.sort();
        roots
            .into_iter()
            .filter_map(|key| self.tree_node(&key, &children, &mut BTreeSet::new()))
            .collect()
    }

    fn tree_node(
        &self,
        key: &str,
        children: &BTreeMap<String, Vec<String>>,
        visiting: &mut BTreeSet<String>,
    ) -> Option<TreeNode> {
        if !visiting.insert(key.to_owned()) {
            return Some(TreeNode {
                key: key.to_owned(),
                children: Vec::new(),
                cycle: true,
            });
        }
        let nested = children
            .get(key)
            .into_iter()
            .flatten()
            .filter_map(|child| self.tree_node(child, children, visiting))
            .collect();
        visiting.remove(key);
        Some(TreeNode {
            key: key.to_owned(),
            children: nested,
            cycle: false,
        })
    }

    pub fn knowledge_groups(&self) -> BTreeMap<String, Vec<String>> {
        let mut groups = BTreeMap::<String, Vec<String>>::new();
        for record in &self.records {
            let namespace = record.key.split('.').next().unwrap_or("unscoped");
            groups
                .entry(format!("{namespace} / {}", record.kind))
                .or_default()
                .push(record.key.clone());
        }
        for values in groups.values_mut() {
            values.sort();
        }
        groups
    }

    /// A bounded, deterministic graph around one selected record.
    pub fn neighborhood(
        &self,
        key: &str,
        max_depth: usize,
        max_nodes: usize,
    ) -> Vec<NeighborhoodEdge> {
        let mut emitted = BTreeSet::new();
        let mut visited = BTreeSet::from([key.to_owned()]);
        let mut queue = VecDeque::from([(key.to_owned(), 0usize)]);
        while let Some((current, depth)) = queue.pop_front() {
            if depth >= max_depth || visited.len() >= max_nodes {
                continue;
            }
            for record in &self.records {
                for relation in &record.relations {
                    if record.key != current && relation.target != current {
                        continue;
                    }
                    let other = if record.key == current {
                        relation.target.clone()
                    } else {
                        record.key.clone()
                    };
                    if self.record(&other).is_none() {
                        continue;
                    }
                    emitted.insert(NeighborhoodEdge {
                        from: record.key.clone(),
                        kind: relation.kind.clone(),
                        to: relation.target.clone(),
                    });
                    if visited.insert(other.clone()) && visited.len() < max_nodes {
                        queue.push_back((other, depth + 1));
                    }
                }
            }
        }
        emitted.into_iter().collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeNode {
    pub key: String,
    pub children: Vec<TreeNode>,
    pub cycle: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct NeighborhoodEdge {
    pub from: String,
    pub kind: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Record {
    pub key: String,
    pub title: String,
    pub kind: String,
    pub state: String,
    pub revision: u32,
    pub body: String,
    pub freshness: String,
    pub plan_of_record: bool,
    pub relations: Vec<Relation>,
    pub acceptance: Vec<AcceptanceCheck>,
    pub provenance: Vec<String>,
    pub history: Vec<String>,
    pub git: GitMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Relation {
    pub kind: String,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AcceptanceCheck {
    pub id: String,
    pub statement: String,
    pub verdict: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GitMetadata {
    pub defined_at: Option<String>,
    pub observed_at: Option<String>,
    pub stale_cause: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Diagnostic {
    pub severity: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotError {
    pub message: String,
}
impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for SnapshotError {}

/// One stage of a load, as the shell reports it.
///
/// A load can take seconds — nearly all of it waiting on git subprocesses — so the shell
/// shows which stage it is on and how far through the ordered list that is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadPhase {
    /// Present-tense description, e.g. "checking git freshness".
    pub label: String,
    /// One-based position in the sequence.
    pub step: usize,
    /// How many stages there are in total.
    pub total: usize,
}

/// The only data-access seam the GUI needs. Implementations may call
/// `akr_cli::review_snapshot::ReviewSnapshot::load` on a worker thread.
pub trait WorkspaceLoader: Send + Sync + 'static {
    fn load(&self, workspace: &Path) -> Result<ReviewSnapshot, SnapshotError> {
        self.load_reporting(workspace, &|_| {})
    }
    /// Loads while reporting stage boundaries. `progress` is called on the
    /// worker thread and must not block.
    fn load_reporting(
        &self,
        workspace: &Path,
        progress: &(dyn Fn(LoadPhase) + Sync),
    ) -> Result<ReviewSnapshot, SnapshotError>;
}

#[derive(Debug, Clone)]
pub struct WorkspaceTab {
    pub workspace: PathBuf,
    pub snapshot: Option<Arc<ReviewSnapshot>>,
    pub selected_key: Option<String>,
    pub load_generation: u64,
    pub error: Option<String>,
    /// The stage the in-flight load last reported, if one is in flight.
    pub phase: Option<LoadPhase>,
}

impl WorkspaceTab {
    pub fn loading(workspace: PathBuf) -> Self {
        Self {
            workspace,
            snapshot: None,
            selected_key: None,
            load_generation: 0,
            error: None,
            phase: None,
        }
    }
}

/// A compact sample keeps the native shell usable before a workspace loader is
/// selected and gives deterministic fixtures to UI tests.
pub fn demo_snapshot(workspace: impl Into<PathBuf>) -> ReviewSnapshot {
    let workspace = workspace.into();
    let milestone = Record {
        key: "akr.milestone.human-record-review".into(),
        title: "Human-facing record review".into(),
        kind: "milestone".into(),
        state: "proposed".into(),
        revision: 1,
        body: "A fast read-only desktop workbench for planning and knowledge review.".into(),
        freshness: "current".into(),
        plan_of_record: true,
        relations: vec![],
        acceptance: vec![AcceptanceCheck {
            id: "desktop-review-usable".into(),
            statement: "The hierarchy is navigable.".into(),
            verdict: "pending".into(),
        }],
        provenance: vec![".akr/records/akr/milestones.akr".into()],
        history: vec!["akr.milestone.human-record-review/1".into()],
        git: GitMetadata::default(),
    };
    let gui = Record {
        key: "akr.work.desktop-review-gui".into(),
        title: "Desktop review GUI".into(),
        kind: "work".into(),
        state: "proposed".into(),
        revision: 1,
        body: "Adapt the native shell into a multi-workspace AKR reviewer.".into(),
        freshness: "current".into(),
        plan_of_record: false,
        relations: vec![
            Relation {
                kind: "part_of".into(),
                target: milestone.key.clone(),
            },
            Relation {
                kind: "depends_on".into(),
                target: "akr.work.review-snapshot".into(),
            },
        ],
        acceptance: vec![],
        provenance: vec!["AKR plan".into()],
        history: vec!["akr.work.desktop-review-gui/1".into()],
        git: GitMetadata::default(),
    };
    let snapshot = Record {
        key: "akr.work.review-snapshot".into(),
        title: "Review snapshot API".into(),
        kind: "work".into(),
        state: "proposed".into(),
        revision: 1,
        body: "Expose the immutable projection consumed by human review clients.".into(),
        freshness: "current".into(),
        plan_of_record: false,
        relations: vec![Relation {
            kind: "part_of".into(),
            target: milestone.key.clone(),
        }],
        acceptance: vec![],
        provenance: vec!["AKR plan".into()],
        history: vec!["akr.work.review-snapshot/1".into()],
        git: GitMetadata::default(),
    };
    ReviewSnapshot {
        workspace,
        project: "AKR demo".into(),
        source_graph: "demo".into(),
        head: None,
        counts: ReviewCounts {
            records: 3,
            revisions: 3,
            live_planning: 3,
            ..ReviewCounts::default()
        },
        diagnostics: vec![],
        records: vec![milestone, gui, snapshot],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hierarchy_is_key_sorted_and_retains_children() {
        let snapshot = demo_snapshot("/demo");
        let roots = snapshot.planning_roots();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].key, "akr.milestone.human-record-review");
        assert_eq!(
            roots[0]
                .children
                .iter()
                .map(|node| node.key.as_str())
                .collect::<Vec<_>>(),
            ["akr.work.desktop-review-gui", "akr.work.review-snapshot"]
        );
    }
    #[test]
    fn neighborhood_is_bounded_and_stable() {
        let snapshot = demo_snapshot("/demo");
        let edges = snapshot.neighborhood("akr.work.desktop-review-gui", 2, 8);
        assert_eq!(edges.len(), 3);
        assert!(edges.windows(2).all(|pair| pair[0] <= pair[1]));
    }
    #[test]
    fn outstanding_lists_only_proposed_records_sorted_by_key() {
        let mut snapshot = demo_snapshot("/demo");
        snapshot.records[0].state = "active".into();
        snapshot.records[2].state = "completed".into();
        snapshot.records.push(Record {
            key: "akr.work.aaa-early-key".into(),
            title: "Sorts first".into(),
            kind: "work".into(),
            state: "proposed".into(),
            ..Record::default()
        });
        let keys = snapshot
            .outstanding_records()
            .iter()
            .map(|record| record.key.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            ["akr.work.aaa-early-key", "akr.work.desktop-review-gui"]
        );
        assert_eq!(snapshot.outstanding_count(), 2);
    }
}

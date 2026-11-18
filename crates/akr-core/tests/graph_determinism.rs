//! Graph algorithms, and the property that matters most about them: determinism.
//!
//! Exit criterion 3 of `docs/13-implementation-roadmap.md` P3 — "cycle reports are
//! deterministic: a graph with several cycles reports the same one on every run and after
//! input shuffling". A cycle report that moves between runs cannot be put in a fixture,
//! and a propagation path that moves between runs is a review-queue entry nobody can act
//! on.
//!
//! Shuffling uses a small seeded generator rather than a dependency: the point is
//! reproducible disorder, and sixteen lines of xorshift give exactly that.

use akr_core::graph::{DiGraph, PROPAGATING_RELATIONS, dependency_graph, propagate_staleness};
use akr_core::model::{
    Kind, Ledger, Project, Record, RecordBuilder, Relation, RevisionId, State, key,
};
use std::collections::BTreeSet;

// -------------------------------------------------------------------------------------
// Seeded shuffling
// -------------------------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = usize::try_from(self.next() % (i as u64 + 1)).expect("in range");
            items.swap(i, j);
        }
    }
}

// -------------------------------------------------------------------------------------
// Cycle detection
// -------------------------------------------------------------------------------------

/// A graph with three disjoint cycles, so "which one is reported" is a real choice.
fn three_cycles() -> Vec<(&'static str, &'static str)> {
    vec![
        ("m", "n"),
        ("n", "m"), // cycle 2
        ("a", "b"),
        ("b", "c"),
        ("c", "a"), // cycle 1
        ("x", "y"),
        ("y", "x"), // cycle 3
        ("p", "q"), // and an acyclic tail
        ("q", "r"),
    ]
}

fn build(edges: &[(&str, &str)]) -> DiGraph<String> {
    let mut graph = DiGraph::new();
    for (from, to) in edges {
        graph.add_edge((*from).to_owned(), (*to).to_owned());
    }
    graph
}

#[test]
fn cycle_report_is_stable_under_shuffling() {
    let baseline = build(&three_cycles()).find_cycle().expect("a cycle exists");
    for seed in 1..200u64 {
        let mut edges = three_cycles();
        Rng::new(seed).shuffle(&mut edges);
        assert_eq!(
            build(&edges).find_cycle(),
            Some(baseline.clone()),
            "seed {seed} reported a different cycle"
        );
    }
}

#[test]
fn cycle_report_prefers_the_lexicographically_first_start() {
    // Nodes are visited in sorted order, so the cycle through `a` wins over the ones
    // through `m` and `x`. Pinned so that a future rewrite cannot quietly change which
    // cycle a fixture expects.
    let cycle = build(&three_cycles()).find_cycle().expect("a cycle exists");
    assert_eq!(cycle.first().map(String::as_str), Some("a"));
    assert_eq!(cycle.first(), cycle.last(), "the path closes");
    assert_eq!(cycle, ["a", "b", "c", "a"]);
}

#[test]
fn acyclic_graphs_report_nothing() {
    let graph = build(&[("a", "b"), ("b", "c"), ("a", "c"), ("d", "c")]);
    assert_eq!(graph.find_cycle(), None);
    assert!(graph.is_acyclic());
}

#[test]
fn self_loops_are_cycles() {
    let graph = build(&[("a", "a")]);
    assert_eq!(
        graph.find_cycle(),
        Some(vec!["a".to_owned(), "a".to_owned()])
    );
}

#[test]
fn deep_chains_do_not_overflow_the_stack() {
    // An explicit stack rather than recursion. Ten thousand nodes is above the target
    // ledger size of `docs/06` §12, so this is comfortably a bound.
    let mut graph = DiGraph::new();
    for i in 0..10_000u32 {
        graph.add_edge(i, i + 1);
    }
    assert!(graph.is_acyclic());
    graph.add_edge(10_000u32, 0);
    assert!(graph.find_cycle().is_some());
}

// -------------------------------------------------------------------------------------
// Reachability and ordering
// -------------------------------------------------------------------------------------

#[test]
fn reachability_is_transitive_and_excludes_the_start() {
    let graph = build(&[("a", "b"), ("b", "c"), ("c", "d"), ("e", "f")]);
    let from_a = graph.reachable_from(&"a".to_owned());
    assert_eq!(
        from_a.iter().map(String::as_str).collect::<Vec<_>>(),
        ["b", "c", "d"]
    );
    assert!(graph.reaches(&"a".to_owned(), &"d".to_owned()));
    assert!(!graph.reaches(&"a".to_owned(), &"e".to_owned()));
    assert!(
        !graph.reaches(&"a".to_owned(), &"a".to_owned()),
        "a node does not reach itself without a cycle"
    );
}

#[test]
fn reachability_terminates_on_a_cycle() {
    let graph = build(&[("a", "b"), ("b", "a")]);
    assert!(graph.reaches(&"a".to_owned(), &"a".to_owned()));
}

#[test]
fn topological_order_is_stable_under_shuffling() {
    let edges = [
        ("build", "parse"),
        ("build", "resolve"),
        ("resolve", "link"),
        ("link", "parse"),
        ("emit", "resolve"),
    ];
    let baseline = build(&edges).topological_order().expect("acyclic");
    for seed in 1..100u64 {
        let mut shuffled = edges.to_vec();
        Rng::new(seed).shuffle(&mut shuffled);
        assert_eq!(build(&shuffled).topological_order(), Some(baseline.clone()));
    }
    // And it really is a topological order.
    let position = |n: &str| baseline.iter().position(|x| x == n).expect("present");
    for (from, to) in edges {
        assert!(position(from) < position(to), "{from} before {to}");
    }
}

#[test]
fn topological_order_is_none_on_a_cycle() {
    assert_eq!(build(&[("a", "b"), ("b", "a")]).topological_order(), None);
}

#[test]
fn reversed_swaps_every_edge_and_keeps_every_node() {
    let graph = build(&[("a", "b"), ("b", "c")]);
    let reversed = graph.reversed();
    assert_eq!(reversed.node_count(), graph.node_count());
    assert_eq!(reversed.edge_count(), graph.edge_count());
    assert!(reversed.reaches(&"c".to_owned(), &"a".to_owned()));
}

// -------------------------------------------------------------------------------------
// Propagation (D-024)
// -------------------------------------------------------------------------------------

/// The save-your-skin freshness graph of `MANIFEST.md` §7, in miniature: two stale
/// observations, two assessments resting on them, a policy resting on one assessment, and
/// a work item that `depends_on` a stale observation directly.
fn freshness_ledger() -> Ledger {
    let mut ledger = Ledger::new(Project::new("save-your-skin", &["sys", "sim", "lege"]));
    let records: Vec<Record> = vec![
        RecordBuilder::new("sim.obs.projection-gaps", 1, Kind::Observation)
            .filled()
            .build(),
        RecordBuilder::new("sim.obs.timestep-drift", 1, Kind::Observation)
            .filled()
            .build(),
        RecordBuilder::new("sys.assessment.projection-gaps", 1, Kind::Assessment)
            .filled()
            .rel(Relation::SupportedBy, "@sim.obs.projection-gaps")
            .build(),
        RecordBuilder::new("sys.assessment.m3-readiness", 1, Kind::Assessment)
            .filled()
            .rel(Relation::SupportedBy, "@sim.obs.timestep-drift")
            .build(),
        RecordBuilder::new("sys.policy.tandem-work", 1, Kind::Policy)
            .filled()
            .state(State::Active)
            .rel(Relation::SupportedBy, "@sys.assessment.projection-gaps")
            .build(),
        RecordBuilder::new("sim.work.rewrite-projection", 1, Kind::Work)
            .filled()
            .state(State::Blocked)
            .rel(Relation::DependsOn, "@sim.obs.projection-gaps")
            .build(),
        // Reached by `part_of` and `after` only: staleness must not travel to it.
        RecordBuilder::new("sys.milestone.m3-playable-day", 1, Kind::Milestone)
            .filled()
            .state(State::Active)
            .build(),
        RecordBuilder::new("lege.work.extract-render-graph", 1, Kind::Work)
            .filled()
            .state(State::Active)
            .rel(Relation::PartOf, "@sys.milestone.m3-playable-day")
            .build(),
    ];
    ledger.extend(records);
    ledger
}

fn id(text: &str, revision: u32) -> RevisionId {
    RevisionId::new(key(text), revision)
}

fn stale_set() -> BTreeSet<RevisionId> {
    [
        id("sim.obs.projection-gaps", 1),
        id("sim.obs.timestep-drift", 1),
    ]
    .into()
}

#[test]
fn propagation_reproduces_the_manifest_shape() {
    // MANIFEST §7, as amended: two stale records, four at risk, maximum depth 2.
    let ledger = freshness_ledger();
    let at_risk = propagate_staleness(&ledger, &stale_set());

    let names: Vec<String> = at_risk
        .iter()
        .map(|r| format!("{} depth {} via {}", r.id, r.depth, r.via))
        .collect();
    assert_eq!(
        names,
        [
            "sim.work.rewrite-projection/1 depth 1 via depends_on",
            "sys.assessment.m3-readiness/1 depth 1 via supported_by",
            "sys.assessment.projection-gaps/1 depth 1 via supported_by",
            "sys.policy.tandem-work/1 depth 2 via supported_by",
        ]
    );
    assert_eq!(at_risk.iter().map(|r| r.depth).max(), Some(2));
}

#[test]
fn propagation_records_the_path_back_to_the_stale_record() {
    let ledger = freshness_ledger();
    let at_risk = propagate_staleness(&ledger, &stale_set());
    let policy = at_risk
        .iter()
        .find(|r| r.id.key.to_string() == "sys.policy.tandem-work")
        .expect("policy is at risk");
    assert_eq!(
        policy
            .path
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        [
            "sys.assessment.projection-gaps/1",
            "sim.obs.projection-gaps/1"
        ],
        "the path names every hop, so a reader can see why"
    );
}

#[test]
fn propagation_does_not_travel_along_part_of() {
    // D-024: three relations and no others. Propagating along containment would flag half
    // the project every time a file changed.
    let ledger = freshness_ledger();
    let at_risk = propagate_staleness(&ledger, &stale_set());
    let flagged: BTreeSet<String> = at_risk.iter().map(|r| r.id.key.to_string()).collect();
    assert!(!flagged.contains("sys.milestone.m3-playable-day"));
    assert!(!flagged.contains("lege.work.extract-render-graph"));
}

#[test]
fn a_stale_record_is_never_also_at_risk() {
    let ledger = freshness_ledger();
    let stale = stale_set();
    for entry in propagate_staleness(&ledger, &stale) {
        assert!(
            !stale.contains(&entry.id),
            "{} is stale, not at risk",
            entry.id
        );
    }
}

#[test]
fn propagation_is_stable_under_shuffling() {
    let baseline = propagate_staleness(&freshness_ledger(), &stale_set());
    for seed in 1..100u64 {
        let template = freshness_ledger();
        let mut records: Vec<Record> = template.records().to_vec();
        Rng::new(seed).shuffle(&mut records);
        let mut shuffled = Ledger::new(template.project.clone());
        shuffled.extend(records);
        assert_eq!(
            propagate_staleness(&shuffled, &stale_set()),
            baseline,
            "seed {seed}"
        );
    }
}

#[test]
fn propagation_is_cycle_safe() {
    // A cyclic dependency graph is V-015's problem. Propagation must not hang while that
    // diagnostic is being produced.
    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.extend([
        RecordBuilder::new("fx.obs.a", 1, Kind::Observation)
            .filled()
            .rel(Relation::DependsOn, "@fx.obs.b")
            .build(),
        RecordBuilder::new("fx.obs.b", 1, Kind::Observation)
            .filled()
            .rel(Relation::DependsOn, "@fx.obs.a")
            .build(),
    ]);
    let stale: BTreeSet<RevisionId> = [id("fx.obs.a", 1)].into();
    let at_risk = propagate_staleness(&ledger, &stale);
    assert_eq!(at_risk.len(), 1);
    assert_eq!(at_risk[0].id, id("fx.obs.b", 1));
}

#[test]
fn the_dependency_graph_uses_exactly_three_relations() {
    assert_eq!(
        PROPAGATING_RELATIONS,
        [
            Relation::SupportedBy,
            Relation::DependsOn,
            Relation::DerivedFrom
        ]
    );
    let graph = dependency_graph(&freshness_ledger());
    // Four propagating edges in the fixture; the `part_of` edge is not one of them.
    assert_eq!(graph.edge_count(), 4);
}

#[test]
fn empty_stale_set_flags_nothing() {
    assert!(propagate_staleness(&freshness_ledger(), &BTreeSet::new()).is_empty());
}

#[test]
fn only_live_records_are_flagged_at_risk() {
    // `docs/02-data-model.md` §6 defines at_risk over live records. A superseded record
    // rests on whatever it rested on; flagging it asks for a review nobody will do.
    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.extend([
        RecordBuilder::new("fx.obs.measurement", 1, Kind::Observation)
            .filled()
            .build(),
        RecordBuilder::new("fx.assessment.retired", 1, Kind::Assessment)
            .filled()
            .state(State::Superseded)
            .rel(Relation::SupportedBy, "@fx.obs.measurement")
            .build(),
        RecordBuilder::new("fx.assessment.current", 1, Kind::Assessment)
            .filled()
            .rel(Relation::SupportedBy, "@fx.obs.measurement")
            .build(),
    ]);

    let stale: BTreeSet<RevisionId> = [id("fx.obs.measurement", 1)].into();
    let flagged: Vec<String> = propagate_staleness(&ledger, &stale)
        .into_iter()
        .map(|r| r.id.to_string())
        .collect();
    assert_eq!(flagged, ["fx.assessment.current/1"]);
}

#[test]
fn doubt_does_not_travel_through_a_terminal_record() {
    // `derived_from` is the one relation that may point at a terminal record. It is
    // provenance: what the retired finding said at the time, not a live dependency.
    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.extend([
        RecordBuilder::new("fx.obs.measurement", 1, Kind::Observation)
            .filled()
            .build(),
        RecordBuilder::new("fx.obs.retired", 1, Kind::Observation)
            .filled()
            .state(State::Disproven)
            .rel(Relation::DerivedFrom, "@fx.obs.measurement")
            .build(),
        RecordBuilder::new("fx.decision.downstream", 1, Kind::Decision)
            .filled()
            .state(State::Active)
            .rel(Relation::DerivedFrom, "@fx.obs.retired/1")
            .build(),
    ]);

    let stale: BTreeSet<RevisionId> = [id("fx.obs.measurement", 1)].into();
    assert!(
        propagate_staleness(&ledger, &stale).is_empty(),
        "the retired observation is not flagged, and nothing beyond it is either"
    );
}

//! The owned projection consumed by human review clients.

use crate::support;

use akr_cli::review_snapshot::{ReviewOptions, ReviewSnapshot};
use support::Example;

#[test]
fn review_snapshot_projects_heads_hierarchy_edges_and_inspector_data() {
    let example = Example::materialise("review-snapshot");
    let snapshot = ReviewSnapshot::load(example.root(), ReviewOptions::default())
        .expect("the review snapshot loads");

    assert!(!snapshot.project.is_empty());
    assert!(!snapshot.source_graph.is_empty());
    assert_eq!(snapshot.counts.records, snapshot.records.len());
    assert!(snapshot.counts.revisions >= snapshot.counts.records);
    assert!(
        snapshot
            .records
            .windows(2)
            .all(|pair| pair[0].key < pair[1].key)
    );
    assert!(snapshot.records.iter().all(|record| !record.id.is_empty()));
    assert!(
        snapshot
            .records
            .iter()
            .any(|record| record.parent.is_some())
    );
    assert!(
        snapshot
            .records
            .iter()
            .any(|record| !record.acceptance.is_empty())
    );
    assert!(
        snapshot
            .records
            .iter()
            .any(|record| !record.relations.is_empty())
    );
    assert!(
        snapshot
            .records
            .iter()
            .flat_map(|record| &record.relations)
            .any(|relation| relation.direction == "inbound")
    );
    assert!(
        snapshot
            .records
            .iter()
            .any(|record| !record.body.is_empty())
    );
    assert!(
        snapshot
            .records
            .iter()
            .all(|record| !record.history.is_empty())
    );
}

#[test]
fn review_snapshot_keeps_freshness_and_git_provenance_owned() {
    let example = Example::materialise("review-snapshot-freshness");
    let snapshot = ReviewSnapshot::load(example.root(), ReviewOptions::default())
        .expect("the review snapshot loads");

    assert!(snapshot.head.is_some());
    assert_eq!(
        snapshot.counts.stale,
        snapshot
            .records
            .iter()
            .filter(|record| record.freshness.status == "stale")
            .count()
    );
    assert!(
        snapshot
            .records
            .iter()
            .any(|record| record.defined_at.is_some() || record.observed_at.is_some())
    );
}

#[test]
fn review_snapshot_reports_a_missing_workspace_without_panicking() {
    let missing = std::env::temp_dir().join(format!("akr-review-missing-{}", std::process::id()));
    let error = ReviewSnapshot::load(&missing, ReviewOptions::default())
        .expect_err("a missing workspace is an error");
    assert!(error.to_string().contains("no .akr directory"));
}

//! Stage E over the worked example, with the synthetic history its MANIFEST freezes.
//!
//! The command-line side of stage E lives in `akr-cli/tests/index.rs`, which builds a
//! cache through the binary. This file covers what that one cannot reach: the descendant
//! verdict of D-016 needs a `last_change` per record and a commit ancestry, and the CLI
//! harness has neither — it lays the ledger into a working tree without committing it, so
//! no record has a history. `MANIFEST.md` §4 supplies both, which is exactly the interface
//! phase P5 fills from a real repository.

use akr_core::model::{Ancestry, Commit, RevisionId, key};
use akr_core::resolve::{BuildInputs, ResolvedModel, Workspace, evidence_links, load_workspace};
use akr_core::store::{IndexInputs, IndexStats, build, cache_path};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// `MANIFEST.md` §4: five commits, C1 through C5, with C5 as HEAD.
const COMMITS: [&str; 5] = [
    "3f0a1c9d5b7e2648a0d4f1b8c36e9752ad014b6f",
    "7c41d0ba92e6f37518a3cd406b5e2f91d8074a63",
    "b2e58f1406c7a9d3e41b60258fa3d7c6195e0b48",
    "5d9c2a70e31f8b46c07d5924ab6e3f1074c9d285",
    "e806b3f54a2d7091c5e13b8a26f490dc7b135e64",
];

fn commit(index: usize) -> Commit {
    Commit::new(COMMITS[index]).expect("a frozen commit")
}

fn id(text: &str, revision: u32) -> RevisionId {
    RevisionId::new(key(text), revision)
}

fn example_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/save-your-skin")
}

fn example() -> &'static Workspace {
    static WORKSPACE: OnceLock<Workspace> = OnceLock::new();
    WORKSPACE.get_or_init(|| {
        let root = example_root();
        let mut workspace = load_workspace(&root, &root.join(".akr")).expect("the example loads");
        workspace.ledger.facts.ancestry = Ancestry::from_pairs(vec![
            (commit(1), commit(0)),
            (commit(2), commit(1)),
            (commit(3), commit(2)),
            (commit(4), commit(3)),
        ]);
        workspace.ledger.facts.last_change = [
            (id("sys.milestone.m1-walking-skeleton", 1), commit(0)),
            (id("sys.milestone.m2-deterministic-sim", 1), commit(1)),
            (id("sys.milestone.m3-playable-day", 1), commit(2)),
            (id("sys.work.m3-plan", 2), commit(3)),
        ]
        .into();
        workspace
    })
}

fn inputs_of(workspace: &Workspace) -> BuildInputs {
    BuildInputs {
        tool: "akr 0.1.0".to_owned(),
        grammar: "0.1".to_owned(),
        vocabulary: "0.1".to_owned(),
        commit: Some(commit(4)),
        built_at: "2026-08-04T00:00:00Z".to_owned(),
        sources: workspace.inputs.sources.clone(),
        canonical_text: workspace.inputs.canonical_text.clone(),
    }
}

/// Builds an index in a scratch directory and hands back the connection and the stats.
fn built(name: &str) -> (rusqlite::Connection, IndexStats, PathBuf) {
    let workspace = example();
    let inputs = inputs_of(workspace);
    let model = ResolvedModel::build(&workspace.ledger, &inputs);
    let queue = akr_core::freshness::ReviewQueue::default();

    let dir = std::env::temp_dir().join(format!("akr-store-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch directory");
    let path = cache_path(&dir);

    let stats = build(
        &path,
        &IndexInputs {
            model: &model,
            queue: &queue,
            spans: &workspace.spans,
            diagnostics: &[],
            today: "2026-08-04",
        },
    )
    .expect("stage E builds");
    let connection = rusqlite::Connection::open(&path).expect("the cache opens");
    (connection, stats, dir)
}

fn shared_built() -> (rusqlite::Connection, &'static IndexStats) {
    static SHARED: OnceLock<(PathBuf, IndexStats)> = OnceLock::new();
    let (path, stats) = SHARED.get_or_init(|| {
        let workspace = example();
        let inputs = inputs_of(workspace);
        let model = ResolvedModel::build(&workspace.ledger, &inputs);
        let queue = akr_core::freshness::ReviewQueue::default();

        let dir = std::env::temp_dir().join(format!("akr-store-shared-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch directory");
        let path = cache_path(&dir);

        let stats = build(
            &path,
            &IndexInputs {
                model: &model,
                queue: &queue,
                spans: &workspace.spans,
                diagnostics: &[],
                today: "2026-08-04",
            },
        )
        .expect("stage E builds");
        (path, stats)
    });
    let connection = rusqlite::Connection::open(path).expect("the cache opens");
    (connection, stats)
}

fn count(connection: &rusqlite::Connection, sql: &str) -> i64 {
    connection
        .query_row(sql, [], |row| row.get(0))
        .unwrap_or_else(|error| panic!("{sql}: {error}"))
}

#[test]
fn every_citation_is_recorded_with_its_descendant_verdict() {
    let (connection, _) = shared_built();

    let links = count(&connection, "SELECT count(*) FROM evidence_links");
    assert!(links > 0, "no citation was evaluated");

    // The stored verdict is derived, not a copy of the evidence's own opinion: a check is
    // satisfied when the evidence passed *and* it was observed after the last change to
    // the thing it verifies (D-016).
    assert_eq!(
        count(
            &connection,
            "SELECT count(*) FROM evidence_links \
             WHERE satisfies <> (result = 'pass' AND descends = 1)"
        ),
        0,
        "a row's satisfies disagrees with its own result and descends"
    );

    // The table and the in-memory model agree about how many citations there are, which is
    // the property that makes the cache safe to read instead of the model.
    let expected = evidence_links(&example().ledger).len();
    assert_eq!(usize::try_from(links).expect("a count"), expected);

    // Commits are stored bare, per the DDL's conventions.
    let observed: String = connection
        .query_row(
            "SELECT observed_at FROM evidence_links LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("a row");
    assert_eq!(observed.len(), 40, "{observed}");
    assert!(!observed.starts_with("git:"), "{observed}");
}

#[test]
fn a_check_is_satisfied_only_by_a_link_that_satisfies_it() {
    let (connection, _) = shared_built();
    assert_eq!(
        count(
            &connection,
            "SELECT count(*) FROM checks c WHERE c.satisfied = 1 AND NOT EXISTS ( \
               SELECT 1 FROM evidence_links e \
                WHERE e.key = c.key AND e.rev = c.rev AND e.check_id = c.check_id \
                  AND e.satisfies = 1)"
        ),
        0,
        "a check is satisfied with nothing in evidence_links to satisfy it"
    );
    // And the converse, which is the one D-016 actually cares about: evidence that
    // predates the last content change does not close anything.
    assert_eq!(
        count(
            &connection,
            "SELECT count(*) FROM checks c WHERE c.satisfied = 0 AND EXISTS ( \
               SELECT 1 FROM evidence_links e \
                WHERE e.key = c.key AND e.rev = c.rev AND e.check_id = c.check_id \
                  AND e.satisfies = 1)"
        ),
        0,
        "a check with satisfying evidence is recorded as unsatisfied"
    );
}

#[test]
fn two_builds_of_the_same_sources_dump_identically() {
    // §11's determinism, stated the way it is actually consumed. Note what is *not*
    // claimed: §7 says the on-disk page layout is not promised to be byte-identical
    // between runs, only that every query result is. So this compares dumps rather than
    // files — a file comparison would be a stricter test of a weaker promise, and would
    // fail for reasons that do not matter.
    let (first, _) = shared_built();
    let (second, _, second_dir) = built("determinism-b");
    assert_eq!(dump(&first), dump(&second));
    drop(second);
    let _ = std::fs::remove_dir_all(second_dir);
}

/// Every table, ordered canonically, as one string.
fn dump(connection: &rusqlite::Connection) -> String {
    let tables = [
        ("meta", "key"),
        ("sources", "path"),
        ("records", "key"),
        ("revisions", "key, rev"),
        ("claims", "key, rev, anchor"),
        ("relations", "from_key, from_rev, relation, to_key, ord"),
        ("scopes", "key, rev, ord"),
        ("watches", "key, rev, ord"),
        ("checks", "key, rev, check_id"),
        (
            "evidence_links",
            "key, rev, check_id, evidence_key, evidence_rev",
        ),
        ("dispositions", "key, rev, child_key"),
        ("resolutions", "key, rev"),
    ];
    let mut out = String::new();
    for (table, order) in tables {
        out.push_str(&format!("-- {table}\n"));
        let mut statement = connection
            .prepare(&format!("SELECT * FROM {table} ORDER BY {order}"))
            .unwrap_or_else(|error| panic!("{table}: {error}"));
        let columns = statement.column_count();
        let rows = statement
            .query_map([], |row| {
                let mut line = String::new();
                for column in 0..columns {
                    let value: rusqlite::types::Value = row.get(column)?;
                    line.push_str(&format!("{value:?}|"));
                }
                Ok(line)
            })
            .expect("query");
        for row in rows {
            out.push_str(&row.expect("row"));
            out.push('\n');
        }
    }
    out
}

#[test]
fn the_index_agrees_with_the_model_it_was_built_from() {
    let (connection, stats) = shared_built();
    let workspace = example();
    let inputs = inputs_of(workspace);
    let model = ResolvedModel::build(&workspace.ledger, &inputs);

    assert_eq!(
        count(&connection, "SELECT count(*) FROM revisions"),
        i64::try_from(model.ledger().records().len()).expect("a count")
    );
    assert_eq!(
        count(&connection, "SELECT count(*) FROM records"),
        i64::try_from(model.ledger().keys().len()).expect("a count")
    );
    assert_eq!(
        count(
            &connection,
            "SELECT count(*) FROM resolutions WHERE is_head = 1"
        ),
        i64::try_from(model.heads.len()).expect("a count")
    );
    assert_eq!(stats.revisions, model.ledger().records().len());
}

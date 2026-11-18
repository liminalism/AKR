//! Stage E — the index cache (`docs/06-compiler-pipeline.md` §7), through the binary.
//!
//! The cache is a private detail (D-019), so these tests reach it the way an operator
//! debugging a build would: by opening the file `akr build` wrote. That is the one context
//! in which reading it directly is legitimate, and it is why these assertions live here
//! rather than in `akr-core` — the claim under test is that *the command* produces a cache
//! that agrees with the sources, not that a function can fill a table.

use crate::support;

use std::path::Path;
use std::process::Command;
use support::Example;

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn akr(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_akr"))
        .args(args)
        .current_dir(root)
        .output()
        .expect("akr runs")
}

/// Opens the cache and answers one scalar query.
fn scalar(example: &Example, sql: &str) -> i64 {
    let path = example.root().join(".akr/cache/index.sqlite");
    let connection = rusqlite::Connection::open(&path).expect("the cache opens");
    connection
        .query_row(sql, [], |row| row.get(0))
        .unwrap_or_else(|error| panic!("{sql}: {error}"))
}

fn text(example: &Example, sql: &str) -> String {
    let path = example.root().join(".akr/cache/index.sqlite");
    let connection = rusqlite::Connection::open(&path).expect("the cache opens");
    connection
        .query_row(sql, [], |row| row.get(0))
        .unwrap_or_else(|error| panic!("{sql}: {error}"))
}

#[test]
fn a_build_writes_a_cache_that_agrees_with_the_sources() {
    let example = Example::materialise("index-build");
    // Materialising already ran a build, so the cache is there. Removing it is how this
    // test gets to watch one being made rather than one being found current.
    std::fs::remove_dir_all(example.root().join(".akr/cache")).expect("the cache goes");
    let run = example.run(&["build"]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert!(
        example.root().join(".akr/cache/index.sqlite").exists(),
        "{}",
        run.output()
    );

    // The counts the command reported and the counts in the cache are the same counts.
    // A cache that disagreed with the summary printed beside it would be the worst kind of
    // wrong: quietly, and only for whoever read the other one.
    let revisions = scalar(&example, "SELECT count(*) FROM revisions");
    assert!(revisions > 30, "only {revisions} revisions");
    assert_eq!(
        scalar(&example, "SELECT count(*) FROM heads"),
        scalar(
            &example,
            "SELECT count(*) FROM resolutions WHERE is_head = 1"
        )
    );
    assert!(
        run.stdout
            .contains(&format!("indexed {revisions} revisions")),
        "{}",
        run.stdout
    );

    // Every table §7 step 3 names is populated, or is empty for a reason. `diagnostics` is
    // empty because the example is clean, which is itself the assertion.
    assert!(scalar(&example, "SELECT count(*) FROM sources") > 0);
    assert!(scalar(&example, "SELECT count(*) FROM records") > 0);
    assert!(scalar(&example, "SELECT count(*) FROM relations") > 0);
    assert!(scalar(&example, "SELECT count(*) FROM scopes") > 0);
    assert!(scalar(&example, "SELECT count(*) FROM checks") > 0);
    assert_eq!(scalar(&example, "SELECT count(*) FROM diagnostics"), 0);

    // `evidence_links` is empty here, and that is the correct answer rather than a gap:
    // this harness copies `.akr/` into the tree without committing it, so no record has a
    // last-change commit and there is no descendant verdict to record. D-016's judgement
    // needs git history, and `citations_are_evaluated_when_the_ledger_has_history` is
    // where it gets some.
    assert_eq!(scalar(&example, "SELECT count(*) FROM evidence_links"), 0);

    // Conventions: hashes and commits are stored bare.
    let hash = text(
        &example,
        "SELECT value FROM meta WHERE key = 'source_graph_hash'",
    );
    assert!(!hash.starts_with("sha256:"), "{hash}");
    assert_eq!(hash.len(), 64, "{hash}");
    let file_hash = text(
        &example,
        "SELECT file_hash FROM sources ORDER BY path LIMIT 1",
    );
    assert_eq!(file_hash.len(), 64, "{file_hash}");
    // The harness lays the ledger over an older committed fixture. The cache is still
    // exactly identified by the source-graph hash above, but must not pretend those
    // uncommitted bytes came from the fixture's HEAD.
    assert_eq!(
        text(&example, "SELECT value FROM meta WHERE key = 'commit'"),
        ""
    );
    let json = example.run(&["build", "--format", "json"]);
    assert_eq!(json.code, 0, "{}", json.output());
    let envelope = akr_core::json::parse(&json.stdout).expect("build emits JSON");
    assert!(
        envelope
            .get("commit")
            .is_some_and(akr_core::json::Value::is_null)
    );
    let check = example.run(&["check"]);
    assert_eq!(check.code, 0, "{}", check.output());
    assert!(check.stdout.contains("commit (none)"), "{}", check.stdout);
}

#[test]
fn a_second_build_finds_the_cache_current_and_leaves_it_alone() {
    let example = Example::materialise("index-current");
    std::fs::remove_dir_all(example.root().join(".akr/cache")).expect("the cache goes");
    assert_eq!(example.run(&["build"]).code, 0);
    let second = example.run(&["build"]);
    assert_eq!(second.code, 0, "{}", second.output());
    // §7 step 2: agreement on the source-graph hash means there is nothing to write, and
    // saying so beats rewriting a megabyte to reach the same bytes.
    assert!(
        second.stdout.contains("index cache current"),
        "{}",
        second.stdout
    );
}

#[test]
fn invalid_enum_values_are_reported_before_index_population() {
    let example = Example::materialise("index-invalid-enum");
    example.write_file(
        ".akr/records/sys/invalid-enum.akr",
        &format!(
            "akr 0.1\nproject save-your-skin\n\nrecord sys.obs.invalid-method/1 : observation {{\n    title \"Invalid method\"\n    state verified\n    statement \"\"\"\n        A deliberately invalid enum value.\n        \"\"\"\n    observed_at git:{}\n    method automated\n}}\n",
            example.commit(5)
        ),
    );

    let build = example.run(&["build"]);
    let output = build.output();
    assert_ne!(build.code, 0, "{output}");
    assert!(output.contains("AKR-T012"), "{output}");
    assert!(output.contains("sys.obs.invalid-method/1"), "{output}");
    assert!(output.contains("`automated`"), "{output}");
    assert!(!output.contains("AKR-I002"), "{output}");
}

#[test]
fn a_projection_only_commit_does_not_move_source_graph_provenance() {
    let root = std::env::temp_dir().join(format!(
        "akr-index-projection-commit-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join(".akr")).expect("workspace directory");
    std::fs::write(
        root.join(".akr/project.akr"),
        "akr 0.1\nproject fixture\n\nnamespace fx \"Fixture knowledge.\"\n\ndefaults {\n    review_after_days 90\n    view_output \"docs/generated\"\n}\n",
    )
    .expect("project source");
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.name", "AKR tests"]);
    git(&root, &["config", "user.email", "akr@example.invalid"]);
    git(&root, &["add", ".akr/project.akr"]);
    git(&root, &["commit", "-q", "-m", "commit ledger source"]);
    let source_commit = git(&root, &["rev-parse", "HEAD"]);

    let build = akr(&root, &["build"]);
    assert!(
        build.status.success(),
        "{}{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    git(&root, &["add", ".akr/akr.lock", "docs/generated"]);
    git(&root, &["commit", "-q", "-m", "commit derived projections"]);
    let projection_commit = git(&root, &["rev-parse", "HEAD"]);
    assert_ne!(source_commit, projection_commit);

    let check = akr(&root, &["build", "--check"]);
    assert!(
        check.status.success(),
        "{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    let lock = std::fs::read_to_string(root.join(".akr/akr.lock")).expect("lock");
    assert!(
        lock.contains(&format!("commit git:{source_commit}")),
        "{lock}"
    );
    assert!(
        !lock.contains(&format!("commit git:{projection_commit}")),
        "{lock}"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn editing_a_source_invalidates_the_cache_silently() {
    let example = Example::materialise("index-invalidate");
    assert_eq!(example.run(&["build"]).code, 0);
    let before = scalar(&example, "SELECT count(*) FROM revisions");

    let path = ".akr/records/sys/terms.akr";
    let source = example.read_file(path);
    example.write_file(
        path,
        &source.replace("Playable day", "Playable day, revised"),
    );

    let run = example.run(&["build"]);
    assert_eq!(run.code, 0, "{}", run.output());
    // Routine invalidation is not a diagnostic (§7 step 2) — the rebuild is silent, and
    // the only evidence of it is that the cache now says the new thing.
    assert!(!run.output().contains("AKR-I"), "{}", run.output());
    assert!(run.stdout.contains("indexed"), "{}", run.stdout);
    assert_eq!(scalar(&example, "SELECT count(*) FROM revisions"), before);
    assert_eq!(
        scalar(
            &example,
            "SELECT count(*) FROM revisions WHERE title LIKE '%, revised'"
        ),
        1
    );
}

#[test]
fn deleting_the_cache_is_always_safe() {
    // D-019's central claim, stated as a test: the cache is derivable, so losing it costs
    // one rebuild and nothing else.
    let example = Example::materialise("index-delete");
    assert_eq!(example.run(&["build"]).code, 0);
    let before = scalar(&example, "SELECT count(*) FROM revisions");

    std::fs::remove_dir_all(example.root().join(".akr/cache")).expect("the cache goes");
    let run = example.run(&["build"]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert_eq!(scalar(&example, "SELECT count(*) FROM revisions"), before);

    // And every other command works without it, because none of them read it.
    std::fs::remove_dir_all(example.root().join(".akr/cache")).expect("the cache goes again");
    let check = example.run(&["check"]);
    assert_eq!(check.code, 0, "{}", check.output());
}

#[test]
fn a_foreign_database_at_the_cache_path_is_refused() {
    let example = Example::materialise("index-foreign");
    let path = example.root().join(".akr/cache/index.sqlite");
    std::fs::create_dir_all(path.parent().expect("cache dir")).expect("mkdir");
    std::fs::remove_file(&path).expect("the real cache goes first");
    let connection = rusqlite::Connection::open(&path).expect("create");
    connection
        .execute("CREATE TABLE somebody_elses (x INTEGER)", [])
        .expect("a table");
    drop(connection);

    // §7 step 1: deleting the wrong file is the operator's decision, not the tool's.
    let run = example.run(&["build"]);
    assert_ne!(run.code, 0, "{}", run.output());
    assert!(run.output().contains("AKR-I004"), "{}", run.output());
    assert!(
        rusqlite::Connection::open(&path)
            .expect("still there")
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name = 'somebody_elses'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .expect("query")
            == 1,
        "the foreign database was clobbered"
    );
}

#[test]
fn no_rebuild_writes_no_cache() {
    let example = Example::materialise("index-no-rebuild");
    std::fs::remove_dir_all(example.root().join(".akr/cache")).expect("the cache goes");
    let run = example.run(&["--no-rebuild", "build"]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert!(
        !example.root().join(".akr/cache/index.sqlite").exists(),
        "a read-only checkout got written to"
    );
    assert!(run.stdout.contains("--no-rebuild"), "{}", run.stdout);
}

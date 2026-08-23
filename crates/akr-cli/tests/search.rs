//! P7's four exit criteria (`docs/13-implementation-roadmap.md` §3).
//!
//! 1. Search results are stable: the same query returns the same order twice.
//! 2. Deleting `.akr/cache/` and rebuilding produces identical query results.
//! 3. **No context bundle changes when the ranker is disabled** — the direct executable
//!    form of "search ranks, never authorises".
//! 4. FTS5 absent degrades to `AKR-I022` on `akr search` and affects nothing else.
//!
//! The third is the important one. The other three are about a cache behaving like a
//! cache; that one is about the boundary the whole design rests on, and it is the only
//! test here that would still matter if search were deleted tomorrow.
//!
//! The whole file needs a ranker to exist. `tests/search_degraded.rs` is its counterpart
//! for a binary built without one, where criterion 4 is the only one that still has
//! meaning — and where it is tested against the real absence rather than a simulated one.
#![cfg(feature = "fts5")]

mod support;

use support::Example;

fn cache(example: &Example) -> std::path::PathBuf {
    example.root().join(".akr/cache/index.sqlite")
}

// -------------------------------------------------------------------------------------
// Exit criterion 1 — stable order
// -------------------------------------------------------------------------------------

#[test]
fn the_same_query_returns_the_same_order_twice() {
    let example = Example::materialise("search-stable");
    assert_eq!(example.run(&["build"]).code, 0);

    let first = example.run(&["search", "projection"]);
    let second = example.run(&["search", "projection"]);
    assert_eq!(first.code, 0, "{}", first.output());
    assert_eq!(first.stdout, second.stdout);
    assert!(first.stdout.contains("results"), "{}", first.stdout);

    // BM25 alone is not a total order — two revisions can score identically, and SQLite
    // may return ties however it likes. The key tiebreak is what makes repetition a
    // guarantee rather than a habit, so a query with many equal scores is the one worth
    // asking twice.
    let broad = example.run(&["search", "the OR a OR day"]);
    let broad_again = example.run(&["search", "the OR a OR day"]);
    assert_eq!(broad.stdout, broad_again.stdout);
}

#[test]
fn a_current_index_search_does_not_invoke_git() {
    let example = Example::materialise("search-no-git-fast-path");
    assert_eq!(example.run(&["build"]).code, 0);

    let run = example.run_without_git(&["search", "projection"]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert!(run.stdout.contains("results"), "{}", run.stdout);
}

#[test]
fn zero_results_is_an_answer_and_exits_zero() {
    let example = Example::materialise("search-empty");
    assert_eq!(example.run(&["build"]).code, 0);
    let run = example.run(&["search", "gorgonzola"]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert!(run.stdout.contains("0 results"), "{}", run.stdout);
}

#[test]
fn natural_language_queries_match_any_term_and_rank_the_overlap() {
    let example = Example::materialise("search-natural-language");
    assert_eq!(example.run(&["build"]).code, 0);

    // No record contains the invented final term. Implicit AND used to turn the whole
    // orientation query into a silent miss even though `projection` has useful hits.
    let run = example.run(&["search", "projection gorgonzola"]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert!(run.stdout.contains("projection"), "{}", run.stdout);
    assert!(!run.stdout.contains("0 results"), "{}", run.stdout);
}

#[test]
fn query_flag_is_an_alias_for_the_positional_query() {
    let example = Example::materialise("search-query-alias");
    let positional = example.run(&["search", "playable day"]);
    let flagged = example.run(&["search", "--query", "playable day"]);
    assert_eq!(flagged.code, 0, "{}", flagged.output());
    assert_eq!(flagged.stdout, positional.stdout);

    let both = example.run(&["search", "playable day", "--query", "other"]);
    assert_eq!(both.code, 2, "{}", both.output());
    assert!(both.output().contains("not both"));
}

#[test]
fn filters_are_applied_before_ranking() {
    let example = Example::materialise("search-filters");
    assert_eq!(example.run(&["build"]).code, 0);

    let all = example.run(&["search", "day"]);
    let milestones = example.run(&["search", "day", "--kind", "milestone"]);
    assert_eq!(milestones.code, 0, "{}", milestones.output());
    for line in milestones.stdout.lines().filter(|l| l.starts_with("  ")) {
        assert!(line.contains(" milestone "), "{line}");
    }
    // A filter narrows the set; it does not reorder what survives it. Compared by key
    // rather than by line, because the columns are padded to the widest cell and a
    // narrower result set legitimately pads them differently.
    let keys = |run: &support::Run, only_milestones: bool| -> Vec<String> {
        run.stdout
            .lines()
            .filter(|line| line.starts_with("  "))
            .filter(|line| !only_milestones || line.contains(" milestone "))
            .filter_map(|line| line.split_whitespace().nth(1).map(ToOwned::to_owned))
            .collect()
    };
    assert_eq!(
        keys(&all, true),
        keys(&milestones, false),
        "filtering changed the surviving order"
    );
}

// -------------------------------------------------------------------------------------
// Exit criterion 2 — a rebuilt cache answers identically
// -------------------------------------------------------------------------------------

#[test]
fn deleting_the_cache_and_rebuilding_answers_identically() {
    let example = Example::materialise("search-rebuild");
    assert_eq!(example.run(&["build"]).code, 0);
    let before = example.run(&["search", "projection"]);
    assert_eq!(before.code, 0, "{}", before.output());

    std::fs::remove_dir_all(example.root().join(".akr/cache")).expect("the cache goes");
    assert_eq!(example.run(&["build"]).code, 0);
    let after = example.run(&["search", "projection"]);

    // Byte-identical, scores included. A cache that ranked differently after a rebuild
    // would make every result an artefact of when the cache happened to be made.
    assert_eq!(before.stdout, after.stdout);
}

// -------------------------------------------------------------------------------------
// Exit criterion 3 — the ranker cannot reach a bundle
// -------------------------------------------------------------------------------------

#[test]
fn no_context_bundle_changes_when_the_ranker_is_disabled() {
    // The executable form of `docs/09-context-assembly.md` §1. "Disabled" is taken at its
    // strongest: not a flag that asks the assembler to skip ranking, but the ranker
    // physically absent — no full-text table, and then no cache at all. If any record's
    // membership or order in a bundle depended on search, one of these three bundles would
    // differ from the others.
    let example = Example::materialise("search-authority");
    assert_eq!(example.run(&["build"]).code, 0);

    let goal = "sys.milestone.m3-playable-day";
    let with_ranker = example.run(&["context", "--goal", goal]);
    assert_eq!(with_ranker.code, 0, "{}", with_ranker.output());
    let json_with_ranker = example.run(&["--format", "json", "context", "--goal", goal]);

    // Drop the full-text table, which is what a cache built without FTS5 looks like.
    {
        let connection = rusqlite::Connection::open(cache(&example)).expect("the cache opens");
        connection
            .execute("DROP TABLE records_fts", [])
            .expect("the ranker goes");
    }
    let without_table = example.run(&["context", "--goal", goal]);
    assert_eq!(without_table.code, 0, "{}", without_table.output());
    assert_eq!(
        with_ranker.stdout, without_table.stdout,
        "a bundle changed when the full-text index was removed"
    );

    // And with no cache at all.
    std::fs::remove_dir_all(example.root().join(".akr/cache")).expect("the cache goes");
    let without_cache = example.run(&["context", "--goal", goal]);
    assert_eq!(without_cache.code, 0, "{}", without_cache.output());
    assert_eq!(
        with_ranker.stdout, without_cache.stdout,
        "a bundle changed when the cache was removed"
    );
    let json_without_cache = example.run(&["--format", "json", "context", "--goal", goal]);
    assert_eq!(json_with_ranker.stdout, json_without_cache.stdout);
}

#[test]
fn a_high_scoring_record_outside_the_goals_reach_stays_out_of_the_bundle() {
    // The same claim from the other side, and the one an agent would actually be harmed
    // by: a record can be the top hit for a term the goal's own title contains and still
    // have no place in the bundle. Standing comes from state, scope and relations, which
    // are declared and reviewable in a diff — never from a score.
    let example = Example::materialise("search-no-authority");
    assert_eq!(example.run(&["build"]).code, 0);

    let hits = example.run(&["search", "day"]);
    assert_eq!(hits.code, 0, "{}", hits.output());
    let ranked: Vec<String> = hits
        .stdout
        .lines()
        .filter(|line| line.starts_with("  "))
        .filter_map(|line| line.split_whitespace().nth(1).map(ToOwned::to_owned))
        .collect();
    assert!(ranked.len() > 1, "{}", hits.stdout);

    let bundle = example.run(&["context", "--goal", "sys.milestone.m5-ship-demo"]);
    assert_eq!(bundle.code, 0, "{}", bundle.output());
    let outside: Vec<&String> = ranked
        .iter()
        .filter(|hit| {
            let key = hit.split('/').next().unwrap_or(hit);
            !bundle.stdout.contains(key)
        })
        .collect();
    assert!(
        !outside.is_empty(),
        "every ranked record happened to be in the bundle, so this proves nothing: {}\n{}",
        hits.stdout,
        bundle.stdout
    );
}

// -------------------------------------------------------------------------------------
// Exit criterion 4 — FTS5 absent
// -------------------------------------------------------------------------------------

#[test]
fn a_cache_without_fts5_fails_search_and_affects_nothing_else() {
    let example = Example::materialise("search-no-fts5");
    assert_eq!(example.run(&["build"]).code, 0);
    {
        let connection = rusqlite::Connection::open(cache(&example)).expect("the cache opens");
        connection
            .execute("DROP TABLE records_fts", [])
            .expect("the table goes");
    }

    let run = example.run(&["search", "projection"]);
    assert_ne!(run.code, 0, "{}", run.output());
    assert!(run.output().contains("AKR-I022"), "{}", run.output());

    // "Affects nothing else" is the half of the criterion that is easy to skip and is the
    // reason the criterion exists: a degraded ranker must not degrade the ledger.
    for args in [
        vec!["check"],
        vec!["get", "@sys.milestone.m3-playable-day"],
        vec!["context", "--goal", "sys.milestone.m3-playable-day"],
        vec!["impact", "@sim.obs.projection-gaps"],
        vec!["review-queue"],
        vec!["view", "roadmap"],
    ] {
        let other = example.run(&args);
        assert_eq!(other.code, 0, "akr {}: {}", args.join(" "), other.output());
    }
}

#[test]
fn punctuation_is_words_and_only_raw_fts_can_be_malformed() {
    let example = Example::materialise("search-malformed");
    assert_eq!(example.run(&["build"]).code, 0);

    // These three all came back as FTS5 errors from real sessions
    // (`raw-autotune.papercut.knowledge-search-and-knowledge-start-via-the`), and the
    // agent that hit them gave up and grepped `.akr/records` — the one thing the search
    // surface exists to make unnecessary. A query is words; punctuation is not syntax.
    for query in [
        "budget, tokens",
        "HDR slice 6 non-default feature",
        "DecodeRequest::default()",
        "\"unterminated",
    ] {
        let run = example.run(&["search", query]);
        assert_eq!(run.code, 0, "{query:?}: {}", run.output());
        assert!(
            !run.output().contains("AKR-X031"),
            "{query:?} must not be a syntax error: {}",
            run.output()
        );
    }

    // Operators are still reachable, and asking for them means owning them: a malformed
    // expression under `--fts` is the caller's, and says so.
    let raw = example.run(&["search", "--fts", "\"unterminated"]);
    assert_ne!(raw.code, 0, "{}", raw.output());
    assert!(raw.output().contains("AKR-X031"), "{}", raw.output());
    assert!(
        raw.output().contains("drop --fts"),
        "the error should name the way out: {}",
        raw.output()
    );
}

// -------------------------------------------------------------------------------------
// A write between build and search refreshes the disposable cache before querying.
// -------------------------------------------------------------------------------------

#[test]
fn a_write_between_build_and_search_refreshes_before_answering() {
    let example = Example::materialise("search-after-write");
    assert_eq!(example.run(&["build"]).code, 0);

    // A fresh build already agrees with the sources, so search leaves the cache alone.
    let fresh = example.run(&["search", "projection"]);
    assert_eq!(fresh.code, 0, "{}", fresh.output());
    let fresh_json = example.run(&["--format", "json", "search", "projection"]);
    assert!(
        fresh_json
            .stdout
            .replace(' ', "")
            .contains("\"index_stale\":false"),
        "{}",
        fresh_json.stdout
    );

    // A source write does not eagerly touch the cache.
    let wrote = example.run(&[
        "papercut",
        "-m",
        "codex",
        "a small friction while testing search staleness",
        "--namespace",
        "sys",
    ]);
    assert_eq!(wrote.code, 0, "{}", wrote.output());

    // The next search refreshes the cache first and can find the just-written record.
    let refreshed = example.run(&["--format", "json", "search", "testing staleness"]);
    assert_eq!(refreshed.code, 0, "{}", refreshed.output());
    assert!(
        refreshed.stdout.contains("papercut"),
        "{}",
        refreshed.stdout
    );
    assert!(
        refreshed
            .stdout
            .replace(' ', "")
            .contains("\"index_stale\":false"),
        "{}",
        refreshed.stdout
    );
    let current = example.run(&["search", "testing staleness"]);
    assert_eq!(current.code, 0, "{}", current.output());
    assert!(
        !current.stdout.contains("stale index"),
        "{}",
        current.stdout
    );
}

#[test]
fn no_rebuild_refuses_a_stale_search_without_touching_the_cache() {
    let example = Example::materialise("search-no-rebuild");
    assert_eq!(example.run(&["build"]).code, 0);
    let cache = example.root().join(".akr/cache/index.sqlite");
    let before = std::fs::read(&cache).expect("built cache");

    let wrote = example.run(&[
        "papercut",
        "-m",
        "codex",
        "a source write that makes the search cache stale",
        "--namespace",
        "sys",
    ]);
    assert_eq!(wrote.code, 0, "{}", wrote.output());

    let refused = example.run(&["--no-rebuild", "search", "search cache stale"]);
    assert_ne!(refused.code, 0, "{}", refused.output());
    assert!(
        refused.output().contains("AKR-I031"),
        "{}",
        refused.output()
    );
    assert!(
        refused.output().contains("drop `--no-rebuild`"),
        "{}",
        refused.output()
    );
    assert_eq!(std::fs::read(&cache).expect("unchanged cache"), before);
}

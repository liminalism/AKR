//! The git layer: commits, ancestry, changed paths, and the working tree.
//!
//! Every test builds a real repository in a temp directory. Ancestry and rename detection
//! are exactly the questions where a hand-rolled model would drift from git, so the tests
//! ask git.

mod support;

#[path = "example_sys_tandem.rs"]
mod example_sys_tandem;
#[path = "freshness.rs"]
mod freshness;
#[path = "synthetic_history.rs"]
mod synthetic_history;

use akr_core::git::{GitError, Repository, ancestry_over, last_change_of, last_changes};
use akr_core::model::{Commit, Ledger, Project, RevisionId, key};
use akr_core::syntax::{lower::lower_all, parse};
use std::path::Path;
use support::TempRepo;

fn commit(hash: &str) -> Commit {
    Commit::new(hash).expect("a full hash")
}

// -------------------------------------------------------------------------------------
// Opening, ancestry, and merges
// -------------------------------------------------------------------------------------

#[test]
fn opening_a_non_repository_says_so() {
    let dir = std::env::temp_dir().join(format!("akr-p5-not-a-repo-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let error = Repository::open(&dir).expect_err("not a repository");
    assert!(matches!(error, GitError::NotARepository(_)));
    assert!(error.to_diagnostic().code == akr_core::git::codes::G001);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn repo_opening_ancestry_merges_and_rewrite_errors() {
    let mut repo = TempRepo::new("git-ancestry");
    let c1 = commit(&repo.commit_file("a.txt", "1\n", "c1"));
    repo.branch("side");
    let side = commit(&repo.commit_file("side.txt", "side\n", "on the side branch"));
    repo.checkout("main");
    let c2 = commit(&repo.commit_file("a.txt", "2\n", "c2"));
    let c3 = commit(&repo.commit_file("main.txt", "main\n", "c3"));
    let merged = commit(&repo.merge("side"));

    let git = Repository::open(repo.root()).expect("opens");
    assert_eq!(git.head().expect("head"), merged);
    let err = git.rev_parse("deadbeef").expect_err("no such revision");
    assert!(matches!(err, GitError::UnknownRevision(_)));
    assert_eq!(err.to_diagnostic().code, akr_core::git::codes::G013);

    // Reflexive, directional, and across a merge.
    assert!(git.is_descendant(&c1, &c1).expect("reflexive"));
    assert!(git.is_descendant(&c2, &c1).expect("later descends"));
    assert!(!git.is_descendant(&c1, &c2).expect("earlier does not"));
    assert!(git.is_descendant(&merged, &c1).expect("q"));
    assert!(
        git.is_descendant(&merged, &side).expect("q"),
        "a merge descends from its second parent"
    );
    assert!(git.is_descendant(&merged, &c3).expect("q"));
    assert!(!git.is_descendant(&side, &c3).expect("q"), "siblings");
    assert!(!git.is_descendant(&c3, &side).expect("q"), "siblings");

    let absent = commit("0123456789abcdef0123456789abcdef01234567");
    assert!(matches!(
        git.is_descendant(&c1, &absent),
        Err(GitError::UnknownRevision(_))
    ));

    // `ancestry_over` reproduces git answers and leaves absent commits unknown.
    let ancestry =
        ancestry_over(&git, [c1.clone(), c2.clone(), c3.clone(), absent.clone()]).expect("builds");
    for (descendant, ancestor) in [(&c3, &c1), (&c3, &c2), (&c2, &c1), (&c1, &c1)] {
        assert_eq!(ancestry.is_descendant(descendant, ancestor), Some(true));
    }
    assert_eq!(ancestry.is_descendant(&c1, &c3), Some(false));
    assert_eq!(ancestry.is_descendant(&c2, &c3), Some(false));
    assert_eq!(ancestry.is_descendant(&c1, &absent), None);

    // Malformed `.akr-commit-map` is reported instead of ignored.
    repo.write(".akr-commit-map", "not-a-commit also-not-a-commit\n");
    let map_err = Repository::open(repo.root()).expect_err("the map is invalid");
    assert!(matches!(
        map_err,
        GitError::InvalidRewriteMap { line: 1, .. }
    ));
    assert_eq!(map_err.to_diagnostic().code, akr_core::git::codes::G014);
}

#[test]
fn reviewed_rewrite_pairs_resolve_git_queries_to_the_replacement() {
    let mut repo = TempRepo::new("rewrite-map");
    repo.commit_file("base.txt", "base\n", "base");
    repo.branch("pre-rewrite");
    let old = commit(&repo.commit_file("old.txt", "old branch\n", "old identity"));
    repo.checkout("main");
    let replacement = commit(&repo.commit_file(
        "replacement.txt",
        "replacement tree\n",
        "replacement identity",
    ));
    let head = commit(&repo.commit_file("after.txt", "after\n", "after replacement"));
    repo.write(
        ".akr-commit-map",
        &format!("{} {} # reviewed during the rewrite\n", old, replacement),
    );

    let git = Repository::open(repo.root()).expect("opens");
    assert!(git.contains(&old));
    assert!(git.is_descendant(&head, &old).expect("mapped ancestry"));
    assert_eq!(
        git.file_at(&old, "replacement.txt").expect("mapped tree"),
        Some("replacement tree\n".to_owned())
    );
    assert!(
        git.run_ls_tree(&old)
            .expect("mapped listing")
            .contains("replacement.txt")
    );
    assert_eq!(
        git.touches_in(Some(&old), &head).expect("mapped range"),
        vec![akr_core::git::Touch {
            commit: head.clone(),
            path: "after.txt".to_owned(),
        }]
    );
    assert!(
        git.unreachable_from(&head, std::slice::from_ref(&old))
            .expect("mapped reachability")
            .is_empty()
    );
    assert_eq!(
        git.topological_order(&[old.clone(), head.clone()])
            .expect("mapped order"),
        vec![head, old]
    );
}

// -------------------------------------------------------------------------------------
// Changed paths, ranges, working tree, and ignores
// -------------------------------------------------------------------------------------

#[test]
fn touches_ranges_renames_deletions_working_tree_and_ignores() {
    let mut repo = TempRepo::new("git-touches");
    repo.write(".gitignore", "target/\n*.tmp\n");
    repo.write("early.rs", "1\n");
    repo.write("gone.rs", "1\n");
    repo.write(
        "engine/bridge.rs",
        "// a reasonably long file so rename detection is unambiguous\nfn main() {}\n",
    );
    let c1 = commit(&repo.commit("early"));

    let c2_base = commit(&repo.commit_file("mid.rs", "1\n", "mid"));
    repo.write("src/b.rs", "2\n");
    repo.write("docs/x.md", "x\n");
    let c3 = commit(&repo.commit("two files"));

    repo.rename("engine/bridge.rs", "engine/seam.rs");
    let c4_renamed = commit(&repo.commit("rename the bridge"));

    repo.remove("gone.rs");
    let c5_deleted = commit(&repo.commit("delete it"));

    repo.write("z.rs", "z\n");
    repo.write("mid.rs", "2\n");
    repo.write("m.rs", "m\n");
    let c6_head = commit(&repo.commit("several"));

    let git = Repository::open(repo.root()).expect("opens");

    // 1. Touches report every path and exclude commits before the range.
    let touches = git.touches_in(Some(&c2_base), &c3).expect("touches");
    let paths: Vec<&str> = touches.iter().map(|t| t.path.as_str()).collect();
    assert_eq!(paths, ["docs/x.md", "src/b.rs"]);
    assert!(touches.iter().all(|t| t.commit == c3));

    // 2. Rename reports both old and new paths; deletion is a touch.
    let rename_paths: Vec<String> = git
        .touches_in(Some(&c3), &c4_renamed)
        .expect("touches")
        .into_iter()
        .map(|t| t.path)
        .collect();
    assert!(
        rename_paths.contains(&"engine/bridge.rs".to_owned()),
        "{rename_paths:?}"
    );
    assert!(
        rename_paths.contains(&"engine/seam.rs".to_owned()),
        "{rename_paths:?}"
    );

    let del_paths: Vec<String> = git
        .touches_in(Some(&c4_renamed), &c5_deleted)
        .expect("touches")
        .into_iter()
        .map(|t| t.path)
        .collect();
    assert_eq!(del_paths, ["gone.rs"]);

    // 3. Touches are deterministic and sorted; commits_in counts the range.
    let once = git
        .touches_in(Some(&c5_deleted), &c6_head)
        .expect("touches");
    let twice = git
        .touches_in(Some(&c5_deleted), &c6_head)
        .expect("touches");
    assert_eq!(once, twice);
    assert_eq!(
        once.iter().map(|t| t.path.as_str()).collect::<Vec<_>>(),
        ["m.rs", "mid.rs", "z.rs"]
    );
    assert_eq!(
        git.commits_in(Some(&c1), &c4_renamed).expect("range").len(),
        3
    );

    // 4. Batched and per-path ignore checks agree.
    let ignore_paths = ["target/debug/**", "src/**", "scratch.tmp", "docs/*.md"];
    let one_at_a_time: Vec<bool> = ignore_paths
        .iter()
        .map(|p| git.is_ignored(p).expect("query"))
        .collect();
    assert_eq!(one_at_a_time, [true, false, true, false]);
    let batched = git.clone();
    batched.prime_ignored(ignore_paths);
    let after: Vec<bool> = ignore_paths
        .iter()
        .map(|p| batched.is_ignored(p).expect("query"))
        .collect();
    assert_eq!(after, one_at_a_time);
    batched.prime_ignored(ignore_paths);
    batched.prime_ignored(std::iter::empty());
    let again: Vec<bool> = ignore_paths
        .iter()
        .map(|p| batched.is_ignored(p).expect("query"))
        .collect();
    assert_eq!(again, one_at_a_time);

    // 5. Clean tree reports nothing; dirty/untracked files report changes on a fresh handle.
    assert!(git.working_tree_changes().expect("status").is_empty());
    repo.write("mid.rs", "edited but not committed\n");
    repo.write("new.rs", "untracked\n");
    let dirty = Repository::open(repo.root())
        .expect("opens")
        .working_tree_changes()
        .expect("status");
    assert!(dirty.contains("mid.rs"), "{dirty:?}");
    assert!(dirty.contains("new.rs"), "{dirty:?}");
}

// -------------------------------------------------------------------------------------
// last_change: the D-016 anchor
// -------------------------------------------------------------------------------------

const RECORD_V1: &str = "\
akr 0.1
project fixtures

record fx.milestone.m1/1 : milestone {
    title \"M1\"
    state active
    intent \"\"\"
        The first milestone.
        \"\"\"
    acceptance {
        check done {
            statement \"\"\"
                It is done.
                \"\"\"
            method manual
        }
    }
}
";

#[test]
fn last_change_tracks_definitional_edits_and_ignores_metadata() {
    let mut repo = TempRepo::new("last-change-all");
    let introduced = repo.commit_file(".akr/records/m.akr", RECORD_V1, "introduce");

    // 1. Untracked path is None; initial commit fills `last_changes`.
    let git0 = Repository::open(repo.root()).expect("opens");
    assert_eq!(
        last_change_of(&git0, ".akr/records/absent.akr", &key("fx.milestone.m1"), 1)
            .expect("query"),
        None
    );
    let ledger0 = load(repo.root(), ".akr/records/m.akr");
    let facts = last_changes(&git0, &ledger0).expect("query");
    assert_eq!(
        facts.get(&RevisionId::new(key("fx.milestone.m1"), 1)),
        Some(&commit(&introduced))
    );

    // 2. Unrelated file, comment, note, completion, and sibling do not move last_change.
    repo.commit_file("unrelated.txt", "a\n", "unrelated");
    let with_comment = RECORD_V1.replace(
        "    state active\n",
        "    # still the plan of record\n    state active\n",
    );
    repo.commit_file(".akr/records/m.akr", &with_comment, "add a comment");
    let with_note = with_comment.replace(
        "    intent \"\"\"\n        The first milestone.\n        \"\"\"\n",
        "    intent \"\"\"\n        The first milestone.\n        \"\"\"\n    note \"\"\"\n        A later aside.\n        \"\"\"\n",
    );
    repo.commit_file(".akr/records/m.akr", &with_note, "annotate");
    let completed = with_note
        .replace("    state active\n", "    state completed\n")
        .replace(
            "            method manual\n",
            "            method manual\n            verified_by [ @fx.evidence.done/1 ]\n",
        );
    repo.commit_file(".akr/records/m.akr", &completed, "complete");
    let with_sibling = format!(
        "{completed}\nrecord fx.milestone.m2/1 : milestone {{\n    title \"M2\"\n    state proposed\n    intent \"\"\"\n        The second milestone.\n        \"\"\"\n    acceptance {{\n        check done {{\n            statement \"\"\"\n                It is done.\n                \"\"\"\n            method manual\n        }}\n    }}\n}}\n"
    );
    repo.commit_file(".akr/records/m.akr", &with_sibling, "add a sibling");

    let git1 = Repository::open(repo.root()).expect("opens");
    assert_eq!(
        last_change_of(&git1, ".akr/records/m.akr", &key("fx.milestone.m1"), 1)
            .expect("query")
            .expect("exists")
            .as_str(),
        introduced
    );

    // 3. Changing body or check statement moves last_change.
    let edited_body =
        with_sibling.replace("The first milestone.", "The first milestone, restated.");
    let body_commit = repo.commit_file(".akr/records/m.akr", &edited_body, "restate");
    let git2 = Repository::open(repo.root()).expect("opens");
    assert_eq!(
        last_change_of(&git2, ".akr/records/m.akr", &key("fx.milestone.m1"), 1)
            .expect("query")
            .expect("exists")
            .as_str(),
        body_commit
    );

    let edited_check = edited_body.replace(
        "The second milestone.\n        \"\"\"\n    acceptance {\n        check done {\n            statement \"\"\"\n                It is done.",
        "The second milestone.\n        \"\"\"\n    acceptance {\n        check done {\n            statement \"\"\"\n                It is done, and measured.",
    );
    let check_commit = repo.commit_file(".akr/records/m.akr", &edited_check, "restate m2 check");
    let git3 = Repository::open(repo.root()).expect("opens");
    assert_eq!(
        last_change_of(&git3, ".akr/records/m.akr", &key("fx.milestone.m2"), 1)
            .expect("query")
            .expect("exists")
            .as_str(),
        check_commit
    );

    // 4. Revising without redefining preserves last_change (D-038); redefining a check moves it.
    let m3_intro = repo.commit_file(
        ".akr/records/m3.akr",
        &RECORD_V1.replace("fx.milestone.m1", "fx.milestone.m3"),
        "m3 intro",
    );
    let m3_rev2_same = format!(
        "{}\n{}",
        RECORD_V1
            .replace("fx.milestone.m1", "fx.milestone.m3")
            .trim_end()
            .replace("    state active\n", "    state superseded\n"),
        "\
record fx.milestone.m3/2 : milestone {
    title \"M1\"
    state active
    intent \"\"\"
        The first milestone.
        \"\"\"
    acceptance {
        check done {
            statement \"\"\"
                It is done.
                \"\"\"
            method manual
            verified_by [ @fx.evidence.done/1 ]
        }
    }
    supersedes [ @fx.milestone.m3/1 ]
}
"
    );
    repo.commit_file(".akr/records/m3.akr", &m3_rev2_same, "m3 revise same");
    let git4 = Repository::open(repo.root()).expect("opens");
    assert_eq!(
        last_change_of(&git4, ".akr/records/m3.akr", &key("fx.milestone.m3"), 2)
            .expect("query")
            .expect("exists")
            .as_str(),
        m3_intro,
        "revising a record is not redefining it (D-038)"
    );

    let m3_rev3_redefined = m3_rev2_same.replace(
        "record fx.milestone.m3/2 : milestone {\n    title \"M1\"\n    state active",
        "record fx.milestone.m3/2 : milestone {\n    title \"M1\"\n    state superseded",
    ) + "\n\
record fx.milestone.m3/3 : milestone {
    title \"M1\"
    state active
    intent \"\"\"
        The first milestone.
        \"\"\"
    acceptance {
        check done {
            statement \"\"\"
                It is done on every supported platform, which is a new requirement.
                \"\"\"
            method manual
        }
    }
    supersedes [ @fx.milestone.m3/2 ]
}
";
    let m3_redef = repo.commit_file(".akr/records/m3.akr", &m3_rev3_redefined, "m3 redefine");
    let git5 = Repository::open(repo.root()).expect("opens");
    assert_eq!(
        last_change_of(&git5, ".akr/records/m3.akr", &key("fx.milestone.m3"), 3)
            .expect("query")
            .expect("exists")
            .as_str(),
        m3_redef,
        "a changed check statement is a redefinition (D-029, D-038)"
    );
}

/// Parses one file into a ledger, tagging records with the path V-003 needs.
fn load(root: &Path, path: &str) -> Ledger {
    let text = std::fs::read_to_string(root.join(path)).expect("readable");
    let parsed = parse(&text, akr_core::diagnostics::FileId(0));
    let file = parsed.file.expect("parses");
    let (mut ledger, _) = lower_all(&[(path.to_owned(), file)]);
    ledger.project = Project::new("fixtures", &["fx"]);
    ledger
}

// -------------------------------------------------------------------------------------
// The memo and shared memo
// -------------------------------------------------------------------------------------

#[test]
fn memo_reachability_and_shared_memo_invalidations() {
    let mut repo = TempRepo::new("git-memo");
    let a = commit(&repo.commit_file("a.txt", "1\n", "a"));
    repo.branch("side");
    let side = commit(&repo.commit_file("side.txt", "s\n", "side"));
    repo.checkout("main");
    let b = commit(&repo.commit_file("a.txt", "2\n", "b"));
    let c = commit(&repo.commit_file("a.txt", "3\n", "c"));
    let d = commit(&repo.commit_file("a.txt", "4\n", "d"));

    // Shared memo drops when HEAD moves.
    let before = Repository::shared(repo.root())
        .expect("opens")
        .commits_touching("a.txt")
        .expect("history");
    assert_eq!(before.len(), 4);

    let head = commit(&repo.commit_file("a.txt", "5\n", "head"));
    let shared_after = Repository::shared(repo.root()).expect("opens");
    let after = shared_after.commits_touching("a.txt").expect("history");
    assert_eq!(after.len(), 5, "the new commit must be visible");
    assert_eq!(after.first(), Some(&head));
    assert!(
        shared_after
            .run_ls_tree(&head)
            .expect("tree")
            .contains("a.txt")
    );

    // Private memo: repeated questions, walk threshold, unmerged branch, absent commit.
    let git = Repository::open(repo.root()).expect("opens");
    for _ in 0..3 {
        assert!(git.contains(&a));
        assert!(git.is_descendant(&head, &a).expect("ancestry"));
        assert!(!git.is_descendant(&a, &head).expect("ancestry"));
    }
    let clone = git.clone();
    assert!(clone.contains(&head));
    assert!(clone.is_descendant(&head, &a).expect("ancestry"));
    for ancestor in [&a, &b, &c, &d] {
        assert!(git.is_descendant(&head, ancestor).expect("ancestry"));
    }
    assert!(!git.is_descendant(&head, &side).expect("ancestry"));
    assert!(git.contains(&side));
    let absent = commit("0123456789012345678901234567890123456789");
    assert!(!git.contains(&absent));
    assert!(git.is_descendant(&head, &absent).is_err());

    // Pairwise handle that never crosses the walk threshold agrees.
    let pairwise = Repository::open(repo.root()).expect("opens");
    assert!(pairwise.is_descendant(&head, &a).expect("ancestry"));
    assert!(!pairwise.is_descendant(&head, &side).expect("ancestry"));
    assert!(pairwise.is_descendant(&head, &head).expect("ancestry"));

    // Shared memo drops when working tree moves, and agrees with private handle.
    assert!(
        shared_after
            .working_tree_changes()
            .expect("status")
            .is_empty()
    );
    assert!(
        shared_after
            .working_tree_changes()
            .expect("status")
            .is_empty()
    );
    repo.write("dirty.txt", "x\n");
    let private = Repository::open(repo.root()).expect("opens");
    let shared_dirty = Repository::shared(repo.root()).expect("opens");
    assert!(
        shared_dirty
            .working_tree_changes()
            .expect("status")
            .contains("dirty.txt")
    );
    assert_eq!(
        private.working_tree_changes().expect("status"),
        shared_dirty.working_tree_changes().expect("status")
    );
    assert_eq!(
        private.commits_touching("a.txt").expect("history"),
        shared_dirty.commits_touching("a.txt").expect("history")
    );
    assert_eq!(
        private.is_descendant(&head, &a).expect("ancestry"),
        shared_dirty.is_descendant(&head, &a).expect("ancestry")
    );
    assert_eq!(
        private.file_at(&a, "a.txt").expect("blob"),
        shared_dirty.file_at(&a, "a.txt").expect("blob")
    );
    assert_eq!(
        private.touches_in(Some(&a), &head).expect("touches"),
        shared_dirty.touches_in(Some(&a), &head).expect("touches")
    );
}

// -------------------------------------------------------------------------------------
// V-105 source hits
// -------------------------------------------------------------------------------------

#[test]
fn tracked_source_hits_handles_source_dumps_overlaps_and_scale() {
    let mut repo = TempRepo::new("source-hits");
    repo.write("trace.jsonl", "a_name_only_in_jsonl\n");
    let mut text = String::from(
        "fn a_real_leaf_test() {}\nfn something_else() {}\nfn continuous_dinner_departure() {}\nmod keybindings;\n",
    );
    for i in 0..400 {
        text.push_str(&format!("fn test_case_{i}() {{ a_b(); c_d(); e_f(); }}\n"));
    }
    repo.write("src/lib.rs", &text);
    let head = commit(&repo.commit("source plus dump"));
    let git = Repository::open(repo.root()).expect("opens");

    let hits = git
        .tracked_source_hits(
            &head,
            [
                "a_real_leaf_test",
                "a_missing_name",
                "a_name_only_in_jsonl",
                "_",
                "something_else",
                "continuous_dinner_departure",
                "dinner",
                "bindings",
            ],
            None,
        )
        .expect("grep");
    assert!(hits.contains("a_real_leaf_test"));
    assert!(!hits.contains("a_missing_name"));
    assert!(!hits.contains("a_name_only_in_jsonl"));
    assert!(!hits.contains("_"));
    assert!(hits.contains("something_else"));
    assert!(hits.contains("continuous_dinner_departure"));
    assert!(hits.contains("dinner"));
    assert!(hits.contains("bindings"));

    let needles: Vec<String> = (0..200).map(|i| format!("test_case_{}", i * 2)).collect();
    let scaled = git
        .tracked_source_hits(&head, needles.iter().map(String::as_str), None)
        .expect("grep");
    assert_eq!(scaled.len(), needles.len());
}

#[test]
#[ignore = "2000-needle performance check: run manually"]
fn tracked_source_hits_cost_does_not_grow_with_needles() {
    let mut repo = TempRepo::new("source-hits-many");
    let mut text = String::new();
    for i in 0..4000 {
        text.push_str(&format!("fn test_case_{i}() {{ a_b(); c_d(); e_f(); }}\n"));
    }
    let head = commit(&repo.commit_file("src/lib.rs", &text, "source"));
    let git = Repository::open(repo.root()).expect("opens");
    let needles: Vec<String> = (0..2000).map(|i| format!("test_case_{}", i * 3)).collect();
    let started = std::time::Instant::now();
    let hits = git
        .tracked_source_hits(&head, needles.iter().map(String::as_str), None)
        .expect("grep");
    let elapsed = started.elapsed();
    assert_eq!(
        hits.len(),
        needles.iter().filter(|n| text.contains(n.as_str())).count()
    );
    assert!(
        elapsed < std::time::Duration::from_secs(20),
        "2000 needles took {elapsed:?}"
    );
}

#[test]
fn dag_ancestry_over_preserves_both_branch_parents_across_a_merge() {
    let mut repo = TempRepo::new("dag-ancestry-merge");
    let base = commit(&repo.commit_file("shared.txt", "base\n", "base"));

    repo.branch("right");
    let r1 = commit(&repo.commit_file("right.txt", "r1\n", "r1"));
    let r2 = commit(&repo.commit_file("right.txt", "r2\n", "r2"));

    repo.checkout("main");
    let l1 = commit(&repo.commit_file("left.txt", "l1\n", "l1"));
    let l2 = commit(&repo.commit_file("left.txt", "l2\n", "l2"));

    let head = commit(&repo.merge("right"));

    let git = Repository::open(repo.root()).expect("opens");
    let ancestry = ancestry_over(
        &git,
        [
            base.clone(),
            l1.clone(),
            l2.clone(),
            r1.clone(),
            r2.clone(),
            head.clone(),
        ],
    )
    .expect("ancestry_over");

    // Both branch chains and the merge commit descend from their respective ancestors.
    assert_eq!(ancestry.is_descendant(&l2, &l1), Some(true));
    assert_eq!(ancestry.is_descendant(&r2, &r1), Some(true));
    assert_eq!(ancestry.is_descendant(&head, &l2), Some(true));
    assert_eq!(ancestry.is_descendant(&head, &r2), Some(true));
    assert_eq!(ancestry.is_descendant(&head, &l1), Some(true));
    assert_eq!(ancestry.is_descendant(&head, &r1), Some(true));
    assert_eq!(ancestry.is_descendant(&head, &base), Some(true));

    // Sibling branch commits do NOT descend from each other (unlike a linearised topo list).
    assert_eq!(ancestry.is_descendant(&l2, &r1), Some(false));
    assert_eq!(ancestry.is_descendant(&r2, &l1), Some(false));
}

#[test]
fn no_ff_merge_into_older_main_does_not_falsely_stale_branch_observation() {
    use akr_core::model::{ContentSlot, ContentValue, Date, Glob, Kind, Ledger, RecordBuilder};

    let mut repo = TempRepo::new("no-ff-merge-freshness");
    let _base = commit(&repo.commit_file("src/engine.rs", "v1\n", "base"));

    repo.branch("feature");
    let f1 = commit(&repo.commit_file("src/engine.rs", "v2\n", "update engine on feature"));

    repo.checkout("main");
    let _m1 = commit(&repo.commit_file("README.md", "docs\n", "unrelated main commit"));
    let merge_head = commit(&repo.merge("feature"));

    let mut ledger = Ledger::new(Project::new("sys", &["sys"]));
    let obs = RecordBuilder::new("sys.obs.engine", 1, Kind::Observation)
        .title("Engine state")
        .content(ContentSlot::Statement, ContentValue::prose("Observed v2."))
        .content(ContentSlot::ObservedAt, ContentValue::Commit(f1.clone()))
        .content(
            ContentSlot::Watches,
            ContentValue::Globs(vec![Glob::new("src/engine.rs")]),
        )
        .build();
    ledger.insert(obs);

    let git = Repository::open(repo.root()).expect("opens");
    let today = Date::parse("2026-11-22").unwrap();
    let queue = akr_core::freshness::derive(&ledger, &git, &merge_head, today).expect("derive");
    assert!(
        queue.stale.is_empty(),
        "merge commit preserved feature's src/engine.rs, so observation at f1 must stay fresh: {:?}",
        queue.stale
    );

    // A genuine post-merge modification to src/engine.rs DOES mark the observation stale.
    let after = commit(&repo.commit_file("src/engine.rs", "v3\n", "post-merge edit"));
    let git2 = Repository::open(repo.root()).expect("opens");
    let queue2 = akr_core::freshness::derive(&ledger, &git2, &after, today).expect("derive");
    assert_eq!(queue2.stale.len(), 1);
}

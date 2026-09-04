//! The git layer: commits, ancestry, changed paths, and the working tree.
//!
//! Every test builds a real repository in a temp directory. Ancestry and rename detection
//! are exactly the questions where a hand-rolled model would drift from git, so the tests
//! ask git.

mod support;

use akr_core::git::{GitError, Repository, ancestry_over, last_change_of, last_changes};
use akr_core::model::{Commit, Ledger, LogicalKey, Project, RevisionId, key};
use akr_core::syntax::{lower::lower_all, parse};
use std::path::Path;
use support::TempRepo;

fn commit(hash: &str) -> Commit {
    Commit::new(hash).expect("a full hash")
}

// -------------------------------------------------------------------------------------
// Opening
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
fn an_unknown_revision_is_reported_as_such() {
    let mut repo = TempRepo::new("unknown-rev");
    repo.commit_file("a.txt", "one\n", "first");
    let git = Repository::open(repo.root()).expect("opens");
    let error = git.rev_parse("deadbeef").expect_err("no such revision");
    assert!(matches!(error, GitError::UnknownRevision(_)));
    assert_eq!(error.to_diagnostic().code, akr_core::git::codes::G013);
}

#[test]
fn malformed_rewrite_maps_are_reported_instead_of_ignored() {
    let mut repo = TempRepo::new("bad-rewrite-map");
    repo.commit_file("a.txt", "one\n", "first");
    repo.write(".akr-commit-map", "not-a-commit also-not-a-commit\n");

    let error = Repository::open(repo.root()).expect_err("the map is invalid");
    assert!(matches!(error, GitError::InvalidRewriteMap { line: 1, .. }));
    assert_eq!(error.to_diagnostic().code, akr_core::git::codes::G014);
}

#[test]
fn head_resolves_to_the_latest_commit() {
    let mut repo = TempRepo::new("head");
    repo.commit_file("a.txt", "one\n", "first");
    let second = repo.commit_file("a.txt", "two\n", "second");
    let git = Repository::open(repo.root()).expect("opens");
    assert_eq!(git.head().expect("head").as_str(), second);
}

// -------------------------------------------------------------------------------------
// Ancestry
// -------------------------------------------------------------------------------------

#[test]
fn ancestry_is_reflexive_and_directional() {
    let mut repo = TempRepo::new("ancestry");
    let first = commit(&repo.commit_file("a.txt", "1\n", "first"));
    let second = commit(&repo.commit_file("a.txt", "2\n", "second"));
    let git = Repository::open(repo.root()).expect("opens");

    assert!(git.is_descendant(&first, &first).expect("reflexive"));
    assert!(git.is_descendant(&second, &first).expect("later descends"));
    assert!(
        !git.is_descendant(&first, &second)
            .expect("earlier does not")
    );
}

#[test]
fn ancestry_across_a_merge_is_answered_by_git() {
    // A hand-rolled first-parent walk gets this wrong; `merge-base --is-ancestor` does not.
    let mut repo = TempRepo::new("merge");
    let base = commit(&repo.commit_file("a.txt", "base\n", "base"));
    repo.branch("side");
    let side = commit(&repo.commit_file("side.txt", "side\n", "on the side branch"));
    repo.checkout("main");
    let main = commit(&repo.commit_file("main.txt", "main\n", "on main"));
    let merged = commit(&repo.merge("side"));

    let git = Repository::open(repo.root()).expect("opens");
    assert!(git.is_descendant(&merged, &base).expect("q"));
    assert!(
        git.is_descendant(&merged, &side).expect("q"),
        "a merge descends from its second parent"
    );
    assert!(git.is_descendant(&merged, &main).expect("q"));
    assert!(!git.is_descendant(&side, &main).expect("q"), "siblings");
    assert!(!git.is_descendant(&main, &side).expect("q"), "siblings");
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

#[test]
fn ancestry_of_a_missing_commit_is_an_error_not_a_false() {
    let mut repo = TempRepo::new("missing-ancestry");
    let only = commit(&repo.commit_file("a.txt", "1\n", "first"));
    let git = Repository::open(repo.root()).expect("opens");
    let absent = commit("0123456789abcdef0123456789abcdef01234567");
    assert!(matches!(
        git.is_descendant(&only, &absent),
        Err(GitError::UnknownRevision(_))
    ));
}

#[test]
fn ancestry_over_reproduces_git_answers_on_linear_history() {
    let mut repo = TempRepo::new("ancestry-over");
    let c1 = commit(&repo.commit_file("a.txt", "1\n", "c1"));
    let c2 = commit(&repo.commit_file("a.txt", "2\n", "c2"));
    let c3 = commit(&repo.commit_file("a.txt", "3\n", "c3"));
    let git = Repository::open(repo.root()).expect("opens");

    let ancestry = ancestry_over(&git, [c1.clone(), c2.clone(), c3.clone()]).expect("builds");
    for (descendant, ancestor) in [(&c3, &c1), (&c3, &c2), (&c2, &c1), (&c1, &c1)] {
        assert_eq!(
            ancestry.is_descendant(descendant, ancestor),
            Some(true),
            "{descendant} should descend from {ancestor}"
        );
    }
    assert_eq!(ancestry.is_descendant(&c1, &c3), Some(false));
    assert_eq!(ancestry.is_descendant(&c2, &c3), Some(false));
}

#[test]
fn ancestry_over_ignores_commits_the_repository_does_not_have() {
    let mut repo = TempRepo::new("ancestry-filter");
    let c1 = commit(&repo.commit_file("a.txt", "1\n", "c1"));
    let git = Repository::open(repo.root()).expect("opens");
    let absent = commit("0123456789abcdef0123456789abcdef01234567");
    let ancestry = ancestry_over(&git, [c1.clone(), absent.clone()]).expect("builds");
    assert_eq!(
        ancestry.is_descendant(&c1, &absent),
        None,
        "an unknown commit stays unknown rather than becoming a false answer"
    );
}

// -------------------------------------------------------------------------------------
// Changed paths
// -------------------------------------------------------------------------------------

#[test]
fn touches_report_every_path_a_range_changed() {
    let mut repo = TempRepo::new("touches");
    let base = commit(&repo.commit_file("src/a.rs", "1\n", "base"));
    repo.write("src/b.rs", "2\n");
    repo.write("docs/x.md", "x\n");
    let second = commit(&repo.commit("two files"));
    let git = Repository::open(repo.root()).expect("opens");

    let touches = git.touches_in(Some(&base), &second).expect("touches");
    let paths: Vec<&str> = touches.iter().map(|t| t.path.as_str()).collect();
    assert_eq!(paths, ["docs/x.md", "src/b.rs"]);
    assert!(touches.iter().all(|t| t.commit == second));
}

#[test]
fn touches_exclude_commits_before_the_range() {
    let mut repo = TempRepo::new("touch-range");
    repo.commit_file("early.rs", "1\n", "early");
    let base = commit(&repo.commit_file("mid.rs", "1\n", "mid"));
    let head = commit(&repo.commit_file("late.rs", "1\n", "late"));
    let git = Repository::open(repo.root()).expect("opens");

    let paths: Vec<String> = git
        .touches_in(Some(&base), &head)
        .expect("touches")
        .into_iter()
        .map(|t| t.path)
        .collect();
    assert_eq!(paths, ["late.rs"]);
}

#[test]
fn a_rename_reports_both_paths() {
    // An observation watching the old location and one watching the new location have
    // each had their subject moved out from under them.
    let mut repo = TempRepo::new("rename");
    let base = commit(&repo.commit_file(
        "engine/bridge.rs",
        "// a reasonably long file so rename detection is unambiguous\nfn main() {}\n",
        "base",
    ));
    repo.rename("engine/bridge.rs", "engine/seam.rs");
    let renamed = commit(&repo.commit("rename the bridge"));
    let git = Repository::open(repo.root()).expect("opens");

    let paths: Vec<String> = git
        .touches_in(Some(&base), &renamed)
        .expect("touches")
        .into_iter()
        .map(|t| t.path)
        .collect();
    assert!(paths.contains(&"engine/bridge.rs".to_owned()), "{paths:?}");
    assert!(paths.contains(&"engine/seam.rs".to_owned()), "{paths:?}");
}

#[test]
fn a_deletion_is_a_touch() {
    let mut repo = TempRepo::new("delete");
    let base = commit(&repo.commit_file("gone.rs", "1\n", "base"));
    repo.remove("gone.rs");
    let deleted = commit(&repo.commit("delete it"));
    let git = Repository::open(repo.root()).expect("opens");
    let paths: Vec<String> = git
        .touches_in(Some(&base), &deleted)
        .expect("touches")
        .into_iter()
        .map(|t| t.path)
        .collect();
    assert_eq!(paths, ["gone.rs"]);
}

#[test]
fn touches_are_deterministic() {
    let mut repo = TempRepo::new("touch-determinism");
    let base = commit(&repo.commit_file("a.rs", "1\n", "base"));
    repo.write("z.rs", "z\n");
    repo.write("a.rs", "2\n");
    repo.write("m.rs", "m\n");
    let head = commit(&repo.commit("several"));
    let git = Repository::open(repo.root()).expect("opens");
    let once = git.touches_in(Some(&base), &head).expect("touches");
    let twice = git.touches_in(Some(&base), &head).expect("touches");
    assert_eq!(once, twice);
    let paths: Vec<&str> = once.iter().map(|t| t.path.as_str()).collect();
    assert_eq!(paths, ["a.rs", "m.rs", "z.rs"], "sorted");
}

#[test]
fn commits_in_a_range_are_counted() {
    let mut repo = TempRepo::new("range-count");
    let base = commit(&repo.commit_file("a.rs", "1\n", "base"));
    repo.commit_file("a.rs", "2\n", "two");
    repo.commit_file("a.rs", "3\n", "three");
    let head = commit(&repo.commit_file("a.rs", "4\n", "four"));
    let git = Repository::open(repo.root()).expect("opens");
    assert_eq!(git.commits_in(Some(&base), &head).expect("range").len(), 3);
}

#[test]
fn the_working_tree_reports_uncommitted_changes() {
    let mut repo = TempRepo::new("dirty");
    repo.commit_file("a.rs", "1\n", "base");
    repo.write("a.rs", "edited but not committed\n");
    repo.write("new.rs", "untracked\n");
    let git = Repository::open(repo.root()).expect("opens");
    let dirty = git.working_tree_changes().expect("status");
    assert!(dirty.contains("a.rs"), "{dirty:?}");
    assert!(dirty.contains("new.rs"), "{dirty:?}");
}

#[test]
fn a_clean_tree_reports_nothing() {
    let mut repo = TempRepo::new("clean");
    repo.commit_file("a.rs", "1\n", "base");
    let git = Repository::open(repo.root()).expect("opens");
    assert!(git.working_tree_changes().expect("status").is_empty());
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
fn last_change_is_the_commit_that_gave_a_record_its_current_text() {
    let mut repo = TempRepo::new("last-change");
    let introduced = repo.commit_file(".akr/records/m.akr", RECORD_V1, "introduce");
    // Two commits that touch the file without changing the record's content.
    repo.commit_file("unrelated.txt", "a\n", "unrelated");
    let with_comment = RECORD_V1.replace(
        "    state active\n",
        "    # still the plan of record\n    state active\n",
    );
    repo.commit_file(".akr/records/m.akr", &with_comment, "add a comment");

    let git = Repository::open(repo.root()).expect("opens");
    let answer = last_change_of(&git, ".akr/records/m.akr", &key("fx.milestone.m1"), 1)
        .expect("query")
        .expect("the record exists");
    assert_eq!(
        answer.as_str(),
        introduced,
        "a comment is not a content change (spec/schema/akr-lock.md §3.3)"
    );
}

#[test]
fn last_change_moves_when_the_record_body_changes() {
    let mut repo = TempRepo::new("last-change-moves");
    repo.commit_file(".akr/records/m.akr", RECORD_V1, "introduce");
    let edited = RECORD_V1.replace("The first milestone.", "The first milestone, restated.");
    let changed = repo.commit_file(".akr/records/m.akr", &edited, "restate");

    let git = Repository::open(repo.root()).expect("opens");
    let answer = last_change_of(&git, ".akr/records/m.akr", &key("fx.milestone.m1"), 1)
        .expect("query")
        .expect("the record exists");
    assert_eq!(answer.as_str(), changed);
}

#[test]
fn last_change_ignores_edits_to_other_records_in_the_same_file() {
    // Adding a record to a file must not invalidate every acceptance check in it.
    let mut repo = TempRepo::new("last-change-sibling");
    let introduced = repo.commit_file(".akr/records/m.akr", RECORD_V1, "introduce");
    let with_sibling = format!(
        "{RECORD_V1}\nrecord fx.milestone.m2/1 : milestone {{\n    title \"M2\"\n    state proposed\n    intent \"\"\"\n        The second milestone.\n        \"\"\"\n    acceptance {{\n        check done {{\n            statement \"\"\"\n                It is done.\n                \"\"\"\n            method manual\n        }}\n    }}\n}}\n"
    );
    repo.commit_file(".akr/records/m.akr", &with_sibling, "add a sibling");

    let git = Repository::open(repo.root()).expect("opens");
    let answer = last_change_of(&git, ".akr/records/m.akr", &key("fx.milestone.m1"), 1)
        .expect("query")
        .expect("the record exists");
    assert_eq!(answer.as_str(), introduced);
}

#[test]
fn last_change_is_none_for_an_untracked_record() {
    let mut repo = TempRepo::new("last-change-untracked");
    repo.commit_file("a.txt", "1\n", "base");
    let git = Repository::open(repo.root()).expect("opens");
    assert_eq!(
        last_change_of(&git, ".akr/records/absent.akr", &key("fx.milestone.m1"), 1).expect("query"),
        None
    );
}

#[test]
fn a_completion_does_not_move_last_change() {
    // D-029: `akr complete` sets state -> completed and adds a `verified_by` to each check.
    // Neither redefines the milestone, so its last content change stays at its definition
    // commit and the evidence that closes it is not stranded (D-016 / V-020, AKR-R022).
    let mut repo = TempRepo::new("last-change-completion");
    let introduced = repo.commit_file(".akr/records/m.akr", RECORD_V1, "introduce");
    let completed = RECORD_V1
        .replace("    state active\n", "    state completed\n")
        .replace(
            "            method manual\n",
            "            method manual\n            verified_by [ @fx.evidence.done/1 ]\n",
        );
    repo.commit_file(".akr/records/m.akr", &completed, "complete");

    let git = Repository::open(repo.root()).expect("opens");
    let answer = last_change_of(&git, ".akr/records/m.akr", &key("fx.milestone.m1"), 1)
        .expect("query")
        .expect("the record exists");
    assert_eq!(
        answer.as_str(),
        introduced,
        "completing a milestone is not a definitional change (D-029)"
    );
}

#[test]
fn a_revision_that_redefines_nothing_does_not_move_last_change() {
    // D-038. A sealed record can only be changed by revising it, and revision 2 did not
    // exist before the commit that wrote it — so by the pre-D-038 reading its last
    // definitional change was always that commit, and every citation it carried was
    // instantly too old. Here revision 2 differs from revision 1 only in the ways
    // revising a record always differs: the revision number, the `supersedes` edge back,
    // the sealed predecessor's state, and a check citation.
    let mut repo = TempRepo::new("last-change-revision");
    let introduced = repo.commit_file(".akr/records/m.akr", RECORD_V1, "introduce");
    let revised = format!(
        "{}\n{}",
        RECORD_V1
            .trim_end()
            .replace("    state active\n", "    state superseded\n"),
        "\
record fx.milestone.m1/2 : milestone {
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
    supersedes [ @fx.milestone.m1/1 ]
}
"
    );
    repo.commit_file(".akr/records/m.akr", &revised, "revise");

    let git = Repository::open(repo.root()).expect("opens");
    let answer = last_change_of(&git, ".akr/records/m.akr", &key("fx.milestone.m1"), 2)
        .expect("query")
        .expect("the record exists");
    assert_eq!(
        answer.as_str(),
        introduced,
        "revising a record is not redefining it (D-038)"
    );
}

#[test]
fn a_revision_that_redefines_a_check_does_move_last_change() {
    // The other half of D-038: it narrows what counts as a redefinition, it does not
    // abolish the gate. A changed check statement is exactly what the gate exists to
    // catch, so evidence from before it must still be too old.
    let mut repo = TempRepo::new("last-change-revision-redefines");
    let introduced = repo.commit_file(".akr/records/m.akr", RECORD_V1, "introduce");
    let revised = format!(
        "{}\n{}",
        RECORD_V1
            .trim_end()
            .replace("    state active\n", "    state superseded\n"),
        "\
record fx.milestone.m1/2 : milestone {
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
    supersedes [ @fx.milestone.m1/1 ]
}
"
    );
    let redefined = repo.commit_file(".akr/records/m.akr", &revised, "redefine");

    let git = Repository::open(repo.root()).expect("opens");
    let answer = last_change_of(&git, ".akr/records/m.akr", &key("fx.milestone.m1"), 2)
        .expect("query")
        .expect("the record exists");
    assert_eq!(
        answer.as_str(),
        redefined,
        "a changed check statement is a redefinition (D-029, D-038)"
    );
    assert_ne!(answer.as_str(), introduced);
}

#[test]
fn a_note_does_not_move_last_change() {
    // D-026 `note` is commentary; D-029 keeps it out of the definitional hash, so annotating
    // a completed record later does not re-strand its evidence.
    let mut repo = TempRepo::new("last-change-note");
    let introduced = repo.commit_file(".akr/records/m.akr", RECORD_V1, "introduce");
    let annotated = RECORD_V1.replace(
        "    intent \"\"\"\n        The first milestone.\n        \"\"\"\n",
        "    intent \"\"\"\n        The first milestone.\n        \"\"\"\n    note \"\"\"\n        A later aside.\n        \"\"\"\n",
    );
    repo.commit_file(".akr/records/m.akr", &annotated, "annotate");

    let git = Repository::open(repo.root()).expect("opens");
    let answer = last_change_of(&git, ".akr/records/m.akr", &key("fx.milestone.m1"), 1)
        .expect("query")
        .expect("the record exists");
    assert_eq!(
        answer.as_str(),
        introduced,
        "a note is not a definitional change (D-029)"
    );
}

#[test]
fn last_change_moves_when_a_check_statement_changes() {
    // The other half of D-029: a real redefinition (a check's statement) still moves the
    // last change, so stale evidence cannot close a milestone whose requirements changed.
    let mut repo = TempRepo::new("last-change-check");
    repo.commit_file(".akr/records/m.akr", RECORD_V1, "introduce");
    let edited = RECORD_V1.replace("It is done.", "It is done, and measured.");
    let changed = repo.commit_file(".akr/records/m.akr", &edited, "restate the check");

    let git = Repository::open(repo.root()).expect("opens");
    let answer = last_change_of(&git, ".akr/records/m.akr", &key("fx.milestone.m1"), 1)
        .expect("query")
        .expect("the record exists");
    assert_eq!(answer.as_str(), changed);
}

#[test]
fn last_changes_fills_the_ledger_facts() {
    let mut repo = TempRepo::new("last-changes");
    let introduced = repo.commit_file(".akr/records/m.akr", RECORD_V1, "introduce");
    let git = Repository::open(repo.root()).expect("opens");

    let ledger = load(repo.root(), ".akr/records/m.akr");
    let facts = last_changes(&git, &ledger).expect("query");
    assert_eq!(
        facts.get(&RevisionId::new(key("fx.milestone.m1"), 1)),
        Some(&commit(&introduced))
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

#[test]
fn a_logical_key_round_trips_through_git_queries() {
    // Guards the string comparison `last_change_of` makes between the CST's key text and
    // the model's key.
    let parsed = LogicalKey::parse("fx.milestone.m1").expect("valid");
    assert_eq!(parsed.to_string(), "fx.milestone.m1");
}

// -------------------------------------------------------------------------------------
// The memo
// -------------------------------------------------------------------------------------

#[test]
fn repeated_questions_are_answered_from_the_memo() {
    let mut repo = TempRepo::new("memo");
    let first = repo.commit_file("a.txt", "one\n", "first");
    let second = repo.commit_file("a.txt", "two\n", "second");
    let git = Repository::open(repo.root()).expect("opens");
    let (first, second) = (commit(&first), commit(&second));

    // Asking the same question repeatedly must not change the answer.
    for _ in 0..3 {
        assert!(git.contains(&first));
        assert!(git.is_descendant(&second, &first).expect("ancestry"));
        assert!(!git.is_descendant(&first, &second).expect("ancestry"));
    }
    // A fresh handle re-asks git rather than inheriting the memo.
    let clone = git.clone();
    assert!(clone.contains(&second));
    assert!(clone.is_descendant(&second, &first).expect("ancestry"));
    // An absent object is still absent, and still not an ancestor of anything.
    let absent = commit("0123456789012345678901234567890123456789");
    assert!(!git.contains(&absent));
    assert!(git.is_descendant(&second, &absent).is_err());
}

#[test]
fn batched_and_per_path_ignore_checks_agree() {
    let mut repo = TempRepo::new("ignored");
    repo.write(".gitignore", "target/\n*.tmp\n");
    repo.write("src/main.rs", "fn main() {}\n");
    repo.commit("ignores");
    let git = Repository::open(repo.root()).expect("opens");

    let paths = ["target/debug/**", "src/**", "scratch.tmp", "docs/*.md"];
    // Per-path answers first, from an unprimed handle.
    let one_at_a_time: Vec<bool> = paths
        .iter()
        .map(|path| git.is_ignored(path).expect("query"))
        .collect();
    assert_eq!(one_at_a_time, [true, false, true, false]);

    // The batch must reach the same verdicts on a handle that has not seen them.
    let batched = git.clone();
    batched.prime_ignored(paths);
    let after: Vec<bool> = paths
        .iter()
        .map(|path| batched.is_ignored(path).expect("query"))
        .collect();
    assert_eq!(after, one_at_a_time);

    // Priming twice, or priming nothing, changes nothing.
    batched.prime_ignored(paths);
    batched.prime_ignored(std::iter::empty());
    let again: Vec<bool> = paths
        .iter()
        .map(|path| batched.is_ignored(path).expect("query"))
        .collect();
    assert_eq!(again, one_at_a_time);
}

#[test]
fn walking_history_reaches_the_same_verdicts_as_pairwise_ancestry() {
    // Past a few questions about one commit, the handle walks that commit's history once
    // and answers from the walk. The verdicts must not change — including the two cases
    // the walk alone cannot distinguish: a commit that exists but is unreachable, and a
    // commit that is not in the repository at all.
    let mut repo = TempRepo::new("reachability");
    let a = commit(&repo.commit_file("a.txt", "1\n", "a"));
    repo.branch("side");
    let side = commit(&repo.commit_file("side.txt", "s\n", "side"));
    repo.checkout("main");
    let b = commit(&repo.commit_file("a.txt", "2\n", "b"));
    let c = commit(&repo.commit_file("a.txt", "3\n", "c"));
    let d = commit(&repo.commit_file("a.txt", "4\n", "d"));
    let head = commit(&repo.commit_file("a.txt", "5\n", "head"));
    let git = Repository::open(repo.root()).expect("opens");

    // Enough distinct questions about `head` to trip the walk, then the ones that matter.
    for ancestor in [&a, &b, &c, &d] {
        assert!(
            git.is_descendant(&head, ancestor).expect("ancestry"),
            "{ancestor} is on the first-parent line"
        );
    }
    assert!(
        !git.is_descendant(&head, &side).expect("ancestry"),
        "an unmerged branch commit exists but is not reachable"
    );
    assert!(git.contains(&side));
    let absent = commit("0123456789012345678901234567890123456789");
    assert!(
        git.is_descendant(&head, &absent).is_err(),
        "an absent commit is unknown, not merely unreachable"
    );

    // A handle that never crosses the threshold answers identically.
    let pairwise = Repository::open(repo.root()).expect("opens");
    assert!(pairwise.is_descendant(&head, &a).expect("ancestry"));
    assert!(!pairwise.is_descendant(&head, &side).expect("ancestry"));
    assert!(pairwise.is_descendant(&head, &head).expect("ancestry"));
}

// -------------------------------------------------------------------------------------
// The shared memo
// -------------------------------------------------------------------------------------

#[test]
fn a_shared_memo_is_dropped_when_the_working_tree_moves() {
    let mut repo = TempRepo::new("shared-worktree");
    repo.commit_file("a.txt", "one\n", "first");

    let clean = Repository::shared(repo.root()).expect("opens");
    assert!(clean.working_tree_changes().expect("status").is_empty());
    // Ask twice: the second answer comes from the memo and must agree with the first.
    assert!(clean.working_tree_changes().expect("status").is_empty());

    repo.write("b.txt", "untracked\n");
    let dirty = Repository::shared(repo.root()).expect("opens");
    assert!(
        dirty
            .working_tree_changes()
            .expect("status")
            .contains("b.txt"),
        "a new handle must see the edit, not the memo it was taken against"
    );
}

#[test]
fn a_shared_memo_is_dropped_when_head_moves() {
    let mut repo = TempRepo::new("shared-head");
    repo.commit_file("a.txt", "one\n", "first");
    let before = Repository::shared(repo.root())
        .expect("opens")
        .commits_touching("a.txt")
        .expect("history");
    assert_eq!(before.len(), 1);

    let second = commit(&repo.commit_file("a.txt", "two\n", "second"));
    let after = Repository::shared(repo.root())
        .expect("opens")
        .commits_touching("a.txt")
        .expect("history");
    assert_eq!(after.len(), 2, "the new commit must be visible");
    assert_eq!(after.first(), Some(&second));

    // Tree listings are commit-keyed, so both commits still answer for themselves.
    let git = Repository::shared(repo.root()).expect("opens");
    assert!(git.run_ls_tree(&second).expect("tree").contains("a.txt"));
}

#[test]
fn a_shared_memo_and_a_private_one_agree() {
    let mut repo = TempRepo::new("shared-agrees");
    let first = commit(&repo.commit_file("a.txt", "one\n", "first"));
    let head = commit(&repo.commit_file("a.txt", "two\n", "second"));
    repo.write("dirty.txt", "x\n");

    let private = Repository::open(repo.root()).expect("opens");
    let shared = Repository::shared(repo.root()).expect("opens");
    assert_eq!(
        private.working_tree_changes().expect("status"),
        shared.working_tree_changes().expect("status")
    );
    assert_eq!(
        private.commits_touching("a.txt").expect("history"),
        shared.commits_touching("a.txt").expect("history")
    );
    assert_eq!(
        private.is_descendant(&head, &first).expect("ancestry"),
        shared.is_descendant(&head, &first).expect("ancestry")
    );
    assert_eq!(
        private.file_at(&first, "a.txt").expect("blob"),
        shared.file_at(&first, "a.txt").expect("blob")
    );
    assert_eq!(
        private.touches_in(Some(&first), &head).expect("touches"),
        shared.touches_in(Some(&first), &head).expect("touches")
    );
}

//! Staleness, propagation, impact, and the review queue, over real git repositories.
//!
//! Exit criterion of `docs/13-implementation-roadmap.md` P5: *changing a watched bridge
//! file marks the corresponding engine-state observation for review and shows the
//! affected roadmap conclusions.* That scenario is
//! [`changing_a_bridge_file_marks_the_engine_state_observation_for_review`], built from
//! the real SYS tandem roadmap at
//! `examples/sys-tandem/legacy/2026-08-03-engine-simulator-tandem-roadmap.md`.

mod support;

use akr_core::freshness::{
    Impact, ReviewQueue, Stale, StaleCause, derive, glob_matches, impact_of_range,
    unmatched_watches, validate_glob,
};
use akr_core::git::{Repository, codes};
use akr_core::model::{
    Commit, ContentSlot, ContentValue, Date, Glob, Kind, Ledger, Project, RecordBuilder,
    RevisionId, State, key,
};
use std::collections::BTreeSet;
use support::TempRepo;

fn today(day: u8) -> Date {
    Date::new(2026, 8, day).expect("a valid date")
}

fn id(text: &str, revision: u32) -> RevisionId {
    RevisionId::new(key(text), revision)
}

fn commit(hash: &str) -> Commit {
    Commit::new(hash).expect("a full hash")
}

/// An observation with an `observed_at` and a set of `watches`.
fn observation(key_text: &str, observed_at: &str, watches: &[&str]) -> akr_core::model::Record {
    RecordBuilder::new(key_text, 1, Kind::Observation)
        .filled()
        .commit(ContentSlot::ObservedAt, observed_at)
        .content(
            ContentSlot::Watches,
            ContentValue::Globs(watches.iter().map(|g| Glob::new(g)).collect()),
        )
        .build()
}

// -------------------------------------------------------------------------------------
// Glob matching (D-008 subset)
// -------------------------------------------------------------------------------------

#[test]
fn globstar_matches_any_run_of_segments() {
    let glob = Glob::new("SYSEngine/crates/sys_game_bridge/**");
    assert!(glob_matches(
        &glob,
        "SYSEngine/crates/sys_game_bridge/src/lib.rs"
    ));
    assert!(glob_matches(
        &glob,
        "SYSEngine/crates/sys_game_bridge/tests/squelch_audit.rs"
    ));
    assert!(!glob_matches(
        &glob,
        "SYSEngine/crates/sys_render/src/lib.rs"
    ));
    assert!(!glob_matches(&glob, "src/engine/mod.rs"));
}

#[test]
fn a_star_does_not_cross_a_separator() {
    let glob = Glob::new("src/*.rs");
    assert!(glob_matches(&glob, "src/main.rs"));
    assert!(!glob_matches(&glob, "src/engine/mod.rs"));
}

#[test]
fn question_marks_and_classes_match_one_character() {
    assert!(glob_matches(&Glob::new("src/?.rs"), "src/a.rs"));
    assert!(!glob_matches(&Glob::new("src/?.rs"), "src/ab.rs"));
    assert!(glob_matches(&Glob::new("src/[a-c]x.rs"), "src/bx.rs"));
    assert!(!glob_matches(&Glob::new("src/[a-c]x.rs"), "src/zx.rs"));
}

#[test]
fn an_exact_path_matches_only_itself() {
    let glob = Glob::new("sim/src/step.rs");
    assert!(glob_matches(&glob, "sim/src/step.rs"));
    assert!(!glob_matches(&glob, "sim/src/step.rs.bak"));
    assert!(!glob_matches(&glob, "sim/src/other.rs"));
}

#[test]
fn the_glob_subset_is_enforced() {
    assert!(validate_glob(&Glob::new("a/**")).is_ok());
    assert!(
        validate_glob(&Glob::new("a/{b,c}")).is_err(),
        "brace expansion"
    );
    assert!(validate_glob(&Glob::new("!a")).is_err(), "negation");
    assert!(validate_glob(&Glob::new("a\\b")).is_err(), "backslash");
    assert!(
        validate_glob(&Glob::new("a/[bc")).is_err(),
        "unterminated class"
    );
    assert!(
        validate_glob(&Glob::new("a/x**")).is_err(),
        "partial globstar"
    );
    assert!(validate_glob(&Glob::new("/a")).is_err(), "absolute");
    assert!(validate_glob(&Glob::new("")).is_err(), "empty");
}

// -------------------------------------------------------------------------------------
// The staleness computation (docs/10 §3)
// -------------------------------------------------------------------------------------

#[test]
fn a_watched_path_that_moved_makes_a_record_stale() {
    let mut repo = TempRepo::new("watch-stale");
    let observed = repo.commit_file("src/project/mod.rs", "1\n", "base");
    let head = repo.commit_file("src/project/pass.rs", "2\n", "change the projection");
    let git = Repository::open(repo.root()).expect("opens");

    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.insert(observation(
        "fx.obs.projection",
        &observed,
        &["src/project/**"],
    ));

    let queue = derive(&ledger, &git, &commit(&head), today(3)).expect("derives");
    assert_eq!(queue.stale.len(), 1);
    assert_eq!(queue.stale[0].id, id("fx.obs.projection", 1));
    match &queue.stale[0].cause {
        StaleCause::Watch { glob, commit, path } => {
            assert_eq!(glob.as_str(), "src/project/**");
            assert_eq!(commit.as_str(), head);
            assert_eq!(path, "src/project/pass.rs");
        }
        other => panic!("expected a watch cause, got {other:?}"),
    }
}

#[test]
fn an_observation_made_at_head_is_not_stale() {
    // The change is already accounted for: the observation was made *at* that commit.
    let mut repo = TempRepo::new("observed-at-head");
    repo.commit_file("src/render/mod.rs", "1\n", "base");
    let head = repo.commit_file("src/render/frame.rs", "2\n", "change the renderer");
    let git = Repository::open(repo.root()).expect("opens");

    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.insert(observation("fx.obs.frames", &head, &["src/render/**"]));

    let queue = derive(&ledger, &git, &commit(&head), today(3)).expect("derives");
    assert!(queue.stale.is_empty(), "{:?}", queue.stale);
}

#[test]
fn a_change_outside_the_watched_paths_does_not_make_a_record_stale() {
    let mut repo = TempRepo::new("unwatched");
    let observed = repo.commit_file("src/project/mod.rs", "1\n", "base");
    let head = repo.commit_file("docs/notes.md", "notes\n", "unrelated");
    let git = Repository::open(repo.root()).expect("opens");

    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.insert(observation(
        "fx.obs.projection",
        &observed,
        &["src/project/**"],
    ));
    assert!(
        derive(&ledger, &git, &commit(&head), today(3))
            .expect("derives")
            .stale
            .is_empty()
    );
}

#[test]
fn a_passed_review_date_makes_a_record_stale_without_any_commit() {
    let mut repo = TempRepo::new("review-after");
    let head = repo.commit_file("a.txt", "1\n", "base");
    let git = Repository::open(repo.root()).expect("opens");

    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.insert(
        RecordBuilder::new("fx.obs.drift", 1, Kind::Observation)
            .filled()
            .commit(ContentSlot::ObservedAt, &head)
            .content(
                ContentSlot::ReviewAfter,
                ContentValue::Date(Date::new(2026, 7, 15).expect("date")),
            )
            .build(),
    );

    let queue = derive(&ledger, &git, &commit(&head), today(3)).expect("derives");
    assert_eq!(queue.stale.len(), 1);
    assert!(matches!(
        queue.stale[0].cause,
        StaleCause::ReviewAfter { .. }
    ));
}

#[test]
fn today_is_an_input_not_a_clock_reading() {
    // The same ledger at the same commit, on two stated days, gives two answers — and each
    // answer is reproducible.
    let mut repo = TempRepo::new("today");
    let head = repo.commit_file("a.txt", "1\n", "base");
    let git = Repository::open(repo.root()).expect("opens");

    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.insert(
        RecordBuilder::new("fx.obs.drift", 1, Kind::Observation)
            .filled()
            .commit(ContentSlot::ObservedAt, &head)
            .content(
                ContentSlot::ReviewAfter,
                ContentValue::Date(Date::new(2026, 8, 10).expect("date")),
            )
            .build(),
    );
    let before = derive(&ledger, &git, &commit(&head), today(9)).expect("derives");
    let after = derive(&ledger, &git, &commit(&head), today(11)).expect("derives");
    assert!(before.stale.is_empty(), "not yet due");
    assert_eq!(after.stale.len(), 1, "overdue");
    // Reproducible.
    let again = derive(&ledger, &git, &commit(&head), today(11)).expect("derives");
    assert_eq!(after.stale, again.stale);
}

#[test]
fn terminal_records_are_never_evaluated() {
    // A `disproven` observation has already been answered; asking whether it is current is
    // meaningless (`docs/10-freshness-and-git.md` §3).
    let mut repo = TempRepo::new("terminal");
    let observed = repo.commit_file("lege/src/a.rs", "1\n", "base");
    let head = repo.commit_file("lege/src/b.rs", "2\n", "change");
    let git = Repository::open(repo.root()).expect("opens");

    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    let mut record = observation("fx.obs.imports", &observed, &["lege/src/**"]);
    record.state = State::Disproven;
    ledger.insert(record);
    assert!(
        derive(&ledger, &git, &commit(&head), today(3))
            .expect("derives")
            .stale
            .is_empty()
    );
}

#[test]
fn normative_and_planning_records_are_never_stale_in_their_own_right() {
    let mut repo = TempRepo::new("non-empirical");
    let head = repo.commit_file("sim/src/a.rs", "1\n", "base");
    repo.commit_file("sim/src/b.rs", "2\n", "change");
    let head2 = repo.rev_parse("HEAD");
    let git = Repository::open(repo.root()).expect("opens");

    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.extend([
        RecordBuilder::new("fx.policy.tandem", 1, Kind::Policy)
            .filled()
            .state(State::Active)
            .path_scope("sim/**")
            .build(),
        RecordBuilder::new("fx.milestone.m1", 1, Kind::Milestone)
            .filled()
            .state(State::Active)
            .build(),
    ]);
    let _ = head;
    assert!(
        derive(&ledger, &git, &commit(&head2), today(3))
            .expect("derives")
            .stale
            .is_empty()
    );
}

// -------------------------------------------------------------------------------------
// Input validation (V-101, V-102, V-103)
// -------------------------------------------------------------------------------------

#[test]
fn an_observed_at_commit_the_repository_lacks_is_g011() {
    let mut repo = TempRepo::new("g011");
    let head = repo.commit_file("a.txt", "1\n", "base");
    let git = Repository::open(repo.root()).expect("opens");

    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.insert(observation(
        "fx.obs.stranded",
        "0123456789abcdef0123456789abcdef01234567",
        &["a.txt"],
    ));
    let queue = derive(&ledger, &git, &commit(&head), today(3)).expect("derives");
    assert!(queue.diagnostics.iter().any(|d| d.code == codes::G011));
}

#[test]
fn an_observed_at_on_a_divergent_branch_is_g012_and_not_a_failure() {
    let mut repo = TempRepo::new("g012");
    repo.commit_file("a.txt", "base\n", "base");
    repo.branch("side");
    let side = repo.commit_file("side.txt", "s\n", "side");
    repo.checkout("main");
    let head = repo.commit_file("main.txt", "m\n", "main");
    let git = Repository::open(repo.root()).expect("opens");

    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.insert(observation("fx.obs.side", &side, &["side.txt"]));
    let queue = derive(&ledger, &git, &commit(&head), today(3)).expect("derives");
    assert!(queue.diagnostics.iter().any(|d| d.code == codes::G012));
    assert_eq!(
        queue
            .diagnostics
            .iter()
            .find(|d| d.code == codes::G012)
            .map(|d| d.severity),
        Some(akr_core::diagnostics::Severity::Warning),
        "not computable is reported, not fatal"
    );
}

#[test]
fn a_reviewed_rewrite_keeps_the_observation_reachable_and_freshness_computable() {
    let mut repo = TempRepo::new("g012-rewrite-map");
    repo.commit_file("base.txt", "base\n", "base");
    repo.branch("pre-rewrite");
    let old = repo.commit_file("watched.txt", "old\n", "old observation identity");
    repo.checkout("main");
    let replacement = repo.commit_file(
        "watched.txt",
        "rewritten\n",
        "replacement observation identity",
    );
    let head = repo.commit_file("watched.txt", "changed\n", "change after observation");
    repo.write(
        ".akr-commit-map",
        &format!("{old} {replacement} # explicitly reviewed\n"),
    );
    let git = Repository::open(repo.root()).expect("opens");

    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.insert(observation("fx.obs.rewritten", &old, &["watched.txt"]));
    let queue = derive(&ledger, &git, &commit(&head), today(3)).expect("derives");

    assert!(
        !queue
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == codes::G011 || diagnostic.code == codes::G012),
        "a reachable reviewed replacement is neither absent nor divergent"
    );
    assert_eq!(queue.stale.len(), 1);
    assert_eq!(queue.stale[0].id, id("fx.obs.rewritten", 1));
}

#[test]
fn a_rewrite_target_that_is_still_divergent_remains_g012() {
    let mut repo = TempRepo::new("g012-rewrite-target-divergent");
    repo.commit_file("base.txt", "base\n", "base");
    repo.branch("pre-rewrite");
    let old = repo.commit_file("watched.txt", "old\n", "old identity");
    let replacement = repo.commit_file("watched.txt", "replacement\n", "replacement identity");
    repo.checkout("main");
    let head = repo.commit_file("main.txt", "main\n", "main");
    repo.write(".akr-commit-map", &format!("{old} {replacement}\n"));
    let git = Repository::open(repo.root()).expect("opens");

    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.insert(observation("fx.obs.side", &old, &["watched.txt"]));
    let queue = derive(&ledger, &git, &commit(&head), today(3)).expect("derives");

    assert!(queue.diagnostics.iter().any(|d| d.code == codes::G012));
}

#[test]
fn a_rewrite_target_the_repository_lacks_is_g011() {
    let mut repo = TempRepo::new("g011-rewrite-target-absent");
    repo.commit_file("base.txt", "base\n", "base");
    repo.branch("pre-rewrite");
    let old = repo.commit_file("watched.txt", "old\n", "old identity");
    repo.checkout("main");
    let head = repo.commit_file("main.txt", "main\n", "main");
    repo.write(
        ".akr-commit-map",
        &format!("{old} 0123456789abcdef0123456789abcdef01234567\n"),
    );
    let git = Repository::open(repo.root()).expect("opens");

    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.insert(observation("fx.obs.absent", &old, &["watched.txt"]));
    let queue = derive(&ledger, &git, &commit(&head), today(3)).expect("derives");

    assert!(queue.diagnostics.iter().any(|d| d.code == codes::G011));
}

#[test]
fn a_malformed_watch_glob_is_g021() {
    let mut repo = TempRepo::new("g021");
    let head = repo.commit_file("a.txt", "1\n", "base");
    let git = Repository::open(repo.root()).expect("opens");
    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.insert(observation("fx.obs.bad", &head, &["src/{a,b}/**"]));
    let queue = derive(&ledger, &git, &commit(&head), today(3)).expect("derives");
    assert!(queue.diagnostics.iter().any(|d| d.code == codes::G021));
}

#[test]
fn a_watch_glob_matching_nothing_is_g022() {
    // Silent rot: the record looks guarded and is not.
    let mut repo = TempRepo::new("g022");
    let head = repo.commit_file("a.txt", "1\n", "base");
    let git = Repository::open(repo.root()).expect("opens");
    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.insert(observation("fx.obs.moved", &head, &["engine/gone/**"]));
    let diagnostics = unmatched_watches(&ledger, &git, &commit(&head)).expect("lists");
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, codes::G022);
}

#[test]
fn a_scope_glob_matching_nothing_is_g023() {
    let mut repo = TempRepo::new("g023");
    let head = repo.commit_file("src/lib.rs", "1\n", "base");
    let git = Repository::open(repo.root()).expect("opens");
    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.insert(
        RecordBuilder::new("fx.policy.mis-scoped", 1, Kind::Policy)
            .filled()
            .state(State::Active)
            .path_scope("path src/**")
            .build(),
    );
    let diagnostics = unmatched_watches(&ledger, &git, &commit(&head)).expect("lists");
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, codes::G023);
    assert!(
        diagnostics[0]
            .help
            .as_deref()
            .is_some_and(|help| help.contains("copied `path ` prefix"))
    );
}

#[test]
fn an_intentionally_ignored_scope_is_not_g023() {
    let mut repo = TempRepo::new("g023-ignored");
    repo.write(".gitignore", ".cache/\n");
    let head = repo.commit_file("src/lib.rs", "1\n", "base");
    let git = Repository::open(repo.root()).expect("opens");
    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.insert(
        RecordBuilder::new("fx.decision.cache", 1, Kind::Decision)
            .filled()
            .state(State::Active)
            .path_scope(".cache/index.sqlite")
            .build(),
    );
    let diagnostics = unmatched_watches(&ledger, &git, &commit(&head)).expect("lists");
    assert!(!diagnostics.iter().any(|d| d.code == codes::G023));
}

#[test]
fn a_review_date_before_the_authoring_date_is_g031() {
    let mut repo = TempRepo::new("g031");
    let head = repo.commit_file("a.txt", "1\n", "base");
    let git = Repository::open(repo.root()).expect("opens");
    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    let mut record = RecordBuilder::new("fx.obs.typo", 1, Kind::Observation)
        .filled()
        .commit(ContentSlot::ObservedAt, &head)
        .content(
            ContentSlot::ReviewAfter,
            ContentValue::Date(Date::new(2026, 1, 1).expect("date")),
        )
        .build();
    record.created_at = Some(Date::new(2026, 6, 1).expect("date"));
    ledger.insert(record);
    let queue = derive(&ledger, &git, &commit(&head), today(3)).expect("derives");
    assert!(queue.diagnostics.iter().any(|d| d.code == codes::G031));
}

#[test]
fn a_dirty_working_tree_on_watched_paths_is_g004() {
    let mut repo = TempRepo::new("g004");
    let head = repo.commit_file("sim/src/step.rs", "1\n", "base");
    repo.write("sim/src/step.rs", "edited but not committed\n");
    let git = Repository::open(repo.root()).expect("opens");
    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.insert(observation("fx.obs.step", &head, &["sim/src/**"]));
    let queue = derive(&ledger, &git, &commit(&head), today(3)).expect("derives");
    assert!(queue.diagnostics.iter().any(|d| d.code == codes::G004));
    assert!(
        queue.stale.is_empty(),
        "freshness is computed from committed history only"
    );
}

// -------------------------------------------------------------------------------------
// The review queue: a build fact, never a diagnostic (D-024)
// -------------------------------------------------------------------------------------

#[test]
fn staleness_carries_no_diagnostic_code() {
    let mut repo = TempRepo::new("no-code");
    let observed = repo.commit_file("sim/src/a.rs", "1\n", "base");
    let head = repo.commit_file("sim/src/b.rs", "2\n", "change");
    let git = Repository::open(repo.root()).expect("opens");
    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.insert(observation("fx.obs.sim", &observed, &["sim/src/**"]));

    let queue = derive(&ledger, &git, &commit(&head), today(3)).expect("derives");
    assert_eq!(queue.stale.len(), 1);
    assert!(
        queue.diagnostics.is_empty(),
        "a stale record is a build fact, not a diagnostic: {:?}",
        queue.diagnostics.iter().map(|d| d.code).collect::<Vec<_>>()
    );
}

#[test]
fn review_clean_is_the_one_opt_in_diagnostic() {
    let mut repo = TempRepo::new("review-clean");
    let observed = repo.commit_file("sim/src/a.rs", "1\n", "base");
    let head = repo.commit_file("sim/src/b.rs", "2\n", "change");
    let git = Repository::open(repo.root()).expect("opens");
    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.insert(observation("fx.obs.sim", &observed, &["sim/src/**"]));

    let queue = derive(&ledger, &git, &commit(&head), today(3)).expect("derives");
    let diagnostic = queue.review_clean_diagnostic().expect("queue is not empty");
    assert_eq!(diagnostic.code, codes::G041);
    assert!(diagnostic.message.contains("1 stale"));

    let clean = ReviewQueue::default();
    assert!(clean.review_clean_diagnostic().is_none());
}

#[test]
fn the_queue_orders_watch_causes_before_review_dates() {
    let mut repo = TempRepo::new("queue-order");
    let observed = repo.commit_file("sim/src/a.rs", "1\n", "base");
    let head = repo.commit_file("sim/src/b.rs", "2\n", "change");
    let git = Repository::open(repo.root()).expect("opens");

    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.insert(observation(
        "fx.obs.zzz-watched",
        &observed,
        &["sim/src/**"],
    ));
    ledger.insert(
        RecordBuilder::new("fx.obs.aaa-dated", 1, Kind::Observation)
            .filled()
            .commit(ContentSlot::ObservedAt, &head)
            .content(
                ContentSlot::ReviewAfter,
                ContentValue::Date(Date::new(2026, 7, 15).expect("date")),
            )
            .build(),
    );

    let queue = derive(&ledger, &git, &commit(&head), today(3)).expect("derives");
    let order: Vec<String> = queue.stale.iter().map(|s| s.id.key.to_string()).collect();
    assert_eq!(
        order,
        ["fx.obs.zzz-watched", "fx.obs.aaa-dated"],
        "a watched path that moved is locatable; a passed date is a prompt"
    );
}

#[test]
fn a_watch_cause_is_reported_in_preference_to_a_passed_date() {
    // `docs/10-freshness-and-git.md` §3: when both conditions hold, the watch cause wins.
    // A moved path names the change to go and look at; a date only says the record is old.
    let mut repo = TempRepo::new("cause-precedence");
    let observed = repo.commit_file("sim/src/step.rs", "1\n", "base");
    let head = repo.commit_file("sim/src/step.rs", "2\n", "change the step");
    let git = Repository::open(repo.root()).expect("opens");

    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.insert(
        RecordBuilder::new("fx.obs.drift", 1, Kind::Observation)
            .filled()
            .commit(ContentSlot::ObservedAt, &observed)
            .content(
                ContentSlot::Watches,
                ContentValue::Globs(vec![Glob::new("sim/src/step.rs")]),
            )
            .content(
                ContentSlot::ReviewAfter,
                ContentValue::Date(Date::new(2026, 7, 15).expect("date")),
            )
            .build(),
    );

    let queue = derive(&ledger, &git, &commit(&head), today(3)).expect("derives");
    assert_eq!(queue.stale.len(), 1);
    assert_eq!(
        queue.stale[0].cause.name(),
        "watch",
        "both conditions hold; the actionable one is reported"
    );
}

#[test]
fn a_record_citing_a_stranded_commit_is_neither_stale_nor_fresh() {
    // Its freshness is not computable. AKR-G011 says so, and the rest of the queue is
    // still produced: one unanswerable record must not abort a build.
    let mut repo = TempRepo::new("stranded");
    let observed = repo.commit_file("sim/src/a.rs", "1\n", "base");
    let head = repo.commit_file("sim/src/b.rs", "2\n", "change");
    let git = Repository::open(repo.root()).expect("opens");

    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.insert(observation(
        "fx.obs.stranded",
        "0123456789abcdef0123456789abcdef01234567",
        &["sim/src/**"],
    ));
    ledger.insert(observation("fx.obs.sound", &observed, &["sim/src/**"]));

    let queue = derive(&ledger, &git, &commit(&head), today(3)).expect("derives");
    assert!(queue.diagnostics.iter().any(|d| d.code == codes::G011));
    let stale: Vec<String> = queue.stale.iter().map(|s| s.id.to_string()).collect();
    assert_eq!(
        stale,
        ["fx.obs.sound/1"],
        "the answerable record is still answered"
    );
}

// -------------------------------------------------------------------------------------
// Impact (docs/10 §6)
// -------------------------------------------------------------------------------------

#[test]
fn impact_reports_nothing_when_the_range_invalidates_nothing() {
    // The negative result the transcript pins: a report of nothing is informative rather
    // than a sign the command did not run.
    let mut repo = TempRepo::new("impact-none");
    let base = repo.commit_file("sim/src/a.rs", "1\n", "base");
    let head = repo.commit_file("lege/src/render/frame.rs", "1\n", "render only");
    let git = Repository::open(repo.root()).expect("opens");

    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    // Observed at HEAD, watching exactly what HEAD touched: already accounted for.
    ledger.insert(observation("fx.obs.frames", &head, &["lege/src/render/**"]));

    let impact = impact_of_range(
        &ledger,
        &git,
        &commit(&base),
        &commit(&head),
        &BTreeSet::new(),
    )
    .expect("impact");
    assert_eq!(impact.commits, 1);
    assert!(impact.touched.contains("lege/src/render/frame.rs"));
    assert!(impact.newly_stale.is_empty());
    assert!(impact.newly_at_risk.is_empty());
}

#[test]
fn impact_reports_what_a_range_would_invalidate_and_what_rests_on_it() {
    let mut repo = TempRepo::new("impact-some");
    let observed = repo.commit_file("sim/src/project/mod.rs", "1\n", "base");
    let head = repo.commit_file("sim/src/project/pass.rs", "2\n", "rewrite the projection");
    let git = Repository::open(repo.root()).expect("opens");

    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.insert(observation(
        "fx.obs.projection",
        &observed,
        &["sim/src/project/**"],
    ));
    ledger.insert(
        RecordBuilder::new("fx.assessment.risk", 1, Kind::Assessment)
            .filled()
            .rel(akr_core::model::Relation::SupportedBy, "@fx.obs.projection")
            .build(),
    );
    ledger.insert(
        RecordBuilder::new("fx.policy.tandem", 1, Kind::Policy)
            .filled()
            .state(State::Active)
            .rel(
                akr_core::model::Relation::SupportedBy,
                "@fx.assessment.risk",
            )
            .build(),
    );

    let impact = impact_of_range(
        &ledger,
        &git,
        &commit(&observed),
        &commit(&head),
        &BTreeSet::new(),
    )
    .expect("impact");
    assert_eq!(impact.newly_stale.len(), 1);
    assert_eq!(impact.newly_stale[0].id, id("fx.obs.projection", 1));
    let flagged: Vec<String> = impact
        .newly_at_risk
        .iter()
        .map(|r| format!("{} depth {}", r.id, r.depth))
        .collect();
    assert_eq!(
        flagged,
        ["fx.assessment.risk/1 depth 1", "fx.policy.tandem/1 depth 2"]
    );
}

#[test]
fn impact_excludes_what_the_caller_already_knows_is_stale() {
    // Impact is news, not a restatement.
    let mut repo = TempRepo::new("impact-known");
    let observed = repo.commit_file("sim/src/project/mod.rs", "1\n", "base");
    let head = repo.commit_file("sim/src/project/pass.rs", "2\n", "change");
    let git = Repository::open(repo.root()).expect("opens");

    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.insert(observation(
        "fx.obs.projection",
        &observed,
        &["sim/src/project/**"],
    ));
    let already: BTreeSet<RevisionId> = [id("fx.obs.projection", 1)].into();
    let impact = impact_of_range(&ledger, &git, &commit(&observed), &commit(&head), &already)
        .expect("impact");
    assert!(impact.newly_stale.is_empty());
}

#[test]
fn impact_on_an_unknown_revision_fails_with_g013() {
    let mut repo = TempRepo::new("impact-bad-rev");
    let head = repo.commit_file("a.txt", "1\n", "base");
    let git = Repository::open(repo.root()).expect("opens");
    let ledger = Ledger::new(Project::new("p", &["fx"]));
    let absent = commit("0123456789abcdef0123456789abcdef01234567");
    let error = impact_of_range(&ledger, &git, &absent, &commit(&head), &BTreeSet::new())
        .expect_err("unknown revision");
    assert_eq!(error.to_diagnostic().code, codes::G013);
}

// -------------------------------------------------------------------------------------
// The exit criterion: the real SYS scenario
// -------------------------------------------------------------------------------------

/// Exit criterion of `docs/13-implementation-roadmap.md` P5.
///
/// The scenario is taken from the document AKR exists to replace,
/// `examples/sys-tandem/legacy/2026-08-03-engine-simulator-tandem-roadmap.md`. That file
/// opens with a hand-maintained banner — `STATUS: LIVE ... Last reviewed: 2026-08-03` —
/// and §1 points at `SYSEngine/docs/BUILD-STATUS.md` as "verified tree state" and at the
/// mid-engine assessment as the gap register that rests on it. Nothing in that
/// arrangement notices when `SYSEngine/crates/sys_game_bridge/**` changes underneath it.
///
/// Here the same three statements are records:
///
/// - an **observation** of the bridge's projection coverage, `watches`ing the bridge;
/// - an **assessment** — the gap register — `supported_by` that observation;
/// - a **milestone** conclusion, M2, whose plan `depends_on` the assessment.
///
/// One commit to a bridge file then does what the banner never did.
#[test]
fn changing_a_bridge_file_marks_the_engine_state_observation_for_review() {
    let mut repo = TempRepo::new("sys-tandem");
    repo.write(
        "SYSEngine/crates/sys_game_bridge/src/lib.rs",
        "// the seam between simulator records and the engine\n",
    );
    repo.write(
        "SYSEngine/crates/sys_game_bridge/tests/squelch_audit.rs",
        "// two #[ignore]d assertions await M1\n",
    );
    repo.write(
        "src/engine/mod.rs",
        "// the simulator's own engine modules\n",
    );
    let audited = repo.commit("the tree as BUILD-STATUS.md verified it");

    let mut ledger = Ledger::new(Project::new("sys", &["eng", "sim", "sys"]));
    ledger.extend([
        // "Engine state: BUILD-STATUS.md (verified tree state)" — §1.
        RecordBuilder::new("eng.obs.bridge-projection-gaps", 1, Kind::Observation)
            .title("Of ~24 player-relevant simulator systems, ~6 are well represented")
            .filled()
            .commit(ContentSlot::ObservedAt, &audited)
            .content(
                ContentSlot::Watches,
                ContentValue::Globs(vec![Glob::new("SYSEngine/crates/sys_game_bridge/**")]),
            )
            .build(),
        // "the gap register" — §1, resting on the verified tree state.
        RecordBuilder::new("eng.assessment.mid-engine-gaps", 1, Kind::Assessment)
            .title("The engine shows a castle with people in it, not a court")
            .filled()
            .rel(
                akr_core::model::Relation::SupportedBy,
                "@eng.obs.bridge-projection-gaps",
            )
            .build(),
        // "M2 — The castle works visibly", whose plan rests on the gap register (§3).
        RecordBuilder::new("sys.work.m2-plan", 1, Kind::Work)
            .title("M2 plan of record: queue pockets, yield, carried objects")
            .filled()
            .state(State::Active)
            .rel(
                akr_core::model::Relation::DependsOn,
                "@eng.assessment.mid-engine-gaps",
            )
            .rel(
                akr_core::model::Relation::PlanOfRecord,
                "@sys.milestone.m2-castle-works-visibly",
            )
            .build(),
        RecordBuilder::new("sys.milestone.m2-castle-works-visibly", 1, Kind::Milestone)
            .title("M2 — The castle works visibly")
            .filled()
            .state(State::Active)
            .build(),
    ]);

    let git = Repository::open(repo.root()).expect("opens");

    // Before: the tree is as it was audited. Nothing is flagged.
    let before = derive(&ledger, &git, &commit(&audited), today(3)).expect("derives");
    assert!(before.is_empty(), "a freshly audited tree is clean");

    // A bridge file changes — the exact event the STATUS banner cannot notice.
    let changed = repo.commit_file(
        "SYSEngine/crates/sys_game_bridge/src/lib.rs",
        "// the seam between simulator records and the engine\n\
         // now projects flows, work packets, occasions and the ledger\n",
        "project four more record families through the bridge",
    );

    let after = derive(&ledger, &git, &commit(&changed), today(3)).expect("derives");

    // 1. The engine-state observation is marked for review, with the cause named.
    assert_eq!(after.stale.len(), 1, "{:?}", after.stale);
    let stale = &after.stale[0];
    assert_eq!(stale.id, id("eng.obs.bridge-projection-gaps", 1));
    match &stale.cause {
        StaleCause::Watch { glob, commit, path } => {
            assert_eq!(glob.as_str(), "SYSEngine/crates/sys_game_bridge/**");
            assert_eq!(commit.as_str(), changed);
            assert_eq!(path, "SYSEngine/crates/sys_game_bridge/src/lib.rs");
        }
        other => panic!("expected a watch cause, got {other:?}"),
    }

    // 2. The affected roadmap conclusions are shown, with the path back to the cause.
    let flagged: Vec<String> = after
        .at_risk
        .iter()
        .map(|entry| format!("{} depth {} via {}", entry.id, entry.depth, entry.via))
        .collect();
    assert_eq!(
        flagged,
        [
            "eng.assessment.mid-engine-gaps/1 depth 1 via supported_by",
            "sys.work.m2-plan/1 depth 2 via depends_on",
        ],
        "the gap register and the M2 plan that rests on it"
    );
    let plan = after
        .at_risk
        .iter()
        .find(|entry| entry.id == id("sys.work.m2-plan", 1))
        .expect("the plan is at risk");
    assert_eq!(
        plan.path
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        [
            "eng.assessment.mid-engine-gaps/1",
            "eng.obs.bridge-projection-gaps/1"
        ],
        "a reader can see why without being told"
    );

    // 3. The milestone itself is not flagged: staleness does not travel along
    //    `plan_of_record` or `part_of` (D-024). Only what declares a dependency.
    assert!(
        !after
            .at_risk
            .iter()
            .any(|entry| entry.id.key.to_string() == "sys.milestone.m2-castle-works-visibly"),
        "a warning that always fires is not a warning"
    );

    // 4. And none of it is a diagnostic: the ledger is not broken, it is out of date.
    assert!(after.diagnostics.is_empty());

    // 5. Re-observing at the new commit clears the observation and everything under it —
    //    which is what the "Last reviewed" line was trying and failing to express.
    let mut reobserved = ledger.clone();
    let records: Vec<_> = reobserved
        .records()
        .iter()
        .map(|record| {
            let mut copy = record.clone();
            if copy.id == id("eng.obs.bridge-projection-gaps", 1) {
                copy.content.insert(
                    ContentSlot::ObservedAt,
                    ContentValue::Commit(commit(&changed)),
                );
            }
            copy
        })
        .collect();
    reobserved = Ledger::new(reobserved.project.clone());
    reobserved.extend(records);
    let cleared = derive(&reobserved, &git, &commit(&changed), today(3)).expect("derives");
    assert!(
        cleared.is_empty(),
        "re-observing the source clears the set: {:?} / {:?}",
        cleared.stale,
        cleared.at_risk
    );
}

#[test]
fn the_bridge_scenario_is_reported_the_same_way_by_impact() {
    // `akr impact --git-diff` answers the same question before the change lands, which is
    // the one moment the author can still re-observe cheaply.
    let mut repo = TempRepo::new("sys-tandem-impact");
    repo.write(
        "SYSEngine/crates/sys_game_bridge/src/lib.rs",
        "// the seam\n",
    );
    let audited = repo.commit("audited");
    let changed = repo.commit_file(
        "SYSEngine/crates/sys_game_bridge/src/lib.rs",
        "// the seam, extended\n",
        "extend the bridge",
    );

    let mut ledger = Ledger::new(Project::new("sys", &["eng"]));
    ledger.extend([
        observation(
            "eng.obs.bridge-projection-gaps",
            &audited,
            &["SYSEngine/crates/sys_game_bridge/**"],
        ),
        RecordBuilder::new("eng.assessment.mid-engine-gaps", 1, Kind::Assessment)
            .filled()
            .rel(
                akr_core::model::Relation::SupportedBy,
                "@eng.obs.bridge-projection-gaps",
            )
            .build(),
    ]);

    let git = Repository::open(repo.root()).expect("opens");
    let impact: Impact = impact_of_range(
        &ledger,
        &git,
        &commit(&audited),
        &commit(&changed),
        &BTreeSet::new(),
    )
    .expect("impact");

    assert_eq!(impact.commits, 1);
    assert_eq!(
        impact.touched.iter().collect::<Vec<_>>(),
        ["SYSEngine/crates/sys_game_bridge/src/lib.rs"]
    );
    let stale: Vec<&Stale> = impact.newly_stale.iter().collect();
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].id, id("eng.obs.bridge-projection-gaps", 1));
    assert_eq!(impact.newly_at_risk.len(), 1);
    assert_eq!(
        impact.newly_at_risk[0].id,
        id("eng.assessment.mid-engine-gaps", 1)
    );
}

// -------------------------------------------------------------------------------------
// Determinism
// -------------------------------------------------------------------------------------

#[test]
fn derivation_is_independent_of_record_insertion_order() {
    let mut repo = TempRepo::new("determinism");
    let observed = repo.commit_file("sim/src/project/mod.rs", "1\n", "base");
    let head = repo.commit_file("sim/src/project/pass.rs", "2\n", "change");
    let git = Repository::open(repo.root()).expect("opens");

    let build = |reverse: bool| {
        let mut records = vec![
            observation("fx.obs.a", &observed, &["sim/src/project/**"]),
            observation("fx.obs.b", &observed, &["sim/src/project/**"]),
            observation("fx.obs.c", &observed, &["sim/src/project/**"]),
        ];
        if reverse {
            records.reverse();
        }
        let mut ledger = Ledger::new(Project::new("p", &["fx"]));
        ledger.extend(records);
        derive(&ledger, &git, &commit(&head), today(3)).expect("derives")
    };
    assert_eq!(build(false).stale, build(true).stale);
    assert_eq!(build(false).stale.len(), 3);
}

#[test]
fn every_freshness_code_is_registered_in_the_runtime_registry() {
    // The `G` range lives in codes-runtime.md, not codes-lang.md
    // (`spec/diagnostics/README.md` §2). An unregistered code is one nobody can look up.
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../spec/diagnostics/codes-runtime.md"
    );
    let text = std::fs::read_to_string(path).expect("codes-runtime.md is readable");
    for code in codes::ALL {
        assert!(
            text.contains(&format!("`{code}`")),
            "{code} is raised in code but absent from spec/diagnostics/codes-runtime.md"
        );
        assert_eq!(
            code.stage(),
            Some(akr_core::diagnostics::Stage::Git),
            "{code} is not a freshness code"
        );
    }
}

#[test]
fn the_v_rules_this_module_cites_are_the_ones_docs_10_catalogues() {
    // V-101..V-104 (`docs/10-freshness-and-git.md` §9). The rule identifiers are internal,
    // so this asserts the mapping through the diagnostics they raise.
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/10-freshness-and-git.md"
    );
    let text = std::fs::read_to_string(path).expect("docs/10 is readable");
    for (rule, code) in [
        ("V-101", "AKR-G011"),
        ("V-101", "AKR-G012"),
        ("V-102", "AKR-G021"),
        ("V-102", "AKR-G022"),
        ("V-103", "AKR-G031"),
        ("V-104", "AKR-G041"),
    ] {
        assert!(text.contains(rule), "{rule} is catalogued");
        assert!(text.contains(code), "{code} is cited");
    }
}

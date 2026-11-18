//! Staleness, propagation, impact, and the review queue, over real git repositories.
//!
//! Exit criterion of `docs/13-implementation-roadmap.md` P5: *changing a watched bridge
//! file marks the corresponding engine-state observation for review and shows the
//! affected roadmap conclusions.* That scenario is
//! [`changing_a_bridge_file_marks_the_engine_state_observation_for_review`], built from
//! the real SYS tandem roadmap at
//! `examples/sys-tandem/legacy/2026-08-03-engine-simulator-tandem-roadmap.md`.

use crate::support;

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
use std::sync::OnceLock;
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
fn glob_matching_and_subset_validation() {
    let globstar = Glob::new("SYSEngine/crates/sys_game_bridge/**");
    assert!(glob_matches(
        &globstar,
        "SYSEngine/crates/sys_game_bridge/src/lib.rs"
    ));
    assert!(glob_matches(
        &globstar,
        "SYSEngine/crates/sys_game_bridge/tests/squelch_audit.rs"
    ));
    assert!(!glob_matches(
        &globstar,
        "SYSEngine/crates/sys_render/src/lib.rs"
    ));
    assert!(!glob_matches(&globstar, "src/engine/mod.rs"));

    let star = Glob::new("src/*.rs");
    assert!(glob_matches(&star, "src/main.rs"));
    assert!(!glob_matches(&star, "src/engine/mod.rs"));

    assert!(glob_matches(&Glob::new("src/?.rs"), "src/a.rs"));
    assert!(!glob_matches(&Glob::new("src/?.rs"), "src/ab.rs"));
    assert!(glob_matches(&Glob::new("src/[a-c]x.rs"), "src/bx.rs"));
    assert!(!glob_matches(&Glob::new("src/[a-c]x.rs"), "src/zx.rs"));

    let exact = Glob::new("sim/src/step.rs");
    assert!(glob_matches(&exact, "sim/src/step.rs"));
    assert!(!glob_matches(&exact, "sim/src/step.rs.bak"));
    assert!(!glob_matches(&exact, "sim/src/other.rs"));

    assert!(validate_glob(&Glob::new("a/**")).is_ok());
    for bad in ["a/{b,c}", "!a", "a\\b", "a/[bc", "a/x**", "/a", ""] {
        assert!(validate_glob(&Glob::new(bad)).is_err(), "{bad:?}");
    }
}

// -------------------------------------------------------------------------------------
// The staleness computation, input validation, and review queue (shared linear repo)
// -------------------------------------------------------------------------------------

#[test]
fn linear_history_staleness_diagnostics_and_queue_ordering() {
    let mut repo = TempRepo::new("freshness-linear");
    repo.write("src/project/mod.rs", "1\n");
    repo.write("src/render/mod.rs", "1\n");
    repo.write("sim/src/step.rs", "1\n");
    let c_base = repo.commit("base");

    repo.write("src/project/pass.rs", "2\n");
    repo.write("src/render/frame.rs", "2\n");
    repo.write("sim/src/step.rs", "2\n");
    let c_mid = repo.commit("mid");

    let head = repo.commit_file("docs/notes.md", "notes\n", "unrelated");
    // Uncommitted files for G004 and G027:
    repo.write("sim/src/step.rs", "edited but not committed\n");
    repo.write("engine/new/mod.rs", "// not committed yet\n");
    let git = Repository::open(repo.root()).expect("opens");

    // 1. Watched path moved -> stale Watch cause; observed at head -> fresh; unwatched -> fresh.
    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.extend([
        observation("fx.obs.projection", &c_base, &["src/project/**"]),
        observation("fx.obs.frames", &head, &["src/render/**"]),
        observation("fx.obs.unwatched", &c_mid, &["src/render/**"]),
    ]);
    let queue = derive(&ledger, &git, &commit(&head), today(3)).expect("derives");
    assert_eq!(queue.stale.len(), 1);
    assert_eq!(queue.stale[0].id, id("fx.obs.projection", 1));
    match &queue.stale[0].cause {
        StaleCause::Watch { glob, commit, path } => {
            assert_eq!(glob.as_str(), "src/project/**");
            assert_eq!(commit.as_str(), c_mid);
            assert_eq!(path, "src/project/pass.rs");
        }
        other => panic!("expected a watch cause, got {other:?}"),
    }
    assert!(
        queue.diagnostics.is_empty(),
        "a stale record is a build fact, not a diagnostic: {:?}",
        queue.diagnostics.iter().map(|d| d.code).collect::<Vec<_>>()
    );
    let g041 = queue.review_clean_diagnostic().expect("queue is not empty");
    assert_eq!(g041.code, codes::G041);
    assert!(g041.message.contains("1 stale"));
    assert!(ReviewQueue::default().review_clean_diagnostic().is_none());

    // 2. Passed review_after makes record stale; today is an input, not a clock reading.
    let mut dated = Ledger::new(Project::new("p", &["fx"]));
    dated.insert(
        RecordBuilder::new("fx.obs.drift", 1, Kind::Observation)
            .filled()
            .commit(ContentSlot::ObservedAt, &head)
            .content(
                ContentSlot::ReviewAfter,
                ContentValue::Date(Date::new(2026, 8, 10).expect("date")),
            )
            .build(),
    );
    assert!(
        derive(&dated, &git, &commit(&head), today(9))
            .expect("derives")
            .stale
            .is_empty()
    );
    let after = derive(&dated, &git, &commit(&head), today(11)).expect("derives");
    assert_eq!(after.stale.len(), 1);
    assert!(matches!(
        after.stale[0].cause,
        StaleCause::ReviewAfter { .. }
    ));

    // 3. Terminal and normative/planning records are never evaluated as stale.
    let mut non_eval = Ledger::new(Project::new("p", &["fx"]));
    let mut disproven = observation("fx.obs.disproven", &c_base, &["src/project/**"]);
    disproven.state = State::Disproven;
    non_eval.extend([
        disproven,
        RecordBuilder::new("fx.policy.tandem", 1, Kind::Policy)
            .filled()
            .state(State::Active)
            .path_scope("src/**")
            .build(),
        RecordBuilder::new("fx.milestone.m1", 1, Kind::Milestone)
            .filled()
            .state(State::Active)
            .build(),
    ]);
    assert!(
        derive(&non_eval, &git, &commit(&head), today(3))
            .expect("derives")
            .stale
            .is_empty()
    );

    // 4. Queue ordering (watch before date) and cause precedence (watch beats date).
    let mut ordered = Ledger::new(Project::new("p", &["fx"]));
    ordered.insert(observation(
        "fx.obs.zzz-watched",
        &c_base,
        &["src/project/**"],
    ));
    ordered.insert(
        RecordBuilder::new("fx.obs.aaa-dated", 1, Kind::Observation)
            .filled()
            .commit(ContentSlot::ObservedAt, &head)
            .content(
                ContentSlot::ReviewAfter,
                ContentValue::Date(Date::new(2026, 7, 15).expect("date")),
            )
            .build(),
    );
    let q_ordered = derive(&ordered, &git, &commit(&head), today(3)).expect("derives");
    let names: Vec<String> = q_ordered
        .stale
        .iter()
        .map(|s| s.id.key.to_string())
        .collect();
    assert_eq!(names, ["fx.obs.zzz-watched", "fx.obs.aaa-dated"]);

    let mut both = Ledger::new(Project::new("p", &["fx"]));
    both.insert(
        RecordBuilder::new("fx.obs.both", 1, Kind::Observation)
            .filled()
            .commit(ContentSlot::ObservedAt, &c_base)
            .content(
                ContentSlot::Watches,
                ContentValue::Globs(vec![Glob::new("src/project/pass.rs")]),
            )
            .content(
                ContentSlot::ReviewAfter,
                ContentValue::Date(Date::new(2026, 7, 15).expect("date")),
            )
            .build(),
    );
    let q_both = derive(&both, &git, &commit(&head), today(3)).expect("derives");
    assert_eq!(q_both.stale.len(), 1);
    assert_eq!(q_both.stale[0].cause.name(), "watch");

    // 5. Diagnostics: G011 (plus partial queue survival), G021, G022, G027, G031, G004.
    let mut stranded_and_sound = Ledger::new(Project::new("p", &["fx"]));
    stranded_and_sound.insert(observation(
        "fx.obs.stranded",
        "0123456789abcdef0123456789abcdef01234567",
        &["src/project/**"],
    ));
    stranded_and_sound.insert(observation("fx.obs.sound", &c_base, &["src/project/**"]));
    let q_stranded = derive(&stranded_and_sound, &git, &commit(&head), today(3)).expect("derives");
    assert!(q_stranded.diagnostics.iter().any(|d| d.code == codes::G011));
    assert_eq!(
        q_stranded
            .stale
            .iter()
            .map(|s| s.id.to_string())
            .collect::<Vec<_>>(),
        ["fx.obs.sound/1"]
    );

    let mut bad_glob = Ledger::new(Project::new("p", &["fx"]));
    bad_glob.insert(observation("fx.obs.bad", &head, &["src/{a,b}/**"]));
    assert!(
        derive(&bad_glob, &git, &commit(&head), today(3))
            .expect("derives")
            .diagnostics
            .iter()
            .any(|d| d.code == codes::G021)
    );

    let mut missing_watch = Ledger::new(Project::new("p", &["fx"]));
    missing_watch.insert(observation("fx.obs.moved", &head, &["engine/gone/**"]));
    let g022 = unmatched_watches(&missing_watch, &git, &commit(&head)).expect("lists");
    assert_eq!(g022.len(), 1);
    assert_eq!(g022[0].code, codes::G022);

    let mut new_watch = Ledger::new(Project::new("p", &["fx"]));
    new_watch.insert(observation("fx.obs.new", &head, &["engine/new/**"]));
    let g027 = unmatched_watches(&new_watch, &git, &commit(&head)).expect("lists");
    assert_eq!(g027.len(), 1);
    assert_eq!(g027[0].code, codes::G027);

    let mut bad_date = Ledger::new(Project::new("p", &["fx"]));
    let mut typo = RecordBuilder::new("fx.obs.typo", 1, Kind::Observation)
        .filled()
        .commit(ContentSlot::ObservedAt, &head)
        .content(
            ContentSlot::ReviewAfter,
            ContentValue::Date(Date::new(2026, 1, 1).expect("date")),
        )
        .build();
    typo.created_at = Some(Date::new(2026, 6, 1).expect("date"));
    bad_date.insert(typo);
    assert!(
        derive(&bad_date, &git, &commit(&head), today(3))
            .expect("derives")
            .diagnostics
            .iter()
            .any(|d| d.code == codes::G031)
    );

    let mut dirty = Ledger::new(Project::new("p", &["fx"]));
    dirty.insert(observation("fx.obs.step", &head, &["sim/src/**"]));
    let q_dirty = derive(&dirty, &git, &commit(&head), today(3)).expect("derives");
    assert!(q_dirty.diagnostics.iter().any(|d| d.code == codes::G004));
    assert!(q_dirty.stale.is_empty());

    // 6. Derivation is independent of record insertion order.
    let build_ordered = |reverse: bool| {
        let mut records = vec![
            observation("fx.obs.a", &c_base, &["src/project/**"]),
            observation("fx.obs.b", &c_base, &["src/project/**"]),
            observation("fx.obs.c", &c_base, &["src/project/**"]),
        ];
        if reverse {
            records.reverse();
        }
        let mut l = Ledger::new(Project::new("p", &["fx"]));
        l.extend(records);
        derive(&l, &git, &commit(&head), today(3)).expect("derives")
    };
    assert_eq!(build_ordered(false).stale, build_ordered(true).stale);
    assert_eq!(build_ordered(false).stale.len(), 3);
}

// -------------------------------------------------------------------------------------
// Divergent branches and rewrite maps (V-101 / G011 / G012)
// -------------------------------------------------------------------------------------

#[test]
fn divergent_branches_and_rewrite_maps_report_g011_and_g012() {
    let mut repo = TempRepo::new("freshness-branches");
    repo.commit_file("base.txt", "base\n", "base");
    repo.branch("side");
    let old = repo.commit_file("watched.txt", "old\n", "old identity");
    let replacement_on_side = repo.commit_file("watched.txt", "side2\n", "side replacement");
    repo.checkout("main");
    let replacement = repo.commit_file("watched.txt", "rewritten\n", "replacement identity");
    let head = repo.commit_file("watched.txt", "changed\n", "change after observation");

    // 1. Divergent branch with no rewrite map -> G012 Warning.
    let git_plain = Repository::open(repo.root()).expect("opens");
    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.insert(observation("fx.obs.side", &old, &["watched.txt"]));
    let queue = derive(&ledger, &git_plain, &commit(&head), today(3)).expect("derives");
    assert_eq!(
        queue
            .diagnostics
            .iter()
            .find(|d| d.code == codes::G012)
            .map(|d| d.severity),
        Some(akr_core::diagnostics::Severity::Warning),
        "not computable is reported, not fatal"
    );

    // 2. Reviewed rewrite onto main -> reachable and stale.
    repo.write(
        ".akr-commit-map",
        &format!("{old} {replacement} # explicitly reviewed\n"),
    );
    let git_mapped = Repository::open(repo.root()).expect("opens");
    let q_mapped = derive(&ledger, &git_mapped, &commit(&head), today(3)).expect("derives");
    assert!(
        !q_mapped
            .diagnostics
            .iter()
            .any(|d| d.code == codes::G011 || d.code == codes::G012)
    );
    assert_eq!(q_mapped.stale.len(), 1);
    assert_eq!(q_mapped.stale[0].id, id("fx.obs.side", 1));

    // 3. Rewrite target still on divergent branch -> remains G012.
    repo.write(".akr-commit-map", &format!("{old} {replacement_on_side}\n"));
    let git_still_side = Repository::open(repo.root()).expect("opens");
    let q_still_side = derive(&ledger, &git_still_side, &commit(&head), today(3)).expect("derives");
    assert!(
        q_still_side
            .diagnostics
            .iter()
            .any(|d| d.code == codes::G012)
    );

    // 4. Rewrite target absent from repo -> G011.
    repo.write(
        ".akr-commit-map",
        &format!("{old} 0123456789abcdef0123456789abcdef01234567\n"),
    );
    let git_absent = Repository::open(repo.root()).expect("opens");
    let q_absent = derive(&ledger, &git_absent, &commit(&head), today(3)).expect("derives");
    assert!(q_absent.diagnostics.iter().any(|d| d.code == codes::G011));
}

// -------------------------------------------------------------------------------------
// Scope globs (shared repo, preserving AKR-G024 test token names)
// -------------------------------------------------------------------------------------

fn shared_scope_repo() -> &'static (TempRepo, Commit, Repository) {
    static REPO: OnceLock<(TempRepo, Commit, Repository)> = OnceLock::new();
    REPO.get_or_init(|| {
        let mut repo = TempRepo::new("freshness-scopes");
        repo.write(".gitignore", ".cache/\n");
        repo.write("src/lib.rs", "1\n");
        repo.write("tools/build.py", "1\n");
        let head = commit(&repo.commit("base"));
        repo.write("engine/new/mod.rs", "// not committed yet\n");
        let git = Repository::open(repo.root()).expect("opens");
        (repo, head, git)
    })
}

/// A bare directory means the directory, with or without a trailing slash.
#[test]
fn a_directory_scope_matches_what_is_under_it() {
    let (_, head, git) = shared_scope_repo();
    for written in ["tools/", "tools", "tools/**", "tools/*", "tools/build.py"] {
        let mut ledger = Ledger::new(Project::new("p", &["fx"]));
        ledger.insert(
            RecordBuilder::new("fx.work.tooling", 1, Kind::Work)
                .filled()
                .state(State::Active)
                .path_scope(written)
                .build(),
        );
        let diagnostics = unmatched_watches(&ledger, git, head).expect("lists");
        assert!(
            !diagnostics.iter().any(|d| d.code == codes::G023),
            "{written:?} should match tools/build.py: {diagnostics:?}"
        );
    }
    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.insert(
        RecordBuilder::new("fx.work.gone", 1, Kind::Work)
            .filled()
            .state(State::Active)
            .path_scope("toolsmiths/")
            .build(),
    );
    let diagnostics = unmatched_watches(&ledger, git, head).expect("lists");
    assert!(diagnostics.iter().any(|d| d.code == codes::G023));
}

#[test]
fn a_scope_glob_over_uncommitted_and_missing_files_is_g023() {
    let (_, head, git) = shared_scope_repo();

    let mut uncommitted = Ledger::new(Project::new("p", &["fx"]));
    uncommitted.insert(
        RecordBuilder::new("fx.policy.new-scoped", 1, Kind::Policy)
            .filled()
            .state(State::Active)
            .path_scope("engine/new/**")
            .build(),
    );
    let diagnostics = unmatched_watches(&uncommitted, git, head).expect("lists");
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, codes::G023);
    assert!(
        diagnostics[0]
            .help
            .as_deref()
            .unwrap_or_default()
            .contains("not committed yet"),
        "{:?}",
        diagnostics[0].help
    );

    let mut missing = Ledger::new(Project::new("p", &["fx"]));
    missing.insert(
        RecordBuilder::new("fx.policy.mis-scoped", 1, Kind::Policy)
            .filled()
            .state(State::Active)
            .path_scope("engine/gone/**")
            .build(),
    );
    let diagnostics = unmatched_watches(&missing, git, head).expect("lists");
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, codes::G023);
}

#[test]
fn a_malformed_scope_glob_is_g021_and_not_a_dead_scope() {
    let (_, head, git) = shared_scope_repo();
    for written in ["path src/**", "path \"src/**\""] {
        let mut ledger = Ledger::new(Project::new("p", &["fx"]));
        ledger.insert(
            RecordBuilder::new("fx.policy.mis-scoped", 1, Kind::Policy)
                .filled()
                .state(State::Active)
                .path_scope(written)
                .build(),
        );
        let diagnostics = unmatched_watches(&ledger, git, head).expect("lists");
        assert_eq!(diagnostics.len(), 1, "for {written:?}");
        assert_eq!(diagnostics[0].code, codes::G021, "for {written:?}");
        assert!(!diagnostics.iter().any(|d| d.code == codes::G023));
    }
}

#[test]
fn a_completed_records_dead_scope_is_still_reported() {
    let (_, head, git) = shared_scope_repo();
    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.insert(
        RecordBuilder::new("fx.work.finished", 1, Kind::Work)
            .filled()
            .state(State::Completed)
            .path_scope("engine/gone/**")
            .build(),
    );
    let diagnostics = unmatched_watches(&ledger, git, head).expect("lists");
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, codes::G023);
}

#[test]
fn a_superseded_records_dead_scope_is_not_reported() {
    let (_, head, git) = shared_scope_repo();
    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.insert(
        RecordBuilder::new("fx.work.retired", 1, Kind::Work)
            .filled()
            .state(State::Superseded)
            .path_scope("engine/gone/**")
            .build(),
    );
    let diagnostics = unmatched_watches(&ledger, git, head).expect("lists");
    assert!(!diagnostics.iter().any(|d| d.code == codes::G023));
}

#[test]
fn an_ignored_scope_is_a_build_fact_rather_than_a_diagnostic() {
    use akr_core::freshness::{UnverifiableReason, unverifiable_scopes};
    let (_, head, git) = shared_scope_repo();
    let mut ledger = Ledger::new(Project::new("p", &["fx"]));
    ledger.insert(
        RecordBuilder::new("fx.decision.cache", 1, Kind::Decision)
            .filled()
            .state(State::Active)
            .path_scope(".cache/index.sqlite")
            .build(),
    );
    let diagnostics = unmatched_watches(&ledger, git, head).expect("lists");
    assert!(!diagnostics.iter().any(|d| d.code == codes::G023));
    let facts = unverifiable_scopes(&ledger, git, head).expect("lists");
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].glob, ".cache/index.sqlite");
    assert_eq!(facts[0].reason, UnverifiableReason::Ignored);
}

// -------------------------------------------------------------------------------------
// Impact (docs/10 §6)
// -------------------------------------------------------------------------------------

#[test]
fn impact_of_range_reports_stale_at_risk_exclusions_and_bad_revision() {
    let mut repo = TempRepo::new("freshness-impact");
    let observed = repo.commit_file("sim/src/project/mod.rs", "1\n", "base");
    let head = repo.commit_file("sim/src/project/pass.rs", "2\n", "rewrite the projection");
    let render_only = repo.commit_file("lege/src/render/frame.rs", "1\n", "render only");
    let git = Repository::open(repo.root()).expect("opens");

    // 1. Observed at HEAD -> nothing invalidated.
    let mut clean = Ledger::new(Project::new("p", &["fx"]));
    clean.insert(observation(
        "fx.obs.frames",
        &render_only,
        &["lege/src/render/**"],
    ));
    let none = impact_of_range(
        &clean,
        &git,
        &commit(&head),
        &commit(&render_only),
        &BTreeSet::new(),
    )
    .expect("impact");
    assert_eq!(none.commits, 1);
    assert!(none.touched.contains("lege/src/render/frame.rs"));
    assert!(none.newly_stale.is_empty());
    assert!(none.newly_at_risk.is_empty());

    // 2. Range invalidates observation and propagates depth 1 and 2.
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
    let some = impact_of_range(
        &ledger,
        &git,
        &commit(&observed),
        &commit(&head),
        &BTreeSet::new(),
    )
    .expect("impact");
    assert_eq!(some.newly_stale.len(), 1);
    assert_eq!(some.newly_stale[0].id, id("fx.obs.projection", 1));
    let flagged: Vec<String> = some
        .newly_at_risk
        .iter()
        .map(|r| format!("{} depth {}", r.id, r.depth))
        .collect();
    assert_eq!(
        flagged,
        ["fx.assessment.risk/1 depth 1", "fx.policy.tandem/1 depth 2"]
    );

    // 3. Excludes what caller already knows is stale.
    let already: BTreeSet<RevisionId> = [id("fx.obs.projection", 1)].into();
    let known = impact_of_range(&ledger, &git, &commit(&observed), &commit(&head), &already)
        .expect("impact");
    assert!(known.newly_stale.is_empty());

    // 4. Unknown revision fails with G013.
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

    // 6. `impact` reports the same bridge scenario before the change lands.
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
    assert_eq!(impact.newly_at_risk.len(), 2);
    assert_eq!(
        impact.newly_at_risk[0].id,
        id("eng.assessment.mid-engine-gaps", 1)
    );
    assert_eq!(impact.newly_at_risk[1].id, id("sys.work.m2-plan", 1));
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

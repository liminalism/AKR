//! Every command of `docs/07-cli.md` §6, exercised through the real binary.
//!
//! The transcripts pin what four commands *print*; this pins that the rest run at all,
//! exit as §3 says, and produce a JSON envelope with the fields §5 fixes. Between them the
//! read surface has no command that has never been executed.

use crate::support;

use support::{Example, SYS_TANDEM};

/// The envelope fields §5 declares, in the order it declares them.
const ENVELOPE: &[&str] = &[
    "\"akr\":",
    "\"tool_version\":",
    "\"command\":",
    "\"commit\":",
    "\"source_graph_hash\":",
    "\"ok\":",
    "\"exit_code\":",
    "\"diagnostics\":",
    "\"result\":",
];

fn assert_envelope(text: &str, command: &str) {
    let mut cursor = 0;
    for field in ENVELOPE {
        let found = text[cursor..]
            .find(field)
            .unwrap_or_else(|| panic!("{command}: envelope has no {field}\n{text}"));
        cursor += found + field.len();
    }
    assert!(
        text.contains(&format!("\"command\": {command:?}")),
        "{command}: envelope names the wrong command\n{text}"
    );
    assert!(
        text.ends_with("}\n"),
        "{command}: envelope is not a document"
    );
}

#[test]
fn help_and_version_need_no_workspace() {
    let dir = std::env::temp_dir();
    for args in [vec!["--help"], vec!["--version"], vec!["propose", "--help"]] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_akr"))
            .args(&args)
            .current_dir(&dir)
            .output()
            .expect("runs");
        assert_eq!(output.status.code(), Some(0), "{args:?}");
        assert!(!output.stdout.is_empty(), "{args:?}");
    }
}

#[test]
fn context_on_an_empty_ledger_explains_how_to_create_the_first_goal() {
    let example = Example::materialise("commands-empty-ledger");
    for directory in [".akr/records", ".akr/archive"] {
        let path = example.root().join(directory);
        if path.exists() {
            std::fs::remove_dir_all(&path).expect("fixture knowledge removed");
        }
    }
    std::fs::create_dir_all(example.root().join(".akr/records"))
        .expect("empty records directory restored");

    let run = example.run(&[
        "--format",
        "json",
        "context",
        "--goal",
        "sys.milestone.first",
    ]);
    assert_eq!(run.code, 3, "{}", run.output());
    assert!(
        run.stdout.contains("knowledge ledger has no records yet"),
        "{}",
        run.output()
    );
    assert!(
        run.stdout
            .contains("create the first planning record with `knowledge.propose`"),
        "{}",
        run.output()
    );
}

#[test]
fn every_read_command_runs_and_produces_an_envelope() {
    let example = Example::materialise("commands");
    let cases: &[(&str, &[&str])] = &[
        ("check", &["check"]),
        ("build", &["build"]),
        ("view", &["view", "roadmap"]),
        ("get", &["get", "@sys.milestone.m3-playable-day"]),
        ("why-current", &["why-current", "@sys.work.m3-plan"]),
        ("impact", &["impact", "@sim.obs.projection-gaps"]),
        ("review-queue", &["review-queue"]),
        ("lock", &["lock", "--check"]),
        (
            "context",
            &["context", "--goal", "sys.milestone.m3-playable-day"],
        ),
        ("explain", &["explain", "AKR-G013"]),
    ];
    for (name, args) in cases {
        let text = example.run(args);
        assert_eq!(text.code, 0, "akr {}: {}", args.join(" "), text.output());
        assert!(
            !text.stdout.is_empty(),
            "akr {} printed nothing",
            args.join(" ")
        );

        let mut json = vec!["--format", "json"];
        json.extend_from_slice(args);
        let envelope = example.run(&json);
        assert_eq!(
            envelope.code,
            0,
            "akr {} --format json: {}",
            args.join(" "),
            envelope.output()
        );
        assert_envelope(&envelope.stdout, name);
    }
}

#[test]
fn fmt_build_and_lock_are_idempotent_on_a_clean_workspace() {
    let example = Example::materialise("fmt-build-lock");
    let check = example.run(&["fmt", "--check"]);
    assert_eq!(check.code, 0, "{}", check.output());
    assert!(
        check.stdout.contains("canonically formatted"),
        "{}",
        check.stdout
    );

    let run = example.run(&["fmt"]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert!(run.stdout.contains("unchanged"), "{}", run.stdout);

    let first = example.run(&["build"]);
    let second = example.run(&["build"]);
    assert_eq!(first.code, 0, "{}", first.output());
    assert_eq!(second.code, 0, "{}", second.output());
    // The second run rewrites nothing: a build that churns files makes the D-025 CI gate
    // unusable, because every checkout would show a diff.
    assert!(
        second.stdout.contains("unchanged") || second.stdout.contains("0 written"),
        "a second build must write nothing:\n{}",
        second.stdout
    );
    assert_eq!(example.run(&["check", "--views-current"]).code, 0);

    assert_eq!(example.run(&["lock", "--check"]).code, 0);
    assert_eq!(example.run(&["lock"]).code, 0);
    assert_eq!(example.run(&["lock", "--check"]).code, 0);
}

#[test]
fn dirty_source_lock_stays_current_when_the_exact_ledger_is_committed() {
    let example = Example::materialise("build-lock-postcommit");
    example.write_file("new-work.akr", "intent \"A freshly authored work item.\"\n");
    let run = example.run(&[
        "propose",
        "sys.work.lock-provenance",
        "--kind",
        "work",
        "--from",
        "new-work.akr",
    ]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert_eq!(example.run(&["build"]).code, 0);
    let before = example.read_file(".akr/akr.lock");
    assert!(!before.contains("    commit git:"));
    example.git(&["add", "-A"]);
    example.git(&["commit", "-m", "Commit the exact prebuilt ledger"]);
    for args in [vec!["build", "--check"], vec!["lock", "--check"]] {
        let run = example.run(&args);
        assert_eq!(run.code, 0, "{}", run.output());
        assert!(
            !run.output()
                .contains("lock check                   changed")
        );
    }
    assert_eq!(before, example.read_file(".akr/akr.lock"));
    example.write_file("more-work.akr", "intent \"A subsequent ledger edit.\"\n");
    assert_eq!(
        example
            .run(&[
                "propose",
                "sys.work.lock-drift",
                "--kind",
                "work",
                "--from",
                "more-work.akr"
            ])
            .code,
        0
    );
    let run = example.run(&["build", "--check"]);
    assert_eq!(run.code, 1, "{}", run.output());
    assert!(run.output().contains("AKR-R052"));
}

#[test]
fn build_check_detects_view_drift() {
    let example = Example::materialise("build-check");
    let build = example.run(&["build"]);
    assert_eq!(build.code, 0, "{}", build.output());

    let roadmap = example.root().join("docs/generated/ROADMAP.md");
    let mut text = std::fs::read_to_string(&roadmap).expect("roadmap view exists");
    text.push_str("\n<!-- drift introduced by build-check test -->\n");
    std::fs::write(&roadmap, text).expect("mutate view");

    let check = example.run(&["build", "--check"]);
    assert_eq!(check.code, 1, "{}", check.output());
    assert!(
        check.stdout.contains("DIFFERS") || check.stderr.contains("AKR-E011"),
        "{}",
        check.output()
    );
}

#[test]
fn build_and_lock_report_the_lock_path_relative_to_the_repo_root() {
    let example = Example::materialise("lock-path");
    // The materialised example's lock is current, and a current lock prints
    // "unchanged" instead of the path under test — so force a rewrite.
    std::fs::remove_file(example.root().join(".akr/akr.lock")).expect("remove lock");
    let build = example.run(&["build"]);
    assert_eq!(build.code, 0, "{}", build.output());
    assert!(
        build.stdout.contains("wrote .akr/akr.lock"),
        "build must name the repo-root-relative lock path:\n{}",
        build.stdout
    );

    // `akr lock` always writes, so no setup is needed for the second message.
    let lock = example.run(&["lock"]);
    assert_eq!(lock.code, 0, "{}", lock.output());
    assert!(
        lock.stdout.contains("wrote .akr/akr.lock"),
        "lock must name the repo-root-relative lock path:\n{}",
        lock.stdout
    );
}

#[test]
fn init_scaffolds_a_workspace_and_never_overwrites() {
    let dir = std::env::temp_dir().join(format!("akr-p6-init-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp directory");

    let run = |args: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_akr"))
            .args(args)
            .current_dir(&dir)
            .output()
            .expect("runs")
    };
    let first = run(&["init", "--project", "demo"]);
    assert_eq!(first.status.code(), Some(0), "{first:?}");
    assert!(dir.join(".akr/project.akr").is_file());
    assert!(dir.join(".akr/records").is_dir());
    assert!(dir.join(".akr/archive").is_dir());

    let agents = std::fs::read_to_string(dir.join("AGENTS.md")).expect("AGENTS.md");
    // D-049: the one brief, between the markers the setup scripts refresh in place.
    assert!(agents.contains("<!-- AKR_START -->"), "{agents}");
    assert!(agents.contains("<!-- AKR_END -->"), "{agents}");
    assert!(agents.contains("## AKR (project knowledge)"));
    assert!(agents.contains("knowledge.context"));
    assert!(
        agents.contains("help the next agent understand"),
        "the brief says what the ledger is for"
    );
    assert!(
        agents.contains("akr change begin") && agents.contains("akr git commit"),
        "the brief names the commit protocol"
    );
    assert_eq!(
        agents.matches("## AKR (project knowledge)").count(),
        1,
        "written once"
    );
    let ignore = std::fs::read_to_string(dir.join(".gitignore")).expect(".gitignore");
    assert!(ignore.contains(".akr/cache/"));
    assert!(ignore.contains(".agent/scratch/"));

    // A fresh workspace checks clean, which is the only useful definition of "scaffolded".
    let check = run(&["check"]);
    assert_eq!(
        check.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );

    let second = run(&["init"]);
    assert_eq!(second.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&second.stderr).contains("AKR-C013"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn explain_covers_both_diagnostic_registries() {
    let example = Example::materialise("explain");
    // `C` codes are this crate's, `L` codes are the language half: `akr explain` reads both
    // registries so an agent never has to know which half a code came from.
    for code in [
        "AKR-C011", "AKR-G013", "AKR-G014", "AKR-E011", "AKR-L001", "AKR-R051",
    ] {
        let run = example.run(&["explain", code]);
        assert_eq!(run.code, 0, "{code}: {}", run.output());
        assert!(run.stdout.contains(code), "{code}: {}", run.stdout);
    }
    let unknown = example.run(&["explain", "AKR-Z999"]);
    assert_eq!(unknown.code, 2, "{}", unknown.output());

    let observation = example.run(&["explain", "observation"]);
    assert_eq!(observation.code, 0, "{}", observation.output());
    assert!(
        observation
            .stdout
            .contains("observed_at: commit (git:<40-hex>)")
    );
    assert!(observation.stdout.contains("relations"));
    assert!(observation.stdout.contains("verified requires provenance"));

    let decision = example.run(&["explain", "D-011"]);
    assert_eq!(decision.code, 0, "{}", decision.output());
    assert!(decision.stdout.contains("docs/DECISIONS.md"));
}

#[test]
fn context_and_start_normalize_paths_and_reject_ambiguous_or_outside_inputs() {
    let example = Example::materialise("context-read-contracts");

    let current = example.run(&["context", "--goal", "@sys.milestone.m3-playable-day/1"]);
    assert_eq!(current.code, 0, "{}", current.output());
    let historical = example.run(&["context", "--goal", "@sys.work.m3-plan/1"]);
    assert_eq!(historical.code, 3, "{}", historical.output());
    assert!(historical.output().contains("AKR-X004"));
    assert!(
        historical
            .output()
            .contains("akr get sys.work.m3-plan --history")
    );
    let anchored = example.run(&[
        "context",
        "--goal",
        "@sys.milestone.m3-playable-day#no-placeholder-assets",
    ]);
    assert_eq!(anchored.code, 3, "{}", anchored.output());
    assert!(anchored.output().contains("AKR-X005"));
    assert!(anchored.output().contains("akr get"));

    let baseline = example.run(&[
        "context",
        "--goal",
        "sys.milestone.m3-playable-day",
        "--paths",
        "sim/src/project/**",
    ]);
    assert_eq!(baseline.code, 0, "{}", baseline.output());
    for path in [
        r"sim\src\project\**".to_owned(),
        r"sim/src\project/**".to_owned(),
        format!(r"{}\sim\src\project\**", example.root().display()),
    ] {
        let run = example.run(&[
            "context",
            "--goal",
            "sys.milestone.m3-playable-day",
            "--paths",
            &path,
        ]);
        assert_eq!(run.code, 0, "{path}: {}", run.output());
        assert_eq!(run.stdout, baseline.stdout, "{path}");
    }

    let outside_root = example
        .root()
        .parent()
        .expect("materialised root has a parent")
        .join("definitely-outside-the-repository");
    let mut sibling_name = example
        .root()
        .file_name()
        .expect("root has a name")
        .to_string_lossy()
        .into_owned();
    sibling_name.push('2');
    let sibling_root = example
        .root()
        .parent()
        .expect("materialised root has a parent")
        .join(sibling_name);
    for bad_root in [outside_root, sibling_root] {
        let bad = format!(r"{}\src\lib.rs", bad_root.display());
        let run = example.run(&[
            "context",
            "--goal",
            "sys.milestone.m3-playable-day",
            "--paths",
            &bad,
        ]);
        assert_eq!(run.code, 3, "{}", run.output());
        assert!(run.output().contains("AKR-X013"), "{}", run.output());
        assert!(
            run.output().contains("outside the repository"),
            "{}",
            run.output()
        );
    }

    let strong = example.run(&["start", "projection", "--format", "json"]);
    assert_eq!(strong.code, 0, "{}", strong.output());
    assert!(
        strong.stdout.contains("\"recommended_context\""),
        "{}",
        strong.stdout
    );
    let weak = example.run(&["start", "widget projection overhaul", "--format", "json"]);
    assert_eq!(weak.code, 0, "{}", weak.output());
    assert!(
        weak.stdout.contains("rewrite-projection"),
        "{}",
        weak.stdout
    );
    assert!(
        !weak.stdout.contains("\"recommended_context\""),
        "{}",
        weak.stdout
    );
    assert!(weak.stdout.contains("\"weak_match\""), "{}", weak.stdout);

    let unique = example.run(&["get", "term.renderer-boundary"]);
    assert_eq!(unique.code, 3, "{}", unique.output());
    assert!(unique.output().contains("AKR-L001"), "{}", unique.output());
    assert!(
        unique
            .output()
            .contains("did you mean @lege.term.renderer-boundary?"),
        "{}",
        unique.output()
    );
    let ambiguous = example.run(&["get", "renderer-boundary"]);
    assert_eq!(ambiguous.code, 3, "{}", ambiguous.output());
    for key in [
        "@lege.decision.renderer-boundary",
        "@lege.term.renderer-boundary",
    ] {
        assert!(ambiguous.output().contains(key), "{}", ambiguous.output());
    }
}

// -------------------------------------------------------------------------------------
// The second worked example
// -------------------------------------------------------------------------------------

#[test]
fn the_tandem_example_validate_alias_and_explain_enums_work_together() {
    let example = Example::of(&SYS_TANDEM, "sys-tandem");
    let check = example.run(&["check"]);
    assert_eq!(check.code, 0, "{}", check.output());
    let validate = example.run(&["validate"]);
    assert_eq!(check.code, validate.code, "{}", validate.output());
    assert_eq!(check.stdout, validate.stdout);
    assert_eq!(
        example.run(&["check", "--review-clean"]).code,
        example.run(&["validate", "--review-clean"]).code
    );

    let queue = example.run(&["review-queue"]);
    assert_eq!(queue.code, 0, "{}", queue.output());
    assert!(
        queue.stdout.contains("2 stale, 4 at risk"),
        "{}",
        queue.stdout
    );

    let observation = example.run(&["explain", "observation"]);
    assert_eq!(observation.code, 0, "{}", observation.output());
    assert!(
        observation
            .stdout
            .contains("method: manual|command|instrumented|observation"),
        "{}",
        observation.stdout
    );
    let evidence = example.run(&["explain", "evidence"]);
    assert!(
        evidence.stdout.contains("result: pass|fail|inconclusive"),
        "{}",
        evidence.stdout
    );
    let decision = example.run(&["explain", "decision"]);
    assert!(decision.stdout.contains("V-021"), "{}", decision.stdout);
    assert!(
        decision.stdout.contains("acknowledged true"),
        "a kind that accepts `contradicts` should say what V-023 wants: {}",
        decision.stdout
    );
    let work = example.run(&["explain", "work"]);
    assert!(!work.stdout.contains("V-021"), "{}", work.stdout);
    assert!(
        work.stdout.contains("topic      not accepted"),
        "{}",
        work.stdout
    );
    assert!(work.stdout.contains("AKR-C004"), "{}", work.stdout);
    let policy = example.run(&["explain", "policy"]);
    assert!(
        !policy.stdout.contains("topic      not accepted"),
        "{}",
        policy.stdout
    );
}

#[test]
fn an_unknown_state_names_the_states_the_kind_has() {
    // A word that is no lifecycle state at all used to be refused without naming one. The
    // word an author reaches for is usually a real lifecycle word from somewhere else —
    // `accepted`, for a decision the user settled — and the bare refusal sent them to
    // `explain` for a list the diagnostic already had.
    let example = Example::of(&SYS_TANDEM, "cli-unknown-state");
    example.write_file(
        ".akr/records/settled.akr",
        "akr 0.1\nproject sys\n\nrecord sys.decision.settled/1 : decision {\n    \
         title \"Settled\"\n    state accepted\n    scope all\n    decision \"\"\"\n        \
         The user settled it.\n        \"\"\"\n}\n",
    );
    let run = example.run(&["check"]);
    assert!(run.output().contains("AKR-T012"), "{}", run.output());
    assert!(
        run.output().contains("proposed")
            && run.output().contains("active")
            && run.output().contains("withdrawn"),
        "the refusal should name the decision lifecycle: {}",
        run.output()
    );
}

#[test]
fn get_shows_the_acceptance_checks_that_complete_will_demand() {
    // `akr complete` demands a mapping for every check, and the block used to appear only
    // under `--detail canonical` — so the first `complete` on a multi-check record was
    // always refused for mappings its author had never been shown.
    let example = Example::materialise("cli-get-acceptance");
    let run = example.run(&["get", "@sys.milestone.m3-playable-day"]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert!(
        run.stdout.contains("acceptance"),
        "the body should carry the block: {}",
        run.stdout
    );

    let json = example.run(&["get", "@sys.milestone.m3-playable-day", "--format", "json"]);
    let checks = json
        .stdout
        .find("\"acceptance\"")
        .map(|at| &json.stdout[at..]);
    let checks = checks.expect("the JSON half carries it too");
    for field in ["\"id\"", "\"statement\"", "\"method\"", "\"verdict\""] {
        assert!(
            checks.contains(field),
            "a check needs {field} to be actionable: {checks}"
        );
    }
}

#[test]
fn a_context_budget_is_a_promise_about_the_delivered_bundle() {
    // The budget is stated in tokens of result and used to be measured against the records
    // the assembler selected, which is a much smaller number: a caller who asked for a
    // small bundle received one several times the size, transport-truncated. Assembly now
    // measures what it rendered — both halves, since MCP sends both — and reports the two
    // figures so the promise is inspectable rather than assumed.
    let example = Example::materialise("cli-context-budget");
    let run = example.run(&[
        "context",
        "--goal",
        "sys.milestone.m3-playable-day",
        "--budget",
        "900",
        "--format",
        "json",
    ]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert!(
        run.stdout.contains("\"requested_tokens\": 900"),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout.contains("\"delivered_tokens\""),
        "{}",
        run.stdout
    );

    // Without a budget there is nothing to report, and no field appears.
    let unbudgeted = example.run(&[
        "context",
        "--goal",
        "sys.milestone.m3-playable-day",
        "--format",
        "json",
    ]);
    assert!(
        !unbudgeted.stdout.contains("requested_tokens"),
        "{}",
        unbudgeted.stdout
    );
}

/// A1/A2: view currency is checked by a PLAIN `akr check`, not only under
/// `--views-current`. Ten of fifteen workspaces in a 2026-09 sweep were shipping views
/// that did not match their ledger and every one of them passed a plain check.
#[test]
fn plain_check_reports_a_content_difference_in_a_view() {
    let example = Example::materialise("plain-check-view-drift");
    assert_eq!(example.run(&["build"]).code, 0);
    // A committed ledger is what makes a stale view a shipped defect rather than unsaved
    // work; uncommitted, the same difference is AKR-E016 (see the test below).
    example.git(&["add", "-A"]);
    example.git(&["commit", "--quiet", "-m", "build"]);

    let roadmap = example.root().join("docs/generated/ROADMAP.md");
    let mut text = std::fs::read_to_string(&roadmap).expect("roadmap view exists");
    text.push_str("\n<!-- a hand edit -->\n");
    std::fs::write(&roadmap, text).expect("mutate view");

    // No `--views-current`: this is the whole point of the change.
    let check = example.run(&["check"]);
    assert_eq!(check.code, 1, "{}", check.output());
    assert!(
        check.output().contains("AKR-E011"),
        "a content difference is an error:\n{}",
        check.output()
    );
}

/// A `tool:`-only difference is AKR-E015 at warning severity and does NOT fail the check,
/// even under the default `--strict`. It is a migration artifact that disappears the
/// first time anyone rebuilds, and no agent can clear it mid-session.
#[test]
fn plain_check_warns_but_does_not_fail_on_a_tool_version_difference() {
    let example = Example::materialise("plain-check-tool-drift");
    assert_eq!(example.run(&["build"]).code, 0);

    let roadmap = example.root().join("docs/generated/ROADMAP.md");
    let text = std::fs::read_to_string(&roadmap).expect("roadmap view exists");
    let stamped: String = text
        .lines()
        .map(|line| {
            if line.trim_start().starts_with("tool: ") {
                "     tool: akr 0.0.1-from-an-older-binary".to_owned()
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    std::fs::write(&roadmap, stamped).expect("mutate view");

    let check = example.run(&["check"]);
    assert_eq!(
        check.code,
        0,
        "a stale tool stamp must not fail the check:\n{}",
        check.output()
    );
    assert!(
        check.output().contains("AKR-E015"),
        "it must still be reported:\n{}",
        check.output()
    );
    assert!(
        !check.output().contains("AKR-E011"),
        "and must not be reported as a content difference:\n{}",
        check.output()
    );
}

/// A2: a workspace with no generated views at all passes a plain check today. pixelkit
/// had six AKR-E012 under `--views-current` and none from `akr check`.
#[test]
fn plain_check_reports_a_missing_view() {
    let example = Example::materialise("plain-check-missing-view");
    assert_eq!(example.run(&["build"]).code, 0);
    example.git(&["add", "-A"]);
    example.git(&["commit", "--quiet", "-m", "build"]);

    std::fs::remove_file(example.root().join("docs/generated/ROADMAP.md")).expect("remove view");

    let check = example.run(&["check"]);
    assert_eq!(check.code, 1, "{}", check.output());
    assert!(
        check.output().contains("AKR-E012"),
        "a missing view is an error:\n{}",
        check.output()
    );
}

/// The mid-session case: an agent that has just written a record has stale views by
/// construction. That is unsaved work, not a shipped defect, so it is AKR-E016 at warning
/// severity rather than AKR-E011 — the same argument that exempts AKR-G004.
///
/// The exit code is deliberately not asserted: writing a record without rebuilding also
/// leaves `akr.lock` stale (AKR-R052), which is a separate registered defect and would
/// make the assertion test something other than its subject.
#[test]
fn a_stale_view_under_an_uncommitted_ledger_is_a_warning_not_an_error() {
    let example = Example::materialise("plain-check-uncommitted-ledger");
    assert_eq!(example.run(&["build"]).code, 0);
    example.git(&["add", "-A"]);
    example.git(&["commit", "--quiet", "-m", "build"]);
    // Clean tree, current views: the baseline the rest of this rests on.
    assert_eq!(example.run(&["check"]).code, 0);

    // Write a record without rebuilding, exactly as a session in progress does.
    let wrote = example.run(&["papercut", "-m", "tester", "A record written mid-session."]);
    assert_eq!(wrote.code, 0, "{}", wrote.output());

    let check = example.run(&["check"]);
    assert!(
        check.output().contains("AKR-E016"),
        "a stale view under an uncommitted ledger is AKR-E016:\n{}",
        check.output()
    );
    assert!(
        !check.output().contains("AKR-E011") && !check.output().contains("AKR-E012"),
        "and is never reported as a shipped stale view:\n{}",
        check.output()
    );
}

#[test]
fn recorded_gate_commands_check_for_phantom_filters_and_pipes() {
    let example = Example::materialise("g024-g025-gates");
    example.write_file(
        "src/gate_fixture.rs",
        "// fn a_real_leaf_test\n// mod inventory_model; mod tests; fn a_leaf\n",
    );
    example.write_file("docs/handoff/trace.jsonl", "a_name_only_in_jsonl\n");

    let cases: &[(&str, &str, bool, bool)] = &[
        (
            "phantom",
            "cargo test -p sys_present a_test_name_that_exists_nowhere",
            true,
            false,
        ),
        (
            "real",
            "cargo test -p sys_present a_real_leaf_test",
            false,
            false,
        ),
        ("no-filter", "just check", false, false),
        (
            "env-underscore",
            "SYS_WORLD=greyhaven cargo run --release",
            false,
            false,
        ),
        (
            "jsonl",
            "cargo test -p sys_present a_name_only_in_jsonl",
            true,
            false,
        ),
        (
            "one-of-two",
            "cargo test -p sys_present a_real_leaf_test a_second_name_that_is_absent",
            false,
            false,
        ),
        (
            "module-path",
            "cargo test -p sys_present inventory_model::tests::a_leaf",
            false,
            false,
        ),
        (
            "pipe",
            "cargo test -p sys_present a_real_leaf_test | tail -80",
            false,
            true,
        ),
        (
            "escaped",
            "cargo fmt --check &amp;&amp; cargo test -p sys_present a_real_leaf_test",
            false,
            true,
        ),
        (
            "plain-and",
            "cargo fmt --check && cargo test -p sys_present a_real_leaf_test",
            false,
            false,
        ),
        (
            "quoted-jq",
            r#"jq -e '{course2_completed: any(.trace[]; contains(\"step: Course(2)\") and contains(\"state: Done\")), clearing_completed: any(.trace[]; contains(\"step: Clearing\") and contains(\"state: Done\")), finalized_course2_rows: ([.courses[] | select(contains(\"course: 2,\"))] | length)} | ., (.course2_completed and .clearing_completed)' SYSEngine/review-evidence/x/dinner-1735-attends.json"#,
            false,
            false,
        ),
        (
            "bash-c",
            "bash -c 'cargo test -p sys_present a_real_leaf_test | tail'",
            false,
            true,
        ),
        (
            "quoted-word",
            "cargo test -p sys_present 'a_test_name_that_exists_nowhere'",
            true,
            false,
        ),
    ];

    let mut extra = String::from(
        "\nrecord sys.work.gate-superseded/1 : work {\n    title \"Superseded gate\"\n    state superseded\n    intent \"Superseded.\"\n    acceptance {\n        check the-gate {\n            statement \"Check.\"\n            method command\n            command \"cargo test -p sys_present a_test_name_that_exists_nowhere\"\n        }\n    }\n}\n\nrecord sys.work.gate-superseded/2 : work {\n    title \"Superseded gate, restated\"\n    state proposed\n    intent \"Supersedes the phantom command.\"\n}\n",
    );
    for (slug, command, _, _) in cases {
        extra.push_str(&format!(
            "\nrecord sys.work.gate-{slug}/1 : work {{\n    title \"Gate {slug}\"\n    state proposed\n    intent \"Gate check {slug}.\"\n    acceptance {{\n        check the-gate {{\n            statement \"Gate.\"\n            method command\n            command \"{command}\"\n        }}\n    }}\n}}\n"
        ));
    }
    let work_path = ".akr/records/sys/work.akr";
    let existing = example.read_file(work_path);
    example.write_file(work_path, &format!("{existing}{extra}"));
    example.git(&["add", "-A"]);
    example.git(&["commit", "--quiet", "-m", "gate fixture"]);

    let build = example.run(&["build"]);
    assert_eq!(build.code, 0, "{}", build.output());
    let check = example.run(&["check", "--format", "json"]);
    let parsed = akr_core::json::parse(&check.stdout).expect("json");
    let diagnostics = parsed
        .get("diagnostics")
        .and_then(akr_core::json::Value::as_array)
        .expect("diagnostics array");

    let has_code = |slug: &str, code: &str| {
        let key = format!("sys.work.gate-{slug}");
        diagnostics.iter().any(|d| {
            d.get("code").and_then(akr_core::json::Value::as_str) == Some(code)
                && d.get("message")
                    .and_then(akr_core::json::Value::as_str)
                    .is_some_and(|m| m.contains(&key))
        })
    };

    assert!(!has_code("superseded", "AKR-G024"), "{}", check.stdout);
    for (slug, _, expect_g024, expect_g025) in cases {
        assert_eq!(
            has_code(slug, "AKR-G024"),
            *expect_g024,
            "gate-{slug} AKR-G024 mismatch:\n{}",
            check.stdout
        );
        assert_eq!(
            has_code(slug, "AKR-G025"),
            *expect_g025,
            "gate-{slug} AKR-G025 mismatch:\n{}",
            check.stdout
        );
    }
}

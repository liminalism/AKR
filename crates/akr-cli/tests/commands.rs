//! Every command of `docs/07-cli.md` §6, exercised through the real binary.
//!
//! The transcripts pin what four commands *print*; this pins that the rest run at all,
//! exit as §3 says, and produce a JSON envelope with the fields §5 fixes. Between them the
//! read surface has no command that has never been executed.

mod support;

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
fn propose_help_shows_a_slot_list_example() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_akr"))
        .args(["propose", "--help"])
        .output()
        .expect("runs");
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{text}");
    assert!(text.contains("slot-list"), "{text}");
    assert!(text.contains("intent"), "{text}");
}

#[test]
fn help_and_version_need_no_workspace() {
    let dir = std::env::temp_dir();
    for args in [vec!["--help"], vec!["--version"]] {
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
fn fmt_leaves_a_canonical_workspace_alone() {
    let example = Example::materialise("fmt");
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
}

#[test]
fn fmt_and_init_refuse_json() {
    let example = Example::materialise("no-json");
    for args in [
        vec!["--format", "json", "fmt", "--check"],
        vec!["--format", "json", "init"],
    ] {
        let run = example.run(&args);
        assert_eq!(run.code, 2, "{args:?}: {}", run.output());
        assert!(run.stderr.contains("AKR-C041"), "{}", run.stderr);
    }
}

#[test]
fn build_is_idempotent() {
    let example = Example::materialise("build-twice");
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
fn lock_check_agrees_with_the_lock_the_tool_writes() {
    let example = Example::materialise("lock-check");
    assert_eq!(example.run(&["lock", "--check"]).code, 0);
    assert_eq!(example.run(&["lock"]).code, 0);
    assert_eq!(example.run(&["lock", "--check"]).code, 0);
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
    assert!(agents.contains("## Project knowledge (AKR)"));
    assert!(agents.contains("knowledge.context"));
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
fn no_command_is_deferred_any_longer() {
    let example = Example::materialise("deferred");
    // Nothing remains deferred: P7 delivered `search` and P8 delivered `import`, so the
    // whole command surface of `docs/07-cli.md` is now wired. `import` in particular no
    // longer reports itself unbuilt (the old `AKR-M002 … arrives with P8`); it runs, and
    // a missing document is an ordinary ledger diagnostic — exit 1, `AKR-M001` — not the
    // environment failure a deferred command used to raise.
    let deferred = example.run(&["import", "docs/legacy.md"]);
    assert_eq!(deferred.code, 1, "{}", deferred.output());
    let text = deferred.output();
    assert!(text.contains("AKR-M001"), "{text}");
    assert!(
        !text.contains("arrives with"),
        "import is no longer deferred: {text}"
    );

    // And `search` is delivered too, in a binary that has a ranker at all.
    #[cfg(feature = "fts5")]
    {
        assert_eq!(example.run(&["build"]).code, 0);
        let search = example.run(&["search", "projection"]);
        assert_eq!(search.code, 0, "{}", search.output());
    }
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
fn context_normalizes_a_current_pin_and_guides_other_reference_forms() {
    let example = Example::materialise("context-reference-contract");
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
}

#[test]
fn context_paths_accepts_native_separators_and_an_absolute_in_repo_path() {
    let example = Example::materialise("context-paths-normalization");
    let baseline = example.run(&[
        "context",
        "--goal",
        "sys.milestone.m3-playable-day",
        "--paths",
        "sim/src/project/**",
    ]);
    assert_eq!(baseline.code, 0, "{}", baseline.output());

    // A backslash path — what an agent on Windows naturally types — must produce the
    // identical bundle, not `AKR-X011` ("backslashes are not path separators").
    let backslash = example.run(&[
        "context",
        "--goal",
        "sys.milestone.m3-playable-day",
        "--paths",
        r"sim\src\project\**",
    ]);
    assert_eq!(backslash.code, 0, "{}", backslash.output());
    assert_eq!(
        backslash.stdout, baseline.stdout,
        "a backslash path must normalize to the same bundle as its forward-slash form"
    );

    // Mixed separators normalize the same way.
    let mixed = example.run(&[
        "context",
        "--goal",
        "sys.milestone.m3-playable-day",
        "--paths",
        r"sim/src\project/**",
    ]);
    assert_eq!(mixed.code, 0, "{}", mixed.output());
    assert_eq!(mixed.stdout, baseline.stdout);

    // An absolute path inside the repository — the shape an agent gets from `pwd`-joining a
    // relative one, or pastes straight from an editor — is made repo-root-relative rather
    // than rejected as `AKR-X011: globs are repo-root-relative and may not start with `/``.
    let absolute = format!(r"{}\sim\src\project\**", example.root().display());
    let absolute_run = example.run(&[
        "context",
        "--goal",
        "sys.milestone.m3-playable-day",
        "--paths",
        &absolute,
    ]);
    assert_eq!(absolute_run.code, 0, "{}", absolute_run.output());
    assert_eq!(absolute_run.stdout, baseline.stdout);
}

#[test]
fn context_paths_rejects_an_absolute_path_outside_the_repository() {
    let example = Example::materialise("context-paths-outside-repo");
    let outside_root = example
        .root()
        .parent()
        .expect("materialised root has a parent")
        .join("definitely-outside-the-repository");
    let outside = format!(r"{}\src\lib.rs", outside_root.display());

    let run = example.run(&[
        "context",
        "--goal",
        "sys.milestone.m3-playable-day",
        "--paths",
        &outside,
    ]);
    assert_eq!(run.code, 3, "{}", run.output());
    assert!(run.output().contains("AKR-X013"), "{}", run.output());
    assert!(
        run.output().contains("outside the repository"),
        "{}",
        run.output()
    );

    // A sibling directory that merely shares a name prefix with the repository root (e.g.
    // `…\commands-context-paths-outside-repo2`) must not be treated as inside it.
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
    let sibling = format!(r"{}\src\lib.rs", sibling_root.display());
    let sibling_run = example.run(&[
        "context",
        "--goal",
        "sys.milestone.m3-playable-day",
        "--paths",
        &sibling,
    ]);
    assert_eq!(sibling_run.code, 3, "{}", sibling_run.output());
    assert!(
        sibling_run.output().contains("AKR-X013"),
        "{}",
        sibling_run.output()
    );
}

// -------------------------------------------------------------------------------------
// The second worked example
// -------------------------------------------------------------------------------------

/// `examples/sys-tandem/` through the binary.
///
/// The second worked example, materialised the same way: its history is synthetic too, so
/// the binary can only see it against a real repository built from `MANIFEST.md` §2. What
/// is asserted is what the manifest freezes — a clean check, and a queue of two stale and
/// four at-risk records — which is the claim the example exists to make.
#[test]
fn the_tandem_example_checks_clean_and_has_the_queue_its_manifest_declares() {
    let example = Example::of(&SYS_TANDEM, "sys-tandem");
    let check = example.run(&["check"]);
    assert_eq!(check.code, 0, "{}", check.output());

    let queue = example.run(&["review-queue"]);
    assert_eq!(queue.code, 0, "{}", queue.output());
    assert!(
        queue.stdout.contains("2 stale, 4 at risk"),
        "{}",
        queue.stdout
    );
}

// -------------------------------------------------------------------------------------
// Help coverage
// -------------------------------------------------------------------------------------

/// Every command answers its own `--help`, listed in the banner or not.
///
/// `akr <command> --help` routes through `help_for`, which returns `Option`: a command
/// with no topic is reported as an *unknown command* rather than as an undocumented one.
/// `git-hook` sat in that gap, so probing its help said it did not exist while the command
/// ran perfectly well — which is a far more misleading answer than no help at all.
#[test]
fn every_command_answers_its_own_help() {
    let example = Example::of(&SYS_TANDEM, "sys-tandem");
    for command in akr_cli::args::COMMANDS {
        let help = example.run(&[command, "--help"]);
        assert_eq!(help.code, 0, "`akr {command} --help`: {}", help.output());
        assert!(
            !help.stdout.contains("unknown command"),
            "`akr {command} --help` reports it as unknown: {}",
            help.stdout
        );
    }

    // The other direction: a command the banner advertises but `COMMANDS` omits is one
    // `nearest` can never suggest after a typo.
    let top = example.run(&["--help"]);
    assert_eq!(top.code, 0, "{}", top.output());
    for listed in top
        .stdout
        .lines()
        .skip_while(|line| !line.starts_with("COMMANDS"))
        .skip(1)
        .take_while(|line| !line.trim().is_empty())
        .filter_map(|line| line.split_whitespace().next())
    {
        assert!(
            akr_cli::args::COMMANDS.contains(&listed),
            "`{listed}` is advertised by --help but missing from COMMANDS"
        );
    }
}

#[test]
fn validate_is_check_under_the_name_the_mcp_tool_uses() {
    // The protocol every agent reads says "run knowledge.validate before handing work
    // back". From a shell that was not a command, and two sessions in one sister project
    // spent the difference working out that `check` was the same thing.
    let example = Example::of(&SYS_TANDEM, "cli-validate-alias");
    let check = example.run(&["check"]);
    let validate = example.run(&["validate"]);
    assert_eq!(check.code, validate.code, "{}", validate.output());
    assert_eq!(check.stdout, validate.stdout);

    // The flags come with it, and so does the help.
    assert_eq!(
        example.run(&["check", "--review-clean"]).code,
        example.run(&["validate", "--review-clean"]).code
    );
    let help = example.run(&["validate", "--help"]);
    assert_eq!(help.code, 0, "{}", help.output());
    assert!(help.stdout.contains("akr validate"), "{}", help.stdout);
    assert!(!help.stdout.contains("unknown command"), "{}", help.stdout);
}

#[test]
fn explain_prints_the_members_of_a_closed_slot_and_the_state_entry_rules() {
    // `explain <kind>` is what both write surfaces point at for "which slots may this
    // kind have". Answering that with the word `enum` for the one slot with a fixed
    // vocabulary sent the author back to guessing — `measurement` is the obvious guess
    // for an observation's `method`, and it is wrong. The state-entry rules are the same
    // shape of omission: V-021 refused an author's first `active` decision after listing
    // `supported_by` among seven optional relations and saying nothing about one of them
    // being required.
    let example = Example::of(&SYS_TANDEM, "cli-explain-enums");

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

    // A kind with no closed slot and no state-entry rule gains neither line.
    let work = example.run(&["explain", "work"]);
    assert!(!work.stdout.contains("V-021"), "{}", work.stdout);

    // `topic`, both ways round. Listing only what a kind accepts left an author who had
    // seen `topic` on the write surface to assume it was universal, and the refusal
    // arrived as an AKR-C004 from a rule `explain` had every chance to mention.
    assert!(
        work.stdout.contains("topic      not accepted"),
        "a planning kind should say it rejects `topic`: {}",
        work.stdout
    );
    assert!(
        work.stdout.contains("AKR-C004"),
        "and name the code it will be refused with: {}",
        work.stdout
    );
    let policy = example.run(&["explain", "policy"]);
    assert!(
        !policy.stdout.contains("topic      not accepted"),
        "a normative kind accepts it: {}",
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
        check.code, 0,
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
/// Sets up a fixture carrying one work record whose single acceptance check runs
/// `command`, plus a committed source file holding `source_text`.
///
/// Self-contained on purpose. The shipped fixtures hold placeholder content, so a test
/// that borrowed a token from them would assert on something incidental. Committing
/// matters too: the detector searches the tracked tree at a resolved commit, so a fixture
/// with no commit exercises nothing at all — which is how the first draft of these tests
/// passed four cases while the detector never ran.
fn gate_fixture(name: &str, command: &str, source_text: &str) -> Example {
    let template = r#"
record NS.work.gate-under-test/1 : work {
    title "A gate under test"
    state proposed
    intent """
        Exists only to carry one acceptance command.
        """
    acceptance {
        check the-gate {
            statement """
                Whatever the command proves.
                """
            method command
            command "CMD"
        }
    }
}
"#;
    gate_fixture_with_template(name, source_text, &template.replace("CMD", command))
}

/// Same shape as [`gate_fixture`], but the phantom command lands on a superseded
/// revision rather than the live head.
///
/// Nobody can revise a non-head revision to repoint a renamed test — only the head is
/// writable — so if the gate fired here it would be permanent noise on a papercut that
/// was already fixed on the successor.
fn superseded_gate_fixture(name: &str, command: &str, source_text: &str) -> Example {
    let template = r#"
record NS.work.gate-under-test/1 : work {
    title "A gate under test"
    state superseded
    intent """
        Exists only to carry one acceptance command, later superseded.
        """
    acceptance {
        check the-gate {
            statement """
                Whatever the command proves.
                """
            method command
            command "CMD"
        }
    }
}

record NS.work.gate-under-test/2 : work {
    title "A gate under test, restated"
    state proposed
    intent """
        Supersedes the revision that carried the phantom command.
        """
}
"#;
    gate_fixture_with_template(name, source_text, &template.replace("CMD", command))
}

/// Sets up a fixture carrying `template` (with `NS` standing for the example's real
/// namespace) plus a committed source file holding `source_text`.
///
/// Self-contained on purpose. The shipped fixtures hold placeholder content, so a test
/// that borrowed a token from them would assert on something incidental. Committing
/// matters too: the detector searches the tracked tree at a resolved commit, so a fixture
/// with no commit exercises nothing at all — which is how the first draft of these tests
/// passed four cases while the detector never ran.
fn gate_fixture_with_template(name: &str, source_text: &str, template: &str) -> Example {
    let example = Example::materialise(name);
    let src = example.root().join("src/gate_fixture.rs");
    std::fs::create_dir_all(src.parent().expect("parent")).expect("create src dir");
    std::fs::write(&src, format!("// {source_text}\n")).expect("write source");

    let dir = std::fs::read_dir(example.root().join(".akr/records"))
        .expect("records dir")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.is_dir())
        .expect("a namespace directory");
    let file = std::fs::read_dir(&dir)
        .expect("namespace dir")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "akr"))
        .expect("a record file");
    let existing = std::fs::read_to_string(&file).expect("read record file");
    let namespace = existing
        .lines()
        .find_map(|l| l.strip_prefix("record "))
        .and_then(|r| r.split('.').next())
        .unwrap_or("sys")
        .to_owned();

    let added = template.replace("NS", &namespace);
    std::fs::write(&file, format!("{existing}{added}")).expect("write record file");

    example.git(&["add", "-A"]);
    example.git(&["commit", "--quiet", "-m", "gate fixture"]);
    example
}

/// A8: a gate naming a test that exists nowhere in tracked source passes having run
/// nothing, because `cargo test <filter>` with a filter matching nothing exits 0.
#[test]
fn a_command_naming_a_test_that_does_not_exist_is_reported() {
    let example = gate_fixture(
        "g024-phantom",
        "cargo test -p sys_present a_test_name_that_exists_nowhere",
        "nothing relevant here",
    );
    let check = example.run(&["check"]);
    assert!(
        check.output().contains("AKR-G024"),
        "a phantom test filter must be reported:\n{}",
        check.output()
    );
}

/// A superseded revision's phantom command must not be reported: nobody can revise a
/// non-head revision to repoint a renamed test, so this would otherwise be permanent
/// noise on a papercut that was already fixed on the successor.
#[test]
fn a_superseded_revisions_phantom_command_is_silent() {
    let example = superseded_gate_fixture(
        "g024-superseded",
        "cargo test -p sys_present a_test_name_that_exists_nowhere",
        "nothing relevant here",
    );
    assert_eq!(example.run(&["build"]).code, 0);
    let check = example.run(&["check"]);
    assert!(
        !check.output().contains("AKR-G024"),
        "a superseded revision's phantom command must stay silent:\n{}",
        check.output()
    );
}

/// The same shape naming a test that really is in tracked source stays silent.
#[test]
fn a_command_naming_a_real_test_is_silent() {
    let example = gate_fixture(
        "g024-real",
        "cargo test -p sys_present a_real_leaf_test",
        "fn a_real_leaf_test",
    );
    let check = example.run(&["check"]);
    assert!(
        !check.output().contains("AKR-G024"),
        "a real test name must not be reported:\n{}",
        check.output()
    );
}

/// A command with no identifier-shaped token is not a test filter and is never reported.
/// `just check` and `cargo check --workspace` must stay silent, or the diagnostic becomes
/// noise on every ledger that gates on a whole suite.
#[test]
fn a_command_with_no_test_filter_is_silent() {
    let example = gate_fixture("g024-no-filter", "just check", "nothing relevant here");
    let check = example.run(&["check"]);
    assert!(
        !check.output().contains("AKR-G024"),
        "a command with no identifier-shaped token must not be reported:\n{}",
        check.output()
    );
}

/// Two tokens where only one exists: the gate can still run something, so it is silent.
/// The rule is "none of them resolves", not "any of them fails".
#[test]
fn a_command_is_silent_when_any_of_its_tokens_exists() {
    let example = gate_fixture(
        "g024-one-of-two",
        "cargo test -p sys_present a_real_leaf_test a_second_name_that_is_absent",
        "fn a_real_leaf_test",
    );
    let check = example.run(&["check"]);
    assert!(
        !check.output().contains("AKR-G024"),
        "one resolving token is enough to keep a gate silent:\n{}",
        check.output()
    );
}

/// THE REGRESSION THAT MATTERS. `cargo test` filters are SUBSTRING matches against the
/// full test path, so a module-path filter never appears literally anywhere in source —
/// `mod inventory_model;` sits in one file and `mod tests` in another, and the filter
/// runs perfectly. Matching the whole token literally is what turned a true 4.2% into a
/// reported 11.5% in the 2026-09 sweep, and Kitchen-Concept's zero real phantoms into
/// eleven apparent ones. A `::` token must resolve segment by segment.
#[test]
fn a_module_path_filter_whose_segments_exist_separately_is_silent() {
    let example = gate_fixture(
        "g024-module-path",
        "cargo test -p sys_present inventory_model::tests::a_leaf",
        "mod inventory_model; mod tests; fn a_leaf",
    );
    let check = example.run(&["check"]);
    assert!(
        !check.output().contains("AKR-G024"),
        "a module-path filter whose segments exist separately must stay silent, which is \
         the false-positive class the check exists to avoid:\n{}",
        check.output()
    );
}

/// A25: a shell pipeline reports its LAST stage's status, so a recorded command ending in
/// `| tail` exits 0 whenever `tail` succeeds — regardless of what the real command did.
/// Three agents hit this independently in one afternoon during the 2026-09 sweep.
#[test]
fn a_recorded_command_whose_status_comes_from_a_pipe_is_reported() {
    let example = gate_fixture(
        "g025-pipe",
        "cargo test -p sys_present a_real_leaf_test | tail -80",
        "fn a_real_leaf_test",
    );
    let check = example.run(&["check"]);
    assert!(
        check.output().contains("AKR-G025"),
        "a piped command's exit status is meaningless and must be reported:\n{}",
        check.output()
    );
    // The test name is real, so this is NOT the phantom diagnostic. The two defects are
    // independent: a gate can name a real test and still be unfalsifiable.
    assert!(
        !check.output().contains("AKR-G024"),
        "and it is not a phantom-token report:\n{}",
        check.output()
    );
}

/// A27: a command slot carrying an HTML-escaped `&amp;&amp;` is not the command it looks
/// like. Records authored through a surface that escapes text picked this up verbatim.
#[test]
fn a_recorded_command_with_an_html_escaped_operator_is_reported() {
    let example = gate_fixture(
        "g025-escaped",
        "cargo fmt --check &amp;&amp; cargo test -p sys_present a_real_leaf_test",
        "fn a_real_leaf_test",
    );
    let check = example.run(&["check"]);
    assert!(
        check.output().contains("AKR-G025"),
        "an HTML-escaped operator must be reported:\n{}",
        check.output()
    );
}

/// A plain `&&` chain is the ordinary way to write a two-part gate and must stay silent.
#[test]
fn a_recorded_command_using_a_plain_and_chain_is_silent() {
    let example = gate_fixture(
        "g025-plain-and",
        "cargo fmt --check && cargo test -p sys_present a_real_leaf_test",
        "fn a_real_leaf_test",
    );
    let check = example.run(&["check"]);
    assert!(
        !check.output().contains("AKR-G025"),
        "a plain && chain is not a pipe and must stay silent:\n{}",
        check.output()
    );
}

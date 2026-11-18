//! The four write tools of `docs/08-mcp.md` §4, and the guarantees §5 and §7 attach.
//!
//! The claims under test are the ones an agent would be harmed by if they were false: a
//! refused write leaves nothing behind, `base_rev` catches a moved head, and a supersession
//! missing a disposition names the children in the payload rather than in prose.

#[path = "differential.rs"]
mod differential;
#[path = "secrets.rs"]
mod secrets;
mod support;
#[path = "trace_and_pass.rs"]
mod trace_and_pass;

use akr_core::json::Value;
use support::{Example, mcp_call};

#[test]
fn absent_commit_is_rejected_before_propose_and_evidence_batch_write() {
    let example = Example::materialise("mcp-invalid-commit-atomicity");
    let before = example.sources();
    for (tool, arguments) in [
        (
            "knowledge.propose",
            r#"{"key":"sys.observation.missing-commit","kind":"observation","title":"Missing commit","slots":{"statement":"Observed on an absent commit.","observed_at":"git:1111111111111111111111111111111111111111","method":"manual"}}"#,
        ),
        (
            "knowledge.evidence_add_many",
            r#"{"evidence":[{"key":"sys.evidence.missing-commit","result":"pass","method":"manual","observed_at":"1111111111111111111111111111111111111111"}]}"#,
        ),
    ] {
        let (payload, is_error) = call(&example, tool, arguments);
        assert!(is_error, "{tool}: {payload:?}");
        assert!(error_text(&payload).contains("AKR-G011"), "{payload:?}");
        assert_eq!(
            before,
            example.sources(),
            "{tool} wrote before checking the commit"
        );
    }
}

/// `knowledge.revise` runs the commit-presence gate `knowledge.propose` runs, before
/// writing, and `akr revise --from` refuses the same input the same way. Before, the
/// revision landed and only the next `akr check` reported AKR-G011.
#[test]
fn absent_commit_is_rejected_before_revise_writes_on_both_surfaces() {
    let example = Example::materialise("mcp-revise-invalid-commit");
    let absent = "1111111111111111111111111111111111111111";
    let before = example.sources();

    let (payload, is_error) = call(
        &example,
        "knowledge.revise",
        &format!(
            r#"{{"key":"sim.obs.projection-gaps","base_rev":1,"slots":{{"observed_at":"git:{absent}"}}}}"#
        ),
    );
    assert!(is_error, "{payload:?}");
    assert!(error_text(&payload).contains("AKR-G011"), "{payload:?}");
    assert_eq!(before, example.sources(), "knowledge.revise wrote first");

    example.write_file("revision.akr", &format!("observed_at git:{absent}\n"));
    let before = example.sources();
    let run = example.run(&[
        "revise",
        "sim.obs.projection-gaps",
        "--from",
        "revision.akr",
    ]);
    assert_eq!(run.code, 3, "{}", run.output());
    assert!(run.output().contains("AKR-G011"), "{}", run.output());
    assert_eq!(before, example.sources(), "akr revise wrote first");

    // A revise that leaves an inherited commit alone is not refused for it (D-039), and a
    // real commit is accepted on both surfaces.
    let (payload, is_error) = call(
        &example,
        "knowledge.revise",
        r#"{"key":"sim.obs.projection-gaps","base_rev":1,"title":"Projection coverage gaps, retitled"}"#,
    );
    assert!(!is_error, "{}", error_text(&payload));
    example.write_file(
        "revision.akr",
        &format!("observed_at git:{}\n", example.commit(5)),
    );
    let run = example.run(&[
        "revise",
        "sim.obs.projection-gaps",
        "--from",
        "revision.akr",
    ]);
    assert_eq!(run.code, 0, "{}", run.output());
}

/// Runs `git` in the example and asserts it succeeded.
fn git(example: &Example, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(example.root())
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The field report, on the surface it was hit on: `knowledge.revise` rewriting an
/// acceptance block to re-cite evidence observed before the check's last redefinition.
/// The uncommitted revision had no `last_change`, which read as "no git facts", so the
/// evidence was taken at its word and the check showed as satisfied until the revision
/// was committed. It now reads as not satisfied before the commit, as it does after.
#[test]
fn knowledge_revise_does_not_launder_evidence_older_than_the_definition() {
    let example = Example::materialise("mcp-revise-uncommitted-recitation");
    let key = "sys.work.recitation-probe";
    let seal_and_commit = |message: &str| {
        assert_eq!(example.run(&["build"]).code, 0);
        git(&example, &["add", "-A"]);
        git(&example, &["commit", "--quiet", "-m", message]);
    };
    let acceptance = |statement: &str, cites: &str| {
        format!(
            r#"[{{"id":"fixture","statement":"{statement}","method":"command","command":"cargo test"{cites}}}]"#
        )
    };
    // Read through the MCP surface itself, so the verdict is the server's.
    let verdict = || {
        let (payload, is_error) = call(&example, "knowledge.get", &format!(r#"{{"ref":"{key}"}}"#));
        assert!(!is_error, "{}", error_text(&payload));
        let text = payload.to_pretty();
        text.lines()
            .find(|line| line.contains("\"verdict\""))
            .unwrap_or_else(|| panic!("no verdict in {text}"))
            .to_owned()
    };

    let (payload, is_error) = call(
        &example,
        "knowledge.propose",
        &format!(
            r#"{{"key":"{key}","kind":"work","title":"Probe","state":"active","slots":{{"intent":"Hold the fixture green."}},"acceptance":{}}}"#,
            acceptance("The fixture passes.", "")
        ),
    );
    assert!(!is_error, "{}", error_text(&payload));
    seal_and_commit("probe work");
    let (payload, is_error) = call(
        &example,
        "knowledge.evidence_add",
        r#"{"key":"sys.evidence.e5","result":"pass","method":"command","command":"cargo test","summary":"The fixture passed."}"#,
    );
    assert!(!is_error, "{}", error_text(&payload));
    seal_and_commit("e5");

    let (payload, is_error) = call(
        &example,
        "knowledge.revise",
        &format!(
            r#"{{"key":"{key}","base_rev":1,"acceptance":{}}}"#,
            acceptance("The fixture passes under the new harness.", "")
        ),
    );
    assert!(!is_error, "{}", error_text(&payload));
    seal_and_commit("redefine the check");

    // Re-cite E5 and redefine again, uncommitted: the rev-8 shape of the report.
    let (payload, is_error) = call(
        &example,
        "knowledge.revise",
        &format!(
            r#"{{"key":"{key}","base_rev":2,"acceptance":{}}}"#,
            acceptance(
                "The fixture passes on every platform.",
                r#","verified_by":["@sys.evidence.e5/1"]"#
            )
        ),
    );
    assert!(!is_error, "{}", error_text(&payload));
    let pending = verdict();
    assert!(
        pending.contains("predates the uncommitted change"),
        "{pending}"
    );
    seal_and_commit("re-cite e5");
    let committed = verdict();
    assert!(
        committed.contains("predates the last change"),
        "{committed}"
    );
}

#[test]
fn configuration_edit_uses_the_cli_pipeline_and_is_available_over_mcp() {
    let example = Example::materialise("mcp-config");
    let original = example.read_file(".akr/project.akr");
    example.write_file(
        ".akr/project.akr",
        &format!("{original}\nuntracked {{ path \"human/**\" path \"binary/**\" }}\n"),
    );
    let (payload, is_error) = call(
        &example,
        "knowledge.config_edit",
        r#"{"action":"remove_untracked","path":"human/**"}"#,
    );
    assert!(!is_error, "{payload:?}");
    let text = example.read_file(".akr/project.akr");
    assert!(!text.contains("human/**"));
    assert!(text.contains("binary/**"));
    let (payload, is_error) = call(&example, "knowledge.config_get", "{}");
    assert!(!is_error, "{payload:?}");
    assert_eq!(
        payload.get("configuration").and_then(Value::as_str),
        Some(text.as_str())
    );
}

#[test]
fn capture_digest_works_for_single_and_batched_mcp_evidence() {
    let example = Example::materialise("mcp-evidence-capture");
    example.write_file(".agent/scratch/raw.bin", "abc");
    for (tool, arguments) in [
        (
            "knowledge.evidence_add",
            r#"{"key":"sys.evidence.capture-single","result":"pass","method":"manual","summary":"Capture inspected.","capture":".agent/scratch/raw.bin"}"#,
        ),
        (
            "knowledge.evidence_add_many",
            r#"{"evidence":[{"key":"sys.evidence.capture-batch","result":"pass","method":"manual","summary":"Capture inspected.","capture":".agent/scratch/raw.bin"}]}"#,
        ),
    ] {
        let (payload, is_error) = call(&example, tool, arguments);
        assert!(!is_error, "{payload:?}");
    }
    std::fs::remove_file(example.root().join(".agent/scratch/raw.bin")).unwrap();
    for key in ["sys.evidence.capture-single", "sys.evidence.capture-batch"] {
        let run = example.run(&["get", key]);
        assert!(
            run.stdout
                .contains("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
        assert!(!run.stdout.contains("\n  artifact"));
    }
    assert_eq!(example.run(&["build"]).code, 0);
}

#[test]
fn mcp_and_cli_revise_prune_the_same_historical_dispositions() {
    let example = Example::materialise("mcp-revise-stale-dispositions");
    example.write_file(
        "parent.akr",
        "state active\nintent \"A parent work item.\"\n",
    );
    example.write_file(
        "child.akr",
        "intent \"A child on the original revision.\"\npart_of [ @sys.work.parent/1 ]\n",
    );
    for (key, file) in [
        ("sys.work.parent", "parent.akr"),
        ("sys.work.child", "child.akr"),
    ] {
        let run = example.run(&["propose", key, "--kind", "work", "--from", file]);
        assert_eq!(run.code, 0, "{}", run.output());
    }
    let run = example.run(&[
        "revise",
        "sys.work.parent",
        "--title",
        "First successor",
        "--disposition",
        "sys.work.child=intentionally_dropped",
    ]);
    assert_eq!(run.code, 0, "{}", run.output());
    let (payload, is_error) = call(
        &example,
        "knowledge.revise",
        r#"{"key":"sys.work.parent","base_rev":2,"title":"Second successor"}"#,
    );
    assert!(!is_error, "{payload:?}");
    let run = example.run(&["get", "sys.work.parent/3", "--detail", "canonical"]);
    assert!(!run.stdout.contains("disposition @sys.work.child"));
    let run = example.run(&["get", "sys.work.parent/2", "--detail", "canonical"]);
    assert!(run.stdout.contains("disposition @sys.work.child"));
}

/// One `tools/call` in-process, returning `(payload, is_error)`.
fn call(example: &Example, tool: &str, arguments: &str) -> (Value, bool) {
    mcp_call(example.root(), tool, arguments)
}

fn error_class(payload: &Value) -> Option<&str> {
    payload
        .get("error")
        .and_then(|e| e.get("class"))
        .and_then(Value::as_str)
}

fn error_text(payload: &Value) -> String {
    payload.to_pretty()
}

#[test]
fn propose_creates_a_record_and_refuses_the_same_key_twice() {
    let example = Example::materialise("mcp-propose");
    let arguments = r#"{
        "key": "sys.policy.day-loop",
        "kind": "policy",
        "title": "The day loop",
        "state": "active",
        "scope": ["all"],
        "slots": { "rule": "The repeating structure of one in-game day: wake, work, evening, sleep." }
    }"#;
    let (payload, is_error) = call(&example, "knowledge.propose", arguments);
    assert!(!is_error, "{}", error_text(&payload));
    assert_eq!(
        payload.get("key").and_then(Value::as_str),
        Some("sys.policy.day-loop")
    );
    assert_eq!(payload.get("rev").and_then(Value::as_integer), Some(1));
    assert_eq!(payload.get("written").and_then(Value::as_bool), Some(true));
    // Every write stales the lock, and the payload says so rather than leaving the agent
    // to discover it from the next `knowledge.validate` (D-014).
    assert_eq!(
        payload.get("lock_stale").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        payload
            .get("next")
            .and_then(|next| next.get("command"))
            .and_then(Value::as_str),
        Some("akr build")
    );
    // §4's remaining two fields describe the revision as it landed, not as it was planned,
    // so they are worth asserting: an agent that read a stale `state` back would think its
    // lifecycle move had not taken.
    assert_eq!(
        payload.get("state").and_then(Value::as_str),
        Some("active"),
        "{}",
        error_text(&payload)
    );
    assert!(
        payload
            .get("content_hash")
            .and_then(Value::as_str)
            .is_some_and(|hash| hash.starts_with("sha256:")),
        "{}",
        error_text(&payload)
    );
    assert!(
        example
            .read_file(".akr/records/sys/policies.akr")
            .contains("sys.policy.day-loop/1")
    );

    // §4: an existing key is an error. The tool never silently turns a proposal into a
    // revision, because an agent that meant to create something and edited it instead has
    // no way to notice.
    let before = example.sources();
    let (payload, is_error) = call(&example, "knowledge.propose", arguments);
    assert!(is_error);
    assert_eq!(
        error_class(&payload),
        Some("invariant"),
        "{}",
        error_text(&payload)
    );
    assert_eq!(
        before,
        example.sources(),
        "a refused write left something behind"
    );
}

#[test]
fn schema_and_argument_refusals_write_nothing() {
    let example = Example::materialise("mcp-schema-refusals");
    let before = example.sources();

    // Scope syntax errors.
    let (payload, is_error) = call(
        &example,
        "knowledge.propose",
        r#"{"key":"sys.policy.bad-scope","kind":"policy","title":"Bad scope","scope":["path src/**"],"slots":{"rule":"A term."}}"#,
    );
    assert!(is_error);
    let text = error_text(&payload);
    assert!(text.contains("rendered AKR syntax"), "{text}");
    assert!(text.contains("src/**"), "{text}");

    let (payload, is_error) = call(
        &example,
        "knowledge.propose",
        r#"{"key":"sys.policy.bad-scope-object","kind":"policy","title":"Bad scope",
            "scope":[{"path":"src/**"}],"slots":{"rule":"A term."}}"#,
    );
    assert!(is_error);
    let text = error_text(&payload);
    assert!(text.contains("form all|path|ref"), "{text}");

    // Topic on non-normative kind.
    let (payload, is_error) = call(
        &example,
        "knowledge.propose",
        r#"{"key":"sys.work.publication","kind":"work","title":"Publish",
            "topic":"publication","slots":{"intent":"Publish the release."}}"#,
    );
    assert!(is_error);
    let text = error_text(&payload);
    assert!(text.contains("only valid on normative kinds"), "{text}");
    assert!(text.contains("work"), "{text}");

    // Line citation without document.
    let (payload, is_error) = call(
        &example,
        "knowledge.propose",
        r#"{"key":"sys.policy.no-document","kind":"policy",
            "title":"T","scope":["all"],"slots":{"rule":"S."},
            "sources":[{"kind":"external","start_line":1,"end_line":2}]}"#,
    );
    assert!(is_error, "{payload:?}");
    assert!(error_text(&payload).contains("document"), "{payload:?}");

    // Claim named like required slot.
    let (payload, is_error) = call(
        &example,
        "knowledge.propose",
        r#"{"key":"sys.work.claim-intent","kind":"work","title":"Claim named intent","claims":[{"anchor":"intent","text":"Do the thing."}]}"#,
    );
    assert!(is_error, "{payload:?}");
    let text = error_text(&payload);
    assert!(text.contains("slots.intent"), "{text}");
    assert!(text.contains("not a slot"), "{text}");

    // Slot the kind does not have -> `schema` class, and record with no body.
    let (payload, is_error) = call(
        &example,
        "knowledge.propose",
        r#"{"key":"sys.policy.day-loop","kind":"policy","title":"T",
            "slots":{"observed_at":"git:0000000000000000000000000000000000000000"}}"#,
    );
    assert!(is_error);
    assert_eq!(
        error_class(&payload),
        Some("schema"),
        "{}",
        error_text(&payload)
    );

    let (payload, is_error) = call(
        &example,
        "knowledge.propose",
        r#"{"key":"sys.policy.day-loop","kind":"policy","title":"T"}"#,
    );
    assert!(is_error);
    assert!(
        error_text(&payload).contains("rule"),
        "{}",
        error_text(&payload)
    );

    // Revise missing or stale base_rev.
    let (payload, is_error) = call(
        &example,
        "knowledge.revise",
        r#"{"key":"sys.term.playable-day"}"#,
    );
    assert!(is_error);
    assert_eq!(
        error_class(&payload),
        Some("usage"),
        "{}",
        error_text(&payload)
    );

    let (payload, is_error) = call(
        &example,
        "knowledge.revise",
        r#"{"key":"sys.term.playable-day","base_rev":7}"#,
    );
    assert!(is_error);
    assert_eq!(
        error_class(&payload),
        Some("conflict"),
        "{}",
        error_text(&payload)
    );
    assert_eq!(
        payload
            .get("error")
            .and_then(|e| e.get("retryable"))
            .and_then(Value::as_bool),
        Some(true),
        "a conflict is the one class an agent may retry"
    );

    assert_eq!(
        before,
        example.sources(),
        "refused writes must leave the workspace byte-identical"
    );
}

#[test]
fn propose_can_create_a_milestone_with_an_acceptance_block() {
    // V-008 requires a milestone to carry a non-empty `acceptance` block from the moment
    // it exists, and `knowledge.propose` had no field for one — the only way to author a
    // milestone was `akr propose --from` with a hand-written record body. This is the fix:
    // an agent should be able to do the whole thing over MCP.
    let example = Example::materialise("mcp-propose");
    let arguments = r#"{
        "key": "sys.milestone.m2",
        "kind": "milestone",
        "title": "M2",
        "slots": { "intent": "Ship the second milestone." },
        "acceptance": [
            { "id": "full-day-demo", "statement": "A full day loop runs end to end.",
              "method": "manual" }
        ]
    }"#;
    let (payload, is_error) = call(&example, "knowledge.propose", arguments);
    assert!(!is_error, "{}", error_text(&payload));
    assert_eq!(payload.get("rev").and_then(Value::as_integer), Some(1));
    let source = example.read_file(".akr/records/sys/milestones.akr");
    assert!(source.contains("acceptance {"), "{source}");
    assert!(source.contains("check full-day-demo"), "{source}");
}

#[test]
fn propose_can_adopt_an_exact_registered_source_passage() {
    let example = Example::materialise("mcp-propose-source-citation");
    let advice = "The day loop must remain deterministic.\n";
    example.write_file("incoming-plan.md", advice);
    let added = example.run(&["source", "add", "incoming-plan.md", "--id", "incoming-plan"]);
    assert_eq!(added.code, 0, "{}", added.output());

    let arguments = format!(
        r#"{{
            "key": "sys.policy.deterministic-day-loop",
            "kind": "policy",
            "title": "The day loop stays deterministic",
            "scope": ["all"],
            "slots": {{ "rule": "The day loop must remain deterministic." }},
            "sources": [{{
                "kind": "external",
                "role": "origin",
                "document": "incoming-plan",
                "start_byte": 0,
                "end_byte": {},
                "start_line": 1,
                "end_line": 1,
                "use": "Adopted as a project requirement."
            }}]
        }}"#,
        advice.len()
    );
    let (payload, is_error) = call(&example, "knowledge.propose", &arguments);
    assert!(!is_error, "{}", error_text(&payload));
    let written = example.read_file(".akr/records/sys/policies.akr");
    assert!(written.contains("document \"incoming-plan\""), "{written}");
    assert!(
        written.contains(&format!("end_byte {}", advice.len())),
        "{written}"
    );
    assert!(written.contains("role origin"), "{written}");
    assert!(written.contains("use \"\"\""), "{written}");
    assert_eq!(example.run(&["build"]).code, 0);
    let checked = example.run(&["check"]);
    assert_eq!(checked.code, 0, "{}", checked.output());
}

#[test]
fn propose_locates_a_citation_given_by_line_alone() {
    // The language stores all four coordinates or none, but an author reads a document by
    // line. Demanding the byte offsets of them meant counting bytes by hand; naming the
    // document is now enough to have them read off the registered bytes.
    let example = Example::materialise("mcp-propose-source-by-line");
    let advice = "Preamble line.\nThe day loop must remain deterministic.\nTrailing line.\n";
    example.write_file("incoming-plan.md", advice);
    let added = example.run(&["source", "add", "incoming-plan.md", "--id", "incoming-plan"]);
    assert_eq!(added.code, 0, "{}", added.output());

    let arguments = r#"{
        "key": "sys.policy.deterministic-day-loop",
        "kind": "policy",
        "title": "The day loop stays deterministic",
        "scope": ["all"],
        "slots": { "rule": "The day loop must remain deterministic." },
        "sources": [{
            "kind": "external",
            "role": "origin",
            "document": "incoming-plan",
            "start_line": 2,
            "end_line": 2,
            "use": "Adopted as a project requirement."
        }]
    }"#;
    let (payload, is_error) = call(&example, "knowledge.propose", arguments);
    assert!(!is_error, "{}", error_text(&payload));

    let written = example.read_file(".akr/records/sys/policies.akr");
    assert!(written.contains("document \"incoming-plan\""), "{written}");
    assert!(written.contains("start_byte 15"), "{written}");
    assert!(written.contains("end_byte 55"), "{written}");
    assert!(written.contains("start_line 2"), "{written}");
    assert!(written.contains("end_line 2"), "{written}");
    // Located bytes carry their own hash, so the citation verifies itself.
    assert!(written.contains("excerpt_hash \"sha256:"), "{written}");

    assert_eq!(example.run(&["build"]).code, 0);
    let checked = example.run(&["check"]);
    assert_eq!(checked.code, 0, "{}", checked.output());
}

#[test]
fn propose_normalizes_a_line_citation_ending_on_a_blank_line() {
    // The locator and AKR-S022 must use one convention. Previously the write accepted
    // end_line 3, generated bytes ending at the blank line, and the next validation
    // reported that those bytes covered only through line 2.
    let example = Example::materialise("mcp-propose-source-blank-end");
    let advice = "Preamble.\nKeep this.\n\nTrailing.\n";
    example.write_file("incoming-plan.md", advice);
    assert_eq!(
        example
            .run(&["source", "add", "incoming-plan.md", "--id", "incoming-plan"])
            .code,
        0
    );

    let (payload, is_error) = call(
        &example,
        "knowledge.propose",
        r#"{
            "key":"sys.policy.keep-this","kind":"policy","title":"Keep this",
            "scope":["all"],"slots":{"rule":"Keep this."},
            "sources":[{"kind":"external","document":"incoming-plan",
                        "start_line":2,"end_line":3}]
        }"#,
    );
    assert!(!is_error, "{}", error_text(&payload));

    let written = example.read_file(".akr/records/sys/policies.akr");
    assert!(written.contains("start_line 2"), "{written}");
    assert!(written.contains("end_line 2"), "{written}");
    assert!(written.contains("start_byte 10"), "{written}");
    assert!(written.contains("end_byte 21"), "{written}");
    let checked = example.run(&["check"]);
    assert_eq!(checked.code, 0, "{}", checked.output());
    assert!(
        !checked.output().contains("AKR-S022"),
        "{}",
        checked.output()
    );
}

#[test]
fn revise_keeps_the_head_source_attributions_it_was_not_asked_to_change() {
    // `sources` was advertised on knowledge.revise and then dropped by the merge, so a
    // revision that said nothing about provenance silently erased it, and one that
    // supplied it was ignored. Both halves are checked here.
    let example = Example::materialise("mcp-revise-sources");
    let advice = "First recommendation.\nSecond recommendation.\n";
    example.write_file("audit.md", advice);
    assert_eq!(
        example
            .run(&["source", "add", "audit.md", "--id", "audit"])
            .code,
        0
    );

    let (payload, is_error) = call(
        &example,
        "knowledge.propose",
        r#"{"key":"sys.policy.audited","kind":"policy",
            "title":"An audited requirement","scope":["all"],
            "slots":{"rule":"First recommendation."},
            "sources":[{"kind":"external","role":"origin","document":"audit",
                        "start_line":1,"end_line":1}]}"#,
    );
    assert!(!is_error, "{}", error_text(&payload));
    let written = example.read_file(".akr/records/sys/policies.akr");
    assert!(written.contains("document \"audit\""), "{written}");

    // A revision that says nothing about sources keeps them.
    let (payload, is_error) = call(
        &example,
        "knowledge.revise",
        r#"{"key":"sys.policy.audited","base_rev":1,
            "title":"An audited requirement, restated"}"#,
    );
    assert!(!is_error, "{}", error_text(&payload));
    let written = example.read_file(".akr/records/sys/policies.akr");
    assert!(
        written.contains("document \"audit\""),
        "revise dropped the attribution: {written}"
    );
    assert!(written.contains("role origin"), "{written}");

    // A revision that supplies them replaces them.
    let (payload, is_error) = call(
        &example,
        "knowledge.revise",
        r#"{"key":"sys.policy.audited","base_rev":1,
            "sources":[{"kind":"external","role":"rationale","document":"audit",
                        "start_line":2,"end_line":2}]}"#,
    );
    assert!(!is_error, "{}", error_text(&payload));
    let written = example.read_file(".akr/records/sys/policies.akr");
    assert!(written.contains("role rationale"), "{written}");
    assert!(written.contains("start_line 2"), "{written}");

    assert_eq!(example.run(&["build"]).code, 0);
    let checked = example.run(&["check"]);
    assert_eq!(checked.code, 0, "{}", checked.output());
}

#[test]
fn revise_keeps_the_slots_the_payload_does_not_mention() {
    let example = Example::materialise("mcp-revise");
    let (payload, is_error) = call(
        &example,
        "knowledge.revise",
        r#"{"key":"sys.policy.tandem-work","base_rev":1,
            "slots":{"rationale":"Rewritten after the M3 replan."}}"#,
    );
    assert!(!is_error, "{}", error_text(&payload));
    assert_eq!(payload.get("rev").and_then(Value::as_integer), Some(2));

    let source = example.read_file(".akr/records/sys/policies.akr");
    assert!(
        source.contains("Rewritten after the M3 replan."),
        "{source}"
    );
    // An agent that had to resend every slot would eventually drop one, and a dropped
    // claim is knowledge lost silently. The unmentioned ones survive.
    assert!(source.contains("claim lag-bound"), "{source}");
    assert!(source.contains("topic tandem-work"), "{source}");
    assert!(source.contains("exceptions"), "{source}");
    // And the head that was retired says so, in the same write.
    assert!(source.contains("state superseded"), "{source}");
}

#[test]
fn revise_carries_a_collated_papercut_string_array() {
    // `knowledge.revise` builds the successor payload from the head. Collated masters
    // carry the dedup keys as string[], and that merged array used to reach the generic
    // scalar renderer and make the tool reject its own payload.
    let example = Example::materialise("mcp-revise-collation");
    example.write_file(
        ".akr/records/sys/papercuts.akr",
        r#"akr 0.1
project tandem

record sys.papercut.collated/1 : papercut {
    title "Collated reports"
    state verified
    statement "Original reports."
    observed_at git:0123456789abcdef0123456789abcdef01234567
    collated [ "other.papercut.one", "other.papercut.two" ]
}
"#,
    );

    let (payload, is_error) = call(
        &example,
        "knowledge.revise",
        r#"{"key":"sys.papercut.collated","base_rev":1,"state":"withdrawn",
            "slots":{"statement":"Resolved reports."}}"#,
    );
    assert!(!is_error, "{}", error_text(&payload));
    let written = example.read_file(".akr/records/sys/papercuts.akr");
    assert!(written.contains("sys.papercut.collated/2"), "{written}");
    assert!(written.contains("other.papercut.one"), "{written}");
    assert!(written.contains("other.papercut.two"), "{written}");
}

#[test]
fn revise_closes_a_papercut_with_a_json_string_date() {
    // JSON has no date type, so `closed_at` arrives as "2026-11-03". It used to be
    // rendered as a quoted string and refused with AKR-T013 "expected a date".
    let example = Example::materialise("mcp-revise-closed-at");
    example.write_file(
        ".akr/records/sys/papercuts.akr",
        r#"akr 0.1
project tandem

record sys.papercut.closable/1 : papercut {
    title "A closable report"
    state verified
    statement "A report that was fixed."
    observed_at git:0123456789abcdef0123456789abcdef01234567
}
"#,
    );

    let (payload, is_error) = call(
        &example,
        "knowledge.revise",
        r#"{"key":"sys.papercut.closable","base_rev":1,
            "slots":{"closed_at":"2026-11-03"}}"#,
    );
    assert!(!is_error, "{}", error_text(&payload));
    let written = example.read_file(".akr/records/sys/papercuts.akr");
    assert!(written.contains("closed_at 2026-11-03"), "{written}");
}

#[test]
fn revise_honours_an_explicit_state_on_a_sealed_head() {
    let example = Example::materialise("mcp-revise-state");
    let (payload, is_error) = call(
        &example,
        "knowledge.revise",
        r#"{"key":"sys.work.m3-audio-pass","base_rev":1,"state":"active"}"#,
    );
    assert!(!is_error, "{}", error_text(&payload));
    assert_eq!(payload.get("rev").and_then(Value::as_integer), Some(2));
    assert_eq!(payload.get("state").and_then(Value::as_str), Some("active"));

    let source = example.read_file(".akr/records/sys/work.akr");
    assert!(source.contains("record sys.work.m3-audio-pass/2 : work"));
    assert!(source.contains("state active"));
}

#[test]
fn supersede_lists_the_unfinished_children_in_the_error_payload() {
    let example = Example::materialise("mcp-supersede");
    let path = ".akr/records/sim/work.akr";
    let source = example.read_file(path);
    example.write_file(
        path,
        &source.replace(
            "part_of [ @sys.milestone.m3-playable-day ]",
            "part_of [ @sys.milestone.m3-playable-day/1 ]",
        ),
    );

    let before = example.sources();
    let (payload, is_error) = call(
        &example,
        "knowledge.supersede",
        r#"{"old_key":"sys.milestone.m3-playable-day"}"#,
    );
    assert!(is_error);
    assert_eq!(
        error_class(&payload),
        Some("invariant"),
        "{}",
        error_text(&payload)
    );
    assert_eq!(
        payload
            .get("error")
            .and_then(|e| e.get("wrote"))
            .and_then(Value::as_bool),
        Some(false),
        "an agent never has to guess whether a failed write left something behind"
    );
    assert_eq!(before, example.sources());

    // §4: the children are *in the payload*, so the agent's next message can name them.
    // Structured, not parsed out of a sentence.
    let diagnostics = payload
        .get("error")
        .and_then(|e| e.get("diagnostics"))
        .and_then(Value::as_array)
        .expect("diagnostics");
    let children = diagnostics
        .iter()
        .find_map(|d| d.get("unfinished_children"))
        .and_then(Value::as_array)
        .expect("the children, structurally");
    assert_eq!(children.len(), 1, "{}", error_text(&payload));
    assert_eq!(
        children[0].get("child").and_then(Value::as_str),
        Some("sim.work.rewrite-projection")
    );
    assert_eq!(
        children[0].get("state").and_then(Value::as_str),
        Some("blocked")
    );

    // The same call with a disposition goes through.
    let (payload, is_error) = call(
        &example,
        "knowledge.supersede",
        r#"{"old_key":"sys.milestone.m3-playable-day",
            "dispositions":[{"child":"sim.work.rewrite-projection",
                             "outcome":"intentionally_dropped",
                             "note":"Rewritten under M4 instead."}]}"#,
    );
    assert!(!is_error, "{}", error_text(&payload));
    assert_eq!(payload.get("rev").and_then(Value::as_integer), Some(2));
}

#[test]
fn supersede_accepts_an_already_proposed_different_key() {
    let example = Example::materialise("mcp-supersede-key");
    let (old, is_error) = call(
        &example,
        "knowledge.propose",
        r#"{"key":"sys.policy.legacy-session","kind":"policy","title":"Legacy session","scope":["all"],"slots":{"rule":"The term being replaced."}}"#,
    );
    assert!(!is_error, "{}", error_text(&old));
    let (accepted, is_error) = call(
        &example,
        "knowledge.revise",
        r#"{"key":"sys.policy.legacy-session","base_rev":1,"slots":{"rule":"The accepted term being replaced."},"state":"active"}"#,
    );
    assert!(!is_error, "{}", error_text(&accepted));
    let (proposed, is_error) = call(
        &example,
        "knowledge.propose",
        r#"{"key":"sys.policy.playable-session","kind":"policy","title":"Playable session","scope":["all"],"slots":{"rule":"A bounded session of play."}}"#,
    );
    assert!(!is_error, "{}", error_text(&proposed));

    let (payload, is_error) = call(
        &example,
        "knowledge.supersede",
        r#"{"old_key":"sys.policy.legacy-session","new_key":"sys.policy.playable-session"}"#,
    );
    assert!(!is_error, "{}", error_text(&payload));
    assert_eq!(
        payload.get("key").and_then(Value::as_str),
        Some("sys.policy.playable-session")
    );
    let source = example.read_file(".akr/records/sys/policies.akr");
    assert!(source.contains("supersedes [ @sys.policy.legacy-session/1 ]"));
}

#[test]
fn supersede_creates_a_same_topic_decision_in_one_call() {
    // The same papercut as the CLI's: knowledge.propose of the replacement is AKR-R002
    // while both decisions are live on one topic, and knowledge.supersede used to refuse
    // content outright.
    let example = Example::materialise("mcp-supersede-create-decision");
    let (detour, is_error) = call(
        &example,
        "knowledge.propose",
        r#"{"key":"lege.decision.snapshot-interface","kind":"decision","title":"Snapshot interface","scope":["lege/**"],"topic":"renderer-boundary","slots":{"decision":"The viewer reads a snapshot interface."},"relations":{"resolves":["@lege.question.text-rendering-owner"]}}"#,
    );
    assert!(is_error, "{}", error_text(&detour));
    assert!(
        error_text(&detour).contains("AKR-R002"),
        "{}",
        error_text(&detour)
    );

    let before = example.sources();
    let (payload, is_error) = call(
        &example,
        "knowledge.supersede",
        r#"{"old_key":"lege.decision.renderer-boundary","new_key":"lege.decision.snapshot-interface","title":"Snapshot interface","scope":["lege/**"],"topic":"renderer-boundary","slots":{"decision":"The viewer reads a snapshot interface."},"relations":{"resolves":["@lege.question.text-rendering-owner"]}}"#,
    );
    assert!(!is_error, "{}", error_text(&payload));
    assert_ne!(before, example.sources());
    assert_eq!(
        payload.get("key").and_then(Value::as_str),
        Some("lege.decision.snapshot-interface")
    );
    assert_eq!(payload.get("rev").and_then(Value::as_integer), Some(1));
    let source = example.read_file(".akr/records/lege/decisions.akr");
    assert!(source.contains("record lege.decision.snapshot-interface/1 : decision"));
    assert!(source.contains("supersedes [ @lege.decision.renderer-boundary/2 ]"));
    assert!(
        example
            .read_file(".akr/records/lege/work.akr")
            .contains("depends_on [ @lege.decision.snapshot-interface ]")
    );

    // Content against an existing key is refused, not silently ignored.
    let (refused, is_error) = call(
        &example,
        "knowledge.supersede",
        r#"{"old_key":"lege.decision.snapshot-interface","new_key":"lege.decision.renderer-boundary","title":"Back again"}"#,
    );
    assert!(is_error);
    assert!(
        error_text(&refused).contains("AKR-L041"),
        "{}",
        error_text(&refused)
    );
}

#[test]
fn supersede_replaces_verified_evidence_with_a_new_record_in_one_call() {
    let example = Example::materialise("mcp-supersede-create-evidence");
    let arguments = format!(
        r#"{{"old_key":"lege.evidence.boundary-lint-pass","new_key":"lege.evidence.boundary-lint-rerun","title":"Boundary lint rerun","slots":{{"result":"pass","method":"command","observed_at":"git:{}","command":"cargo run -p tools -- boundary-lint --crate lege","summary":"Zero engine types, re-run against the corrected lint."}}}}"#,
        example.commit(4)
    );
    let (payload, is_error) = call(&example, "knowledge.supersede", &arguments);
    assert!(!is_error, "{}", error_text(&payload));
    let source = example.read_file(".akr/records/lege/evidence.akr");
    assert!(source.contains("record lege.evidence.boundary-lint-rerun/1 : evidence"));
    assert!(source.contains("supersedes [ @lege.evidence.boundary-lint-pass/1 ]"));
    assert!(source.contains("state superseded"), "{source}");
    let rerun = &source[source
        .find("record lege.evidence.boundary-lint-rerun/1")
        .expect("successor")..];
    assert!(rerun.contains("state verified"), "{rerun}");
}

#[test]
fn complete_names_the_unsatisfied_checks_in_the_error_payload() {
    let example = Example::materialise("mcp-complete");
    let before = example.sources();
    let (payload, is_error) = call(
        &example,
        "knowledge.complete",
        r#"{"key":"sys.milestone.m3-playable-day"}"#,
    );
    assert!(is_error);
    assert_eq!(
        error_class(&payload),
        Some("invariant"),
        "{}",
        error_text(&payload)
    );
    assert_eq!(before, example.sources());

    let checks = payload
        .get("error")
        .and_then(|e| e.get("diagnostics"))
        .and_then(Value::as_array)
        .expect("diagnostics")
        .iter()
        .find_map(|d| d.get("unsatisfied_checks"))
        .and_then(Value::as_array)
        .expect("the checks, structurally");
    assert_eq!(
        checks[0].get("id").and_then(Value::as_str),
        Some("no-placeholder-assets")
    );
    assert!(
        checks[0]
            .get("reason")
            .and_then(Value::as_str)
            .is_some_and(|reason| !reason.is_empty()),
        "the reason distinguishes 'no evidence' from 'evidence predates the change'"
    );
}

#[test]
fn a_write_is_visible_to_the_next_read_through_either_surface() {
    let example = Example::materialise("mcp-visible");
    let (payload, is_error) = call(
        &example,
        "knowledge.propose",
        r#"{"key":"sys.policy.day-loop","kind":"policy","title":"The day loop","state":"active",
            "scope":["all"],
            "slots":{"rule":"The repeating structure of one in-game day."}}"#,
    );
    assert!(!is_error, "{}", error_text(&payload));

    // Through the tool.
    let (fetched, is_error) = call(
        &example,
        "knowledge.get",
        r#"{"ref":"@sys.policy.day-loop"}"#,
    );
    assert!(!is_error, "{}", error_text(&fetched));
    assert_eq!(fetched.get("rev").and_then(Value::as_integer), Some(1));

    // And through the command line, which is the same ledger seen from the other side.
    let run = example.run(&["get", "@sys.policy.day-loop"]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert!(run.stdout.contains("The day loop"), "{}", run.stdout);
}

#[test]
fn evidence_add_creates_a_verified_record_with_head_as_default_commit() {
    // Bug report item 4: closing out a milestone over MCP required shelling out to
    // `akr evidence add`. The tool is the CLI command's twin: same request type, same
    // write pipeline, and — deliberately — no field for what the evidence verifies
    // (D-016).
    let example = Example::materialise("mcp-evidence");
    let arguments = r#"{
        "key": "sys.evidence.asset-audit",
        "result": "pass",
        "method": "command",
        "command": "cargo run -p tools -- audit-assets",
        "summary": "Zero placeholder assets on the day-loop path."
    }"#;
    let (payload, is_error) = call(&example, "knowledge.evidence_add", arguments);
    assert!(!is_error, "{}", error_text(&payload));
    assert_eq!(payload.get("rev").and_then(Value::as_integer), Some(1));
    // Empirical kinds have no proposal state: evidence lands `verified`.
    assert_eq!(
        payload.get("state").and_then(Value::as_str),
        Some("verified"),
        "{}",
        error_text(&payload)
    );
    let source = example.read_file(".akr/records/sys/evidence.akr");
    assert!(source.contains("sys.evidence.asset-audit/1"), "{source}");
    assert!(source.contains("result pass"), "{source}");
    // `observed_at` defaulted to HEAD — a real commit of the materialised repository.
    assert!(source.contains("observed_at"), "{source}");

    // Idempotent by key, like propose: a second add is an error, not a second record.
    let (payload, is_error) = call(&example, "knowledge.evidence_add", arguments);
    assert!(is_error, "{}", error_text(&payload));
}

#[test]
fn an_unkept_artifact_under_scratch_is_refused_on_every_evidence_route() {
    // V-025 lives in the ledger rather than on `--artifact`, because evidence reaches the
    // ledger by four routes and a guard on one flag leaves the other three open. The two
    // MCP routes are here: `knowledge.evidence_add`, which builds the request field by
    // field, and `knowledge.propose` with kind evidence, which serialises `artifact` as an
    // ordinary slot and never touches the evidence builder at all.
    let example = Example::materialise("mcp-evidence-scratch");

    let (payload, is_error) = call(
        &example,
        "knowledge.evidence_add",
        r#"{
        "key": "sys.evidence.ocr-benchmark",
        "result": "pass",
        "method": "command",
        "command": "cargo bench",
        "artifact": ".agent/scratch/ocr-benchmark/out.txt"
    }"#,
    );
    assert!(is_error, "evidence_add accepted it: {payload:?}");
    assert!(
        error_text(&payload).contains("AKR-T023"),
        "{}",
        error_text(&payload)
    );

    let (payload, is_error) = call(
        &example,
        "knowledge.propose",
        r#"{
        "key": "sys.evidence.proposed-benchmark",
        "kind": "evidence",
        "title": "Proposed benchmark",
        "slots": {
            "result": "pass",
            "method": "command",
            "command": "cargo bench",
            "artifact": ".agent/scratch/ocr-benchmark/out.txt"
        }
    }"#,
    );
    assert!(is_error, "propose accepted it: {payload:?}");
    assert!(
        error_text(&payload).contains("AKR-T023"),
        "{}",
        error_text(&payload)
    );

    assert!(
        !example
            .read_file(".akr/records/sys/evidence.akr")
            .contains(".agent/scratch"),
        "a refusal writes nothing"
    );
}

#[test]
fn an_artifact_in_a_kept_scratch_entry_is_accepted_on_every_evidence_route() {
    let example = Example::materialise("mcp-evidence-kept-scratch");
    example.write_file(".agent/scratch/ocr-benchmark/out.txt", "benchmark output\n");
    let kept = example.run(&[
        "scratch",
        "keep",
        "ocr-benchmark",
        "--reason",
        "evidence artifact must remain available",
    ]);
    assert_eq!(kept.code, 0, "{}", kept.output());

    let (payload, is_error) = call(
        &example,
        "knowledge.evidence_add",
        r#"{
            "key":"sys.evidence.kept-benchmark","result":"pass","method":"command",
            "command":"cargo bench","artifact":".agent/scratch/ocr-benchmark/out.txt"
        }"#,
    );
    assert!(!is_error, "{}", error_text(&payload));

    let (payload, is_error) = call(
        &example,
        "knowledge.propose",
        &format!(
            r#"{{
            "key":"sys.evidence.kept-proposal","kind":"evidence","title":"Kept proposal",
            "slots":{{"result":"pass","method":"command","command":"cargo bench",
                     "observed_at":"git:{}",
                     "artifact":".agent/scratch/ocr-benchmark/out.txt"}}
        }}"#,
            example.commit(5)
        ),
    );
    assert!(!is_error, "{}", error_text(&payload));
}

#[test]
fn evidence_add_many_is_one_atomic_write() {
    let example = Example::materialise("mcp-evidence-many");
    let arguments = r#"{
        "evidence": [
            {
                "key": "sys.evidence.batch-build",
                "result": "pass",
                "method": "command",
                "command": "cargo check",
                "summary": "The workspace compiled."
            },
            {
                "key": "sys.evidence.batch-review",
                "result": "pass",
                "method": "manual",
                "summary": "The manual review passed."
            }
        ]
    }"#;
    let (payload, is_error) = call(&example, "knowledge.evidence_add_many", arguments);
    assert!(!is_error, "{}", error_text(&payload));
    assert_eq!(payload.get("written").and_then(Value::as_integer), Some(2));
    assert_eq!(
        payload
            .get("evidence")
            .and_then(Value::as_array)
            .map(|items| items.len()),
        Some(2)
    );
    let source = example.read_file(".akr/records/sys/evidence.akr");
    assert!(source.contains("sys.evidence.batch-build/1"), "{source}");
    assert!(source.contains("sys.evidence.batch-review/1"), "{source}");

    let duplicate = r#"{
        "evidence": [
            {"key":"sys.evidence.batch-duplicate","result":"pass","method":"manual"},
            {"key":"sys.evidence.batch-duplicate","result":"pass","method":"manual"}
        ]
    }"#;
    let (payload, is_error) = call(&example, "knowledge.evidence_add_many", duplicate);
    assert!(is_error, "{}", error_text(&payload));
    let source = example.read_file(".akr/records/sys/evidence.akr");
    assert!(
        !source.contains("batch-duplicate"),
        "a refused batch wrote:\n{source}"
    );
}

#[test]
fn papercut_is_one_call_and_lands_in_its_own_view() {
    // D-027: the message is the whole ceremony. The tool allocates the key, fills the
    // slots, and the aggregate appears in PAPERCUTS.md on the next build.
    let example = Example::materialise("mcp-papercut");
    let arguments = r#"{
        "agent": "claude",
        "namespace": "sys",
        "message": "Ran akr search right after a write and got stale results; akr build in between fixed it."
    }"#;
    let (payload, is_error) = call(&example, "knowledge.papercut", arguments);
    assert!(!is_error, "{}", error_text(&payload));
    assert_eq!(payload.get("rev").and_then(Value::as_integer), Some(1));
    assert_eq!(
        payload.get("state").and_then(Value::as_str),
        Some("verified"),
        "{}",
        error_text(&payload)
    );
    let source = example.read_file(".akr/records/sys/papercuts.akr");
    assert!(source.contains(": papercut {"), "{source}");
    assert!(source.contains("author \"claude\""), "{source}");

    // A second papercut with the same message gets its own key, because a log never
    // refuses an entry.
    let (payload, is_error) = call(&example, "knowledge.papercut", arguments);
    assert!(!is_error, "{}", error_text(&payload));
    assert!(
        payload
            .get("key")
            .and_then(Value::as_str)
            .is_some_and(|key| key.ends_with("-2")),
        "{}",
        error_text(&payload)
    );

    // The build emits the view, newest first, with agent and citation.
    let run = example.run(&["build"]);
    assert_eq!(run.code, 0, "{}", run.output());
    let view = example.read_file("docs/generated/PAPERCUTS.md");
    assert!(view.contains("# Papercuts"), "{view}");
    assert!(view.contains("[claude]"), "{view}");
    assert!(view.contains("stale results"), "{view}");
}
#[test]
fn a_tolerated_contradiction_can_be_acknowledged_through_the_write_tools() {
    // V-023 refuses two live records that contradict each other unless one of them says
    // the contradiction is knowingly tolerated. `acknowledged` is a common slot, so it was
    // in neither `slots` nor `relations` and no field carried it: the only way through
    // this surface to satisfy the rule was to drop the `contradicts` edge, which is the
    // contradiction going unrecorded that the rule exists to prevent.
    let example = Example::materialise("mcp-acknowledged");

    let (payload, is_error) = call(
        &example,
        "knowledge.propose",
        r#"{"key":"sim.observation.frame-budget-met","kind":"observation",
            "title":"The frame budget is met","scope":["all"],
            "slots":{"statement":"The 99th-percentile frame time is inside the budget.",
                     "observed_at":"COMMIT","method":"instrumented"}}"#
            .replace("COMMIT", example.commit(4))
            .as_str(),
    );
    assert!(!is_error, "{}", error_text(&payload));

    // Without the marker the contradiction is refused: the ledger it would produce does
    // not validate, so nothing is written.
    let (payload, is_error) = call(
        &example,
        "knowledge.propose",
        r#"{"key":"sim.observation.frame-budget-missed","kind":"observation",
            "title":"The frame budget is missed","scope":["all"],
            "slots":{"statement":"A different capture put it outside the budget.",
                     "observed_at":"COMMIT","method":"instrumented"},
            "relations":{"contradicts":["@sim.observation.frame-budget-met"]}}"#
            .replace("COMMIT", example.commit(4))
            .as_str(),
    );
    assert!(is_error, "an undispositioned contradiction must be refused");
    assert!(
        error_text(&payload).contains("AKR-R041"),
        "{}",
        error_text(&payload)
    );

    // With it the write lands, and the marker is in the record.
    let (payload, is_error) = call(
        &example,
        "knowledge.propose",
        r#"{"key":"sim.observation.frame-budget-missed","kind":"observation",
            "title":"The frame budget is missed","scope":["all"],
            "slots":{"statement":"A different capture put it outside the budget.",
                     "observed_at":"COMMIT","method":"instrumented"},
            "relations":{"contradicts":["@sim.observation.frame-budget-met"]},
            "acknowledged":true}"#
            .replace("COMMIT", example.commit(4))
            .as_str(),
    );
    assert!(!is_error, "{}", error_text(&payload));
    let written = example.read_file(".akr/records/sim/observations.akr");
    assert!(written.contains("acknowledged true"), "{written}");

    // And a revision that says nothing about it keeps it — dropping the marker would
    // reintroduce AKR-R041 on the next build, from an edit that never mentioned it.
    let (payload, is_error) = call(
        &example,
        "knowledge.revise",
        r#"{"key":"sim.observation.frame-budget-missed","base_rev":1,
            "title":"The frame budget is missed on the second capture"}"#,
    );
    assert!(!is_error, "{}", error_text(&payload));
    let written = example.read_file(".akr/records/sim/observations.akr");
    assert!(
        written.contains("acknowledged true"),
        "revise dropped the marker: {written}"
    );
    assert_eq!(example.run(&["build"]).code, 0);
    let checked = example.run(&["check"]);
    assert_eq!(checked.code, 0, "{}", checked.output());
}

#[test]
fn revise_keeps_the_normative_topic_it_was_not_asked_to_change() {
    // The same shape as the `sources` merge defect: `topic` is a common slot the payload
    // builder never carried, so a revision that renamed a policy silently retired its
    // exclusivity handle (D-004b) — and a payload that supplied one was ignored.
    let example = Example::materialise("mcp-revise-topic");
    let (payload, is_error) = call(
        &example,
        "knowledge.propose",
        r#"{"key":"sys.policy.review-cadence","kind":"policy",
            "title":"Reviews run weekly","scope":["all"],"topic":"cadence",
            "slots":{"rule":"Every open plan is reviewed once a week."}}"#,
    );
    assert!(!is_error, "{}", error_text(&payload));
    assert!(
        example
            .read_file(".akr/records/sys/policies.akr")
            .contains("topic cadence")
    );

    let (payload, is_error) = call(
        &example,
        "knowledge.revise",
        r#"{"key":"sys.policy.review-cadence","base_rev":1,
            "title":"Reviews run weekly, on Mondays"}"#,
    );
    assert!(!is_error, "{}", error_text(&payload));
    assert!(
        example
            .read_file(".akr/records/sys/policies.akr")
            .contains("topic cadence"),
        "revise dropped the topic: {}",
        example.read_file(".akr/records/sys/policies.akr")
    );
}

#[test]
fn propose_many_writes_connected_track_and_ordered_milestones_atomically() {
    let example = Example::materialise("mcp-propose-many");
    let arguments = r#"{
        "records": [
            {
                "key": "sys.work.launcher",
                "kind": "work",
                "title": "Launcher work",
                "slots": { "intent": "Standing work for phased launcher work." }
            },
            {
                "key": "sys.milestone.phase-0",
                "kind": "milestone",
                "title": "Phase 0: app skeleton",
                "slots": { "intent": "Stand up the initial app skeleton." },
                "relations": {
                    "part_of": ["@sys.work.launcher"],
                    "depends_on": ["@sys.req.deterministic-sim"]
                },
                "acceptance": [
                    { "id": "skeleton-boots", "statement": "App skeleton boots.", "method": "manual" }
                ]
            },
            {
                "key": "sys.milestone.phase-1",
                "kind": "milestone",
                "title": "Phase 1: core loop",
                "slots": { "intent": "Wire the core loop." },
                "relations": {
                    "part_of": ["@sys.work.launcher"],
                    "after": ["@sys.milestone.phase-0"],
                    "depends_on": ["@sys.req.deterministic-sim"]
                },
                "acceptance": [
                    { "id": "core-loop", "statement": "Core loop passes tests.", "method": "command", "command": "cargo test" }
                ]
            }
        ]
    }"#;
    let (payload, is_error) = call(&example, "knowledge.propose_many", arguments);
    assert!(!is_error, "{}", error_text(&payload));
    assert_eq!(payload.get("written").and_then(Value::as_integer), Some(3));
    assert_eq!(
        payload
            .get("records")
            .and_then(Value::as_array)
            .map(<[Value]>::len),
        Some(3)
    );
    let tracks = example.read_file(".akr/records/sys/work.akr");
    assert!(tracks.contains("sys.work.launcher/1"), "{tracks}");
    let milestones = example.read_file(".akr/records/sys/milestones.akr");
    assert!(
        milestones.contains("sys.milestone.phase-0/1"),
        "{milestones}"
    );
    assert!(
        milestones.contains("sys.milestone.phase-1/1"),
        "{milestones}"
    );
    assert!(
        milestones.contains("after [ @sys.milestone.phase-0 ]"),
        "{milestones}"
    );
    assert_eq!(example.run(&["build"]).code, 0);
    let check = example.run(&["check"]);
    assert_eq!(check.code, 0, "{}", check.output());
}

#[test]
fn propose_many_is_atomic_and_diagnoses_milestone_implements() {
    let example = Example::materialise("mcp-propose-many-atomic");
    let before = example.sources();
    let arguments = r#"{
        "records": [
            {
                "key": "sys.work.launcher",
                "kind": "work",
                "title": "Launcher work",
                "slots": { "intent": "Standing work." }
            },
            {
                "key": "sys.milestone.phase-0",
                "kind": "milestone",
                "title": "Phase 0",
                "slots": { "intent": "Mis-declared implements on a milestone." },
                "relations": {
                    "part_of": ["@sys.work.launcher"],
                    "implements": ["@sys.req.deterministic-sim"]
                },
                "acceptance": [
                    { "id": "boot", "statement": "Boots cleanly.", "method": "manual" }
                ]
            }
        ]
    }"#;
    let (payload, is_error) = call(&example, "knowledge.propose_many", arguments);
    assert!(is_error, "{payload:?}");
    let text = error_text(&payload);
    assert!(text.contains("AKR-L032"), "{text}");
    assert!(text.contains("its domain is work, decision"), "{text}");
    assert!(text.contains("depends_on"), "{text}");
    assert_eq!(
        before,
        example.sources(),
        "a refused batch must write nothing"
    );
}

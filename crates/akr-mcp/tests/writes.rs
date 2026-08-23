//! The four write tools of `docs/08-mcp.md` §4, and the guarantees §5 and §7 attach.
//!
//! The claims under test are the ones an agent would be harmed by if they were false: a
//! refused write leaves nothing behind, `base_rev` catches a moved head, and a supersession
//! missing a disposition names the children in the payload rather than in prose.

mod support;

use akr_core::json::{Value, parse};
use support::{Example, mcp_binary, one_line};

/// One `tools/call`, returning `(payload, is_error)`.
fn call(example: &Example, tool: &str, arguments: &str) -> (Value, bool) {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let arguments = one_line(arguments);
    let request = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\
         \"params\":{{\"name\":\"{tool}\",\"arguments\":{arguments}}}}}\n"
    );
    let mut child = Command::new(mcp_binary())
        .arg("--dir")
        .arg(example.root())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("akr-mcp runs");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(request.as_bytes())
        .expect("write");
    let output = child.wait_with_output().expect("akr-mcp exits");
    let line = String::from_utf8_lossy(&output.stdout);
    let response = parse(line.trim()).unwrap_or_else(|error| {
        panic!(
            "no JSON-RPC response: {error}\n{line}\n{}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    let result = response.get("result").expect("a result").clone();
    let is_error = result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    (
        result
            .get("structuredContent")
            .cloned()
            .unwrap_or(Value::Null),
        is_error,
    )
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
fn papercut_write_preserves_a_legacy_observation_method() {
    let example = Example::materialise("mcp-papercut-legacy-observation-method");
    let legacy = format!(
        "akr 0.1\nproject save-your-skin\n\nrecord sys.observation.legacy-method/1 : observation {{\n    title \"Legacy direct observation\"\n    state verified\n    statement \"\"\"\n        A historical record used the formerly accepted direct-observation method.\n        \"\"\"\n    observed_at git:{}\n    method observation\n}}\n",
        example.commit(5)
    );
    example.write_file(".akr/records/sys/legacy-observation.akr", &legacy);

    let (payload, is_error) = call(
        &example,
        "knowledge.papercut",
        r#"{
            "agent": "regression",
            "namespace": "sys",
            "message": "The historical observation remained readable while logging this papercut."
        }"#,
    );
    assert!(!is_error, "{}", error_text(&payload));
    assert_eq!(
        example.read_file(".akr/records/sys/legacy-observation.akr"),
        legacy,
        "the write must not rewrite sealed historical bytes"
    );
}

#[test]
fn propose_creates_a_record_and_refuses_the_same_key_twice() {
    let example = Example::materialise("mcp-propose");
    let arguments = r#"{
        "key": "sys.term.day-loop",
        "kind": "term",
        "title": "The day loop",
        "state": "active",
        "scope": ["all"],
        "slots": { "definition": "The repeating structure of one in-game day: wake, work, evening, sleep." }
    }"#;
    let (payload, is_error) = call(&example, "knowledge.propose", arguments);
    assert!(!is_error, "{}", error_text(&payload));
    assert_eq!(
        payload.get("key").and_then(Value::as_str),
        Some("sys.term.day-loop")
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
            .read_file(".akr/records/sys/terms.akr")
            .contains("sys.term.day-loop/1")
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
fn propose_rejects_rendered_scope_syntax_with_the_bare_form_remedy() {
    let example = Example::materialise("mcp-scope-contract");
    let (payload, is_error) = call(
        &example,
        "knowledge.propose",
        r#"{"key":"sys.term.bad-scope","kind":"term","title":"Bad scope","scope":["path src/**"],"slots":{"definition":"A term."}}"#,
    );
    assert!(is_error);
    let text = error_text(&payload);
    assert!(text.contains("rendered AKR syntax"), "{text}");
    assert!(text.contains("src/**"), "{text}");

    let (payload, is_error) = call(
        &example,
        "knowledge.propose",
        r#"{"key":"sys.term.bad-scope-object","kind":"term","title":"Bad scope",
            "scope":[{"path":"src/**"}],"slots":{"definition":"A term."}}"#,
    );
    assert!(is_error);
    let text = error_text(&payload);
    assert!(text.contains("form all|path|ref"), "{text}");
}

#[test]
fn propose_rejects_topic_on_work_before_ledger_validation() {
    let example = Example::materialise("mcp-topic-contract");
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
            "key": "sys.requirement.deterministic-day-loop",
            "kind": "requirement",
            "title": "The day loop stays deterministic",
            "scope": ["all"],
            "slots": {{ "statement": "The day loop must remain deterministic." }},
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
    let written = example.read_file(".akr/records/sys/requirements.akr");
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
        "key": "sys.requirement.deterministic-day-loop",
        "kind": "requirement",
        "title": "The day loop stays deterministic",
        "scope": ["all"],
        "slots": { "statement": "The day loop must remain deterministic." },
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

    let written = example.read_file(".akr/records/sys/requirements.akr");
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
            "key":"sys.requirement.keep-this","kind":"requirement","title":"Keep this",
            "scope":["all"],"slots":{"statement":"Keep this."},
            "sources":[{"kind":"external","document":"incoming-plan",
                        "start_line":2,"end_line":3}]
        }"#,
    );
    assert!(!is_error, "{}", error_text(&payload));

    let written = example.read_file(".akr/records/sys/requirements.akr");
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
fn a_line_citation_without_a_document_is_a_schema_error() {
    let example = Example::materialise("mcp-propose-source-lines-no-document");
    let before = example.sources();
    let (payload, is_error) = call(
        &example,
        "knowledge.propose",
        r#"{"key":"sys.requirement.no-document","kind":"requirement",
            "title":"T","scope":["all"],"slots":{"statement":"S."},
            "sources":[{"kind":"external","start_line":1,"end_line":2}]}"#,
    );
    assert!(is_error, "{payload:?}");
    assert!(error_text(&payload).contains("document"), "{payload:?}");
    assert_eq!(before, example.sources(), "a refused write left something");
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
        r#"{"key":"sys.requirement.audited","kind":"requirement",
            "title":"An audited requirement","scope":["all"],
            "slots":{"statement":"First recommendation."},
            "sources":[{"kind":"external","role":"origin","document":"audit",
                        "start_line":1,"end_line":1}]}"#,
    );
    assert!(!is_error, "{}", error_text(&payload));
    let written = example.read_file(".akr/records/sys/requirements.akr");
    assert!(written.contains("document \"audit\""), "{written}");

    // A revision that says nothing about sources keeps them.
    let (payload, is_error) = call(
        &example,
        "knowledge.revise",
        r#"{"key":"sys.requirement.audited","base_rev":1,
            "title":"An audited requirement, restated"}"#,
    );
    assert!(!is_error, "{}", error_text(&payload));
    let written = example.read_file(".akr/records/sys/requirements.akr");
    assert!(
        written.contains("document \"audit\""),
        "revise dropped the attribution: {written}"
    );
    assert!(written.contains("role origin"), "{written}");

    // A revision that supplies them replaces them.
    let (payload, is_error) = call(
        &example,
        "knowledge.revise",
        r#"{"key":"sys.requirement.audited","base_rev":1,
            "sources":[{"kind":"external","role":"rationale","document":"audit",
                        "start_line":2,"end_line":2}]}"#,
    );
    assert!(!is_error, "{}", error_text(&payload));
    let written = example.read_file(".akr/records/sys/requirements.akr");
    assert!(written.contains("role rationale"), "{written}");
    assert!(written.contains("start_line 2"), "{written}");

    assert_eq!(example.run(&["build"]).code, 0);
    let checked = example.run(&["check"]);
    assert_eq!(checked.code, 0, "{}", checked.output());
}

#[test]
fn a_malformed_payload_is_a_schema_error_and_writes_nothing() {
    let example = Example::materialise("mcp-schema");
    let before = example.sources();
    // A slot the kind does not have. The grammar refuses it, so the class is `schema` and
    // the agent knows to fix the content rather than to retry or to escalate.
    let (payload, is_error) = call(
        &example,
        "knowledge.propose",
        r#"{"key":"sys.term.day-loop","kind":"term","title":"T",
            "slots":{"observed_at":"git:0000000000000000000000000000000000000000"}}"#,
    );
    assert!(is_error);
    assert_eq!(
        error_class(&payload),
        Some("schema"),
        "{}",
        error_text(&payload)
    );
    assert_eq!(before, example.sources());

    // And a record with no body at all: `docs/07-cli.md` §6 says a body source is
    // effectively mandatory, and the tool surface inherits that.
    let (payload, is_error) = call(
        &example,
        "knowledge.propose",
        r#"{"key":"sys.term.day-loop","kind":"term","title":"T"}"#,
    );
    assert!(is_error);
    assert_eq!(before, example.sources());
    assert!(
        error_text(&payload).contains("definition"),
        "{}",
        error_text(&payload)
    );
}

#[test]
fn revise_requires_base_rev_and_rejects_a_stale_one() {
    let example = Example::materialise("mcp-base-rev");
    let before = example.sources();

    // Missing entirely.
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

    // Present but stale: the head is at revision 1, so 7 is a claim about a ledger that
    // does not exist. §7 calls this the only concurrency control the surface has, and it
    // is enough because a human is watching the same working tree.
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
    assert_eq!(before, example.sources());
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
        r#"{"key":"sys.term.legacy-session","kind":"term","title":"Legacy session","scope":["all"],"slots":{"definition":"The term being replaced."}}"#,
    );
    assert!(!is_error, "{}", error_text(&old));
    let (accepted, is_error) = call(
        &example,
        "knowledge.revise",
        r#"{"key":"sys.term.legacy-session","base_rev":1,"slots":{"definition":"The accepted term being replaced."},"state":"active"}"#,
    );
    assert!(!is_error, "{}", error_text(&accepted));
    let (proposed, is_error) = call(
        &example,
        "knowledge.propose",
        r#"{"key":"sys.term.playable-session","kind":"term","title":"Playable session","scope":["all"],"slots":{"definition":"A bounded session of play."}}"#,
    );
    assert!(!is_error, "{}", error_text(&proposed));

    let (payload, is_error) = call(
        &example,
        "knowledge.supersede",
        r#"{"old_key":"sys.term.legacy-session","new_key":"sys.term.playable-session"}"#,
    );
    assert!(!is_error, "{}", error_text(&payload));
    assert_eq!(
        payload.get("key").and_then(Value::as_str),
        Some("sys.term.playable-session")
    );
    let source = example.read_file(".akr/records/sys/terms.akr");
    assert!(source.contains("supersedes [ @sys.term.legacy-session/1 ]"));
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
        r#"{"key":"sys.term.day-loop","kind":"term","title":"The day loop","state":"active",
            "scope":["all"],
            "slots":{"definition":"The repeating structure of one in-game day."}}"#,
    );
    assert!(!is_error, "{}", error_text(&payload));

    // Through the tool.
    let (fetched, is_error) = call(&example, "knowledge.get", r#"{"ref":"@sys.term.day-loop"}"#);
    assert!(!is_error, "{}", error_text(&fetched));
    assert_eq!(fetched.get("rev").and_then(Value::as_integer), Some(1));

    // And through the command line, which is the same ledger seen from the other side.
    let run = example.run(&["get", "@sys.term.day-loop"]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert!(run.stdout.contains("The day loop"), "{}", run.stdout);
}

#[test]
fn no_tool_can_reach_the_sqlite_cache() {
    // D-019, restated in §6: the cache is a private detail of stage E. The assertion that
    // matters is negative and structural — no tool name mentions it, and no tool's schema
    // exposes a query hole.
    for tool in akr_mcp::schema::TOOLS {
        assert!(!tool.name.contains("query"), "{}", tool.name);
        assert!(!tool.name.contains("sql"), "{}", tool.name);
        let schema = akr_mcp::schema::input_schema(tool.name)
            .expect("every declared tool has a schema")
            .to_pretty();
        assert!(!schema.to_lowercase().contains("sqlite"), "{}", tool.name);
        assert!(!schema.to_lowercase().contains("select "), "{}", tool.name);
    }
    // Every declared tool is implemented and every implemented tool is declared. The
    // count itself is deliberately not asserted: it is prose about the registry, and
    // prose about a registry is the thing that drifts.
    for tool in akr_mcp::schema::TOOLS {
        assert!(
            akr_mcp::schema::output_schema(tool.name).is_some(),
            "{} has no output schema",
            tool.name
        );
    }
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
        r#"{
            "key":"sys.evidence.kept-proposal","kind":"evidence","title":"Kept proposal",
            "slots":{"result":"pass","method":"command","command":"cargo bench",
                     "observed_at":"git:0123456789abcdef0123456789abcdef01234567",
                     "artifact":".agent/scratch/ocr-benchmark/out.txt"}
        }"#,
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

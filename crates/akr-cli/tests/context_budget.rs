//! `--budget` is a promise about the delivered bundle, and the goal is the last thing it
//! spends.
//!
//! The field report this file reproduces: `knowledge.context` with `budget_tokens: 12000`
//! on a 2,700-record ledger returned a 210,403-character bundle that the host refused and
//! spilled to a file, and the goal record in the structured half carried only its key,
//! revision and title. Two causes, both reproduced here on a fixture ledger with a large
//! normative scope:
//!
//! 1. The token estimate read a dotted key, a JSON field or a box-drawing rule as one
//!    token, so a bundle "within" budget was several times over it in any real tokenizer,
//!    and the only reduction available was prose — the keys, headers and relation lines
//!    of a hundred normative records stayed whatever the budget said.
//! 2. The JSON half never carried any record's body, the goal's included.

mod support;

use support::Example;

const GOAL: &str = "sys.milestone.m3-playable-day";
const BULK: usize = 150;

/// The example, plus `BULK` workspace-wide policies with long rules: the shape of a mature
/// ledger whose every `scope [ all ]` record lands in every bundle.
fn crowded(name: &str) -> Example {
    let example = Example::materialise(name);
    let mut text = String::from("akr 0.1\nproject save-your-skin\n");
    for n in 0..BULK {
        text.push_str(&format!(
            r#"
record sys.policy.bulk-rule-number-{n:03}/1 : policy {{
    title "Workspace-wide rule number {n} that every bundle has to carry"
    state active
    scope [ all ]
    topic bulk-topic-{n:03}
    rule """
        Rule {n} applies everywhere in the workspace, which is why the context
        assembler selects it for every goal. Its prose is long enough to matter to a
        budget: it restates the obligation, the reason for it, and the exception that
        nobody remembers, so that a reader who meets it cold can still act on it.
        """
    exceptions [ @sys.track.lighting ]
    author "dkoepke"
    created_at 2026-02-02
}}
"#
        ));
    }
    example.write_file(".akr/records/sys/bulk-policies.akr", &text);
    let locked = example.run(&["lock"]);
    assert_eq!(locked.code, 0, "{}", locked.output());
    example
}

/// Roughly four characters a token over both halves, the way a host sizes a tool result.
fn delivered_chars(text: &str, json: &str) -> usize {
    text.chars().count() + json.chars().count()
}

#[test]
fn an_over_budget_bundle_is_reduced_to_the_budget_and_keeps_the_goal() {
    let example = crowded("context-over-budget");
    let budget = 6_000usize;

    let unbudgeted = example.run(&["context", "--goal", GOAL]);
    assert_eq!(unbudgeted.code, 0, "{}", unbudgeted.output());
    let unbudgeted_json = example.run(&["context", "--goal", GOAL, "--format", "json"]);
    let full = delivered_chars(&unbudgeted.stdout, &unbudgeted_json.stdout);
    assert!(
        full / 4 > budget * 3,
        "the fixture has to be far over budget to prove anything: {full} chars, {budget}"
    );

    let args = ["context", "--goal", GOAL, "--budget", &budget.to_string()].map(str::to_owned);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let text = example.run(&args);
    assert_eq!(text.code, 0, "{}", text.output());
    let mut json_args = args.clone();
    json_args.extend(["--format", "json"]);
    let json = example.run(&json_args);
    assert_eq!(json.code, 0, "{}", json.output());

    // (b) The delivered bundle honours the budget, within the estimate's slack. The JSON
    // envelope (`akr`, `diagnostics`, …) is a few dozen tokens the result does not count.
    let delivered = delivered_chars(&text.stdout, &json.stdout);
    assert!(
        delivered / 4 <= budget + budget / 5,
        "{delivered} chars (~{} tokens) for a budget of {budget}",
        delivered / 4
    );

    // (a) The goal's intent is never what goes, in either half.
    let intent = "A player can start, play and finish one in-game day";
    assert!(text.stdout.contains(intent), "{}", text.stdout);
    let parsed = akr_core::json::parse(&json.stdout).expect("JSON");
    let result = parsed.get("result").expect("result");
    let body = result
        .get("goal")
        .and_then(|goal| goal.get("body"))
        .and_then(akr_core::json::Value::as_str)
        .expect("the goal carries its body in the structured half");
    assert!(body.contains(intent), "{body}");

    // What was withheld is named, with the way back to it.
    let withheld: Vec<String> = result
        .get("withheld")
        .and_then(akr_core::json::Value::as_array)
        .expect("withheld")
        .iter()
        .flat_map(|group| {
            group
                .get("records")
                .and_then(akr_core::json::Value::as_array)
                .unwrap_or_default()
                .iter()
                .filter_map(akr_core::json::Value::as_str)
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(!withheld.is_empty(), "{}", json.stdout);
    assert!(text.stdout.contains("WITHHELD ("), "{}", text.stdout);
    assert!(text.stdout.contains("akr get <key>"), "{}", text.stdout);
    for id in &withheld {
        let slug = id
            .split('/')
            .next()
            .and_then(|key| key.rsplit('.').next())
            .expect("a key");
        assert!(
            text.stdout.contains(slug),
            "{id} is withheld but the text form does not name it"
        );
    }
    let accounting = result.get("budget").expect("budget accounting");
    assert!(
        accounting.get("continue").is_some(),
        "a reduced bundle says how to continue: {}",
        accounting.to_pretty()
    );
    // A withheld record really is out of the sections that render records. Staleness
    // still names it: a warning is never reduced.
    let in_sections = result
        .get("sections")
        .and_then(akr_core::json::Value::as_array)
        .expect("sections")
        .iter()
        .filter(|section| {
            section.get("id").and_then(akr_core::json::Value::as_str) != Some("staleness")
        })
        .flat_map(|section| {
            section
                .get("records")
                .and_then(akr_core::json::Value::as_array)
                .unwrap_or_default()
                .to_vec()
        })
        .filter_map(|record| {
            let key = record.get("key")?.as_str()?.to_owned();
            let rev = record.get("rev")?.as_integer()?;
            Some(format!("{key}/{rev}"))
        })
        .collect::<Vec<_>>();
    for id in &withheld {
        assert!(!in_sections.contains(id), "{id} is both withheld and shown");
    }

    // Deterministic: the same request is the same bytes.
    assert_eq!(text.stdout, example.run(&args).stdout);
}

#[test]
fn a_budget_the_bundle_already_fits_reduces_nothing() {
    let example = Example::materialise("context-under-budget");
    let run = example.run(&[
        "context", "--goal", GOAL, "--budget", "100000", "--format", "json",
    ]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert!(run.stdout.contains("\"withheld\": []"), "{}", run.stdout);
    assert!(
        run.stdout.contains("\"truncated_prose\": []"),
        "{}",
        run.stdout
    );
    assert!(!run.stdout.contains("\"continue\""), "{}", run.stdout);
}

//! Per-tool output budgets, and the accounting that says whether they are working.
//!
//! `sources/context-reduction.md`. An MCP tool result does not cost what it weighs once —
//! it stays in the conversation, so every later request in the session pays for it again.
//! A four-thousand-token result read twenty times later is eighty thousand token-turns of
//! exposure. Claude Code warns at ten thousand tokens and permits twenty-five; those
//! limits are right for an exceptional log retrieval and much too generous for routine
//! project state.
//!
//! So the limits live here instead, per tool, and they are deliberately small.
//!
//! # Truncation is useful or it does not happen
//!
//! A result over its hard limit returns a compact preview or a real first page and an
//! actionable continuation. It is never replaced by a notice containing none of the
//! requested knowledge: that failure mode forced agents to bypass the supported tools.

use akr_core::json::Value;

/// What a tool's output should cost, and what it must not exceed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    /// What the result should come in under.
    pub target_tokens: usize,
    /// The point past which the result is replaced by a summary and a cursor.
    pub hard_tokens: usize,
}

impl Budget {
    const fn new(target_tokens: usize, hard_tokens: usize) -> Self {
        Self {
            target_tokens,
            hard_tokens,
        }
    }
}

/// The budget for a tool.
///
/// These are engineering targets rather than protocol requirements, and they are the
/// table from `sources/context-reduction.md` with one change: validation *failure* gets
/// more room than validation success, because a diagnostic nobody can read costs a whole
/// extra round trip.
#[must_use]
pub fn budget_for(tool: &str) -> Budget {
    match tool {
        "knowledge.start" => Budget::new(1_500, 2_000),
        "knowledge.search" | "knowledge.source_search" => Budget::new(600, 1_000),
        "knowledge.get" => Budget::new(1_000, 1_500),
        "knowledge.source_get" => Budget::new(1_500, 2_500),
        "knowledge.context" => Budget::new(3_000, 4_000),
        "knowledge.impact" => Budget::new(800, 1_500),
        "knowledge.explain" => Budget::new(800, 1_500),
        "knowledge.validate" => Budget::new(500, 3_000),
        // An advisor packet is the one deliberately large read on this surface: the point
        // of it is that the second model arrives knowing the workspace. It is also
        // addressable, so the overflow path has somewhere real to send the caller -- one
        // section at a time, rather than a truncated everything.
        "knowledge.handoff_open" => Budget::new(2_000, 3_500),
        "knowledge.handoff_expand" | "knowledge.handoff_reveal" => Budget::new(1_000, 2_000),
        // A result is what a parent reads instead of a transcript, and the coverage
        // roll-up is what it delegates the next wave from; both are read repeatedly by
        // one agent, so they get room without getting the packet's.
        "knowledge.handoff_results" | "knowledge.handoff_coverage" => Budget::new(1_200, 2_500),
        "knowledge.handoff_capsule" | "knowledge.handoff_session_show" => Budget::new(1_000, 2_000),
        "knowledge.handoff_list" | "knowledge.handoff_verify" => Budget::new(600, 1_000),
        // Every write tool. A successful write has nothing to say but where it landed.
        _ => Budget::new(300, 500),
    }
}

/// The crude token estimate the context budget uses: words plus punctuation.
///
/// The same estimator on both sides on purpose. Two estimators would disagree, and the
/// one that mattered would be whichever was wrong.
#[must_use]
pub fn estimate_tokens(text: &str) -> usize {
    text.split_whitespace()
        .map(|word| 1 + word.chars().filter(|c| c.is_ascii_punctuation()).count() / 3)
        .sum()
}

/// What enforcement did to a result.
#[derive(Debug, Clone, PartialEq)]
pub struct Enforced {
    /// The text half, possibly replaced by a summary.
    pub text: Option<String>,
    /// The structured half, possibly replaced by a summary.
    pub structured: Value,
    /// Whether anything was withheld.
    pub truncated: bool,
    /// The estimate before enforcement.
    pub estimated_tokens: usize,
}

/// Applies `tool`'s budget to a result.
///
/// Under the hard limit, nothing happens — most calls are small and paying a rewrite for
/// them would be silly. Over it, both halves are replaced by a compact summary naming the
/// tool, the size that was refused and how to ask for less.
#[must_use]
pub fn enforce(
    tool: &str,
    text: Option<&str>,
    structured: &Value,
    internally_budgeted: Option<usize>,
    arguments: &Value,
) -> Enforced {
    let budget = budget_for(tool);
    // Start and context already assembled to the caller's explicit budget. Applying a
    // second ceiling afterwards both wastes that work and makes the input contract
    // untrue, especially because MCP duplicates some information across text and
    // structured halves.
    let hard_tokens = internally_budgeted.map_or(budget.hard_tokens, |_| usize::MAX);
    let rendered = structured.to_pretty();
    // Both halves, because both reach the model: a client that shows `content` and a
    // client that parses `structuredContent` are each paying for their own half.
    let estimated = estimate_tokens(text.unwrap_or_default()) + estimate_tokens(&rendered);

    if estimated <= hard_tokens {
        return Enforced {
            text: text.map(ToOwned::to_owned),
            structured: structured.clone(),
            truncated: false,
            estimated_tokens: estimated,
        };
    }

    let (summary, structured) = if matches!(tool, "knowledge.search" | "knowledge.source_search") {
        search_page(tool, structured, arguments, hard_tokens, estimated)
    } else {
        compact_preview(tool, structured, arguments, hard_tokens, estimated)
    };

    Enforced {
        text: Some(summary),
        structured,
        truncated: true,
        estimated_tokens: estimated,
    }
}

/// How to ask this tool for less. Concrete arguments, not "try narrowing your query".
fn narrowing_advice(tool: &str, arguments: &Value) -> String {
    match tool {
        "knowledge.search" | "knowledge.source_search" => {
            "Call again with a smaller `limit`, or narrow with `kinds`, `states` or \
             `documents`."
        }
        "knowledge.get" => match arguments.get("detail").and_then(Value::as_str) {
            Some("canonical") => {
                "Call again with `detail: \"body\"`, which still includes acceptance \
                 checks and their statements. `detail: \"summary\"` drops them."
            }
            _ => "Call again with `detail: \"summary\"`, or with `relations: false`.",
        },
        "knowledge.source_get" => {
            "Call again with `detail: \"snippet\"`, or name a `chunk` instead of a \
             document `id`."
        }
        "knowledge.context" => {
            "Call again with a smaller `budget_tokens`, or with `paths` narrowed to the \
             files you are about to touch."
        }
        "knowledge.impact" => "Call again with a smaller `depth`.",
        "knowledge.handoff_open" => {
            "Read one section at a time with `knowledge.handoff_expand`: task, workspace, \
             project, session, scope, assignment, inherited."
        }
        "knowledge.validate" => {
            "Call again with a smaller `limit`, or continue from `next_offset`."
        }
        _ => "Call again with narrower arguments.",
    }
    .to_owned()
}

/// A ready-made retry, so the next call is a copy rather than a guess.
fn narrowing_arguments(tool: &str, original: &Value) -> Value {
    let changes = match tool {
        "knowledge.get" => match original.get("detail").and_then(Value::as_str) {
            Some("canonical") => vec![("detail", Value::string("body"))],
            _ => vec![
                ("detail", Value::string("summary")),
                ("relations", Value::bool(false)),
            ],
        },
        "knowledge.source_get" => vec![("detail", Value::string("snippet"))],
        "knowledge.search" | "knowledge.source_search" => vec![("limit", Value::integer(5))],
        "knowledge.start" => vec![("budget_tokens", Value::integer(1_500))],
        "knowledge.context" => vec![("budget_tokens", Value::integer(2_000))],
        "knowledge.impact" => vec![("depth", Value::integer(1))],
        "knowledge.handoff_open" => {
            // The continuation is a different tool, so the shared "same tool, narrower
            // arguments" shape does not fit: name the tool that does answer.
            return Value::object(vec![
                ("tool", Value::string("knowledge.handoff_expand")),
                (
                    "arguments",
                    Value::object(vec![
                        (
                            "packet",
                            original.get("packet").cloned().unwrap_or(Value::Null),
                        ),
                        ("section", Value::string("task")),
                    ]),
                ),
            ]);
        }
        "knowledge.validate" => vec![("limit", Value::integer(3)), ("offset", Value::integer(0))],
        _ => return Value::Null,
    };
    let mut arguments = match original {
        Value::Object(fields) => fields.clone(),
        _ => Vec::new(),
    };
    for (name, value) in changes {
        arguments.retain(|(existing, _)| existing != name);
        arguments.push((name.to_owned(), value));
    }
    Value::object(vec![
        ("tool", Value::string(tool.to_owned())),
        ("arguments", Value::Object(arguments)),
    ])
}

fn compact_preview(
    tool: &str,
    structured: &Value,
    arguments: &Value,
    hard_tokens: usize,
    estimated: usize,
) -> (String, Value) {
    let advice = narrowing_advice(tool, arguments);
    let continuation = narrowing_arguments(tool, arguments);
    let mut preview = preview_value(structured, 0);
    let counts = counts_of(structured);
    append_fields(
        &mut preview,
        vec![
            ("truncated", Value::bool(true)),
            ("tool", Value::string(tool.to_owned())),
            ("estimated_tokens", usize_value(estimated)),
            ("hard_limit_tokens", usize_value(hard_tokens)),
            ("continuation", continuation.clone()),
            ("help", Value::string(advice.clone())),
            ("counts", counts),
        ],
    );
    let text = format!(
        "{tool} returned a compact preview of an approximately {estimated}-token result.\n\
         {advice}\nContinuation:\n{}\n\n{}",
        continuation.to_pretty(),
        preview.to_pretty()
    );
    (text, preview)
}

fn search_page(
    tool: &str,
    structured: &Value,
    arguments: &Value,
    hard_tokens: usize,
    estimated: usize,
) -> (String, Value) {
    let results = structured
        .get("results")
        .and_then(Value::as_array)
        .unwrap_or(&[]);
    if results.is_empty() {
        return compact_preview(tool, structured, arguments, hard_tokens, estimated);
    }
    let base_offset = arguments
        .get("offset")
        .and_then(Value::as_integer)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(0);
    let mut keep = results.len();
    let mut candidate = structured.clone();
    let mut candidate_text = String::new();

    while keep > 0 {
        let next = base_offset.saturating_add(keep);
        let continuation = search_continuation(tool, arguments, next);
        candidate = structured.clone();
        replace_search_page(
            &mut candidate,
            &results[..keep],
            next,
            estimated,
            hard_tokens,
            &continuation,
        );
        candidate_text = render_search_preview(tool, &results[..keep], base_offset, &continuation);
        let size = estimate_tokens(&candidate_text) + estimate_tokens(&candidate.to_pretty());
        if size <= hard_tokens || keep == 1 {
            break;
        }
        keep -= 1;
    }
    (candidate_text, candidate)
}

fn search_continuation(tool: &str, arguments: &Value, offset: usize) -> Value {
    let mut continuation = narrowing_arguments(tool, arguments);
    if let Value::Object(fields) = &mut continuation
        && let Some((_, Value::Object(args))) =
            fields.iter_mut().find(|(name, _)| name == "arguments")
    {
        args.retain(|(name, _)| name != "offset");
        args.push(("offset".to_owned(), usize_value(offset)));
    }
    continuation
}

fn replace_search_page(
    value: &mut Value,
    results: &[Value],
    next_offset: usize,
    estimated: usize,
    hard_tokens: usize,
    continuation: &Value,
) {
    let counts = counts_of(value);
    let Value::Object(fields) = value else { return };
    fields.retain(|(name, _)| {
        !matches!(
            name.as_str(),
            "results"
                | "count"
                | "truncated"
                | "has_more"
                | "next_offset"
                | "estimated_tokens"
                | "hard_limit_tokens"
                | "continuation"
        )
    });
    fields.extend([
        ("results".to_owned(), Value::array(results.to_vec())),
        ("count".to_owned(), usize_value(results.len())),
        ("truncated".to_owned(), Value::bool(true)),
        ("has_more".to_owned(), Value::bool(true)),
        ("next_offset".to_owned(), usize_value(next_offset)),
        ("estimated_tokens".to_owned(), usize_value(estimated)),
        ("hard_limit_tokens".to_owned(), usize_value(hard_tokens)),
        ("continuation".to_owned(), continuation.clone()),
        ("counts".to_owned(), counts),
    ]);
}

fn render_search_preview(
    tool: &str,
    results: &[Value],
    offset: usize,
    continuation: &Value,
) -> String {
    let mut text = format!("{tool} results from offset {offset}:\n");
    for result in results {
        if tool == "knowledge.search" {
            let key = result
                .get("key")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let rev = result.get("rev").and_then(Value::as_integer).unwrap_or(0);
            let title = result
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default();
            text.push_str(&format!("- @{key}/{rev} — {title}\n"));
        } else {
            let document = result
                .get("document")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let chunk = result
                .get("chunk")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let heading = result
                .get("heading")
                .and_then(Value::as_str)
                .unwrap_or_default();
            text.push_str(&format!(
                "- source:{document} chunk {chunk} — {heading} [NON-AUTHORITATIVE]\n"
            ));
        }
    }
    text.push_str(&format!("Continue with:\n{}", continuation.to_pretty()));
    text
}

fn preview_value(value: &Value, depth: usize) -> Value {
    match value {
        Value::String(text) if text.chars().count() > 240 => {
            let mut preview: String = text.chars().take(240).collect();
            preview.push('…');
            Value::string(preview)
        }
        Value::Array(items) => Value::array(
            items
                .iter()
                .take(if depth < 2 { 2 } else { 1 })
                .map(|item| preview_value(item, depth + 1))
                .collect(),
        ),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(name, item)| (name.clone(), preview_value(item, depth + 1)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn append_fields(value: &mut Value, additions: Vec<(&str, Value)>) {
    let Value::Object(fields) = value else { return };
    for (name, item) in additions {
        fields.retain(|(existing, _)| existing != name);
        fields.push((name.to_owned(), item));
    }
}

fn usize_value(value: usize) -> Value {
    Value::integer(i64::try_from(value).unwrap_or(i64::MAX))
}

/// The scalar and array-length fields of a payload, so a withheld result still counts.
fn counts_of(structured: &Value) -> Value {
    let Value::Object(fields) = structured else {
        return Value::Object(Vec::new());
    };
    let mut out = Vec::new();
    for (name, value) in fields {
        match value {
            Value::Array(items) => out.push((
                format!("{name}_count"),
                Value::integer(i64::try_from(items.len()).unwrap_or(i64::MAX)),
            )),
            Value::Integer(number) => out.push((name.clone(), Value::Integer(*number))),
            Value::Bool(flag) => out.push((name.clone(), Value::Bool(*flag))),
            _ => {}
        }
    }
    Value::Object(out)
}

// ---------------------------------------------------------------------------------------
// accounting
// ---------------------------------------------------------------------------------------

/// One line of the accounting log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    /// The tool name.
    pub tool: String,
    /// Bytes of arguments.
    pub input_bytes: usize,
    /// Bytes of the text half.
    pub text_output_bytes: usize,
    /// Bytes of the structured half.
    pub structured_output_bytes: usize,
    /// The estimate over both halves.
    pub estimated_output_tokens: usize,
    /// Whether the budget withheld the result.
    pub truncated: bool,
    /// Wall clock, milliseconds.
    pub duration_ms: u64,
}

impl Call {
    /// The JSONL line this call writes: one compact JSON document, no newlines.
    ///
    /// Written by hand rather than through the pretty printer, because JSONL means one
    /// document per line and the pretty printer's whole job is putting newlines in.
    #[must_use]
    pub fn to_line(&self) -> String {
        let tool = self.tool.replace('\\', "\\\\").replace('"', "\\\"");
        format!(
            "{{\"tool\":\"{tool}\",\"input_bytes\":{},\"text_output_bytes\":{},\
             \"structured_output_bytes\":{},\"estimated_output_tokens\":{},\
             \"truncated\":{},\"duration_ms\":{}}}",
            self.input_bytes,
            self.text_output_bytes,
            self.structured_output_bytes,
            self.estimated_output_tokens,
            self.truncated,
            self.duration_ms,
        )
    }

    /// Parses a line back, for `akr mcp stats`. Unreadable lines are skipped by the caller.
    #[must_use]
    pub fn from_line(line: &str) -> Option<Self> {
        let value = akr_core::json::parse(line).ok()?;
        let integer = |name: &str| -> usize {
            value
                .get(name)
                .and_then(Value::as_integer)
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(0)
        };
        Some(Self {
            tool: value.get("tool")?.as_str()?.to_owned(),
            input_bytes: integer("input_bytes"),
            text_output_bytes: integer("text_output_bytes"),
            structured_output_bytes: integer("structured_output_bytes"),
            estimated_output_tokens: integer("estimated_output_tokens"),
            truncated: value
                .get("truncated")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            duration_ms: u64::try_from(integer("duration_ms")).unwrap_or(0),
        })
    }
}

/// Appends a call to the accounting log, if one is configured.
///
/// Best-effort by design: accounting that could fail a tool call would be a way for
/// instrumentation to break the thing it is measuring. A full disk loses a line.
pub fn record(path: Option<&std::path::Path>, call: &Call) {
    let Some(path) = path else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{}", call.to_line());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn big(words: usize) -> Value {
        Value::object(vec![(
            "results",
            Value::array(
                (0..words)
                    .map(|n| Value::string(format!("result number {n} with some words in it")))
                    .collect(),
            ),
        )])
    }

    #[test]
    fn a_small_result_passes_through_untouched() {
        let structured = Value::object(vec![("ok", Value::bool(true))]);
        let enforced = enforce(
            "knowledge.validate",
            Some("ok\n"),
            &structured,
            None,
            &Value::Object(Vec::new()),
        );
        assert!(!enforced.truncated);
        assert_eq!(enforced.text.as_deref(), Some("ok\n"));
        assert_eq!(enforced.structured, structured);
    }

    #[test]
    fn an_oversized_search_returns_a_useful_page() {
        let structured = big(400);
        let arguments = Value::object(vec![("query", Value::string("example"))]);
        let enforced = enforce("knowledge.search", None, &structured, None, &arguments);
        assert!(enforced.truncated);
        assert_ne!(enforced.structured, structured);
        assert_eq!(
            enforced.structured.get("truncated"),
            Some(&Value::Bool(true))
        );
        assert!(
            enforced
                .structured
                .get("results")
                .and_then(Value::as_array)
                .is_some_and(|results| !results.is_empty()),
            "truncation must never replace all useful content with a notice"
        );
        let continuation = enforced
            .structured
            .get("continuation")
            .expect("an exact next page");
        assert_eq!(
            continuation.get("arguments").and_then(|a| a.get("query")),
            Some(&Value::String("example".to_owned()))
        );
        assert!(
            continuation
                .get("arguments")
                .and_then(|a| a.get("offset"))
                .and_then(Value::as_integer)
                .is_some_and(|offset| offset > 0),
            "the continuation must advance instead of retrying page one"
        );
    }

    #[test]
    fn a_compact_preview_preserves_required_retry_arguments() {
        let enforced = enforce(
            "knowledge.get",
            None,
            &big(400),
            None,
            &Value::object(vec![("ref", Value::string("@sys.term.example/1"))]),
        );
        let continuation = enforced
            .structured
            .get("continuation")
            .expect("a ready-made retry");
        assert_eq!(
            continuation.get("arguments").and_then(|a| a.get("detail")),
            Some(&Value::String("summary".to_owned()))
        );
        assert_eq!(
            continuation
                .get("arguments")
                .and_then(|a| a.get("relations")),
            Some(&Value::Bool(false))
        );
        assert_eq!(
            continuation.get("arguments").and_then(|a| a.get("ref")),
            Some(&Value::String("@sys.term.example/1".to_owned()))
        );
        assert!(
            enforced.text.expect("a summary").contains("detail"),
            "the text half has to carry the advice too"
        );
    }

    #[test]
    fn an_oversized_canonical_get_continues_at_body_not_summary() {
        // `detail: summary` drops the acceptance block, which is the reason an agent
        // asked for canonical in the first place — to reproduce checks verbatim.
        let enforced = enforce(
            "knowledge.get",
            None,
            &big(400),
            None,
            &Value::object(vec![
                ("ref", Value::string("@sys.work.example/1")),
                ("detail", Value::string("canonical")),
            ]),
        );
        let continuation = enforced
            .structured
            .get("continuation")
            .expect("a ready-made retry");
        assert_eq!(
            continuation.get("arguments").and_then(|a| a.get("detail")),
            Some(&Value::String("body".to_owned()))
        );
        let text = enforced.text.expect("advice");
        assert!(text.contains("body"), "{text}");
        assert!(
            text.contains("acceptance"),
            "the advice has to say what summary would lose: {text}"
        );
    }

    #[test]
    fn a_paged_result_still_carries_its_original_counts() {
        let enforced = enforce(
            "knowledge.search",
            None,
            &big(400),
            None,
            &Value::object(vec![("query", Value::string("example"))]),
        );
        assert_eq!(
            enforced
                .structured
                .get("counts")
                .and_then(|c| c.get("results_count")),
            Some(&Value::Integer(400)),
            "a caller must be able to tell 'nothing matched' from 'too much matched'"
        );
    }

    #[test]
    fn an_oversized_source_search_preserves_filters_and_advances() {
        let arguments = Value::object(vec![
            ("query", Value::string("decoder advice")),
            ("mode", Value::string("literal")),
            ("documents", Value::array(vec![Value::string("audit-2026")])),
        ]);
        let enforced = enforce("knowledge.source_search", None, &big(400), None, &arguments);
        let retry = enforced
            .structured
            .get("continuation")
            .and_then(|continuation| continuation.get("arguments"))
            .expect("source search has an exact next page");
        assert_eq!(retry.get("query"), arguments.get("query"));
        assert_eq!(retry.get("mode"), arguments.get("mode"));
        assert_eq!(retry.get("documents"), arguments.get("documents"));
        assert!(
            retry
                .get("offset")
                .and_then(Value::as_integer)
                .is_some_and(|offset| offset > 0)
        );
    }

    #[test]
    fn both_halves_count_towards_the_budget() {
        // The duplication the design set warns about: a client that shows text and parses
        // structure pays for both, so the guard has to measure both.
        let structured = big(200);
        let text = structured.to_pretty();
        let arguments = Value::object(vec![("query", Value::string("example"))]);
        let with_text = enforce(
            "knowledge.search",
            Some(&text),
            &structured,
            None,
            &arguments,
        );
        let without = enforce("knowledge.search", None, &structured, None, &arguments);
        assert!(with_text.estimated_tokens > without.estimated_tokens);
    }

    #[test]
    fn writes_get_the_smallest_budget() {
        assert!(
            budget_for("knowledge.propose").hard_tokens
                < budget_for("knowledge.context").hard_tokens
        );
    }

    #[test]
    fn explicit_assembly_budgets_replace_default_transport_ceilings() {
        let structured = big(1_000);
        let estimated = estimate_tokens(&structured.to_pretty());
        assert!(estimated > budget_for("knowledge.context").hard_tokens);
        for tool in ["knowledge.start", "knowledge.context"] {
            let enforced = enforce(
                tool,
                None,
                &structured,
                Some(1),
                &Value::object(vec![("budget_tokens", Value::integer(1))]),
            );
            assert!(!enforced.truncated, "{tool}");
            assert_eq!(enforced.structured, structured, "{tool}");
        }
    }

    #[test]
    fn a_call_line_is_one_json_document() {
        let line = Call {
            tool: "knowledge.get".into(),
            input_bytes: 42,
            text_output_bytes: 100,
            structured_output_bytes: 200,
            estimated_output_tokens: 80,
            truncated: false,
            duration_ms: 7,
        }
        .to_line();
        assert!(!line.contains('\n'));
        assert!(line.contains("\"tool\":\"knowledge.get\""), "{line}");
    }
}

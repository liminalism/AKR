//! Every tool of [`crate::schema::TOOLS`], each over the function the command line calls.
//!
//! The catalogue is named in exactly one place — the `TOOLS` table — and this module
//! dispatches over it. Prose that counts the tools drifts the moment one is added, which
//! is how the help text came to advertise nine of them while `tools/list` returned eleven.
//!
//! Every tool here does three things and no more: translate its JSON arguments into the
//! request type `akr-cli` already takes, call the same function `akr <command>` calls, and
//! shape the result as `docs/08-mcp.md` §3 or §4 declares. The ledger logic is entirely in
//! `akr-core`; the command logic is entirely in `akr-cli`.
//!
//! # Read and write are separated here, structurally
//!
//! §7: "Read tools never touch `.akr/records/`." [`is_write`] is the one place that
//! distinction is recorded, and [`call`] routes on it — a read tool cannot reach the write
//! module because the match arms do not lead there.

use akr_cli::args::{Command, Format, Global, Profile};
use akr_cli::commands::{self, Output};
use akr_cli::session::{EnvError, Exit, Session};
use akr_core::json::Value;
use akr_core::model::{Glob, Kind, LogicalKey, ScopeTerm, scopes_overlap};
use std::path::{Path, PathBuf};

use crate::errors::{Class, ToolError, class_of, first_error_code};
use crate::record;

/// Whether a tool writes to `.akr/records/` (§2).
#[must_use]
pub fn is_write(name: &str) -> bool {
    crate::schema::TOOLS
        .iter()
        .find(|tool| tool.name == name)
        .is_some_and(|tool| tool.writes)
}

fn missing_commit(session: &Session, suggest_observed_at: bool) -> ToolError {
    // The flag decides whether `observed_at` is worth suggesting: both branches used to
    // say it does, so a caller with no such argument was told to pass one.
    let hint = if suggest_observed_at {
        "; make an initial commit, or pass `observed_at`"
    } else {
        "; make an initial commit"
    };
    if session.repository.is_some() {
        ToolError::new(
            "AKR-G001",
            format!("no commit to record: the repository has no commits yet{hint}"),
        )
    } else {
        ToolError::new(
            "AKR-G001",
            format!("no commit to record: not inside a git repository{hint}"),
        )
    }
}

/// The payload that returns to the MCP protocol.
#[must_use]
pub enum ToolResult {
    /// Human-readable + structured text output.
    Read {
        /// Exact CLI output.
        text: String,
        /// Structured `result` payload.
        structured: Value,
    },
    /// Structured-only output.
    Structured(Value),
}

impl ToolResult {
    /// Returns a reference to the structured payload for MCP transport.
    #[must_use]
    pub fn structured(&self) -> &Value {
        match self {
            Self::Read { structured, .. } => structured,
            Self::Structured(structured) => structured,
        }
    }

    /// Returns the human-readable text, when this result came from a read tool.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Read { text, .. } => Some(text.as_str()),
            Self::Structured(_) => None,
        }
    }
}

/// Runs a tool.
///
/// # Errors
/// [`ToolError`] carrying §5's class, summary and diagnostic array.
pub fn call(root: &Path, name: &str, arguments: &Value) -> Result<ToolResult, ToolError> {
    match name {
        "knowledge.search" => search(root, arguments),
        "knowledge.start" => start(root, arguments),
        "knowledge.explain" => explain(arguments),
        "knowledge.get" => get(root, arguments),
        "knowledge.context" => context(root, arguments),
        "knowledge.source_list" => source_list(root, arguments),
        "knowledge.source_add" => source_add(root, arguments),
        "knowledge.source_search" => source_search(root, arguments),
        "knowledge.source_get" => source_get(root, arguments),
        "knowledge.source_verify" => source_verify(root, arguments),
        "knowledge.source_supersede" => source_supersede(root, arguments),
        "knowledge.source_status" => source_status(root, arguments),
        "knowledge.source_dependents" => source_dependents(root, arguments),
        "knowledge.source_finalize" => source_finalize(root, arguments),
        "knowledge.impact" => impact(root, arguments),
        "knowledge.validate" => validate(root, arguments),
        "knowledge.propose" => propose(root, arguments),
        "knowledge.revise" => revise(root, arguments),
        "knowledge.supersede" => supersede(root, arguments),
        "knowledge.complete" => complete(root, arguments),
        "knowledge.evidence_add" => evidence_add(root, arguments),
        "knowledge.evidence_add_many" => evidence_add_many(root, arguments),
        "knowledge.papercut" => papercut(root, arguments),
        other => Err(ToolError::new(
            "AKR-X041",
            format!("unknown tool {other:?}; the catalogue is closed for 0.1"),
        )),
    }
}

// -------------------------------------------------------------------------------------
// read tools
// -------------------------------------------------------------------------------------

fn source_list(root: &Path, arguments: &Value) -> Result<ToolResult, ToolError> {
    run_read(
        root,
        Command::SourceList {
            all: flag(arguments, "all_versions"),
        },
    )
}

fn source_add(root: &Path, arguments: &Value) -> Result<ToolResult, ToolError> {
    run_write(
        root,
        Command::SourceAdd {
            path: PathBuf::from(required_str(arguments, "path")?),
            id: optional_str(arguments, "id"),
            title: optional_str(arguments, "title"),
            origin: optional_str(arguments, "origin"),
            observed_at: optional_str(arguments, "observed_at"),
            scope: optional_str(arguments, "scope"),
        },
    )
}

fn search(root: &Path, arguments: &Value) -> Result<ToolResult, ToolError> {
    let query = required_str(arguments, "query")?;
    let offset = arguments
        .get("offset")
        .and_then(Value::as_integer)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(0)
        .min(100);
    let page_size = arguments
        .get("limit")
        .and_then(Value::as_integer)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(20)
        .clamp(1, 100);
    // Fetch one extra ranked row so the MCP surface can publish an honest continuation.
    // The search store deliberately exposes at most its documented top 100 results.
    let fetch_limit = offset.saturating_add(page_size).saturating_add(1).min(100);
    let command = Command::Search {
        query: query.to_owned(),
        // Never raw FTS5 over MCP. An agent writing a query has no way to know that a
        // comma is an operator, and the failure mode was measured: it gave up and grepped
        // `.akr/records`.
        raw_fts: false,
        kinds: string_list(arguments, "kinds"),
        states: string_list(arguments, "states"),
        limit: Some(fetch_limit),
    };
    let mut session = open(root, false)?;
    let mut output = commands::run(&mut session, &command).map_err(environment)?;
    paginate_search(&mut output, offset, page_size);
    let text = output.text.clone();
    let mut structured = finish(&session.sources, output)?;
    let candidates = search_planning_candidates(&session, &structured, &[]);
    let recommended_context = search_recommended_context(&structured, &[], None);
    if let Value::Object(fields) = &mut structured {
        if let Some(recommended_context) = recommended_context {
            fields.push(("recommended_context".to_owned(), recommended_context));
        } else if !candidates.is_empty() {
            fields.push(("planning_candidates".to_owned(), Value::array(candidates)));
        }
    }
    Ok(ToolResult::Read { text, structured })
}

/// Slices the deterministic ranked result set and publishes an exact next-page offset.
fn paginate_search(output: &mut Output, offset: usize, page_size: usize) {
    let all = output
        .result
        .get("results")
        .and_then(Value::as_array)
        .unwrap_or(&[])
        .to_vec();
    let page: Vec<Value> = all.iter().skip(offset).take(page_size).cloned().collect();
    let has_more = offset.saturating_add(page.len()) < all.len();
    let next_offset = has_more.then(|| offset.saturating_add(page.len()));

    if let Value::Object(fields) = &mut output.result {
        for (name, value) in fields.iter_mut() {
            match name.as_str() {
                "results" => *value = Value::array(page.clone()),
                "count" => {
                    *value = Value::integer(i64::try_from(page.len()).unwrap_or(i64::MAX));
                }
                _ => {}
            }
        }
        fields.retain(|(name, _)| !matches!(name.as_str(), "has_more" | "next_offset"));
        fields.push(("has_more".to_owned(), Value::bool(has_more)));
        fields.push((
            "next_offset".to_owned(),
            next_offset.map_or(Value::Null, |next| {
                Value::integer(i64::try_from(next).unwrap_or(i64::MAX))
            }),
        ));
    }
    output.text = render_search_page(&page, offset, next_offset);
}

fn render_search_page(results: &[Value], offset: usize, next_offset: Option<usize>) -> String {
    let mut text = String::new();
    for result in results {
        let key = result
            .get("key")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let rev = result.get("rev").and_then(Value::as_integer).unwrap_or(0);
        let kind = result
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let state = result
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let title = result
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let score = result
            .get("score")
            .and_then(Value::as_str)
            .unwrap_or_default();
        text.push_str(&format!(
            "  {score}  {key}/{rev}  {kind} {state}  {title}\n"
        ));
    }
    text.push_str(&format!(
        "{} result{} from offset {offset}\n",
        results.len(),
        if results.len() == 1 { "" } else { "s" }
    ));
    if let Some(next) = next_offset {
        text.push_str(&format!("more results: continue at offset {next}\n"));
    }
    text
}

fn start(root: &Path, arguments: &Value) -> Result<ToolResult, ToolError> {
    let task = required_str(arguments, "task")?;
    let paths: Vec<Glob> = arguments
        .get("paths")
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_str)
        .map(Glob::new)
        .collect();
    let budget = arguments
        .get("budget_tokens")
        .and_then(Value::as_integer)
        .and_then(|value| usize::try_from(value).ok());
    run_read(
        root,
        Command::Start {
            task: task.to_owned(),
            paths,
            budget,
        },
    )
}

fn explain(arguments: &Value) -> Result<ToolResult, ToolError> {
    let subject = required_str(arguments, "subject")?;
    let command = Command::Explain {
        subject: subject.to_owned(),
    };
    run_vocabulary(command)
}

/// A JSON array of strings, or nothing.
fn string_list(arguments: &Value, field: &str) -> Vec<String> {
    arguments
        .get(field)
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect()
}

fn search_planning_candidates(session: &Session, result: &Value, paths: &[Glob]) -> Vec<Value> {
    let model = session.resolve();
    let request_scope: Vec<ScopeTerm> = paths.iter().cloned().map(ScopeTerm::Path).collect();
    let mut out = Vec::new();
    for hit in result
        .get("results")
        .and_then(Value::as_array)
        .unwrap_or(&[])
    {
        let kind = hit.get("kind").and_then(Value::as_str).unwrap_or_default();
        if !matches!(kind, "milestone" | "work" | "track") {
            continue;
        }
        let key = hit.get("key").and_then(Value::as_str).unwrap_or_default();
        let title = hit.get("title").and_then(Value::as_str).unwrap_or_default();
        let state = hit.get("state").and_then(Value::as_str).unwrap_or_default();
        let rev = hit.get("rev").and_then(Value::as_integer);
        let reference = match (rev, rev.is_some_and(|rev| rev >= 0)) {
            (Some(rev), true) => format!("@{key}/{rev}"),
            _ => format!("@{key}"),
        };
        let path_overlap = if request_scope.is_empty() {
            false
        } else if let Ok(reference) = akr_core::model::Reference::parse(&reference) {
            session
                .ledger
                .resolve(&reference)
                .ok()
                .flatten()
                .is_some_and(|record| {
                    scopes_overlap(
                        &record.scope,
                        &request_scope,
                        &model.ledger().part_of_index(),
                    )
                })
        } else {
            false
        };
        out.push(Value::object(vec![
            ("reference", Value::string(reference)),
            ("title", Value::string(title.to_owned())),
            ("kind", Value::string(kind.to_owned())),
            ("state", Value::string(state.to_owned())),
            ("path_overlap", Value::bool(path_overlap)),
        ]));
    }
    out
}

fn get(root: &Path, arguments: &Value) -> Result<ToolResult, ToolError> {
    let reference = required_str(arguments, "ref")?;
    let detail = match arguments.get("detail").and_then(Value::as_str) {
        None => akr_cli::args::Detail::default(),
        Some(name) => akr_cli::args::Detail::from_name(name).ok_or_else(|| {
            ToolError::new(
                "AKR-C004",
                format!("`detail`: {name:?} is not `summary`, `body` or `canonical`"),
            )
        })?,
    };
    let command = Command::Get {
        reference: reference.to_owned(),
        history: flag(arguments, "history"),
        // §3's sample shows relations, so they are on by default here where the CLI
        // makes them opt-in: an agent pays for a second call, a human pays for a
        // wider terminal.
        relations: arguments
            .get("relations")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        detail,
    };
    run_read(root, command)
}

fn context(root: &Path, arguments: &Value) -> Result<ToolResult, ToolError> {
    let goal = required_str(arguments, "goal")?;
    let paths: Vec<Glob> = arguments
        .get("paths")
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_str)
        .map(Glob::new)
        .collect();
    let budget = arguments
        .get("budget_tokens")
        .and_then(Value::as_integer)
        .and_then(|n| usize::try_from(n).ok());
    // The same `Command::Context` the binary builds from `--goal`, `--paths` and
    // `--budget`, so §1's invariant holds by construction rather than by agreement.
    let command = Command::Context {
        goal: goal.to_owned(),
        paths,
        budget,
    };
    let mut session = open(root, false)?;
    let output = match commands::run(&mut session, &command) {
        Ok(output) => output,
        Err(error) if error.code == "AKR-X001" && session.ledger.records().is_empty() => {
            return Err(environment(error));
        }
        Err(error) if error.code == "AKR-X001" => {
            return Err(unresolved_goal(error, &session, goal, &command));
        }
        Err(error) => return Err(environment(error)),
    };
    let text = output.text.clone();
    let structured = finish(&session.sources, output)?;
    Ok(ToolResult::Read { text, structured })
}

/// `knowledge.source_search` — the source library, over the same command `akr source
/// search` runs.
///
/// The tool takes an enum string rather than the CLI's two flags, for the reason
/// `docs/08-mcp.md` §2 gives about one-character interfaces: an agent reads
/// `"literal"` and a flag pair is a thing it has to remember the exclusivity rule for.
fn source_search(root: &Path, arguments: &Value) -> Result<ToolResult, ToolError> {
    let query = required_str(arguments, "query")?;
    let mode = arguments
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("text");
    if !matches!(mode, "text" | "literal" | "fts") {
        return Err(ToolError::new(
            "AKR-C004",
            format!("`mode`: {mode:?} is not `text`, `literal` or `fts`"),
        ));
    }
    let offset = arguments
        .get("offset")
        .and_then(Value::as_integer)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(0)
        .min(100);
    let page_size = arguments
        .get("limit")
        .and_then(Value::as_integer)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(10)
        .clamp(1, 100);
    let fetch_limit = offset.saturating_add(page_size).saturating_add(1).min(100);
    let command = Command::SourceSearch {
        query: query.to_owned(),
        mode: mode.to_owned(),
        documents: string_list(arguments, "documents"),
        all_versions: flag(arguments, "all_versions"),
        limit: Some(fetch_limit),
    };
    let mut session = open(root, false)?;
    let mut output = commands::run(&mut session, &command).map_err(environment)?;
    paginate_source_search(&mut output, offset, page_size);
    let text = output.text.clone();
    let structured = finish(&session.sources, output)?;
    Ok(ToolResult::Read { text, structured })
}

fn paginate_source_search(output: &mut Output, offset: usize, page_size: usize) {
    let all = output
        .result
        .get("results")
        .and_then(Value::as_array)
        .unwrap_or(&[])
        .to_vec();
    let page: Vec<Value> = all.iter().skip(offset).take(page_size).cloned().collect();
    let has_more = offset.saturating_add(page.len()) < all.len();
    let next_offset = has_more.then(|| offset.saturating_add(page.len()));
    if let Value::Object(fields) = &mut output.result {
        for (name, value) in fields.iter_mut() {
            match name.as_str() {
                "results" => *value = Value::array(page.clone()),
                "count" => {
                    *value = Value::integer(i64::try_from(page.len()).unwrap_or(i64::MAX));
                }
                _ => {}
            }
        }
        fields.retain(|(name, _)| !matches!(name.as_str(), "has_more" | "next_offset"));
        fields.push(("has_more".to_owned(), Value::bool(has_more)));
        fields.push((
            "next_offset".to_owned(),
            next_offset.map_or(Value::Null, |next| {
                Value::integer(i64::try_from(next).unwrap_or(i64::MAX))
            }),
        ));
    }
    output.text = format!(
        "NON-AUTHORITATIVE source results from offset {offset}\n{}",
        Value::array(page).to_pretty()
    );
    if let Some(next) = next_offset {
        output
            .text
            .push_str(&format!("\nmore results: continue at offset {next}\n"));
    }
}

/// `knowledge.source_get` — one passage of one registered source.
///
/// `detail` defaults to `section` rather than `whole`, which is the token decision of
/// `sources/context-reduction.md`: the cheapest call must not be the one that returns a
/// forty-thousand-token report.
fn source_get(root: &Path, arguments: &Value) -> Result<ToolResult, ToolError> {
    let chunk = arguments.get("chunk").and_then(Value::as_str);
    let id = arguments.get("id").and_then(Value::as_str);
    let detail = arguments
        .get("detail")
        .and_then(Value::as_str)
        .unwrap_or("section");
    if !matches!(detail, "snippet" | "section" | "whole") {
        return Err(ToolError::new(
            "AKR-C004",
            format!("`detail`: {detail:?} is not `snippet`, `section` or `whole`"),
        ));
    }
    let command = match (chunk, id) {
        (Some(_), Some(_)) => {
            return Err(ToolError::new(
                "AKR-C005",
                "`chunk` and `id` are mutually exclusive",
            ));
        }
        (Some(chunk), None) => Command::SourceGetChunk {
            chunk: chunk.to_owned(),
            // `section` is one chunk either side; `snippet` is the chunk alone. `whole`
            // is meaningless for a chunk id, so it widens as far as `section` does and
            // the caller is expected to pass `id` when it wants the document.
            neighbors: usize::from(detail != "snippet"),
        },
        (None, Some(id)) => Command::SourceGet {
            id: id.to_owned(),
            whole: detail == "whole",
            lines: arguments
                .get("lines")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            section: None,
        },
        (None, None) => {
            return Err(ToolError::new(
                "AKR-C003",
                "knowledge.source_get requires exactly one of `chunk` or `id`",
            ));
        }
    };
    run_read(root, command)
}

fn source_verify(root: &Path, _arguments: &Value) -> Result<ToolResult, ToolError> {
    run_read(root, Command::SourceVerify)
}

fn source_supersede(root: &Path, arguments: &Value) -> Result<ToolResult, ToolError> {
    run_write(
        root,
        Command::SourceSupersede {
            old_id: required_str(arguments, "old_id")?.to_owned(),
            new_path: PathBuf::from(required_str(arguments, "new_path")?),
            new_id: optional_str(arguments, "new_id"),
        },
    )
}

fn source_status(root: &Path, arguments: &Value) -> Result<ToolResult, ToolError> {
    run_read(
        root,
        Command::SourceStatus {
            id: required_str(arguments, "id")?.to_owned(),
        },
    )
}

fn source_dependents(root: &Path, arguments: &Value) -> Result<ToolResult, ToolError> {
    run_read(
        root,
        Command::SourceDependents {
            id: required_str(arguments, "id")?.to_owned(),
        },
    )
}

fn source_finalize(root: &Path, arguments: &Value) -> Result<ToolResult, ToolError> {
    let retain = arguments
        .get("retain")
        .and_then(Value::as_str)
        .unwrap_or("cited");
    let context = arguments
        .get("context")
        .and_then(Value::as_str)
        .unwrap_or("block");
    if !matches!(retain, "cited" | "metadata") || !matches!(context, "exact" | "block") {
        return Err(ToolError::new(
            "AKR-C004",
            "source finalization retention is cited|metadata and context is exact|block",
        ));
    }
    run_write(
        root,
        Command::SourceFinalize {
            id: required_str(arguments, "id")?.to_owned(),
            retain: retain.to_owned(),
            context: context.to_owned(),
            remove_file: flag(arguments, "remove_file"),
            dry_run: flag(arguments, "dry_run"),
        },
    )
}

fn impact(root: &Path, arguments: &Value) -> Result<ToolResult, ToolError> {
    let reference = arguments.get("ref").and_then(Value::as_str);
    let git_diff = arguments.get("git_diff").and_then(Value::as_str);
    match (reference, git_diff) {
        (None, None) => Err(ToolError::new(
            "AKR-C003",
            "knowledge.impact requires exactly one of `ref` or `git_diff`",
        )),
        (Some(_), Some(_)) => Err(ToolError::new(
            "AKR-C005",
            "`ref` and `git_diff` are mutually exclusive",
        )),
        _ => {
            let command = Command::Impact {
                reference: reference.map(ToOwned::to_owned),
                git_diff: git_diff.map(ToOwned::to_owned),
            };
            run_read(root, command)
        }
    }
}

/// `knowledge.validate` — §3's `{ok, diagnostics, counts}`.
///
/// The one read tool whose payload is not the command's `result` verbatim: `akr check`
/// reports the pipeline's stage counts, and an agent wants a verdict. Both are computed
/// from the same [`Output`], so they cannot disagree.
fn validate(root: &Path, arguments: &Value) -> Result<ToolResult, ToolError> {
    let mut session = open(root, false)?;
    let command = Command::Check {
        scratch_clean: flag(arguments, "scratch_clean"),
        review_clean: flag(arguments, "review_clean"),
        views_current: false,
    };
    let output = commands::run(&mut session, &command).map_err(environment)?;
    let diagnostics = commands::diagnostics_json(&output.diagnostics, &session.sources);
    let diagnostics_total = diagnostics.len();
    let offset = arguments
        .get("offset")
        .and_then(Value::as_integer)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(0);
    let limit = arguments
        .get("limit")
        .and_then(Value::as_integer)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(5)
        .clamp(1, 100);
    let diagnostics: Vec<Value> = diagnostics.into_iter().skip(offset).take(limit).collect();
    let returned = diagnostics.len();
    let next_offset = offset.saturating_add(returned);
    let has_more = next_offset < diagnostics_total;
    let counts = Value::object(vec![
        ("records", field(&output.result, "records")),
        ("revisions", field(&output.result, "revisions")),
        ("stale", field(&output.result, "stale")),
        ("at_risk", field(&output.result, "at_risk")),
    ]);
    // Never an error: `knowledge.validate` reports a verdict, and a ledger with
    // diagnostics is a fact about the ledger rather than a failure of the call.
    Ok(ToolResult::Structured(Value::object(vec![
        ("ok", Value::bool(output.exit == Exit::Ok)),
        ("diagnostics", Value::array(diagnostics)),
        (
            "diagnostics_total",
            Value::integer(i64::try_from(diagnostics_total).unwrap_or(i64::MAX)),
        ),
        (
            "diagnostics_returned",
            Value::integer(i64::try_from(returned).unwrap_or(i64::MAX)),
        ),
        (
            "diagnostics_offset",
            Value::integer(i64::try_from(offset).unwrap_or(i64::MAX)),
        ),
        ("has_more", Value::bool(has_more)),
        (
            "next_offset",
            if has_more {
                Value::integer(i64::try_from(next_offset).unwrap_or(i64::MAX))
            } else {
                Value::Null
            },
        ),
        ("counts", counts),
    ])))
}

// -------------------------------------------------------------------------------------
// write tools
// -------------------------------------------------------------------------------------

fn propose(root: &Path, arguments: &Value) -> Result<ToolResult, ToolError> {
    let session = open(root, true)?;
    let key = required_str(arguments, "key")?;
    let kind_name = required_str(arguments, "kind")?;
    let title = required_str(arguments, "title")?;
    let kind = akr_core::model::Kind::from_name(kind_name)
        .ok_or_else(|| ToolError::new("AKR-C004", format!("`{kind_name}` is not a record kind")))?;
    let parsed = key_of(key)?;

    let source = record::to_source(
        &record::Heading {
            root: &session.root,
            project: &session.ledger.project.name,
            key: &parsed,
            revision: 1,
            kind,
            title,
            state: arguments.get("state").and_then(Value::as_str),
        },
        arguments,
    )?;
    let template = record::parse(&source, &parsed)?;
    let context = write_context(&session);
    Ok(ToolResult::Structured(write_result(
        &session,
        &parsed,
        akr_core::ops::propose(&context, &parsed, kind, title, Some(template)),
    )?))
}

fn revise(root: &Path, arguments: &Value) -> Result<ToolResult, ToolError> {
    let session = open(root, true)?;
    let key = required_str(arguments, "key")?;
    let parsed = key_of(key)?;
    let head = session
        .ledger
        .head(&parsed)
        .ok()
        .ok_or_else(|| ToolError::new("AKR-L001", format!("{key} does not resolve")))?;

    // §3: `base_rev` must equal the current head revision. This is the only concurrency
    // control the surface has, and it is enough because the store is a git working tree a
    // human is also watching.
    let base_rev = arguments
        .get("base_rev")
        .and_then(Value::as_integer)
        .ok_or_else(|| ToolError::new("AKR-C003", "knowledge.revise requires `base_rev`"))?;
    if base_rev != i64::from(head.id.revision) {
        return Err(ToolError::new(
            "AKR-C033",
            format!(
                "base_rev {base_rev} is not the head of {key}, which is at revision {}",
                head.id.revision
            ),
        ));
    }

    let title = arguments
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or(&head.title);
    let source = record::to_source(
        &record::Heading {
            root: &session.root,
            project: &session.ledger.project.name,
            key: &parsed,
            revision: head.id.revision + 1,
            kind: head.kind,
            title,
            state: arguments.get("state").and_then(Value::as_str),
        },
        &merged(arguments, head),
    )?;
    let replacement = record::parse(&source, &parsed)?;
    let requested_state = arguments
        .get("state")
        .and_then(Value::as_str)
        .map(|_| replacement.state);

    let edits = akr_core::ops::Edits {
        title: Some(title.to_owned()),
        // `ops::revise` deliberately resets a content-only revision of a sealed head to
        // the class initial state. An explicit lifecycle request is different: carry it
        // separately so the new revision lands in the state the caller asked for.
        state: requested_state,
        replace_with: Some(Box::new(replacement)),
    };
    let dispositions = dispositions(arguments)?;
    let context = write_context(&session);
    Ok(ToolResult::Structured(write_result(
        &session,
        &parsed,
        akr_core::ops::revise_with_dispositions(
            &context,
            &parsed,
            akr_core::ops::ReviseMode::Auto,
            &edits,
            &dispositions,
        ),
    )?))
}

fn supersede(root: &Path, arguments: &Value) -> Result<ToolResult, ToolError> {
    let session = open(root, true)?;
    let key = required_str(arguments, "old_key")?;
    let parsed = key_of(key)?;
    let replacement = arguments
        .get("new_key")
        .and_then(Value::as_str)
        .map(key_of)
        .transpose()?
        .unwrap_or_else(|| parsed.clone());
    if replacement != parsed && session.ledger.head(&replacement).is_err() {
        return Err(ToolError::new(
            "AKR-L001",
            format!(
                "replacement {replacement} does not exist; create it with knowledge.propose, then retry knowledge.supersede"
            ),
        ));
    }
    if arguments.get("slots").is_some() {
        return Err(ToolError::new(
            "AKR-C004",
            "knowledge.supersede does not author content; use knowledge.revise for the same key or knowledge.propose for a different replacement, then omit `slots` here",
        ));
    }
    let dispositions = dispositions(arguments)?;
    let context = write_context(&session);
    let result = if replacement == parsed {
        akr_core::ops::supersede(&context, &parsed, &dispositions)
    } else {
        akr_core::ops::supersede_with(&context, &parsed, &replacement, &dispositions)
    };
    Ok(ToolResult::Structured(write_result(
        &session,
        &replacement,
        result,
    )?))
}

fn complete(root: &Path, arguments: &Value) -> Result<ToolResult, ToolError> {
    let session = open(root, true)?;
    let key = required_str(arguments, "key")?;
    let parsed = key_of(key)?;
    let mut checks = Vec::new();
    if let Some(Value::Object(pairs)) = arguments.get("checks") {
        for (id, reference) in pairs {
            let text = reference.as_str().ok_or_else(|| {
                ToolError::new("AKR-C004", format!("check `{id}` needs a reference"))
            })?;
            let reference = akr_core::model::Reference::parse(text).map_err(|e| {
                ToolError::new(
                    "AKR-C004",
                    format!("check `{id}`: {text:?} is not a reference: {e}"),
                )
            })?;
            checks.push((id.clone(), reference));
        }
    }
    let context = write_context(&session);
    Ok(ToolResult::Structured(write_result(
        &session,
        &parsed,
        akr_core::ops::complete(&context, &parsed, &checks),
    )?))
}

/// `knowledge.evidence_add`, over the same [`akr_core::evidence::AddEvidence`] request
/// `akr evidence add` builds.
///
/// The schema deliberately has no field for what the evidence verifies (D-016): the link
/// is authored on the check (`verified_by`) or supplied to `knowledge.complete`.
/// One evidence record.
///
/// The translation from JSON to an [`akr_cli::write::EvidenceRequest`] is argument
/// mapping, which is this crate's job; everything after it — parsing the key, defaulting
/// the commit, building the record, checking the commit exists, writing — belongs to
/// `akr-cli` and is reached through it, so the command line and this tool cannot drift.
fn evidence_add(root: &Path, arguments: &Value) -> Result<ToolResult, ToolError> {
    let session = open(root, true)?;
    let request = evidence_request(arguments)?;
    let parsed = key_of(&request.key)?;
    let context = write_context(&session);
    let (_, record, commit) =
        akr_cli::write::evidence_record(&session, &context, &request).map_err(environment)?;
    akr_cli::write::ensure_commits_exist(&session, &[commit]).map_err(environment)?;
    let title = record.title.clone();
    Ok(ToolResult::Structured(write_result(
        &session,
        &parsed,
        akr_core::ops::propose(
            &context,
            &parsed,
            akr_core::model::Kind::Evidence,
            &title,
            Some(record),
        ),
    )?))
}

/// Adds several evidence records through one parse/apply/validate/fsync transaction.
///
/// The batch itself lives in `akr-cli` alongside `akr evidence add-many`, which reads the
/// same records from a file. This tool had its own copy, which is how it came to be
/// reachable here and nowhere else.
fn evidence_add_many(root: &Path, arguments: &Value) -> Result<ToolResult, ToolError> {
    let session = open(root, true)?;
    let items = arguments
        .get("evidence")
        .and_then(Value::as_array)
        .ok_or_else(|| ToolError::new("AKR-C003", "`evidence` must be a non-empty array"))?;
    if items.is_empty() {
        return Err(ToolError::new(
            "AKR-C003",
            "`evidence` must contain at least one record",
        ));
    }
    if items.len() > 100 {
        return Err(ToolError::new(
            "AKR-C004",
            "`evidence` accepts at most 100 records per transaction",
        ));
    }

    let context = write_context(&session);
    let mut records = Vec::with_capacity(items.len());
    for item in items {
        let request = evidence_request(item)?;
        let (_, record, _) =
            akr_cli::write::evidence_record(&session, &context, &request).map_err(environment)?;
        records.push(record);
    }
    let target = records[0].id.key.clone();
    let result = akr_core::ops::propose_many(&context, &records);
    akr_cli::write::ensure_commits_exist(
        &session,
        &records
            .iter()
            .filter_map(
                |record| match record.get(akr_core::model::ContentSlot::ObservedAt) {
                    Some(akr_core::model::ContentValue::Commit(commit)) => Some(commit.clone()),
                    _ => None,
                },
            )
            .collect::<Vec<_>>(),
    )
    .map_err(environment)?;
    Ok(ToolResult::Structured(write_many_result(
        &session, &target, result,
    )?))
}

/// One evidence payload, translated to the surface-agnostic request `akr-cli` takes.
fn evidence_request(arguments: &Value) -> Result<akr_cli::write::EvidenceRequest, ToolError> {
    Ok(akr_cli::write::EvidenceRequest {
        key: required_str(arguments, "key")?.to_owned(),
        result: optional_str(arguments, "result"),
        method: optional_str(arguments, "method"),
        title: optional_str(arguments, "title"),
        command: optional_str(arguments, "command"),
        artifact: optional_str(arguments, "artifact"),
        summary: optional_str(arguments, "summary"),
        // `git:` is the reference spelling in a record; the request takes the bare hash,
        // as the command line does.
        observed_at: optional_str(arguments, "observed_at")
            .map(|text| text.strip_prefix("git:").unwrap_or(&text).to_owned()),
    })
}

/// `knowledge.papercut`, over the same [`akr_core::papercut`] request `akr papercut`
/// builds (D-027). The message is the whole ceremony.
fn papercut(root: &Path, arguments: &Value) -> Result<ToolResult, ToolError> {
    let session = open(root, true)?;
    let message = required_str(arguments, "message")?;
    let agent = required_str(arguments, "agent")?;
    let namespace = arguments.get("namespace").and_then(Value::as_str);

    let commit = session
        .commit
        .clone()
        .ok_or_else(|| missing_commit(&session, false))?;
    let key = akr_core::papercut::allocate_key(&session.ledger, namespace, message)
        .map_err(|e| ToolError::new("AKR-C004", e.to_string()))?;
    let request = akr_core::papercut::LogPapercut {
        message: message.to_owned(),
        agent: agent.to_owned(),
        observed_at: commit,
        created_at: Some(session.today),
        about: arguments
            .get("about")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    };
    let record = request.to_record(key.clone());
    let title = record.title.clone();
    let context = write_context(&session);
    Ok(ToolResult::Structured(write_result(
        &session,
        &key,
        akr_core::ops::propose(
            &context,
            &key,
            akr_core::model::Kind::Papercut,
            &title,
            Some(record),
        ),
    )?))
}

// -------------------------------------------------------------------------------------
// shared plumbing
// -------------------------------------------------------------------------------------

/// Opens the workspace, exactly as the command line opens it.
fn open(root: &Path, writing: bool) -> Result<Session, ToolError> {
    let global = Global {
        dir: PathBuf::from(root),
        profile: Profile::Strict,
        format: Format::Json,
        ..Global::default()
    };
    let mut session = Session::open_ledger(&global).map_err(environment)?;
    if writing {
        session.ensure_git_facts().map_err(environment)?;
    }
    Ok(session)
}

/// Runs a read command, keeping both CLI text and JSON.
///
/// Verbatim is the point. §1's invariant is that a tool and its command produce the same
/// answer, and the cheapest way to keep that true is for the tool to add nothing.
fn run_read(root: &Path, command: Command) -> Result<ToolResult, ToolError> {
    let mut session = open(root, false)?;
    let output = commands::run(&mut session, &command).map_err(environment)?;
    let text = output.text.clone();
    let structured = finish(&session.sources, output)?;
    Ok(ToolResult::Read { text, structured })
}

fn run_write(root: &Path, command: Command) -> Result<ToolResult, ToolError> {
    let mut session = open(root, true)?;
    let output = commands::run(&mut session, &command).map_err(environment)?;
    Ok(ToolResult::Structured(finish(&session.sources, output)?))
}

/// Runs a command that answers from the vocabulary tables, with no ledger open.
///
/// `explain` reports what a kind requires, and the moment that is worth asking is the
/// moment a record was written wrongly — so it must not need a ledger that parses. It also
/// has no arm in `commands::dispatch`, which is why routing it through [`run_read`] reached
/// an `unreachable!` and the server reported `AKR-X099` instead of the schema.
fn run_vocabulary(command: Command) -> Result<ToolResult, ToolError> {
    let Some(result) = commands::run_standalone(&command) else {
        return Err(ToolError {
            class: Class::Internal,
            summary: format!(
                "`{}` needs a ledger and cannot be answered from the vocabulary tables",
                command.name()
            ),
            diagnostics: Vec::new(),
            wrote: false,
        });
    };
    let output = result.map_err(environment)?;
    let text = output.text.clone();
    let structured = finish(&akr_core::diagnostics::SourceMap::new(), output)?;
    Ok(ToolResult::Read { text, structured })
}

#[derive(Debug)]
struct ContextCandidate {
    key: String,
    reference: String,
    title: String,
    kind: String,
    state: String,
    path_overlap: bool,
}

fn unresolved_goal(error: EnvError, session: &Session, goal: &str, command: &Command) -> ToolError {
    let mut candidates = context_candidates(session, goal, command);
    candidates.truncate(20);
    let mut diagnostics = vec![Value::object(vec![
        ("code", Value::string(error.code)),
        ("severity", Value::string("error")),
        ("message", Value::string(error.message.clone())),
    ])];
    if let Some(first) = candidates.first() {
        let total = candidates.len();
        let plural = if total == 1 {
            "candidate"
        } else {
            "candidates"
        };
        diagnostics.push(Value::object(vec![
            ("code", Value::string("AKR-X001")),
            ("severity", Value::string("info")),
            (
                "message",
                Value::string(format!("planning goal candidates ({total} {plural})")),
            ),
            (
                "next",
                Value::object(vec![
                    ("tool", Value::string("knowledge.context")),
                    (
                        "arguments",
                        Value::object({
                            let mut args = Vec::with_capacity(3);
                            if let Command::Context { paths, budget, .. } = command {
                                args.push(("goal", Value::string(first.key.clone())));
                                if !paths.is_empty() {
                                    args.push((
                                        "paths",
                                        Value::array(
                                            paths
                                                .iter()
                                                .map(|path| Value::string(path.as_str()))
                                                .collect(),
                                        ),
                                    ));
                                }
                                if let Some(budget) = budget {
                                    args.push((
                                        "budget_tokens",
                                        Value::integer(i64::try_from(*budget).unwrap_or(i64::MAX)),
                                    ));
                                }
                            }
                            args
                        }),
                    ),
                ]),
            ),
                (
                    "path_overlap_hint",
                    Value::string(format!(
                        "preferred entries have path_overlap = true; use context goal={} with an exact key",
                        first.key
                    )),
                ),
            ("candidates", Value::array(context_candidates_as_json(&candidates))),
        ]));
        ToolError {
            class: class_of(error.code),
            summary: error.message,
            diagnostics,
            wrote: false,
        }
    } else {
        ToolError::new(error.code, error.message)
    }
}

fn context_candidates(session: &Session, goal: &str, command: &Command) -> Vec<ContextCandidate> {
    let paths = match command {
        Command::Context { paths, .. } => paths.as_slice(),
        _ => &[],
    };
    let model = session.resolve();
    let request_scope: Vec<ScopeTerm> = paths.iter().cloned().map(ScopeTerm::Path).collect();
    let goal = goal.trim_start_matches('@').to_ascii_lowercase();
    let mut out = Vec::new();
    for record in model.ledger().records() {
        if !matches!(record.kind, Kind::Milestone | Kind::Work | Kind::Track) {
            continue;
        }
        if !model.is_head(&record.id) {
            continue;
        }
        if !record.is_live() {
            continue;
        }
        let title = record.title.to_lowercase();
        let key = record.id.key.to_string().to_ascii_lowercase();
        let score = usize::from(key.contains(&goal)) * 8
            + usize::from(title.contains(&goal)) * 4
            + usize::from(
                record
                    .id
                    .key
                    .segments()
                    .iter()
                    .any(|segment| goal.contains(&segment.as_str().to_ascii_lowercase())),
            ) * 2;
        let path_overlap = if request_scope.is_empty() {
            false
        } else {
            scopes_overlap(
                &record.scope,
                &request_scope,
                &model.ledger().part_of_index(),
            )
        };
        if score > 0 {
            out.push((
                score,
                ContextCandidate {
                    key: record.id.key.to_string(),
                    reference: record.id.to_string(),
                    title: record.title.clone(),
                    kind: record.kind.name().to_owned(),
                    state: record.state.name().to_owned(),
                    path_overlap,
                },
            ));
        }
    }
    out.sort_by(|a, b| {
        b.0.cmp(&a.0).then_with(|| {
            b.1.path_overlap
                .cmp(&a.1.path_overlap)
                .then_with(|| a.1.reference.cmp(&b.1.reference))
        })
    });
    out.into_iter().map(|(_, candidate)| candidate).collect()
}

fn search_recommended_context(
    result: &Value,
    paths: &[Glob],
    budget_tokens: Option<usize>,
) -> Option<Value> {
    let results = result.get("results")?.as_array()?;
    if results.is_empty() {
        return None;
    }
    let top = results.first()?;
    let goal = top.get("key")?.as_str()?;
    let top_kind = top.get("kind")?.as_str()?;
    if !matches!(top_kind, "milestone" | "work" | "track") {
        return None;
    }
    let top_score = search_score(top)?;
    if let Some(second) = results.get(1)
        && let Some(second_score) = search_score(second)
        && top_score <= second_score + 0.5
    {
        return None;
    }
    let mut arguments = vec![("goal".to_owned(), Value::string(goal.to_owned()))];
    if !paths.is_empty() {
        arguments.push((
            "paths".to_owned(),
            Value::array(
                paths
                    .iter()
                    .map(|path| Value::string(path.as_str()))
                    .collect(),
            ),
        ));
    }
    if let Some(budget) = budget_tokens {
        arguments.push((
            "budget_tokens".to_owned(),
            Value::integer(i64::try_from(budget).unwrap_or(i64::MAX)),
        ));
    }
    Some(Value::Object(vec![
        ("tool".to_owned(), Value::string("knowledge.context")),
        (
            "arguments".to_owned(),
            Value::Object(arguments.into_iter().collect()),
        ),
    ]))
}

fn search_score(result: &Value) -> Option<f64> {
    match result.get("score") {
        Some(Value::String(text)) => text.parse().ok(),
        Some(Value::Integer(value)) => Some(*value as f64),
        _ => None,
    }
}

fn context_candidates_as_json(candidates: &[ContextCandidate]) -> Vec<Value> {
    candidates
        .iter()
        .map(|candidate| {
            Value::object(vec![
                (
                    "reference",
                    Value::string(format!("@{}", candidate.reference)),
                ),
                ("title", Value::string(candidate.title.clone())),
                ("kind", Value::string(candidate.kind.clone())),
                ("state", Value::string(candidate.state.clone())),
                ("path_overlap", Value::bool(candidate.path_overlap)),
            ])
        })
        .collect()
}

fn finish(sources: &akr_core::diagnostics::SourceMap, output: Output) -> Result<Value, ToolError> {
    if output.exit == Exit::Ok {
        return Ok(output.result);
    }
    let diagnostics = commands::diagnostics_json(&output.diagnostics, sources);
    let code = first_error_code(&diagnostics)
        .unwrap_or("AKR-R001")
        .to_owned();
    let summary = output
        .diagnostics
        .first()
        .map_or_else(|| "the command failed".to_owned(), |d| d.message.clone());
    Err(ToolError {
        class: class_of(&code),
        summary,
        diagnostics,
        wrote: false,
    })
}

/// The write context, from `akr-cli` so that it is the same one the command line uses.
///
/// Building a bare `WriteContext` here left every record written over MCP without an
/// `author`, while the identical command-line write recorded one from `git config
/// user.name` (D-005). Two surfaces, two answers, for as long as nothing compared the
/// bytes they produced.
fn write_context(session: &Session) -> akr_core::ops::WriteContext {
    akr_cli::write::context_of(session)
}

/// Renders an `ops` outcome as §4's payload, or as §5's refusal.
///
/// `target` is the key the tool was asked about. A revision or a supersession touches two
/// revisions of it — the retired head and its successor — and `Applied::changes` is in key
/// order, so the successor is the *last* of them, not the first. The payload names the
/// revision the write produced, because that is the one the agent's next `base_rev` has to
/// be.
fn write_result(
    session: &Session,
    target: &LogicalKey,
    result: akr_core::ops::WriteResult,
) -> Result<Value, ToolError> {
    match result {
        Ok(applied) => {
            let first = applied
                .changes
                .iter()
                .filter(|change| &change.id.key == target)
                .max_by_key(|change| change.id.revision)
                .or_else(|| applied.changes.first());
            let path = applied
                .files
                .first()
                .map(|file| session.akr_dir.join(file))
                .and_then(|full| {
                    full.strip_prefix(&session.root)
                        .ok()
                        .map(|p| p.to_string_lossy().replace('\\', "/"))
                })
                .unwrap_or_default();
            let mut fields = vec![
                (
                    "key",
                    Value::string(first.map(|c| c.id.key.to_string()).unwrap_or_default()),
                ),
                (
                    "rev",
                    Value::integer(first.map_or(0, |c| i64::from(c.id.revision))),
                ),
                ("path", Value::string(path)),
                ("written", Value::bool(true)),
                ("lock_stale", Value::bool(applied.lock_stale)),
            ];
            // Advisory, never blocking, and the CLI prints the same lines: a revision of a
            // completed record has just put every acceptance reference back in question,
            // and this is the only moment anyone is looking.
            if !applied.notes.is_empty() {
                fields.push((
                    "notes",
                    Value::array(applied.notes.iter().cloned().map(Value::string).collect()),
                ));
            }
            if applied.lock_stale {
                fields.push((
                    "next",
                    Value::object(vec![
                        ("command", Value::string("akr build")),
                        (
                            "reason",
                            Value::string(
                                "refresh akr.lock and generated views before knowledge.validate",
                            ),
                        ),
                    ]),
                ));
            }
            // §4's `state` and `content_hash` describe the revision that just landed, so
            // they have to come from a workspace read *after* the write. Reload only the
            // ledger bytes: `Session::open` also derives Git freshness facts and used to
            // repeat the expensive history walk solely to render these two fields.
            if let Some(change) = first
                && let Ok(written) =
                    akr_core::resolve::load_workspace(&session.root, &session.akr_dir)
            {
                if let Some(record) = written.ledger.get(&change.id) {
                    fields.insert(2, ("state", Value::string(record.state.name())));
                }
                if let Some(text) = written.inputs.canonical_text.get(&change.id) {
                    let hash = akr_core::hash::content_hash(text);
                    fields.push(("content_hash", Value::string(hash.to_string())));
                }
            }
            Ok(Value::object(fields))
        }
        Err(refused) => {
            let mut diagnostics = vec![diagnostic_json(&refused.diagnostic())];
            diagnostics.extend(refused.diagnostics.iter().map(diagnostic_json));
            let mut error = ToolError {
                class: class_of(refused.code.as_str()),
                summary: refused.message.clone(),
                diagnostics,
                wrote: false,
            };
            // §4: the children are listed in the error payload, so the agent's next
            // message can name them. That is the moment the design cares most about.
            if !refused.unfinished_children.is_empty() {
                error.diagnostics.push(Value::object(vec![
                    ("code", Value::string(refused.code.as_str())),
                    ("severity", Value::string("error")),
                    (
                        "message",
                        Value::string("unfinished children need a disposition"),
                    ),
                    (
                        "unfinished_children",
                        Value::array(
                            refused
                                .unfinished_children
                                .iter()
                                .map(|child| {
                                    Value::object(vec![
                                        ("child", Value::string(child.key.to_string())),
                                        ("state", Value::string(child.state.name())),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                ]));
            }
            if !refused.unsatisfied_checks.is_empty() {
                error.diagnostics.push(Value::object(vec![
                    ("code", Value::string(refused.code.as_str())),
                    ("severity", Value::string("error")),
                    (
                        "message",
                        Value::string("acceptance checks are not satisfied"),
                    ),
                    (
                        "unsatisfied_checks",
                        Value::array(
                            refused
                                .unsatisfied_checks
                                .iter()
                                .map(|check| {
                                    Value::object(vec![
                                        ("id", Value::string(check.id.clone())),
                                        ("reason", Value::string(check.reason.clone())),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                ]));
            }
            Err(error)
        }
    }
}

fn write_many_result(
    session: &Session,
    first_target: &LogicalKey,
    result: akr_core::ops::WriteResult,
) -> Result<Value, ToolError> {
    let applied = match result {
        Ok(applied) => applied,
        Err(refused) => return write_result(session, first_target, Err(refused)),
    };
    let written = akr_core::resolve::load_workspace(&session.root, &session.akr_dir).ok();
    let entries = applied
        .changes
        .iter()
        .map(|change| {
            let path = session
                .akr_dir
                .join(&change.file)
                .strip_prefix(&session.root)
                .ok()
                .map(|path| path.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            let mut fields = vec![
                ("key", Value::string(change.id.key.to_string())),
                ("rev", Value::integer(i64::from(change.id.revision))),
                ("path", Value::string(path)),
            ];
            if let Some(workspace) = &written {
                if let Some(record) = workspace.ledger.get(&change.id) {
                    fields.push(("state", Value::string(record.state.name())));
                }
                if let Some(text) = workspace.inputs.canonical_text.get(&change.id) {
                    fields.push((
                        "content_hash",
                        Value::string(akr_core::hash::content_hash(text).to_string()),
                    ));
                }
            }
            Value::object(fields)
        })
        .collect();
    let mut fields = vec![
        ("evidence", Value::array(entries)),
        (
            "written",
            Value::integer(i64::try_from(applied.changes.len()).unwrap_or(i64::MAX)),
        ),
        ("lock_stale", Value::bool(applied.lock_stale)),
    ];
    if applied.lock_stale {
        fields.push((
            "next",
            Value::object(vec![
                ("command", Value::string("akr build")),
                (
                    "reason",
                    Value::string("refresh akr.lock and generated views before knowledge.validate"),
                ),
            ]),
        ));
    }
    Ok(Value::object(fields))
}

fn diagnostic_json(diagnostic: &akr_core::diagnostics::Diagnostic) -> Value {
    let mut fields = vec![
        ("code", Value::string(diagnostic.code.as_str())),
        ("severity", Value::string("error")),
        ("message", Value::string(diagnostic.message.clone())),
    ];
    if let Some(rule) = diagnostic.rule {
        fields.push(("rule", Value::string(rule.to_string())));
    }
    if let Some(help) = &diagnostic.help {
        fields.push(("help", Value::string(help.clone())));
    }
    Value::object(fields)
}

fn environment(error: EnvError) -> ToolError {
    // `AKR-I022` and `AKR-M002` are the deferred surfaces — search and import — and they
    // are environment failures in the sense that matters: not the agent's fault, and not
    // fixable by trying again.
    let class = match class_of(error.code) {
        Class::Invariant => Class::Environment,
        other => other,
    };
    // Carry the CLI's `help:` line into the diagnostic, the same optional field
    // `diagnostic_json` already emits (docs/07-cli.md §5). Without it, an agent that hits
    // `AKR-C011` over MCP is told there is no ledger but not that `akr init` makes one —
    // the remedy the command line prints.
    let mut fields = vec![
        ("code", Value::string(error.code)),
        ("severity", Value::string("error")),
        ("message", Value::string(error.message.clone())),
    ];
    if let Some(help) = &error.help {
        fields.push(("help", Value::string(help.clone())));
    }
    ToolError {
        class,
        summary: error.message,
        diagnostics: vec![Value::object(fields)],
        wrote: false,
    }
}

fn dispositions(arguments: &Value) -> Result<Vec<akr_core::ops::DispositionRequest>, ToolError> {
    let mut out = Vec::new();
    for item in arguments
        .get("dispositions")
        .and_then(Value::as_array)
        .unwrap_or_default()
    {
        let child = item
            .get("child")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::new("AKR-C003", "each disposition needs a `child`"))?;
        let outcome_name = item
            .get("outcome")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::new("AKR-C003", "each disposition needs an `outcome`"))?;
        let outcome = akr_core::model::Outcome::ALL
            .iter()
            .copied()
            .find(|candidate| candidate.name() == outcome_name)
            .ok_or_else(|| {
                ToolError::new(
                    "AKR-C004",
                    format!("`{outcome_name}` is not a disposition outcome"),
                )
            })?;
        out.push(akr_core::ops::DispositionRequest {
            child: key_of(child)?,
            outcome,
            into: item
                .get("into")
                .and_then(Value::as_str)
                .map(key_of)
                .transpose()?,
            note: item
                .get("note")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
        });
    }
    Ok(out)
}

/// A revision's payload merged onto its head, so an edit naming one slot keeps the rest.
///
/// `knowledge.revise` takes the slots that change, not the whole record — an agent that
/// had to resend every slot would eventually drop one, and dropping a slot is how a record
/// silently loses a claim.
fn merged(arguments: &Value, head: &akr_core::model::Record) -> Value {
    let mut slots: Vec<(String, Value)> = Vec::new();
    for (slot, value) in &head.content {
        let rendered = match value {
            akr_core::model::ContentValue::Prose(text)
            | akr_core::model::ContentValue::Text(text) => Value::string(text.clone()),
            akr_core::model::ContentValue::Date(date) => Value::string(date.to_string()),
            akr_core::model::ContentValue::Commit(commit) => Value::string(commit.as_str()),
            akr_core::model::ContentValue::Enum(word) => Value::string(word.to_string()),
            akr_core::model::ContentValue::Strings(items) => {
                Value::array(items.iter().map(|s| Value::string(s.clone())).collect())
            }
            akr_core::model::ContentValue::Globs(items) => {
                Value::array(items.iter().map(|g| Value::string(g.as_str())).collect())
            }
            akr_core::model::ContentValue::Refs(items) => {
                Value::array(items.iter().map(|r| Value::string(r.to_string())).collect())
            }
        };
        slots.push((slot.name().to_owned(), rendered));
    }
    if let Some(Value::Object(given)) = arguments.get("slots") {
        for (name, value) in given {
            slots.retain(|(existing, _)| existing != name);
            slots.push((name.clone(), value.clone()));
        }
    }

    let mut relations: Vec<(String, Value)> = Vec::new();
    for (relation, targets) in &head.relations {
        relations.push((
            relation.name().to_owned(),
            Value::array(
                targets
                    .iter()
                    .map(|r| Value::string(r.to_string()))
                    .collect(),
            ),
        ));
    }
    if let Some(Value::Object(given)) = arguments.get("relations") {
        for (name, value) in given {
            relations.retain(|(existing, _)| existing != name);
            relations.push((name.clone(), value.clone()));
        }
    }

    let mut fields = vec![
        ("slots".to_owned(), Value::Object(slots)),
        ("relations".to_owned(), Value::Object(relations)),
    ];
    if let Some(scope) = arguments.get("scope") {
        fields.push(("scope".to_owned(), scope.clone()));
    } else if !head.scope.is_empty() {
        fields.push((
            "scope".to_owned(),
            Value::array(
                head.scope
                    .iter()
                    .map(|term| match term {
                        akr_core::model::ScopeTerm::All => Value::string("all"),
                        akr_core::model::ScopeTerm::Path(glob) => Value::string(glob.as_str()),
                        akr_core::model::ScopeTerm::Ref(reference) => {
                            Value::string(reference.to_string())
                        }
                    })
                    .collect(),
            ),
        ));
    }
    for name in [
        "claims",
        "retired_claims",
        "author",
        "created_at",
        "acceptance",
        "acknowledged",
        "topic",
    ] {
        if let Some(value) = arguments.get(name) {
            fields.push((name.to_owned(), value.clone()));
        }
    }

    // Two more the merge used to build a fresh payload without, and so silently dropped
    // from the next revision: `topic` is a normative record's exclusivity handle (D-004b),
    // and `acknowledged` is what keeps a tolerated contradiction from failing V-023. Both
    // carry forward by default, for the same reason `sources` does below — an edit naming
    // one slot must not retire a marker the author never mentioned.
    if arguments.get("topic").is_none()
        && let Some(topic) = &head.topic
    {
        fields.push(("topic".to_owned(), Value::string(topic.to_string())));
    }
    if arguments.get("acknowledged").is_none() && head.acknowledged {
        fields.push(("acknowledged".to_owned(), Value::bool(true)));
    }
    if let Some(acceptance) = &head.acceptance
        && arguments.get("acceptance").is_none()
        && !acceptance.checks.is_empty()
    {
        fields.push((
            "acceptance".to_owned(),
            Value::array(
                acceptance
                    .checks
                    .iter()
                    .map(|check| {
                        let mut check_fields = vec![
                            ("id", Value::string(check.id.to_string())),
                            ("statement", Value::string(check.statement.clone())),
                            ("method", Value::string(check.method.name())),
                        ];
                        if let Some(command) = &check.command {
                            check_fields.push(("command", Value::string(command.clone())));
                        }
                        if !check.verified_by.is_empty() {
                            check_fields.push((
                                "verified_by",
                                Value::array(
                                    check
                                        .verified_by
                                        .iter()
                                        .map(|r| Value::string(r.to_string()))
                                        .collect(),
                                ),
                            ));
                        }
                        Value::object(check_fields)
                    })
                    .collect(),
            ),
        ));
    }
    if arguments.get("claims").is_none() && !head.claims.is_empty() {
        fields.push((
            "claims".to_owned(),
            Value::array(
                head.claims
                    .iter()
                    .map(|claim| {
                        Value::object(vec![
                            ("anchor", Value::string(claim.anchor.to_string())),
                            ("text", Value::string(claim.text.clone())),
                        ])
                    })
                    .collect(),
            ),
        ));
    }
    // `sources` is advertised on knowledge.revise, but the merge used to build a fresh
    // payload out of slots, relations, scope, acceptance and claims alone — so an omitted
    // `sources` silently dropped the head's attributions and an explicit one was ignored.
    // Provenance is the one part of a record the ledger cannot reconstruct from anything
    // else, so it carries forward by default and is replaced only when asked for.
    if let Some(sources) = arguments.get("sources") {
        fields.push(("sources".to_owned(), sources.clone()));
    } else if !head.sources.is_empty() {
        fields.push(("sources".to_owned(), sources_payload(&head.sources)));
    }
    Value::Object(fields)
}

/// The head's source attributions, in the shape `record::to_source` reads them back from.
fn sources_payload(sources: &[akr_core::model::Source]) -> Value {
    Value::array(
        sources
            .iter()
            .map(|source| {
                let mut fields = vec![(
                    "kind".to_owned(),
                    Value::string(match source.kind {
                        akr_core::model::SourceKind::Legacy => "legacy",
                        akr_core::model::SourceKind::External => "external",
                        akr_core::model::SourceKind::Internal => "internal",
                    }),
                )];
                if let Some(role) = source.role {
                    fields.push(("role".to_owned(), Value::string(role.as_str())));
                }
                for (name, value) in [
                    ("path", source.path.as_ref()),
                    ("url", source.url.as_ref()),
                    ("excerpt", source.excerpt.as_ref()),
                    ("document", source.document.as_ref()),
                    ("use", source.use_note.as_ref()),
                ] {
                    if let Some(value) = value {
                        fields.push((name.to_owned(), Value::string(value.clone())));
                    }
                }
                if let Some(range) = &source.range {
                    for (name, value) in [
                        ("start_byte", range.start_byte),
                        ("end_byte", range.end_byte),
                        ("start_line", u64::from(range.start_line)),
                        ("end_line", u64::from(range.end_line)),
                    ] {
                        fields.push((
                            name.to_owned(),
                            Value::integer(i64::try_from(value).unwrap_or(i64::MAX)),
                        ));
                    }
                    if let Some(hash) = &range.excerpt_hash {
                        fields.push(("excerpt_hash".to_owned(), Value::string(hash.clone())));
                    }
                }
                Value::Object(fields)
            })
            .collect(),
    )
}

fn key_of(text: &str) -> Result<akr_core::model::LogicalKey, ToolError> {
    let trimmed = text.strip_prefix('@').unwrap_or(text);
    let trimmed = trimmed.split_once('/').map_or(trimmed, |(key, _)| key);
    akr_core::model::LogicalKey::parse(trimmed).map_err(|e| {
        ToolError::new(
            "AKR-C004",
            format!(
                "{text:?} is not a key: {e}; keys are dot-delimited — \
                 namespace.topic.slug — and the first segment must be a namespace \
                 declared in .akr/project.akr"
            ),
        )
    })
}

fn required_str<'a>(arguments: &'a Value, name: &str) -> Result<&'a str, ToolError> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::new("AKR-C003", format!("`{name}` is required")))
}

fn optional_str(arguments: &Value, name: &str) -> Option<String> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn flag(arguments: &Value, name: &str) -> bool {
    arguments
        .get(name)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn field(result: &Value, name: &str) -> Value {
    result.get(name).cloned().unwrap_or(Value::Integer(0))
}

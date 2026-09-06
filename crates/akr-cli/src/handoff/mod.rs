//! Handing work between agents: capsules, packets, results, coverage.
//!
//! # The primitive
//!
//! > A bounded, snapshot-bound, inheritable context object for transferring work between
//! > agents.
//!
//! Advisor review is one application of it; a subagent assignment is another; an
//! adversarial review is a third. They are the same object with different **disclosure**,
//! because the axis that actually separates them is epistemic — how much of the parent's
//! *judgement* the child is allowed to see — and not what the child is called.
//!
//! # Why it is three levels and not one
//!
//! ```text
//! PROJECT CAPSULE   pc-…    what is true of this repository
//!       ↓
//! SESSION CAPSULE   sx-…    what is true of this session
//!       ↓
//! PACKET            wk-… sc-… rv-… ad-…    what is true of this assignment
//! ```
//!
//! Five subagents launched from one session each re-derive the toolchain, the module map,
//! the build and test commands, the AKR ledger's shape and the current plan. That is five
//! copies of one answer, paid for five times, and they do not always agree — which is
//! worse than the cost, because the disagreements are invisible. The capsules exist so the
//! answer is established once and *inherited by reference*: a packet names what it
//! inherits, and [`ops::open`] resolves the chain at read time. Nothing is copied, so
//! nothing goes stale independently.
//!
//! # The one rule
//!
//! > Give an agent everything another agent already paid to establish as **fact**. Do not
//! > give it another agent's **judgement** unless its role benefits from that judgement.
//!
//! [`packet::Mode`] is that rule in the type system. `worker` discloses the parent's notes,
//! because a worker continuing the same job should not repeat it. `scout`, `reviewer` and
//! `advisor` withhold them, because an agent hired to look for itself cannot be handed the
//! parent's looking as the definition of the task. Every mode can [`ops::reveal`], and
//! revealing is recorded, so whether a pass was independent stays knowable afterwards.
//!
//! # What is not here
//!
//! AKR owns state, context, packet construction, retrieval, provenance and snapshot
//! verification. It does not launch agents, choose models, price them, or know a harness's
//! spawn mechanism. A packet is provider-neutral, and a data model coupled to this month's
//! agent harness would date within a release.
//!
//! Capsules, packets and results are disposable coordination state. They live in
//! `.agent/handoffs/`, are gitignored, and are invisible to `akr search`, `akr context`
//! and the compiler. What survives them is whatever the work made durable: a record, with
//! evidence, through the ordinary write pipeline.
//!
//! `docs/17-handoff.md`, D-040 and D-041.

pub mod capsule;
pub mod ops;
pub mod packet;
pub mod result;
pub mod session_head;
pub mod snapshot;

pub use session_head::{Handoff, assemble};

use akr_core::json::Value;

/// A command an agent can run, and what it produced when somebody last ran it.
///
/// Shared by capsules, packets and results: "cargo test" is a project fact, "cargo test =
/// 757 passed" is a session fact, and "the chroma benchmark" is a coverage fact, but they
/// are the same shape and splitting them into three types bought nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedCommand {
    /// The exact command line.
    pub command: String,
    /// What running it produced: `pass`, a measurement, or nothing recorded.
    pub result: Option<String>,
}

impl PreparedCommand {
    /// Splits a `command` or `command=result` argument.
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        match raw.split_once('=') {
            Some((command, result)) if !command.trim().is_empty() && !result.trim().is_empty() => {
                Self {
                    command: command.trim().to_owned(),
                    result: Some(result.trim().to_owned()),
                }
            }
            _ => Self {
                command: raw.trim().to_owned(),
                result: None,
            },
        }
    }

    /// One line: the command, and what it gave.
    #[must_use]
    pub fn line(&self) -> String {
        self.result.as_ref().map_or_else(
            || self.command.clone(),
            |r| format!("{}  -> {r}", self.command),
        )
    }

    /// The stored form.
    #[must_use]
    pub fn to_json(&self) -> Value {
        Value::object(vec![
            ("command", Value::string(self.command.clone())),
            (
                "result",
                self.result
                    .as_ref()
                    .map_or(Value::Null, |r| Value::string(r.clone())),
            ),
        ])
    }

    /// Reads a list of them back.
    #[must_use]
    pub fn list_from_json(value: Option<&Value>, field: &str) -> Vec<Self> {
        value
            .and_then(|value| value.get(field))
            .and_then(Value::as_array)
            .unwrap_or_default()
            .iter()
            .map(|entry| Self {
                command: text(entry, "command"),
                result: optional(entry, "result"),
            })
            .collect()
    }
}

/// A JSON array of the given strings.
#[must_use]
pub fn strings(items: &[String]) -> Value {
    Value::array(items.iter().cloned().map(Value::string).collect())
}

/// A JSON array of the given commands.
#[must_use]
pub fn commands_json(items: &[PreparedCommand]) -> Value {
    Value::array(items.iter().map(PreparedCommand::to_json).collect())
}

/// A required string field, empty when absent.
#[must_use]
pub fn text(value: &Value, field: &str) -> String {
    value
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// An optional string field. An empty string reads as absent.
#[must_use]
pub fn optional(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(ToOwned::to_owned)
}

/// A string array nested under `value`, or an empty vector.
#[must_use]
pub fn list(value: Option<&Value>, field: &str) -> Vec<String> {
    value
        .and_then(|value| value.get(field))
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect()
}

/// Renders a labelled block, or nothing when there is nothing to say.
#[must_use]
pub fn block(label: &str, items: &[String], indent: &str) -> String {
    if items.is_empty() {
        return String::new();
    }
    let mut out = format!("{label}\n");
    for item in items {
        out.push_str(&format!("{indent}{item}\n"));
    }
    out
}

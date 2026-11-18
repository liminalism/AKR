//! The error mapping of `docs/08-mcp.md` §5.
//!
//! Every failure carries a coarse **class** alongside the diagnostic array, so an agent can
//! branch on what to do next without knowing the code table. The classes are the actions,
//! not the causes: `conflict` means re-read and retry, `invariant` means stop and ask a
//! human, `environment` means it was never the agent's fault.

use akr_core::json::Value;

/// What an agent should do about a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Fix the call. Never retry unchanged.
    Usage,
    /// The reference is wrong. Search or re-read.
    NotFound,
    /// The proposed content is malformed. Fix and resubmit.
    Schema,
    /// The ledger would become incoherent. Surface it to a human.
    Invariant,
    /// Re-read the head and rebase the edit. Retryable once.
    Conflict,
    /// Not the agent's fault and not fixable by it. Stop and report.
    Environment,
    /// The call succeeded with a caveat that belongs in the agent's report.
    Degraded,
    /// The server contained an unexpected implementation failure. Retry once.
    Internal,
}

impl Class {
    /// The wire name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Usage => "usage",
            Self::NotFound => "not_found",
            Self::Schema => "schema",
            Self::Invariant => "invariant",
            Self::Conflict => "conflict",
            Self::Environment => "environment",
            Self::Degraded => "degraded",
            Self::Internal => "internal",
        }
    }

    /// Whether an agent may retry the same call unchanged.
    ///
    /// Only `conflict` is, and only once: the head moved, so re-reading it and resubmitting
    /// is a different call in every way that matters except the shape.
    #[must_use]
    pub const fn retryable(self) -> bool {
        matches!(self, Self::Conflict | Self::Internal)
    }
}

/// The class a diagnostic code belongs to (§5).
///
/// The explicit rows come first, then the letter ranges. A code that matches neither is
/// `invariant`: the conservative answer, because it tells the agent to stop and ask rather
/// than to retry something the tool did not understand.
#[must_use]
pub fn class_of(code: &str) -> Class {
    match code {
        // Conflict is checked before the `C` range: `AKR-C032` and `AKR-C033` are about a
        // head that moved, not about a malformed invocation.
        "AKR-C032" | "AKR-C033" => Class::Conflict,
        "AKR-C011" | "AKR-C012" | "AKR-C042" | "AKR-C045" | "AKR-C046" | "AKR-G001"
        | "AKR-G003" | "AKR-I003" | "AKR-I031" | "AKR-I032" | "AKR-I022" => Class::Environment,
        "AKR-X041" => Class::Usage,
        "AKR-X099" => Class::Internal,
        "AKR-L001" | "AKR-L004" | "AKR-X001" | "AKR-E003" => Class::NotFound,
        "AKR-L006" | "AKR-L012" | "AKR-L021" | "AKR-L031" => Class::Invariant,
        "AKR-X033" | "AKR-G004" | "AKR-X012" | "AKR-X022" => Class::Degraded,
        _ => match range(code) {
            Some('C') => Class::Usage,
            Some('P' | 'T') => Class::Schema,
            Some('R') => Class::Invariant,
            Some('G' | 'I' | 'M') => Class::Environment,
            _ => Class::Invariant,
        },
    }
}

/// The letter of an `AKR-x###` code.
fn range(code: &str) -> Option<char> {
    code.strip_prefix("AKR-")?.chars().next()
}

/// A tool failure, ready to render as §5's payload.
#[derive(Debug, Clone)]
pub struct ToolError {
    /// The coarse class.
    pub class: Class,
    /// The one-line summary.
    pub summary: String,
    /// The diagnostic array of `docs/07-cli.md` §5.
    pub diagnostics: Vec<Value>,
    /// Whether anything reached the disk. Always `false` for a write.
    pub wrote: bool,
}

impl ToolError {
    /// A failure with one code and one message.
    #[must_use]
    pub fn new(code: &str, summary: impl Into<String>) -> Self {
        let summary = summary.into();
        Self {
            class: class_of(code),
            diagnostics: vec![Value::object(vec![
                ("code", Value::string(code)),
                ("severity", Value::string("error")),
                ("message", Value::string(summary.clone())),
            ])],
            summary,
            wrote: false,
        }
    }

    /// The same, carrying the diagnostics a command already produced.
    #[must_use]
    pub fn with_diagnostics(mut self, diagnostics: Vec<Value>) -> Self {
        if !diagnostics.is_empty() {
            self.diagnostics = diagnostics;
        }
        self
    }

    /// The §5 payload.
    #[must_use]
    pub fn to_json(&self) -> Value {
        Value::object(vec![(
            "error",
            Value::object(vec![
                ("class", Value::string(self.class.name())),
                ("summary", Value::string(self.summary.clone())),
                ("diagnostics", Value::array(self.diagnostics.clone())),
                ("retryable", Value::bool(self.class.retryable())),
                ("wrote", Value::bool(self.wrote)),
            ]),
        )])
    }
}

/// The first error-severity code in a diagnostic array, for classifying a failed command.
#[must_use]
pub fn first_error_code(diagnostics: &[Value]) -> Option<&str> {
    diagnostics
        .iter()
        .find(|d| d.get("severity").and_then(Value::as_str) == Some("error"))
        .or_else(|| diagnostics.first())
        .and_then(|d| d.get("code"))
        .and_then(Value::as_str)
}

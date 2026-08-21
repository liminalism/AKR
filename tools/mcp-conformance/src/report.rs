//! What one client generation observed, in terms no SDK owns.
//!
//! Each generation drives a different, mutually incompatible SDK; the assertions must not.
//! Every adapter reduces what it saw to the types here — strings and `serde_json::Value` —
//! and `checks.rs` judges those. A generation that predates a concept records
//! `Unsupported` rather than failing, so "rmcp 1.x has no discovery" reads as an absence
//! and not as a defect.

use serde_json::Value;

/// One conformance question and how it came out.
pub struct Check {
    pub name: String,
    pub outcome: Outcome,
}

pub enum Outcome {
    Pass,
    Fail(String),
    /// This generation has no concept of the thing being asked about.
    NotApplicable(String),
}

impl Check {
    pub fn pass(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            outcome: Outcome::Pass,
        }
    }
    pub fn fail(name: impl Into<String>, why: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            outcome: Outcome::Fail(why.into()),
        }
    }
    pub fn skip(name: impl Into<String>, why: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            outcome: Outcome::NotApplicable(why.into()),
        }
    }
    pub fn failed(&self) -> bool {
        matches!(self.outcome, Outcome::Fail(_))
    }
}

/// Why something was not observed.
pub enum NotObserved {
    /// The SDK predates the concept entirely.
    Unsupported(&'static str),
    /// The SDK has the concept and the attempt failed.
    Failed(String),
}

/// One `tools/call`, as the client saw it.
pub struct CallOutcome {
    pub tool: String,
    /// `Err` means the SDK could not deserialise the response at all. That is the failure
    /// this whole tool exists to catch: the server answered successfully and the client
    /// still could not read it.
    pub result: Result<CallOk, String>,
}

pub struct CallOk {
    pub is_error: Option<bool>,
    pub content_blocks: usize,
    pub structured: Option<Value>,
}

/// Everything one generation saw.
pub struct Observations {
    pub generation: &'static str,
    /// Set when the connection itself failed; nothing else could then be observed.
    pub fatal: Option<String>,
    /// Requested protocol version, and what the server negotiated in return.
    pub negotiated: Vec<(String, Result<String, String>)>,
    /// `supportedVersions` from `server/discover`.
    pub discovery: Result<Vec<String>, NotObserved>,
    /// Whether the server was reachable through the discovery-first lifecycle.
    pub discovery_lifecycle: Result<String, NotObserved>,
    /// Every tool the server listed, with the `outputSchema` it declared for it.
    pub tools: Vec<(String, Option<Value>)>,
    /// One entry per `tools/call` attempted.
    pub calls: Vec<CallOutcome>,
}

impl Observations {
    pub fn fatal(generation: &'static str, why: impl Into<String>) -> Self {
        Self {
            generation,
            fatal: Some(why.into()),
            negotiated: Vec::new(),
            discovery: Err(NotObserved::Failed("connection failed".into())),
            discovery_lifecycle: Err(NotObserved::Failed("connection failed".into())),
            tools: Vec::new(),
            calls: Vec::new(),
        }
    }
}

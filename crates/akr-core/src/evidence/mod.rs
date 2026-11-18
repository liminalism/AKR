//! Recording what was observed, and attaching it to an acceptance check.
//!
//! The library operation behind `akr evidence add` (`docs/07-cli.md`) and behind the
//! `--check` argument of `akr complete`.
//!
//! # The write pipeline
//!
//! Every write in AKR goes parse → apply → **validate the resulting ledger** → canonically
//! format → write, and fails completely rather than partially (`docs/07-cli.md` §4). This
//! module implements the middle three: it applies a change to a cloned ledger, validates
//! the *result*, and hands back either the new records or the diagnostics that stopped
//! them. Nothing here touches the filesystem — serialising and writing is the caller's,
//! which keeps the atomicity guarantee in one place.
//!
//! # Evidence never declares what it verifies
//!
//! [`AddEvidence`] has no field for "what this proves", and that absence is the point
//! (D-016). The `verified_by` link is authored on the **check**, and [`attach`] is the
//! only way to create it. A two-directional link would be two sources of truth and a
//! reconciliation rule nobody wants to write.

use crate::diagnostics::{Diagnostic, Severity};
use crate::model::{
    Acceptance, Check, CheckMethod, Commit, ContentSlot, ContentValue, Date, EvidenceResult, Kind,
    Ledger, LogicalKey, Record, Reference, RevisionId, Segment, State,
};
use crate::validate::validate_all;
use std::collections::BTreeMap;

/// Why an evidence operation could not be performed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidenceError {
    /// The key already exists; evidence is added, never overwritten.
    KeyExists(LogicalKey),
    /// The record the check belongs to does not exist.
    UnknownOwner(LogicalKey),
    /// The owner has no acceptance block, or no check with that identifier.
    UnknownCheck {
        /// The owning record.
        owner: RevisionId,
        /// The check identifier that was not found.
        check: String,
    },
    /// The evidence record named does not exist.
    UnknownEvidence(Reference),
    /// The reference names a record that is not an `evidence` record (V-005).
    NotEvidence {
        /// What was named.
        reference: Reference,
        /// What kind it turned out to be.
        kind: Kind,
    },
    /// The write would have left the ledger invalid, so nothing was written
    /// (`AKR-C031`).
    WouldNotValidate(Vec<Diagnostic>),
}

impl std::fmt::Display for EvidenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::KeyExists(key) => write!(f, "{key} already exists; use `akr revise`"),
            Self::UnknownOwner(key) => write!(f, "no record with key {key}"),
            Self::UnknownCheck { owner, check } => {
                write!(f, "{owner} has no acceptance check `{check}`")
            }
            Self::UnknownEvidence(reference) => write!(f, "{reference} does not resolve"),
            Self::NotEvidence { reference, kind } => {
                write!(f, "{reference} is a {kind}, not an evidence record")
            }
            Self::WouldNotValidate(diagnostics) => write!(
                f,
                "write aborted: the resulting ledger did not validate ({} diagnostics); \
                 nothing was written",
                diagnostics.len()
            ),
        }
    }
}

impl std::error::Error for EvidenceError {}

/// What to record about a check that was actually run.
///
/// Note what is absent: there is no field naming what this evidence proves. See the
/// module documentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddEvidence {
    /// The key for the new record.
    pub key: LogicalKey,
    /// The one-line label; every generated view's heading for it.
    pub title: String,
    /// Pass, fail, or inconclusive. A `fail` is a legitimate and valuable record.
    pub result: EvidenceResult,
    /// How the check was carried out.
    pub method: CheckMethod,
    /// The commit the check was run at. `akr evidence add` defaults this to HEAD.
    pub observed_at: Commit,
    /// The exact command, where there was one.
    pub command: Option<String>,
    /// A recorded artefact — a log, a capture, a report.
    pub artifact: Option<String>,
    /// What was seen.
    pub summary: Option<String>,
    /// Who ran it.
    pub author: Option<String>,
    /// The authoring date.
    pub created_at: Option<Date>,
    /// The file the record is written to; carried so V-003 can be checked.
    pub file: Option<String>,
}

impl AddEvidence {
    /// A minimal request: a key, a title, a result, a method, and a commit.
    #[must_use]
    pub fn new(
        key: LogicalKey,
        title: impl Into<String>,
        result: EvidenceResult,
        method: CheckMethod,
        observed_at: Commit,
    ) -> Self {
        Self {
            key,
            title: title.into(),
            result,
            method,
            observed_at,
            command: None,
            artifact: None,
            summary: None,
            author: None,
            created_at: None,
            file: None,
        }
    }

    /// Sets the command that was run.
    #[must_use]
    pub fn command(mut self, command: impl Into<String>) -> Self {
        self.command = Some(command.into());
        self
    }

    /// Sets the recorded artefact.
    #[must_use]
    pub fn artifact(mut self, artifact: impl Into<String>) -> Self {
        self.artifact = Some(artifact.into());
        self
    }

    /// Sets the summary prose.
    #[must_use]
    pub fn summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }

    /// Sets the file the record will be written to.
    #[must_use]
    pub fn file(mut self, file: impl Into<String>) -> Self {
        self.file = Some(file.into());
        self
    }

    /// Builds the record this request describes, without validating anything.
    #[must_use]
    pub fn to_record(&self) -> Record {
        let mut content: BTreeMap<ContentSlot, ContentValue> = BTreeMap::new();
        content.insert(
            ContentSlot::Result,
            ContentValue::Enum(
                Segment::new(self.result.name()).expect("a result name is a valid segment"),
            ),
        );
        content.insert(
            ContentSlot::Method,
            ContentValue::Enum(
                Segment::new(self.method.name()).expect("a method name is a valid segment"),
            ),
        );
        content.insert(
            ContentSlot::ObservedAt,
            ContentValue::Commit(self.observed_at.clone()),
        );
        if let Some(command) = &self.command {
            content.insert(ContentSlot::Command, ContentValue::Text(command.clone()));
        }
        if let Some(artifact) = &self.artifact {
            content.insert(ContentSlot::Artifact, ContentValue::Text(artifact.clone()));
        }
        if let Some(summary) = &self.summary {
            content.insert(ContentSlot::Summary, ContentValue::Prose(summary.clone()));
        }

        Record {
            id: RevisionId::new(self.key.clone(), 1),
            kind: Kind::Evidence,
            title: self.title.clone(),
            // Empirical kinds have no proposal state: an observation either was made or
            // was not (`spec/tables/vocabulary.json`).
            state: State::Verified,
            scope: Vec::new(),
            topic: None,
            content,
            claims: Vec::new(),
            retired_claims: Vec::new(),
            acceptance: None,
            dispositions: Vec::new(),
            relations: BTreeMap::new(),
            acknowledged: false,
            author: self.author.clone(),
            created_at: self.created_at,
            sources: Vec::new(),
            file: self.file.clone(),
        }
    }
}

/// What a successful write produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    /// The ledger with the change applied.
    pub ledger: Ledger,
    /// The revisions this operation created or replaced.
    pub touched: Vec<RevisionId>,
}

/// Adds an evidence record.
///
/// Validates the **resulting** ledger, not the change: adding a record that breaks an
/// invariant elsewhere fails, even though the record itself is well formed.
///
/// # Errors
/// [`EvidenceError::KeyExists`] when the key is taken — evidence is added, never
/// overwritten — and [`EvidenceError::WouldNotValidate`] when the result would be
/// invalid, in which case nothing is applied.
pub fn add(ledger: &Ledger, request: &AddEvidence) -> Result<Written, EvidenceError> {
    if !ledger.revisions_of(&request.key).is_empty() {
        return Err(EvidenceError::KeyExists(request.key.clone()));
    }
    let record = request.to_record();
    let id = record.id.clone();

    let mut next = ledger.clone();
    next.insert(record);
    settle(next, vec![id])
}

/// Attaches an evidence record to an acceptance check, by adding a `verified_by`
/// reference to the check.
///
/// The link runs in exactly one direction: from the thing being verified to the evidence
/// (D-016). This function edits the **check**, never the evidence record.
///
/// The reference is written **pinned** when `pin` is set, which is the recommended form
/// for citing evidence (D-009 guidance: "pin when citing evidence or narrating history").
/// A floating citation would silently re-point if the evidence key ever gained a new
/// revision, which is precisely what a completed acceptance check must not do.
///
/// # Errors
/// [`EvidenceError::UnknownOwner`], [`EvidenceError::UnknownCheck`],
/// [`EvidenceError::UnknownEvidence`], [`EvidenceError::NotEvidence`], or
/// [`EvidenceError::WouldNotValidate`].
pub fn attach(
    ledger: &Ledger,
    owner: &LogicalKey,
    check_id: &str,
    evidence: &Reference,
    pin: bool,
) -> Result<Written, EvidenceError> {
    let head = ledger
        .head(owner)
        .map_err(|_| EvidenceError::UnknownOwner(owner.clone()))?;
    let owner_id = head.id.clone();

    let target = ledger
        .resolve(evidence)
        .ok()
        .flatten()
        .ok_or_else(|| EvidenceError::UnknownEvidence(evidence.clone()))?;
    if target.kind != Kind::Evidence {
        return Err(EvidenceError::NotEvidence {
            reference: evidence.clone(),
            kind: target.kind,
        });
    }
    let citation = if pin {
        Reference::pinned(target.id.key.clone(), target.id.revision)
    } else {
        Reference::head(target.id.key.clone())
    };

    let Some(acceptance) = &head.acceptance else {
        return Err(EvidenceError::UnknownCheck {
            owner: owner_id,
            check: check_id.to_owned(),
        });
    };
    if !acceptance.checks.iter().any(|c| c.id.as_str() == check_id) {
        return Err(EvidenceError::UnknownCheck {
            owner: owner_id,
            check: check_id.to_owned(),
        });
    }

    let mut next = Ledger::new(ledger.project.clone());
    next.facts = ledger.facts.clone();
    for record in ledger.records() {
        let mut copy = record.clone();
        if copy.id == owner_id
            && let Some(acceptance) = &mut copy.acceptance
        {
            for check in &mut acceptance.checks {
                if check.id.as_str() == check_id && !check.verified_by.contains(&citation) {
                    check.verified_by.push(citation.clone());
                    // Canonical order within a relation array: by key, then revision,
                    // then anchor (D-012).
                    check.verified_by.sort();
                }
            }
        }
        next.insert(copy);
    }
    settle(next, vec![owner_id])
}

/// Adds an evidence record and attaches it to a check in one operation, which is what
/// `akr evidence add --check <id>` does.
///
/// The two halves are applied together and validated once, so a request that would leave
/// an orphan evidence record attached to nothing fails whole.
///
/// # Errors
/// Any error from [`add`] or [`attach`].
pub fn add_and_attach(
    ledger: &Ledger,
    request: &AddEvidence,
    owner: &LogicalKey,
    check_id: &str,
) -> Result<Written, EvidenceError> {
    // Apply the addition without validating: an evidence record cited by nothing is
    // legal, but validating twice would report the same intermediate state twice.
    if !ledger.revisions_of(&request.key).is_empty() {
        return Err(EvidenceError::KeyExists(request.key.clone()));
    }
    let record = request.to_record();
    let evidence_id = record.id.clone();
    let mut staged = ledger.clone();
    staged.insert(record);

    let reference = Reference::pinned(evidence_id.key.clone(), evidence_id.revision);
    let mut written = attach(&staged, owner, check_id, &reference, true)?;
    written.touched.push(evidence_id);
    written.touched.sort();
    Ok(written)
}

/// Validates a candidate ledger and either accepts it or reports why not.
///
/// Warnings are left to the caller's profile: applying `--strict` is the CLI's job, not
/// this crate's (D-013). Only errors abort a write here.
fn settle(candidate: Ledger, touched: Vec<RevisionId>) -> Result<Written, EvidenceError> {
    let diagnostics: Vec<Diagnostic> = validate_all(&candidate)
        .into_iter()
        .filter(|d| d.severity == Severity::Error)
        .collect();
    if diagnostics.is_empty() {
        Ok(Written {
            ledger: candidate,
            touched,
        })
    } else {
        Err(EvidenceError::WouldNotValidate(diagnostics))
    }
}

/// Builds an acceptance block from check identifiers, for tests and for `akr propose`.
#[must_use]
pub fn acceptance_of(checks: &[(&str, CheckMethod)]) -> Acceptance {
    Acceptance {
        checks: checks
            .iter()
            .map(|(id, method)| Check {
                id: Segment::new(id).expect("a valid check identifier"),
                statement: format!("check {id}"),
                method: *method,
                command: None,
                verified_by: Vec::new(),
                passed_at: None,
            })
            .collect(),
    }
}

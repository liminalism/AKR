//! Logging a papercut: a small friction hit while working, recorded in the moment
//! (D-027).
//!
//! The library operation behind `akr papercut` and the `knowledge.papercut` MCP tool.
//! Like [`crate::evidence`], this module builds the record; serialising and writing is
//! the caller's, through the one write pipeline of `docs/07-cli.md` §4.
//!
//! # Zero ceremony, by construction
//!
//! A papercut costs one message. Everything else — the key, the slug, the commit, the
//! author, the date — is filled in here, because a log that asks for ceremony does not
//! get written in the moment, and a papercut written later is a papercut forgotten.

use crate::import::slug_of;
use crate::model::{
    Commit, ContentSlot, ContentValue, Date, Kind, Ledger, LogicalKey, Record, RevisionId, State,
};
use std::collections::BTreeMap;

pub mod collate;

/// What to log. The message is the only thing the caller has to say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogPapercut {
    /// One or two sentences: what you were doing, what got in the way, and — as a
    /// bonus — a guess at the cause or fix.
    pub message: String,
    /// Who hit it: a model or harness name. Lands in the `author` slot.
    pub agent: String,
    /// The commit it happened at. The tooling defaults this to HEAD.
    pub observed_at: Commit,
    /// The authoring date. The tooling fills this from `--today` or the system date.
    pub created_at: Option<Date>,
    /// What the friction was *with*, when that is not this project (D-033).
    ///
    /// Papercuts are the one record kind whose subject is sometimes the tool rather than
    /// the project being worked on: an agent in `jpegxl-rs` hits an AKR bug and has
    /// nowhere but `jpegxl-rs`'s ledger to put it. `about` is that distinction, written
    /// down — `--about akr` — so `akr papercut collate` can find it instead of somebody
    /// having to think to go and read a sibling's ledger.
    pub about: Option<String>,
}

/// Why a papercut key could not be allocated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PapercutKeyError {
    /// The requested namespace is not declared in the project.
    UnknownNamespace(String),
    /// The project declares no namespaces at all.
    NoNamespaces,
}

impl std::fmt::Display for PapercutKeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownNamespace(name) => {
                write!(f, "namespace {name:?} is not declared in .akr/project.akr")
            }
            Self::NoNamespaces => write!(f, "the project declares no namespaces"),
        }
    }
}

impl std::error::Error for PapercutKeyError {}

/// Allocates the key for a new papercut: `<namespace>.papercut.<slug-of-message>`,
/// suffixed `-2`, `-3`, … until it collides with nothing in the ledger.
///
/// With no namespace given, the namespace this project's papercuts already go to is
/// used; see [`select_namespace`] for the rest of the rule.
///
/// # Errors
/// [`PapercutKeyError`] when the namespace cannot be determined.
pub fn allocate_key(
    ledger: &Ledger,
    namespace: Option<&str>,
    message: &str,
) -> Result<LogicalKey, PapercutKeyError> {
    let namespace = select_namespace(ledger, namespace)?;

    let base = {
        let slug = slug_of(message);
        if slug.is_empty() {
            "papercut".to_owned()
        } else {
            slug
        }
    };
    let taken: std::collections::BTreeSet<String> = ledger
        .records()
        .iter()
        .map(|record| record.id.key.to_string())
        .collect();
    let mut candidate = format!("{namespace}.papercut.{base}");
    let mut n = 1usize;
    while taken.contains(&candidate) {
        n += 1;
        candidate = format!("{namespace}.papercut.{base}-{n}");
    }
    Ok(LogicalKey::parse(&candidate).expect("namespace and slug are valid segments"))
}

/// Resolves the namespace used by zero-ceremony papercut commands.
///
/// This is shared by logging and collation: a default collation is about the namespace
/// it will write into, while `--all` is the explicit opt-in to unrelated sister-project
/// papercuts.
pub fn select_namespace(
    ledger: &Ledger,
    namespace: Option<&str>,
) -> Result<String, PapercutKeyError> {
    let declared: Vec<String> = ledger
        .project
        .namespaces
        .iter()
        .map(ToString::to_string)
        .collect();
    match namespace {
        Some(name) => {
            if !declared.iter().any(|d| d == name) {
                return Err(PapercutKeyError::UnknownNamespace(name.to_owned()));
            }
            Ok(name.to_owned())
        }
        // A default, not an ambiguity. D-027 puts the whole ceremony of a papercut in
        // one call, and a workspace with several namespaces was the one shape where that
        // was untrue: the call was refused, the agent had to go and read project.akr for
        // a name the ledger already knew, and retry — twice over from Lege-ecosystem,
        // whose second namespace is a performance sub-ledger
        // (`akr.papercut.collated-19-papercuts-from-evidence-intake`).
        //
        // The rule is: where papercuts already go, then where the project's records
        // already are, then alphabetically. `Project::namespaces` is a set, so
        // declaration order is not something the model keeps and "the first declared"
        // is not available to be the default — but neither of those tie-breaks needs it.
        // The first says a workspace that has logged papercuts keeps logging them in the
        // same place. The second decides the first papercut of a workspace that has not:
        // the namespace carrying the most records is the project-wide one in every
        // multi-namespace ledger to hand, which is where a papercut about the project
        // belongs. `--namespace` says otherwise in every case, and nothing about a
        // papercut depends on the choice — it has no scope, no topic, no watches, and
        // never goes stale.
        None => {
            let mut papercuts: BTreeMap<String, usize> = BTreeMap::new();
            let mut records: BTreeMap<String, usize> = BTreeMap::new();
            for record in ledger.records() {
                let namespace = record.id.key.namespace().to_string();
                if record.kind == Kind::Papercut {
                    *papercuts.entry(namespace.clone()).or_default() += 1;
                }
                *records.entry(namespace).or_default() += 1;
            }
            let rank = |name: &String| {
                (
                    papercuts.get(name).copied().unwrap_or(0),
                    records.get(name).copied().unwrap_or(0),
                )
            };
            // `declared` is sorted, so keeping the first strict maximum breaks a tie
            // alphabetically.
            declared
                .iter()
                .fold(None::<((usize, usize), &String)>, |best, name| {
                    let score = rank(name);
                    match best {
                        Some((seen, _)) if seen >= score => best,
                        _ => Some((score, name)),
                    }
                })
                .map(|(_, name)| name.clone())
                .ok_or(PapercutKeyError::NoNamespaces)
        }
    }
}

impl LogPapercut {
    /// Builds the record this request describes, without validating anything.
    #[must_use]
    pub fn to_record(&self, key: LogicalKey) -> Record {
        let mut content: BTreeMap<ContentSlot, ContentValue> = BTreeMap::new();
        content.insert(
            ContentSlot::Statement,
            ContentValue::Prose(self.message.clone()),
        );
        content.insert(
            ContentSlot::ObservedAt,
            ContentValue::Commit(self.observed_at.clone()),
        );
        if let Some(about) = &self.about {
            content.insert(ContentSlot::About, ContentValue::Text(about.clone()));
        }

        Record {
            id: RevisionId::new(key, 1),
            kind: Kind::Papercut,
            title: title_of(&self.message),
            // Empirical kinds have no proposal state: the friction either was hit or
            // was not.
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
            author: Some(self.agent.clone()),
            created_at: self.created_at,
            sources: Vec::new(),
            file: None,
        }
    }
}

/// The message as a title: its first line, ellipsized near 72 bytes on a word break.
fn title_of(message: &str) -> String {
    let line = message.lines().next().unwrap_or("").trim();
    if line.len() <= 72 {
        return line.to_owned();
    }
    let cut = line[..72].rfind(' ').unwrap_or(72);
    format!("{}…", line[..cut].trim_end())
}

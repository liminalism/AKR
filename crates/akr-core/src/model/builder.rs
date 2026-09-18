//! Test-facing builders: construct records with no text format in sight.
//!
//! These are the P1 answer to "how do you test a model before a parser exists". They
//! panic on malformed input rather than returning results, because every call site is a
//! test or a fixture with a literal in it, and `?` on a literal is noise.

use super::ident::{Commit, Date, Glob, LogicalKey, Segment};
use super::kind::{ContentSlot, Kind};
use super::record::{
    Acceptance, Check, CheckMethod, Claim, ContentValue, Disposition, Outcome, Record, Source,
    SourceKind,
};
use super::refs::{Reference, RevisionId};
use super::relation::Relation;
use super::scope::ScopeTerm;
use super::state::State;
use std::collections::BTreeMap;

/// Parses a key.
///
/// # Panics
/// Panics if the key is malformed.
#[must_use]
pub fn key(text: &str) -> LogicalKey {
    LogicalKey::parse(text).expect("valid key")
}

/// Parses a reference in any of the four forms.
///
/// # Panics
/// Panics if the reference is malformed.
#[must_use]
pub fn reference(text: &str) -> Reference {
    Reference::parse(text).expect("valid reference")
}

/// Builds a [`Record`] fluently.
#[derive(Debug, Clone)]
pub struct RecordBuilder {
    record: Record,
}

impl RecordBuilder {
    /// Starts a record with the given key, revision and kind.
    ///
    /// The state defaults to the first initial state of the kind's class, so that the
    /// common case needs no `.state()` call.
    ///
    /// # Panics
    /// Panics if the key is malformed.
    #[must_use]
    pub fn new(key_text: &str, revision: u32, kind: Kind) -> Self {
        Self {
            record: Record {
                id: RevisionId::new(key(key_text), revision),
                kind,
                title: format!("{kind} {key_text}"),
                state: kind.class().initial()[0],
                scope: Vec::new(),
                topic: None,
                content: BTreeMap::new(),
                claims: Vec::new(),
                retired_claims: Vec::new(),
                acceptance: None,
                dispositions: Vec::new(),
                relations: BTreeMap::new(),
                acknowledged: false,
                author: None,
                created_at: None,
                sources: Vec::new(),
                file: None,
            },
        }
    }

    /// Sets the title.
    #[must_use]
    pub fn title(mut self, title: &str) -> Self {
        self.record.title = title.to_owned();
        self
    }

    /// Sets the lifecycle state.
    #[must_use]
    pub fn state(mut self, state: State) -> Self {
        self.record.state = state;
        self
    }

    /// Sets the scope.
    #[must_use]
    pub fn scope(mut self, scope: Vec<ScopeTerm>) -> Self {
        self.record.scope = scope;
        self
    }

    /// Adds a `path` scope term.
    #[must_use]
    pub fn path_scope(mut self, glob: &str) -> Self {
        self.record.scope.push(ScopeTerm::Path(Glob::new(glob)));
        self
    }

    /// Sets the scope to `[ all ]`.
    #[must_use]
    pub fn all_scope(mut self) -> Self {
        self.record.scope = vec![ScopeTerm::All];
        self
    }

    /// Sets the `topic`.
    ///
    /// # Panics
    /// Panics if the topic is not a valid segment.
    #[must_use]
    pub fn topic(mut self, topic: &str) -> Self {
        self.record.topic = Some(Segment::new(topic).expect("valid topic"));
        self
    }

    /// Sets a content slot to an arbitrary value.
    #[must_use]
    pub fn content(mut self, slot: ContentSlot, value: ContentValue) -> Self {
        self.record.content.insert(slot, value);
        self
    }

    /// Sets a content slot to a prose value.
    #[must_use]
    pub fn prose(self, slot: ContentSlot, text: &str) -> Self {
        self.content(slot, ContentValue::prose(text))
    }

    /// Sets a content slot to a commit.
    ///
    /// # Panics
    /// Panics if the commit is not 40 lowercase hex digits.
    #[must_use]
    pub fn commit(self, slot: ContentSlot, hash: &str) -> Self {
        self.content(
            slot,
            ContentValue::Commit(Commit::new(hash).expect("valid commit")),
        )
    }

    /// Sets a content slot to an enum member.
    ///
    /// # Panics
    /// Panics if the member is not a valid segment.
    #[must_use]
    pub fn enum_value(self, slot: ContentSlot, member: &str) -> Self {
        self.content(
            slot,
            ContentValue::Enum(Segment::new(member).expect("valid enum member")),
        )
    }

    /// Fills every required content slot for the kind with placeholder values, so that a
    /// test exercising one rule does not trip V-008 by accident.
    #[must_use]
    pub fn filled(mut self) -> Self {
        for spec in self.record.kind.content_slots() {
            if !spec.required || self.record.content.contains_key(&spec.slot) {
                continue;
            }
            let value = match spec.slot {
                ContentSlot::ObservedAt | ContentSlot::AsOf => ContentValue::Commit(
                    Commit::new("3f0a1c9d5b7e2648a0d4f1b8c36e9752ad014b6f").expect("valid commit"),
                ),
                ContentSlot::Result => {
                    ContentValue::Enum(Segment::new("pass").expect("valid segment"))
                }
                ContentSlot::Method => {
                    ContentValue::Enum(Segment::new("command").expect("valid segment"))
                }
                ContentSlot::Target | ContentSlot::ReviewAfter | ContentSlot::ClosedAt => {
                    ContentValue::Date(Date::new(2026, 1, 1).expect("valid date"))
                }
                _ => ContentValue::prose("placeholder"),
            };
            self.record.content.insert(spec.slot, value);
        }
        if self.record.kind.class().scope_required() && self.record.scope.is_empty() {
            self.record.scope.push(ScopeTerm::All);
        }
        if self.record.kind.requires_acceptance() && self.record.acceptance.is_none() {
            self.record.acceptance = Some(Acceptance {
                checks: vec![Check {
                    id: Segment::new("placeholder").expect("valid segment"),
                    statement: "placeholder".to_owned(),
                    method: CheckMethod::Manual,
                    command: None,
                    verified_by: Vec::new(),
                }],
            });
        }
        self
    }

    /// Adds a claim.
    ///
    /// # Panics
    /// Panics if the anchor is not a valid segment.
    #[must_use]
    pub fn claim(mut self, anchor: &str, text: &str) -> Self {
        self.record.claims.push(Claim {
            anchor: Segment::new(anchor).expect("valid anchor"),
            text: text.to_owned(),
            supported_by: Vec::new(),
        });
        self
    }

    /// Marks anchors as retired by this revision.
    ///
    /// # Panics
    /// Panics if an anchor is not a valid segment.
    #[must_use]
    pub fn retires(mut self, anchors: &[&str]) -> Self {
        self.record.retired_claims.extend(
            anchors
                .iter()
                .map(|a| Segment::new(a).expect("valid anchor")),
        );
        self
    }

    /// Adds a relation target.
    ///
    /// # Panics
    /// Panics if the reference is malformed.
    #[must_use]
    pub fn rel(mut self, relation: Relation, target: &str) -> Self {
        self.record
            .relations
            .entry(relation)
            .or_default()
            .push(reference(target));
        self
    }

    /// Adds an acceptance check.
    ///
    /// # Panics
    /// Panics if the identifier is not a valid segment, or a reference is malformed.
    #[must_use]
    pub fn check(mut self, id: &str, method: CheckMethod, verified_by: &[&str]) -> Self {
        let acceptance = self
            .record
            .acceptance
            .get_or_insert_with(Acceptance::default);
        acceptance.checks.push(Check {
            id: Segment::new(id).expect("valid check id"),
            statement: format!("check {id}"),
            method,
            command: None,
            verified_by: verified_by.iter().map(|r| reference(r)).collect(),
        });
        self
    }

    /// Adds a disposition.
    ///
    /// # Panics
    /// Panics if a reference is malformed.
    #[must_use]
    pub fn disposition(mut self, target: &str, outcome: Outcome, into: Option<&str>) -> Self {
        self.record.dispositions.push(Disposition {
            target: reference(target),
            outcome,
            into: into.map(reference),
            note: None,
        });
        self
    }

    /// Adds a `source` block.
    #[must_use]
    pub fn source(mut self, kind: SourceKind, path: Option<&str>) -> Self {
        self.record.sources.push(Source {
            kind,
            role: None,
            path: path.map(ToOwned::to_owned),
            url: None,
            excerpt: None,
            document: None,
            range: None,
            use_note: None,
        });
        self
    }

    /// Sets the acknowledged flag.
    #[must_use]
    pub fn acknowledged(mut self, acknowledged: bool) -> Self {
        self.record.acknowledged = acknowledged;
        self
    }

    /// Records which file the record came from, for V-003.
    #[must_use]
    pub fn file(mut self, path: &str) -> Self {
        self.record.file = Some(path.to_owned());
        self
    }

    /// Finishes the record.
    #[must_use]
    pub fn build(self) -> Record {
        self.record
    }
}

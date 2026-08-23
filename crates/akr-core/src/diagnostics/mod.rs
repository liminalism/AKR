//! Diagnostic codes, severities, and the diagnostic type.
//!
//! # Designed for spans that do not exist yet
//!
//! P1 produces diagnostics with no source spans, because there are no source files. Every
//! diagnostic instead carries a [`Subject`]: the record, slot, key, or file it is about.
//! P2 attaches spans by mapping subjects to byte ranges and filling [`Label::span`]. No
//! signature changes, and no consumer written against this type breaks.
//!
//! Rendering (the caret form of `spec/diagnostics/README.md` §5) is P2's job.

mod render;

pub use render::{SourceFile, SourceMap, render};

use crate::model::{ContentSlot, LogicalKey, Reference, Relation, RevisionId, Segment};
use std::fmt;

/// A diagnostic code such as `AKR-R014`.
///
/// Codes are defined once in `spec/diagnostics/codes-lang.md`, never renumbered and never
/// reused (D-013). The constants this crate raises live in [`codes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Code(&'static str);

impl Code {
    /// Wraps a static code string. Not validated; use the constants in [`codes`].
    #[must_use]
    pub const fn new(code: &'static str) -> Self {
        Self(code)
    }

    /// The code as written, for example `"AKR-R014"`.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        self.0
    }

    /// The pipeline stage that raises this code, from its stage letter.
    ///
    /// Returns `None` for a code that does not match the `AKR-<letter><nnn>` scheme.
    #[must_use]
    pub fn stage(&self) -> Option<Stage> {
        Stage::from_letter(self.0.as_bytes().get(4).copied()? as char)
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// A pipeline stage, identified by the letter in a diagnostic code (D-013).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Stage {
    /// `P` — lexing, grammar, literal forms.
    Parse,
    /// `F` — canonicalisation.
    Format,
    /// `T` — kind schema, required slots, enum values.
    Type,
    /// `L` — reference, anchor, namespace and kind-correctness resolution.
    Link,
    /// `R` — heads, graphs, acceptance, sealing, contradictions.
    Resolve,
    /// `I` — index construction.
    Index,
    /// `E` — projections and view freshness.
    Emit,
    /// `X` — context assembly and search.
    Context,
    /// `G` — git freshness and impact.
    Git,
    /// `C` — invocation and workspace configuration.
    Cli,
    /// `M` — legacy import.
    Migration,
}

impl Stage {
    /// Maps a stage letter to its stage.
    #[must_use]
    pub fn from_letter(c: char) -> Option<Self> {
        Some(match c {
            'P' => Self::Parse,
            'F' => Self::Format,
            'T' => Self::Type,
            'L' => Self::Link,
            'R' => Self::Resolve,
            'I' => Self::Index,
            'E' => Self::Emit,
            'X' => Self::Context,
            'G' => Self::Git,
            'C' => Self::Cli,
            'M' => Self::Migration,
            _ => return None,
        })
    }
}

/// Diagnostic severity. Two levels and no third (D-013).
///
/// Under the default `--strict` profile a warning fails the build; applying that profile
/// is the caller's job, not this crate's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// The ledger is not well formed, or a stated invariant is broken.
    Error,
    /// The ledger builds, but something is very likely wrong.
    Warning,
}

/// A validation rule identifier, rendered `V-001`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuleId(pub u16);

impl fmt::Display for RuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "V-{:03}", self.0)
    }
}

/// A source file, assigned by whatever loaded the ledger. Unused in P1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileId(pub u32);

/// A byte range in a source file. Produced by P2; always `None` in P1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Span {
    /// Which file.
    pub file: FileId,
    /// Start byte offset, inclusive.
    pub start: u32,
    /// End byte offset, exclusive.
    pub end: u32,
}

/// Which part of a record a label is about.
///
/// This is what lets P1 produce useful diagnostics with no spans, and what P2 uses to
/// find the span to attach.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SlotRef {
    /// The `title` slot.
    Title,
    /// The `state` slot.
    State,
    /// The `scope` slot.
    Scope,
    /// The `topic` slot.
    Topic,
    /// A kind-specific content slot.
    Content(ContentSlot),
    /// A relation slot.
    Relation(Relation),
    /// A `claim` block, by anchor.
    Claim(Segment),
    /// The `acceptance` block.
    Acceptance,
    /// A `check` block, by identifier.
    Check(Segment),
    /// A `disposition` block, by the reference in its head.
    Disposition(Reference),
    /// The `retired_claims` slot.
    RetiredClaims,
    /// A `source` block, by position.
    Source(usize),
}

/// What a [`Label`] points at.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Subject {
    /// A whole record revision.
    Revision(RevisionId),
    /// One slot or block of a record revision.
    Slot(RevisionId, SlotRef),
    /// A logical key, when the fault is about the key rather than one revision.
    Key(LogicalKey),
    /// A source file.
    File(String),
    /// The ledger as a whole.
    Ledger,
}

/// One pointer within a diagnostic: a subject, an optional span, and an optional note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    /// What this label is about.
    pub subject: Subject,
    /// Where it is in source. `None` until P2 fills it in.
    pub span: Option<Span>,
    /// The text rendered beside the caret.
    pub message: Option<String>,
}

impl Label {
    /// A label with no span and no message.
    #[must_use]
    pub fn new(subject: Subject) -> Self {
        Self {
            subject,
            span: None,
            message: None,
        }
    }

    /// A label with a message.
    #[must_use]
    pub fn with_message(subject: Subject, message: impl Into<String>) -> Self {
        Self {
            subject,
            span: None,
            message: Some(message.into()),
        }
    }
}

/// A single diagnostic.
///
/// Ordering is by `(code, primary subject)`, which is what makes
/// [`crate::validate::validate_all`] deterministic before spans exist. P2 re-sorts by
/// span once files are known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// The code, from `spec/diagnostics/codes-lang.md`.
    pub code: Code,
    /// Error or warning.
    pub severity: Severity,
    /// The rule that raised it, where one did.
    pub rule: Option<RuleId>,
    /// The one-line message.
    pub message: String,
    /// What the diagnostic is primarily about.
    pub primary: Label,
    /// Secondary locations.
    pub notes: Vec<Label>,
    /// The `help:` line.
    pub help: Option<String>,
}

impl Diagnostic {
    /// An error diagnostic.
    #[must_use]
    pub fn error(code: Code, rule: RuleId, subject: Subject, message: impl Into<String>) -> Self {
        Self {
            code,
            severity: Severity::Error,
            rule: Some(rule),
            message: message.into(),
            primary: Label::new(subject),
            notes: Vec::new(),
            help: None,
        }
    }

    /// A warning diagnostic.
    #[must_use]
    pub fn warning(code: Code, rule: RuleId, subject: Subject, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            ..Self::error(code, rule, subject, message)
        }
    }

    /// Adds a secondary label.
    #[must_use]
    pub fn note(mut self, label: Label) -> Self {
        self.notes.push(label);
        self
    }

    /// Sets the `help:` line.
    #[must_use]
    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// A deterministic sort key that does not depend on spans.
    #[must_use]
    pub fn sort_key(&self) -> (Code, String) {
        (self.code, format!("{:?}", self.primary.subject))
    }
}

/// The codes this crate raises. Each is registered in `spec/diagnostics/codes-lang.md`,
/// which `tests/codes_registry.rs` verifies.
pub mod codes {
    use super::Code;

    macro_rules! codes {
        ($($(#[$m:meta])* $name:ident = $lit:literal;)*) => {
            $($(#[$m])* pub const $name: Code = Code::new($lit);)*
            /// Every code this crate can raise.
            pub const ALL: &[Code] = &[$($name),*];
        };
    }

    codes! {
        /// Unexpected token.
        P001 = "AKR-P001";
        /// Byte order mark.
        P002 = "AKR-P002";
        /// Carriage return.
        P003 = "AKR-P003";
        /// Missing final newline.
        P004 = "AKR-P004";
        /// Missing header.
        P005 = "AKR-P005";
        /// Unsupported grammar major version.
        P006 = "AKR-P006";
        /// Unknown grammar minor version.
        P007 = "AKR-P007";
        /// Missing project declaration.
        P008 = "AKR-P008";
        /// Empty source file.
        P009 = "AKR-P009";
        /// Newline in a quoted string.
        P011 = "AKR-P011";
        /// Unknown escape sequence.
        P012 = "AKR-P012";
        /// Unterminated string.
        P013 = "AKR-P013";
        /// Unterminated prose block.
        P014 = "AKR-P014";
        /// Tab in prose indentation.
        P015 = "AKR-P015";
        /// Prose content on the opening line.
        P016 = "AKR-P016";
        /// Prose closing delimiter shares a line.
        P017 = "AKR-P017";
        /// Prose block contains a triple quote.
        P018 = "AKR-P018";
        /// Abbreviated commit hash.
        P021 = "AKR-P021";
        /// Invalid date.
        P022 = "AKR-P022";
        /// Timestamp is not UTC.
        P023 = "AKR-P023";
        /// Leading zero in an integer.
        P024 = "AKR-P024";
        /// Invalid revision number.
        P025 = "AKR-P025";
        /// Uppercase in a commit hash.
        P026 = "AKR-P026";
        /// Duplicate slot.
        P031 = "AKR-P031";
        /// Duplicate block head.
        P032 = "AKR-P032";
        /// Block head where none is permitted.
        P033 = "AKR-P033";
        /// Missing block head.
        P034 = "AKR-P034";
        /// Malformed identifier.
        P041 = "AKR-P041";
        /// Malformed key.
        P042 = "AKR-P042";
        /// Malformed reference.
        P043 = "AKR-P043";
        /// Unbalanced brace.
        P044 = "AKR-P044";
        /// Unbalanced bracket.
        P045 = "AKR-P045";
        /// Content after a closing brace.
        P046 = "AKR-P046";
        /// File is not canonically formatted.
        F001 = "AKR-F001";
        /// Slot order is not canonical.
        F002 = "AKR-F002";
        /// Records are not sorted.
        F003 = "AKR-F003";
        /// Array wrapping is not canonical.
        F004 = "AKR-F004";
        /// Indentation is not canonical.
        F005 = "AKR-F005";
        /// Trailing whitespace.
        F006 = "AKR-F006";
        /// Blank line inside a record body.
        F007 = "AKR-F007";
        /// Trailing comma.
        F008 = "AKR-F008";
        /// Empty array.
        F009 = "AKR-F009";
        /// Prose indentation is not canonical.
        F010 = "AKR-F010";
        /// Unsorted array elements.
        F011 = "AKR-F011";
        /// Missing required slot.
        T001 = "AKR-T001";
        /// Unknown slot for this kind.
        T002 = "AKR-T002";
        /// Block not permitted for this kind.
        T005 = "AKR-T005";
        /// Missing required block.
        T006 = "AKR-T006";
        /// Unknown kind.
        T003 = "AKR-T003";
        /// Unknown block.
        T004 = "AKR-T004";
        /// Block in the wrong place.
        T007 = "AKR-T007";
        /// Illegal state for the kind's class.
        T011 = "AKR-T011";
        /// Unknown enum value.
        T012 = "AKR-T012";
        /// Wrong value type.
        T013 = "AKR-T013";
        /// Observation missing `observed_at`.
        T021 = "AKR-T021";
        /// Evidence missing a required slot.
        T022 = "AKR-T022";
        /// Evidence artefact cited from the disposable scratch directory.
        T023 = "AKR-T023";
        /// Resolved question missing a resolution.
        T031 = "AKR-T031";
        /// Malformed scope term.
        T032 = "AKR-T032";
        /// `topic` on a non-normative kind.
        T034 = "AKR-T034";
        /// Unresolved reference.
        L001 = "AKR-L001";
        /// Key has no resolvable head.
        L002 = "AKR-L002";
        /// Unknown revision.
        L003 = "AKR-L003";
        /// Undeclared namespace.
        L004 = "AKR-L004";
        /// Key split across files.
        L006 = "AKR-L006";
        /// Unknown anchor.
        L011 = "AKR-L011";
        /// Retired anchor.
        L012 = "AKR-L012";
        /// Historical reference in a live slot.
        L021 = "AKR-L021";
        /// Relation target out of range.
        L031 = "AKR-L031";
        /// Relation source out of domain.
        L032 = "AKR-L032";
        /// Kind-restricted slot target invalid.
        L033 = "AKR-L033";
        /// Duplicate revision.
        L041 = "AKR-L041";
        /// Two live revisions of one key.
        R001 = "AKR-R001";
        /// Normative topic conflict.
        R002 = "AKR-R002";
        /// Supersession cycle.
        R011 = "AKR-R011";
        /// Relation cycle.
        R012 = "AKR-R012";
        /// Ordering cycle.
        R013 = "AKR-R013";
        /// Missing disposition.
        R014 = "AKR-R014";
        /// Disposition outcome mismatch.
        R015 = "AKR-R015";
        /// Disposition of a non-child.
        R016 = "AKR-R016";
        /// Supersession across kinds.
        R017 = "AKR-R017";
        /// Multiple plans of record.
        R018 = "AKR-R018";
        /// Live record relies on an invalid terminal record.
        R021 = "AKR-R021";
        /// Completion with unsatisfied acceptance.
        R022 = "AKR-R022";
        /// Blocked without a blocker.
        R023 = "AKR-R023";
        /// Active decision cites nothing.
        R031 = "AKR-R031";
        /// Observation lacks provenance.
        R032 = "AKR-R032";
        /// Undispositioned contradiction.
        R041 = "AKR-R041";
        /// Sealed revision modified.
        R051 = "AKR-R051";
        /// Lock stale or incomplete.
        R052 = "AKR-R052";
    }

    /// Codes the write operations raise that belong to the CLI stage.
    ///
    /// These are registered in `spec/diagnostics/codes-runtime.md`, which Writer B owns.
    /// They live in their own module so that [`ALL`] stays exactly the set of language
    /// codes `tests/codes_registry.rs` checks against `codes-lang.md`.
    pub mod cli {
        use super::super::Code;

        /// `project.akr` is missing.
        pub const C012: Code = Code::new("AKR-C012");
        /// Write aborted; the result did not validate.
        pub const C031: Code = Code::new("AKR-C031");
        /// The write would modify a sealed revision.
        pub const C032: Code = Code::new("AKR-C032");
        /// The write target is not the head revision.
        pub const C033: Code = Code::new("AKR-C033");

        /// Every CLI-stage code this crate raises.
        pub const ALL: &[Code] = &[C012, C031, C032, C033];
    }

    /// Codes stage E raises (`docs/06-compiler-pipeline.md` §7).
    ///
    /// Registered in `spec/diagnostics/codes-runtime.md`, and separate from [`ALL`] for
    /// the same reason [`cli`] is: these are facts about the environment or about an
    /// internal invariant, never about the ledger. Routine invalidation raises none of
    /// them — a cache that *had* to be rebuilt is not a problem, and saying so would train
    /// operators to ignore the ones that are.
    pub mod index {
        use super::super::Code;

        /// The cache file exists and cannot be opened.
        pub const I001: Code = Code::new("AKR-I001");
        /// The populate transaction could not commit.
        pub const I002: Code = Code::new("AKR-I002");
        /// `.akr/cache/` is missing and cannot be created, or is read-only.
        pub const I003: Code = Code::new("AKR-I003");
        /// Some other SQLite database sits at the cache path.
        pub const I004: Code = Code::new("AKR-I004");
        /// `PRAGMA integrity_check` did not return `ok`.
        pub const I011: Code = Code::new("AKR-I011");
        /// A resolved head has no `resolutions` row.
        pub const I012: Code = Code::new("AKR-I012");
        /// A row count disagrees with the resolved model.
        pub const I013: Code = Code::new("AKR-I013");
        /// `records_fts` could not be built.
        pub const I021: Code = Code::new("AKR-I021");
        /// A full-text query against a cache built without FTS5.
        pub const I022: Code = Code::new("AKR-I022");
        /// A rebuild was needed while `--no-rebuild` was in force.
        pub const I031: Code = Code::new("AKR-I031");
        /// Another process holds the cache lock.
        pub const I032: Code = Code::new("AKR-I032");

        /// The search query could not be parsed by the full-text engine.
        pub const X031: Code = Code::new("AKR-X031");
        /// A configured ranker could not be reached.
        pub const X032: Code = Code::new("AKR-X032");

        /// Every stage E code this crate raises.
        pub const ALL: &[Code] = &[
            I001, I002, I003, I004, I011, I012, I013, I021, I022, I031, I032, X031, X032,
        ];
    }

    /// Codes the migration workflow raises (`docs/12-migration.md` §9).
    ///
    /// Registered in `spec/diagnostics/codes-runtime.md`, and separate from [`ALL`] for
    /// the same reason [`cli`] is. Migration raises no rule identifiers of its own: the
    /// invariants it relies on — V-020 for completion, V-002 for namespaces — are the
    /// ordinary ones, and these codes cover only what is specific to moving legacy prose
    /// into the ledger.
    pub mod migration {
        use super::super::Code;

        /// The import source does not exist.
        pub const M001: Code = Code::new("AKR-M001");
        /// The import source is not Markdown or plain text.
        pub const M002: Code = Code::new("AKR-M002");
        /// No durable claim was extracted (warning).
        pub const M011: Code = Code::new("AKR-M011");
        /// An imported key collides with an existing key.
        pub const M012: Code = Code::new("AKR-M012");
        /// An imported key's namespace is not declared in `project.akr`.
        pub const M013: Code = Code::new("AKR-M013");
        /// An imported record has no `source { kind legacy }` block.
        pub const M021: Code = Code::new("AKR-M021");
        /// A legacy source path does not exist at HEAD (warning).
        pub const M022: Code = Code::new("AKR-M022");
        /// An imported document has no tracking work record.
        pub const M031: Code = Code::new("AKR-M031");
        /// A legacy document was archived while its tracking record is incomplete.
        pub const M032: Code = Code::new("AKR-M032");
        /// Import produced warnings under the strict profile.
        pub const M041: Code = Code::new("AKR-M041");
        /// An imported record is not in `proposed` state.
        pub const M042: Code = Code::new("AKR-M042");

        /// Every migration code this crate raises.
        pub const ALL: &[Code] = &[
            M001, M002, M011, M012, M013, M021, M022, M031, M032, M041, M042,
        ];
    }
}

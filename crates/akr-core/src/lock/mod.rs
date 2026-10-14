//! `akr.lock`: the model, the reader, the writer, and verification.
//!
//! The lock makes a build reproducible and makes a floating reference's target change
//! visible in review. Its format is specified in `spec/schema/akr-lock.md`; this module
//! implements it.
//!
//! # Why there is a reader here at all
//!
//! The lock is written in the AKR grammar (D-014), so in the finished toolchain it is
//! read by the same lexer and parser as record files. That parser is phase P2. This
//! module contains a small, self-contained reader for the lock's own item set — four
//! block shapes, no prose, no arrays, no comments-with-attachment — which is enough to
//! read and verify a lock without waiting for it.
//!
//! **Seam.** When P2 lands, [`Lock::parse`] should be re-expressed over the shared token
//! stream and this reader deleted. The round-trip tests in `tests/lock_roundtrip.rs` pin
//! the byte-level behaviour, so the swap is checkable rather than hopeful. Nothing else
//! in the crate depends on how the text is read.
//!
//! # Ordering
//!
//! [`Lock::render`] emits fully sorted output (`spec/schema/akr-lock.md` §4), so two
//! builds of the same sources produce byte-identical files and a diff shows only what
//! changed. Sorting happens at render time rather than being demanded of the caller.

use crate::diagnostics::{Diagnostic, RuleId, Subject, codes};
use crate::model::{
    Commit, ContentHash, Ledger, LogicalKey, Reference, RevisionId, SealFact, State,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// The lock's grammar header, in place of `akr 0.1`.
pub const LOCK_HEADER: &str = "akr-lock 0.1";

// -------------------------------------------------------------------------------------
// Model
// -------------------------------------------------------------------------------------

/// The `build` block: what produced this lock, and against what.
///
/// [`Default`] is written by hand rather than derived because [`ContentHash`] is a
/// newtype over `String` with no `Default` of its own, and giving it one would invite an
/// empty digest to be mistaken for a real answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Build {
    /// Tool name and version, for example `akr 0.1.0`.
    pub tool: String,
    /// Grammar version of the sources.
    pub grammar: String,
    /// `vocabulary_version` from `spec/tables/vocabulary.json`.
    pub vocabulary: String,
    /// The commit the build resolved against.
    pub commit: Option<Commit>,
    /// The source-graph hash (`spec/schema/akr-lock.md` §3.2).
    pub source_graph: ContentHash,
    /// UTC timestamp, **informational only**.
    ///
    /// Held as validated text rather than a parsed instant: the model has no timestamp
    /// type because nothing in the design set computes with one. It is the single field
    /// that changes on every build regardless of content, and it is excluded from every
    /// comparison (see [`Lock::verify`]) so that it never causes a spurious `AKR-R052`.
    pub built_at: String,
}

impl Default for Build {
    fn default() -> Self {
        Self {
            tool: String::new(),
            grammar: String::new(),
            vocabulary: String::new(),
            commit: None,
            source_graph: ContentHash(String::new()),
            built_at: String::new(),
        }
    }
}

/// One `source` entry: a `.akr` file the build read.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SourceEntry {
    /// Repo-root-relative path with forward slashes.
    pub path: String,
    /// SHA-256 over the file's raw bytes on disk (§3.1).
    pub hash: ContentHash,
    /// Record count. Informational; makes a truncated file obvious at a glance.
    pub records: u32,
}

/// One `resolution` entry: a floating reference and what it resolved to.
///
/// Pinned references are never recorded — a pinned reference cannot change what it points
/// at, so locking it would be noise.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ResolutionEntry {
    /// The referring revision, always pinned.
    pub from: RevisionId,
    /// The slot the reference appeared in.
    pub slot: String,
    /// What `@key` resolved to, always pinned.
    pub to: RevisionId,
    /// Content hash of the resolved revision (§3.3).
    ///
    /// This is what makes a repointing visible even when the revision number is
    /// unchanged, which happens when a `proposed` head is edited in place.
    pub hash: ContentHash,
}

/// One `seal` entry: a revision in a state other than `proposed` (D-015).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SealEntry {
    /// The sealed revision, always pinned.
    pub id: RevisionId,
    /// Its state when sealed. Informational; the seal applies to any non-`proposed` state.
    pub state: State,
    /// Content hash of the revision (§3.3).
    pub hash: ContentHash,
}

/// A parsed or constructed `akr.lock`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Lock {
    /// The project name, matching the `project` line.
    pub project: String,
    /// Build inputs.
    pub build: Build,
    /// Source files, sorted by path at render time.
    pub sources: Vec<SourceEntry>,
    /// Floating-reference resolutions.
    pub resolutions: Vec<ResolutionEntry>,
    /// Sealed revisions.
    pub seals: Vec<SealEntry>,
}

// -------------------------------------------------------------------------------------
// Errors
// -------------------------------------------------------------------------------------

/// Why a lock could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockError {
    /// 1-based line number.
    pub line: usize,
    /// What went wrong.
    pub message: String,
}

impl fmt::Display for LockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "akr.lock line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for LockError {}

fn err<T>(line: usize, message: impl Into<String>) -> Result<T, LockError> {
    Err(LockError {
        line,
        message: message.into(),
    })
}

// -------------------------------------------------------------------------------------
// Verification
// -------------------------------------------------------------------------------------

/// One way in which a lock disagrees with what the build computed.
///
/// Deliberately a *description*, not a diagnostic: turning these into `AKR-R051` and
/// `AKR-R052` is V-024's job, which reads [`crate::model::LedgerFacts`]. Keeping the two
/// apart means the lock module needs no opinion about severity or rule identifiers.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mismatch {
    /// A `build` slot other than `built_at` differs.
    Build {
        /// Which slot.
        slot: &'static str,
        /// What the lock records.
        recorded: String,
        /// What the build computed.
        computed: String,
    },
    /// A source file's hash differs, or the file is new or gone.
    Source {
        /// The path.
        path: String,
        /// The lock's hash, if it has an entry.
        recorded: Option<ContentHash>,
        /// The computed hash, if the file was read.
        computed: Option<ContentHash>,
    },
    /// A floating reference now resolves elsewhere, or is new or gone.
    Resolution {
        /// The referring revision.
        from: RevisionId,
        /// The slot.
        slot: String,
        /// What the lock says it resolved to.
        recorded: Option<RevisionId>,
        /// What it resolves to now.
        computed: Option<RevisionId>,
    },
    /// A sealed revision's hash differs, or the seal is new or gone.
    Seal {
        /// The revision.
        id: RevisionId,
        /// The lock's hash, if it has an entry.
        recorded: Option<ContentHash>,
        /// The computed hash, if the revision still exists.
        computed: Option<ContentHash>,
    },
}

impl Lock {
    /// Differences relevant to lock currency, preserving exact provenance comparison
    /// in [`Self::verify`].
    ///
    /// A lock authored from dirty sources records no commit. Committing those exact
    /// bytes supplies provenance without invalidating the build. Accept this one-way
    /// transition only when every other verified field agrees; never invent a commit
    /// in the recorded lock or excuse a source, resolution, seal, or tool mismatch.
    #[must_use]
    pub fn currency_mismatches(&self, computed: &Self) -> Vec<Mismatch> {
        let mismatches = self.verify(computed);
        if self.build.commit.is_none()
            && computed.build.commit.is_some()
            && mismatches
                .iter()
                .all(|mismatch| matches!(mismatch, Mismatch::Build { slot: "commit", .. }))
        {
            Vec::new()
        } else {
            mismatches
        }
    }

    /// Verifies this lock against a freshly computed one.
    ///
    /// Compares **everything except `build.built_at`** (`spec/schema/akr-lock.md` §6).
    /// `built_at` changes on every build regardless of content; comparing it would make
    /// every lock permanently stale.
    ///
    /// Returns the mismatches in a deterministic order. An empty result means the lock is
    /// current.
    #[must_use]
    pub fn verify(&self, computed: &Self) -> Vec<Mismatch> {
        let mut out = Vec::new();

        let mut build_slot = |slot: &'static str, a: &str, b: &str| {
            if a != b {
                out.push(Mismatch::Build {
                    slot,
                    recorded: a.to_owned(),
                    computed: b.to_owned(),
                });
            }
        };
        build_slot("tool", &self.build.tool, &computed.build.tool);
        build_slot("grammar", &self.build.grammar, &computed.build.grammar);
        build_slot(
            "vocabulary",
            &self.build.vocabulary,
            &computed.build.vocabulary,
        );
        build_slot(
            "commit",
            &self
                .build
                .commit
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            &computed
                .build
                .commit
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
        );
        build_slot(
            "source_graph",
            &self.build.source_graph.0,
            &computed.build.source_graph.0,
        );
        // built_at is deliberately not compared.

        let recorded_sources: BTreeMap<&str, &SourceEntry> =
            self.sources.iter().map(|s| (s.path.as_str(), s)).collect();
        let computed_sources: BTreeMap<&str, &SourceEntry> = computed
            .sources
            .iter()
            .map(|s| (s.path.as_str(), s))
            .collect();
        let paths: BTreeSet<&str> = recorded_sources
            .keys()
            .chain(computed_sources.keys())
            .copied()
            .collect();
        for path in paths {
            let a = recorded_sources.get(path).map(|s| s.hash.clone());
            let b = computed_sources.get(path).map(|s| s.hash.clone());
            if a != b {
                out.push(Mismatch::Source {
                    path: path.to_owned(),
                    recorded: a,
                    computed: b,
                });
            }
        }

        let key = |r: &ResolutionEntry| (r.from.clone(), r.slot.clone());
        let recorded_res: BTreeMap<_, _> = self.resolutions.iter().map(|r| (key(r), r)).collect();
        let computed_res: BTreeMap<_, _> =
            computed.resolutions.iter().map(|r| (key(r), r)).collect();
        let res_keys: BTreeSet<_> = recorded_res
            .keys()
            .chain(computed_res.keys())
            .cloned()
            .collect();
        for k in res_keys {
            let a = recorded_res.get(&k);
            let b = computed_res.get(&k);
            if a.map(|r| (&r.to, &r.hash)) != b.map(|r| (&r.to, &r.hash)) {
                out.push(Mismatch::Resolution {
                    from: k.0,
                    slot: k.1,
                    recorded: a.map(|r| r.to.clone()),
                    computed: b.map(|r| r.to.clone()),
                });
            }
        }

        let recorded_seals: BTreeMap<_, _> = self.seals.iter().map(|s| (&s.id, s)).collect();
        let computed_seals: BTreeMap<_, _> = computed.seals.iter().map(|s| (&s.id, s)).collect();
        let seal_keys: BTreeSet<_> = recorded_seals
            .keys()
            .chain(computed_seals.keys())
            .copied()
            .collect();
        for id in seal_keys {
            let a = recorded_seals.get(id).map(|s| s.hash.clone());
            let b = computed_seals.get(id).map(|s| s.hash.clone());
            if a != b {
                out.push(Mismatch::Seal {
                    id: id.clone(),
                    recorded: a,
                    computed: b,
                });
            }
        }

        out
    }

    /// Fills the [`LedgerFacts`](crate::model::LedgerFacts) that V-024 reads.
    ///
    /// This is the whole integration between the lock and the rule catalogue: populate
    /// `facts.seals` and `facts.lock_present`, and V-024 raises `AKR-R051` (hash drift)
    /// and `AKR-R052` (missing entry) with no rule change at all.
    ///
    /// `computed` maps each revision to the hash the build computed for it — from
    /// [`crate::hash::content_hash`] over the record's canonical text. A revision absent
    /// from `computed` gets `SealFact::computed = None`, which V-024 treats as "not
    /// checked" rather than "mismatched": a build that could not produce canonical text
    /// must not accuse anyone of editing a sealed record.
    pub fn apply_facts(&self, ledger: &mut Ledger, computed: &BTreeMap<RevisionId, ContentHash>) {
        ledger.facts.lock_present = true;
        let recorded: BTreeMap<&RevisionId, &SealEntry> =
            self.seals.iter().map(|seal| (&seal.id, seal)).collect();

        let mut seals: BTreeMap<RevisionId, SealFact> = BTreeMap::new();
        let ids: BTreeSet<&RevisionId> = recorded.keys().copied().chain(computed.keys()).collect();
        for id in ids {
            seals.insert(
                id.clone(),
                SealFact {
                    recorded: recorded.get(id).map(|seal| seal.hash.clone()),
                    recorded_state: recorded.get(id).map(|seal| seal.state),
                    computed: computed.get(id).cloned(),
                },
            );
        }
        ledger.facts.seals = seals;
    }

    /// Renders the lock in canonical AKR syntax, fully sorted
    /// (`spec/schema/akr-lock.md` §4).
    ///
    /// Byte-identical for identical content: `build` first, then `source` by path
    /// bytewise, then `resolution` by referring key, referring revision, slot name and
    /// target key, then `seal` by key and revision ascending. One blank line between
    /// top-level items, none inside a block, and exactly one trailing newline.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(LOCK_HEADER);
        out.push('\n');
        out.push_str(&format!("project {}\n\n", self.project));

        out.push_str("build {\n");
        out.push_str(&format!("    tool {}\n", quote(&self.build.tool)));
        out.push_str(&format!("    grammar {}\n", quote(&self.build.grammar)));
        out.push_str(&format!(
            "    vocabulary {}\n",
            quote(&self.build.vocabulary)
        ));
        if let Some(commit) = &self.build.commit {
            out.push_str(&format!("    commit {commit}\n"));
        }
        out.push_str(&format!(
            "    source_graph {}\n",
            quote(&self.build.source_graph.0)
        ));
        if !self.build.built_at.is_empty() {
            out.push_str(&format!("    built_at {}\n", self.build.built_at));
        }
        out.push_str("}\n");

        let mut sources = self.sources.clone();
        sources.sort_by(|a, b| a.path.as_bytes().cmp(b.path.as_bytes()));
        for source in &sources {
            out.push_str(&format!("\nsource {} {{\n", quote(&source.path)));
            out.push_str(&format!("    hash {}\n", quote(&source.hash.0)));
            out.push_str(&format!("    records {}\n", source.records));
            out.push_str("}\n");
        }

        let mut resolutions = self.resolutions.clone();
        resolutions.sort_by(|a, b| {
            (&a.from.key, a.from.revision, &a.slot, &a.to.key).cmp(&(
                &b.from.key,
                b.from.revision,
                &b.slot,
                &b.to.key,
            ))
        });
        for resolution in &resolutions {
            out.push_str(&format!("\nresolution @{} {{\n", resolution.from));
            out.push_str(&format!("    slot {}\n", resolution.slot));
            out.push_str(&format!("    to @{}\n", resolution.to));
            out.push_str(&format!("    hash {}\n", quote(&resolution.hash.0)));
            out.push_str("}\n");
        }

        let mut seals = self.seals.clone();
        seals.sort_by(|a, b| (&a.id.key, a.id.revision).cmp(&(&b.id.key, b.id.revision)));
        for seal in &seals {
            out.push_str(&format!("\nseal @{} {{\n", seal.id));
            out.push_str(&format!("    state {}\n", seal.state.name()));
            out.push_str(&format!("    hash {}\n", quote(&seal.hash.0)));
            out.push_str("}\n");
        }

        out
    }

    /// Reads a lock from AKR text.
    ///
    /// # Errors
    /// Returns [`LockError`] naming the line for a bad header, an unknown item, a
    /// malformed reference, an unknown state, or a missing required slot.
    pub fn parse(text: &str) -> Result<Self, LockError> {
        Reader::new(text).read()
    }
}

fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

// -------------------------------------------------------------------------------------
// Reader
// -------------------------------------------------------------------------------------

/// A line-oriented reader for the lock's four item shapes.
///
/// See the module docs for why this exists and when it should be deleted.
struct Reader<'a> {
    lines: Vec<&'a str>,
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            lines: text.lines().collect(),
            at: 0,
        }
    }

    fn read(mut self) -> Result<Lock, LockError> {
        let mut lock = Lock::default();
        let mut seen_header = false;
        let mut seen_build = false;

        while let Some(line) = self.next_significant() {
            let lineno = self.at;
            let trimmed = line.trim();

            if !seen_header {
                if trimmed != LOCK_HEADER {
                    return err(
                        lineno,
                        format!("expected `{LOCK_HEADER}`, found {trimmed:?}"),
                    );
                }
                seen_header = true;
                continue;
            }

            if let Some(name) = trimmed.strip_prefix("project ") {
                lock.project = name.trim().to_owned();
                continue;
            }

            if trimmed == "build {" {
                if seen_build {
                    return err(lineno, "a lock has exactly one build block");
                }
                seen_build = true;
                lock.build = self.read_build()?;
                continue;
            }

            if let Some(rest) = trimmed.strip_prefix("source ") {
                let path = unquote(rest.trim_end_matches('{').trim(), lineno)?;
                let slots = self.read_block()?;
                lock.sources.push(SourceEntry {
                    path,
                    hash: ContentHash(unquote(self.need(&slots, "hash", lineno)?, lineno)?),
                    records: self.need(&slots, "records", lineno)?.parse().map_err(|_| {
                        LockError {
                            line: lineno,
                            message: "records must be an integer".to_owned(),
                        }
                    })?,
                });
                continue;
            }

            if let Some(rest) = trimmed.strip_prefix("resolution ") {
                let from = pinned(rest.trim_end_matches('{').trim(), lineno)?;
                let slots = self.read_block()?;
                lock.resolutions.push(ResolutionEntry {
                    from,
                    slot: self.need(&slots, "slot", lineno)?.to_owned(),
                    to: pinned(self.need(&slots, "to", lineno)?, lineno)?,
                    hash: ContentHash(unquote(self.need(&slots, "hash", lineno)?, lineno)?),
                });
                continue;
            }

            if let Some(rest) = trimmed.strip_prefix("seal ") {
                let id = pinned(rest.trim_end_matches('{').trim(), lineno)?;
                let slots = self.read_block()?;
                let state_name = self.need(&slots, "state", lineno)?;
                let Some(state) = State::from_name(state_name) else {
                    return err(lineno, format!("unknown state {state_name:?}"));
                };
                lock.seals.push(SealEntry {
                    id,
                    state,
                    hash: ContentHash(unquote(self.need(&slots, "hash", lineno)?, lineno)?),
                });
                continue;
            }

            return err(lineno, format!("unexpected item {trimmed:?}"));
        }

        if !seen_header {
            return err(1, "empty lock file");
        }
        Ok(lock)
    }

    fn read_build(&mut self) -> Result<Build, LockError> {
        let lineno = self.at;
        let slots = self.read_block()?;
        Ok(Build {
            tool: unquote(self.need(&slots, "tool", lineno)?, lineno)?,
            grammar: unquote(self.need(&slots, "grammar", lineno)?, lineno)?,
            vocabulary: unquote(self.need(&slots, "vocabulary", lineno)?, lineno)?,
            commit: match slots.get("commit") {
                Some(text) => Some(Commit::new(text).map_err(|e| LockError {
                    line: lineno,
                    message: e.to_string(),
                })?),
                None => None,
            },
            source_graph: ContentHash(unquote(self.need(&slots, "source_graph", lineno)?, lineno)?),
            built_at: slots
                .get("built_at")
                .copied()
                .unwrap_or_default()
                .to_owned(),
        })
    }

    /// Reads slot lines up to the closing brace of the block just opened.
    fn read_block(&mut self) -> Result<BTreeMap<&'a str, &'a str>, LockError> {
        let mut slots = BTreeMap::new();
        loop {
            let Some(line) = self.next_line() else {
                return err(self.at, "unterminated block");
            };
            let trimmed = line.trim();
            if trimmed == "}" {
                return Ok(slots);
            }
            if trimmed.is_empty() {
                continue;
            }
            let Some((name, value)) = trimmed.split_once(char::is_whitespace) else {
                return err(
                    self.at,
                    format!("expected `<slot> <value>`, found {trimmed:?}"),
                );
            };
            if slots.insert(name, value.trim()).is_some() {
                return err(self.at, format!("repeated slot {name:?}"));
            }
        }
    }

    fn need(
        &self,
        slots: &BTreeMap<&'a str, &'a str>,
        name: &str,
        line: usize,
    ) -> Result<&'a str, LockError> {
        slots.get(name).copied().ok_or(LockError {
            line,
            message: format!("missing required slot `{name}`"),
        })
    }

    fn next_line(&mut self) -> Option<&'a str> {
        let line = self.lines.get(self.at).copied()?;
        self.at += 1;
        Some(line)
    }

    /// The next line that is neither blank nor a whole-line comment.
    fn next_significant(&mut self) -> Option<&'a str> {
        loop {
            let line = self.next_line()?;
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            return Some(line);
        }
    }
}

fn unquote(text: &str, line: usize) -> Result<String, LockError> {
    let trimmed = text.trim();
    let Some(inner) = trimmed.strip_prefix('"').and_then(|t| t.strip_suffix('"')) else {
        return err(line, format!("expected a quoted string, found {trimmed:?}"));
    };
    let mut out = String::with_capacity(inner.len());
    let mut escaped = false;
    for ch in inner.chars() {
        if escaped {
            out.push(match ch {
                'n' => '\n',
                't' => '\t',
                'r' => '\r',
                other => other,
            });
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else {
            out.push(ch);
        }
    }
    Ok(out)
}

/// Parses `@key/N`, requiring the revision: every reference in a lock is pinned.
fn pinned(text: &str, line: usize) -> Result<RevisionId, LockError> {
    let reference = Reference::parse(text.trim()).map_err(|e| LockError {
        line,
        message: e.to_string(),
    })?;
    match reference.revision {
        Some(revision) => Ok(RevisionId::new(reference.key, revision)),
        None => err(
            line,
            format!("{text:?} must be pinned; a lock records revisions"),
        ),
    }
}

// -------------------------------------------------------------------------------------
// Helpers for building a lock
// -------------------------------------------------------------------------------------

/// Turns lock drift into `AKR-R052` diagnostics.
///
/// V-024 raises `AKR-R052` for a sealed revision the lock has no entry for, which it can
/// see through [`crate::model::LedgerFacts`]. It cannot see the *other* half of lock
/// currency — a `build` slot or a source-file hash that no longer matches the sources —
/// because those are not facts about any record. This function reports that half, under
/// the same code and the same rule.
///
/// The subject is the lock file itself, because the fault is the lock's rather than any
/// record's, and the fix is `akr build` rather than an edit.
///
/// Resolution and seal mismatches are deliberately **not** reported here: the first is
/// what a lock diff is for, and the second is V-024's `AKR-R051`. Reporting them twice
/// would double every diagnostic about a modified record.
#[must_use]
pub fn currency_diagnostics(recorded: &Lock, computed: &Lock, path: &str) -> Vec<Diagnostic> {
    const RULE: RuleId = RuleId(24);
    let mut out = Vec::new();
    for mismatch in recorded.currency_mismatches(computed) {
        let detail = match mismatch {
            Mismatch::Build {
                slot,
                recorded,
                computed,
            } => {
                format!("build.{slot} records {recorded:?}, the build computed {computed:?}")
            }
            Mismatch::Source {
                path,
                recorded,
                computed,
            } => match (recorded, computed) {
                (Some(_), Some(_)) => {
                    format!("source {path} has changed since the lock was written")
                }
                (Some(_), None) => format!("source {path} is in the lock but not on disk"),
                (None, _) => format!("source {path} is on disk but not in the lock"),
            },
            Mismatch::Resolution { .. } | Mismatch::Seal { .. } => continue,
        };
        out.push(
            Diagnostic::error(
                codes::R052,
                RULE,
                Subject::File(path.to_owned()),
                format!("akr.lock does not match the sources: {detail}"),
            )
            .help("run `akr build`; never hand-merge a lock"),
        );
    }
    out.sort_by_key(Diagnostic::sort_key);
    out.dedup_by(|a, b| a.message == b.message);
    out
}

/// Every sealed revision of a ledger, in canonical order.
///
/// A revision is sealed when its state is anything other than `proposed` (D-015).
#[must_use]
pub fn sealed_revisions(ledger: &Ledger) -> Vec<&RevisionId> {
    let mut ids: Vec<&RevisionId> = ledger
        .records()
        .iter()
        .filter(|r| r.is_sealed())
        .map(|r| &r.id)
        .collect();
    ids.sort();
    ids
}

/// Groups keys by the file they live in, for the `records` count of a `source` entry.
#[must_use]
pub fn records_per_file(ledger: &Ledger) -> BTreeMap<String, u32> {
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    for record in ledger.records() {
        if let Some(file) = &record.file {
            *counts.entry(file.clone()).or_default() += 1;
        }
    }
    counts
}

/// Every key a ledger knows, sorted. Convenience for lock construction and tests.
#[must_use]
pub fn keys(ledger: &Ledger) -> Vec<LogicalKey> {
    ledger.keys().into_iter().cloned().collect()
}

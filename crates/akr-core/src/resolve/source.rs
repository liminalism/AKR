//! Loading a workspace from disk: files in, ledger and build inputs out.
//!
//! This is the front half of `akr build` steps 4–7 (`docs/06-compiler-pipeline.md` §13):
//! discover the sources by a sorted walk, hash their raw bytes, parse, lower, and produce
//! the [`BuildInputs`] the resolver needs.
//!
//! # Canonical text and the content hash
//!
//! `spec/schema/akr-lock.md` §3.3 defines the revision content hash over "the canonically
//! formatted text of that record alone". [`canonical_record_text`] produces exactly that
//! by handing one record at a time to the phase P2 formatter and taking what it emits,
//! which keeps this module out of the business of knowing how a record is written.

use super::{BuildInputs, SourceFile};
use crate::diagnostics::{Diagnostic, SlotRef, SourceMap, Span, Subject};
use crate::hash::source_file_hash;
use crate::model::{ContentSlot, Ledger, LogicalKey, Relation, RevisionId, Segment};
use crate::syntax::cst::{BodyItem, File, Item};
use crate::syntax::{format, lower::lower_all, parse};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::{fs, io};

/// Maps a diagnostic [`Subject`] to the span of the thing it names.
///
/// This is the P2/P3 join: the validation rules produce subject-bearing diagnostics with
/// no spans, because a rule reasons about records rather than about bytes, and the CST
/// knows where each record sits. Filling [`Label::span`](crate::diagnostics::Label) needs
/// exactly this table and nothing else, and building it during loading costs one pass
/// over the CST the parser already produced.
#[derive(Debug, Clone, Default)]
pub struct SpanIndex {
    spans: BTreeMap<Subject, Span>,
}

impl SpanIndex {
    /// The span of a subject, if the index has one.
    #[must_use]
    pub fn get(&self, subject: &Subject) -> Option<Span> {
        self.spans.get(subject).copied().or_else(|| match subject {
            // A slot the index does not know falls back to its record, which is always
            // better than no location at all.
            Subject::Slot(id, _) => self.spans.get(&Subject::Revision(id.clone())).copied(),
            _ => None,
        })
    }

    /// Attaches spans to a diagnostic and its notes, in place.
    pub fn attach(&self, diagnostic: &mut Diagnostic) {
        if diagnostic.primary.span.is_none() {
            diagnostic.primary.span = self.get(&diagnostic.primary.subject);
        }
        for note in &mut diagnostic.notes {
            if note.span.is_none() {
                note.span = self.get(&note.subject);
            }
        }
    }

    /// Attaches spans to every diagnostic in a slice.
    pub fn attach_all(&self, diagnostics: &mut [Diagnostic]) {
        for diagnostic in diagnostics {
            self.attach(diagnostic);
        }
    }

    fn insert(&mut self, subject: Subject, span: Span) {
        self.spans.entry(subject).or_insert(span);
    }
}

/// A workspace read from disk.
#[derive(Debug)]
pub struct Workspace {
    /// The ledger, with every record tagged with the file it came from (V-003).
    pub ledger: Ledger,
    /// Build inputs: sources, their raw-byte hashes, and canonical text per revision.
    pub inputs: BuildInputs,
    /// Diagnostics from stages A and B.
    pub diagnostics: Vec<Diagnostic>,
    /// The `akr.lock` text, if the workspace has one.
    pub lock_text: Option<String>,
    /// Every file, for rendering diagnostics.
    pub sources: SourceMap,
    /// Subject-to-span, for attaching locations to rule diagnostics.
    pub spans: SpanIndex,
}

/// Reads every `.akr` source under an `.akr/` directory.
///
/// Discovery is a recursive walk whose entries are sorted by full path with a plain byte
/// comparison, so file order is the same on every filesystem
/// (`docs/06-compiler-pipeline.md` §3). `akr.lock` is read but is not a source: it is not
/// hashed into the source graph and it contributes no records.
///
/// `root` is the repository root; `akr_dir` is the `.akr` directory beneath it. Paths in
/// the returned [`SourceFile`]s are relative to `root` with forward slashes, which is the
/// form the lock records.
///
/// # Errors
/// Returns any I/O error from walking or reading.
pub fn load_workspace(root: &Path, akr_dir: &Path) -> io::Result<Workspace> {
    let mut paths = Vec::new();
    collect_akr_files(akr_dir, &mut paths)?;
    paths.sort();

    let mut sources = Vec::new();
    let mut parsed_files: Vec<(String, File)> = Vec::new();
    let mut canonical_text: BTreeMap<RevisionId, String> = BTreeMap::new();
    let mut diagnostics = Vec::new();
    let mut lock_text = None;
    let mut source_map = SourceMap::new();
    let mut spans = SpanIndex::default();

    for path in &paths {
        let bytes = fs::read(path)?;
        let relative = relative_slash(root, path);
        if path.file_name().is_some_and(|n| n == "akr.lock") {
            lock_text = Some(String::from_utf8_lossy(&bytes).into_owned());
            continue;
        }

        let text = String::from_utf8_lossy(&bytes).into_owned();
        let file_id = source_map.add(&relative, &text);
        let parsed = parse(&text, file_id);
        diagnostics.extend(parsed.diagnostics);

        let mut records = 0u32;
        if let Some(file) = parsed.file {
            for (at, item) in file.items.iter().enumerate() {
                if let Item::Record(record) = item {
                    records += 1;
                    index_record_spans(&mut spans, record);
                    if let Ok(key) = LogicalKey::parse(&record.key)
                        && let Some(canonical) = canonical_record_text(&file, at)
                    {
                        canonical_text.insert(RevisionId::new(key, record.revision), canonical);
                    }
                }
            }
            parsed_files.push((relative.clone(), file));
        }

        sources.push(SourceFile {
            path: relative,
            hash: source_file_hash(&bytes),
            records,
            byte_len: bytes.len() as u64,
        });
    }

    let (mut ledger, lower_diagnostics) = lower_all(&parsed_files);
    diagnostics.extend(lower_diagnostics);
    ledger.facts.scratch_kept = crate::scratch::kept_entries(root);

    Ok(Workspace {
        ledger,
        inputs: BuildInputs {
            sources,
            canonical_text,
            ..BuildInputs::default()
        },
        diagnostics,
        lock_text,
        sources: source_map,
        spans,
    })
}

/// The canonical text of one record: from the `record` keyword through its closing brace,
/// with LF endings, no leading indentation, and a single trailing newline.
///
/// Produced by formatting a file that contains only that record and dropping the two
/// header lines. Using the real formatter rather than slicing the input is what makes the
/// hash stable across a reformat: text that was already canonical formats to itself, and
/// text that was not formats to the same canonical bytes as its reformatted twin.
///
/// Returns `None` if the index does not name a record.
#[must_use]
pub fn canonical_record_text(file: &File, index: usize) -> Option<String> {
    if !matches!(file.items.get(index), Some(Item::Record(_))) {
        return None;
    }
    let mut single = file.clone();
    single.leading = Vec::new();
    single.trailing = Vec::new();
    single.blank_before_header = false;
    single.items = vec![file.items[index].clone()];

    let rendered = format(&single);
    let start = rendered.find("\nrecord ")? + 1;
    Some(rendered[start..].to_owned())
}

/// The canonical text of one record with lifecycle and completion bookkeeping removed,
/// for the D-029 "last *definitional* change" computation.
///
/// Drops the `state` and `note` slots and every `verified_by` slot, wherever they occur.
/// `akr complete` writes exactly these — `state` becomes `completed` and each satisfied
/// check gains a `verified_by` — and D-026's `note` is commentary. None of them redefine
/// what the record *requires*, so a change confined to them must not move the record's
/// last content change and strand the very evidence that closes it (D-016 / V-020). Only
/// [`last_change_of`](crate::git::last_change_of) hashes this projection; the D-015 seal
/// still hashes the whole [`canonical_record_text`].
///
/// Returns `None` if the index does not name a record.
#[must_use]
pub fn definitional_record_text(file: &File, index: usize) -> Option<String> {
    if !matches!(file.items.get(index), Some(Item::Record(_))) {
        return None;
    }
    let mut single = file.clone();
    single.leading = Vec::new();
    single.trailing = Vec::new();
    single.blank_before_header = false;
    let mut item = file.items[index].clone();
    if let Item::Record(record) = &mut item {
        strip_bookkeeping(&mut record.body);
    }
    single.items = vec![item];

    let rendered = format(&single);
    let start = rendered.find("\nrecord ")? + 1;
    Some(rendered[start..].to_owned())
}

/// Removes `state`, `note` and `verified_by` slots from a body, recursing into blocks so a
/// check's `verified_by` inside an `acceptance` block is reached.
fn strip_bookkeeping(body: &mut Vec<BodyItem>) {
    body.retain(|item| !matches!(item, BodyItem::Slot(slot) if is_bookkeeping(&slot.name)));
    for item in body.iter_mut() {
        if let BodyItem::Block(block) = item {
            strip_bookkeeping(&mut block.body);
        }
    }
}

/// The slot names that are lifecycle or completion bookkeeping, never definition.
fn is_bookkeeping(name: &str) -> bool {
    matches!(name, "state" | "note" | "verified_by")
}

/// Indexes the spans of one record: the revision itself, then each slot and block.
///
/// The revision's span is its **header line**, not its whole body: a caret under fifty
/// lines of record is not a location, it is a shrug. Slots and blocks get their own
/// spans, so a diagnostic about `state` points at `state`.
fn index_record_spans(spans: &mut SpanIndex, record: &crate::syntax::cst::Record) {
    let Ok(key) = LogicalKey::parse(&record.key) else {
        return;
    };
    let id = RevisionId::new(key, record.revision);

    // From `record` through the opening brace. Canonical form puts exactly `" {"` after
    // the kind (D-012), so the header ends two bytes past the kind word.
    let header = Span {
        file: record.span.file,
        start: record.span.start,
        end: record.kind_span.end.saturating_add(2).min(record.span.end),
    };
    spans.insert(Subject::Revision(id.clone()), header);

    for item in &record.body {
        let span = item.span();
        let slot = match item {
            crate::syntax::cst::BodyItem::Slot(slot) => slot_ref(&slot.name),
            crate::syntax::cst::BodyItem::Block(block) => match block.name.as_str() {
                "acceptance" => Some(SlotRef::Acceptance),
                "claim" => Segment::new(&block.head_text()).ok().map(SlotRef::Claim),
                "check" => Segment::new(&block.head_text()).ok().map(SlotRef::Check),
                _ => None,
            },
        };
        if let Some(slot) = slot {
            spans.insert(Subject::Slot(id.clone(), slot), span);
        }
    }
}

/// Maps a slot name to the [`SlotRef`] the rules use.
fn slot_ref(name: &str) -> Option<SlotRef> {
    Some(match name {
        "title" => SlotRef::Title,
        "state" => SlotRef::State,
        "scope" => SlotRef::Scope,
        "topic" => SlotRef::Topic,
        "retired_claims" => SlotRef::RetiredClaims,
        other => {
            if let Some(relation) = Relation::from_name(other) {
                SlotRef::Relation(relation)
            } else {
                SlotRef::Content(ContentSlot::from_name(other)?)
            }
        }
    })
}

/// Recursively collects `.akr` files, including `akr.lock`.
fn collect_akr_files(dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)?
        .collect::<io::Result<Vec<_>>>()?
        .into_iter()
        .map(|e| e.path())
        .collect();
    entries.sort();
    for entry in entries {
        if entry.is_dir() {
            // The index cache is generated and never a source (D-019).
            if entry.file_name().is_some_and(|n| n == "cache") {
                continue;
            }
            collect_akr_files(&entry, out)?;
        } else if entry.extension().is_some_and(|e| e == "akr" || e == "lock") {
            out.push(entry);
        }
    }
    Ok(())
}

/// A repo-root-relative path with forward slashes, as the lock records them.
fn relative_slash(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

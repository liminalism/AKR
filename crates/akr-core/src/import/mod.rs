//! Legacy-document extraction for `akr import` (`docs/12-migration.md`).
//!
//! This module is the **deterministic floor** of P8: given the text of a Markdown or
//! plain-text document, it proposes one draft claim per heading (per paragraph for plain
//! text), with no model anywhere near it. Model-assisted extraction, where a person uses
//! one, happens *before* this pipeline and produces an ordinary document for it to read
//! (`docs/12` §6) — nothing in this crate calls a model or ever will.
//!
//! Three properties the tests pin, because the workflow depends on them:
//!
//! - **Excerpts are verbatim.** Every [`DraftClaim::excerpt`] is a byte-identical
//!   substring of the input, produced by slicing it, never by rebuilding it. A
//!   paraphrased excerpt would defeat the review step's audit (`docs/12` §6).
//! - **Extraction is pure.** Same text in, same claims out; no filesystem, no clock.
//! - **Kinds are proposals.** The classifier below is a handful of keyword rules, stated
//!   in [`classify`]'s documentation and applied in order. It exists so a draft lands in
//!   a plausible file, not to be right; the reviewer's step 4 verdict is the decision.
//!
//! [`audit`] is the other half of the module: the check-time pass over a ledger that
//! raises `AKR-M022`, `AKR-M031` and `AKR-M032` for legacy provenance that has decayed
//! after import — the document deleted, the tracking record missing, the document
//! archived before its disposition finished.

use crate::diagnostics::{Diagnostic, Label, Severity, Subject, codes::migration};
use crate::model::{Kind, Ledger, Segment, SourceKind, State};

// -------------------------------------------------------------------------------------
// formats
// -------------------------------------------------------------------------------------

/// The importable source formats. 0.1 imports Markdown and plain text only
/// (`AKR-M002` for anything else).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Headings segment the document; one claim per heading.
    Markdown,
    /// Blank lines segment the document; one claim per paragraph.
    PlainText,
}

impl Format {
    /// The format a file extension names, or `None` for an unimportable one.
    #[must_use]
    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension.to_ascii_lowercase().as_str() {
            "md" | "markdown" => Some(Self::Markdown),
            "txt" | "text" => Some(Self::PlainText),
            _ => None,
        }
    }
}

// -------------------------------------------------------------------------------------
// extraction
// -------------------------------------------------------------------------------------

/// One durable-claim proposal drafted from the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftClaim {
    /// The proposed title: the heading text, or a plain-text paragraph's first line.
    pub title: String,
    /// The proposed kind, from [`classify`]. A proposal for review, not a verdict.
    pub kind: Kind,
    /// A valid key segment derived from the title, unique within this extraction.
    pub slug: String,
    /// The verbatim passage the claim came from — always a substring of the input.
    pub excerpt: String,
    /// The 1-based line the claim starts on, for the dry-run listing.
    pub line: usize,
}

/// A relative path the document links to, for the `AKR-M022` check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkReference {
    /// The link target, fragment stripped.
    pub target: String,
    /// The 1-based line of the target.
    pub line: usize,
    /// The 1-based column of the target.
    pub column: usize,
}

/// What [`extract`] read out of a document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Extraction {
    /// The drafted claims, in document order.
    pub claims: Vec<DraftClaim>,
    /// Paragraphs read and not used as an excerpt — the dry-run's "skipped" count.
    pub paragraphs_skipped: usize,
    /// Relative link targets, for existence checking by the caller.
    pub links: Vec<LinkReference>,
}

/// Extracts draft claims from a document. Pure and deterministic.
#[must_use]
pub fn extract(text: &str, format: Format) -> Extraction {
    match format {
        Format::Markdown => extract_markdown(text),
        Format::PlainText => extract_plain(text),
    }
}

/// One claim per heading. The section body's first paragraph is the excerpt; a heading
/// with no body uses its own text. A level-1 heading with no body is treated as the
/// document's title, not a claim, when other headings exist.
fn extract_markdown(text: &str) -> Extraction {
    struct Section {
        title: String,
        level: usize,
        line: usize,
        body: Vec<(usize, usize)>, // paragraph byte ranges
    }

    let mut sections: Vec<Section> = Vec::new();
    let mut paragraphs_total = 0usize;
    for (start, end, line) in paragraph_ranges(text) {
        let slice = &text[start..end];
        if let Some((level, title)) = heading_of(slice) {
            // A paragraph beginning with a heading line: the heading opens a section and
            // any remainder of the paragraph is its first body text.
            sections.push(Section {
                title,
                level,
                line,
                body: Vec::new(),
            });
            let after = slice.find('\n').map(|i| start + i + 1);
            if let Some(after) = after
                && !text[after..end].trim().is_empty()
            {
                paragraphs_total += 1;
                if let Some(section) = sections.last_mut() {
                    section.body.push((after, end));
                }
            }
        } else {
            paragraphs_total += 1;
            if let Some(section) = sections.last_mut() {
                section.body.push((start, end));
            }
        }
    }

    let multiple = sections.len() > 1;
    let mut claims = Vec::new();
    let mut used = 0usize;
    let mut slugs = SlugSet::default();
    for section in &sections {
        if section.title.is_empty() {
            continue;
        }
        if multiple && section.level == 1 && section.body.is_empty() {
            continue; // the document's own title
        }
        let excerpt = section.body.first().map_or_else(
            || section.title.clone(),
            |&(start, end)| {
                used += 1;
                text[start..end].trim_end().to_owned()
            },
        );
        claims.push(claim_of(
            &section.title,
            excerpt,
            section.line,
            &mut slugs,
            claims.len(),
        ));
    }

    Extraction {
        claims,
        paragraphs_skipped: paragraphs_total - used,
        links: links_of(text),
    }
}

/// One claim per paragraph. The first line is the title, the whole paragraph the excerpt.
fn extract_plain(text: &str) -> Extraction {
    let mut claims = Vec::new();
    let mut slugs = SlugSet::default();
    for (start, end, line) in paragraph_ranges(text) {
        let slice = text[start..end].trim_end();
        let title = slice.lines().next().unwrap_or_default().trim();
        if title.is_empty() {
            continue;
        }
        let title = ellipsize(title, 80);
        claims.push(claim_of(
            &title,
            slice.to_owned(),
            line,
            &mut slugs,
            claims.len(),
        ));
    }
    Extraction {
        claims,
        paragraphs_skipped: 0,
        links: Vec::new(),
    }
}

fn claim_of(
    title: &str,
    excerpt: String,
    line: usize,
    slugs: &mut SlugSet,
    index: usize,
) -> DraftClaim {
    DraftClaim {
        title: title.to_owned(),
        kind: classify(title, &excerpt),
        slug: slugs.claim(&slug_of(title), index),
        excerpt,
        line,
    }
}

/// The kind a claim most plausibly is, from keyword rules applied in order:
///
/// 1. a title ending in `?` is a `question`;
/// 2. "decided", "decision" or "we chose" makes a `decision`;
/// 3. "must", "shall", "ongoing", "standing", "always", "never", "every" or "policy"
///    makes a `policy` — the one kind for a standing rule, requirement or limit since
///    `requirement` and `track` were retired (D-046);
/// 4. everything else is `work` — the state a durable claim of unknown kind is safest
///    in, because a `proposed` work record asserts nothing normative.
///
/// `milestone`, `observation`, `evidence` and `assessment` are never proposed: each has
/// a required slot the document cannot honestly fill (acceptance checks, `observed_at`),
/// and inventing one would put fabricated structure in front of the reviewer.
#[must_use]
pub fn classify(title: &str, excerpt: &str) -> Kind {
    if title.trim_end().ends_with('?') {
        return Kind::Question;
    }
    let text = format!(" {} {} ", title.to_lowercase(), excerpt.to_lowercase());
    let has = |words: &[&str]| {
        words.iter().any(|w| {
            text.match_indices(w).any(|(at, _)| {
                let before = text[..at].chars().next_back().unwrap_or(' ');
                let after = text[at + w.len()..].chars().next().unwrap_or(' ');
                !before.is_alphanumeric() && !after.is_alphanumeric()
            })
        })
    };
    if has(&["decided", "decision", "we chose"]) {
        Kind::Decision
    } else if has(&[
        "must", "shall", "ongoing", "standing", "always", "never", "every", "policy",
    ]) {
        Kind::Policy
    } else {
        Kind::Work
    }
}

/// Paragraph byte ranges with their 1-based starting line, split on blank lines.
fn paragraph_ranges(text: &str) -> Vec<(usize, usize, usize)> {
    let mut ranges = Vec::new();
    let mut start: Option<(usize, usize)> = None;
    let mut offset = 0usize;
    for (line, raw) in (1usize..).zip(text.split_inclusive('\n')) {
        if raw.trim().is_empty() {
            if let Some((s, l)) = start.take() {
                ranges.push((s, offset, l));
            }
        } else if start.is_none() {
            start = Some((offset, line));
        }
        offset += raw.len();
    }
    if let Some((s, l)) = start {
        ranges.push((s, text.len(), l));
    }
    ranges
}

/// The heading level and text, when a paragraph's first line is `#`-headed.
fn heading_of(slice: &str) -> Option<(usize, String)> {
    let first = slice.lines().next()?;
    let level = first.bytes().take_while(|&b| b == b'#').count();
    if level == 0 || level > 6 {
        return None;
    }
    let rest = &first[level..];
    if !rest.starts_with(' ') && !rest.is_empty() {
        return None;
    }
    Some((
        level,
        rest.trim().trim_end_matches('#').trim_end().to_owned(),
    ))
}

/// Relative link targets: `](target)` with schemes, anchors and mailto skipped.
fn links_of(text: &str) -> Vec<LinkReference> {
    let mut links = Vec::new();
    let mut search = 0usize;
    while let Some(at) = text[search..].find("](") {
        let target_start = search + at + 2;
        let Some(close) = text[target_start..].find(')') else {
            break;
        };
        let raw = &text[target_start..target_start + close];
        search = target_start + close + 1;
        let target = raw.split('#').next().unwrap_or_default().trim();
        if target.is_empty() || target.contains("://") || raw.starts_with('#') {
            continue;
        }
        if target.starts_with("mailto:") {
            continue;
        }
        let before = &text[..target_start];
        let line = before.matches('\n').count() + 1;
        let column = before.len() - before.rfind('\n').map_or(0, |i| i + 1) + 1;
        links.push(LinkReference {
            target: target.to_owned(),
            line,
            column,
        });
    }
    links
}

/// A slug allocator: valid segments, unique within one extraction.
#[derive(Default)]
struct SlugSet {
    taken: std::collections::BTreeSet<String>,
}

impl SlugSet {
    fn claim(&mut self, base: &str, index: usize) -> String {
        let base = if base.is_empty() {
            format!("claim-{}", index + 1)
        } else {
            base.to_owned()
        };
        let mut candidate = base.clone();
        let mut n = 1usize;
        while !self.taken.insert(candidate.clone()) {
            n += 1;
            candidate = format!("{base}-{n}");
        }
        candidate
    }
}

/// A title reduced to the D-005 segment charset, truncated at a hyphen near 48 bytes.
#[must_use]
pub fn slug_of(title: &str) -> String {
    let mut slug = String::new();
    let mut hyphen = true; // suppress leading hyphens
    for c in title.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || (c.is_ascii_digit() && !slug.is_empty()) {
            slug.push(c);
            hyphen = false;
        } else if !hyphen && !slug.is_empty() {
            slug.push('-');
            hyphen = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.len() > 48 {
        let cut = slug[..48].rfind('-').unwrap_or(48);
        slug.truncate(cut);
    }
    match Segment::new(&slug) {
        Ok(_) => slug,
        Err(_) => String::new(),
    }
}

fn ellipsize(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut cut = max;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    text[..cut].trim_end().to_owned()
}

// -------------------------------------------------------------------------------------
// the check-time audit
// -------------------------------------------------------------------------------------

/// The post-import decay checks of `docs/12-migration.md` §3 and §4, over head revisions.
///
/// The audit is **anchored on tracking records**, not on legacy provenance in general.
/// A document is "under migration" exactly when a tracking `work` record cites it — a
/// `work` record carrying that legacy path *and* a non-empty acceptance block, which is
/// D-022's one-record-per-migrated-document pattern. Only those documents are audited:
///
/// - `AKR-M022` (warning) — a migrated document's `source { kind legacy }` path no longer
///   exists at HEAD. Its excerpt is now unverifiable, which a reader should know.
/// - `AKR-M032` (error) — a migrated document sits under an `archive/` directory while
///   its tracking record is not `completed`. Archiving waits for full disposition.
///
/// A `source { kind legacy }` block on a record with **no** tracking record is left
/// silent. That is not an oversight: D-022 §2 makes provenance a block repeatable on any
/// record, and a mature ledger cites the documents its knowledge derives from without
/// inventing a migration for each — the deliberate, blessed steady state of
/// `examples/sys-tandem/MANIFEST.md` §8, where a `check` exits 0 with twenty such
/// citations and one tracker. `AKR-M031` — the *absence* of a required tracker — is
/// therefore an import-time concern, guaranteed by `ops::import` always writing one, and
/// not re-derived here, because a mature ledger gives the audit no way to tell an
/// unfinished import from a permanent provenance citation.
///
/// `exists` answers whether a repo-relative path exists at HEAD; `head` is the commit
/// named in the messages.
#[must_use]
pub fn audit(ledger: &Ledger, head: &str, exists: &dyn Fn(&str) -> bool) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let heads: Vec<&crate::model::Record> = ledger
        .keys()
        .into_iter()
        .filter_map(|key| ledger.head(key).ok())
        .collect();

    let tracker_of = |path: &str| {
        heads.iter().find(|record| {
            record.kind == Kind::Work
                && record
                    .acceptance
                    .as_ref()
                    .is_some_and(|a| !a.checks.is_empty())
                && legacy_paths(record).any(|p| p == path)
        })
    };

    let mut flagged_archived: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();

    for record in &heads {
        for path in legacy_paths(record) {
            // Only documents the ledger itself marks as migrating — via a tracking work
            // record — are audited. Bare provenance citations are silent (MANIFEST §8).
            let Some(tracker) = tracker_of(path) else {
                continue;
            };
            if !exists(path) {
                diagnostics.push(Diagnostic {
                    code: migration::M022,
                    severity: Severity::Warning,
                    rule: None,
                    message: format!("{}: source path {path} does not exist at {head}", record.id),
                    primary: Label::new(Subject::Revision(record.id.clone())),
                    notes: Vec::new(),
                    help: Some(
                        "the excerpt is now unverifiable; restore the document or accept the loss"
                            .to_owned(),
                    ),
                });
            }
            if is_archived(path)
                && tracker.state != State::Completed
                && flagged_archived.insert(path.to_owned())
            {
                diagnostics.push(Diagnostic {
                    code: migration::M032,
                    severity: Severity::Error,
                    rule: None,
                    message: format!(
                        "{path} is archived but {} is in state {}",
                        tracker.id, tracker.state
                    ),
                    primary: Label::new(Subject::Revision(tracker.id.clone())),
                    notes: Vec::new(),
                    help: Some(
                        "complete the tracking record before archiving the document".to_owned(),
                    ),
                });
            }
        }
    }
    diagnostics
}

fn legacy_paths(record: &crate::model::Record) -> impl Iterator<Item = &str> {
    record
        .sources
        .iter()
        .filter(|s| s.kind == SourceKind::Legacy)
        .filter_map(|s| s.path.as_deref())
}

fn is_archived(path: &str) -> bool {
    std::path::Path::new(path)
        .components()
        .any(|c| c.as_os_str() == "archive")
}

//! Deterministic Markdown chunking for the source library.
//!
//! A registered source is immutable, so its byte ranges are stable coordinates and its
//! chunk boundaries are a pure function of (bytes, parser version). That is the whole
//! reason chunking is allowed to be approximate: a chunk boundary in the wrong place
//! costs search quality and nothing else. Project semantics live in `.akr/`; nothing
//! here can create, retire or authorise a record.
//!
//! # The rules, from `docs/15-external-sources.md` §4
//!
//! 1. Headings establish a section path and are never chunks themselves.
//! 2. Paragraphs and list groups are semantic blocks.
//! 3. A fenced code block is never split, and never opens a section.
//! 4. A table is never split.
//! 5. Consecutive blocks under one heading are packed to roughly
//!    [`TARGET_TOKENS`]..[`MAX_TOKENS`] estimated tokens.
//! 6. No chunk crosses a heading boundary.
//! 7. No overlap: `akr source get --chunk <id> --neighbors 1` is how a caller widens.
//! 8. The parser version is stored with every chunk, so a scanner change is visible
//!    rather than silent.
//!
//! # Fences before headings
//!
//! The scanner recognises a fence *before* it looks for a heading. A `# comment` line
//! inside a shell block would otherwise reset the section path, which is exactly the
//! bug a naive line-oriented heading parser has: it reads a code sample as structure.

use crate::hash::Sha256;

/// Bumped whenever a change to this module would move a chunk boundary.
///
/// Stored in `source_chunks.parser_version` and mixed into every chunk id, so a bump
/// invalidates derived chunk ids without touching a single record: record citations name
/// a document and a byte range, never a chunk (D-031).
pub const PARSER_VERSION: i64 = 1;

/// The size a packed chunk aims for.
pub const TARGET_TOKENS: usize = 450;
/// The size a packed chunk will not exceed by adding another block.
pub const MAX_TOKENS: usize = 700;

/// What sort of block a chunk was built from.
///
/// A chunk that packed several blocks together takes the kind of its first block, which
/// is only ever used for display and for ranking weight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkKind {
    /// Ordinary prose.
    Prose,
    /// One or more list items.
    List,
    /// A fenced or indented code block.
    Code,
    /// A pipe table, header and all.
    Table,
    /// A block quote.
    Quote,
}

impl ChunkKind {
    /// The name stored in `source_chunks.kind`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prose => "prose",
            Self::List => "list",
            Self::Code => "code",
            Self::Table => "table",
            Self::Quote => "quote",
        }
    }
}

/// One retrieval unit of one registered source document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceChunk {
    /// Position within the document, from zero. Display only; not identity.
    pub ordinal: u32,
    /// The heading path in force, outermost first.
    pub heading_path: Vec<String>,
    /// What the chunk mostly is.
    pub kind: ChunkKind,
    /// Byte offset of the first byte, inclusive.
    pub start_byte: u64,
    /// Byte offset one past the last byte.
    pub end_byte: u64,
    /// One-based line of `start_byte`.
    pub start_line: u32,
    /// One-based line of the last byte.
    pub end_line: u32,
    /// The exact source slice.
    pub raw_text: String,
    /// Normalised text for ranking: prose with soft wraps joined.
    pub search_text: String,
    /// Technical identifiers, expanded into their searchable variants.
    pub symbols: Vec<String>,
    /// `sha256:` + hex over `raw_text`.
    pub content_hash: String,
}

impl SourceChunk {
    /// The derived chunk id: a digest of document identity, parser version and byte range.
    ///
    /// Derived on purpose. A chunk id is a cursor into a rebuildable index, so it must
    /// change when the scanner changes; a record citation must not, which is why records
    /// cite `(document, byte range)` and never this.
    ///
    /// The document *id* goes in beside its content hash because two catalog entries may
    /// legitimately hold identical bytes — a document re-registered under a new id to
    /// supersede itself, for one — and two entries sharing a chunk id would collide.
    #[must_use]
    pub fn id(&self, document_id: &str, document_content_hash: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(document_id.as_bytes());
        hasher.update(b"\0");
        hasher.update(document_content_hash.as_bytes());
        hasher.update(b"\0");
        hasher.update(PARSER_VERSION.to_string().as_bytes());
        hasher.update(b"\0");
        hasher.update(self.start_byte.to_string().as_bytes());
        hasher.update(b"\0");
        hasher.update(self.end_byte.to_string().as_bytes());
        format!("c_{}", &hasher.finish().to_hex()[..16])
    }

    /// The heading path as one `a > b > c` string, for display and for the FTS column.
    #[must_use]
    pub fn heading(&self) -> String {
        self.heading_path.join(" > ")
    }
}

// ---------------------------------------------------------------------------------------
// the scanner
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    Heading(usize),
    Paragraph,
    List,
    Code,
    Table,
    Quote,
    Break,
}

#[derive(Debug, Clone)]
struct Block {
    kind: BlockKind,
    start: usize,
    end: usize,
    start_line: u32,
    end_line: u32,
    text: String,
}

/// Chunks a Markdown document.
///
/// Never fails and never rejects: a source is registered bytes, and refusing to index
/// something because its Markdown is unusual would make the library less useful without
/// making the ledger more correct.
#[must_use]
pub fn chunk_markdown(source: &str) -> Vec<SourceChunk> {
    let blocks = scan(source);
    pack(source, &blocks)
}

/// Splits the document into blocks, tracking line numbers as it goes.
fn scan(source: &str) -> Vec<Block> {
    let lines = line_spans(source);
    let mut blocks = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let (start, end, number) = lines[i];
        let text = &source[start..end];
        let trimmed = text.trim();

        // Blank lines separate blocks and belong to none of them.
        if trimmed.is_empty() {
            i += 1;
            continue;
        }

        // Fences first: a `#` or `|` inside a code block is content, not structure.
        if let Some(fence) = opening_fence(text) {
            let mut j = i + 1;
            while j < lines.len() {
                let (s, e, _) = lines[j];
                if closes_fence(&source[s..e], fence) {
                    j += 1;
                    break;
                }
                j += 1;
            }
            blocks.push(block(source, &lines, BlockKind::Code, i, j));
            i = j;
            continue;
        }

        if let Some(level) = heading_level(text) {
            blocks.push(block(source, &lines, BlockKind::Heading(level), i, i + 1));
            i += 1;
            continue;
        }

        if is_thematic_break(trimmed) {
            blocks.push(block(source, &lines, BlockKind::Break, i, i + 1));
            i += 1;
            continue;
        }

        if is_table_row(trimmed) {
            let mut j = i;
            while j < lines.len() {
                let (s, e, _) = lines[j];
                if !is_table_row(source[s..e].trim()) {
                    break;
                }
                j += 1;
            }
            blocks.push(block(source, &lines, BlockKind::Table, i, j));
            i = j;
            continue;
        }

        if is_list_marker(text) {
            // A list runs to the first line that is neither a marker, a continuation nor
            // an interior blank line. Continuations are indented; that is what keeps a
            // nested item and its paragraph inside one block.
            let mut j = i + 1;
            let mut last_content = i + 1;
            while j < lines.len() {
                let (s, e, _) = lines[j];
                let line = &source[s..e];
                if line.trim().is_empty() {
                    j += 1;
                    continue;
                }
                if is_list_marker(line) || indent_of(line) >= 2 {
                    if opening_fence(line).is_some() && indent_of(line) < 2 {
                        break;
                    }
                    j += 1;
                    last_content = j;
                    continue;
                }
                break;
            }
            blocks.push(block(source, &lines, BlockKind::List, i, last_content));
            i = last_content;
            continue;
        }

        let quote = trimmed.starts_with('>');
        let mut j = i + 1;
        while j < lines.len() {
            let (s, e, _) = lines[j];
            let line = &source[s..e];
            let t = line.trim();
            if t.is_empty()
                || heading_level(line).is_some()
                || opening_fence(line).is_some()
                || is_list_marker(line)
                || is_thematic_break(t)
                || is_table_row(t)
                || t.starts_with('>') != quote
            {
                break;
            }
            j += 1;
        }
        let kind = if quote {
            BlockKind::Quote
        } else {
            BlockKind::Paragraph
        };
        blocks.push(block(source, &lines, kind, i, j));
        i = j;
        let _ = number;
    }

    blocks
}

/// Packs blocks into chunks, one heading section at a time.
fn pack(source: &str, blocks: &[Block]) -> Vec<SourceChunk> {
    let mut heading_path: Vec<String> = Vec::new();
    let mut chunks: Vec<SourceChunk> = Vec::new();
    let mut pending: Vec<&Block> = Vec::new();

    let flush = |pending: &mut Vec<&Block>, path: &[String], chunks: &mut Vec<SourceChunk>| {
        if pending.is_empty() {
            return;
        }
        chunks.push(build_chunk(source, pending, path, chunks.len()));
        pending.clear();
    };

    for block in blocks {
        match block.kind {
            BlockKind::Heading(level) => {
                flush(&mut pending, &heading_path, &mut chunks);
                heading_path.truncate(level.saturating_sub(1));
                while heading_path.len() < level.saturating_sub(1) {
                    heading_path.push(String::new());
                }
                heading_path.push(heading_text(&block.text));
            }
            BlockKind::Break => {}
            _ => {
                // A block that would take the chunk past MAX_TOKENS starts a new one, and
                // a block that is itself over the maximum stands alone rather than being
                // split — splitting a code block is how a search hit stops compiling.
                let size: usize = pending.iter().map(|b| estimate_tokens(&b.text)).sum();
                let next = estimate_tokens(&block.text);
                if !pending.is_empty() && (size >= TARGET_TOKENS || size + next > MAX_TOKENS) {
                    flush(&mut pending, &heading_path, &mut chunks);
                }
                pending.push(block);
            }
        }
    }
    flush(&mut pending, &heading_path, &mut chunks);
    chunks
}

fn build_chunk(
    source: &str,
    blocks: &[&Block],
    heading_path: &[String],
    ordinal: usize,
) -> SourceChunk {
    let first = blocks[0];
    let last = blocks[blocks.len() - 1];
    // The exact slice, blank lines and all: a chunk that does not reproduce its own byte
    // range would make `akr source get --chunk` a paraphrase of the registered source.
    let raw_text = source[first.start..last.end].to_owned();
    let kind = match first.kind {
        BlockKind::Code => ChunkKind::Code,
        BlockKind::Table => ChunkKind::Table,
        BlockKind::List => ChunkKind::List,
        BlockKind::Quote => ChunkKind::Quote,
        _ => ChunkKind::Prose,
    };
    let search_text = normalise(&raw_text, kind);
    let symbols = symbols_of(&raw_text);
    let mut hasher = Sha256::new();
    hasher.update(raw_text.as_bytes());
    SourceChunk {
        ordinal: u32::try_from(ordinal).unwrap_or(u32::MAX),
        heading_path: heading_path
            .iter()
            .filter(|segment| !segment.is_empty())
            .cloned()
            .collect(),
        kind,
        start_byte: first.start as u64,
        end_byte: last.end as u64,
        start_line: first.start_line,
        end_line: last.end_line,
        content_hash: format!("sha256:{}", hasher.finish().to_hex()),
        raw_text,
        search_text,
        symbols,
    }
}

// ---------------------------------------------------------------------------------------
// normalisation
// ---------------------------------------------------------------------------------------

/// The ranking form of a chunk: soft wraps joined, markers dropped, fences kept as text.
///
/// Joining wrapped lines is the point. `docs/15-external-sources.md` §4: two documents
/// that differ only in where their prose wraps must rank identically, or source
/// formatting would decide search results.
fn normalise(raw: &str, kind: ChunkKind) -> String {
    if kind == ChunkKind::Code {
        return raw
            .lines()
            .filter(|line| !line.trim_start().starts_with("```"))
            .collect::<Vec<_>>()
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
    }
    let mut out = String::new();
    for line in raw.lines() {
        let mut text = line.trim();
        for marker in ["- ", "* ", "+ ", "> ", "| "] {
            text = text.strip_prefix(marker).unwrap_or(text);
        }
        if text.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(text);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Technical identifiers, each expanded into the forms somebody might type.
///
/// `DecodeRequest::default()` is one token to a human and four to a tokeniser, and the
/// query is as likely to be `decode request default` as the exact call. Expanding at
/// index time costs a column; expanding at query time would cost correctness, because
/// the expansion would have to guess which words were ever one identifier.
fn symbols_of(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |value: String| {
        if !value.is_empty() && !out.contains(&value) {
            out.push(value);
        }
    };
    for token in raw.split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '`' | '"')) {
        let token = token.trim_matches(|c: char| matches!(c, '(' | ')' | '.' | '*' | '_' | '\''));
        if token.len() < 3 || token.len() > 120 {
            continue;
        }
        // An *interior* capital, not merely a capital: "The" starts a sentence, whereas
        // "DecodeRequest" is a name. Treating every capitalised word as an identifier
        // would fill the symbol column with the first word of every sentence.
        let camel_case = if token.chars().skip(1).any(char::is_uppercase) {
            token.chars().any(char::is_lowercase)
        } else {
            false
        };
        let technical = token.contains("::")
            || token.contains('_')
            || token.contains('/')
            || (token.contains('.') && !token.ends_with('.'))
            || camel_case;
        if !technical || !token.chars().any(char::is_alphanumeric) {
            continue;
        }
        push(token.to_owned());
        let split: Vec<String> = token
            .split([':', '_', '/', '.', '-'])
            .filter(|part| !part.is_empty())
            .flat_map(split_camel)
            .collect();
        if split.len() > 1 {
            push(split.join(" ").to_lowercase());
        }
        for part in split {
            if part.len() >= 3 {
                push(part.to_lowercase());
            }
        }
    }
    out
}

fn split_camel(part: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for c in part.chars() {
        if c.is_uppercase() && !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
        current.push(c);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// The same crude estimate the context budget uses: words plus punctuation.
#[must_use]
pub fn estimate_tokens(text: &str) -> usize {
    text.split_whitespace()
        .map(|word| 1 + word.chars().filter(|c| c.is_ascii_punctuation()).count() / 3)
        .sum()
}

// ---------------------------------------------------------------------------------------
// line-level predicates
// ---------------------------------------------------------------------------------------

/// `(start, end, one-based line number)` for every line, newline excluded.
fn line_spans(source: &str) -> Vec<(usize, usize, u32)> {
    let mut out = Vec::new();
    let bytes = source.as_bytes();
    let mut start = 0;
    let mut number = 1;
    for (i, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            let mut end = i;
            if end > start && bytes[end - 1] == b'\r' {
                end -= 1;
            }
            out.push((start, end, number));
            number += 1;
            start = i + 1;
        }
    }
    if start < bytes.len() {
        out.push((start, bytes.len(), number));
    }
    out
}

fn block(
    source: &str,
    lines: &[(usize, usize, u32)],
    kind: BlockKind,
    from: usize,
    to: usize,
) -> Block {
    let to = to.max(from + 1).min(lines.len());
    let (start, _, start_line) = lines[from];
    let (_, end, end_line) = lines[to - 1];
    Block {
        kind,
        start,
        end,
        start_line,
        end_line,
        text: source[start..end].to_owned(),
    }
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// The fence character and run length, when this line opens one.
fn opening_fence(line: &str) -> Option<(char, usize)> {
    let trimmed = line.trim_start();
    if indent_of(line) >= 4 {
        return None;
    }
    for marker in ['`', '~'] {
        let run = trimmed.chars().take_while(|c| *c == marker).count();
        if run >= 3 {
            return Some((marker, run));
        }
    }
    None
}

fn closes_fence(line: &str, fence: (char, usize)) -> bool {
    let trimmed = line.trim();
    let run = trimmed.chars().take_while(|c| *c == fence.0).count();
    run >= fence.1 && trimmed.len() == run
}

fn heading_level(line: &str) -> Option<usize> {
    if indent_of(line) >= 4 {
        return None;
    }
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ') {
        return Some(hashes);
    }
    None
}

fn heading_text(line: &str) -> String {
    line.trim_start()
        .trim_start_matches('#')
        .trim()
        .trim_end_matches('#')
        .trim()
        .to_owned()
}

fn is_thematic_break(trimmed: &str) -> bool {
    if trimmed.len() < 3 {
        return false;
    }
    ['-', '*', '_']
        .iter()
        .any(|marker| trimmed.chars().all(|c| c == *marker))
}

fn is_table_row(trimmed: &str) -> bool {
    trimmed.starts_with('|') && trimmed.len() > 1
}

fn is_list_marker(line: &str) -> bool {
    if indent_of(line) >= 4 {
        return false;
    }
    let trimmed = line.trim_start();
    if is_thematic_break(trimmed) {
        return false;
    }
    for marker in ["- ", "* ", "+ "] {
        if trimmed.starts_with(marker) {
            return true;
        }
    }
    let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && trimmed[digits..].starts_with('.') && trimmed[digits + 1..].starts_with(' ')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_heading_sets_the_path_and_is_not_itself_a_chunk() {
        let chunks = chunk_markdown("# One\n\nAlpha.\n\n## Two\n\nBeta.\n");
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].heading_path, vec!["One".to_owned()]);
        assert_eq!(
            chunks[1].heading_path,
            vec!["One".to_owned(), "Two".to_owned()]
        );
        assert!(!chunks.iter().any(|c| c.raw_text.starts_with('#')));
    }

    #[test]
    fn a_hash_inside_a_fence_does_not_move_the_section() {
        // The bug a line-oriented heading parser has, pinned.
        let source = "# Real\n\n```sh\n# not a heading\necho hi\n```\n\nAfter.\n";
        let chunks = chunk_markdown(source);
        assert!(
            chunks
                .iter()
                .all(|c| c.heading_path == vec!["Real".to_owned()]),
            "{:?}",
            chunks.iter().map(SourceChunk::heading).collect::<Vec<_>>()
        );
    }

    #[test]
    fn wrapping_prose_differently_does_not_change_the_search_text() {
        let wrapped = chunk_markdown(
            "Repair the full-plane allocation before adding\narchitecture-specific SIMD.\n",
        );
        let flat = chunk_markdown(
            "Repair the full-plane allocation before adding architecture-specific SIMD.\n",
        );
        assert_eq!(wrapped.len(), 1);
        assert_eq!(wrapped[0].search_text, flat[0].search_text);
    }

    #[test]
    fn a_code_block_is_never_split() {
        let body = (0..400)
            .map(|n| format!("let x{n} = {n};"))
            .collect::<Vec<_>>()
            .join("\n");
        let chunks = chunk_markdown(&format!("# H\n\n```rust\n{body}\n```\n"));
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].raw_text.contains("let x399"));
        assert_eq!(chunks[0].kind, ChunkKind::Code);
    }

    #[test]
    fn a_table_stays_whole() {
        let source = "| a | b |\n| - | - |\n| 1 | 2 |\n| 3 | 4 |\n";
        let chunks = chunk_markdown(source);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].kind, ChunkKind::Table);
        assert!(chunks[0].raw_text.contains("| 3 | 4 |"));
    }

    #[test]
    fn byte_ranges_slice_the_original_exactly() {
        let source = "# H\n\nAlpha beta.\n\n- one\n- two\n\n```\ncode\n```\n";
        for chunk in chunk_markdown(source) {
            let start = usize::try_from(chunk.start_byte).unwrap();
            let end = usize::try_from(chunk.end_byte).unwrap();
            assert_eq!(&source[start..end], chunk.raw_text, "{chunk:?}");
        }
    }

    #[test]
    fn line_numbers_agree_with_the_byte_range() {
        let source = "# H\n\nAlpha.\n\nBeta.\n";
        for chunk in chunk_markdown(source) {
            let start = usize::try_from(chunk.start_byte).unwrap();
            let line = source[..start].matches('\n').count() + 1;
            assert_eq!(u32::try_from(line).unwrap(), chunk.start_line);
        }
    }

    #[test]
    fn symbols_expand_technical_identifiers() {
        let symbols = symbols_of("Call DecodeRequest::default() before inverse_97_2d_in_place.");
        assert!(symbols.iter().any(|s| s == "DecodeRequest::default"));
        assert!(symbols.iter().any(|s| s == "decode request default"));
        assert!(symbols.iter().any(|s| s == "inverse_97_2d_in_place"));
        assert!(symbols.iter().any(|s| s == "place"));
    }

    #[test]
    fn ordinary_prose_contributes_no_symbols() {
        assert!(symbols_of("The quick brown fox jumped over the lazy dog.").is_empty());
    }

    #[test]
    fn a_chunk_id_changes_with_the_document_but_not_with_its_neighbours() {
        let chunks = chunk_markdown("# H\n\nAlpha.\n\n## I\n\nBeta.\n");
        let a = chunks[0].id("one", "sha256:aa");
        assert_ne!(a, chunks[0].id("one", "sha256:bb"));
        // Identical bytes registered twice are still two documents.
        assert_ne!(a, chunks[0].id("two", "sha256:aa"));
        assert_eq!(a, chunks[0].id("one", "sha256:aa"));
    }

    #[test]
    fn packing_keeps_a_chunk_under_the_maximum() {
        let paragraph = "word ".repeat(400);
        let source = format!("# H\n\n{paragraph}\n\n{paragraph}\n\n{paragraph}\n");
        let chunks = chunk_markdown(&source);
        assert_eq!(chunks.len(), 3, "each paragraph is its own chunk");
        for chunk in &chunks {
            assert!(
                estimate_tokens(&chunk.raw_text) <= MAX_TOKENS,
                "{} tokens",
                estimate_tokens(&chunk.raw_text)
            );
        }
    }

    #[test]
    fn small_neighbouring_blocks_pack_together() {
        let source = "# H\n\nOne.\n\nTwo.\n\nThree.\n";
        assert_eq!(chunk_markdown(source).len(), 1);
    }

    #[test]
    fn a_chunk_never_crosses_a_heading() {
        let source = "# A\n\nOne.\n\n# B\n\nTwo.\n";
        let chunks = chunk_markdown(source);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].heading_path, vec!["A".to_owned()]);
        assert_eq!(chunks[1].heading_path, vec!["B".to_owned()]);
    }
}

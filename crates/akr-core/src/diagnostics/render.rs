//! Source maps and the rendered diagnostic form of `spec/diagnostics/README.md` §5.

use super::{Diagnostic, FileId, Severity, Span};

/// One source file the diagnostics can point into.
#[derive(Debug, Clone)]
pub struct SourceFile {
    /// Its identifier.
    pub id: FileId,
    /// Its path, as it should appear in a diagnostic.
    pub path: String,
    /// Its contents.
    pub text: String,
    line_starts: Vec<u32>,
}

impl SourceFile {
    /// Indexes a file's line starts.
    #[must_use]
    pub fn new(id: FileId, path: &str, text: &str) -> Self {
        let mut line_starts = vec![0];
        for (offset, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(u32::try_from(offset + 1).unwrap_or(u32::MAX));
            }
        }
        Self {
            id,
            path: path.to_owned(),
            text: text.to_owned(),
            line_starts,
        }
    }

    /// The 1-based line and column of a byte offset.
    ///
    /// The column counts characters, not bytes, so a diagnostic on a line containing
    /// non-ASCII prose still lines its caret up.
    #[must_use]
    pub fn location(&self, offset: u32) -> (u32, u32) {
        let line_index = self
            .line_starts
            .partition_point(|start| *start <= offset)
            .max(1)
            - 1;
        let line_start = self.line_starts[line_index] as usize;
        let end = (offset as usize).min(self.text.len());
        let column = self.text[line_start..end].chars().count() + 1;
        (
            u32::try_from(line_index + 1).unwrap_or(u32::MAX),
            u32::try_from(column).unwrap_or(u32::MAX),
        )
    }

    /// The text of a 1-based line, without its newline.
    #[must_use]
    pub fn line(&self, line: u32) -> &str {
        let index = (line as usize).saturating_sub(1);
        let Some(start) = self.line_starts.get(index) else {
            return "";
        };
        let end = self
            .line_starts
            .get(index + 1)
            .map_or(self.text.len(), |next| (*next as usize).saturating_sub(1));
        &self.text[*start as usize..end.min(self.text.len())]
    }
}

/// Every file a diagnostic might point into.
#[derive(Debug, Clone, Default)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    /// An empty map.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a file and returns its identifier.
    pub fn add(&mut self, path: &str, text: &str) -> FileId {
        let id = FileId(u32::try_from(self.files.len()).unwrap_or(u32::MAX));
        self.files.push(SourceFile::new(id, path, text));
        id
    }

    /// Looks a file up.
    #[must_use]
    pub fn get(&self, id: FileId) -> Option<&SourceFile> {
        self.files.iter().find(|f| f.id == id)
    }

    /// Every file, in the order they were added.
    pub fn iter(&self) -> impl Iterator<Item = &SourceFile> {
        self.files.iter()
    }
}

/// Renders a diagnostic in the caret form.
///
/// A diagnostic with no span renders its header, notes and help without the source
/// excerpt, which is what P1's span-less diagnostics do until P3 attaches spans through
/// [`super::Subject`].
#[must_use]
pub fn render(diagnostic: &Diagnostic, sources: &SourceMap) -> String {
    let severity = match diagnostic.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    };
    let mut out = format!("{severity}[{}]: {}\n", diagnostic.code, diagnostic.message);
    if let Some(excerpt) = excerpt(
        diagnostic.primary.span,
        diagnostic.primary.message.as_deref(),
        sources,
    ) {
        out.push_str(&excerpt);
    }
    for note in &diagnostic.notes {
        out.push_str(&format!(
            "note: {}\n",
            note.message.as_deref().unwrap_or("see also")
        ));
        if let Some(excerpt) = location_line(note.span, sources) {
            out.push_str(&excerpt);
        }
    }
    if let Some(help) = &diagnostic.help {
        let rule = diagnostic
            .rule
            .map(|r| format!(" (see {r})"))
            .unwrap_or_default();
        out.push_str(&format!("help: {help}{rule}\n"));
    }
    out
}

fn location_line(span: Option<Span>, sources: &SourceMap) -> Option<String> {
    let span = span?;
    let file = sources.get(span.file)?;
    let (line, column) = file.location(span.start);
    Some(format!("  --> {}:{line}:{column}\n", file.path))
}

fn excerpt(span: Option<Span>, message: Option<&str>, sources: &SourceMap) -> Option<String> {
    let span = span?;
    let file = sources.get(span.file)?;
    let (line, column) = file.location(span.start);
    let text = file.line(line);
    let gutter = line.to_string().len().max(2);
    let pad = " ".repeat(gutter);

    let start_char = column as usize - 1;
    let end_char = {
        let (end_line, end_column) = file.location(span.end);
        if end_line == line {
            end_column as usize - 1
        } else {
            text.chars().count()
        }
    };
    let width = end_char.saturating_sub(start_char).max(1);

    let mut out = format!("  --> {}:{line}:{column}\n", file.path);
    out.push_str(&format!("{pad} |\n"));
    out.push_str(&format!("{line:>gutter$} | {text}\n"));
    out.push_str(&format!(
        "{pad} | {}{}{}\n",
        " ".repeat(start_char),
        "^".repeat(width),
        message.map(|m| format!(" {m}")).unwrap_or_default()
    ));
    out.push_str(&format!("{pad} |\n"));
    Some(out)
}

//! Collating papercuts from sister projects into a master record (D-030).
//!
//! `akr papercut collate` reads the live papercut heads of every workspace under a scan
//! directory — by default the siblings of the AKR workspace — and gathers every one not
//! already absorbed into a single new master papercut record in the AKR ledger. The
//! source keys land in the record's `collated` slot, which is the dedup set the next run
//! checks: a source papercut is processed once, and the sisters are never written to.
//!
//! Reading is all this module does; serialising and writing are the caller's, through the
//! one write pipeline of `docs/07-cli.md` §4, exactly as `akr papercut` itself works
//! (D-027).

use crate::model::{
    Commit, ContentSlot, ContentValue, Date, Kind, Ledger, LogicalKey, Record, RevisionId, State,
};
use crate::resolve::load_workspace;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// One source papercut, as the master record will list it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollatedPapercut {
    /// The sibling project's directory name.
    pub project: String,
    /// The source papercut's logical key, e.g. `bpg.papercut.fts5-error`.
    pub key: LogicalKey,
    /// The source papercut's one-line title.
    pub title: String,
    /// The source papercut's `about` subject, when it declared one (D-033).
    pub about: Option<String>,
    /// The source papercut's full statement.
    ///
    /// Carried, not summarised. The first collation stored keys and truncated titles and
    /// told the reader to go and open the owning project's ledger — which is eight
    /// checkouts for eighteen papercuts, and is the reason none of them were acted on.
    /// A collation that cannot be worked from is a list of homework, not a report.
    pub statement: String,
}

/// Which of the sisters' papercuts a collation absorbs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Subject {
    /// Only those whose `about` names one of these subjects.
    ///
    /// Several, because one tool is not one name. A papercut is written wherever the
    /// agent was working, by whichever agent was working, and the subject is free text:
    /// the same tool arrives as `akr`, `AKR`, `akr-mcp` and `akr CLI`. A filter that
    /// accepted exactly one spelling left the rest in the pile it was supposed to clear.
    Named(Vec<String>),
    /// Only those with no `about` at all — the sister project's own frictions.
    ///
    /// Not the default, because the interesting case is the opposite one: a papercut
    /// about *this* tool, logged wherever the agent happened to be working.
    Untagged,
    /// Every live papercut, whatever it is about.
    #[default]
    Any,
}

impl Subject {
    fn accepts(&self, about: Option<&str>) -> bool {
        match self {
            Self::Named(names) => about.is_some_and(|about| {
                let about = fold(about);
                names.iter().any(|name| fold(name) == about)
            }),
            Self::Untagged => about.is_none(),
            Self::Any => true,
        }
    }
}

/// A subject reduced to what two spellings of the same name have in common.
///
/// Case and punctuation are how a free-text subject varies between the agents that write
/// it — `akr-mcp`, `AKR MCP`, `akr_mcp` are one subject and three strings — so they are
/// what the comparison drops. Nothing else is folded: `akr` and `akr-mcp` stay two
/// subjects, because they are, and merging them would be the same silent over-reach in
/// the other direction.
fn fold(subject: &str) -> String {
    subject
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// One `about` subject seen in a scan, and how much of it was left behind.
///
/// The count is the point. "155 left behind" tells a reader they have missed something
/// and nothing about what to do next; the same 155 broken down as `ripgrep 30, cargo 18,
/// AKR 2` turns the leftovers into a decision — the two are this project's under another
/// spelling, and the rest belong to whoever owns ripgrep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectTally {
    /// The subject as written, or `None` for papercuts with no `about` at all.
    pub subject: Option<String>,
    /// How many live, uncollated papercuts carried it.
    pub total: usize,
    /// How many of those the subject filter refused.
    pub left_behind: usize,
}

/// What a scan of one directory found, and what remains to be collated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collate {
    /// The scan directory that was read.
    pub source: String,
    /// Directories under `source` that hold a workspace.
    pub projects: Vec<String>,
    /// Workspaces that failed to load and were skipped, by directory name.
    pub skipped: Vec<String>,
    /// Papercuts not yet collated anywhere, sorted by project then key.
    pub entries: Vec<CollatedPapercut>,
    /// Live papercuts left behind because their subject did not match.
    ///
    /// Counted rather than silently dropped: a collation that quietly ignored two thirds
    /// of what it read would be worse than no collation, because it would look complete.
    pub filtered_out: usize,
    /// Every `about` subject seen while scanning, sorted, with its tally.
    pub subjects_seen: Vec<SubjectTally>,
}

/// The keys a ledger has already absorbed, from every `collated` slot in its history.
///
/// A terminal master means its reports were resolved or dispositioned; it does not make
/// the source reports new again. Plain `akr papercut` records never carry the slot, so
/// they contribute nothing; only collation records do.
#[must_use]
pub fn already_collated(ledger: &Ledger) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for record in ledger.records() {
        if record.kind != Kind::Papercut {
            continue;
        }
        if let Some(ContentValue::Strings(keys)) = record.get(ContentSlot::Collated) {
            out.extend(keys.iter().cloned());
        }
    }
    out
}

/// Scans `scan_dir` for sibling workspaces and gathers their live papercut heads.
///
/// A sibling is a direct subdirectory of `scan_dir` that has an `.akr/` directory.
/// `exclude` names the caller's own workspace root, which is skipped even when it sits
/// inside `scan_dir`. Projects whose workspace fails to load are recorded in
/// `skipped` rather than failing the scan, so one broken sibling cannot block the rest.
/// `already` is the dedup set; keys in it are left out of `entries`.
#[must_use]
pub fn collect(
    scan_dir: &Path,
    exclude: &Path,
    already: &BTreeSet<String>,
    subject: &Subject,
) -> Collate {
    let source = scan_dir.to_string_lossy().into_owned();
    let mut projects = Vec::new();
    let mut skipped = Vec::new();
    let mut entries = Vec::new();
    let mut filtered_out = 0usize;
    let mut subjects: BTreeMap<Option<String>, (usize, usize)> = BTreeMap::new();

    let Ok(read) = std::fs::read_dir(scan_dir) else {
        return Collate {
            source,
            projects,
            skipped,
            entries,
            filtered_out,
            subjects_seen: Vec::new(),
        };
    };
    let mut dirs: Vec<std::path::PathBuf> = read
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();

    for dir in dirs {
        if same_dir(&dir, exclude) {
            continue;
        }
        let project = dir
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        let akr_dir = dir.join(".akr");
        if !akr_dir.is_dir() {
            continue;
        }
        let workspace = match load_workspace(&dir, &akr_dir) {
            Ok(workspace) => workspace,
            Err(_) => {
                skipped.push(project);
                continue;
            }
        };
        projects.push(project.clone());

        let mut keys: Vec<LogicalKey> = workspace
            .ledger
            .records()
            .iter()
            .filter(|record| record.kind == Kind::Papercut && record.is_live())
            .map(|record| record.id.key.clone())
            .collect();
        keys.sort();
        for key in keys {
            if already.contains(&key.to_string()) {
                continue;
            }
            let head = workspace.ledger.head(&key).ok();
            let title = head.map_or_else(String::new, |record| record.title.clone());
            let about = head.and_then(about_of);
            let statement = head.and_then(statement_of).unwrap_or_default();
            let tally = subjects.entry(about.clone()).or_insert((0, 0));
            tally.0 += 1;
            if !subject.accepts(about.as_deref()) {
                tally.1 += 1;
                filtered_out += 1;
                continue;
            }
            entries.push(CollatedPapercut {
                project: project.clone(),
                key,
                title,
                about,
                statement,
            });
        }
    }

    Collate {
        source,
        projects,
        skipped,
        entries,
        filtered_out,
        subjects_seen: subjects
            .into_iter()
            .map(|(subject, (total, left_behind))| SubjectTally {
                subject,
                total,
                left_behind,
            })
            .collect(),
    }
}

/// The `about` subject of a papercut record, if it declared one.
#[must_use]
pub fn about_of(record: &Record) -> Option<String> {
    match record.get(ContentSlot::About) {
        Some(ContentValue::Text(text) | ContentValue::Prose(text)) => Some(text.clone()),
        _ => None,
    }
}

/// A papercut's statement, flattened to one paragraph.
fn statement_of(record: &Record) -> Option<String> {
    match record.get(ContentSlot::Statement) {
        Some(ContentValue::Prose(text) | ContentValue::Text(text)) => Some(
            text.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join(" "),
        ),
        _ => None,
    }
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// The master record `akr papercut collate` proposes.
///
/// Like [`super::LogPapercut`], this builds the record; serialising and writing are the
/// caller's. The message is generated here — the collation is the ceremony — and the
/// `collated` slot is the structured dedup key the next run reads back.
pub struct CollateRequest {
    /// The scan directory, named in the statement.
    pub source: String,
    /// Every project that contributed an entry, for the title.
    pub projects: Vec<String>,
    /// The papercuts being absorbed.
    pub entries: Vec<CollatedPapercut>,
    /// The commit the collation is observed at.
    pub observed_at: Commit,
    /// The authoring date.
    pub created_at: Date,
    /// Who ran it; lands in the `author` slot.
    pub author: Option<String>,
    /// The subject the collation was narrowed to, if any; lands in `about` (D-033).
    pub about: Option<String>,
}

impl CollateRequest {
    /// Builds the record this request describes, without validating anything.
    #[must_use]
    pub fn to_record(&self, key: LogicalKey) -> Record {
        let mut content: BTreeMap<ContentSlot, ContentValue> = BTreeMap::new();
        content.insert(
            ContentSlot::Statement,
            ContentValue::Prose(self.statement()),
        );
        content.insert(
            ContentSlot::ObservedAt,
            ContentValue::Commit(self.observed_at.clone()),
        );
        content.insert(
            ContentSlot::Collated,
            ContentValue::Strings(self.entries.iter().map(|e| e.key.to_string()).collect()),
        );
        if let Some(about) = &self.about {
            content.insert(ContentSlot::About, ContentValue::Text(about.clone()));
        }

        Record {
            id: RevisionId::new(key, 1),
            kind: Kind::Papercut,
            title: self.title(),
            // Empirical kinds have no proposal state (D-027).
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
            created_at: Some(self.created_at),
            sources: Vec::new(),
            file: None,
        }
    }

    /// The one-line label: how many papercuts, from which projects.
    fn title(&self) -> String {
        let projects = project_list(&self.projects, 4);
        format!(
            "Collated {} papercut{} from {}",
            self.entries.len(),
            if self.entries.len() == 1 { "" } else { "s" },
            projects
        )
    }

    /// The body: the provenance line, then each absorbed papercut in full.
    ///
    /// In full, deliberately. The master record is the only copy of these findings in
    /// this repository, and a summary that ends "see the owning project's ledger" makes
    /// acting on eighteen papercuts an eight-checkout errand — which is how the first
    /// collation came to be read once and worked from never.
    fn statement(&self) -> String {
        let mut out = format!(
            "Collated {} papercut{} from {} on {} (D-030). Each source key is also in the \
             collated slot, which is the dedup set for the next run; the sisters were read \
             and never written.\n\n",
            self.entries.len(),
            if self.entries.len() == 1 { "" } else { "s" },
            self.source,
            self.created_at,
        );
        for entry in &self.entries {
            let about = entry
                .about
                .as_deref()
                .map_or_else(String::new, |about| format!(" [about {about}]"));
            out.push_str(&format!(
                "## {} @{}{about}\n{}\n\n",
                entry.project,
                entry.key,
                if entry.statement.is_empty() {
                    entry.title.clone()
                } else {
                    entry.statement.clone()
                },
            ));
        }
        out
    }
}

/// A comma-separated list of names, truncated to `limit` with an "and N more" tail.
fn project_list(names: &[String], limit: usize) -> String {
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    match names.len() {
        0 => "no projects".to_owned(),
        1 => names[0].to_owned(),
        n if n <= limit => format!("{} and {}", names[..n - 1].join(", "), names[n - 1]),
        _ => format!(
            "{} and {} more",
            names[..limit].join(", "),
            names.len() - limit
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_list_truncates() {
        assert_eq!(project_list(&[], 4), "no projects");
        assert_eq!(project_list(&["a".to_owned()], 4), "a");
        assert_eq!(
            project_list(&["a".to_owned(), "b".to_owned()], 4),
            "a and b"
        );
        assert_eq!(
            project_list(&["a".to_owned(), "b".to_owned(), "c".to_owned()], 2),
            "a, b and 1 more"
        );
    }
}

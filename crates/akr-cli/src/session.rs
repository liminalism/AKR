//! Locating and loading a workspace, and the exit-status contract.
//!
//! Every command starts the same way (`docs/07-cli.md` §1): walk up from `--dir` for a
//! `.akr/` directory, read `project.akr`, then run as much of the pipeline as the command
//! needs. The failures along that path are environment failures, exit status 3 — "fix the
//! checkout", as distinct from exit 1's "fix the ledger".

use crate::args::{Format, Global, Profile};
use akr_core::diagnostics::{Diagnostic, Severity, SourceMap, render};
use akr_core::freshness::ReviewQueue;
use akr_core::git::Repository;
use akr_core::json::Value;
use akr_core::model::{Commit, Date, Ledger};
use akr_core::render::Freshness;
use akr_core::resolve::{BuildInputs, ResolvedModel, SpanIndex, Workspace, load_workspace};
use std::path::{Path, PathBuf};

/// The tool version reported everywhere: the banner, the lock, the JSON envelope.
///
/// Read from the crate rather than written out, because a hand-maintained copy drifts
/// silently and this one had: the workspace reached 0.3.0 while every lock file and every
/// generated view header went on attributing itself to "akr 0.1.0". The lock records the
/// tool *that wrote it*, so a stale constant makes the provenance field say the wrong
/// thing — in the one place a provenance tool cannot afford to.
pub const TOOL_VERSION: &str = env!("CARGO_PKG_VERSION");
/// The grammar version this build speaks.
pub const GRAMMAR_VERSION: &str = "0.1";
/// The vocabulary version this build was checked against.
pub const VOCABULARY_VERSION: &str = "0.2";

/// Process exit statuses (`docs/07-cli.md` §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// The command did what it was asked.
    Ok = 0,
    /// One or more diagnostics of effective severity `error`.
    Diagnostics = 1,
    /// The invocation was malformed. Nothing was read and nothing was written.
    Usage = 2,
    /// The workspace or repository is unusable. Not a ledger problem.
    Environment = 3,
}

/// An environment failure: exit status 3.
#[derive(Debug, Clone)]
pub struct EnvError {
    /// The `AKR-C0nn` or `AKR-G0nn` code.
    pub code: &'static str,
    /// The message.
    pub message: String,
    /// The `help:` line.
    pub help: Option<String>,
}

impl EnvError {
    /// An environment failure with a code and a message.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            help: None,
        }
    }

    /// Adds a `help:` line.
    #[must_use]
    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }
}

impl std::fmt::Display for EnvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "error[{}]: {}", self.code, self.message)
    }
}

/// A located workspace, loaded and resolved.
pub struct Session {
    /// Where the repository root is.
    pub root: PathBuf,
    /// Where `.akr/` is.
    pub akr_dir: PathBuf,
    /// The ledger, with facts attached.
    pub ledger: Ledger,
    /// Build inputs, including canonical text per revision.
    pub inputs: BuildInputs,
    /// Diagnostics from stages A and B.
    pub parse_diagnostics: Vec<Diagnostic>,
    /// The lock text, if the workspace has one.
    pub lock_text: Option<String>,
    /// Every source file, for rendering diagnostics.
    pub sources: SourceMap,
    /// Subject-to-span, for attaching locations.
    pub spans: SpanIndex,
    /// The repository, when one is available.
    pub repository: Option<Repository>,
    /// The commit the build resolved against.
    pub commit: Option<Commit>,
    /// The date `review_after` is compared against.
    pub today: Date,
    /// The global flags.
    pub global: Global,
    /// Whether repository identity, history facts and source provenance have been loaded.
    ///
    /// Search can answer from a current disposable index without any of those facts.  A
    /// long-lived MCP server therefore starts with a ledger-only session and upgrades it
    /// only when a command actually asks a Git-backed question.
    git_facts_loaded: bool,
}

/// Walks up from `start` looking for a `.akr/` directory.
///
/// # Errors
/// [`EnvError`] `AKR-C011` when there is none, and `AKR-C012` when it has no
/// `project.akr`.
pub fn locate(start: &Path) -> Result<(PathBuf, PathBuf), EnvError> {
    let start = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());
    let mut cursor = start.as_path();
    loop {
        let candidate = cursor.join(".akr");
        if candidate.is_dir() {
            if !candidate.join("project.akr").is_file() {
                return Err(EnvError::new(
                    "AKR-C012",
                    format!("{}/.akr/project.akr is missing", cursor.display()),
                )
                .help("a workspace without a project file declares no namespaces"));
            }
            return Ok((cursor.to_path_buf(), candidate));
        }
        match cursor.parent() {
            Some(parent) => cursor = parent,
            None => {
                return Err(EnvError::new(
                    "AKR-C011",
                    format!(
                        "no .akr directory found in {} or any parent",
                        start.display()
                    ),
                )
                .help("run `akr init` to create one"));
            }
        }
    }
}

impl Session {
    /// Loads the workspace `global.dir` sits in.
    ///
    /// Git is optional: a workspace outside a repository still parses, resolves and
    /// renders, and only the freshness half is unavailable. That keeps `akr fmt` and
    /// `akr get` usable in a directory somebody is still setting up.
    ///
    /// # Errors
    /// [`EnvError`] for a missing workspace, an unreadable one, or a shallow repository.
    pub fn open(global: &Global) -> Result<Self, EnvError> {
        let mut session = Self::open_ledger(global)?;
        session.ensure_git_facts()?;
        Ok(session)
    }

    /// Loads and resolves the AKR sources without spawning Git.
    ///
    /// Callers must use [`Self::ensure_git_facts`] before freshness, provenance or
    /// history-dependent work.  This split is primarily for `search`/`start`: when their
    /// disposable index is current, Git is not part of the answer at all.
    ///
    /// # Errors
    /// [`EnvError`] for a missing or unreadable workspace.
    pub fn open_ledger(global: &Global) -> Result<Self, EnvError> {
        let (root, akr_dir) = locate(&global.dir)?;
        let workspace: Workspace = load_workspace(&root, &akr_dir).map_err(|error| {
            EnvError::new(
                "AKR-C011",
                format!("cannot read {}: {error}", akr_dir.display()),
            )
        })?;

        let today = global.today.unwrap_or_else(current_date);
        let mut inputs = workspace.inputs;
        inputs.tool = format!("akr {TOOL_VERSION}");
        inputs.grammar = GRAMMAR_VERSION.to_owned();
        inputs.vocabulary = VOCABULARY_VERSION.to_owned();

        Ok(Self {
            root,
            akr_dir,
            ledger: workspace.ledger,
            inputs,
            parse_diagnostics: workspace.diagnostics,
            lock_text: workspace.lock_text,
            sources: workspace.sources,
            spans: workspace.spans,
            repository: None,
            commit: None,
            today,
            global: global.clone(),
            git_facts_loaded: false,
        })
    }

    /// Adds repository identity, historical facts and exact source provenance once.
    ///
    /// Repeated calls are free.  A session is an immutable view of the source graph it
    /// loaded, so facts never need to be refreshed in-place; callers create a new session
    /// after a write or external edit.
    ///
    /// # Errors
    /// [`EnvError`] when a requested commit is absent or repository history is shallow.
    pub fn ensure_git_facts(&mut self) -> Result<(), EnvError> {
        if self.git_facts_loaded {
            return Ok(());
        }
        // `shared` rather than `open`: a one-shot command pays two extra queries for the
        // freshness check, and a long-lived host — the MCP server, which opens a session
        // per tool call — reuses everything git already answered about an unchanged
        // repository instead of re-asking per call.
        let repository = match Repository::shared(&self.root) {
            Ok(repository) => Some(repository),
            Err(akr_core::git::GitError::ShallowHistory) => {
                return Err(EnvError::new(
                    "AKR-G003",
                    "repository history is shallow; ancestry cannot be decided",
                )
                .help("fetch the full history"));
            }
            Err(error @ akr_core::git::GitError::InvalidRewriteMap { .. }) => {
                return Err(EnvError::new("AKR-G014", error.to_string()).help(
                    "use one `<old-commit> <replacement-commit>` pair per line; both must be full lowercase commit ids",
                ));
            }
            Err(_) => None,
        };
        let commit = match (&self.global.at, &repository) {
            (Some(at), Some(repository)) => {
                if !repository.contains(at) {
                    return Err(EnvError::new(
                        "AKR-G013",
                        format!("--at: {at} is not a commit in this repository"),
                    ));
                }
                Some(at.clone())
            }
            (Some(at), None) => Some(at.clone()),
            (None, Some(repository)) => repository.head().ok(),
            (None, None) => None,
        };

        if let Some(repository) = &repository {
            if let Ok(last_change) = akr_core::git::last_changes(repository, &self.ledger) {
                self.ledger.facts.last_change = last_change;
            }
            let commits: Vec<Commit> = self
                .ledger
                .facts
                .last_change
                .values()
                .cloned()
                .chain(
                    self.ledger
                        .records()
                        .iter()
                        .filter_map(akr_core::freshness::observed_at)
                        .cloned(),
                )
                .chain(commit.clone())
                .collect();
            if let Some(head) = &commit
                && let Ok(off_branch) = repository.unreachable_from(head, &commits)
            {
                self.ledger.facts.off_branch = off_branch;
            }
            if let Ok(ancestry) = akr_core::git::ancestry_over(repository, commits) {
                self.ledger.facts.ancestry = ancestry;
            }
        }

        self.inputs.commit = repository.as_ref().and_then(|repository| {
            source_graph_commit(repository, &self.inputs.sources, self.global.at.as_ref())
        });
        self.repository = repository;
        self.commit = commit;
        self.git_facts_loaded = true;
        Ok(())
    }

    /// Attaches the lock's seal facts, so V-024 fires (D-015).
    pub fn attach_lock(&mut self) {
        let Some(text) = &self.lock_text else { return };
        let Ok(lock) = akr_core::lock::Lock::parse(text) else {
            return;
        };
        let computed: std::collections::BTreeMap<_, _> = self
            .inputs
            .canonical_text
            .iter()
            .map(|(id, text)| (id.clone(), akr_core::hash::content_hash(text)))
            .collect();
        lock.apply_facts(&mut self.ledger, &computed);
    }

    /// Runs stages C and D.
    #[must_use]
    pub fn resolve(&self) -> ResolvedModel<'_> {
        ResolvedModel::build(&self.ledger, &self.inputs)
    }

    /// Derives the review queue, when a repository is available.
    #[must_use]
    pub fn review_queue(&self) -> ReviewQueue {
        let (Some(repository), Some(commit)) = (&self.repository, &self.commit) else {
            return ReviewQueue::default();
        };
        let mut queue = akr_core::freshness::derive(&self.ledger, repository, commit, self.today)
            .unwrap_or_default();
        if let Ok(mut diagnostics) =
            akr_core::freshness::unmatched_watches(&self.ledger, repository, commit)
        {
            queue.diagnostics.append(&mut diagnostics);
            queue.diagnostics.sort_by_key(Diagnostic::sort_key);
        }
        queue
    }

    /// The freshness a renderer needs.
    #[must_use]
    pub fn freshness(&self, queue: &ReviewQueue) -> Freshness {
        Freshness::from_stale(&self.ledger, queue.stale_set()).with_causes(
            queue
                .stale
                .iter()
                .map(|entry| (entry.id.clone(), entry.cause.clone()))
                .collect(),
        )
    }

    /// Every diagnostic from stages A–D, with spans attached and in a stable order.
    #[must_use]
    pub fn diagnostics(&self, model: &ResolvedModel<'_>) -> Vec<Diagnostic> {
        let mut out = self.parse_diagnostics.clone();
        out.extend(model.diagnostics.clone());
        // The migration audit (docs/12 §3-§4): a *migrated* document whose provenance
        // has decayed — the document gone (AKR-M022), or archived before its tracking
        // record completed (AKR-M032). Bare `source { kind legacy }` citations with no
        // tracking record are left alone; see `akr_core::import::audit`.
        let head = self
            .commit
            .as_ref()
            .map_or_else(|| "HEAD".to_owned(), |c| c.as_str()[..8].to_owned());
        out.extend(akr_core::import::audit(&self.ledger, &head, &|path| {
            self.root.join(path).exists()
        }));
        self.spans.attach_all(&mut out);
        out.sort_by_key(Diagnostic::sort_key);
        out
    }

    /// The view output directory, from `project.akr`'s `defaults` or the default.
    #[must_use]
    pub fn view_dir(&self) -> PathBuf {
        self.root.join("docs/generated")
    }

    /// The source-graph hash of the loaded inputs, as the JSON envelope reports it.
    #[must_use]
    pub fn source_graph(&self) -> String {
        {
            let mut sources = self.inputs.sources.clone();
            sources.sort();
            sources.dedup_by(|a, b| a.path == b.path);
            akr_core::hash::source_graph_hash(sources.iter().map(|s| (s.path.as_str(), &s.hash)))
                .to_string()
        }
    }
}

/// The newest useful commit that contains every loaded source byte exactly.
fn source_graph_commit(
    repository: &Repository,
    sources: &[akr_core::resolve::SourceFile],
    requested: Option<&Commit>,
) -> Option<Commit> {
    let changes = repository.working_tree_changes().ok()?;
    if sources.iter().any(|source| changes.contains(&source.path)) {
        return None;
    }

    let contains_graph = |commit: &Commit| {
        sources.iter().all(|source| {
            repository
                .file_at(commit, &source.path)
                .ok()
                .flatten()
                .is_some_and(|text| {
                    akr_core::hash::source_file_hash(text.as_bytes()) == source.hash
                })
        })
    };
    if let Some(requested) = requested {
        return contains_graph(requested).then(|| requested.clone());
    }

    let mut candidates: Vec<Commit> = sources
        .iter()
        .filter_map(|source| {
            repository
                .commits_touching(&source.path)
                .ok()?
                .into_iter()
                .next()
        })
        .collect();
    candidates.sort();
    candidates.dedup();
    if let Ok(ordered) = repository.topological_order(&candidates)
        && let Some(commit) = ordered.into_iter().find(|commit| contains_graph(commit))
    {
        return Some(commit);
    }

    repository.head().ok().filter(contains_graph)
}

/// Whether a diagnostic counts as an error under the active profile (D-013).
#[must_use]
pub fn is_fatal(diagnostic: &Diagnostic, profile: Profile) -> bool {
    // `AKR-G004` is a fact about the *working tree*, not about the ledger, and it is
    // exempt from the strict promotion for the same reason staleness never changes an
    // exit code (D-024): it says the reader should not be misled, not that anything is
    // wrong.
    //
    // Left promotable, it made "akr check is clean" unreachable for an agent mid-task,
    // because an agent mid-task always has uncommitted edits in watched paths. The two
    // ways out were both bad — commit prematurely to satisfy the check, or run
    // `--lenient` and lose every other strict signal along with this one — which is
    // exactly what was reported from a real session
    // (`jpegxl-rs.papercut.akr-check-strict-exits-1-on-akr-g004-alone-when`).
    if matches!(diagnostic.code.as_str(), "AKR-G004" | "AKR-G023") {
        return false;
    }
    match profile {
        Profile::Strict => true,
        Profile::Lenient => diagnostic.severity == Severity::Error,
    }
}

/// Renders diagnostics, then reports how many were fatal.
#[must_use]
pub fn report(
    diagnostics: &[Diagnostic],
    sources: &SourceMap,
    profile: Profile,
) -> (String, usize) {
    let mut out = String::new();
    let mut fatal = 0;
    let mut promoted = 0;
    for diagnostic in diagnostics {
        out.push_str(&render(diagnostic, sources));
        out.push('\n');
        if is_fatal(diagnostic, profile) {
            fatal += 1;
            if diagnostic.severity == Severity::Warning {
                promoted += 1;
            }
        }
    }
    if fatal > 0 {
        out.push_str(&format!(
            "{fatal} error{}{}\n",
            if fatal == 1 { "" } else { "s" },
            if promoted == 0 {
                String::new()
            } else {
                format!(
                    " under --strict ({promoted} warning{} promoted)",
                    if promoted == 1 { "" } else { "s" }
                )
            }
        ));
    }
    (out, fatal)
}

/// The JSON form of a diagnostic (`docs/07-cli.md` §5).
#[must_use]
pub fn diagnostic_json(diagnostic: &Diagnostic, sources: &SourceMap, profile: Profile) -> Value {
    let mut fields = vec![
        ("code", Value::string(diagnostic.code.as_str())),
        (
            "severity",
            Value::string(match diagnostic.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
            }),
        ),
        (
            "effective_severity",
            Value::string(if is_fatal(diagnostic, profile) {
                "error"
            } else {
                "warning"
            }),
        ),
    ];
    if let Some(rule) = diagnostic.rule {
        fields.push(("rule", Value::string(rule.to_string())));
    }
    fields.push(("message", Value::string(diagnostic.message.clone())));
    if let Some(span) = diagnostic.primary.span
        && let Some(file) = sources.get(span.file)
    {
        let (line, column) = file.location(span.start);
        fields.push(("path", Value::string(file.path.clone())));
        fields.push(("line", Value::integer(i64::from(line))));
        fields.push(("column", Value::integer(i64::from(column))));
    }
    if let Some(help) = &diagnostic.help {
        fields.push(("help", Value::string(help.clone())));
    }
    Value::object(fields)
}

/// The envelope every JSON command prints (`docs/07-cli.md` §5).
#[must_use]
pub fn envelope(
    command: &str,
    commit: Option<&str>,
    source_graph: &str,
    exit: Exit,
    diagnostics: Vec<Value>,
    result: Value,
) -> Value {
    Value::object(vec![
        ("akr", Value::string("0.1")),
        ("tool_version", Value::string(TOOL_VERSION)),
        ("command", Value::string(command)),
        ("commit", commit.map_or(Value::Null, Value::string)),
        ("source_graph_hash", Value::string(source_graph)),
        ("ok", Value::bool(exit == Exit::Ok)),
        ("exit_code", Value::integer(exit as i64)),
        ("diagnostics", Value::array(diagnostics)),
        ("result", result),
    ])
}

/// Whether the caller asked for JSON.
#[must_use]
pub const fn wants_json(global: &Global) -> bool {
    matches!(global.format, Format::Json)
}

/// Today's date, when `--today` was not given.
///
/// The only clock reading in the whole tool, and it happens *here* — outside the build —
/// so that everything downstream takes a date as an input and stays reproducible
/// (`docs/06-compiler-pipeline.md` §4).
fn current_date() -> Date {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    civil_from_days(days)
}

/// Howard Hinnant's `civil_from_days`, which needs no dependency and no leap-second table.
fn civil_from_days(days: i64) -> Date {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    Date::new(
        i32::try_from(year).unwrap_or(2026),
        u8::try_from(m).unwrap_or(1),
        u8::try_from(d).unwrap_or(1),
    )
    .unwrap_or_else(|_| Date::new(2026, 1, 1).expect("a valid fallback date"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use akr_core::diagnostics::{Code, Label, Subject};

    fn warning() -> Diagnostic {
        Diagnostic {
            code: Code::new("AKR-G022"),
            severity: Severity::Warning,
            rule: None,
            message: "a watched path disappeared".to_owned(),
            primary: Label::new(Subject::Ledger),
            notes: Vec::new(),
            help: None,
        }
    }

    #[test]
    fn strict_report_names_promoted_warnings() {
        let (text, fatal) = report(&[warning()], &SourceMap::new(), Profile::Strict);
        assert_eq!(fatal, 1);
        assert!(
            text.ends_with("1 error under --strict (1 warning promoted)\n"),
            "{text}"
        );
    }

    #[test]
    fn structured_diagnostic_carries_declared_and_effective_severity() {
        let json = diagnostic_json(&warning(), &SourceMap::new(), Profile::Strict);
        assert_eq!(
            json.get("severity").and_then(Value::as_str),
            Some("warning")
        );
        assert_eq!(
            json.get("effective_severity").and_then(Value::as_str),
            Some("error")
        );
    }
}

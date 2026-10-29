//! Stage E — the index cache (`docs/06-compiler-pipeline.md` §7).
//!
//! A materialisation of the resolved model into `.akr/cache/index.sqlite`, whose DDL is
//! [`spec/schema/index.sql`](../../../../spec/schema/index.sql) — included here verbatim
//! rather than retyped, so the schema the tool creates and the schema the spec describes
//! cannot drift apart.
//!
//! # What this is not (D-019)
//!
//! A cache. Never authoritative, never committed, always rebuildable, and read by nothing
//! outside this crate. Deleting it costs one rebuild. That boundary is what keeps the
//! schema free to change, and it is the reason this module exposes a *builder* and a
//! *query*, never a connection: a caller that could reach the connection could reach the
//! schema, and the schema would then have compatibility obligations the design has not
//! promised.
//!
//! # Invalidation is silent
//!
//! §7 step 2: a `meta` row that disagrees with the tool's values means drop everything and
//! rebuild. That is routine, not a diagnostic. The `AKR-I` codes here mean the cache could
//! not be built — never that it had to be.

use crate::diagnostics::{Code, Diagnostic, Label, Severity, Subject, codes::index as codes};
use crate::freshness::ReviewQueue;
use crate::resolve::{ResolvedModel, SpanIndex};
use std::path::{Path, PathBuf};
use std::time::Duration;

mod populate;
#[cfg(feature = "fts5")]
mod search;
#[cfg(feature = "fts5")]
mod sources;

#[cfg(feature = "fts5")]
pub use search::{Hit, Request, escape_query as escape_record_query, search};
#[cfg(feature = "fts5")]
pub use sources::{
    ChunkText, QueryMode, SOURCES_SCHEMA_VERSION, SOURCES_SQL, SourceHit, SourceIndexStats,
    SourceRequest, document_chunks, escape_query, get_chunk, search as search_sources,
    sources_cache_path, sync as sync_sources,
};

/// The DDL, verbatim from the spec.
pub const SCHEMA_SQL: &str = include_str!("../../../../spec/schema/index.sql");

/// `meta.schema_version`. Bumped by hand when [`SCHEMA_SQL`] changes.
///
/// A bump means a full rebuild and there is no migration path, which is the whole point:
/// the cache is derivable, so throwing it away is always cheaper than migrating it.
/// `tests/store_schema.rs` fails when the DDL changes without a bump, because a forgotten
/// bump is the one way this could go wrong quietly — every reader would keep using a cache
/// whose shape no longer matches the code reading it.
pub const SCHEMA_VERSION: i64 = 2;

/// Why stage E could not build the cache.
///
/// Never a routine invalidation, and never a fact about the ledger: every variant here is
/// an environment or an internal-invariant failure, which is why they are `AKR-I` codes
/// and not `AKR-R` ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexError {
    /// The `AKR-I` code.
    pub code: Code,
    /// The one-line message, already carrying the path or the SQLite detail.
    pub message: String,
}

impl IndexError {
    fn new(code: Code, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// The diagnostic form, for a caller that reports rather than returns.
    #[must_use]
    pub fn diagnostic(&self) -> Diagnostic {
        Diagnostic {
            code: self.code,
            severity: Severity::Error,
            rule: None,
            message: self.message.clone(),
            primary: Label::new(Subject::Ledger),
            notes: Vec::new(),
            help: None,
        }
    }
}

impl std::fmt::Display for IndexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

/// What stage E wrote, for `akr build`'s summary and for the row-count invariant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IndexStats {
    /// Rows in `records`.
    pub records: usize,
    /// Rows in `revisions`.
    pub revisions: usize,
    /// Rows in `relations`.
    pub relations: usize,
    /// Rows in `records_fts`, or zero when built without FTS5.
    pub indexed: usize,
    /// Whether the cache was rebuilt from scratch rather than found current.
    pub rebuilt: bool,
    /// Whether this build wrote a full-text index.
    pub full_text: bool,
}

/// Everything stage E materialises, gathered by the caller that already has it.
///
/// Taking one struct rather than six arguments is not tidiness: `today` and the
/// diagnostics come from the CLI session, the queue from stage D's freshness pass, and
/// the spans from the parser, and a positional signature over those would be a trap.
#[derive(Debug, Clone, Copy)]
pub struct IndexInputs<'a> {
    /// Stages C and D.
    pub model: &'a ResolvedModel<'a>,
    /// The staleness verdicts, which become the `resolutions` flags.
    pub queue: &'a ReviewQueue,
    /// Subject-to-span, for `revisions.span_start` and `span_end`.
    pub spans: &'a SpanIndex,
    /// Every diagnostic this build collected, in emission order.
    pub diagnostics: &'a [Diagnostic],
    /// The date `review_after` was compared against, as `meta.today`.
    pub today: &'a str,
}

/// Whether this binary builds a full-text index.
#[must_use]
pub const fn has_fts5() -> bool {
    cfg!(feature = "fts5")
}

/// Builds or refreshes the index at `path`.
///
/// The procedure of §7, in order: open, decide whether the cache is current, rebuild
/// inside one transaction when it is not, verify the invariants, commit, integrity-check.
///
/// A cache whose `meta` already agrees with the model is left alone and reported with
/// `rebuilt: false` — the source-graph hash covers every byte of every source file, so
/// agreement means there is nothing to write.
///
/// # Errors
/// [`IndexError`] carrying the `AKR-I` code of whichever step failed.
pub fn build(path: &Path, inputs: &IndexInputs) -> Result<IndexStats, IndexError> {
    if let Some(parent) = path.parent()
        && let Err(error) = std::fs::create_dir_all(parent)
    {
        return Err(IndexError::new(
            codes::I003,
            format!(
                "index cache directory {} is not writable: {error}",
                parent.display()
            ),
        ));
    }

    let mut connection = open(path)?;
    if is_current(&connection, inputs) {
        let stats = counts(&connection).unwrap_or_default();
        return Ok(IndexStats {
            rebuilt: false,
            ..stats
        });
    }

    let stats = populate::rebuild(&mut connection, inputs)?;

    // Step 5. `integrity_check` is cheap next to the populate and it is the only check
    // that would catch a cache that is corrupt rather than merely wrong.
    let verdict: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|error| {
            IndexError::new(
                codes::I011,
                format!("index integrity check failed after write: {error}"),
            )
        })?;
    if verdict != "ok" {
        return Err(IndexError::new(
            codes::I011,
            format!("index integrity check failed after write: {verdict}"),
        ));
    }

    Ok(IndexStats {
        rebuilt: true,
        ..stats
    })
}

/// The conventional cache path beneath an `.akr` directory.
#[must_use]
pub fn cache_path(akr_dir: &Path) -> PathBuf {
    akr_dir.join("cache").join("index.sqlite")
}

/// Whether the cache at `path` was built from a different source graph than the one the
/// caller is holding — i.e. the ledger has been written since the last `akr build`.
///
/// This is the observable half of the "invalidation is silent" policy. `akr build` drops
/// and rebuilds a disagreeing cache; `akr search`, which by D-019 must never write the
/// cache, cannot. It can only *notice*, so that a read taken between a write and the next
/// build is not silently answered from stale rows. The verdict never changes an exit code
/// (D-024): staleness is a fact to surface, not a failure.
///
/// `expected` is the current source-graph hash, with or without its `sha256:` prefix; the
/// cache stores it bare. A missing cache, a cache with no `source_graph_hash` row, or an
/// unreadable one all answer `false` — the first is another command's error to raise
/// (`AKR-I031`), and the last two are not staleness.
#[must_use]
pub fn is_stale_against(path: &Path, expected: &str) -> bool {
    if !path.exists() {
        return false;
    }
    let Ok(connection) = open_configured(path) else {
        return false;
    };
    let expected = expected.strip_prefix("sha256:").unwrap_or(expected);
    let stored: Option<String> = connection
        .query_row(
            "SELECT value FROM meta WHERE key = 'source_graph_hash'",
            [],
            |row| row.get(0),
        )
        .ok();
    stored.is_some_and(|found| found != expected)
}

/// Whether the cache at `path` lacks the full-text table this binary would build.
///
/// A cache is shared by every binary that opens the workspace, and they need not agree on
/// the `fts5` feature: a `--no-default-features` build, or a host that links `akr-core`
/// without it, writes a cache with no `records_fts`. Its source-graph stamp is still
/// current, so [`is_stale_against`] calls it fresh, and a full-text binary that trusted
/// that verdict went on to answer `akr start` with `AKR-I022` — a code meaning *this
/// binary* cannot search — when one rebuild would have answered it. [`build`] already
/// treats the mismatch as not current; this is the same question asked by a reader that
/// must decide whether to rebuild before it searches.
///
/// `false` in a binary without FTS5 (it has nothing better to build), for a missing cache
/// (another check's business), and for an unreadable one (`AKR-I001` is raised by the
/// read that follows).
#[must_use]
pub fn lacks_full_text(path: &Path) -> bool {
    if !has_fts5() || !path.exists() {
        return false;
    }
    let Ok(connection) = open_configured(path) else {
        return false;
    };
    has_meta(&connection) && !stored_full_text(&connection)
}

/// The `source_graph_hash` that the on-disk cache is currently stamped with, if any.
#[must_use]
pub fn cached_source_graph_hash(path: &Path) -> Option<String> {
    let connection = open_configured(path).ok()?;
    connection
        .query_row(
            "SELECT value FROM meta WHERE key = 'source_graph_hash'",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok()
}

/// How long a concurrent session waits on a busy cache before failing.
///
/// Several agent hosts each spawn their own `akr-mcp`. They share one on-disk SQLite
/// cache, and a default `busy_timeout` of zero turns ordinary overlap into `AKR-I001`.
const CACHE_BUSY_TIMEOUT: Duration = Duration::from_millis(5_000);

pub(crate) fn open_configured(path: &Path) -> rusqlite::Result<rusqlite::Connection> {
    let connection = rusqlite::Connection::open(path)?;
    configure_cache(&connection)?;
    Ok(connection)
}

fn configure_cache(connection: &rusqlite::Connection) -> rusqlite::Result<()> {
    connection.busy_timeout(CACHE_BUSY_TIMEOUT)?;
    // WAL lets readers proceed while another session rebuilds. `synchronous=NORMAL` is
    // the usual WAL pairing; the cache is disposable, so a torn write is a rebuild.
    let _ = connection.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;");
    Ok(())
}

/// Opens the cache, refusing a file that is not one.
fn open(path: &Path) -> Result<rusqlite::Connection, IndexError> {
    let existed = path.exists();
    let connection = open_configured(path).map_err(|error| {
        IndexError::new(
            codes::I001,
            format!("cannot read index cache at {}: {error}", path.display()),
        )
    })?;

    // §7 step 1: a SQLite database at the path that is not an AKR index is `AKR-I004`.
    // Deleting somebody else's database is the operator's decision, not the tool's, so
    // this refuses rather than clobbering.
    if existed {
        let tables: i64 = connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'table'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| {
                IndexError::new(
                    codes::I001,
                    format!("cannot read index cache at {}: {error}", path.display()),
                )
            })?;
        if tables > 0 && !has_meta(&connection) {
            return Err(IndexError::new(
                codes::I004,
                format!(
                    "{} is a SQLite database but has no AKR meta table",
                    path.display()
                ),
            ));
        }
    }
    Ok(connection)
}

fn has_meta(connection: &rusqlite::Connection) -> bool {
    connection
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'meta'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .is_ok_and(|count| count > 0)
}

/// Whether the cache already matches this model, per §7 step 2.
///
/// Absent, unreadable or disagreeing metadata all mean the same thing — rebuild — and all
/// of it is silent. The five inputs are exactly the ones the spec names; `built_at` is
/// deliberately not among them, because it is informational and comparing it would make
/// every build a rebuild.
fn is_current(connection: &rusqlite::Connection, inputs: &IndexInputs) -> bool {
    if !has_meta(connection) {
        return false;
    }
    let expected = meta_rows(inputs);
    expected
        .iter()
        .filter(|(key, _)| *key != "built_at")
        .all(|(key, value)| {
            connection
                .query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
                    row.get::<_, String>(0)
                })
                .is_ok_and(|found| &found == value)
        })
        && stored_full_text(connection) == has_fts5()
}

/// Whether the cache on disk carries a full-text table.
///
/// Asked of `sqlite_master` rather than of a `meta` row on purpose: this is the question
/// `akr search` has to answer before it can run, and the honest answer is whether the
/// table is *there*, not what a previous build believed about itself.
pub(crate) fn stored_full_text(connection: &rusqlite::Connection) -> bool {
    connection
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name = 'records_fts'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .is_ok_and(|count| count > 0)
}

/// The `meta` table's rows, in the order the spec lists them.
fn meta_rows(inputs: &IndexInputs) -> Vec<(&'static str, String)> {
    let model = inputs.model;
    vec![
        ("schema_version", SCHEMA_VERSION.to_string()),
        ("tool_version", model.tool_version.clone()),
        ("grammar_version", model.grammar_version.clone()),
        ("vocabulary_version", model.vocabulary_version.clone()),
        // Bare, per the DDL's conventions: hashes carry no `sha256:` and commits no
        // `git:`. The prefixes are a source-syntax affordance, and the cache is not source.
        (
            "source_graph_hash",
            model
                .source_graph
                .to_string()
                .strip_prefix("sha256:")
                .unwrap_or(&model.source_graph.to_string())
                .to_owned(),
        ),
        (
            "commit",
            model
                .commit
                .as_ref()
                .map(|c| {
                    c.as_str()
                        .strip_prefix("git:")
                        .unwrap_or(c.as_str())
                        .to_owned()
                })
                .unwrap_or_default(),
        ),
        ("today", inputs.today.to_owned()),
        ("built_at", model.built_at.clone()),
    ]
}

fn counts(connection: &rusqlite::Connection) -> Option<IndexStats> {
    let count = |table: &str| -> Option<usize> {
        connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get::<_, i64>(0)
            })
            .ok()
            .and_then(|n| usize::try_from(n).ok())
    };
    let full_text = stored_full_text(connection);
    Some(IndexStats {
        records: count("records")?,
        revisions: count("revisions")?,
        relations: count("relations")?,
        indexed: if full_text {
            count("records_fts").unwrap_or(0)
        } else {
            0
        },
        rebuilt: false,
        full_text,
    })
}

/// The DDL to run, with the full-text table removed when this binary has no FTS5.
///
/// Cutting the statement out rather than skipping its creation at runtime is what makes
/// P7 exit criterion 4 reachable: the cache genuinely has no `records_fts`, so `akr
/// search` fails the way it would on a binary built without the extension, instead of
/// pretending to.
pub(crate) fn ddl() -> String {
    if has_fts5() {
        return SCHEMA_SQL.to_owned();
    }
    let Some(start) = SCHEMA_SQL.find("CREATE VIRTUAL TABLE records_fts") else {
        return SCHEMA_SQL.to_owned();
    };
    let Some(end) = SCHEMA_SQL[start..].find(");") else {
        return SCHEMA_SQL.to_owned();
    };
    let mut out = String::with_capacity(SCHEMA_SQL.len());
    out.push_str(&SCHEMA_SQL[..start]);
    out.push_str(&SCHEMA_SQL[start + end + 2..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ddl_is_the_spec_file() {
        assert!(SCHEMA_SQL.contains("CREATE TABLE revisions"));
        assert!(SCHEMA_SQL.contains("CREATE VIRTUAL TABLE records_fts"));
    }

    #[test]
    fn dropping_fts5_removes_exactly_the_virtual_table() {
        // The rest of the schema has to survive the cut, or a binary without FTS5 would
        // fail to build any index at all rather than one without a full-text table.
        let stripped = {
            let start = SCHEMA_SQL
                .find("CREATE VIRTUAL TABLE records_fts")
                .expect("the statement");
            let end = SCHEMA_SQL[start..].find(");").expect("its terminator");
            format!("{}{}", &SCHEMA_SQL[..start], &SCHEMA_SQL[start + end + 2..])
        };
        assert!(!stripped.contains("CREATE VIRTUAL TABLE"));
        assert!(stripped.contains("CREATE TABLE revisions"));
        assert!(stripped.contains("CREATE VIEW heads"));
        assert!(stripped.contains("CREATE VIEW review_queue"));
    }

    #[test]
    fn an_empty_database_can_be_created_from_the_ddl() {
        let connection = rusqlite::Connection::open_in_memory().expect("in-memory");
        connection.execute_batch(&ddl()).expect("the DDL runs");
        assert!(has_meta(&connection));
        assert_eq!(stored_full_text(&connection), has_fts5());
    }
}

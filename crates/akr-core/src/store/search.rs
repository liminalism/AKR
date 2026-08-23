//! `akr search` — BM25 over `records_fts`, with key as the tiebreak.
//!
//! # Ranking only
//!
//! `docs/09-context-assembly.md` §10: nothing enters a context bundle because it matched a
//! query, and no ranking signal raises a record's authority. This module is reachable from
//! the search command and from nothing else, which is the structural form of that promise
//! — the context assembler cannot call it because it does not know it exists.
//!
//! # Why the order is stable
//!
//! BM25 alone is not a total order: two revisions can score identically, and SQLite is
//! free to return ties in any order it likes. The key tiebreak makes the order total, so
//! the same query returns the same sequence twice (P7 exit criterion 1) rather than
//! usually-the-same.

use super::{IndexError, codes, stored_full_text};
use std::path::Path;

/// One search result, in the order `akr search` prints them.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    /// The record's key.
    pub key: String,
    /// The live revision that matched.
    pub rev: u32,
    /// Its kind.
    pub kind: String,
    /// Its state.
    pub state: String,
    /// Its title.
    pub title: String,
    /// The BM25 score, sign-flipped so that larger is better.
    pub score: f64,
}

/// A search request. Filters are applied before ranking (`docs/07-cli.md` §6).
#[derive(Debug, Clone, Default)]
pub struct Request {
    /// What to look for.
    ///
    /// Ordinary words unless [`Self::raw_fts`] is set. Punctuation is escaped into
    /// terms rather than handed to the engine as operators.
    pub query: String,
    /// Treat [`Self::query`] as a raw FTS5 expression.
    ///
    /// Off by default, and that default is the fix for a real friction: an agent asked
    /// for `HDR slice 6 non-default feature` and got `no such column: default`, asked
    /// for a query containing a comma and got `fts5: syntax error near ","`, and gave up
    /// and grepped `.akr/records` — which is the one thing the whole surface exists to
    /// make unnecessary. Operators are worth having; they are not worth being the
    /// default spelling of "find me these words".
    pub raw_fts: bool,
    /// Restrict to these kinds. Empty means every kind.
    pub kinds: Vec<String>,
    /// Restrict to these states. Empty means every state.
    pub states: Vec<String>,
    /// Maximum results. `None` is the documented default of 20.
    pub limit: Option<usize>,
}

/// Runs a query against the cache at `path`.
///
/// # Errors
/// `AKR-I022` when the cache carries no full-text table, `AKR-X031` when the engine
/// rejects the query, and `AKR-I001` when the cache cannot be read at all.
pub fn search(path: &Path, request: &Request) -> Result<Vec<Hit>, IndexError> {
    if !path.exists() {
        return Err(IndexError::new(
            codes::I031,
            "index is stale and rebuilding is disabled",
        ));
    }
    let connection = super::open_configured(path).map_err(|error| {
        IndexError::new(
            codes::I001,
            format!("cannot read index cache at {}: {error}", path.display()),
        )
    })?;

    // The honest question is whether the table is there, not what a previous build
    // believed about itself.
    if !stored_full_text(&connection) {
        return Err(IndexError::new(
            codes::I022,
            "search requires a full-text index; this cache was built without FTS5",
        ));
    }

    let limit = request.limit.unwrap_or(20).min(100);
    let expression = if request.raw_fts {
        request.query.clone()
    } else {
        escape_query(&request.query)
    };
    if expression.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut sql = String::from(
        "SELECT records_fts.key, records_fts.rev, records_fts.kind, r.state, \
         records_fts.title, bm25(records_fts) AS score \
         FROM records_fts \
         JOIN revisions r ON r.key = records_fts.key AND r.rev = records_fts.rev \
         WHERE records_fts MATCH ?1",
    );
    if !request.kinds.is_empty() {
        sql.push_str(&format!(
            " AND records_fts.kind IN ({})",
            placeholders(request.kinds.len(), 2)
        ));
    }
    if !request.states.is_empty() {
        sql.push_str(&format!(
            " AND r.state IN ({})",
            placeholders(request.states.len(), 2 + request.kinds.len())
        ));
    }
    // Ascending, because FTS5's bm25 is more negative the better the match. The key
    // tiebreak is what makes this a total order.
    sql.push_str(" ORDER BY score ASC, records_fts.key ASC LIMIT ");
    sql.push_str(&limit.to_string());

    let mut bound: Vec<&dyn rusqlite::ToSql> = vec![&expression];
    for kind in &request.kinds {
        bound.push(kind);
    }
    for state in &request.states {
        bound.push(state);
    }

    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| query_error(&error))?;
    let rows = statement
        .query_map(bound.as_slice(), |row| {
            Ok(Hit {
                key: row.get(0)?,
                rev: row.get::<_, i64>(1)?.try_into().unwrap_or(0),
                kind: row.get(2)?,
                state: row.get(3)?,
                title: row.get(4)?,
                score: -row.get::<_, f64>(5)?,
            })
        })
        .map_err(|error| query_error(&error))?;

    let mut hits = Vec::new();
    for row in rows {
        hits.push(row.map_err(|error| query_error(&error))?);
    }
    Ok(hits)
}

/// A query the engine would not take is the caller's fault, not the cache's.
///
/// With escaping on by default this is now reachable only through `--fts`, which is the
/// right place for it: somebody who asked for operators wants to hear that the operators
/// were wrong.
fn query_error(error: &rusqlite::Error) -> IndexError {
    IndexError::new(
        codes::X031,
        format!(
            "search query: {error}\nhelp: this is a raw FTS5 expression; drop --fts to \
             search for the words themselves"
        ),
    )
}

/// Turns a caller's words into an FTS5 expression that cannot be a syntax error.
///
/// Each whitespace-separated term becomes one quoted phrase with its punctuation turned
/// into token boundaries, matching what the tokeniser stored, and the phrases are joined
/// with OR. Natural-language orientation queries routinely contain words no one record
/// repeats; requiring every word made a multi-word task silently return zero even when
/// several strong single-word matches existed. BM25 still ranks records matching more of
/// the query first. `DecodeRequest::default()` becomes the phrase
/// `"DecodeRequest default"`; a bare comma disappears; `non-default` stops being read as
/// a column name.
#[must_use]
pub fn escape_query(query: &str) -> String {
    query
        .split_whitespace()
        .map(|term| {
            term.split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|term| !term.is_empty())
        .map(|term| format!("\"{term}\""))
        .collect::<Vec<_>>()
        .join(" OR ")
}

fn placeholders(count: usize, first: usize) -> String {
    (first..first + count)
        .map(|n| format!("?{n}"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::escape_query;

    #[test]
    fn the_three_queries_that_failed_in_the_field_now_parse() {
        // Every one of these came back as an FTS5 error from a real session
        // (`raw-autotune.papercut.knowledge-search-and-knowledge-start-via-the`).
        assert_eq!(escape_query("budget, tokens"), "\"budget\" OR \"tokens\"");
        assert_eq!(
            escape_query("HDR slice 6 non-default feature"),
            "\"HDR\" OR \"slice\" OR \"6\" OR \"non default\" OR \"feature\""
        );
        assert_eq!(
            escape_query("DecodeRequest::default()"),
            "\"DecodeRequest default\""
        );
    }

    #[test]
    fn an_all_punctuation_query_escapes_to_nothing() {
        // Which the caller reads as "no matches" rather than as a parse error.
        assert_eq!(escape_query(",,, ;;"), "");
        assert_eq!(escape_query("   "), "");
    }

    #[test]
    fn ordinary_words_are_unchanged_but_quoted() {
        assert_eq!(
            escape_query("decoder optimisation"),
            "\"decoder\" OR \"optimisation\""
        );
    }
}

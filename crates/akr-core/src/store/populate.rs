//! The populate transaction of `docs/06-compiler-pipeline.md` §7, steps 3 and 4.
//!
//! Every row here is derivable from the resolved model, and nothing here derives anything
//! new. That is the property that makes the cache safe to delete: a query answered from
//! these tables and the same query answered from the model in memory agree, because the
//! tables *are* the model, spelled out.
//!
//! # Order is part of the contract
//!
//! Rows go in in canonical order — key, then revision — so that a dump is stable across
//! builds (§11). It costs a sort that the model has already done and it buys a diffable
//! artefact, which is worth more than the alternative for anyone debugging a build.

use super::{IndexError, IndexInputs, IndexStats, codes, ddl, has_fts5, meta_rows};
use crate::diagnostics::{Diagnostic, Subject};
use crate::model::{ContentSlot, ContentValue, Record, RevisionId, ScopeTerm};
use crate::resolve::RefSite;
use rusqlite::{Connection, Transaction, params};

/// Drops everything and rebuilds, in one transaction.
pub(super) fn rebuild(
    connection: &mut Connection,
    inputs: &IndexInputs,
) -> Result<IndexStats, IndexError> {
    drop_everything(connection)?;
    connection
        .execute_batch(&ddl())
        .map_err(|error| write_failed("cannot create the index schema", &error))?;

    let transaction = connection
        .transaction()
        .map_err(|error| write_failed("cannot open the index transaction", &error))?;

    let stats = write_all(&transaction, inputs)?;

    transaction
        .commit()
        .map_err(|error| write_failed("cannot commit the index transaction", &error))?;
    Ok(stats)
}

fn write_failed(what: &str, error: &rusqlite::Error) -> IndexError {
    IndexError {
        code: codes::I002,
        message: format!("{what}: {error}"),
    }
}

#[cfg(feature = "fts5")]
fn fts_failed(error: &rusqlite::Error) -> IndexError {
    IndexError {
        code: codes::I021,
        message: format!("cannot build records_fts: {error}"),
    }
}

/// Drops every object, so a schema bump needs no migration path (D-019).
///
/// Foreign keys go off for the duration. They are declared `ON` by the DDL and enforced
/// for every write, but a *drop* under them fails as soon as a parent table goes before
/// its children — and there is no drop order that satisfies a schema with cycles in it,
/// only one that happens to work until the schema changes. Turning them off is the honest
/// version of "none of these tables is about to exist".
fn drop_everything(connection: &Connection) -> Result<(), IndexError> {
    connection
        .execute_batch("PRAGMA foreign_keys = OFF")
        .map_err(|error| write_failed("cannot suspend foreign keys", &error))?;

    let mut objects: Vec<(String, String)> = Vec::new();
    {
        let mut statement = connection
            .prepare(
                "SELECT type, name FROM sqlite_master \
                 WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%'",
            )
            .map_err(|error| write_failed("cannot read the index schema", &error))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| write_failed("cannot read the index schema", &error))?;
        for row in rows {
            objects
                .push(row.map_err(|error| write_failed("cannot read the index schema", &error))?);
        }
    }
    // Shadow tables of an FTS5 virtual table disappear with it, and dropping them first
    // would fail, so the virtual table goes first and the leftovers are ignored.
    for (kind, name) in objects.iter().filter(|(_, n)| n == "records_fts") {
        let _ = connection.execute(&format!("DROP {kind} IF EXISTS \"{name}\""), []);
    }
    for (kind, name) in &objects {
        if name == "records_fts" || name.starts_with("records_fts_") {
            continue;
        }
        connection
            .execute(&format!("DROP {kind} IF EXISTS \"{name}\""), [])
            .map_err(|error| write_failed(&format!("cannot drop {name}"), &error))?;
    }
    Ok(())
}

#[expect(
    clippy::too_many_lines,
    reason = "thirteen tables written in the order §7 step 3 lists them; splitting it \
              per table would scatter one transaction across a dozen signatures that each \
              need the same four inputs"
)]
fn write_all(tx: &Transaction, inputs: &IndexInputs) -> Result<IndexStats, IndexError> {
    let model = inputs.model;
    let ledger = model.ledger();

    // meta
    for (key, value) in meta_rows(inputs) {
        tx.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)",
            params![key, value],
        )
        .map_err(|error| write_failed("cannot write meta", &error))?;
    }

    // sources
    for source in &model.sources {
        tx.execute(
            "INSERT INTO sources (path, file_hash, byte_len, archived) VALUES (?1, ?2, ?3, ?4)",
            params![
                source.path,
                bare_hash(&source.hash.to_string()),
                i64::try_from(source.byte_len).unwrap_or(i64::MAX),
                i64::from(is_archived(&source.path)),
            ],
        )
        .map_err(|error| write_failed("cannot write sources", &error))?;
    }

    // records — one row per key, in key order.
    let mut record_rows = 0usize;
    for key in ledger.keys() {
        let revisions = ledger.revisions_of(key);
        let Some(first) = revisions.first() else {
            continue;
        };
        // Every revision of a key lives in one file (V-003), so the first one's file is
        // the key's file. A record built in code has none, and is skipped rather than
        // given a fabricated path that would break the foreign key.
        let Some(path) = first.file.clone() else {
            continue;
        };
        tx.execute(
            "INSERT INTO records (key, namespace, kind, class, path, head_rev, rev_count) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                key.to_string(),
                key.namespace().as_str(),
                first.kind.name(),
                first.kind.class().name(),
                path,
                model.heads.get(key).map(|id| i64::from(id.revision)),
                i64::try_from(revisions.len()).unwrap_or(i64::MAX),
            ],
        )
        .map_err(|error| write_failed("cannot write records", &error))?;
        record_rows += 1;
    }

    // revisions, and everything hanging off one revision.
    let mut revision_rows = 0usize;
    let mut relation_rows = 0usize;
    for record in sorted(ledger.records()) {
        if record.file.is_none() {
            continue;
        }
        let (span_start, span_end) = span_of(inputs, &record.id);
        tx.execute(
            "INSERT INTO revisions (key, rev, kind, class, state, live, sealed, title, topic, \
             content_hash, body, rationale, context, consequences, resolution, measure, cadence, \
             confidence, result, method, command, artifact, summary, observed_at, as_of, \
             review_after, target, author, created_at, acknowledged, span_start, span_end) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, \
             ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32)",
            params![
                record.id.key.to_string(),
                i64::from(record.id.revision),
                record.kind.name(),
                record.kind.class().name(),
                record.state.name(),
                i64::from(record.is_live()),
                i64::from(record.is_sealed()),
                record.title,
                record.topic.as_ref().map(Segment::as_str),
                model
                    .content_hash(&record.id)
                    .map(|hash| bare_hash(&hash.to_string()))
                    .unwrap_or_default(),
                body_of(record),
                text_slot(record, ContentSlot::Rationale),
                text_slot(record, ContentSlot::Context),
                text_slot(record, ContentSlot::Consequences),
                text_slot(record, ContentSlot::Resolution),
                text_slot(record, ContentSlot::Measure),
                text_slot(record, ContentSlot::Cadence),
                text_slot(record, ContentSlot::Confidence),
                text_slot(record, ContentSlot::Result),
                text_slot(record, ContentSlot::Method),
                text_slot(record, ContentSlot::Command),
                text_slot(record, ContentSlot::Artifact),
                text_slot(record, ContentSlot::Summary),
                text_slot(record, ContentSlot::ObservedAt),
                text_slot(record, ContentSlot::AsOf),
                text_slot(record, ContentSlot::ReviewAfter),
                text_slot(record, ContentSlot::Target),
                record.author.clone(),
                record.created_at.as_ref().map(ToString::to_string),
                i64::from(record.acknowledged),
                span_start,
                span_end,
            ],
        )
        .map_err(|error| write_failed("cannot write revisions", &error))?;
        revision_rows += 1;

        // claims
        for (ord, claim) in record.claims.iter().enumerate() {
            tx.execute(
                "INSERT INTO claims (key, rev, anchor, text, retired, ord) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    record.id.key.to_string(),
                    i64::from(record.id.revision),
                    claim.anchor.as_str(),
                    claim.text,
                    i64::from(record.retired_claims.contains(&claim.anchor)),
                    i64::try_from(ord).unwrap_or(i64::MAX),
                ],
            )
            .map_err(|error| write_failed("cannot write claims", &error))?;
        }

        // scopes
        for (ord, term) in record.scope.iter().enumerate() {
            let (form, ref_key, glob, prefix) = match term {
                ScopeTerm::All => ("all", None, None, None),
                ScopeTerm::Ref(reference) => ("ref", Some(reference.key.to_string()), None, None),
                ScopeTerm::Path(glob) => (
                    "path",
                    None,
                    Some(glob.as_str().to_owned()),
                    Some(literal_prefix(glob.as_str())),
                ),
            };
            tx.execute(
                "INSERT INTO scopes (key, rev, form, ref_key, glob, prefix, ord) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    record.id.key.to_string(),
                    i64::from(record.id.revision),
                    form,
                    ref_key,
                    glob,
                    prefix,
                    i64::try_from(ord).unwrap_or(i64::MAX),
                ],
            )
            .map_err(|error| write_failed("cannot write scopes", &error))?;
        }

        // watches, with the freshness verdict attached (D-024).
        for (ord, glob) in globs(record, ContentSlot::Watches).iter().enumerate() {
            let matched = watch_match(inputs, &record.id, glob);
            tx.execute(
                "INSERT INTO watches (key, rev, glob, prefix, matched_by, matched_path, ord) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    record.id.key.to_string(),
                    i64::from(record.id.revision),
                    glob,
                    literal_prefix(glob),
                    matched.as_ref().map(|(commit, _)| commit.clone()),
                    matched.as_ref().map(|(_, path)| path.clone()),
                    i64::try_from(ord).unwrap_or(i64::MAX),
                ],
            )
            .map_err(|error| write_failed("cannot write watches", &error))?;
        }

        // checks, and the evidence links that decide them.
        if let Some(acceptance) = &record.acceptance {
            for (ord, check) in acceptance.checks.iter().enumerate() {
                let satisfied = model
                    .checks_of(&record.id)
                    .into_iter()
                    .find(|verdict| verdict.check == check.id)
                    .is_some_and(|verdict| verdict.verdict.is_satisfied());
                tx.execute(
                    "INSERT INTO checks (key, rev, check_id, statement, method, command, \
                     satisfied, ord) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        record.id.key.to_string(),
                        i64::from(record.id.revision),
                        check.id.as_str(),
                        check.statement,
                        check.method.name(),
                        check.command.clone(),
                        i64::from(satisfied),
                        i64::try_from(ord).unwrap_or(i64::MAX),
                    ],
                )
                .map_err(|error| write_failed("cannot write checks", &error))?;
            }
        }

        // dispositions
        for (ord, disposition) in record.dispositions.iter().enumerate() {
            tx.execute(
                "INSERT INTO dispositions (key, rev, child_key, child_rev, outcome, into_key, \
                 note, ord) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    record.id.key.to_string(),
                    i64::from(record.id.revision),
                    disposition.target.key.to_string(),
                    disposition.target.revision.map(i64::from),
                    disposition.outcome.name(),
                    disposition.into.as_ref().map(|r| r.key.to_string()),
                    disposition.note.clone(),
                    i64::try_from(ord).unwrap_or(i64::MAX),
                ],
            )
            .map_err(|error| write_failed("cannot write dispositions", &error))?;
        }
    }

    // evidence_links — every citation, with the descendant verdict that decides
    // satisfaction. Evidence never declares what it verifies (D-016), so this edge runs
    // one way only and is derived from the check side.
    for link in crate::resolve::evidence_links(ledger) {
        tx.execute(
            "INSERT OR IGNORE INTO evidence_links (key, rev, check_id, evidence_key, \
             evidence_rev, result, observed_at, last_change, descends, satisfies) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                link.owner.key.to_string(),
                i64::from(link.owner.revision),
                link.check.as_str(),
                link.evidence.key.to_string(),
                i64::from(link.evidence.revision),
                link.result.name(),
                bare_commit(link.observed_at.as_str()),
                bare_commit(link.last_change.as_str()),
                i64::from(link.descends),
                i64::from(link.satisfies),
            ],
        )
        .map_err(|error| write_failed("cannot write evidence_links", &error))?;
    }

    // relations — one row per typed edge occurrence, from the resolved edges so that
    // `resolved_rev` is what the build actually resolved rather than a second opinion.
    let mut ordinals: std::collections::BTreeMap<(String, u32, &str, String), i64> =
        std::collections::BTreeMap::new();
    for edge in &model.edges {
        let RefSite::Relation(relation) = edge.site else {
            continue;
        };
        let Some(target) = &edge.to else {
            continue;
        };
        let from_key = edge.from.key.to_string();
        let to_key = edge.reference.key.to_string();
        let slot = relation.name();
        let ord = ordinals
            .entry((from_key.clone(), edge.from.revision, slot, to_key.clone()))
            .or_insert(0);
        tx.execute(
            "INSERT OR IGNORE INTO relations (from_key, from_rev, relation, to_key, to_rev, \
             to_anchor, resolved_rev, ord) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                from_key,
                i64::from(edge.from.revision),
                slot,
                to_key,
                edge.reference.revision.map(i64::from),
                edge.reference.anchor.as_ref().map(Segment::as_str),
                i64::from(target.revision),
                *ord,
            ],
        )
        .map_err(|error| write_failed("cannot write relations", &error))?;
        *ord += 1;
        relation_rows += 1;
    }

    // resolutions — stage D's verdict per revision, plus the freshness flags.
    for record in sorted(ledger.records()) {
        if record.file.is_none() {
            continue;
        }
        let stale = inputs.queue.stale.iter().find(|s| s.id == record.id);
        let at_risk = inputs.queue.at_risk.iter().find(|a| a.id == record.id);
        tx.execute(
            "INSERT INTO resolutions (key, rev, is_head, superseded_by, stale, stale_cause, \
             stale_detail, at_risk, at_risk_depth, at_risk_path) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                record.id.key.to_string(),
                i64::from(record.id.revision),
                i64::from(model.is_head(&record.id)),
                superseded_by(model, &record.id),
                i64::from(stale.is_some()),
                stale.map(|s| s.cause.name()),
                stale.map(|s| stale_detail(&s.cause)),
                i64::from(at_risk.is_some()),
                at_risk.map(|a| i64::try_from(a.depth).unwrap_or(i64::MAX)),
                at_risk.map(|a| at_risk_path(&record.id, &a.path)),
            ],
        )
        .map_err(|error| write_failed("cannot write resolutions", &error))?;
    }

    // diagnostics — everything stages A-F collected, so `akr check --format json` and the
    // MCP validate tool read one table rather than two code paths.
    for (seq, diagnostic) in inputs.diagnostics.iter().enumerate() {
        let (path, start, end) = diagnostic_span(inputs, diagnostic);
        tx.execute(
            "INSERT INTO diagnostics (seq, code, severity, stage, rule, message, path, \
             span_start, span_end, key, rev) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                i64::try_from(seq).unwrap_or(i64::MAX),
                diagnostic.code.as_str(),
                severity_name(diagnostic.severity),
                stage_of(diagnostic.code.as_str()),
                diagnostic.rule.map(|rule| rule.to_string()),
                diagnostic.message,
                path,
                start,
                end,
                subject_key(&diagnostic.primary.subject),
                subject_rev(&diagnostic.primary.subject),
            ],
        )
        .map_err(|error| write_failed("cannot write diagnostics", &error))?;
    }

    let indexed = write_full_text(tx, inputs)?;

    // Step 4: the invariants. Both are about stage E rather than about the ledger, which
    // is why they are internal-error codes and not validation rules.
    for (key, head) in &model.heads {
        let present: i64 = tx
            .query_row(
                "SELECT count(*) FROM resolutions WHERE key = ?1 AND rev = ?2",
                params![key.to_string(), i64::from(head.revision)],
                |row| row.get(0),
            )
            .unwrap_or(0);
        if present == 0 && ledger.get(head).is_some_and(|r| r.file.is_some()) {
            return Err(IndexError {
                code: codes::I012,
                message: format!(
                    "resolved head {}/{} was not written to the index",
                    key, head.revision
                ),
            });
        }
    }
    check_count(tx, "revisions", revision_rows)?;
    check_count(tx, "records", record_rows)?;

    Ok(IndexStats {
        records: record_rows,
        revisions: revision_rows,
        relations: relation_rows,
        indexed,
        rebuilt: true,
        full_text: has_fts5(),
    })
}

/// Populates `records_fts` from live revisions only.
///
/// Live only, because search ranks what is current: a superseded revision that outranked
/// its successor would be the surface teaching an agent to cite retired knowledge.
#[cfg(feature = "fts5")]
fn write_full_text(tx: &Transaction, inputs: &IndexInputs) -> Result<usize, IndexError> {
    let model = inputs.model;
    let mut rows = 0usize;
    for record in sorted(model.ledger().records()) {
        if record.file.is_none() || !record.is_live() {
            continue;
        }
        let claims = record
            .claims
            .iter()
            .map(|claim| claim.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        // Aliases are indexed so that searching a synonym finds the `term` that fixes the
        // project's meaning for it — usually the record the searcher actually wanted
        // (`docs/09-context-assembly.md` §10).
        let aliases = strings(record, ContentSlot::Aliases).join(" ");
        // D-047: a work record's reasoning is what a later reader searches for — "why
        // did we drop the colliders" — so the trace is ranked with the body it explains.
        let mut body = body_of(record).unwrap_or_default();
        for entry in strings(record, ContentSlot::Trace) {
            body.push('\n');
            body.push_str(&entry);
        }
        tx.execute(
            "INSERT INTO records_fts (key, rev, kind, title, body, claims, aliases) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                record.id.key.to_string(),
                i64::from(record.id.revision),
                record.kind.name(),
                record.title,
                body,
                claims,
                aliases,
            ],
        )
        .map_err(|error| fts_failed(&error))?;
        rows += 1;
    }
    Ok(rows)
}

#[cfg(not(feature = "fts5"))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "same signature as the fts5 build, so the caller has no cfg in it"
)]
fn write_full_text(_tx: &Transaction, _inputs: &IndexInputs) -> Result<usize, IndexError> {
    Ok(0)
}

fn check_count(tx: &Transaction, table: &str, expected: usize) -> Result<(), IndexError> {
    let found: i64 = tx
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap_or(-1);
    if usize::try_from(found).ok() != Some(expected) {
        return Err(IndexError {
            code: codes::I013,
            message: format!(
                "index holds {found} {table} rows; the resolved model holds {expected}"
            ),
        });
    }
    Ok(())
}

// -------------------------------------------------------------------------------------
// projections from the model
// -------------------------------------------------------------------------------------

use crate::model::Segment;

fn sorted(records: &[Record]) -> Vec<&Record> {
    let mut out: Vec<&Record> = records.iter().collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// The kind's required prose body, which is one slot per kind (`revisions.body`).
fn body_of(record: &Record) -> Option<String> {
    for slot in [
        ContentSlot::Definition,
        ContentSlot::Statement,
        ContentSlot::Rule,
        ContentSlot::Decision,
        ContentSlot::Intent,
        ContentSlot::Question,
    ] {
        if let Some(text) = text_slot(record, slot) {
            return Some(text);
        }
    }
    None
}

fn text_slot(record: &Record, slot: ContentSlot) -> Option<String> {
    record.get(slot).map(|value| match value {
        ContentValue::Text(text) | ContentValue::Prose(text) => text.clone(),
        ContentValue::Date(date) => date.to_string(),
        ContentValue::Commit(commit) => commit.as_str().to_owned(),
        ContentValue::Enum(segment) => segment.as_str().to_owned(),
        ContentValue::Strings(items) => items.join(" "),
        ContentValue::Globs(globs) => globs
            .iter()
            .map(|g| g.as_str().to_owned())
            .collect::<Vec<_>>()
            .join(" "),
        ContentValue::Refs(refs) => refs
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" "),
    })
}

/// A string-array slot, for the aliases the full-text index carries.
#[cfg(feature = "fts5")]
fn strings(record: &Record, slot: ContentSlot) -> Vec<String> {
    match record.get(slot) {
        Some(ContentValue::Strings(items)) => items.clone(),
        Some(other) => vec![format!("{other:?}")],
        None => Vec::new(),
    }
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect()
}

fn globs(record: &Record, slot: ContentSlot) -> Vec<String> {
    match record.get(slot) {
        Some(ContentValue::Globs(globs)) => globs.iter().map(|g| g.as_str().to_owned()).collect(),
        _ => Vec::new(),
    }
}

/// The literal part of a glob, before the first wildcard.
///
/// The index stores it so a path lookup can use an ordinary range scan instead of matching
/// every glob in the ledger against every path.
fn literal_prefix(glob: &str) -> String {
    glob.split(['*', '?', '['])
        .next()
        .unwrap_or("")
        .rsplit_once('/')
        .map_or_else(String::new, |(head, _)| format!("{head}/"))
}

fn is_archived(path: &str) -> bool {
    path.contains("/archive/") || path.starts_with("archive/")
}

fn bare_hash(hash: &str) -> String {
    hash.strip_prefix("sha256:").unwrap_or(hash).to_owned()
}

/// The DDL's convention: commit columns hold 40 hex digits without the `git:` prefix.
fn bare_commit(commit: &str) -> String {
    commit.strip_prefix("git:").unwrap_or(commit).to_owned()
}

fn severity_name(severity: crate::diagnostics::Severity) -> &'static str {
    match severity {
        crate::diagnostics::Severity::Error => "error",
        crate::diagnostics::Severity::Warning => "warning",
    }
}

/// The pipeline stage a code belongs to, as `diagnostics.stage` constrains it.
fn stage_of(code: &str) -> &'static str {
    match code.as_bytes().get(4) {
        Some(b'P') => "parse",
        Some(b'F') => "format",
        Some(b'T') => "type",
        Some(b'L') => "link",
        Some(b'R') => "resolve",
        Some(b'I') => "index",
        Some(b'E') => "emit",
        Some(b'X') => "context",
        Some(b'G') => "git",
        Some(b'M') => "migration",
        _ => "cli",
    }
}

fn span_of(inputs: &IndexInputs, id: &RevisionId) -> (i64, i64) {
    inputs
        .spans
        .get(&Subject::Revision(id.clone()))
        .map_or((0, 0), |span| (i64::from(span.start), i64::from(span.end)))
}

fn diagnostic_span(inputs: &IndexInputs, diagnostic: &Diagnostic) -> (String, i64, i64) {
    let span = diagnostic
        .primary
        .span
        .or_else(|| inputs.spans.get(&diagnostic.primary.subject));
    let path = match &diagnostic.primary.subject {
        Subject::File(path) => path.clone(),
        Subject::Revision(id) | Subject::Slot(id, _) => inputs
            .model
            .ledger()
            .get(id)
            .and_then(|record| record.file.clone())
            .unwrap_or_default(),
        Subject::Key(key) => inputs
            .model
            .ledger()
            .revisions_of(key)
            .first()
            .and_then(|record| record.file.clone())
            .unwrap_or_default(),
        Subject::Ledger => String::new(),
    };
    span.map_or((path.clone(), 0, 0), |span| {
        (path, i64::from(span.start), i64::from(span.end))
    })
}

fn subject_key(subject: &Subject) -> Option<String> {
    match subject {
        Subject::Revision(id) | Subject::Slot(id, _) => Some(id.key.to_string()),
        Subject::Key(key) => Some(key.to_string()),
        Subject::File(_) | Subject::Ledger => None,
    }
}

fn subject_rev(subject: &Subject) -> Option<i64> {
    match subject {
        Subject::Revision(id) | Subject::Slot(id, _) => Some(i64::from(id.revision)),
        _ => None,
    }
}

/// The key of whatever supersedes this revision, per stage D's chains.
fn superseded_by(model: &crate::resolve::ResolvedModel, id: &RevisionId) -> Option<String> {
    if model.is_head(id) {
        return None;
    }
    let chain = model.supersession.get(&id.key)?;
    let position = chain.iter().position(|entry| entry == id)?;
    chain
        .get(position + 1)
        .map(|successor| successor.key.to_string())
}

fn stale_detail(cause: &crate::freshness::StaleCause) -> String {
    match cause {
        crate::freshness::StaleCause::Watch { glob, commit, path } => {
            format!("{} matched {} in {}", glob.as_str(), path, commit.as_str())
        }
        crate::freshness::StaleCause::ReviewAfter { date } => date.to_string(),
    }
}

/// `@a -> @b -> @c`, so a reader sees why a record is at risk rather than only that it is.
fn at_risk_path(from: &RevisionId, path: &[RevisionId]) -> String {
    let mut out = format!("@{}", from.key);
    for step in path {
        out.push_str(&format!(" -> @{}", step.key));
    }
    out
}

/// The commit and path that made a watch fire, when one did.
fn watch_match(inputs: &IndexInputs, id: &RevisionId, glob: &str) -> Option<(String, String)> {
    inputs.queue.stale.iter().find_map(|stale| {
        if &stale.id != id {
            return None;
        }
        match &stale.cause {
            crate::freshness::StaleCause::Watch {
                glob: fired,
                commit,
                path,
            } if fired.as_str() == glob => Some((commit.as_str().to_owned(), path.clone())),
            _ => None,
        }
    })
}

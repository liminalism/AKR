//! Staleness, propagation, impact, and the review queue.
//!
//! `docs/10-freshness-and-git.md` is normative. This module implements §3 (the staleness
//! computation), §4 (reverse propagation, over [`crate::graph`]), §6
//! (`akr impact --git-diff`) and §7 (review-queue ordering).
//!
//! # The two invariants everything here rests on
//!
//! **The compiler never declares a record false.** It flags `stale` and `at_risk`, both
//! of which mean "look at this". Neither means "this is wrong" (D-003).
//!
//! **Staleness is a build fact, not a diagnostic.** No `AKR-*` code, no diagnostic
//! stream, no effect on any exit status (D-024). The `AKR-G` codes this module produces
//! report faults in the *inputs* — a commit that is not in the repository, a glob that
//! cannot match — never the flags themselves. The single opt-in exception is
//! `AKR-G041`, which reports an unmet `--review-clean` request.
//!
//! # Today is an input
//!
//! `review_after` needs a date, and it is threaded in explicitly rather than read from a
//! clock, so that two runs on the same sources at the same commit on the same stated day
//! produce identical flags (`docs/06-compiler-pipeline.md` §4).

pub mod glob;

use crate::diagnostics::{Diagnostic, Subject};
use crate::git::{GitError, Repository, Touch, codes};
use crate::graph::{AtRisk, propagate_staleness, sorted_records};
use crate::model::{
    Commit, ContentSlot, ContentValue, Date, Glob, Ledger, Record, RevisionId, State,
};
use std::collections::{BTreeMap, BTreeSet};

pub use glob::{GlobError, matches as glob_matches, validate as validate_glob};

// -------------------------------------------------------------------------------------
// Facts
// -------------------------------------------------------------------------------------

/// Why a record is stale (`docs/10-freshness-and-git.md` §3).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum StaleCause {
    /// A commit reachable from HEAD but not from `observed_at` touched a watched path.
    Watch {
        /// The glob that matched.
        glob: Glob,
        /// The commit that touched it.
        commit: Commit,
        /// The path it touched.
        path: String,
    },
    /// The `review_after` date has passed.
    ReviewAfter {
        /// The date that passed.
        date: Date,
    },
}

impl StaleCause {
    /// `watch` or `review_after`, as the index stores it
    /// (`spec/schema/index.sql`, `resolutions.stale_cause`).
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Watch { .. } => "watch",
            Self::ReviewAfter { .. } => "review_after",
        }
    }
}

/// One stale record and its cause.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Stale {
    /// The stale revision.
    pub id: RevisionId,
    /// Why.
    pub cause: StaleCause,
    /// Where it was observed.
    pub observed_at: Option<Commit>,
}

/// The review queue: what the build flagged, and what went wrong while flagging it.
#[derive(Debug, Clone, Default)]
pub struct ReviewQueue {
    /// Stale records, ordered by `docs/10-freshness-and-git.md` §7.
    pub stale: Vec<Stale>,
    /// At-risk records, ordered by propagation depth then key.
    pub at_risk: Vec<AtRisk>,
    /// `AKR-G` diagnostics about the *inputs* — never about the flags themselves.
    pub diagnostics: Vec<Diagnostic>,
}

impl ReviewQueue {
    /// The stale revisions, as a set, for [`propagate_staleness`] and for rendering.
    #[must_use]
    pub fn stale_set(&self) -> BTreeSet<RevisionId> {
        self.stale.iter().map(|s| s.id.clone()).collect()
    }

    /// Whether anything is flagged. `akr check --review-clean` fails when this is true.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.stale.is_empty() && self.at_risk.is_empty()
    }

    /// The `AKR-G041` diagnostic for an unmet `--review-clean` request, or `None`.
    ///
    /// Raised **only** on request: staleness is a build fact and never a diagnostic
    /// (D-024). A gate that fails because knowledge aged teaches contributors to delete
    /// the aged knowledge (`docs/10-freshness-and-git.md` §10).
    #[must_use]
    pub fn review_clean_diagnostic(&self) -> Option<Diagnostic> {
        (!self.is_empty()).then(|| {
            Diagnostic::error(
                codes::G041,
                crate::git::V104,
                Subject::Ledger,
                format!(
                    "review queue holds {} stale and {} at-risk records",
                    self.stale.len(),
                    self.at_risk.len()
                ),
            )
            .help("run `akr review-queue` for the full list, or drop --review-clean")
        })
    }
}

// -------------------------------------------------------------------------------------
// Derivation
// -------------------------------------------------------------------------------------

/// Derives the review queue for a ledger at a commit, on a stated day.
///
/// Implements `docs/10-freshness-and-git.md` §3 and §4 exactly, including the single bulk
/// path query of step 3: the union of every watch glob is asked of git once, and
/// per-record work is then a set intersection.
///
/// # Errors
/// Propagates any git failure that makes the question unanswerable.
pub fn derive(
    ledger: &Ledger,
    repository: &Repository,
    head: &Commit,
    today: Date,
) -> Result<ReviewQueue, GitError> {
    let mut queue = ReviewQueue::default();

    // 1. Live empirical revisions, in key order. Terminal records are not evaluated at
    //    all: a `disproven` observation has already been answered.
    let watched: Vec<&Record> = sorted_records(ledger)
        .into_iter()
        .filter(|r| r.kind.class() == crate::model::Class::Empirical && r.is_live())
        .collect();

    // 2. Validate the inputs before trusting them (V-101, V-102, V-103).
    //    Every one of those checks asks whether a record's `observed_at` is reachable from
    //    the head being resolved against, so walk that history once rather than asking git
    //    per record — the difference is one process against one per empirical record.
    if watched.len() > 1 {
        repository.prime_reachable(head);
    }
    queue
        .diagnostics
        .extend(validate_inputs(ledger, repository, head, &watched)?);

    // 3. One bulk query over the widest range any record cares about.
    let mut oldest: Option<Commit> = None;
    for candidate in watched.iter().filter_map(|r| observed_at(r)) {
        if !repository.contains(candidate) {
            continue;
        }
        oldest = match oldest {
            // Keep whichever is the ancestor of the other, so the single bulk query below
            // spans every range any record cares about.
            Some(current) if repository.is_descendant(&current, candidate)? => {
                Some(candidate.clone())
            }
            Some(current) => Some(current),
            None => Some(candidate.clone()),
        };
    }
    let touches = repository.touches_in(oldest.as_ref(), head)?;

    // 4. Per record. Both conditions are tested, and a matched watch is reported in
    //    preference to a passed date: a moved path names the change to look at, where a
    //    date only says the record is old (`docs/10-freshness-and-git.md` §3).
    let mut stale: Vec<Stale> = Vec::new();
    for record in &watched {
        let cause = match watch_cause(repository, record, &touches)? {
            Some(cause) => Some(cause),
            None => review_after(record)
                .filter(|date| *date < today)
                .map(|date| StaleCause::ReviewAfter { date }),
        };
        if let Some(cause) = cause {
            stale.push(Stale {
                id: record.id.clone(),
                cause,
                observed_at: observed_at(record).cloned(),
            });
        }
    }

    stale.sort_by_key(order_key);
    let set: BTreeSet<RevisionId> = stale.iter().map(|s| s.id.clone()).collect();
    queue.at_risk = propagate_staleness(ledger, &set);
    queue.stale = stale;
    Ok(queue)
}

/// Review-queue ordering (`docs/10-freshness-and-git.md` §7): stale before at-risk is the
/// caller's grouping; within stale, cause `watch` before cause `review_after`, then the
/// matching commit or the date, then the key.
fn order_key(entry: &Stale) -> (u8, String, String) {
    match &entry.cause {
        StaleCause::Watch { commit, .. } => (0, commit.as_str().to_owned(), entry.id.to_string()),
        StaleCause::ReviewAfter { date } => (1, date.to_string(), entry.id.to_string()),
    }
}

/// The first watch glob matched by a commit the record does not already contain.
///
/// Globs are tried in authored order and paths in sorted order, so the reported cause is
/// the same on every run.
fn watch_cause(
    repository: &Repository,
    record: &Record,
    touches: &[Touch],
) -> Result<Option<StaleCause>, GitError> {
    let Some(observed) = observed_at(record) else {
        return Ok(None);
    };
    if !repository.contains(observed) {
        // The record cites a commit this repository does not have — a rebase, a
        // force-push, or a commit from somewhere else. `AKR-G011` has already said so;
        // its freshness is simply not computable, and guessing would be worse than
        // saying nothing. One unanswerable record must not abort the whole queue.
        return Ok(None);
    }
    let globs = watches(record);
    if globs.is_empty() {
        return Ok(None);
    }
    // Memoised per record: the same commit appears on many touches.
    let mut already: BTreeMap<&Commit, bool> = BTreeMap::new();
    for glob in &globs {
        for touch in touches {
            if !glob_matches(glob, &touch.path) {
                continue;
            }
            let contained = match already.get(&touch.commit) {
                Some(answer) => *answer,
                None => {
                    let answer = repository.is_descendant(observed, &touch.commit)?;
                    already.insert(&touch.commit, answer);
                    answer
                }
            };
            if contained {
                continue; // the observation already accounts for this commit
            }
            return Ok(Some(StaleCause::Watch {
                glob: glob.clone(),
                commit: touch.commit.clone(),
                path: touch.path.clone(),
            }));
        }
    }
    Ok(None)
}

/// V-101, V-102 and V-103 over the records that carry freshness inputs.
fn validate_inputs(
    ledger: &Ledger,
    repository: &Repository,
    head: &Commit,
    watched: &[&Record],
) -> Result<Vec<Diagnostic>, GitError> {
    let mut out = Vec::new();

    // V-104's other half: a dirty tree makes a clean queue misleading.
    let dirty = repository.working_tree_changes()?;
    let watched_dirty: BTreeSet<&String> = watched
        .iter()
        .flat_map(|r| watches(r))
        .flat_map(|glob| {
            dirty
                .iter()
                .filter(move |path| glob_matches(&glob, path))
                .collect::<Vec<_>>()
        })
        .collect();
    if !watched_dirty.is_empty() {
        out.push(
            Diagnostic::warning(
                codes::G004,
                crate::git::V101,
                Subject::Ledger,
                format!(
                    "{} watched path(s) have uncommitted changes",
                    watched_dirty.len()
                ),
            )
            .help("freshness is computed from committed history only"),
        );
    }

    for record in watched {
        // V-101: observed_at exists, and is an ancestor of the resolved commit.
        if let Some(commit) = observed_at(record) {
            if repository.contains(commit) {
                if !repository.is_descendant(head, commit)? {
                    out.push(Diagnostic::warning(
                        codes::G012,
                        crate::git::V101,
                        Subject::Revision(record.id.clone()),
                        format!(
                            "{}: observed_at {commit} is not an ancestor of {head}",
                            record.id
                        ),
                    ));
                }
            } else {
                out.push(
                    Diagnostic::error(
                        codes::G011,
                        crate::git::V101,
                        Subject::Revision(record.id.clone()),
                        format!(
                            "{}: observed_at {commit} is not present in this repository",
                            record.id
                        ),
                    )
                    .help("a rebase or a force-push can strand a commit an observation cites"),
                );
            }
        }

        // V-102: watch globs are in the subset and can still match something.
        for glob in watches(record) {
            if let Err(error) = validate_glob(&glob) {
                out.push(Diagnostic::error(
                    codes::G021,
                    crate::git::V102,
                    Subject::Revision(record.id.clone()),
                    format!("{}: watches {:?}: {error}", record.id, glob.as_str()),
                ));
            }
        }

        // V-103: review_after is not earlier than created_at.
        if let (Some(review), Some(created)) = (review_after(record), record.created_at)
            && review < created
        {
            out.push(
                Diagnostic::warning(
                    codes::G031,
                    crate::git::V103,
                    Subject::Revision(record.id.clone()),
                    format!(
                        "{}: review_after {review} precedes created_at {created}",
                        record.id
                    ),
                )
                .help("the record is stale from the moment it is written"),
            );
        }
    }

    let _ = ledger;
    out.sort_by_key(Diagnostic::sort_key);
    Ok(out)
}

/// Whether a record still asserts the scope it names.
///
/// Wider than [`Record::is_live`] on purpose. Freshness is about what may still go
/// stale, so it stops at the live set; a scope is a different claim — it says what this
/// work touched — and a reader consults it long after the work finished. A census of
/// fifteen ledgers on 2026-09-08 found 88 of 254 dead scope globs sitting on `completed`
/// records, silently unreported for exactly this reason (D-010, V-102).
fn asserts_its_scope(record: &crate::model::Record) -> bool {
    record.is_live() || record.state == crate::model::State::Completed
}

/// A scope path that names something git cannot report a change to, so the record it
/// governs can never be checked against it. Not a diagnostic: the scope may be perfectly
/// correct and the exclusion perfectly deliberate. It is a build fact, in the same sense
/// staleness and scratch are (D-024, D-036).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiableScope {
    /// The record whose scope names it.
    pub id: crate::model::RevisionId,
    /// The glob as written.
    pub glob: String,
    /// Why git cannot answer for it.
    pub reason: UnverifiableReason,
}

/// Why a scope path cannot be verified against git.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnverifiableReason {
    /// The repository ignores the path, so no commit will ever touch it.
    Ignored,
    /// The path lies inside a submodule, which `ls-tree` never recurses into.
    Submodule,
    /// `project.akr` declares the tree deliberately untracked.
    Declared,
}

impl std::fmt::Display for UnverifiableReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Ignored => "the repository ignores it",
            Self::Submodule => "it lies inside a submodule",
            Self::Declared => "project.akr declares it untracked",
        })
    }
}

/// Whether a glob lies at or under a tree `project.akr` declares untracked.
///
/// Compares literal prefixes, exactly as [`inside_submodule`] does: the declaration names
/// a tree, and any scope reaching into that tree inherits it. A declaration is a claim
/// about the repository, so it is checked against git in [`unmatched_watches`] — declaring
/// a tree that is in fact tracked is an authoring mistake, not a licence to go quiet.
fn is_declared_untracked(declared: &[crate::model::Glob], glob: &str) -> bool {
    let literal = literal_prefix(glob);
    if literal.is_empty() {
        return false;
    }
    declared.iter().any(|d| {
        let root = literal_prefix(d.as_str());
        !root.is_empty() && (literal == root || literal.starts_with(&format!("{root}/")))
    })
}

/// The part of a glob before its first wildcard segment.
fn literal_prefix(glob: &str) -> String {
    glob.split('/')
        .take_while(|s| !s.contains(['*', '?', '[']))
        .collect::<Vec<_>>()
        .join("/")
}

/// Scope paths git cannot answer for, reported as facts rather than as diagnostics.
///
/// These are the two cases [`unmatched_watches`] deliberately stays quiet about, because
/// neither is an authoring mistake: an ignored path may be excluded from the repository
/// on purpose, and a submodule is tracked as a gitlink that `ls-tree` never enters. But
/// staying quiet is what let them accumulate unseen — 96 ignored and 7 submodule cases in
/// the 2026-09-08 census — so they are surfaced here, where a build fact belongs.
///
/// # Errors
/// Propagates any git failure.
pub fn unverifiable_scopes(
    ledger: &Ledger,
    repository: &Repository,
    head: &Commit,
) -> Result<Vec<UnverifiableScope>, GitError> {
    let submodules = repository.run_ls_tree_submodules(head)?;
    let mut out = Vec::new();
    for record in sorted_records(ledger) {
        if !asserts_its_scope(record) {
            continue;
        }
        for term in &record.scope {
            let crate::model::ScopeTerm::Path(glob) = term else {
                continue;
            };
            if validate_glob(glob).is_err() {
                continue;
            }
            // `.agent/scratch/**` is AKR's own working directory. It is gitignored by
            // `akr init`, `akr scratch keep` invites records to cite into it, and `akr
            // check` already reports it on its own line. Counting it here buried the real
            // findings: one workspace contributed 64 of 78 from nine scratch globs alone.
            if glob.as_str().starts_with(".agent/scratch") {
                continue;
            }
            let reason = if inside_submodule(&submodules, glob.as_str()) {
                UnverifiableReason::Submodule
            } else if is_declared_untracked(&ledger.project.untracked, glob.as_str()) {
                UnverifiableReason::Declared
            } else if repository.is_ignored(glob.as_str())? {
                UnverifiableReason::Ignored
            } else {
                continue;
            };
            out.push(UnverifiableScope {
                id: record.id.clone(),
                glob: glob.as_str().to_owned(),
                reason,
            });
        }
    }
    out.sort_by(|a, b| (&a.id, &a.glob).cmp(&(&b.id, &b.glob)));
    Ok(out)
}

/// Whether a glob's literal prefix lies at or under a submodule path.
fn inside_submodule(submodules: &std::collections::BTreeSet<String>, glob: &str) -> bool {
    let literal = literal_prefix(glob);
    if literal.is_empty() {
        return false;
    }
    submodules
        .iter()
        .any(|m| literal == *m || literal.starts_with(&format!("{m}/")))
}

/// V-102's second half, which needs the file list rather than the history: a glob that
/// matches nothing at the resolved commit can never fire again.
///
/// Separated from [`derive`] because it needs the tree listing, which is a different
/// query and one a caller may not want on every build.
///
/// # Errors
/// Propagates any git failure.
pub fn unmatched_watches(
    ledger: &Ledger,
    repository: &Repository,
    head: &Commit,
) -> Result<Vec<Diagnostic>, GitError> {
    let listing = repository.run_ls_tree(head)?;
    // Ask git about every scope glob at once. The per-glob calls below then hit the memo,
    // which on Windows is the difference between one process and one per glob.
    repository.prime_ignored(
        sorted_records(ledger)
            .iter()
            .filter(|r| asserts_its_scope(r))
            .flat_map(|record| {
                record.scope.iter().filter_map(|term| match term {
                    // Validated first, deliberately. `git check-ignore -- /abs/path/**`
                    // exits 128 `fatal: Invalid path`, and one such glob anywhere in the
                    // ledger used to fail this whole function — taking every scope and
                    // watch diagnostic in the workspace with it, silently.
                    crate::model::ScopeTerm::Path(glob) if validate_glob(glob).is_ok() => {
                        Some(glob.as_str())
                    }
                    _ => None,
                })
            }),
    );
    let submodules = repository.run_ls_tree_submodules(head)?;
    let mut out = Vec::new();
    for record in sorted_records(ledger) {
        if !asserts_its_scope(record) {
            continue;
        }
        for glob in watches(record) {
            if validate_glob(&glob).is_err() {
                continue; // AKR-G021 already reported it
            }
            if !listing.iter().any(|path| glob_matches(&glob, path)) {
                out.push(
                    Diagnostic::warning(
                        codes::G022,
                        crate::git::V102,
                        Subject::Revision(record.id.clone()),
                        format!(
                            "{}: watches {:?} matches no path at {head}",
                            record.id,
                            glob.as_str()
                        ),
                    )
                    .help("the watched code moved or was deleted; the record can no longer go stale by that glob"),
                );
            }
        }
        for term in &record.scope {
            let crate::model::ScopeTerm::Path(glob) = term else {
                continue;
            };
            // Until 0.5.0 a malformed scope glob was silently skipped, so `path "path
            // src/**"` and a double-wrapped `path "path \"a/**\""` were stored, matched
            // nothing, and surfaced — if at all — as a dead scope rather than as the
            // authoring slip they are. A malformed glob is not a dead one; say which it
            // is (D-008, V-102).
            if let Err(error) = validate_glob(glob) {
                out.push(Diagnostic::error(
                    codes::G021,
                    crate::git::V102,
                    Subject::Revision(record.id.clone()),
                    format!("{}: scope path {:?}: {error}", record.id, glob.as_str()),
                ));
                continue;
            }
            // Three silences that are not mistakes, and are reported as build facts by
            // [`unverifiable_scopes`] instead: a path the repository ignores, a path
            // inside a submodule, which `ls-tree` never recurses into, and a tree
            // `project.akr` declares deliberately untracked.
            if repository.is_ignored(glob.as_str())?
                || inside_submodule(&submodules, glob.as_str())
                || is_declared_untracked(&ledger.project.untracked, glob.as_str())
            {
                continue;
            }
            if !listing.iter().any(|path| glob_matches(glob, path)) {
                out.push(
                    Diagnostic::warning(
                        codes::G023,
                        crate::git::V102,
                        Subject::Revision(record.id.clone()),
                        format!(
                            "{}: scope path {:?} matches no tracked path at {head}",
                            record.id,
                            glob.as_str()
                        ),
                    )
                    .help("the path moved or was deleted; an unmatched scope cannot govern or become stale with its intended code"),
                );
            }
        }
    }
    // An `untracked` declaration silences every scope beneath it, so it is the one place
    // in the ledger where being wrong is invisible by construction. Check it against git
    // rather than taking the author's word: if the tree is in fact tracked, the
    // declaration is muting scopes that git could have answered for all along.
    for glob in &ledger.project.untracked {
        let root = literal_prefix(glob.as_str());
        if root.is_empty() {
            continue;
        }
        if listing
            .iter()
            .any(|path| path == &root || path.starts_with(&format!("{root}/")))
        {
            out.push(
                Diagnostic::warning(
                    codes::G026,
                    crate::git::V102,
                    Subject::Ledger,
                    format!(
                        "project.akr declares {:?} untracked, but {root:?} is tracked at {head}",
                        glob.as_str()
                    ),
                )
                .help(
                    "the declaration silences every scope beneath it; drop it, or narrow it to the part git really does not hold",
                ),
            );
        }
    }
    out.sort_by_key(Diagnostic::sort_key);
    Ok(out)
}

// -------------------------------------------------------------------------------------
// Impact
// -------------------------------------------------------------------------------------

/// What a commit range would invalidate (`docs/10-freshness-and-git.md` §6).
#[derive(Debug, Clone, Default)]
pub struct Impact {
    /// How many commits the range holds.
    pub commits: usize,
    /// Every path the range touched, sorted.
    pub touched: BTreeSet<String>,
    /// Records the range makes stale that were not stale before.
    pub newly_stale: Vec<Stale>,
    /// Records that become at-risk because of those, with depth and path.
    pub newly_at_risk: Vec<AtRisk>,
}

/// Runs the impact analysis over a commit range.
///
/// Staleness answers "what is questionable now?"; impact answers "what would this change
/// make questionable?" — the same computation against a stated range instead of against
/// `(observed_at, HEAD]`.
///
/// A record whose `observed_at` is **not** an ancestor of `from` has already accounted for
/// part of the range, so it is tested only against the commits it does not contain.
/// Without that condition every observation in the repository would be reported as
/// endangered by any large range, and nobody would run the command twice.
///
/// `already_stale` is the set the caller already knows about — usually
/// [`ReviewQueue::stale_set`]. Records in it are excluded from `newly_stale`, which is
/// what makes the result *news* rather than a restatement.
///
/// # Errors
/// Propagates any git failure, including [`GitError::UnknownRevision`] for either end.
pub fn impact_of_range(
    ledger: &Ledger,
    repository: &Repository,
    from: &Commit,
    to: &Commit,
    already_stale: &BTreeSet<RevisionId>,
) -> Result<Impact, GitError> {
    // Both ends are checked before the range is walked, so an unknown revision reports
    // `AKR-G013` — "not a commit in this repository" — rather than surfacing as a raw
    // git failure the caller has to interpret.
    for end in [from, to] {
        if !repository.contains(end) {
            return Err(GitError::UnknownRevision(end.as_str().to_owned()));
        }
    }
    let commits = repository.commits_in(Some(from), to)?;
    let touches = repository.touches_in(Some(from), to)?;
    let touched: BTreeSet<String> = touches.iter().map(|t| t.path.clone()).collect();

    let mut newly_stale = Vec::new();
    for record in sorted_records(ledger) {
        if record.kind.class() != crate::model::Class::Empirical || !record.is_live() {
            continue;
        }
        if already_stale.contains(&record.id) {
            continue;
        }
        if let Some(cause) = watch_cause(repository, record, &touches)? {
            newly_stale.push(Stale {
                id: record.id.clone(),
                cause,
                observed_at: observed_at(record).cloned(),
            });
        }
    }
    newly_stale.sort_by_key(order_key);

    let set: BTreeSet<RevisionId> = newly_stale.iter().map(|s| s.id.clone()).collect();
    let newly_at_risk = propagate_staleness(ledger, &set)
        .into_iter()
        .filter(|entry| !already_stale.contains(&entry.id))
        .collect();

    Ok(Impact {
        commits: commits.len(),
        touched,
        newly_stale,
        newly_at_risk,
    })
}

// -------------------------------------------------------------------------------------
// Slot accessors
// -------------------------------------------------------------------------------------

/// The `observed_at` of an observation or evidence record, or the `as_of` of an
/// assessment.
#[must_use]
pub fn observed_at(record: &Record) -> Option<&Commit> {
    record
        .get(ContentSlot::ObservedAt)
        .or_else(|| record.get(ContentSlot::AsOf))
        .and_then(ContentValue::as_commit)
}

/// The `watches` globs of a record, in authored order.
#[must_use]
pub fn watches(record: &Record) -> Vec<Glob> {
    match record.get(ContentSlot::Watches) {
        Some(ContentValue::Globs(globs)) => globs.clone(),
        _ => Vec::new(),
    }
}

/// The `review_after` date of a record.
#[must_use]
pub fn review_after(record: &Record) -> Option<Date> {
    match record.get(ContentSlot::ReviewAfter) {
        Some(ContentValue::Date(date)) => Some(*date),
        _ => None,
    }
}

/// Whether a state is one this module evaluates. Terminal records are never stale.
#[must_use]
pub const fn is_evaluated(state: State) -> bool {
    !matches!(
        state,
        State::Disproven | State::Superseded | State::Withdrawn
    )
}

#[cfg(test)]
mod declared_untracked_tests {
    use super::is_declared_untracked;
    use crate::model::Glob;

    #[test]
    fn a_declared_tree_covers_the_scopes_beneath_it_and_nothing_else() {
        // SaveYourSkin's shape: character art is real, on disk, and deliberately not in
        // git (280211c), while eleven records scope into it.
        let declared = [
            Glob::new("SYSEngine/assets/characters/**"),
            Glob::new("SYSEngine/assets_external/**"),
        ];
        assert!(is_declared_untracked(
            &declared,
            "SYSEngine/assets/characters/garments/**"
        ));
        assert!(is_declared_untracked(
            &declared,
            "SYSEngine/assets/characters/human/**"
        ));
        assert!(is_declared_untracked(&declared, "SYSEngine/assets_external/**"));
        // A sibling under the same parent is not covered: the declaration names a tree,
        // not a prefix of a path string.
        assert!(!is_declared_untracked(
            &declared,
            "SYSEngine/assets/object_library/**"
        ));
        assert!(!is_declared_untracked(
            &declared,
            "SYSEngine/assets_external_notes/**"
        ));
        assert!(!is_declared_untracked(&declared, "src/**"));
        // A glob that names no literal prefix cannot be matched against a tree, and must
        // not be silenced by one — otherwise `**/x.rs` would inherit every declaration.
        assert!(!is_declared_untracked(&declared, "**/lib.rs"));
        // And a declaration that is itself all wildcard silences nothing.
        assert!(!is_declared_untracked(&[Glob::new("**")], "src/**"));
    }
}

#[cfg(test)]
mod submodule_tests {
    use super::inside_submodule;
    use std::collections::BTreeSet;

    #[test]
    fn a_glob_under_a_gitlink_is_recognised_and_a_sibling_is_not() {
        // `ls-tree` reports a submodule as one entry and never recurses into it, so
        // `libbpg/**` matches nothing while `libbpg` itself is perfectly well tracked.
        let submodules: BTreeSet<String> = ["libbpg".to_owned(), "vendor/x264".to_owned()]
            .into_iter()
            .collect();
        assert!(inside_submodule(&submodules, "libbpg/**"));
        assert!(inside_submodule(&submodules, "libbpg"));
        assert!(inside_submodule(&submodules, "libbpg/src/*.c"));
        assert!(inside_submodule(&submodules, "vendor/x264/**"));
        assert!(!inside_submodule(&submodules, "libbpg-tools/**"));
        assert!(!inside_submodule(&submodules, "vendor/**"));
        assert!(!inside_submodule(&submodules, "src/**"));
        // A glob whose first segment is already a wildcard names no literal prefix.
        assert!(!inside_submodule(&submodules, "**/lib.rs"));
    }
}

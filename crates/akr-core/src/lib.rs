//! AKR core: the semantic model, its invariants, and the validation rules.
//!
//! This crate is built semantics-first (`docs/13-implementation-roadmap.md` §1). Phase P1
//! contains no text format at all: there is no lexer, no parser, and no serialisation
//! dependency. Records are constructed programmatically, and every rule in
//! `docs/05-validation-rules.md` is expressed and tested against the in-memory model
//! before any syntax exists.
//!
//! # Layout
//!
//! - [`model`] — kinds, classes, lifecycle states, relations, records, references, scope.
//! - [`syntax`] — lexer, parser, CST, canonical formatter, and lowering to the model.
//! - [`ops`] — the validated write operations of `docs/07` §4 and §6.
//! - [`validate`] — `V-001`..`V-025` as named functions over a [`model::Ledger`].
//! - [`diagnostics`] — codes, severity, subjects, and the span-ready diagnostic type.
//! - [`hash`] — SHA-256 and the three hashes of `spec/schema/akr-lock.md` §3.
//! - [`graph`] — deterministic cycles, reachability, and staleness propagation.
//! - [`resolve`] — stages C and D: linking, heads, chains, and the resolved model.
//! - [`lock`] — `akr.lock`: model, reader, writer, and verification.
//! - [`render`] — stage F: generated views, the banner, and the views-current gate.
//! - [`git`] — commits, ancestry, and changed paths, through the subprocess.
//! - [`freshness`] — staleness, propagation, impact, and the review queue.
//! - [`context`] — deterministic context assembly for agents.
//! - [`json`] — the minimal writer behind `--format json`.
//! - [`evidence`] — recording what was observed, and attaching it to a check.
//!
//! # Sources of truth
//!
//! `spec/tables/vocabulary.json` is authoritative for names; the tables in [`model`] are
//! checked against it by `tests/vocabulary.rs`. `docs/02-data-model.md` is authoritative
//! for meaning, and `spec/diagnostics/codes-lang.md` for diagnostic codes.

pub mod change;
pub mod context;
pub mod diagnostics;
pub mod evidence;
pub mod freshness;
pub mod gate;
pub mod git;
pub mod graph;
pub mod hash;
pub mod import;
pub mod ingest;
pub mod json;
pub mod lock;
pub mod model;
pub mod ops;
pub mod papercut;
pub mod render;
pub mod resolve;
pub mod scratch;
pub mod source;
pub mod store;
pub mod syntax;
pub mod validate;

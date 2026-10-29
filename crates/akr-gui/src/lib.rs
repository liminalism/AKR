//! Read-only, native review workbench for AKR workspaces.
//!
//! `akr-gui` deliberately owns no ledger parser.  The `WorkspaceLoader` seam
//! accepts the immutable projection supplied by `akr-cli`, which makes the
//! desktop process a presentation client rather than a second authority.

#![forbid(unsafe_code)]
#![allow(missing_docs)] // The adapter is intentionally public while its CLI API settles.

pub mod model;
pub mod ui;

pub use model::{
    AcceptanceCheck, Diagnostic, Record, Relation, ReviewSnapshot, SnapshotError, WorkspaceLoader,
    WorkspaceTab,
};
pub use ui::{AppModel, Filter, Panel, TreeMode};

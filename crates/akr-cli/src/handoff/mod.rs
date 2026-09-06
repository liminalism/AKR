//! Handing work to somebody else: the session head, and the advisor packet.
//!
//! Two projections live here, and they answer two different questions.
//!
//! [`session_head`] answers *"what is going on in this workspace?"* for the agent that is
//! about to do the work. It is prepended by `akr start` and `knowledge.start`.
//!
//! [`packet`] answers *"what does a second model need in order to review this
//! independently?"* — `docs/17-advisor-packets.md`, D-040. The distinction that makes it
//! worth building is one the session head does not have to make: an advisor packet
//! separates **administrative facts**, which the preparing agent is allowed to compress,
//! from **worker interpretation**, which it is not allowed to substitute for the problem.
//! The facts are what stop the advisor rediscovering the build command; the interpretation
//! is what would stop it noticing what the preparing agent missed, so it is withheld until
//! the advisor has looked for itself.
//!
//! Neither projection is authority. Records remain authoritative for intent, git for
//! snapshots, and a packet is disposable coordination state that lives outside `.akr/`.

pub mod advisor;
pub mod packet;
pub mod session_head;
pub mod snapshot;

pub use session_head::{Handoff, assemble};

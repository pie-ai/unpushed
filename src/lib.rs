//! Overview of unpushed commits across many git checkouts.
//!
//! **This is not a stable API.** The crate exists to serve the `unpushed`
//! binary and its integration tests; it is published only so that
//! `cargo install unpushed` works. Items may change or disappear in any
//! release, including patch releases. Depend on the command, not on this.
//!
//! The crate is split so the data path and the interface stay separable:
//!
//! - [`model`] — the status model, including the states that are easy to
//!   conflate (no upstream vs. nothing to push, failed scan vs. clean).
//! - [`discover`] — finding checkouts exactly one level below a directory.
//! - [`scan`] — reading local status in parallel, without touching the network.
//! - [`git`] — fetch and push, run as `git` subprocesses.
//! - [`app`] — application state and key handling, free of terminal code.
//! - [`ui`] — ratatui rendering.
//! - [`tui`] — terminal setup, event loop and worker pool.
//! - [`table`] — plain stdout rendering for non-interactive use.

pub mod app;
pub mod discover;
pub mod git;
pub mod model;
pub mod scan;
pub mod table;
pub mod tui;
pub mod ui;

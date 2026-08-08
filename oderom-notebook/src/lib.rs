//! ODEROM notebook -- Etapa 3 (DESIGN-NOTEBOOK.md): the block-notebook
//! logic layer over `oderom-session`, no window, no Tauri. Same
//! relationship `oderom-repl` has to `oderom-session`: every rule here
//! (classification, reconstruction-as-a-unit, the three declaration
//! states, error attribution, save/load) is testable by script before
//! any UI touches it.
//!
//! The one rule that makes a notebook different from the REPL:
//! `Notebook::execute_block` on a declaration block reconstructs the
//! *whole* `Model` from every declaration block's current text, every
//! time -- never an accumulation of individually-executed definitions.
//! See `notebook.rs`'s own doc comment and DESIGN-NOTEBOOK.md section 2
//! for why: that accumulation is exactly how invisible state (a
//! definition that took effect once but isn't visible in any block's
//! current text anymore) would creep back in.

mod block;
mod notebook;
mod persist;

pub use block::{Block, BlockId, BlockOutput, DeclarationStatus};
pub use notebook::{BeginExecution, Notebook};
pub use persist::{load, load_from_text, parse_sources, render, save, save_to_text, BlockContainsDelimiter};

// The gallery catalog lives in `oderom-cli` (the bottom of the
// dependency chain both this crate and the standalone CLI binary sit
// above), so `oderom load NAME` (CLI) and `Notebook::load_gallery_entry`
// (here) share exactly one list of entries -- re-exported so existing
// callers of `oderom_notebook::gallery` (this crate's own tests
// included) don't need a separate `oderom-cli` dependency just to name
// a `GalleryEntry`. See `oderom_cli::gallery`'s own doc comment for why
// it moved here from this crate.
pub use oderom_cli::gallery;

// Re-exported so a caller of this crate doesn't need a separate
// `oderom-session` dependency just to name a query block's `EntryId`,
// inspect its `EntryState` via `Notebook::session().entries()`, or (for
// `oderom-app`'s `execute_block` command, Etapa 3b) run and store a
// `Notebook::begin_execute`-returned `PendingQuery`.
pub use oderom_session::{EntryId, EntryState, PendingQuery, PendingQueryResult};

//! ODEROM UI session model -- no UI, testable headless (that's the
//! point: see DESIGN-UI-SESSION.md). Definitions (`Document`), worksheet
//! entries (`Entry`/`EntryState`), obsolescence by name, and the
//! compute cache all live here; a future Tauri crate depends on this
//! one and translates `Session`'s methods into `#[tauri::command]`s,
//! never the other way around.

mod cache;
mod document;
mod entry;
mod fingerprint;
mod run;
mod session;

pub use document::{Document, Generation};
pub use entry::{Entry, EntryId, EntryResult, EntryState};
pub use fingerprint::DefFingerprint;
pub use session::{EvalSummary, PendingQuery, PendingQueryResult, QueryStart, Session};

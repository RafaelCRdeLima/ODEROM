//! A shared cancellation flag, cheap enough to poll from inside a hot
//! arithmetic loop (`AtomicBool::load(Relaxed)`) and cloneable across
//! the thread boundary a background worker computation runs on.
//!
//! Lives here, not in `oderom-cli` (where the REPL's `ExecutionContext`
//! already had its own private `AtomicBool`) or `oderom-expr` (where the
//! deep, per-loop checks actually live), because both need the same
//! primitive and neither depends on the other: `oderom-core` is the one
//! crate everything else already depends on.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Cloning shares the same underlying flag -- `cancel()` on any clone is
/// visible to every other clone, which is the whole point: one token
/// handed to a REPL's Ctrl+C handler and also armed as the thread-local
/// scope `oderom-expr`'s hot loops poll (see `oderom_expr::cancel`).
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        CancelToken(Arc::new(AtomicBool::new(false)))
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// `Relaxed` is enough: this is a single flag with no other memory
    /// it needs to synchronize alongside, checked repeatedly in a
    /// polling loop rather than a one-shot signal that needs to
    /// happen-before anything else (same reasoning as
    /// `oderom_cli::commands::ExecutionContext::is_cancelled`, which
    /// this type now backs).
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

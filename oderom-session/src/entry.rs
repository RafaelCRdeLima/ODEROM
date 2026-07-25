//! One worksheet entry and its lifecycle -- DESIGN-UI-SESSION.md
//! section 1. Persisted form (a future save-file) is `input` alone,
//! never `state`: saving a computed result would let a stale answer
//! survive closing the app, exactly the disease this whole design
//! exists to avoid.

use std::collections::BTreeSet;

use crate::document::Generation;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct EntryId(pub u64);

pub struct Entry {
    pub id: EntryId,
    pub input: String,
    pub state: EntryState,
}

pub enum EntryState {
    /// Never run (just added) -- also what every entry becomes when a
    /// saved worksheet is reopened, since output is never persisted.
    Pending,
    /// Computing right now, on a thread of the caller's choosing --
    /// `Session::begin_query`/`finish_query` (Etapa 3b,
    /// DESIGN-NOTEBOOK.md's cancellation section) is what actually
    /// makes this state observable; before that split existed, every
    /// caller ran queries synchronously to completion and this variant,
    /// though declared, was never reachable.
    Running,
    Done { result: EntryResult, used: BTreeSet<String>, as_of: Generation },
    /// Same payload as `Done`, preserved -- never discarded. The
    /// difference is purely how the UI is meant to show it: visibly
    /// no-longer-trustworthy, not hidden and not silently kept looking
    /// current.
    Stale { result: EntryResult, used: BTreeSet<String>, as_of: Generation },
    /// The computation was interrupted before finishing -- distinct
    /// from `Failed`: this isn't a mistake in the input, it's an
    /// attempt that was deliberately stopped (`ExecutionContext::cancel`),
    /// detected via the same typed `CliError::Component(ComponentError::Cancelled)`
    /// path `oderom_expr::run_cancellable`'s unwind already produces --
    /// never string-matched on the rendered message. Carries no
    /// payload: whatever the query would have returned never existed.
    /// A caller that wants an *earlier* result to stay visible keeps
    /// that as a separate, still-`Done`/`Stale` entry rather than
    /// looking for one here -- see `oderom-notebook`'s own handling.
    Cancelled,
    Failed { message: String, line: Option<u32>, column: Option<u32> },
}

#[derive(Clone, Debug, PartialEq)]
pub struct EntryResult {
    pub latex: String,
    pub unicode: String,
    /// The same result, split into per-component pieces meant for a
    /// KaTeX-backed frontend (`oderom-app`): each entry's `formula` is
    /// exactly what a "copy this component's LaTeX" click should place
    /// on the clipboard -- clean math, no orbit-size annotation, no
    /// plain-English summary line mixed in (the bug `latex` above would
    /// have if fed to KaTeX one line at a time without splitting first;
    /// `latex`/`unicode` stay flat strings because `oderom-repl` and the
    /// CLI print them as-is to a terminal, which was never the problem).
    /// One entry, with no `orbit_note`, for a bare `scalar`/
    /// `kretschmann` result -- see `oderom-session::run::Rendered::render_structured`.
    pub components: Vec<oderom_components::RenderedComponent>,
    /// Plain-English lines with no formula of their own at all
    /// (truncation count, identically-zero count) -- never something to
    /// run through KaTeX, never something a component click should
    /// copy.
    pub summary: Vec<String>,
    pub elapsed_ms: u64,
}

impl EntryState {
    /// `true` for `Done`/`Stale` -- the two states that actually carry a
    /// result to show. `Pending`/`Running`/`Failed` don't.
    pub fn has_result(&self) -> bool {
        matches!(self, EntryState::Done { .. } | EntryState::Stale { .. })
    }

    pub fn is_stale(&self) -> bool {
        matches!(self, EntryState::Stale { .. })
    }

    /// Transitions `Done`/`Stale` to `Stale` if `used` intersects
    /// `changed_or_removed` -- the one place obsolescence actually gets
    /// applied, called once per entry from `Session::evaluate_definitions`.
    /// A no-op for every other state (nothing to go stale).
    pub fn mark_stale_if_affected(&mut self, changed_or_removed: &BTreeSet<String>) {
        let (used, result, as_of) = match self {
            EntryState::Done { used, result, as_of } => (used, result, as_of),
            _ => return,
        };
        if !used.is_disjoint(changed_or_removed) {
            *self = EntryState::Stale { result: result.clone(), used: used.clone(), as_of: *as_of };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn done(used: &[&str]) -> EntryState {
        EntryState::Done {
            result: EntryResult { latex: String::new(), unicode: String::new(), components: Vec::new(), summary: Vec::new(), elapsed_ms: 0 },
            used: used.iter().map(|s| s.to_string()).collect(),
            as_of: Generation(0),
        }
    }

    #[test]
    fn mark_stale_if_affected_transitions_on_overlap() {
        let mut state = done(&["g", "schw"]);
        state.mark_stale_if_affected(&BTreeSet::from(["schw".to_string()]));
        assert!(state.is_stale());
    }

    #[test]
    fn mark_stale_if_affected_is_a_no_op_without_overlap() {
        let mut state = done(&["g", "schw"]);
        state.mark_stale_if_affected(&BTreeSet::from(["other_metric".to_string()]));
        assert!(!state.is_stale());
        assert!(state.has_result());
    }

    #[test]
    fn pending_is_never_affected() {
        let mut state = EntryState::Pending;
        state.mark_stale_if_affected(&BTreeSet::from(["g".to_string()]));
        assert!(!state.has_result());
        assert!(!state.is_stale());
    }
}

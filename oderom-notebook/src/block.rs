//! One notebook block and its displayable output -- DESIGN-NOTEBOOK.md
//! sections 3/5. A block's *kind* (declaration vs. query) is never
//! stored: `oderom_cli::parser::classify_block` re-derives it from the
//! current `source` every time it matters, so there is nothing here
//! that can drift out of sync with what the text actually says.

use oderom_session::EntryId;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct BlockId(pub u64);

pub struct Block {
    pub id: BlockId,
    pub source: String,
    pub output: BlockOutput,
    /// Which explicit execution (Shift/Ctrl/Alt-Enter on *this* block,
    /// never a side-effected reconstruction of another declaration
    /// block) this is, session-wide -- Jupyter's own `In [n]`
    /// numbering, not tied to block position. `None` before this
    /// specific block has ever itself been the direct target of an
    /// execute. `Notebook::execute_block` is the only place this is
    /// ever set, from one shared, monotonically increasing counter
    /// (`Notebook`'s own `next_execution_count`) -- so numbers are
    /// unique and reflect the real order queries were run in, even
    /// across edits/reruns.
    pub execution_count: Option<u64>,
    /// Etapa 3b, "obsolescência visível". `source` exactly as it was
    /// the last time *this* block was itself the direct target of
    /// `execute_block` -- always `Some` exactly when `execution_count`
    /// is `Some` (the two are set together, nowhere else). Comparing
    /// current `source` against this is the whole "own edit" half of
    /// [`Block::is_obsolete`] -- a live comparison, not a flag that only
    /// ever turns on, so undoing an edit back to this exact text clears
    /// it with no special-casing.
    pub(crate) executed_source: Option<String>,
    /// The other half: set whenever an *earlier* block in the notebook
    /// is edited, deleted, or has a new block inserted after it
    /// (`Notebook::mark_cascade_obsolete_from`) -- conservative on
    /// purpose (see that function's doc comment for why: neither this
    /// crate nor a mere edit has fine-grained enough dependency
    /// information to narrow it safely without ever executing
    /// anything). Cleared only when *this* block is itself directly
    /// executed again -- "só dele": a block below an edit stays marked
    /// until it is individually reexecuted, never as a side effect of
    /// something else running.
    pub(crate) obsolete_by_cascade: bool,
}

impl Block {
    /// `true` for a block that has a real, previously-computed result
    /// on screen (declaration or query alike) that no longer
    /// trustworthily describes the current state of things -- either
    /// because its own text has since changed, or because an earlier
    /// block's edit/delete/insert conservatively might have affected
    /// it. Purely a display fact: never checked by any execution path,
    /// only read by the DTO layer (`oderom-app/src-tauri/src/lib.rs`).
    ///
    /// Always `false` for a block that has never itself been the
    /// direct target of an execute (`execution_count` is `None`) -- a
    /// block that has never shown `[n]` has nothing to mark obsolete
    /// either; it stays `[ ]`.
    pub fn is_obsolete(&self) -> bool {
        if self.execution_count.is_none() {
            return false;
        }
        self.obsolete_by_cascade || self.executed_source.as_deref() != Some(self.source.as_str())
    }

    pub(crate) fn new(id: BlockId, source: String) -> Self {
        Block { id, source, output: BlockOutput::NeverRun, execution_count: None, executed_source: None, obsolete_by_cascade: false }
    }
}

/// Three states for a declaration block (DESIGN-NOTEBOOK.md section 3)
/// -- deliberately not four: a block that has never taken part in any
/// reconstruction resolves to `Divergent`, the same "not yet reflected
/// in the live Model" description that fits it, rather than a separate
/// "never run" case.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeclarationStatus {
    /// The block's *current* text is exactly what's in the live Model.
    Confirmed,
    /// Edited (or newly created, or newly reclassified as a
    /// declaration) since the last successful reconstruction.
    Divergent,
    /// This block is the attributable cause of the last failed
    /// reconstruction. The message is the rendered `CliError` --
    /// stored as text, not the error type itself, since a block's
    /// output is a pure display fact once computed, never something a
    /// caller needs to pattern-match further.
    Error(String),
}

/// What a block currently shows below its source. `Query`'s only
/// payload is the `EntryId` -- the actual `EntryState` (`Done`/`Stale`/
/// `Failed`/...) always comes from `Session::entries()`, never
/// duplicated here, so there is exactly one place that can disagree
/// with itself.
pub enum BlockOutput {
    /// Never executed, and never swept into a declaration
    /// reconstruction triggered by another block either -- the only
    /// state before this block's kind is even known.
    NeverRun,
    Declaration(DeclarationStatus),
    Query(EntryId),
    /// Etapa 3b (cancelamento, DESIGN-NOTEBOOK.md): the block's most
    /// recent execution attempt is still running or ended in
    /// cancellation -- `attempt`'s own `EntryState` (`Running` or
    /// `Cancelled`) is which; read via `Session::entries()`, same as
    /// `Query`'s payload, never duplicated here. Settles back to plain
    /// `Query(attempt)` the moment the attempt finishes normally
    /// (`Notebook::finish_query`) -- this variant only exists for the
    /// in-between and the cancelled-forever cases.
    ///
    /// `previous`, if `Some`, is an *earlier* successful/failed run's
    /// entry that predates this attempt -- deliberately kept alive, not
    /// removed, so its result stays visible (obsolete, never hidden)
    /// for as long as this attempt is running or stays cancelled,
    /// instead of vanishing the instant a reexecution starts.
    Attempt { attempt: EntryId, previous: Option<EntryId> },
    /// `classify_block` itself failed: the leading keyword is none of
    /// the ten recognized ones (including an empty block). The
    /// rendered `CliError`, same reasoning as `DeclarationStatus::Error`.
    Unrecognized(String),
}

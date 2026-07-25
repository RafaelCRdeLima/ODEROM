//! The notebook itself: an ordered list of [`Block`]s over one
//! [`Session`] -- DESIGN-NOTEBOOK.md sections 2-6. The one rule
//! everything here exists to implement: "the Model is derived from the
//! current text of every declaration block, reconstructed as a unit
//! whenever any declaration block is executed" -- never an accumulation
//! of individually-executed definitions, which is exactly how invisible
//! state would creep back in.

use crate::block::{Block, BlockId, BlockOutput, DeclarationStatus};
use oderom_cli::commands::ExecutionContext;
use oderom_cli::parser::{classify_block, parse_model, BlockKind};
use oderom_cli::CliError;
use oderom_session::{PendingQuery, QueryStart, Session};
use std::sync::Arc;

pub struct Notebook {
    session: Session,
    blocks: Vec<Block>,
    next_block_id: u64,
    /// Shared source for `Block::execution_count` -- one counter for
    /// the whole notebook, matching Jupyter's own `In [n]`: it counts
    /// explicit executions in the order they happened, not block
    /// position, and never resets on edit or reconstruction.
    next_execution_count: u64,
    /// Set by `load`/`save` (`persist.rs`) -- `None` for a notebook that
    /// has never been saved to or opened from a file (e.g. a freshly
    /// seeded example). Purely informational (what to show as the
    /// notebook's name, what path a bare "save" would reuse); never
    /// read by any of the logic above.
    current_path: Option<std::path::PathBuf>,
    /// Etapa 3b, segunda parte (exclusão mútua, DESIGN-NOTEBOOK.md
    /// section 10.8): the block currently executing, if any, and the
    /// handle needed to cancel it. `Option`, not a map -- deliberately
    /// global, not per-block: at most one execution runs at a time
    /// across the *whole* notebook, never two blocks concurrently. A
    /// declaration and a query that depends on it running in parallel
    /// would make the query's result depend on which thread the OS
    /// scheduler happened to run first, not on the text on screen --
    /// silent semantic non-determinism, not a memory-safety data race,
    /// and worse for it (nothing crashes to reveal it). See
    /// `begin_execute`'s own doc comment for the full reasoning and
    /// what was rejected instead.
    running: Option<(BlockId, Arc<ExecutionContext>)>,
}

impl Default for Notebook {
    fn default() -> Self {
        Notebook::new()
    }
}

/// What [`Notebook::begin_execute`] hands back -- see that method's own
/// doc comment.
pub enum BeginExecution {
    Done,
    Started(PendingQuery),
    /// Refused: some *other* execution (`by` -- which may be this same
    /// block, if it was already running) is already in flight, and
    /// `oderom-notebook` never runs two at once. Nothing about the
    /// requested block changed -- no `execution_count` bump, no
    /// `output` touched, exactly as if this call had never happened.
    Blocked { by: BlockId },
    NotFound,
}

impl Notebook {
    pub fn new() -> Self {
        Notebook { session: Session::new(), blocks: Vec::new(), next_block_id: 0, next_execution_count: 1, current_path: None, running: None }
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    pub fn block(&self, id: BlockId) -> Option<&Block> {
        self.blocks.iter().find(|b| b.id == id)
    }

    pub fn current_path(&self) -> Option<&std::path::Path> {
        self.current_path.as_deref()
    }

    pub(crate) fn set_current_path(&mut self, path: Option<std::path::PathBuf>) {
        self.current_path = path;
    }

    /// Inserts a new, empty-output block right after `after` (or at the
    /// end, if `after` is `None` or no longer present). Creation is the
    /// one place order is decided in v1 -- blocks are not reorderable
    /// once created (DESIGN-NOTEBOOK.md section 1).
    ///
    /// Etapa 3b: also conservatively marks every already-executed block
    /// *below* the insertion point obsolete (`mark_cascade_obsolete_from`'s
    /// own doc comment has the reasoning) -- a block inserted between
    /// two others can affect line-based error attribution or introduce
    /// a name that changes what they mean, and there is no cheap way to
    /// know it didn't without reconstructing, which a mere insertion
    /// must never do.
    pub fn create_block_after(&mut self, after: Option<BlockId>, source: String) -> BlockId {
        let id = BlockId(self.next_block_id);
        self.next_block_id += 1;
        let block = Block::new(id, source);
        let insert_index = match after.and_then(|a| self.blocks.iter().position(|b| b.id == a)) {
            Some(index) => {
                self.blocks.insert(index + 1, block);
                index + 1
            }
            None => {
                self.blocks.push(block);
                self.blocks.len() - 1
            }
        };
        self.mark_cascade_obsolete_from(insert_index + 1);
        id
    }

    /// `load NOME` (Rodada Galeria): pastes gallery entry `name`'s
    /// declaration text as new blocks, one per `GalleryEntry::blocks`
    /// entry, right after `after` -- exactly [`Notebook::create_block_after`]
    /// called once per block, chained so each new block follows the
    /// previous one in the order the entry lists them. Never touches any
    /// existing block: this is pure text injection (`gallery` module's
    /// own doc comment), so the same conservative cascade-obsolete
    /// marking `create_block_after` already does for any insertion is
    /// all that happens to the rest of the notebook. Returns the new
    /// blocks' ids in order, or `Err` if `name` isn't in
    /// [`oderom_cli::gallery::ENTRIES`] (re-exported as `crate::gallery`)
    /// -- nothing created either way.
    pub fn load_gallery_entry(&mut self, after: Option<BlockId>, name: &str) -> Result<Vec<BlockId>, oderom_cli::gallery::UnknownGalleryEntry> {
        let entry = oderom_cli::gallery::find(name).ok_or_else(|| oderom_cli::gallery::UnknownGalleryEntry { name: name.to_string() })?;
        let mut cursor = after;
        let mut created = Vec::with_capacity(entry.blocks.len());
        for block_source in entry.blocks {
            let id = self.create_block_after(cursor, (*block_source).to_string());
            cursor = Some(id);
            created.push(id);
        }
        Ok(created)
    }

    /// Replaces `id`'s source. A block that has never itself been
    /// directly executed reclassifies immediately -- "editar ->
    /// divergente" is instantaneous, not deferred to the next
    /// Shift+Enter (DESIGN-NOTEBOOK.md section 3) -- since there is no
    /// previous result on screen to preserve.
    ///
    /// A block that *has* a previously-executed result (declaration or
    /// query alike) is different, per Etapa 3b: `output` is left
    /// completely untouched here, no matter how the new text
    /// classifies -- the old result stays on screen exactly as it was,
    /// now readable as obsolete via `Block::is_obsolete` (a comparison
    /// against `executed_source`, not a mutation performed here). Only
    /// `execute_block` ever replaces a previously-executed block's
    /// output. This is a deliberate reversal of this crate's earlier
    /// "drop the old result on edit" policy (DESIGN-NOTEBOOK.md section
    /// 5's original wording) -- showing nothing next to changed source
    /// turned out to be its own kind of lie by omission once a block
    /// could be edited long after execution: it hid a result that was
    /// still true a moment ago instead of saying so.
    ///
    /// Either way, every already-executed block strictly below this one
    /// is conservatively marked obsolete too, if the text actually
    /// changed (`mark_cascade_obsolete_from`).
    pub fn edit_block(&mut self, id: BlockId, new_source: String) {
        let Some(index) = self.blocks.iter().position(|b| b.id == id) else { return };
        let source_changed = self.blocks[index].source != new_source;
        self.blocks[index].source = new_source.clone();

        if self.blocks[index].execution_count.is_none() {
            self.blocks[index].output = match classify_block(&new_source) {
                Ok(BlockKind::Declaration) => BlockOutput::Declaration(DeclarationStatus::Divergent),
                Ok(BlockKind::Query) => BlockOutput::NeverRun,
                Err(e) => BlockOutput::Unrecognized(e.to_string()),
            };
        }

        if source_changed {
            self.mark_cascade_obsolete_from(index + 1);
        }
    }

    /// Removes `id`. Does *not* trigger a declaration reconstruction --
    /// a deleted declaration's definition only disappears from the
    /// `Model` at the next reconstruction, same as leaving it out of a
    /// hand-edited `.od` file would (DESIGN-NOTEBOOK.md section 2). A
    /// query block's entry, if any, is removed from the session right
    /// away -- same reasoning as `Session::remove_entry`'s own doc
    /// comment: an orphaned entry for a block that no longer exists is
    /// exactly the kind of thing that degrades the obsolescence scan
    /// over a session's lifetime.
    ///
    /// Etapa 3b: also conservatively marks every already-executed block
    /// that was below `id` obsolete, same reasoning as
    /// `create_block_after`'s own doc comment, mirrored for removal.
    /// Cancels any execution in flight for `id` first (nothing left to
    /// finish for a block that's gone -- `finish_query`'s own
    /// not-found check would already no-op harmlessly if this were
    /// skipped, but cancelling promptly means the worker thread stops
    /// doing wasted work instead of running to completion unread), then
    /// removes whatever entries the block referenced.
    pub fn delete_block(&mut self, id: BlockId) {
        let Some(index) = self.blocks.iter().position(|b| b.id == id) else { return };
        if self.running.as_ref().is_some_and(|(running_id, _)| *running_id == id) {
            let (_, ctx) = self.running.take().expect("just confirmed Some above");
            ctx.cancel();
        }
        match self.blocks[index].output {
            BlockOutput::Query(old_id) => self.session.remove_entry(old_id),
            BlockOutput::Attempt { attempt, previous } => {
                self.session.remove_entry(attempt);
                if let Some(prev_id) = previous {
                    self.session.remove_entry(prev_id);
                }
            }
            BlockOutput::NeverRun | BlockOutput::Declaration(_) | BlockOutput::Unrecognized(_) => {}
        }
        self.blocks.remove(index);
        // `index` now names what was the *next* block -- everything
        // that used to be below the deleted one.
        self.mark_cascade_obsolete_from(index);
    }

    /// Shift+Enter, the synchronous, blocking-until-done form:
    /// classifies `id`'s current text and routes accordingly -- a query
    /// block runs to completion on the calling thread (fine for tests
    /// and any caller that doesn't need the window to stay responsive
    /// meanwhile, same reasoning as `Session::run_entry` vs.
    /// `run_entry_with_context`); a declaration block triggers a full
    /// reconstruction of *every* declaration block's current text
    /// (DESIGN-NOTEBOOK.md section 2), not just this one. A block whose
    /// text doesn't classify as either shows that fact directly, never
    /// silently guessed.
    ///
    /// `oderom-app` calls [`Notebook::begin_execute`] instead, which
    /// returns immediately for a query and hands the actual computation
    /// to a caller-chosen thread (Etapa 3b, cancellation) -- this method
    /// is really just `begin_execute` followed by running the
    /// `PendingQuery` and `finish_query` right here, with no thread
    /// boundary at all.
    pub fn execute_block(&mut self, id: BlockId) {
        if let BeginExecution::Started(pending) = self.begin_execute(id) {
            let result = pending.run();
            self.finish_query(id, result);
        }
    }

    /// The cancellable half of `execute_block`. Declaration/unrecognized
    /// blocks (and a query that fails before there was anything to
    /// thread off -- no document yet, or unparseable input) run fully
    /// synchronously here already, same as `execute_block` always did:
    /// [`BeginExecution::Done`] means there is nothing further to do.
    /// [`BeginExecution::Started`] means a query's computation has been
    /// handed off -- call `.run()` on it from any thread that must not
    /// hold whatever lock guards this `Notebook`, then feed the result
    /// to [`Notebook::finish_query`] (same `BlockId`).
    ///
    /// [`BeginExecution::Blocked`], untouched and unstamped, if *any*
    /// execution is already in flight anywhere in the notebook -- not
    /// only for `id` itself. Deliberate, decided explicitly (Etapa 3b,
    /// segunda parte, DESIGN-NOTEBOOK.md section 10.8), not a leftover
    /// side effect of the cancellable split: running a declaration and
    /// a query that depends on it concurrently would let the query's
    /// answer depend on which of the two threads the OS scheduler
    /// happens to run first, against the same keystrokes, in the same
    /// notebook -- silent semantic non-determinism, worse than the
    /// `ComputeCache` sharing concern the cancellation work first
    /// flagged, and the reason mutual exclusion won over a queue (a
    /// queue is its own new "waiting to run" state with no good answer
    /// for "the user edited or deleted a queued block", and is the
    /// closest this project should come to "something executes without
    /// being asked to at that moment"). Blocking only became acceptable
    /// once cancellation existed: the user is never stuck, always able
    /// to cancel the one thing running and free up everything else.
    pub fn begin_execute(&mut self, id: BlockId) -> BeginExecution {
        let Some(index) = self.blocks.iter().position(|b| b.id == id) else { return BeginExecution::NotFound };
        if let Some((busy_id, _)) = &self.running {
            return BeginExecution::Blocked { by: *busy_id };
        }

        // Stamped on *this* block only, regardless of outcome (matches
        // Jupyter: `In [n]` still advances on a cell that errors) --
        // never on other declaration blocks a reconstruction happens to
        // sweep in as a side effect. One counter, session-wide, so the
        // numbers are the real order queries ran in even across edits.
        self.blocks[index].execution_count = Some(self.next_execution_count);
        self.next_execution_count += 1;

        let source = self.blocks[index].source.clone();
        // Etapa 3b: "executar de novo limpa a marca dele -- só dele."
        // Snapshot the text being run right now so a later edit can be
        // compared against exactly this, and clear this block's own
        // cascade mark -- nobody else's `obsolete_by_cascade` is
        // touched here, including blocks a declaration reconstruction
        // sweeps in as a side effect below.
        self.blocks[index].executed_source = Some(source.clone());
        self.blocks[index].obsolete_by_cascade = false;

        match classify_block(&source) {
            Ok(BlockKind::Query) => self.begin_execute_query(index, id, source),
            Ok(BlockKind::Declaration) => {
                self.reconstruct_declarations(id);
                BeginExecution::Done
            }
            Err(e) => {
                self.blocks[index].output = BlockOutput::Unrecognized(e.to_string());
                BeginExecution::Done
            }
        }
    }

    fn begin_execute_query(&mut self, index: usize, id: BlockId, source: String) -> BeginExecution {
        // An earlier result (or a still-running/cancelled attempt's own
        // reference to one) survives a reexecution until the outcome of
        // *this* attempt is known -- "o resultado antigo continua
        // visível" applies here exactly as it does to a plain edit.
        let previous = match self.blocks[index].output {
            BlockOutput::Query(old_id) => Some(old_id),
            BlockOutput::Attempt { previous, .. } => previous,
            BlockOutput::NeverRun | BlockOutput::Declaration(_) | BlockOutput::Unrecognized(_) => None,
        };
        match self.session.begin_query(source) {
            QueryStart::Failed(new_id) => {
                // Nothing was actually threaded off (no document yet,
                // or the input didn't even parse) -- behaves exactly
                // like today's synchronous failure always has: the new
                // result replaces the old one right away, same as any
                // other non-cancelled reexecution.
                if let Some(old_id) = previous {
                    self.session.remove_entry(old_id);
                }
                self.blocks[index].output = BlockOutput::Query(new_id);
                BeginExecution::Done
            }
            QueryStart::Running(new_id, pending) => {
                self.running = Some((id, pending.context()));
                self.blocks[index].output = BlockOutput::Attempt { attempt: new_id, previous };
                BeginExecution::Started(pending)
            }
        }
    }

    /// The other half of [`Notebook::begin_execute`]'s `Started` case:
    /// stores a `PendingQuery::run()` result, called with `id` and no
    /// other assumption about which thread ran the computation.
    ///
    /// If the attempt was cancelled, the block *stays* `Attempt{attempt,
    /// previous}` -- `attempt`'s own `EntryState` is now `Cancelled`,
    /// and `previous` (if any) keeps carrying an earlier real result,
    /// still visible, forever obsolete until this specific block is
    /// reexecuted (DESIGN-NOTEBOOK.md's cancellation section: "não
    /// apague saída por causa de cancelamento"). If it finished
    /// normally (`Done`/`Failed`), the block settles back to plain
    /// `Query(attempt)` and `previous`, if any, is finally removed --
    /// exactly like any other non-cancelled reexecution always has.
    ///
    /// A silent no-op if `id` no longer exists (deleted while its
    /// computation was still in flight) or its output somehow isn't
    /// `Attempt` anymore (defensive only -- nothing in this crate
    /// produces that combination; see `begin_execute`'s own
    /// already-running guard, which is what keeps this call unambiguous
    /// about which attempt it's for).
    pub fn finish_query(&mut self, id: BlockId, result: oderom_session::PendingQueryResult) {
        if self.running.as_ref().is_some_and(|(running_id, _)| *running_id == id) {
            self.running = None;
        }
        let Some(index) = self.blocks.iter().position(|b| b.id == id) else { return };
        let BlockOutput::Attempt { attempt, previous } = self.blocks[index].output else { return };
        let was_cancelled = self.session.finish_query(attempt, result);
        if !was_cancelled {
            if let Some(old_id) = previous {
                self.session.remove_entry(old_id);
            }
            self.blocks[index].output = BlockOutput::Query(attempt);
        }
    }

    /// Requests cancellation of `id`'s in-flight execution, if any --
    /// same `ExecutionContext::cancel` mechanism the REPL's Ctrl+C
    /// already uses, just reached through a button instead of a signal
    /// handler. A no-op if `id` isn't currently running (already
    /// settled, or never started) -- never an error, matching this
    /// crate's existing "no such id, nothing to do" convention
    /// (`Session::remove_entry`'s own doc comment). Returns
    /// immediately either way: this only *requests* cancellation, it
    /// does not wait for the computation to actually stop (that's
    /// `finish_query`, called once it does).
    pub fn cancel_block(&mut self, id: BlockId) {
        if let Some((running_id, ctx)) = &self.running {
            if *running_id == id {
                ctx.cancel();
            }
        }
    }

    /// The core rule (DESIGN-NOTEBOOK.md section 2): concatenate every
    /// block currently classifying as a declaration, in notebook order,
    /// and reevaluate as one unit -- exactly what `:reload` already
    /// does with a whole `.od` file's text. `just_executed` is only
    /// used as the last-resort fallback for error attribution (section
    /// 4) when even the incremental-prefix search can't pin a culprit,
    /// which should not happen in practice.
    fn reconstruct_declarations(&mut self, just_executed: BlockId) {
        let mut decl_ids: Vec<BlockId> = Vec::new();
        let mut concatenated = String::new();
        let mut line_ranges: Vec<(BlockId, u32, u32)> = Vec::new();
        let mut current_line: u32 = 1;

        for block in &self.blocks {
            if matches!(classify_block(&block.source), Ok(BlockKind::Declaration)) {
                let start = current_line;
                concatenated.push_str(&block.source);
                concatenated.push('\n');
                let lines_in_block = block.source.lines().count().max(1) as u32;
                current_line += lines_in_block;
                line_ranges.push((block.id, start, current_line - 1));
                decl_ids.push(block.id);
            }
        }

        match self.session.evaluate_definitions(concatenated) {
            Ok(_summary) => {
                for id in &decl_ids {
                    self.set_declaration_status(*id, DeclarationStatus::Confirmed);
                }
            }
            Err(err) => {
                let culprit = attribute_error(&err, &self.blocks, &decl_ids, &line_ranges).unwrap_or(just_executed);
                for id in &decl_ids {
                    if *id == culprit {
                        self.set_declaration_status(*id, DeclarationStatus::Error(err.to_string()));
                    } else if !matches!(self.block(*id).map(|b| &b.output), Some(BlockOutput::Declaration(_))) {
                        // Swept into a reconstruction for the first time
                        // (was `NeverRun`, `Query`, or `Unrecognized`) --
                        // there is no prior `DeclarationStatus` to keep,
                        // so resolve to the honest "not yet confirmed"
                        // state rather than inventing a fourth one.
                        self.set_declaration_status(*id, DeclarationStatus::Divergent);
                    }
                    // else: already `Declaration(_)` -- "os demais
                    // mantêm o que tinham", left untouched.
                }
            }
        }
    }

    fn set_declaration_status(&mut self, id: BlockId, status: DeclarationStatus) {
        if let Some(block) = self.blocks.iter_mut().find(|b| b.id == id) {
            block.output = BlockOutput::Declaration(status);
        }
    }

    /// Etapa 3b, "obsolescência visível" -- the conservative half.
    /// Marks every block at or after `from_index` that has ever itself
    /// been directly executed (`execution_count.is_some()`) obsolete.
    ///
    /// Why conservative rather than precise: `oderom-session` already
    /// tracks fine-grained, per-symbol dependency information for query
    /// entries (`Entry`'s `used: BTreeSet<String>`, compared against
    /// `changed_or_removed_names` in `Session::evaluate_definitions`),
    /// but it only updates when a reconstruction actually *runs* --
    /// which happens only from an explicit declaration-block execution,
    /// never from a mere edit. This function is called from
    /// `edit_block`/`delete_block`/`create_block_after`, none of which
    /// may ever reconstruct or run anything ("nada recalcula sozinho"),
    /// so there is no fresher dependency information available at the
    /// moment a cascade needs deciding. `oderom-notebook` itself has no
    /// block-to-block dependency graph at all -- blocks are positions
    /// over one shared `Session`, not nodes with tracked edges. Marking
    /// every executed block below the change point errs toward warning
    /// too much, which is the safe side to err on.
    fn mark_cascade_obsolete_from(&mut self, from_index: usize) {
        for block in self.blocks.iter_mut().skip(from_index) {
            if block.execution_count.is_some() {
                block.obsolete_by_cascade = true;
            }
        }
    }

    /// "Limpar execução" -- the honest "kernel reset": every block's
    /// execution state goes back to freshly parsed (never run), and
    /// every scrap of session state (the `Document`/`Model` -- including
    /// declared aliases, which live entirely inside it and nowhere else,
    /// see `oderom-session::Document`'s own doc comment -- every
    /// `Entry`, the whole `ComputeCache`) is discarded, along with
    /// `next_execution_count` itself, so the very next execution after
    /// this is `[1]`, not a continuation of whatever numbering was
    /// already in flight. Block SOURCE TEXT is the one thing this never
    /// touches, and neither is `current_path` -- this never reads or
    /// writes any file.
    ///
    /// The correctness bar this exists to satisfy: the state right after
    /// this call must be indistinguishable from opening this exact
    /// text -- byte for byte, same blocks, same order -- as a brand new
    /// file. Implemented by literally reusing that same "how do you
    /// build fresh block state from source text alone" logic
    /// `persist::load` already has (extract every block's current
    /// source, in order, and replay it through `create_block_after` on a
    /// brand new `Notebook`) rather than hand-rolling a second version
    /// of it that could quietly drift out of sync -- there is exactly
    /// one place in this crate that knows how to construct "freshly
    /// opened" block state.
    ///
    /// Any execution still in flight is cancelled first (mirroring
    /// `delete_block`'s own precedent) and `next_block_id` is
    /// deliberately NOT reset along with everything else: block ids
    /// restart at 0 for a `Notebook::new()`, but this keeps counting up
    /// from wherever it already was, so a cancelled attempt's eventual,
    /// late-arriving `finish_query` call can never land on an unrelated
    /// new block that happens to reuse its old numeric id -- the one
    /// place this reset is deliberately NOT a byte-for-byte "as if
    /// freshly opened" state, because a freshly opened file was never
    /// running anything to begin with, and this one might have been the
    /// instant before this call.
    pub fn clear_execution(&mut self) {
        if let Some((_, ctx)) = self.running.take() {
            ctx.cancel();
        }
        let sources: Vec<String> = self.blocks.iter().map(|b| b.source.clone()).collect();
        let path = self.current_path.take();
        let next_block_id = self.next_block_id;
        *self = Notebook::new();
        self.next_block_id = next_block_id;
        self.current_path = path;
        for source in sources {
            self.create_block_after(None, source);
        }
    }
}

/// DESIGN-NOTEBOOK.md section 4. A `Parse` error carries a line number
/// that maps directly to one block's tracked range -- O(1), exact. Any
/// other error (post-parse `Model` construction -- `CoreError::DuplicateName`
/// and friends, which have no position because they happen after
/// parsing) has none, so this reparses growing prefixes of the *same*
/// declaration blocks, in the *same* order, until the same kind of
/// failure reappears: the block whose addition first breaks it is
/// unambiguously the cause. For a duplicate name this lands on exactly
/// the second block that declares it, with no special case for that
/// error type -- the mechanism is general, not name-specific.
fn attribute_error(err: &CliError, blocks: &[Block], decl_ids: &[BlockId], line_ranges: &[(BlockId, u32, u32)]) -> Option<BlockId> {
    if let CliError::Parse { position: Some(pos), .. } = err {
        if let Some((id, _, _)) = line_ranges.iter().find(|(_, start, end)| pos.line >= *start && pos.line <= *end) {
            return Some(*id);
        }
    }
    let mut prefix = String::new();
    for id in decl_ids {
        let block = blocks.iter().find(|b| b.id == *id).expect("decl_ids only contains ids present in blocks");
        prefix.push_str(&block.source);
        prefix.push('\n');
        if parse_model(&prefix).is_err() {
            return Some(*id);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use oderom_session::EntryState;

    fn decl_status(nb: &Notebook, id: BlockId) -> &DeclarationStatus {
        match &nb.block(id).unwrap().output {
            BlockOutput::Declaration(status) => status,
            BlockOutput::NeverRun => panic!("expected a Declaration output, got NeverRun"),
            BlockOutput::Query(_) => panic!("expected a Declaration output, got Query"),
            BlockOutput::Attempt { .. } => panic!("expected a Declaration output, got Attempt"),
            BlockOutput::Unrecognized(msg) => panic!("expected a Declaration output, got Unrecognized({msg:?})"),
        }
    }

    // ---------------------------------------------------------------
    // Query blocks: create, execute, edit, delete, reexecute
    // ---------------------------------------------------------------

    const SCHWARZSCHILD_A: &str = "manifold M dim 4\nbundle TM on M dim 4";
    const SCHWARZSCHILD_B: &str = "chart schw on M coords (t, r, theta, phi)";
    const SCHWARZSCHILD_C: &str = "metric g on schw bundle TM {\n  [t,t] = -(1 - 2*M/r),\n  [r,r] = 1/(1 - 2*M/r),\n  [theta,theta] = r^2,\n  [phi,phi] = r^2 * sin(theta)^2\n}";

    fn schwarzschild_notebook() -> (Notebook, BlockId, BlockId, BlockId) {
        let mut nb = Notebook::new();
        let a = nb.create_block_after(None, SCHWARZSCHILD_A.to_string());
        let b = nb.create_block_after(Some(a), SCHWARZSCHILD_B.to_string());
        let c = nb.create_block_after(Some(b), SCHWARZSCHILD_C.to_string());
        nb.execute_block(c); // any declaration block reconstructs all three
        (nb, a, b, c)
    }

    #[test]
    fn a_query_block_executes_and_shows_a_result() {
        let (mut nb, ..) = schwarzschild_notebook();
        let q = nb.create_block_after(None, "ricci".to_string());
        nb.execute_block(q);
        let BlockOutput::Query(id) = nb.block(q).unwrap().output else { panic!("expected Query output") };
        let entry = nb.session().entries().iter().find(|e| e.id == id).unwrap();
        assert!(entry.state.has_result());
    }

    #[test]
    fn reexecuting_a_query_block_many_times_does_not_grow_session_entries() {
        let (mut nb, ..) = schwarzschild_notebook();
        let q = nb.create_block_after(None, "ricci".to_string());
        for _ in 0..50 {
            nb.execute_block(q);
        }
        assert_eq!(nb.session().entries().len(), 1, "expected exactly one live entry for the one query block, got {}", nb.session().entries().len());
    }

    #[test]
    fn a_freshly_created_block_has_no_execution_count() {
        let mut nb = Notebook::new();
        let id = nb.create_block_after(None, "manifold M dim 4".to_string());
        assert_eq!(nb.block(id).unwrap().execution_count, None);
    }

    #[test]
    fn executing_a_declaration_block_stamps_only_itself_not_the_blocks_it_sweeps_in() {
        // schwarzschild_notebook() already executed block c (the
        // metric), which reconstructs a and b too as a side effect --
        // but a and b were never themselves the direct target of an
        // execute, so they must not have a Jupyter-style number either.
        let (nb, a, b, c) = schwarzschild_notebook();
        assert_eq!(nb.block(c).unwrap().execution_count, Some(1));
        assert_eq!(nb.block(a).unwrap().execution_count, None, "a was only swept into c's reconstruction, never itself executed");
        assert_eq!(nb.block(b).unwrap().execution_count, None, "b was only swept into c's reconstruction, never itself executed");
    }

    #[test]
    fn execution_counts_increase_monotonically_in_the_order_blocks_are_run() {
        let (mut nb, a, b, _c) = schwarzschild_notebook();
        nb.execute_block(a);
        nb.execute_block(b);
        let ea = nb.block(a).unwrap().execution_count.unwrap();
        let eb = nb.block(b).unwrap().execution_count.unwrap();
        assert!(eb > ea, "expected b's execution number ({eb}) to be greater than a's ({ea})");
    }

    #[test]
    fn rerunning_a_block_gives_it_a_new_higher_execution_count_each_time() {
        let (mut nb, a, ..) = schwarzschild_notebook();
        nb.execute_block(a);
        let first = nb.block(a).unwrap().execution_count.unwrap();
        nb.execute_block(a);
        let second = nb.block(a).unwrap().execution_count.unwrap();
        assert!(second > first, "expected a fresh, higher number on rerun: first={first}, second={second}");
    }

    #[test]
    fn editing_a_directly_executed_query_block_keeps_the_old_result_visible_but_obsolete() {
        // Etapa 3b reverses the earlier "drop the result on edit"
        // policy for exactly this case: a block that has itself been
        // directly executed keeps showing that result, unchanged, until
        // it is reexecuted -- editing only makes it readable as
        // obsolete (see `edit_block`'s own doc comment).
        let (mut nb, ..) = schwarzschild_notebook();
        let q = nb.create_block_after(None, "ricci".to_string());
        nb.execute_block(q);
        let BlockOutput::Query(id) = nb.block(q).unwrap().output else { panic!("expected Query output") };
        assert!(!nb.block(q).unwrap().is_obsolete());

        nb.edit_block(q, "scalar".to_string());
        assert!(
            matches!(nb.block(q).unwrap().output, BlockOutput::Query(seen_id) if seen_id == id),
            "editing a directly-executed block must not touch its output at all -- only reexecuting may"
        );
        assert_eq!(nb.session().entries().len(), 1, "the old entry must survive an edit, not be dropped");
        assert!(nb.block(q).unwrap().is_obsolete(), "the block's own text no longer matches what was executed");
    }

    #[test]
    fn deleting_a_query_block_removes_its_session_entry() {
        let (mut nb, ..) = schwarzschild_notebook();
        let q = nb.create_block_after(None, "ricci".to_string());
        nb.execute_block(q);
        assert_eq!(nb.session().entries().len(), 1);
        nb.delete_block(q);
        assert_eq!(nb.session().entries().len(), 0);
        assert!(nb.block(q).is_none());
    }

    #[test]
    fn a_block_with_no_recognized_leading_keyword_is_unrecognized_not_silently_guessed() {
        let mut nb = Notebook::new();
        let id = nb.create_block_after(None, "bogus nonsense".to_string());
        nb.execute_block(id);
        assert!(matches!(nb.block(id).unwrap().output, BlockOutput::Unrecognized(_)));
    }

    // ---------------------------------------------------------------
    // Declaration blocks: reconstruction as a unit, the three states
    // ---------------------------------------------------------------

    #[test]
    fn executing_any_declaration_block_reconstructs_every_declaration_block() {
        let (nb, a, b, c) = schwarzschild_notebook();
        // Executed only `c` (the metric) -- `a` and `b` must still have
        // been swept into the same reconstruction and confirmed.
        assert_eq!(*decl_status(&nb, a), DeclarationStatus::Confirmed);
        assert_eq!(*decl_status(&nb, b), DeclarationStatus::Confirmed);
        assert_eq!(*decl_status(&nb, c), DeclarationStatus::Confirmed);
        assert!(nb.session().document().is_some());
    }

    #[test]
    fn editing_a_declaration_block_marks_it_divergent_immediately_without_executing() {
        let (mut nb, a, ..) = schwarzschild_notebook();
        assert_eq!(*decl_status(&nb, a), DeclarationStatus::Confirmed);
        nb.edit_block(a, format!("{SCHWARZSCHILD_A}\n"));
        assert_eq!(*decl_status(&nb, a), DeclarationStatus::Divergent, "editing must mark divergent right away, before any Shift+Enter");
    }

    #[test]
    fn executing_one_declaration_block_picks_up_an_unexecuted_edit_in_another() {
        // The whole point of "reconstruct as a unit": editing chart's
        // coordinate list (block b) without executing b, then executing
        // metric's block (c), must still incorporate b's edit -- not
        // just c's own text. Renaming "phi" needs the SAME rename on
        // both sides (chart's coordinate list and the metric's `[phi,phi]`
        // bracket reference -- resolved by name, not position) to stay
        // valid, same as `oderom-session`'s own
        // `changing_only_the_chart_changes_only_its_own_fingerprint`
        // test does via one global replace; here it's split across two
        // blocks on purpose, since that split is exactly what's being
        // tested -- only `c` is ever given its own Shift+Enter.
        let (mut nb, _a, b, c) = schwarzschild_notebook();
        nb.edit_block(b, SCHWARZSCHILD_B.replace("phi", "psi"));
        nb.edit_block(c, SCHWARZSCHILD_C.replace("phi", "psi"));
        assert_eq!(*decl_status(&nb, b), DeclarationStatus::Divergent, "b was edited but never itself executed");

        nb.execute_block(c); // never touches b directly
        assert_eq!(*decl_status(&nb, b), DeclarationStatus::Confirmed, "b's unexecuted edit must have been picked up by c's reconstruction");
        let chart = &nb.session().document().unwrap().model.charts["schw"];
        assert!(chart.coords.contains(&"psi".to_string()), "expected the unexecuted edit in block b to have been picked up: {:?}", chart.coords);
    }

    #[test]
    fn deleting_a_declaration_block_only_removes_its_definition_at_the_next_reconstruction() {
        let (mut nb, a, b, c) = schwarzschild_notebook();
        nb.delete_block(b); // the chart -- metric g on schw would no longer resolve
        // Deletion itself must not have re-evaluated anything: the
        // Model is untouched, still has the old (3-block) definitions.
        assert!(nb.session().document().unwrap().model.charts.contains_key("schw"));

        nb.execute_block(a); // now trigger a reconstruction, without block b at all
        // "chart schw" no longer exists among declaration blocks, so
        // "metric g on schw" (block c) can no longer resolve --
        // reconstruction fails, and c is the attributable cause (it's
        // the one referencing the now-missing chart).
        assert!(matches!(decl_status(&nb, c), DeclarationStatus::Error(_)));
    }

    #[test]
    fn a_reconstruction_failure_only_marks_the_culprit_an_untouched_block_stays_confirmed() {
        let (mut nb, a, _b, c) = schwarzschild_notebook();
        // Break `c` (the metric) by editing it into a syntax error, and
        // execute it -- `a` was never touched since the last successful
        // reconstruction, so it must stay Confirmed, not flip to
        // anything else.
        nb.edit_block(c, "metric g on schw bundle TM { this is not valid".to_string());
        nb.execute_block(c);
        assert!(matches!(decl_status(&nb, c), DeclarationStatus::Error(_)));
        assert_eq!(*decl_status(&nb, a), DeclarationStatus::Confirmed, "an untouched, previously-confirmed block must not be disturbed by an unrelated failure");
    }

    #[test]
    fn a_reconstruction_failure_leaves_an_already_divergent_untouched_block_divergent_not_confirmed() {
        // The exact scenario from the session log: edit A and B, run B,
        // B is the culprit -- A must show Divergent (its own edit is
        // genuinely not in the live Model), never silently Confirmed.
        let (mut nb, a, _b, c) = schwarzschild_notebook();
        nb.edit_block(a, format!("{SCHWARZSCHILD_A}\n"));
        assert_eq!(*decl_status(&nb, a), DeclarationStatus::Divergent);

        nb.edit_block(c, "metric g on schw bundle TM { still not valid".to_string());
        nb.execute_block(c);

        assert!(matches!(decl_status(&nb, c), DeclarationStatus::Error(_)));
        assert_eq!(*decl_status(&nb, a), DeclarationStatus::Divergent, "A's own edit is genuinely not reflected in the (unchanged) live Model -- it must not read as Confirmed");
    }

    #[test]
    fn a_never_run_block_swept_into_a_failed_reconstruction_resolves_to_divergent() {
        let mut nb = Notebook::new();
        let a = nb.create_block_after(None, "manifold M dim 4".to_string());
        let broken = nb.create_block_after(Some(a), "bundle TM on nonexistent_manifold dim 4".to_string());
        nb.execute_block(broken);
        assert!(matches!(decl_status(&nb, broken), DeclarationStatus::Error(_)));
        assert_eq!(*decl_status(&nb, a), DeclarationStatus::Divergent, "a, never previously reconstructed, must resolve to Divergent, not stay in some fourth 'never run' state");
    }

    #[test]
    fn a_success_after_a_prior_failure_confirms_every_declaration_block_including_the_previously_erroring_one() {
        let (mut nb, a, _b, c) = schwarzschild_notebook();
        nb.edit_block(c, "metric g on schw bundle TM { not valid".to_string());
        nb.execute_block(c);
        assert!(matches!(decl_status(&nb, c), DeclarationStatus::Error(_)));

        // Fix it and execute again -- every declaration block, not just
        // c, must come back to Confirmed: the Model is one thing, the
        // display must agree with it as a whole.
        nb.edit_block(c, SCHWARZSCHILD_C.to_string());
        nb.execute_block(c);
        assert_eq!(*decl_status(&nb, a), DeclarationStatus::Confirmed);
        assert_eq!(*decl_status(&nb, c), DeclarationStatus::Confirmed);
    }

    // ---------------------------------------------------------------
    // Error attribution
    // ---------------------------------------------------------------

    #[test]
    fn a_parse_error_is_attributed_to_the_block_whose_line_it_falls_in() {
        let mut nb = Notebook::new();
        let a = nb.create_block_after(None, "manifold M dim 4".to_string());
        nb.execute_block(a); // `a` alone succeeds first, so it's genuinely Confirmed before `broken` ever exists
        assert_eq!(*decl_status(&nb, a), DeclarationStatus::Confirmed);

        let broken = nb.create_block_after(Some(a), "chart schw on M coords (t, r".to_string()); // unterminated
        nb.execute_block(broken);
        assert!(matches!(decl_status(&nb, broken), DeclarationStatus::Error(_)));
        assert_eq!(*decl_status(&nb, a), DeclarationStatus::Confirmed, "the well-formed, already-confirmed, untouched block must not be disturbed by a later block's own parse error");
    }

    #[test]
    fn a_duplicate_name_error_is_attributed_to_the_second_block_declaring_that_name_not_whichever_was_executed() {
        let mut nb = Notebook::new();
        let first = nb.create_block_after(None, "manifold M dim 4".to_string());
        let second = nb.create_block_after(Some(first), "manifold M dim 4".to_string()); // same name, again
        nb.execute_block(first); // executing the FIRST (correct) block still fails, because the whole unit fails
        assert!(matches!(decl_status(&nb, second), DeclarationStatus::Error(_)), "expected the SECOND declarer to be blamed, not the block that was executed");
        assert_eq!(*decl_status(&nb, first), DeclarationStatus::Divergent);
    }

    #[test]
    fn using_an_alias_declared_later_in_the_notebook_attributes_the_error_to_the_earlier_using_block_with_a_message_naming_the_cause() {
        // Action at a distance: declaring `f := ...` in a LATER block can
        // retroactively break an EARLIER block that used `f` as an
        // ordinary free variable -- confirmed correct behavior (a stray
        // free `f` colliding with an alias declared anywhere else in the
        // notebook is almost always a mistake, so failing loudly is
        // right), but the user staring at the now-broken earlier block
        // needs the message itself to say why, since nothing about the
        // earlier block's own text changed.
        let mut nb = Notebook::new();
        let decls = nb.create_block_after(None, "manifold M dim 2\nbundle TM on M dim 2\nchart c on M coords (x, y)".to_string());
        let uses_f_too_early = nb.create_block_after(Some(decls), "metric g on c bundle TM {\n  [x,x] = f,\n  [y,y] = 1\n}".to_string());
        nb.execute_block(uses_f_too_early);
        assert_eq!(*decl_status(&nb, uses_f_too_early), DeclarationStatus::Confirmed, "f is not declared as an alias anywhere yet, so it must still resolve as an ordinary free variable");

        let declares_f_late = nb.create_block_after(Some(uses_f_too_early), "f := 1 - x".to_string());
        nb.execute_block(declares_f_late);

        let DeclarationStatus::Error(message) = decl_status(&nb, uses_f_too_early) else {
            panic!("expected the earlier, now-broken block to carry the error, got {:?}", decl_status(&nb, uses_f_too_early));
        };
        assert!(
            message.contains("alias `f`") && message.contains("before its own") && message.contains("f := ..."),
            "message must name the cause (a later `f := ...` alias declaration exists) rather than just saying an opaque 'error' -- got: {message}"
        );
    }

    // ---------------------------------------------------------------
    // Obsolescence still works through the notebook (mechanism itself
    // is oderom-session's, this only proves the glue doesn't break it)
    // ---------------------------------------------------------------

    #[test]
    fn editing_and_reexecuting_a_declaration_marks_a_dependent_query_block_stale_then_recompute_clears_it() {
        let (mut nb, _a, _b, c) = schwarzschild_notebook();
        let q = nb.create_block_after(None, "ricci".to_string());
        nb.execute_block(q);
        let BlockOutput::Query(qid) = nb.block(q).unwrap().output else { panic!("expected Query output") };
        assert!(!nb.session().entries().iter().find(|e| e.id == qid).unwrap().state.is_stale());

        let edited = SCHWARZSCHILD_C.replace("2*M/r", "3*M/r");
        nb.edit_block(c, edited);
        nb.execute_block(c);
        assert!(nb.session().entries().iter().find(|e| e.id == qid).unwrap().state.is_stale(), "expected the query block to go stale after its metric was redefined");

        nb.execute_block(q); // "recompute" -- reexecuting IS how the notebook recomputes a query block
        // Reexecuting replaces the entry (a new `EntryId`, the old one
        // removed -- see `execute_query_block`), so the block's output
        // must be reread, not the `qid` captured before this call.
        let BlockOutput::Query(new_qid) = nb.block(q).unwrap().output else { panic!("expected Query output") };
        assert_ne!(new_qid, qid, "reexecuting must not reuse the old EntryId");
        assert!(!nb.session().entries().iter().any(|e| e.id == qid), "the old entry must have been removed, not left as an orphan");
        let entry = nb.session().entries().iter().find(|e| e.id == new_qid).unwrap();
        assert!(!entry.state.is_stale());
        assert!(matches!(entry.state, EntryState::Done { .. }));
    }

    // ---------------------------------------------------------------
    // Obsolescência visível (Etapa 3b) -- purely informational: no test
    // in this section ever asserts anything computed a NEW result;
    // every one of them asserts the opposite (session entry counts,
    // declaration statuses, and source text all stay exactly as they
    // were across an edit/delete/insert).
    // ---------------------------------------------------------------

    /// Four blocks, each *directly* Shift-Entered in notebook order --
    /// the real interaction the acceptance criteria describe ("executar
    /// os quatro blocos"), not `schwarzschild_notebook`'s single
    /// metric-block execution that sweeps the other two in as a side
    /// effect. All four end up with their own `execution_count`, same
    /// as pressing Shift+Enter four times in the real UI does
    /// (`oderom-app/src-tauri/tests/keymap.rs`'s own Phase 1).
    fn four_directly_executed_blocks_notebook() -> (Notebook, BlockId, BlockId, BlockId, BlockId) {
        let mut nb = Notebook::new();
        let a = nb.create_block_after(None, SCHWARZSCHILD_A.to_string());
        let b = nb.create_block_after(Some(a), SCHWARZSCHILD_B.to_string());
        let c = nb.create_block_after(Some(b), SCHWARZSCHILD_C.to_string());
        let q = nb.create_block_after(Some(c), "ricci".to_string());
        nb.execute_block(a);
        nb.execute_block(b);
        nb.execute_block(c);
        nb.execute_block(q);
        (nb, a, b, c, q)
    }

    #[test]
    fn editing_a_block_marks_it_and_every_executed_block_below_it_obsolete_but_runs_nothing() {
        let (mut nb, a, b, c, q) = four_directly_executed_blocks_notebook();
        let entries_before = nb.session().entries().len();
        assert!(!nb.block(b).unwrap().is_obsolete());

        nb.edit_block(b, SCHWARZSCHILD_B.replace("phi", "psi"));

        assert!(nb.block(b).unwrap().is_obsolete(), "b's own text no longer matches what was executed");
        assert!(nb.block(c).unwrap().is_obsolete(), "c is below b and was already executed");
        assert!(nb.block(q).unwrap().is_obsolete(), "q is below b and was already executed");
        assert!(!nb.block(a).unwrap().is_obsolete(), "a is above b -- must be untouched");
        assert_eq!(entries_before, nb.session().entries().len(), "marking obsolete must never run or remove anything");
        assert_eq!(*decl_status(&nb, c), DeclarationStatus::Confirmed, "c's declaration status is a separate signal from obsolete -- nothing has reconstructed yet, so it hasn't changed");
    }

    #[test]
    fn editing_an_alias_block_marks_a_later_block_that_uses_it_obsolete() {
        // Expression aliases (`NOME := EXPR`, oderom-cli's own parser)
        // are pure syntactic sugar with no dedicated notebook-level
        // dependency tracking of their own -- the whole point (see
        // `classify_block`'s doc comment) is that they ride the SAME
        // conservative, purely positional "mark everything below an
        // edited block obsolete" rule every other declaration already
        // uses (`mark_cascade_obsolete_from`, called from `edit_block`
        // unconditionally, content-blind). This test exists purely to
        // CONFIRM that already-existing rule actually covers this case
        // end to end, not to add anything new for it to cover.
        let mut nb = Notebook::new();
        let alias_block = nb.create_block_after(None, "f := 1 - 2*M/r".to_string());
        let decls = nb.create_block_after(
            Some(alias_block),
            "manifold M dim 2\nbundle TM on M dim 2\nchart c on M coords (r, theta)".to_string(),
        );
        let metric_block = nb.create_block_after(
            Some(decls),
            "metric g on c bundle TM {\n  [r,r] = f,\n  [theta,theta] = 1\n}".to_string(),
        );
        // `alias_block` itself directly executed too, not just swept
        // into `metric_block`'s reconstruction -- `is_obsolete()` only
        // means anything for a block with its own `execution_count`
        // (`Block::is_obsolete`'s own doc comment: never-directly-run
        // is a separate, permanent "false", by design), same as every
        // other obsolescence test in this file executes each block it
        // asserts on directly.
        nb.execute_block(alias_block);
        nb.execute_block(metric_block); // reconstructs alias_block+decls+metric_block as one unit
        assert!(!nb.block(metric_block).unwrap().is_obsolete());

        nb.edit_block(alias_block, "f := 1 - 3*M/r".to_string());

        assert!(nb.block(alias_block).unwrap().is_obsolete(), "alias_block's own text no longer matches what was executed");
        assert!(nb.block(metric_block).unwrap().is_obsolete(), "metric_block is below alias_block and used it -- must be marked obsolete by the existing conservative rule alone");
    }

    #[test]
    fn reexecuting_clears_only_that_blocks_own_obsolete_mark() {
        let (mut nb, _a, b, c, q) = four_directly_executed_blocks_notebook();
        nb.edit_block(b, SCHWARZSCHILD_B.replace("phi", "psi"));
        nb.edit_block(c, SCHWARZSCHILD_C.replace("phi", "psi"));

        nb.execute_block(b); // reconstructs a+b+c as a unit, including c's still-unexecuted edit

        assert!(!nb.block(b).unwrap().is_obsolete(), "b was just directly executed");
        assert!(nb.block(c).unwrap().is_obsolete(), "c was only swept into b's reconstruction, never itself reexecuted -- stays marked ('só dele')");
        assert!(nb.block(q).unwrap().is_obsolete(), "q hasn't been reexecuted at all");
    }

    #[test]
    fn undoing_an_edit_back_to_the_executed_text_clears_the_mark_not_a_sticky_flag() {
        let (mut nb, .., c, _q) = four_directly_executed_blocks_notebook();
        nb.edit_block(c, format!("{SCHWARZSCHILD_C}\n"));
        assert!(nb.block(c).unwrap().is_obsolete());

        nb.edit_block(c, SCHWARZSCHILD_C.to_string()); // back to exactly the text that was executed
        assert!(!nb.block(c).unwrap().is_obsolete(), "text is byte-identical to what was executed -- a live comparison, not a flag that only ever turns on");
    }

    #[test]
    fn deleting_a_block_marks_only_the_executed_blocks_below_it_obsolete() {
        let (mut nb, a, b, c, q) = four_directly_executed_blocks_notebook();
        nb.delete_block(b);
        assert!(!nb.block(a).unwrap().is_obsolete(), "a is above the deleted block");
        assert!(nb.block(c).unwrap().is_obsolete());
        assert!(nb.block(q).unwrap().is_obsolete());
    }

    #[test]
    fn a_never_directly_executed_block_can_never_be_obsolete() {
        let mut nb = Notebook::new();
        let id = nb.create_block_after(None, "manifold M dim 4".to_string());
        nb.edit_block(id, "manifold M dim 5".to_string());
        assert!(!nb.block(id).unwrap().is_obsolete());
        assert_eq!(nb.block(id).unwrap().execution_count, None);
    }

    // ---------------------------------------------------------------
    // Cancellation (Etapa 3b, DESIGN-NOTEBOOK.md). `PendingQuery::run()`
    // runs on a real `std::thread::spawn` worker in every test that
    // actually cancels something -- the exact cross-thread handoff
    // `oderom-app` uses, not just the data structures exercised inline.
    // ---------------------------------------------------------------

    use oderom_session::EntryId;

    /// `g_tt`/`g_rr` not reciprocal -- this project's own standing
    /// non-terminating-computation fixture
    /// (`oderom-session/tests/cancellation.rs`), found from the real
    /// freeze this whole feature exists to fix (see the session log).
    /// Reused verbatim rather than inventing a second one.
    const NON_RECIPROCAL: &str = "
manifold M dim 4
bundle TM on M dim 4
chart schw on M coords (t, r, theta, phi)
metric g on schw bundle TM {
  [t,t] = -(1 - 2*M/r + 1/r^2),
  [r,r] = 1/(1 - 2*M/r + Q^2/r^2),
  [theta,theta] = r^2,
  [phi,phi] = r^2 * sin(theta)^2
}
";

    /// The reciprocal counterpart -- finishes in well under a second,
    /// used wherever a test needs a *real*, successfully computed
    /// result to exist before things go slow.
    const RECIPROCAL_METRIC: &str = "
manifold M dim 4
bundle TM on M dim 4
chart schw on M coords (t, r, theta, phi)
metric g on schw bundle TM {
  [t,t] = -(1 - 2*M/r + Q^2/r^2),
  [r,r] = 1/(1 - 2*M/r + Q^2/r^2),
  [theta,theta] = r^2,
  [phi,phi] = r^2 * sin(theta)^2
}
";

    /// Starts `id`, runs it on a real worker thread, cancels it after a
    /// short real sleep (same 300ms this project's own
    /// `oderom-session/tests/cancellation.rs` uses -- long enough to be
    /// well past the between-component checkpoint and genuinely stuck
    /// inside a single component's `normalize()` call), and feeds the
    /// result back through `finish_query`. Panics if `id` wasn't
    /// actually a query with something to run (a test setup bug, not a
    /// thing worth a softer failure mode here).
    fn cancel_in_flight(nb: &mut Notebook, id: BlockId) {
        let BeginExecution::Started(pending) = nb.begin_execute(id) else { panic!("expected BeginExecution::Started") };
        let ctx = pending.context();
        let worker = std::thread::spawn(move || pending.run());
        std::thread::sleep(std::time::Duration::from_millis(300));
        ctx.cancel();
        let result = worker.join().expect("PendingQuery::run must catch its own cancellation unwind internally, never let it escape the worker thread");
        nb.finish_query(id, result);
    }

    #[test]
    fn begin_execute_on_a_query_block_shows_running_immediately_and_settles_on_finish() {
        let mut nb = Notebook::new();
        let a = nb.create_block_after(None, RECIPROCAL_METRIC.to_string());
        nb.execute_block(a);
        let q = nb.create_block_after(Some(a), "kretschmann".to_string());

        let BeginExecution::Started(pending) = nb.begin_execute(q) else { panic!("expected Started") };
        let BlockOutput::Attempt { attempt, previous } = nb.block(q).unwrap().output else { panic!("expected Attempt immediately, before .run() was even called") };
        assert_eq!(previous, None, "first-ever execution -- nothing to fall back to");
        let entry = nb.session().entries().iter().find(|e| e.id == attempt).unwrap();
        assert!(matches!(entry.state, EntryState::Running));

        let result = pending.run();
        nb.finish_query(q, result);
        assert!(matches!(nb.block(q).unwrap().output, BlockOutput::Query(_)), "must have settled back to plain Query once it finished normally");
    }

    #[test]
    fn cancelling_a_first_ever_execution_leaves_the_block_cancelled_with_no_previous_result() {
        let mut nb = Notebook::new();
        let a = nb.create_block_after(None, NON_RECIPROCAL.to_string());
        nb.execute_block(a); // fast -- declaration reconstruction only, no curvature involved
        let q = nb.create_block_after(Some(a), "kretschmann".to_string());

        cancel_in_flight(&mut nb, q);

        let BlockOutput::Attempt { attempt, previous } = nb.block(q).unwrap().output else {
            panic!("expected the block to stay in Attempt form after a cancellation, not revert to NeverRun or anything else")
        };
        assert_eq!(previous, None, "this was the block's first-ever execution -- nothing to fall back to, so no output beyond the cancelled state itself");
        let entry = nb.session().entries().iter().find(|e| e.id == attempt).unwrap();
        assert!(matches!(entry.state, EntryState::Cancelled));
    }

    /// Setup shared by the next two tests: a query block with a real,
    /// successful result, then edited-and-reexecuted against a metric
    /// that hangs, then cancelled. Building it once and using it twice
    /// (mirroring `four_directly_executed_blocks_notebook`'s own
    /// pattern earlier in this file) keeps the two assertions about it
    /// (what cancelling alone leaves behind; what a further reexecution
    /// then does) as separate, focused tests without duplicating a
    /// three-thread-hop setup.
    fn notebook_with_a_cancelled_reexecution() -> (Notebook, BlockId, BlockId, EntryId) {
        let mut nb = Notebook::new();
        let a = nb.create_block_after(None, RECIPROCAL_METRIC.to_string());
        nb.execute_block(a);
        let q = nb.create_block_after(Some(a), "kretschmann".to_string());
        nb.execute_block(q); // fast, real result
        let BlockOutput::Query(first_id) = nb.block(q).unwrap().output else { panic!("expected Query") };
        assert!(nb.session().entries().iter().find(|e| e.id == first_id).unwrap().state.has_result());

        nb.edit_block(a, NON_RECIPROCAL.to_string());
        nb.execute_block(a); // fast -- reconstruction only
        cancel_in_flight(&mut nb, q); // THIS reexecution hangs, then gets cancelled

        (nb, a, q, first_id)
    }

    #[test]
    fn cancelling_a_reexecution_keeps_the_previous_result_visible_and_marks_it_obsolete() {
        let (nb, _a, q, first_id) = notebook_with_a_cancelled_reexecution();

        let BlockOutput::Attempt { attempt, previous } = nb.block(q).unwrap().output else { panic!("expected Attempt") };
        assert_eq!(previous, Some(first_id), "the earlier Done entry must still be referenced, not dropped");
        assert!(matches!(nb.session().entries().iter().find(|e| e.id == attempt).unwrap().state, EntryState::Cancelled));

        let prev_entry = nb.session().entries().iter().find(|e| e.id == first_id).unwrap();
        assert!(prev_entry.state.has_result(), "the earlier result must survive a cancelled reexecution -- 'não apague saída por causa de cancelamento'");

        // `Block::is_obsolete()` (the position/self-edit signal from
        // the earlier obsolescence round) is deliberately NOT asserted
        // here: whether `previous`'s result reads as obsolete on screen
        // is unconditional on `previous` being `Some` at all (any
        // `previous` is, by construction, superseded by a newer
        // attempt) -- it does not depend on that separate flag, which
        // `begin_execute` resets for the block being directly targeted
        // regardless of whether this specific attempt goes on to
        // succeed, fail, or get cancelled. The frontend gives
        // Attempt-state blocks their own complete visual treatment
        // (`oderom-app/dist/notebook.js`) rather than layering the
        // generic obsolete marker on top of it.
    }

    #[test]
    fn reexecuting_a_cancelled_block_completes_normally_and_clears_the_cancelled_state() {
        let (mut nb, a, q, first_id) = notebook_with_a_cancelled_reexecution();

        nb.edit_block(a, RECIPROCAL_METRIC.to_string()); // revert -- fast again
        nb.execute_block(a);
        nb.execute_block(q); // fast now -- completes normally, no cancellation this time

        assert!(matches!(nb.block(q).unwrap().output, BlockOutput::Query(_)), "must have settled back to plain Query, no longer Attempt/Cancelled");
        assert!(!nb.session().entries().iter().any(|e| e.id == first_id), "the old previous entry must have been removed once a normal result replaced it, same as any other reexecution");
        assert!(!nb.block(q).unwrap().is_obsolete(), "a fresh, successful, directly-targeted reexecution must clear its own obsolete mark");
    }

    #[test]
    fn cancelling_a_block_does_not_trigger_execution_of_any_other_block() {
        let mut nb = Notebook::new();
        let a = nb.create_block_after(None, NON_RECIPROCAL.to_string());
        nb.execute_block(a);
        let other = nb.create_block_after(Some(a), "scalar".to_string()); // deliberately never executed
        let q = nb.create_block_after(Some(other), "kretschmann".to_string());

        let BeginExecution::Started(pending) = nb.begin_execute(q) else { panic!("expected Started") };
        let worker = std::thread::spawn(move || pending.run());
        std::thread::sleep(std::time::Duration::from_millis(300));
        nb.cancel_block(q); // the same method a Tauri `cancel_block` command calls -- not `ExecutionContext::cancel` directly
        let result = worker.join().unwrap();
        nb.finish_query(q, result);

        assert_eq!(nb.block(other).unwrap().execution_count, None, "cancelling q must not have executed `other`");
        assert!(matches!(nb.block(other).unwrap().output, BlockOutput::NeverRun));
    }

    #[test]
    fn begin_execute_is_blocked_while_the_same_block_is_already_running() {
        let mut nb = Notebook::new();
        let a = nb.create_block_after(None, NON_RECIPROCAL.to_string());
        nb.execute_block(a);
        let q = nb.create_block_after(Some(a), "kretschmann".to_string());

        let BeginExecution::Started(pending) = nb.begin_execute(q) else { panic!("expected Started") };
        let count_after_first_start = nb.block(q).unwrap().execution_count;

        let second = nb.begin_execute(q);
        assert!(matches!(second, BeginExecution::Blocked { by } if by == q), "a second begin_execute while already running must be Blocked, not a competing second attempt");
        assert_eq!(nb.block(q).unwrap().execution_count, count_after_first_start, "must not have bumped the execution counter for the refused second attempt");

        // Clean up the original attempt so its thread/context don't
        // outlive the test: cancelling before ever spawning a thread at
        // all still aborts promptly (same as `a_pre_cancelled_context_aborts_instead_of_computing`
        // in oderom-session -- the very first checkpoint already sees it).
        let ctx = pending.context();
        ctx.cancel();
        let result = pending.run();
        nb.finish_query(q, result);
    }

    // ---------------------------------------------------------------
    // Mutual exclusion across DIFFERENT blocks (Etapa 3b, segunda
    // parte, DESIGN-NOTEBOOK.md section 10.8) -- a deliberate decision,
    // not an accidental consequence of the cancellable split: at most
    // one execution runs at a time across the WHOLE notebook, so a
    // declaration and a query that depends on it can never race against
    // each other on thread scheduling.
    // ---------------------------------------------------------------

    #[test]
    fn a_different_block_is_blocked_while_one_execution_is_in_flight_and_is_left_completely_untouched() {
        let mut nb = Notebook::new();
        let a = nb.create_block_after(None, NON_RECIPROCAL.to_string());
        nb.execute_block(a);
        let q1 = nb.create_block_after(Some(a), "kretschmann".to_string());
        let q2 = nb.create_block_after(Some(q1), "riemann".to_string());

        let BeginExecution::Started(pending1) = nb.begin_execute(q1) else { panic!("expected Started") };

        let refused = nb.begin_execute(q2);
        assert!(matches!(refused, BeginExecution::Blocked { by } if by == q1), "expected q2 to be blocked by q1, the one actually running");
        assert_eq!(nb.block(q2).unwrap().execution_count, None, "a refused block must never gain an execution_count -- no attempt happened");
        assert!(matches!(nb.block(q2).unwrap().output, BlockOutput::NeverRun), "a refused block's output must be completely untouched");

        let ctx1 = pending1.context();
        ctx1.cancel();
        let result1 = pending1.run();
        nb.finish_query(q1, result1);
    }

    #[test]
    fn a_declaration_block_is_also_blocked_while_a_query_is_running_elsewhere() {
        // The exact scenario the decision is about: a declaration whose
        // reconstruction could otherwise race a query reading the model
        // concurrently.
        let mut nb = Notebook::new();
        let a = nb.create_block_after(None, NON_RECIPROCAL.to_string());
        nb.execute_block(a);
        let q = nb.create_block_after(Some(a), "kretschmann".to_string());
        let other_decl = nb.create_block_after(None, "manifold Other dim 2".to_string());

        let BeginExecution::Started(pending) = nb.begin_execute(q) else { panic!("expected Started") };

        let refused = nb.begin_execute(other_decl);
        assert!(matches!(refused, BeginExecution::Blocked { by } if by == q));
        assert_eq!(nb.block(other_decl).unwrap().execution_count, None);
        assert!(matches!(nb.block(other_decl).unwrap().output, BlockOutput::NeverRun));

        let ctx = pending.context();
        ctx.cancel();
        let result = pending.run();
        nb.finish_query(q, result);
    }

    #[test]
    fn cancelling_the_running_block_unblocks_a_previously_refused_block() {
        let mut nb = Notebook::new();
        let a = nb.create_block_after(None, NON_RECIPROCAL.to_string());
        nb.execute_block(a);
        let q1 = nb.create_block_after(Some(a), "kretschmann".to_string());
        let q2 = nb.create_block_after(Some(q1), "riemann".to_string());

        let BeginExecution::Started(pending1) = nb.begin_execute(q1) else { panic!("expected Started") };
        assert!(matches!(nb.begin_execute(q2), BeginExecution::Blocked { .. }));

        let ctx1 = pending1.context();
        ctx1.cancel();
        let result1 = pending1.run();
        nb.finish_query(q1, result1);

        // q1 settled (cancelled) -- q2 must now be free to start.
        assert!(matches!(nb.begin_execute(q2), BeginExecution::Started(_)), "expected q2 to be unblocked once q1 (the thing blocking it) was cancelled and settled");
    }

    #[test]
    fn the_running_block_finishing_naturally_also_unblocks_a_previously_refused_block() {
        // Same as the cancellation case above, but q1 completes on its
        // own (a fast, reciprocal metric) instead of being cancelled --
        // "terminando o primeiro naturalmente" must unblock just as
        // reliably as cancelling it does.
        const RECIPROCAL_METRIC: &str = "
manifold M dim 4
bundle TM on M dim 4
chart schw on M coords (t, r, theta, phi)
metric g on schw bundle TM {
  [t,t] = -(1 - 2*M/r + Q^2/r^2),
  [r,r] = 1/(1 - 2*M/r + Q^2/r^2),
  [theta,theta] = r^2,
  [phi,phi] = r^2 * sin(theta)^2
}
";
        let mut nb = Notebook::new();
        let a = nb.create_block_after(None, RECIPROCAL_METRIC.to_string());
        nb.execute_block(a);
        let q1 = nb.create_block_after(Some(a), "kretschmann".to_string());
        let q2 = nb.create_block_after(Some(q1), "riemann".to_string());

        let BeginExecution::Started(pending1) = nb.begin_execute(q1) else { panic!("expected Started") };
        assert!(matches!(nb.begin_execute(q2), BeginExecution::Blocked { .. }));

        let result1 = pending1.run(); // fast metric -- finishes on its own, never cancelled
        nb.finish_query(q1, result1);
        assert!(matches!(nb.block(q1).unwrap().output, BlockOutput::Query(_)), "q1 should have settled normally, not stayed Attempt/Cancelled");

        let BeginExecution::Started(pending2) = nb.begin_execute(q2) else { panic!("expected q2 to be unblocked once q1 finished on its own") };
        // Drain q2 so nothing is left running when the test ends.
        let result2 = pending2.run();
        nb.finish_query(q2, result2);
    }

    #[test]
    fn editing_and_focus_related_operations_are_never_blocked_while_something_runs() {
        // "bloquear execução não pode bloquear a interface" -- edit_block
        // (the Rust side of typing/focus never touches `running` at
        // all) must keep working normally on the running block, on a
        // refused block, and on completely unrelated blocks, the whole
        // time something is executing.
        let mut nb = Notebook::new();
        let a = nb.create_block_after(None, NON_RECIPROCAL.to_string());
        nb.execute_block(a);
        let q1 = nb.create_block_after(Some(a), "kretschmann".to_string());
        let other = nb.create_block_after(Some(q1), "manifold Other dim 2".to_string());

        let BeginExecution::Started(pending1) = nb.begin_execute(q1) else { panic!("expected Started") };

        // Editing the block that is not running.
        nb.edit_block(other, "manifold Other dim 3".to_string());
        assert_eq!(nb.block(other).unwrap().source, "manifold Other dim 3");

        // Editing the running block itself -- allowed (per the existing
        // obsolescence design, its `output` just stays untouched since
        // `execution_count.is_some()`), does not disturb the in-flight
        // computation.
        nb.edit_block(q1, "kretschmann".to_string());
        assert_eq!(nb.block(q1).unwrap().source, "kretschmann");

        let ctx1 = pending1.context();
        ctx1.cancel();
        let result1 = pending1.run();
        nb.finish_query(q1, result1);
    }

    #[test]
    fn deleting_a_block_with_an_in_flight_execution_cancels_it_and_removes_its_entries() {
        let mut nb = Notebook::new();
        let a = nb.create_block_after(None, NON_RECIPROCAL.to_string());
        nb.execute_block(a);
        let q = nb.create_block_after(Some(a), "kretschmann".to_string());

        let BeginExecution::Started(pending) = nb.begin_execute(q) else { panic!("expected Started") };
        let ctx = pending.context();

        nb.delete_block(q);

        assert!(ctx.is_cancelled(), "deleting a block with an execution in flight must cancel it, not let it run to completion unread");
        assert!(nb.block(q).is_none());
        let _ = pending.run(); // let the (now cancelled) computation actually return, so it doesn't outlive the test
    }

    #[test]
    fn cancel_block_on_a_block_that_isnt_running_is_a_no_op() {
        let mut nb = Notebook::new();
        let a = nb.create_block_after(None, "manifold M dim 4".to_string());
        nb.execute_block(a);
        nb.cancel_block(a); // never a query, never running -- must not panic or disturb anything
        assert!(matches!(nb.block(a).unwrap().output, BlockOutput::Declaration(DeclarationStatus::Confirmed)));
    }

    // ---------------------------------------------------------------
    // "Limpar execução" (Botão 1): every block back to freshly parsed,
    // every scrap of session state discarded, block TEXT untouched.
    // ---------------------------------------------------------------

    /// The strongest version of the claim: an alias declared and used
    /// (so it genuinely affected a real Model, not just displayed
    /// something) stops resolving after `clear_execution` -- proving the
    /// compiled session state is gone, not just the on-screen `[ ]`.
    /// Re-running the SAME (still-intact) declaration text afterward
    /// must bring the alias back, since text was never touched -- the
    /// two halves of the claim checked in one test so neither can be
    /// satisfied by accident (e.g. a version that wipes text too would
    /// still pass the first half).
    #[test]
    fn an_alias_used_before_clearing_no_longer_resolves_until_its_own_declaration_block_runs_again() {
        let mut nb = Notebook::new();
        let alias = nb.create_block_after(None, "f := 1 - 2*M/r".to_string());
        let decls = nb.create_block_after(
            Some(alias),
            "manifold M dim 4\nbundle TM on M dim 4\nchart schw on M coords (t, r, theta, phi)".to_string(),
        );
        let metric = nb.create_block_after(
            Some(decls),
            "metric g on schw bundle TM {\n  [t,t] = -f,\n  [r,r] = 1/f,\n  [theta,theta] = r^2,\n  [phi,phi] = r^2\n}".to_string(),
        );
        let query = nb.create_block_after(Some(metric), "ricci".to_string());

        nb.execute_block(metric); // reconstructs alias+decls+metric as one unit -- f resolves
        assert!(matches!(nb.block(metric).unwrap().output, BlockOutput::Declaration(DeclarationStatus::Confirmed)), "setup: the alias must resolve before clearing, or this test proves nothing");
        nb.execute_block(query);
        assert!(
            matches!(&nb.block(query).unwrap().output, BlockOutput::Query(id) if nb.session().entries().iter().find(|e| e.id == *id).unwrap().state.has_result()),
            "setup: the query must have a real result before clearing"
        );

        nb.clear_execution();

        assert!(nb.session().document().is_none(), "clear_execution must discard the compiled Model (and with it, the alias's expanded value) entirely, not just hide it");

        // Re-fetch by position: clear_execution rebuilds every block with
        // a NEW BlockId (see its own doc comment on next_block_id) --
        // the pre-clear ids above are stale now.
        assert_eq!(nb.blocks().len(), 4);
        let new_query_id = nb.blocks()[3].id;
        let new_metric_id = nb.blocks()[2].id;
        assert_eq!(nb.blocks()[3].source, "ricci", "block text must survive untouched");

        // Attempting to run the query block alone, WITHOUT first
        // re-running any declaration block, must fail -- nothing is
        // declared anymore, so `f`/the metric it depended on genuinely
        // do not exist, not merely display as if they didn't.
        nb.execute_block(new_query_id);
        let BlockOutput::Query(entry_id) = nb.block(new_query_id).unwrap().output else { panic!("expected Query output") };
        let entry = nb.session().entries().iter().find(|e| e.id == entry_id).unwrap();
        assert!(!entry.state.has_result(), "the alias/metric must not resolve anymore -- session state was supposed to be gone");

        // The other half: the text is genuinely intact, so re-running
        // the SAME declaration text brings the alias back exactly as
        // before -- this is "text preserved", not "state secretly kept".
        nb.execute_block(new_metric_id);
        assert!(matches!(nb.block(new_metric_id).unwrap().output, BlockOutput::Declaration(DeclarationStatus::Confirmed)), "re-running the intact declaration text must resolve the alias again");
    }

    /// Every field `Block::is_obsolete`/the gutter/the output area reads
    /// goes back to a freshly-created block's own values -- not just
    /// "looks empty" for one field while another lags behind.
    #[test]
    fn clear_execution_resets_every_blocks_execution_state_but_never_its_source() {
        let (mut nb, a, b, c, q) = four_directly_executed_blocks_notebook();
        nb.edit_block(b, SCHWARZSCHILD_B.replace("phi", "psi")); // give b/c/q a real obsolete mark to prove it clears too
        assert!(nb.block(c).unwrap().is_obsolete());

        let sources_before: Vec<String> = [a, b, c, q].iter().map(|id| nb.block(*id).unwrap().source.clone()).collect();

        nb.clear_execution();

        let blocks_after = nb.blocks();
        assert_eq!(blocks_after.len(), 4);
        let sources_after: Vec<&str> = blocks_after.iter().map(|b| b.source.as_str()).collect();
        // b's edit (still un-executed at the moment of clearing) is part
        // of its own source text, so it must survive too -- clearing
        // never reverts an edit.
        let edited_b = SCHWARZSCHILD_B.replace("phi", "psi");
        let expected: Vec<&str> = vec![SCHWARZSCHILD_A, &edited_b, SCHWARZSCHILD_C, "ricci"];
        assert_eq!(sources_after, expected, "block text must be preserved exactly, edits included");
        assert_eq!(sources_after, sources_before, "must match whatever was on screen right before clearing, byte for byte");

        for block in blocks_after {
            assert_eq!(block.execution_count, None, "execution_count must reset to None (gutter shows [ ])");
            assert!(!block.is_obsolete(), "no block can be obsolete when none has ever been executed");
            assert!(matches!(block.output, BlockOutput::NeverRun), "output must reset to NeverRun -- no stale result displayed");
        }
    }

    /// The gutter numbering itself: re-executing after a clear starts
    /// again at `[1]`, never continuing whatever number was already in
    /// flight before the clear.
    #[test]
    fn clear_execution_resets_the_execution_counter_so_the_next_run_is_1() {
        let (mut nb, a, b, ..) = four_directly_executed_blocks_notebook();
        assert!(nb.block(a).unwrap().execution_count.unwrap() >= 1);
        assert!(nb.block(b).unwrap().execution_count.unwrap() > nb.block(a).unwrap().execution_count.unwrap());

        nb.clear_execution();

        let first_id = nb.blocks()[0].id;
        nb.execute_block(first_id);
        assert_eq!(nb.block(first_id).unwrap().execution_count, Some(1), "the first execution after clearing must be [1], not a continuation of the old numbering");
    }

    /// `current_path` is deliberately the one piece of state
    /// `clear_execution` does NOT touch -- this button never reads or
    /// writes a file, so whatever file (if any) the notebook came from
    /// stays remembered.
    #[test]
    fn clear_execution_never_touches_current_path() {
        let (mut nb, ..) = four_directly_executed_blocks_notebook();
        assert_eq!(nb.current_path(), None);
        nb.clear_execution();
        assert_eq!(nb.current_path(), None, "clear_execution must not invent a path");

        let path = std::path::PathBuf::from("/tmp/whatever-this-test-never-actually-writes.odn");
        nb.set_current_path(Some(path.clone()));
        nb.clear_execution();
        assert_eq!(nb.current_path(), Some(path.as_path()), "clear_execution must preserve an existing current_path exactly");
    }

    /// The correctness bar stated in `clear_execution`'s own doc
    /// comment, checked directly rather than assumed: the state right
    /// after clearing is compared against the INDEPENDENTLY implemented
    /// save-then-load round trip (`persist.rs`), which this project
    /// already trusts to build genuinely fresh block state from text
    /// alone (`save_then_load_round_trips_source_and_starts_every_block_fresh`).
    /// If the two ever disagree, `clear_execution` is not actually
    /// equivalent to "reopen this text as a new file", whatever it looks
    /// like on screen.
    #[test]
    fn clear_execution_produces_state_identical_to_a_save_then_load_round_trip() {
        let (mut original, ..) = four_directly_executed_blocks_notebook();
        let sources: Vec<String> = original.blocks().iter().map(|b| b.source.clone()).collect();

        let path = std::env::temp_dir().join(format!(
            "oderom-notebook-test-clear-execution-equivalence-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        crate::persist::save(&mut original, &path).unwrap();
        let loaded = crate::persist::load(&path).unwrap();
        std::fs::remove_file(&path).ok();

        original.clear_execution();

        let cleared_sources: Vec<&str> = original.blocks().iter().map(|b| b.source.as_str()).collect();
        let loaded_sources: Vec<&str> = loaded.blocks().iter().map(|b| b.source.as_str()).collect();
        assert_eq!(cleared_sources, loaded_sources);
        assert_eq!(cleared_sources, sources.iter().map(String::as_str).collect::<Vec<_>>());

        for (cleared, loaded) in original.blocks().iter().zip(loaded.blocks().iter()) {
            assert_eq!(cleared.execution_count, loaded.execution_count, "both None");
            assert!(matches!(cleared.output, BlockOutput::NeverRun));
            assert!(matches!(loaded.output, BlockOutput::NeverRun));
            assert_eq!(cleared.is_obsolete(), loaded.is_obsolete());
        }
        assert_eq!(original.session().document().is_none(), loaded.session().document().is_none());
        assert!(original.session().document().is_none());
    }

    /// Neither clearing nor the underlying command it backs ever
    /// executes anything on its own -- confirmed directly: a notebook
    /// that never had ANY block executed produces the exact same
    /// (empty/NeverRun) state after `clear_execution` as before it, and
    /// no `Session::document` ever gets created by the call itself.
    #[test]
    fn clear_execution_never_executes_anything_by_itself() {
        let mut nb = Notebook::new();
        nb.create_block_after(None, "manifold M dim 4".to_string());
        nb.create_block_after(None, "ricci".to_string());
        assert!(nb.session().document().is_none());

        nb.clear_execution();

        assert!(nb.session().document().is_none(), "clear_execution must never itself trigger a reconstruction");
        for block in nb.blocks() {
            assert!(matches!(block.output, BlockOutput::NeverRun));
        }
    }
}

//! The top-level state a future UI (or, for now, a headless test/REPL)
//! operates on -- DESIGN-UI-SESSION.md. No window, no Tauri: this crate
//! doesn't know either exists.

use crate::cache::ComputeCache;
use crate::document::{changed_or_removed_names, Document, Generation};
use crate::entry::{Entry, EntryId, EntryResult, EntryState};
use crate::run::run_query;
use oderom_cli::commands::ExecutionContext;
use oderom_cli::parser::{parse_query, Query};
use oderom_cli::CliError;
use oderom_components::ComponentError;
use std::collections::BTreeSet;
use std::sync::Arc;

#[derive(Debug)]
pub struct EvalSummary {
    pub names: Vec<String>,
    pub elapsed_ms: u64,
}

/// A query that has been parsed and snapshotted against a specific
/// `Document` generation, ready to run -- deliberately decoupled from
/// `&mut Session` (Etapa 3b, DESIGN-NOTEBOOK.md's cancellation
/// section): the whole reason this split exists is so the actual
/// (possibly slow, possibly non-terminating) computation can happen on
/// a thread that never holds whatever lock a caller needs to keep
/// answering other requests meanwhile -- for `oderom-notebook`, that's
/// the app-wide `Mutex<Notebook>` a hung query used to hold for its
/// entire duration.
///
/// `document` is an owned clone, not a borrow: `Model`/`Document`
/// derive `Clone` for exactly this (see `Model`'s own doc comment) --
/// cheap, since the expensive intermediate values a query produces live
/// in `ComputeCache`, not in `Document`. `cache` is *moved* out of
/// `Session` (`Session::begin_query`, via `mem::take`), not cloned --
/// nothing outside a running query ever needs to read it, so there is
/// nothing to keep in sync, only something to hand back
/// (`Session::finish_query`) once this is done. If a second query
/// starts on a different entry while this one is still out, it works
/// against a fresh, temporarily-empty cache instead of this one --
/// slower (a cache miss instead of a hit), never wrong: see this
/// crate's own `PendingQuery` tests and DESIGN-NOTEBOOK.md for why nothing
/// stronger is needed for the notebook's actual usage pattern (at most
/// one long-running query realistically ever in flight at a time).
pub struct PendingQuery {
    document: Document,
    cache: ComputeCache,
    query: Query,
    ctx: Arc<ExecutionContext>,
}

impl PendingQuery {
    /// The same `Arc<ExecutionContext>` `run` below will use --
    /// `.cancel()` on this from any other thread interrupts the
    /// computation, same mechanism the REPL's Ctrl+C already relies on.
    /// Cloning an `Arc` is cheap; callers that need to cancel this
    /// later (`oderom-notebook`'s own `running` map) keep a clone of
    /// this, not the `PendingQuery` itself (which `run` consumes).
    pub fn context(&self) -> Arc<ExecutionContext> {
        self.ctx.clone()
    }

    /// Does the actual computation -- safe to call from any thread,
    /// including one that outlives whatever spawned it: touches nothing
    /// but its own owned fields and `oderom_cli`/`oderom_components`'s
    /// pure functions. Consumes `self` since a `PendingQuery` is only
    /// ever meant to run once.
    pub fn run(mut self) -> PendingQueryResult {
        let as_of = self.document.generation;
        let outcome = run_query(&self.document, &mut self.cache, &self.query, &self.ctx);
        PendingQueryResult { cache: self.cache, as_of, outcome }
    }
}

/// What `PendingQuery::run` produces -- handed to `Session::finish_query`
/// to actually store. Kept as its own type (not just the bare `Result`)
/// so `cache`/`as_of` travel back to `Session` alongside the outcome,
/// not as separate out-parameters a caller could forget to thread
/// through correctly.
pub struct PendingQueryResult {
    cache: ComputeCache,
    as_of: Generation,
    outcome: Result<(EntryResult, BTreeSet<String>), CliError>,
}

/// What `Session::begin_query` hands back -- `Failed` when there was
/// nothing to actually thread off (no document yet, or `input` doesn't
/// even parse as a query): the entry already carries its final state,
/// synchronously, exactly as `run_entry` always has for these same
/// cases. `Running` is the real split: a `PendingQuery` to run
/// elsewhere, an `EntryId` already showing `EntryState::Running` in
/// `Session::entries()` right now, before that run has even started.
pub enum QueryStart {
    Failed(EntryId),
    Running(EntryId, PendingQuery),
}

#[derive(Default)]
pub struct Session {
    source: String,
    document: Option<Document>,
    entries: Vec<Entry>,
    next_entry_id: u64,
    next_generation: u64,
    cache: ComputeCache,
}

impl Session {
    pub fn new() -> Self {
        Session::default()
    }

    /// `None` before the first successful `evaluate_definitions` call in
    /// this session's life; never `None` again after that (a *failed*
    /// evaluation leaves whatever was there, seciton 4's atomicity).
    pub fn document(&self) -> Option<&Document> {
        self.document.as_ref()
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Removes the entry with `id`, if present -- a no-op otherwise
    /// (mirrors `recompute_entry_with_context`'s own "no such entry,
    /// nothing to do" convention rather than erroring on a stale id).
    ///
    /// Exists for `oderom-notebook`: reexecuting a query block calls
    /// this on the block's previous `EntryId` before creating a new
    /// one, so the same block executed many times leaves exactly one
    /// live entry, not one per execution. Without it, `session.entries()`
    /// grows without bound as a notebook is worked on -- unlike the
    /// REPL, where reexecuting the same input is rare, a notebook block
    /// is reexecuted routinely, and the obsolescence scan in
    /// `evaluate_definitions` walks every entry on every declaration
    /// reconstruction, so the cost compounds with session lifetime.
    pub fn remove_entry(&mut self, id: EntryId) {
        self.entries.retain(|e| e.id != id);
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    /// Parses `source`, and on success atomically replaces the current
    /// `Document` -- on failure, leaves it untouched and returns the
    /// error with its line/column (DESIGN-UI-SESSION.md section 4).
    /// Marks every `Entry` whose `used` set overlaps a changed-or-removed
    /// name `Stale` (section 3) -- computed here, against the outgoing
    /// `Document`, before it's dropped.
    pub fn evaluate_definitions(&mut self, source: String) -> Result<EvalSummary, CliError> {
        let start = std::time::Instant::now();
        let generation = Generation(self.next_generation);
        let new_document = Document::parse(&source, generation)?;

        if let Some(old_document) = &self.document {
            let changed = changed_or_removed_names(old_document, &new_document);
            for entry in &mut self.entries {
                entry.state.mark_stale_if_affected(&changed);
            }
        }

        let names: Vec<String> = new_document.fingerprints.keys().cloned().collect();
        self.next_generation += 1;
        self.source = source;
        self.document = Some(new_document); // the atomic swap: one assignment, after everything above succeeded
        Ok(EvalSummary { names, elapsed_ms: start.elapsed().as_millis() as u64 })
    }

    /// Parses `input` as a `Query` (the *same* grammar the `.od` document
    /// uses, DESIGN-UI-SESSION.md section 2's correction -- `parse_query`
    /// is not a bespoke keyword match) and runs it against the current
    /// `Document`, creating a new `Entry` in `Done` state on success or
    /// `Failed` on error. Returns the new entry's id either way -- the
    /// caller looks up `entries()` for the result/error, matching how a
    /// worksheet actually accumulates entries rather than just returning
    /// one output and forgetting it happened.
    ///
    /// Runs synchronously, on the calling thread, with a throwaway
    /// `ExecutionContext` nobody can cancel -- fine for tests and any
    /// caller that doesn't need to interrupt a slow query mid-flight.
    /// `oderom-repl` uses [`Session::run_entry_with_context`] instead,
    /// sharing one `ExecutionContext` across a worker thread and its own
    /// Ctrl+C handler.
    pub fn run_entry(&mut self, input: String) -> Result<EntryId, CliError> {
        self.run_entry_with_context(input, &ExecutionContext::new())
    }

    pub fn run_entry_with_context(&mut self, input: String, ctx: &ExecutionContext) -> Result<EntryId, CliError> {
        let id = EntryId(self.next_entry_id);
        self.next_entry_id += 1;
        let state = self.run_query_as_state(&input, ctx);
        self.entries.push(Entry { id, input, state });
        Ok(id)
    }

    /// Re-runs an existing entry's own input against the *current*
    /// `Document` -- the "recalcular" the acceptance criterion names:
    /// a `Stale` entry goes back to `Done`/`Failed` at the current
    /// generation. Same synchronous-vs-cancellable split as
    /// `run_entry`/`run_entry_with_context`.
    pub fn recompute_entry(&mut self, id: EntryId) -> Result<(), CliError> {
        self.recompute_entry_with_context(id, &ExecutionContext::new())
    }

    pub fn recompute_entry_with_context(&mut self, id: EntryId, ctx: &ExecutionContext) -> Result<(), CliError> {
        let Some(index) = self.entries.iter().position(|e| e.id == id) else {
            return Ok(()); // no such entry -- nothing to do, not an error worth a variant for yet
        };
        let input = self.entries[index].input.clone();
        let state = self.run_query_as_state(&input, ctx);
        self.entries[index].state = state;
        Ok(())
    }

    fn run_query_as_state(&mut self, input: &str, ctx: &ExecutionContext) -> EntryState {
        let Some(document) = &self.document else {
            return EntryState::Failed { message: "no definitions evaluated yet".to_string(), line: None, column: None };
        };
        outcome_to_state(document.generation, parse_query(input).and_then(|query| run_query(document, &mut self.cache, &query, ctx)))
    }

    /// The cancellable, off-thread half of running a query -- Etapa 3b
    /// (DESIGN-NOTEBOOK.md's cancellation section). Unlike
    /// `run_entry`/`run_entry_with_context`, this never blocks on the
    /// computation itself: it does only the parts that must happen
    /// against `&mut self` (snapshotting the current `Document`, taking
    /// the cache, creating the `Entry` immediately in `Running` state)
    /// and hands everything else back as a `PendingQuery` for the
    /// caller to `.run()` wherever it won't hold a lock this `Session`
    /// might be behind.
    ///
    /// `QueryStart::Failed` short-circuits the same two cases
    /// `run_query_as_state` already short-circuits synchronously (no
    /// document yet; `input` doesn't parse) -- nothing to thread off
    /// either way, so nothing is.
    pub fn begin_query(&mut self, input: String) -> QueryStart {
        let id = EntryId(self.next_entry_id);
        self.next_entry_id += 1;

        let Some(document) = self.document.clone() else {
            self.entries.push(Entry { id, input, state: EntryState::Failed { message: "no definitions evaluated yet".to_string(), line: None, column: None } });
            return QueryStart::Failed(id);
        };
        let query = match parse_query(&input) {
            Ok(q) => q,
            Err(CliError::Parse { message, position }) => {
                self.entries.push(Entry { id, input, state: EntryState::Failed { message, line: position.map(|p| p.line), column: position.map(|p| p.column) } });
                return QueryStart::Failed(id);
            }
            Err(e) => {
                self.entries.push(Entry { id, input, state: EntryState::Failed { message: e.to_string(), line: None, column: None } });
                return QueryStart::Failed(id);
            }
        };

        let ctx = ExecutionContext::new();
        let cache = std::mem::take(&mut self.cache);
        self.entries.push(Entry { id, input, state: EntryState::Running });
        QueryStart::Running(id, PendingQuery { document, cache, query, ctx })
    }

    /// Stores a `PendingQuery::run()` result against the entry
    /// `begin_query` created for it, and restores the cache it borrowed.
    /// Returns whether the computation was cancelled -- the caller
    /// (`oderom-notebook`) needs that to decide whether to keep or drop
    /// an earlier result the block may have had (`Notebook::finish_query`'s
    /// own doc comment).
    pub fn finish_query(&mut self, id: EntryId, result: PendingQueryResult) -> bool {
        let PendingQueryResult { cache, as_of, outcome } = result;
        self.cache = cache;
        let is_cancelled = matches!(&outcome, Err(CliError::Component(ComponentError::Cancelled)));
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) {
            entry.state = outcome_to_state(as_of, outcome);
        }
        is_cancelled
    }
}

/// Shared by the synchronous (`run_query_as_state`) and cancellable
/// (`Session::finish_query`) paths -- one place deciding what a
/// `run_query` outcome means as an `EntryState`, so the two can never
/// quietly disagree (e.g. one recognizing a cancellation distinctly and
/// the other still calling it a plain `Failed`).
fn outcome_to_state(as_of: Generation, outcome: Result<(EntryResult, BTreeSet<String>), CliError>) -> EntryState {
    match outcome {
        Ok((result, used)) => EntryState::Done { result, used, as_of },
        Err(CliError::Component(ComponentError::Cancelled)) => EntryState::Cancelled,
        Err(CliError::Parse { message, position }) => EntryState::Failed { message, line: position.map(|p| p.line), column: position.map(|p| p.column) },
        Err(e) => EntryState::Failed { message: e.to_string(), line: None, column: None },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHWARZSCHILD: &str = "
manifold M dim 4
bundle TM on M dim 4
chart schw on M coords (t, r, theta, phi)
metric g on schw bundle TM {
  [t,t] = -(1 - 2*M/r),
  [r,r] = 1/(1 - 2*M/r),
  [theta,theta] = r^2,
  [phi,phi] = r^2 * sin(theta)^2
}
";

    #[test]
    fn full_cycle_define_query_redefine_stale_recompute_fresh() {
        let mut session = Session::new();

        // define
        session.evaluate_definitions(SCHWARZSCHILD.to_string()).unwrap();

        // query
        let id = session.run_entry("ricci".to_string()).unwrap();
        let entry = session.entries().iter().find(|e| e.id == id).unwrap();
        assert!(entry.state.has_result(), "expected a result");
        assert!(!entry.state.is_stale());

        // redefine (change the metric)
        let edited = SCHWARZSCHILD.replace("2*M/r", "3*M/r");
        session.evaluate_definitions(edited).unwrap();

        // verify obsolescence
        let entry = session.entries().iter().find(|e| e.id == id).unwrap();
        assert!(entry.state.is_stale(), "entry should be stale after redefining the metric it used");
        assert!(entry.state.has_result(), "a stale entry still carries its last-known result, not nothing");

        // recalculate
        session.recompute_entry(id).unwrap();

        // verify it's current again
        let entry = session.entries().iter().find(|e| e.id == id).unwrap();
        assert!(!entry.state.is_stale(), "entry should be fresh again after recomputing");
        assert!(entry.state.has_result());
    }

    #[test]
    fn editing_the_chart_also_invalidates_an_entry_that_only_named_the_metric() {
        let mut session = Session::new();
        session.evaluate_definitions(SCHWARZSCHILD.to_string()).unwrap();
        let id = session.run_entry("christoffel".to_string()).unwrap();

        // See document.rs's equivalent test for why "phi" -> "psi"
        // (renamed everywhere, chart list and the metric's own
        // by-name bracket reference) is the edit that isolates "only
        // the chart changed" under this grammar.
        let edited = SCHWARZSCHILD.replace("phi", "psi");
        session.evaluate_definitions(edited).unwrap();

        let entry = session.entries().iter().find(|e| e.id == id).unwrap();
        assert!(entry.state.is_stale(), "editing the chart must invalidate an entry that used the metric declared on it");
    }

    #[test]
    fn unrelated_redefinition_does_not_invalidate_anything() {
        let mut session = Session::new();
        session.evaluate_definitions(SCHWARZSCHILD.to_string()).unwrap();
        let id = session.run_entry("ricci".to_string()).unwrap();

        // Re-evaluating byte-identical source changes nothing.
        session.evaluate_definitions(SCHWARZSCHILD.to_string()).unwrap();

        let entry = session.entries().iter().find(|e| e.id == id).unwrap();
        assert!(!entry.state.is_stale(), "re-evaluating identical definitions must not invalidate anything");
    }

    #[test]
    fn a_failed_evaluation_leaves_the_previous_document_valid() {
        let mut session = Session::new();
        session.evaluate_definitions(SCHWARZSCHILD.to_string()).unwrap();
        let before = session.document().unwrap().generation;

        let err = session.evaluate_definitions("bogus keyword here".to_string()).unwrap_err();
        assert!(matches!(err, CliError::Parse { .. }));

        let after = session.document().unwrap().generation;
        assert_eq!(before, after, "a failed evaluate_definitions must not replace the Document");

        // The session is still fully usable against the old definitions.
        let id = session.run_entry("ricci".to_string()).unwrap();
        assert!(session.entries().iter().find(|e| e.id == id).unwrap().state.has_result());
    }

    /// The failure above is a lexer/grammar error (`CliError::Parse`).
    /// Atomicity has to hold just as well for a *post-parse* failure --
    /// syntactically valid, rejected while building the `Model` itself
    /// (`Registry::declare_manifold` returning `CoreError::DuplicateName`,
    /// propagated as `CliError::Core`, not `CliError::Parse`) -- since
    /// `Document::parse` calls `parser::parse_model`, and everything
    /// *inside* that single function call, whatever kind of error it
    /// returns, has to leave `Session::document` untouched the same way.
    #[test]
    fn a_post_parse_model_construction_failure_also_leaves_the_previous_document_valid() {
        let mut session = Session::new();
        session.evaluate_definitions(SCHWARZSCHILD.to_string()).unwrap();
        let before = session.document().unwrap().generation;

        let redeclared = format!("{SCHWARZSCHILD}\nmanifold M dim 4\n");
        let err = session.evaluate_definitions(redeclared).unwrap_err();
        assert!(matches!(err, CliError::Core(_)), "expected a post-parse CoreError, got {err:?}");

        let after = session.document().unwrap().generation;
        assert_eq!(before, after, "a failed evaluate_definitions must not replace the Document, regardless of which stage inside it failed");

        let id = session.run_entry("ricci".to_string()).unwrap();
        assert!(session.entries().iter().find(|e| e.id == id).unwrap().state.has_result());
    }

    /// `remove_entry` exists specifically so a notebook block that gets
    /// reexecuted -- the routine case there, unlike the REPL -- doesn't
    /// leave one orphaned entry per execution. Simulates exactly that
    /// usage: remove the previous entry (if any) right before creating
    /// the next one, the same pattern `oderom-notebook`'s query-block
    /// execution uses.
    #[test]
    fn reexecuting_the_same_entry_a_hundred_times_does_not_grow_the_entry_list() {
        let mut session = Session::new();
        session.evaluate_definitions(SCHWARZSCHILD.to_string()).unwrap();

        let mut current: Option<EntryId> = None;
        for _ in 0..100 {
            if let Some(id) = current.take() {
                session.remove_entry(id);
            }
            current = Some(session.run_entry("ricci".to_string()).unwrap());
        }

        assert_eq!(session.entries().len(), 1, "expected exactly one live entry after 100 remove-then-create cycles, found {}", session.entries().len());
    }

    #[test]
    fn removing_an_unknown_entry_id_is_a_no_op() {
        let mut session = Session::new();
        session.evaluate_definitions(SCHWARZSCHILD.to_string()).unwrap();
        let id = session.run_entry("ricci".to_string()).unwrap();
        session.remove_entry(EntryId(id.0 + 999));
        assert_eq!(session.entries().len(), 1);
    }

    #[test]
    fn running_the_same_query_twice_reuses_the_cache() {
        let mut session = Session::new();
        session.evaluate_definitions(SCHWARZSCHILD.to_string()).unwrap();
        session.run_entry("ricci".to_string()).unwrap();
        session.run_entry("riemann".to_string()).unwrap();
        // riemann_mixed was computed once (by "ricci") and reused by
        // "riemann" -- a cache hit, not a second computation.
        assert_eq!(session.cache.riemann_mixed.len(), 1);
    }

    /// The typography/click-to-copy round's own real regression: an
    /// `EntryResult` for a components-list query must come back with
    /// `components` split apart from `summary`, each `RenderedComponent`
    /// carrying a clean, named-index formula with no orbit-size prose
    /// glued in -- exactly what `oderom-app`'s DTO layer forwards to the
    /// frontend for KaTeX rendering and click-to-copy, so a regression
    /// here would silently break both.
    #[test]
    fn riemann_entry_result_carries_structured_named_index_components_separate_from_summary() {
        let mut session = Session::new();
        session.evaluate_definitions(SCHWARZSCHILD.to_string()).unwrap();
        let id = session.run_entry("riemann".to_string()).unwrap();
        let entry = session.entries().iter().find(|e| e.id == id).unwrap();
        let EntryState::Done { result, .. } = &entry.state else { panic!("expected Done") };

        assert!(!result.components.is_empty(), "expected at least one independent nonzero component");
        assert!(
            result.components.iter().any(|c| c.formula.contains("R_{t r t r}")),
            "expected a named, space-separated, fully covariant index (Schwarzschild's riemann is metric-sourced): {:?}",
            result.components.iter().map(|c| &c.formula).collect::<Vec<_>>()
        );
        assert!(
            !result.components.iter().any(|c| c.formula.contains("components by symmetry")),
            "the orbit note must never be glued into a component's own formula string"
        );
        assert!(
            result.components.iter().any(|c| c.orbit_note.is_some()),
            "Schwarzschild's riemann has symmetry-related components; expected at least one real orbit_note"
        );
        assert!(
            result.summary.iter().any(|line| line.contains("independent component") && line.contains("identically zero")),
            "expected the zero-count line in `summary`, separate from any component: {:?}",
            result.summary
        );
    }

    /// Marco 6 step 4, round B: `geodesic` through the session/notebook
    /// path (`Query::Geodesic`, not `Query::Command`) -- proves the
    /// second `run_query_inner` branch, `Rendered::Equations`, and
    /// `check_affine_parameter_collision` are all wired correctly end to
    /// end, cache-aware and cancellable exactly like the other five
    /// commands, not just reachable from the standalone CLI binary
    /// (`oderom-cli/tests/end_to_end.rs` already covers that path).
    #[test]
    fn geodesic_entry_result_carries_four_structured_equations() {
        let mut session = Session::new();
        session.evaluate_definitions(SCHWARZSCHILD.to_string()).unwrap();
        let id = session.run_entry("geodesic tau".to_string()).unwrap();
        let entry = session.entries().iter().find(|e| e.id == id).unwrap();
        let EntryState::Done { result, .. } = &entry.state else { panic!("expected Done") };

        assert_eq!(result.components.len(), 4, "{:?}", result.components.iter().map(|c| &c.formula).collect::<Vec<_>>());
        assert!(result.latex.contains("\\ddot{t}"), "{}", result.latex);
        assert!(result.unicode.contains("t''(tau)"), "{}", result.unicode);
    }

    /// The mandatory collision rule reached through the session path
    /// too, not just the standalone CLI: a parameter equal to a chart
    /// coordinate must fail cleanly, never silently produce an
    /// ambiguous equation.
    #[test]
    fn geodesic_entry_fails_cleanly_when_the_parameter_collides_with_a_coordinate() {
        let mut session = Session::new();
        session.evaluate_definitions(SCHWARZSCHILD.to_string()).unwrap();
        let id = session.run_entry("geodesic r".to_string()).unwrap();
        let entry = session.entries().iter().find(|e| e.id == id).unwrap();
        let EntryState::Failed { message, .. } = &entry.state else { panic!("expected Failed") };
        assert!(message.contains("collides with chart coordinate"), "{message}");
    }

    /// Marco 6 step 4, round C: `accel` through the session/notebook
    /// path (`Query::Accel`, not `Query::Command`) -- proves the third
    /// `run_query_inner` branch and `Rendered::AccelEquations` are wired
    /// correctly end to end, same as `geodesic` above.
    #[test]
    fn accel_entry_result_carries_four_solved_equations() {
        let mut session = Session::new();
        session.evaluate_definitions(SCHWARZSCHILD.to_string()).unwrap();
        let id = session.run_entry("accel tau".to_string()).unwrap();
        let entry = session.entries().iter().find(|e| e.id == id).unwrap();
        let EntryState::Done { result, .. } = &entry.state else { panic!("expected Done") };

        assert_eq!(result.components.len(), 4, "{:?}", result.components.iter().map(|c| &c.formula).collect::<Vec<_>>());
        assert!(result.latex.contains("\\ddot{t} = "), "{}", result.latex);
        assert!(result.unicode.contains("t''(tau) = "), "{}", result.unicode);
        // Never the canonical `= 0` form -- `geodesic`'s own shape,
        // untouched by this command.
        assert!(!result.unicode.contains("= 0"), "{}", result.unicode);
    }

    /// Same collision rule reached through the session path for `accel`
    /// too, named correctly (not a leftover "geodesic" in the message).
    #[test]
    fn accel_entry_fails_cleanly_when_the_parameter_collides_with_a_coordinate() {
        let mut session = Session::new();
        session.evaluate_definitions(SCHWARZSCHILD.to_string()).unwrap();
        let id = session.run_entry("accel r".to_string()).unwrap();
        let entry = session.entries().iter().find(|e| e.id == id).unwrap();
        let EntryState::Failed { message, .. } = &entry.state else { panic!("expected Failed") };
        assert!(message.contains("collides with chart coordinate"), "{message}");
        assert!(message.contains("accel needs a distinct name"), "{message}");
    }

    /// Marco 6 step 5: `einstein` through the session/notebook path --
    /// an ordinary `Query::Command` (unlike `geodesic`/`accel`'s own
    /// variants), so this exercises the new `CommandName::Einstein` arm
    /// inside `run_command_query`'s match, not a new `Query` variant.
    /// Schwarzschild is vacuum, so all ten independent components must
    /// come back identically zero -- the same golden check the
    /// `oderom-components`/CLI levels already establish, now confirmed
    /// through the cache-aware session path too.
    #[test]
    fn einstein_entry_result_shows_all_ten_independent_components_as_zero_on_schwarzschild() {
        let mut session = Session::new();
        session.evaluate_definitions(SCHWARZSCHILD.to_string()).unwrap();
        let id = session.run_entry("einstein".to_string()).unwrap();
        let entry = session.entries().iter().find(|e| e.id == id).unwrap();
        let EntryState::Done { result, .. } = &entry.state else { panic!("expected Done") };
        assert!(result.components.is_empty(), "all ten components are identically zero, so none should be listed: {:?}", result.components.iter().map(|c| &c.formula).collect::<Vec<_>>());
        assert!(
            result.summary.iter().any(|line| line.contains("10") && line.contains("identically zero")),
            "expected the zero-count line in `summary`: {:?}",
            result.summary
        );
    }

    /// `einstein` needs a real metric -- reached through the session
    /// path against a bare-connection fixture, same refusal the CLI
    /// level already confirms.
    #[test]
    fn einstein_entry_fails_cleanly_from_a_bare_connection() {
        let mut session = Session::new();
        session
            .evaluate_definitions("manifold M dim 2\nbundle TM on M dim 2\nchart flat2 on M coords (x, y)\nconnection Gamma on flat2 {\n  [x,y,y] = x\n}".to_string())
            .unwrap();
        let id = session.run_entry("einstein".to_string()).unwrap();
        let entry = session.entries().iter().find(|e| e.id == id).unwrap();
        let EntryState::Failed { message, .. } = &entry.state else { panic!("expected Failed") };
        assert!(message.contains("needs a metric"), "{message}");
    }

    // -------------------------------------------------------------
    // Index-variance marker (Rodada Variancia): `riemann [up,down,down,down]`
    // and friends, exercised end to end through the real query grammar
    // and session dispatch -- `oderom-components/tests/schwarzschild.rs`/
    // `reissner_nordstrom.rs` already prove the underlying arithmetic
    // (`change_variance`) directly; these prove the query surface wires
    // it up correctly: grammar, refusals, arity, and real rendered
    // typography.
    // -------------------------------------------------------------

    /// The "real window" case: Riemann shown in a genuinely mixed
    /// variance on a real metric, rendered with real sub/superscript
    /// typography -- Schwarzschild's Riemann tensor is curved (only its
    /// Ricci trace vanishes), so this has real, nonzero, non-trivial
    /// components to show, not an all-zero result that would leave the
    /// typography unexercised.
    #[test]
    fn riemann_up_down_down_down_on_schwarzschild_renders_with_mixed_typography() {
        let mut session = Session::new();
        session.evaluate_definitions(SCHWARZSCHILD.to_string()).unwrap();
        let id = session.run_entry("riemann [up,down,down,down]".to_string()).unwrap();
        let entry = session.entries().iter().find(|e| e.id == id).unwrap();
        let EntryState::Done { result, .. } = &entry.state else { panic!("expected Done") };
        assert!(!result.components.is_empty(), "Schwarzschild's Riemann tensor is curved -- expected at least one nonzero independent component");
        // Unicode target's own mixed-variance notation (format_indices):
        // a leading `^name` group for the contravariant slot, plain
        // names after it for the three covariant ones -- e.g.
        // `R[^t,r,t,r] = ...`. Every shown component names the raised
        // slot with a `^`, none of the other three.
        for c in &result.components {
            assert!(c.formula.contains('^'), "expected a `^`-marked contravariant index in {:?}", c.formula);
        }
    }

    /// The round trip, exercised through the query surface itself (not
    /// just `change_variance` directly, `oderom-components/tests/schwarzschild.rs`'s
    /// own job): raising then immediately lowering the same index must
    /// show identically zero components for Schwarzschild's Ricci
    /// (already zero to begin with, any variance) -- a coarse but real
    /// confirmation that two chained variance queries against the same
    /// document don't silently diverge from the direct, unmarked
    /// `ricci` query's own already-established zero result.
    #[test]
    fn ricci_up_up_on_schwarzschild_is_still_identically_zero() {
        let mut session = Session::new();
        session.evaluate_definitions(SCHWARZSCHILD.to_string()).unwrap();
        let id = session.run_entry("ricci [up,up]".to_string()).unwrap();
        let entry = session.entries().iter().find(|e| e.id == id).unwrap();
        let EntryState::Done { result, .. } = &entry.state else { panic!("expected Done") };
        assert!(result.components.is_empty(), "Schwarzschild's Ricci is vacuum -- R^ab should still be identically zero: {:?}", result.components);
        assert!(result.summary.iter().any(|line| line.contains("identically zero")), "{:?}", result.summary);
    }

    /// The safety barrier: Christoffel is not a tensor, so a
    /// variance marker on it must refuse with the explanatory message,
    /// not silently compute something. `christoffel` with NO marker is
    /// untouched (confirmed by the many other tests in this file that
    /// already query it directly).
    #[test]
    fn christoffel_with_a_variance_marker_refuses_because_it_is_not_a_tensor() {
        let mut session = Session::new();
        session.evaluate_definitions(SCHWARZSCHILD.to_string()).unwrap();
        let id = session.run_entry("christoffel [up,down,down]".to_string()).unwrap();
        let entry = session.entries().iter().find(|e| e.id == id).unwrap();
        let EntryState::Failed { message, .. } = &entry.state else { panic!("expected Failed") };
        assert!(message.contains("not a tensor"), "{message}");
        assert!(message.contains("riemann"), "expected the refusal to point the user at `riemann` instead: {message}");
    }

    /// A variance marker on a bare scalar (no indices at all) is a
    /// clean, explained error -- not silently ignored, not a panic.
    #[test]
    fn scalar_with_a_variance_marker_refuses_because_it_has_no_indices() {
        let mut session = Session::new();
        session.evaluate_definitions(SCHWARZSCHILD.to_string()).unwrap();
        let id = session.run_entry("scalar [up]".to_string()).unwrap();
        let entry = session.entries().iter().find(|e| e.id == id).unwrap();
        let EntryState::Failed { message, .. } = &entry.state else { panic!("expected Failed") };
        assert!(message.contains("scalar"), "{message}");
    }

    /// The marker's own length must match the named tensor's rank --
    /// three entries for Riemann (rank 4) is a clean arity error, not a
    /// silently-padded or silently-truncated guess.
    #[test]
    fn riemann_with_a_three_entry_marker_refuses_with_a_clear_arity_mismatch() {
        let mut session = Session::new();
        session.evaluate_definitions(SCHWARZSCHILD.to_string()).unwrap();
        let id = session.run_entry("riemann [up,down,down]".to_string()).unwrap();
        let entry = session.entries().iter().find(|e| e.id == id).unwrap();
        let EntryState::Failed { message, .. } = &entry.state else { panic!("expected Failed") };
        assert!(message.contains('4') && message.contains('3'), "expected the arity mismatch (4 vs 3) named in the message: {message}");
    }

    /// A variance marker always needs a real metric, even for `ricci`,
    /// which normally works fine from a bare connection with no marker
    /// at all -- the same `NeedsMetric` refusal `scalar`/`einstein`
    /// already give, now also reachable through a command that doesn't
    /// otherwise require one.
    #[test]
    fn ricci_with_a_variance_marker_from_a_bare_connection_needs_a_metric() {
        let mut session = Session::new();
        session
            .evaluate_definitions("manifold M dim 2\nbundle TM on M dim 2\nchart flat2 on M coords (x, y)\nconnection Gamma on flat2 {\n  [x,y,y] = x\n}".to_string())
            .unwrap();
        // Confirm the unmarked query still works from the bare
        // connection first -- proves the refusal below is specifically
        // about the marker, not some unrelated fixture problem.
        let unmarked_id = session.run_entry("ricci".to_string()).unwrap();
        let unmarked_entry = session.entries().iter().find(|e| e.id == unmarked_id).unwrap();
        assert!(unmarked_entry.state.has_result(), "ricci with no marker should still work from a bare connection");

        let id = session.run_entry("ricci [up,up]".to_string()).unwrap();
        let entry = session.entries().iter().find(|e| e.id == id).unwrap();
        let EntryState::Failed { message, .. } = &entry.state else { panic!("expected Failed") };
        assert!(message.contains("needs a metric"), "{message}");
    }

    /// Reissner-Nordstrom, not Schwarzschild -- the case this whole
    /// engine was rebuilt around (DESIGN-RATIONAL-FORM.md) and the
    /// project's standing permanent regression fixture
    /// (`oderom-components/tests/reissner_nordstrom.rs`). Exercised here
    /// too so the session layer's first real test isn't only the
    /// single-parameter case.
    const REISSNER_NORDSTROM: &str = "
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

    #[test]
    fn full_cycle_on_reissner_nordstrom() {
        let mut session = Session::new();
        session.evaluate_definitions(REISSNER_NORDSTROM.to_string()).unwrap();

        let id = session.run_entry("kretschmann".to_string()).unwrap();
        let entry = session.entries().iter().find(|e| e.id == id).unwrap();
        assert!(entry.state.has_result(), "expected a result");
        let EntryState::Done { result, .. } = &entry.state else { panic!("expected Done") };
        // 48M^2/r^6 - 96MQ^2/r^7 + 56Q^4/r^8, whatever exact rendering
        // normalize()'s canonical form picks -- just confirm it actually
        // completed with a nonempty, non-error answer.
        assert!(!result.unicode.is_empty());

        let edited = REISSNER_NORDSTROM.replace("2*M/r", "3*M/r");
        session.evaluate_definitions(edited).unwrap();
        let entry = session.entries().iter().find(|e| e.id == id).unwrap();
        assert!(entry.state.is_stale());

        session.recompute_entry(id).unwrap();
        let entry = session.entries().iter().find(|e| e.id == id).unwrap();
        assert!(!entry.state.is_stale());
        assert!(entry.state.has_result());
    }

    /// Marco 6 step 6, through the session/notebook path: the golden
    /// check, Schwarzschild's Weyl tensor equal to its Riemann tensor
    /// byte for byte -- the same identity `oderom-components`/the CLI
    /// already confirm, now through the cache-aware session layer too
    /// (which reuses the same `weyl_tensor` cache slot regardless of
    /// which of `weyl`/`weylsquare` asked for it first).
    #[test]
    fn weyl_entry_result_equals_riemann_on_schwarzschild() {
        let mut session = Session::new();
        session.evaluate_definitions(SCHWARZSCHILD.to_string()).unwrap();
        let weyl_id = session.run_entry("weyl".to_string()).unwrap();
        let riemann_id = session.run_entry("riemann".to_string()).unwrap();
        let weyl_entry = session.entries().iter().find(|e| e.id == weyl_id).unwrap();
        let riemann_entry = session.entries().iter().find(|e| e.id == riemann_id).unwrap();
        let EntryState::Done { result: weyl_result, .. } = &weyl_entry.state else { panic!("expected Done") };
        let EntryState::Done { result: riemann_result, .. } = &riemann_entry.state else { panic!("expected Done") };
        let weyl_formulas: Vec<&str> = weyl_result.components.iter().map(|c| c.formula.split_once(" = ").unwrap().1).collect();
        let riemann_formulas: Vec<&str> = riemann_result.components.iter().map(|c| c.formula.split_once(" = ").unwrap().1).collect();
        assert_eq!(weyl_formulas, riemann_formulas);
    }

    /// The mandatory dimension barrier, through the session path: a 2D
    /// metric must refuse cleanly, not divide by zero or hang.
    #[test]
    fn weyl_entry_fails_cleanly_in_two_dimensions() {
        let mut session = Session::new();
        session
            .evaluate_definitions(
                "manifold H2 dim 2\nbundle TH2 on H2 dim 2\nchart polar on H2 coords (chi, phi)\nmetric g on polar bundle TH2 {\n  [chi,chi] = 1,\n  [phi,phi] = sinh(chi)^2\n}"
                    .to_string(),
            )
            .unwrap();
        let id = session.run_entry("weyl".to_string()).unwrap();
        let entry = session.entries().iter().find(|e| e.id == id).unwrap();
        let EntryState::Failed { message, .. } = &entry.state else { panic!("expected Failed") };
        assert!(message.contains("not defined in 2 dimensions"), "{message}");
    }

    /// `riccisquare`/`gaussbonnet`, through the session path: Reissner-
    /// Nordstrom gives a genuinely nonzero invariant for both.
    #[test]
    fn riccisquare_and_gaussbonnet_entries_are_nonzero_on_reissner_nordstrom() {
        let mut session = Session::new();
        session.evaluate_definitions(REISSNER_NORDSTROM.to_string()).unwrap();
        for query in ["riccisquare", "gaussbonnet"] {
            let id = session.run_entry(query.to_string()).unwrap();
            let entry = session.entries().iter().find(|e| e.id == id).unwrap();
            let EntryState::Done { result, .. } = &entry.state else { panic!("expected Done for {query}") };
            assert_ne!(result.unicode.trim(), "0", "{query} should be nonzero on Reissner-Nordstrom");
        }
    }

    /// The whole reason `run_entry_with_context` exists (`oderom-repl`'s
    /// Ctrl+C, ETAPA 2): cancelling actually stops the computation and
    /// returns promptly, not after finishing the full Kretschmann
    /// computation and discarding the result. A context cancelled
    /// *before* the query starts is the simplest deterministic version
    /// of this to test (no timing race to get right) -- it exercises the
    /// exact same checkpoint path a mid-flight cancel would hit on its
    /// very first check, which is enough to prove the wiring works;
    /// `oderom-repl`'s own manual testing covers real Ctrl+C-mid-flight
    /// timing, which isn't reproducible deterministically here anyway.
    #[test]
    fn a_pre_cancelled_context_aborts_instead_of_computing() {
        let mut session = Session::new();
        session.evaluate_definitions(REISSNER_NORDSTROM.to_string()).unwrap();

        let ctx = ExecutionContext::new();
        ctx.cancel();
        let start = std::time::Instant::now();
        let id = session.run_entry_with_context("kretschmann".to_string(), &ctx).unwrap();
        let elapsed = start.elapsed();

        assert!(elapsed < std::time::Duration::from_secs(1), "cancelled query took {elapsed:?} -- did not abort promptly");
        let entry = session.entries().iter().find(|e| e.id == id).unwrap();
        // Etapa 3b: a cancellation is now its own distinct `EntryState`,
        // detected via the typed `CliError::Component(ComponentError::Cancelled)`
        // path, not a `Failed` whose message happens to say "cancel".
        assert!(matches!(entry.state, EntryState::Cancelled), "expected Cancelled");
    }

    /// `g_tt = -(1 - 2M/r + 1/r^2)`, `g_rr = 1/(1 - 2M/r + Q^2/r^2)` --
    /// not reciprocal (`oderom-session/tests/cancellation.rs` has the
    /// full characterization). Used here, not Reissner-Nordstrom, because
    /// this is the metric that *guarantees* the cancellation is caught
    /// deep inside `poly_gcd`/the subresultant PRS -- RN's per-component
    /// checkpoint is reached so quickly (~13-23ms, `oderom-components/
    /// tests/diagnostic_cancel_latency.rs`) that a test against it could
    /// easily be satisfied by the cheap between-component `Result` path
    /// alone, never actually exercising the panic-based unwind this test
    /// is about.
    const NON_RECIPROCAL_TWO_PARAM: &str = "
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

    /// Answers the consistency question directly, not by argument:
    /// `run_cancellable`'s panic-based unwind (`oderom_expr::cancel`) can
    /// abort in the middle of `poly_gcd`'s recursive descent or the
    /// subresultant PRS loop -- does that leave anything behind that a
    /// later query could trip over? `NON_RECIPROCAL_TWO_PARAM` never
    /// returns on its own within any reasonable time, so cancelling it
    /// after 300ms is guaranteed to have unwound out of *something*
    /// mid-computation, not just hit a between-component boundary.
    #[test]
    fn mid_flight_cancellation_leaves_no_partial_cache_entry() {
        let mut session = Session::new();
        session.evaluate_definitions(NON_RECIPROCAL_TWO_PARAM.to_string()).unwrap();

        let ctx = ExecutionContext::new();
        let worker_ctx = ctx.clone();
        let handle = std::thread::spawn(move || {
            let id = session.run_entry_with_context("kretschmann".to_string(), &worker_ctx).unwrap();
            (session, id)
        });
        std::thread::sleep(std::time::Duration::from_millis(300));
        ctx.cancel();
        let (mut session, cancelled_id) = handle.join().expect("run_query must catch its own cancellation unwind internally, never let it escape the worker thread");

        let cancelled_entry = session.entries().iter().find(|e| e.id == cancelled_id).unwrap();
        // Etapa 3b: a distinct `Cancelled` state now, not a `Failed`
        // whose message happens to mention cancellation.
        assert!(matches!(cancelled_entry.state, EntryState::Cancelled), "expected the cancelled entry to be Cancelled");

        // No partial entry in the cache -- `LruCache::get_or_try_insert_with`
        // only calls `insert` after `compute()` returns `Ok`, so a panic
        // inside `compute()` (what a mid-flight cancellation is) never
        // reaches it. That is an argument from reading the code; this is
        // the measurement.
        assert_eq!(session.cache.riemann_mixed.len(), 0, "a cancelled riemann_mixed computation must not leave a partial entry in the cache");
        assert_eq!(session.cache.riemann_cov.len(), 0);
        assert_eq!(session.cache.ricci_tensor.len(), 0);
        assert_eq!(session.cache.kretschmann.len(), 0);

        // The session as a whole, not just this one entry: an unrelated
        // query -- `christoffel`, deliberately, since it is the one
        // stage that does *not* depend on `riemann_mixed` and so is
        // unaffected by this metric's own hang -- still works
        // afterward, proving nothing outside the cancelled entry itself
        // was left in a state "that doesn't correspond to anything".
        let christoffel_id = session.run_entry("christoffel".to_string()).unwrap();
        assert!(session.entries().iter().find(|e| e.id == christoffel_id).unwrap().state.has_result());
    }

    /// The other half of the same question, on Reissner-Nordstrom
    /// (which *does* finish on its own, unlike the metric above): does a
    /// query cancelled mid-flight and then retried, uncancelled,
    /// recompute from scratch and land on exactly the answer a session
    /// that was never cancelled would give -- not a stale, partial, or
    /// merely "some" value.
    #[test]
    fn retrying_after_a_mid_flight_cancellation_matches_a_never_cancelled_session_exactly() {
        let mut session = Session::new();
        session.evaluate_definitions(REISSNER_NORDSTROM.to_string()).unwrap();

        let ctx = ExecutionContext::new();
        let worker_ctx = ctx.clone();
        let handle = std::thread::spawn(move || {
            session.run_entry_with_context("kretschmann".to_string(), &worker_ctx).unwrap();
            session
        });
        // RN's own full kretschmann is ~1s in release -- a few tens of
        // ms lands inside the computation without waiting for it to
        // finish on its own.
        std::thread::sleep(std::time::Duration::from_millis(50));
        ctx.cancel();
        let mut session = handle.join().expect("run_query must catch its own cancellation unwind internally");
        assert_eq!(session.cache.riemann_mixed.len(), 0, "cancelled before completion -- nothing should be cached yet");

        let retry_id = session.run_entry("kretschmann".to_string()).unwrap();
        let retry_entry = session.entries().iter().find(|e| e.id == retry_id).unwrap();
        assert!(retry_entry.state.has_result(), "expected the retried query to actually finish");
        assert_eq!(session.cache.riemann_mixed.len(), 1, "expected the retry to have recomputed and populated the cache, not found it pre-poisoned or pre-populated");

        let mut clean_session = Session::new();
        clean_session.evaluate_definitions(REISSNER_NORDSTROM.to_string()).unwrap();
        let clean_id = clean_session.run_entry("kretschmann".to_string()).unwrap();
        let clean_entry = clean_session.entries().iter().find(|e| e.id == clean_id).unwrap();

        let EntryState::Done { result: retry_result, .. } = &retry_entry.state else { panic!("expected Done") };
        let EntryState::Done { result: clean_result, .. } = &clean_entry.state else { panic!("expected Done") };
        assert_eq!(retry_result.unicode, clean_result.unicode, "the post-cancellation session's answer must match a session that was never cancelled, exactly");
    }

    // ---------------------------------------------------------------
    // begin_query/finish_query -- the split `PendingQuery::run()` needs
    // to happen off whatever thread is holding `&mut Session` (Etapa
    // 3b, DESIGN-NOTEBOOK.md's cancellation section). Every test below
    // calls `.run()` inline, on the same thread, since these are about
    // the *split itself* (what's true immediately after `begin_query`,
    // before anything has run; what `finish_query` stores); the
    // cross-thread case is exactly what `mid_flight_cancellation_leaves_no_partial_cache_entry`
    // and `cancelling_a_non_reciprocal_metric_mid_flight_aborts_in_under_a_second`
    // (this file / `tests/cancellation.rs`) already cover for
    // `run_entry_with_context`, and `PendingQuery`'s own fields are the
    // same kinds of values (`Document`, `ComputeCache`, `Query`,
    // `Arc<ExecutionContext>`) already proven `Send` there.

    #[test]
    fn begin_query_shows_running_before_anything_has_actually_run() {
        let mut session = Session::new();
        session.evaluate_definitions(REISSNER_NORDSTROM.to_string()).unwrap();

        let QueryStart::Running(id, _pending) = session.begin_query("kretschmann".to_string()) else {
            panic!("expected QueryStart::Running");
        };
        let entry = session.entries().iter().find(|e| e.id == id).unwrap();
        assert!(matches!(entry.state, EntryState::Running), "expected Running immediately, before .run() was even called");
    }

    #[test]
    fn finish_query_stores_a_successful_result_and_restores_the_cache() {
        let mut session = Session::new();
        session.evaluate_definitions(REISSNER_NORDSTROM.to_string()).unwrap();

        let QueryStart::Running(id, pending) = session.begin_query("kretschmann".to_string()) else {
            panic!("expected QueryStart::Running");
        };
        assert_eq!(session.cache.riemann_mixed.len(), 0, "cache was taken by begin_query -- nothing left behind while a query is out");

        let result = pending.run();
        let was_cancelled = session.finish_query(id, result);

        assert!(!was_cancelled);
        let entry = session.entries().iter().find(|e| e.id == id).unwrap();
        assert!(entry.state.has_result());
        assert_eq!(session.cache.riemann_mixed.len(), 1, "expected the cache populated during .run() to have been restored, not discarded");
    }

    #[test]
    fn finish_query_reports_and_stores_a_genuine_cancellation() {
        let mut session = Session::new();
        session.evaluate_definitions(NON_RECIPROCAL_TWO_PARAM.to_string()).unwrap();

        let QueryStart::Running(id, pending) = session.begin_query("kretschmann".to_string()) else {
            panic!("expected QueryStart::Running");
        };
        let ctx = pending.context();
        let worker = std::thread::spawn(move || pending.run());
        std::thread::sleep(std::time::Duration::from_millis(300));
        ctx.cancel();
        let result = worker.join().expect("PendingQuery::run must catch its own cancellation unwind internally");

        let was_cancelled = session.finish_query(id, result);
        assert!(was_cancelled);
        let entry = session.entries().iter().find(|e| e.id == id).unwrap();
        assert!(matches!(entry.state, EntryState::Cancelled));
    }

    #[test]
    fn begin_query_with_no_document_yet_fails_synchronously_with_no_pending_work() {
        let mut session = Session::new();
        let QueryStart::Failed(id) = session.begin_query("kretschmann".to_string()) else {
            panic!("expected QueryStart::Failed -- nothing to run against without a Document");
        };
        let entry = session.entries().iter().find(|e| e.id == id).unwrap();
        assert!(matches!(&entry.state, EntryState::Failed { message, .. } if message.contains("no definitions")));
    }

    #[test]
    fn begin_query_with_unparseable_input_fails_synchronously_with_no_pending_work() {
        let mut session = Session::new();
        session.evaluate_definitions(REISSNER_NORDSTROM.to_string()).unwrap();
        let QueryStart::Failed(id) = session.begin_query("not a real query".to_string()) else {
            panic!("expected QueryStart::Failed -- input doesn't even parse as a query");
        };
        let entry = session.entries().iter().find(|e| e.id == id).unwrap();
        assert!(matches!(entry.state, EntryState::Failed { .. }));
    }

    /// The other reason `Document` needed to derive `Clone` (`Model`'s
    /// own doc comment): the answer a `PendingQuery` computes must match
    /// what it was a snapshot *of*, even if `Session`'s own live
    /// document moves on to a new generation while the query is still
    /// running -- never silently graded against whatever's current by
    /// the time `finish_query` runs.
    #[test]
    fn a_pending_query_runs_against_its_own_snapshot_even_if_the_session_moves_on_meanwhile() {
        let mut session = Session::new();
        session.evaluate_definitions(REISSNER_NORDSTROM.to_string()).unwrap();
        let snapshot_generation = session.document().unwrap().generation;

        let QueryStart::Running(id, pending) = session.begin_query("kretschmann".to_string()) else {
            panic!("expected QueryStart::Running");
        };

        // The live session moves on to a new generation before the
        // pending query is ever run.
        let edited = REISSNER_NORDSTROM.replace("2*M/r", "3*M/r");
        session.evaluate_definitions(edited).unwrap();
        assert_ne!(session.document().unwrap().generation, snapshot_generation);

        let result = pending.run();
        session.finish_query(id, result);

        let entry = session.entries().iter().find(|e| e.id == id).unwrap();
        let EntryState::Done { as_of, .. } = &entry.state else { panic!("expected Done") };
        assert_eq!(*as_of, snapshot_generation, "the stored generation must be the one the query actually ran against, not whatever is live now");
    }
}

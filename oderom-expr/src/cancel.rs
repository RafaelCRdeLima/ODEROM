//! Deep cancellation: lets a background query abort from *inside*
//! `normalize()`/`poly_gcd`/the subresultant PRS loop, not only between
//! whole components. Found necessary the hard way -- a metric where
//! `g_tt`/`g_rr` are not reciprocal (see `canonical.rs`'s doc comment on
//! `normalize_via_rational_form` for the actual mechanism) can make a
//! *single* component's `normalize()` call itself run away, and the
//! per-component checkpoint in `oderom-components::curvature` never gets
//! reached because that one call never returns.
//!
//! Same shape as [`crate::poly::TrigRewriteSuppressor`]: a thread-local,
//! armed for a scope's lifetime by an RAII guard, restored on drop
//! (including on unwind). Correct today for the same reason that one is:
//! one query runs start to finish on one thread (see that type's own
//! doc comment on what would break this if that ever changes).
//!
//! # Why panic, not `Result`
//!
//! `normalize()` is called from deep inside `Expr`'s ordinary arithmetic
//! (`Add`/`Mul`/`Neg` impls, `diff`, `substitute`, ...) with no `Result`
//! anywhere in those signatures. Threading a `Result<Expr, Cancelled>`
//! through every one of those call sites, across the whole crate, to
//! serve one non-local-exit use case is a large, invasive change for a
//! narrow problem. A panic unwinds through arbitrary call depth without
//! touching any of those signatures.
//!
//! This *could* be confused with this crate's other use of `panic!`:
//! "an implementation assumption is violated" (subresultant PRS's exact-
//! division checks, `poly_gcd`'s recursion-depth `debug_assert`, ...) --
//! those are meant to crash loudly, never be swallowed. `run_cancellable`
//! keeps that guarantee: it downcasts the unwind payload, and resumes
//! (re-panics) anything that isn't specifically our own [`Cancelled`]
//! marker. A real bug still crashes exactly as loudly as before; only a
//! genuine cancellation is caught.

use oderom_core::CancelToken;
use std::cell::RefCell;
use std::panic::{self, AssertUnwindSafe};

// `run_cancellable` below depends on `catch_unwind` actually catching --
// under `panic = "abort"` (a real, common `[profile.release]` setting;
// Cargo requires one strategy for the whole dependency graph, so this
// also catches it if set anywhere upstream, e.g. a binary crate's own
// Cargo.toml, not only this workspace's), a cancellation panic aborts
// the whole process instead of unwinding to here -- silently, until the
// day someone presses Ctrl+C and the REPL (or any caller) dies instead
// of cancelling. Caught at compile time instead of relying on anyone
// remembering this file exists.
#[cfg(panic = "abort")]
compile_error!(
    "oderom-expr's cancellation (see cancel.rs) requires catch_unwind, which does not catch under `panic = \"abort\"`. \
     Do not set `panic = \"abort\"` in any profile that builds this crate -- see the comment in the workspace Cargo.toml's [profile.release]."
);

thread_local! {
    static CANCEL_TOKEN: RefCell<Option<CancelToken>> = const { RefCell::new(None) };
}

/// Unwind payload used exclusively for cancellation -- never constructed
/// or matched on outside this module. The private field keeps it
/// impossible to build (and thus impossible to fake a cancellation
/// unwind) from outside.
pub struct Cancelled(());

/// RAII guard: arms the thread-local cancellation token for its
/// lifetime, restores the previous value on drop -- nested scopes (a
/// query calling into another cancellable computation) stay correct,
/// same as `TrigRewriteSuppressor`.
struct CancellationScope {
    previous: Option<CancelToken>,
}

impl CancellationScope {
    fn new(token: CancelToken) -> Self {
        let previous = CANCEL_TOKEN.with(|c| c.replace(Some(token)));
        CancellationScope { previous }
    }
}

impl Drop for CancellationScope {
    fn drop(&mut self) {
        CANCEL_TOKEN.with(|c| *c.borrow_mut() = self.previous.take());
    }
}

/// Checked at the specific hot loops named in the session log:
/// `normalize_via_rational_form`'s entry, `poly_gcd_bounded`'s recursive
/// descent, and the subresultant PRS's own iteration
/// (`rational_function.rs`'s `gcd`). A no-op (does not even touch the
/// thread-local's `Option`'s inner value) when no scope is currently
/// armed -- e.g. tests that call `normalize()` directly, outside
/// `run_cancellable`, are unaffected.
///
/// Overhead measured, not estimated (`tests::check_cancelled_overhead_is_negligible`
/// below, release build): 1.44ns/call over 2,000,000 calls -- an
/// uncontended thread-local access plus one `Relaxed` atomic load,
/// negligible next to a single polynomial multiplication, let alone a
/// whole PRS iteration.
pub(crate) fn check_cancelled() {
    let cancelled = CANCEL_TOKEN.with(|c| match c.borrow().as_ref() {
        Some(token) => token.is_cancelled(),
        None => false,
    });
    if cancelled {
        panic::panic_any(Cancelled(()));
    }
}

/// Runs `f` with `token` armed as the thread-local every `check_cancelled`
/// call below sees, converting a `Cancelled` unwind into `Err`. Any other
/// panic (a genuine invariant violation -- see module docs) is resumed
/// unchanged: this must never turn a real bug into a false "cancelled".
pub fn run_cancellable<T>(token: CancelToken, f: impl FnOnce() -> T) -> Result<T, Cancelled> {
    let _scope = CancellationScope::new(token);
    match panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(v) => Ok(v),
        Err(payload) => match payload.downcast::<Cancelled>() {
            Ok(cancelled) => Err(*cancelled),
            Err(payload) => panic::resume_unwind(payload),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unarmed_scope_never_cancels() {
        // No `run_cancellable` on the stack -- `check_cancelled` must be
        // a true no-op, not panic because there's nothing to ask.
        check_cancelled();
        check_cancelled();
    }

    #[test]
    fn cancelling_the_token_aborts_and_is_reported_as_cancelled() {
        let token = CancelToken::new();
        token.cancel();
        let result = run_cancellable(token, || {
            check_cancelled();
            unreachable!("check_cancelled should have unwound before this runs");
        });
        assert!(result.is_err());
    }

    #[test]
    fn an_uncancelled_token_lets_the_computation_finish_normally() {
        let token = CancelToken::new();
        let result = run_cancellable(token, || {
            check_cancelled();
            42
        });
        assert_eq!(result.ok(), Some(42));
    }

    #[test]
    #[should_panic(expected = "a genuine bug, not a cancellation")]
    fn a_real_panic_inside_run_cancellable_is_not_swallowed() {
        let token = CancelToken::new();
        let _ = run_cancellable(token, || {
            panic!("a genuine bug, not a cancellation");
        });
    }

    /// Measured, not estimated, per this project's own convention
    /// (`ODEROM_TRACE_SUBRES`, `diagnostic_cancel_latency.rs`, ...):
    /// an uncancelled `check_cancelled()` is a thread-local lookup plus
    /// one `Relaxed` atomic load, called once per subresultant PRS
    /// iteration / `poly_gcd_bounded` recursion / `normalize()` entry --
    /// all of which do orders of magnitude more work than that per call,
    /// so this asserts a generous upper bound (catches a regression like
    /// an accidental lock or allocation creeping in) rather than pinning
    /// an exact number that would be flaky across machines.
    #[test]
    fn check_cancelled_overhead_is_negligible() {
        let token = CancelToken::new();
        const N: u32 = 2_000_000;
        let elapsed = run_cancellable(token, || {
            let start = std::time::Instant::now();
            for _ in 0..N {
                check_cancelled();
            }
            start.elapsed()
        })
        .ok()
        .unwrap();
        let per_call_ns = elapsed.as_nanos() as f64 / N as f64;
        eprintln!("check_cancelled: {per_call_ns:.2}ns/call over {N} calls ({elapsed:?} total)");
        assert!(per_call_ns < 100.0, "check_cancelled got unexpectedly expensive: {per_call_ns:.2}ns/call");
    }

    #[test]
    fn nested_scopes_restore_the_outer_token_on_exit() {
        let outer = CancelToken::new();
        let inner = CancelToken::new();
        inner.cancel(); // outer stays live
        let outer_result = run_cancellable(outer.clone(), || {
            let inner_result = run_cancellable(inner, || {
                check_cancelled();
                unreachable!();
            });
            assert!(inner_result.is_err());
            // Back under `outer`'s scope now, which is not cancelled.
            check_cancelled();
            "outer finished"
        });
        assert_eq!(outer_result.ok(), Some("outer finished"));
    }
}

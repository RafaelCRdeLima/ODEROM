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
// Só o caminho com unwind usa `panic`/`AssertUnwindSafe`; em wasm32 as
// variantes abaixo não os tocam, e o crate compila com `warnings = deny`.
#[cfg(not(target_arch = "wasm32"))]
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
//
// A exceção é `wasm32`: aquele alvo é `panic = "abort"` por construção,
// e não há como desligar isso. Lá a cancelação profunda simplesmente não
// existe (ver `check_cancelled`/`run_cancellable` abaixo, na variante
// `wasm32`), e o navegador cancela encerrando o Web Worker que hospeda o
// módulo -- o que mata a computação de forma bem mais definitiva que um
// unwind. Manter o `compile_error!` aqui tornaria o alvo inatingível
// para proteger uma garantia que, lá, é dada por outro mecanismo.
#[cfg(all(panic = "abort", not(target_arch = "wasm32")))]
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
#[cfg(target_arch = "wasm32")]
pub(crate) fn check_cancelled() {
    // Sem unwind neste alvo: um `panic_any` aqui abortaria o módulo
    // inteiro em vez de voltar a `run_cancellable`, o que derrubaria a
    // aba em vez de cancelar a consulta. Ver a nota na guarda acima.
}

#[cfg(not(target_arch = "wasm32"))]
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
#[cfg(target_arch = "wasm32")]
pub fn run_cancellable<T>(token: CancelToken, f: impl FnOnce() -> T) -> Result<T, Cancelled> {
    // O token continua armado -- quem o consultar diretamente (os
    // checkpoints por componente em `oderom-components`) segue vendo o
    // pedido de cancelamento. O que não existe aqui é a saída não-local
    // de dentro de `normalize()`: ela dependia de unwind.
    let _scope = CancellationScope::new(token);
    Ok(f())
}

#[cfg(not(target_arch = "wasm32"))]
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

    /// A cancelação em wasm é dada por encerrar o Web Worker, não por
    /// unwind -- e é por isso que `check_cancelled` é um no-op lá. Este
    /// teste guarda a metade nativa da promessa: no alvo com unwind, a
    /// cancelação profunda tem de continuar funcionando. Se alguém
    /// generalizar o `#[cfg]` de wasm para todos os alvos "para
    /// simplificar", isto falha imediatamente.
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn deep_cancellation_still_unwinds_on_targets_that_have_unwinding() {
        let token = CancelToken::new();
        token.cancel();
        let result = run_cancellable(token, || {
            check_cancelled();
            unreachable!("check_cancelled deveria ter desenrolado a pilha");
        });
        assert!(result.is_err(), "cancelação profunda deixou de funcionar");
    }

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

    /// `check_cancelled()` must stay a thread-local lookup plus one
    /// `Relaxed` atomic load -- the regression this guards against is a
    /// lock or an allocation creeping into a function called once per
    /// subresultant-PRS iteration, per `poly_gcd_bounded` recursion and
    /// per `normalize()` entry.
    ///
    /// Asserted as a **ratio against a bare `Relaxed` load measured in
    /// the same loop on the same machine**, not as an absolute
    /// nanosecond bound. The previous version asserted `< 100ns/call`
    /// and described the margin as "orders of magnitude"; measured, it
    /// was ~46ns/call in a debug build, i.e. barely 2x, and it failed
    /// outright inside a full `cargo test --workspace` run under load --
    /// reporting machine contention as a code defect. That is the third
    /// absolute-time threshold in this codebase to do so.
    ///
    /// A ratio is the right instrument *here* specifically because this
    /// is a tight-loop microbenchmark, so it genuinely is throughput-
    /// bound and the baseline scales with whatever is slowing the
    /// machine. That reasoning does not transfer to every timing test:
    /// `oderom-session`'s cancellation-latency test was tried this way
    /// and it did not work, because latency there depends on scheduler
    /// contention rather than throughput (see its own comment).
    #[test]
    fn check_cancelled_overhead_is_negligible() {
        use std::sync::atomic::{AtomicBool, Ordering};
        const N: u32 = 2_000_000;

        // Baseline: the bare atomic load `check_cancelled` is allowed to
        // cost, with no thread-local access around it.
        let baseline_flag = AtomicBool::new(false);
        let start = std::time::Instant::now();
        for _ in 0..N {
            std::hint::black_box(baseline_flag.load(Ordering::Relaxed));
        }
        let baseline = start.elapsed().as_nanos() as f64 / N as f64;

        let token = CancelToken::new();
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

        // Generous: a lock or an allocation would be one to two orders
        // of magnitude over a plain relaxed load, so this catches the
        // regression while leaving room for the thread-local access and
        // for measurement noise.
        const MAX_RATIO: f64 = 25.0;
        let ratio = per_call_ns / baseline.max(0.05);
        eprintln!("check_cancelled: {per_call_ns:.2}ns/call, baseline load {baseline:.2}ns/call, ratio {ratio:.1}x (limit {MAX_RATIO}x)");
        assert!(
            ratio < MAX_RATIO,
            "check_cancelled got unexpectedly expensive: {per_call_ns:.2}ns/call against a {baseline:.2}ns/call bare atomic load ({ratio:.1}x) -- a lock or allocation likely crept in"
        );
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

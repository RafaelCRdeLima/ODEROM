//! Diagnostic (not an acceptance test): direct, first-hand proof that
//! cancelling a running query's worker thread makes that *actual OS
//! thread* terminate -- not just that `Notebook::finish_query` updates
//! the block's displayed state. A `JoinHandle` is the only thing that
//! can answer this authoritatively: `is_finished()`/`.join()` reflect
//! the real thread's own exit, not anything this crate or `oderom-app`
//! reports about itself.
//!
//! Run with:
//! ```text
//! cargo test -p oderom-notebook --test diagnostic_thread_termination -- --ignored --nocapture
//! ```
//! `--ignored`: this is a real, multi-second run against a metric that
//! never finishes on its own (same reasoning as
//! `oderom-components/tests/diagnostic_cancel_latency.rs`'s own
//! `--ignored` diagnostics) -- not something `cargo test` should run by
//! default.

use oderom_notebook::{BeginExecution, Notebook};
use std::time::{Duration, Instant};

/// Same non-reciprocal shape as `oderom-session/tests/cancellation.rs`
/// and `oderom-notebook/src/notebook.rs`'s own cancellation tests --
/// this project's standing non-terminating-computation fixture, found
/// from the real freeze this whole feature exists to fix.
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

#[test]
#[ignore]
fn cancelling_makes_the_real_worker_thread_terminate() {
    let mut nb = Notebook::new();
    let a = nb.create_block_after(None, NON_RECIPROCAL.to_string());
    nb.execute_block(a); // fast -- declaration reconstruction only
    let q = nb.create_block_after(Some(a), "kretschmann".to_string());

    let BeginExecution::Started(pending) = nb.begin_execute(q) else { panic!("expected Started") };
    let ctx = pending.context();

    let handle = std::thread::spawn(move || pending.run());

    // Give it real time inside the stage that used to hang for 60+s --
    // same 300ms `oderom-session/tests/cancellation.rs` itself uses --
    // long enough to be genuinely stuck deep inside a single
    // component's `normalize()` call, not just between components.
    std::thread::sleep(Duration::from_millis(300));

    // The direct, load-bearing assertion this diagnostic exists for:
    // before cancelling, the real OS thread is provably still alive.
    // Without this, "it terminated after cancel" would be unfalsifiable
    // (a thread that was never actually running "terminates" trivially).
    assert!(!handle.is_finished(), "expected the worker thread to still be genuinely running before cancellation -- if this fails, the diagnostic below would prove nothing");

    let cancel_requested_at = Instant::now();
    ctx.cancel();

    // Poll `is_finished()` (never `.join()` directly -- a bare `.join()`
    // would block this test indefinitely if the thread never actually
    // exits, which is exactly the failure mode being checked for)
    // bounded by a generous deadline; record how long it actually took.
    let deadline = cancel_requested_at + Duration::from_secs(10);
    while !handle.is_finished() {
        assert!(Instant::now() < deadline, "worker thread did not terminate within 10s of cancellation -- this is the orphan-thread failure mode this diagnostic exists to catch");
        std::thread::sleep(Duration::from_millis(5));
    }
    let observed_latency = cancel_requested_at.elapsed();

    // `.join()` now -- guaranteed to return immediately (`is_finished()`
    // just confirmed it), but this is what actually reclaims the OS
    // thread and proves it did not merely *report* finished while still
    // technically alive.
    let result = handle.join().expect("PendingQuery::run must catch its own cancellation unwind internally, never let it escape the worker thread");

    eprintln!("worker thread terminated {observed_latency:?} after cancel() was called (bound: per-loop-iteration checkpoints inside normalize()/poly_gcd, not just between tensor components -- see DESIGN-NOTEBOOK.md section 10.7 and oderom-expr/src/cancel.rs)");
    assert!(observed_latency < Duration::from_secs(2), "expected sub-2s termination (the deep, inside-normalize() checkpoints exist specifically so this never approaches the 10s bound) -- got {observed_latency:?}");

    // Finally, feed the result back and confirm the notebook layer
    // agrees with what the thread itself just proved.
    nb.finish_query(q, result);
    assert!(matches!(
        nb.block(q).unwrap().output,
        oderom_notebook::BlockOutput::Attempt { .. }
    ));
}

#[test]
#[ignore]
fn without_cancelling_the_worker_thread_never_terminates_on_its_own() {
    // The negative control: proves `NON_RECIPROCAL` really is
    // non-terminating in this test's own timeframe, not merely slow --
    // without this, "cancelling makes it terminate" could be trivially
    // true of a computation that was about to finish on its own anyway.
    let mut nb = Notebook::new();
    let a = nb.create_block_after(None, NON_RECIPROCAL.to_string());
    nb.execute_block(a);
    let q = nb.create_block_after(Some(a), "kretschmann".to_string());

    let BeginExecution::Started(pending) = nb.begin_execute(q) else { panic!("expected Started") };
    let handle = std::thread::spawn(move || pending.run());

    std::thread::sleep(Duration::from_secs(5));
    assert!(!handle.is_finished(), "expected NON_RECIPROCAL to still be running on its own after 5s with no cancellation -- if it already finished, it is not exercising the non-terminating case the other diagnostic in this file depends on");

    // Leaked deliberately (never cancelled, never joined) -- this test's
    // only job is the negative-control assertion above; letting the
    // process exit without waiting out a non-terminating computation is
    // the point, not an oversight.
    std::mem::forget(handle);
}

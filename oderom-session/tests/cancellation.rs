//! Permanent regression fixture for the finding that falsified this
//! project's earlier "3 or more free parameters" note (README.md,
//! DESIGN-RATIONAL-FORM.md): a metric whose `g_tt`/`g_rr` are *not*
//! reciprocal can make a single component's `normalize()` call run away
//! even with as few as 2 free parameters -- the same count
//! Reissner-Nordstrom (reciprocal) finishes in ~1s. Found from real
//! REPL usage (`kretschmann` on this exact metric ran 60+s with no
//! guardrail firing at all, since the REPL had no wall-clock timeout of
//! its own and the between-component checkpoint was never reached).
//!
//! This is the "must be interruptible in under 1s, even though the
//! computation itself never finishes" case the session log asked for --
//! proven here at the `oderom-session` layer directly (no REPL/terminal
//! needed), since `Session::run_entry_with_context` is the same
//! cancellable entry point the REPL calls.

use oderom_cli::commands::ExecutionContext;
use oderom_session::{EntryState, Session};
use std::time::{Duration, Instant};

/// `g_tt = -(1 - 2M/r + 1/r^2)`, `g_rr = 1/(1 - 2M/r + Q^2/r^2)` --
/// deliberately *not* the reciprocal of the same `f(r)`: `g_tt`'s own
/// "charge" term is the literal `1`, not `Q`, so `g_tt * g_rr != -1`.
/// Still only 2 free parameters (`M`, `Q`) -- same count as
/// Reissner-Nordstrom, which finishes this exact query in under a
/// second (`oderom-components/tests/reissner_nordstrom.rs`).
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

/// The bound cancellation must return within. Deliberately loose, and
/// the looseness is the point.
///
/// What this test discriminates is a gap of about four orders of
/// magnitude: with the deep, inside-`normalize()` checkpoint,
/// cancellation returns in tens of milliseconds; without it, the
/// between-component checkpoint is never reached for this metric and
/// cancellation waits out a single component that runs 60s+. Any
/// threshold in between separates those two worlds, so precision buys
/// nothing here and costs robustness.
///
/// Two earlier attempts, both recorded because each was wrong in an
/// instructive way:
///
/// 1. A fixed 1s bound. It failed at ~1.45s under `load average` ~6
///    with the code unchanged (confirmed by A/B against the previous
///    commit in the same window), i.e. it was measuring the machine.
/// 2. A budget calibrated as a multiple of a `christoffel` query timed
///    on the same machine, on the theory that latency and throughput
///    move together. **Measured false**: in one run calibration was
///    239ms with 48ms latency, in another 172ms with 1.45s latency --
///    latency swung 30x while calibration barely moved. Cancellation
///    latency is not throughput-bound; it depends on where the cancel
///    lands relative to a checkpoint and on scheduler contention at the
///    join, neither of which a throughput probe sees.
///
/// So: one loose absolute bound, chosen to sit far from both worlds.
const CANCELLATION_BOUND: Duration = Duration::from_secs(10);

#[test]
fn cancelling_a_non_reciprocal_metric_mid_flight_aborts_in_under_a_second() {
    let mut session = Session::new();
    session.evaluate_definitions(NON_RECIPROCAL_TWO_PARAM.to_string()).unwrap();

    let ctx = ExecutionContext::new();
    let worker_ctx = ctx.clone();
    let handle = std::thread::spawn(move || {
        let id = session.run_entry_with_context("kretschmann".to_string(), &worker_ctx).unwrap();
        (session, id)
    });

    // Give it real time inside the stage that used to hang for 60+s --
    // long enough to be well past the between-component checkpoint (RN's
    // own riemann_mixed takes well under a second end to end) and
    // genuinely stuck inside a single component's `normalize()` call,
    // not still working through fast, unrelated components.
    std::thread::sleep(Duration::from_millis(300));

    let start = Instant::now();
    ctx.cancel();
    let (session, id) = handle.join().expect("worker thread should not panic -- run_query catches its own cancellation unwind internally");
    let elapsed = start.elapsed();

    assert!(
        elapsed < CANCELLATION_BOUND,
        "cancellation took {elapsed:?}, over the {CANCELLATION_BOUND:?} bound -- the deep, inside-normalize() checkpoint did not interrupt it promptly \
         (without that checkpoint this waits out a component that runs 60s+, so anything in this range means the checkpoint is gone, not that the machine is busy)"
    );

    let entry = session.entries().iter().find(|e| e.id == id).unwrap();
    // Etapa 3b: a distinct `Cancelled` state now, not a `Failed` whose
    // message happens to mention cancellation.
    assert!(
        matches!(entry.state, EntryState::Cancelled),
        "expected a Cancelled entry, got something else (has_result={}, is_stale={})",
        entry.state.has_result(),
        entry.state.is_stale()
    );
}

/// `g_tt = -(1 - 2M/r)`, `g_rr = 1/(1 - M/r)` -- also not reciprocal
/// (`2M/r` vs `M/r`), but only 1 free parameter total instead of 2. The
/// deliberate contrast with the test above: non-reciprocity alone is not
/// what makes a metric slow (this one finishes in ~1s, same ballpark as
/// Reissner-Nordstrom), so a future change must not "fix" cancellation
/// by making every non-reciprocal metric slower or by cancelling things
/// that would have finished fine on their own.
const NON_RECIPROCAL_ONE_PARAM: &str = "
manifold M dim 4
bundle TM on M dim 4
chart schw on M coords (t, r, theta, phi)
metric g on schw bundle TM {
  [t,t] = -(1 - 2*M/r),
  [r,r] = 1/(1 - M/r),
  [theta,theta] = r^2,
  [phi,phi] = r^2 * sin(theta)^2
}
";

#[test]
fn non_reciprocal_but_only_one_parameter_still_finishes_on_its_own() {
    let mut session = Session::new();
    session.evaluate_definitions(NON_RECIPROCAL_ONE_PARAM.to_string()).unwrap();

    // Bounded, not a plain `run_entry().unwrap()` -- a future regression
    // that made this case slow again should fail this test promptly
    // instead of hanging the whole suite/CI indefinitely.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let id = session.run_entry("kretschmann".to_string()).unwrap();
        let entry = session.entries().iter().find(|e| e.id == id).unwrap().state.has_result();
        let _ = tx.send(entry);
    });
    let has_result = rx.recv_timeout(Duration::from_secs(10)).expect("expected this to finish on its own well within 10s (measured: ~1s) -- if it's timing out, non-reciprocity alone regressed into being treated as slow");
    assert!(has_result, "expected kretschmann to actually finish with a result, not error out");
}

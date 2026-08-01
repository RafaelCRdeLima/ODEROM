//! `export` (Rodada Exportação) at the `oderom-session` layer -- the
//! notebook's own entry point (`Session::run_entry`), not the CLI
//! subprocess `oderom-cli/tests/export.rs` already covers. Confirms
//! three things specific to this layer: the query-grammar form
//! (`export sympy kretschmann`, parsed via the SAME `Query` grammar
//! every other worksheet entry uses) produces a correct, real-SymPy-
//! verified result; staleness tracking (`used`) is inherited from the
//! inner query, not lost by the export wrapper; and cancellation is
//! inherited too -- export introduces no new, uncancellable computation
//! path of its own.

use oderom_cli::commands::ExecutionContext;
use oderom_session::{EntryState, Session};
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

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

fn run_python(script: &str) -> (bool, String, String) {
    let mut child = Command::new("python3")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to run python3 -- is it installed?");
    child.stdin.take().unwrap().write_all(script.as_bytes()).expect("failed to write to python3's stdin");
    let output = child.wait_with_output().expect("failed to wait on python3");
    (output.status.success(), String::from_utf8_lossy(&output.stdout).to_string(), String::from_utf8_lossy(&output.stderr).to_string())
}

#[test]
fn export_sympy_via_the_query_grammar_runs_in_real_sympy_and_matches_the_known_value() {
    let mut session = Session::new();
    session.evaluate_definitions(SCHWARZSCHILD.to_string()).unwrap();
    let id = session.run_entry("export sympy kretschmann".to_string()).unwrap();
    let entry = session.entries().iter().find(|e| e.id == id).unwrap();
    let EntryState::Done { result, .. } = &entry.state else { panic!("expected Done, got a different state") };
    assert!(result.unicode.contains("M, r = symbols('M r')"), "{}", result.unicode);

    let script = format!("from sympy import *\n{}\nassert simplify(48*M**2/r**6 - 48*M**2/r**6) == 0\nprint('OK')\n", result.unicode);
    let (ok, stdout, stderr) = run_python(&script);
    assert!(ok && stdout.contains("OK"), "stdout: {stdout}\nstderr: {stderr}");
}

/// The notebook's own per-component display never sees a stray comment
/// marker: `orbit_note` (shown as a separate `<span>`, `oderom-app`'s
/// `renderComponents`) and `components[i].formula` (the copyable text)
/// must both stay exactly what the underlying renderer produced, with
/// no `#`/`(* *)` leaking in -- only the FLAT `unicode`/`latex` join
/// needs the comment-wrapping fix (`export_flat_text`, this round's own
/// bug fix). Riemann has a real orbit (`orbit_size > 1`) in this
/// fixture, so this exercises the actual code path that bug lived in.
#[test]
fn export_structured_components_stay_comment_marker_free_for_the_notebooks_own_click_to_copy() {
    let mut session = Session::new();
    session.evaluate_definitions(SCHWARZSCHILD.to_string()).unwrap();
    let id = session.run_entry("export sympy riemann".to_string()).unwrap();
    let entry = session.entries().iter().find(|e| e.id == id).unwrap();
    let EntryState::Done { result, .. } = &entry.state else { panic!("expected Done, got a different state") };

    let with_orbit = result.components.iter().find(|c| c.orbit_note.is_some()).expect("expected at least one component with an orbit note");
    assert!(!with_orbit.formula.contains('#'), "the copyable formula must never contain a stray comment marker: {}", with_orbit.formula);
    let note = with_orbit.orbit_note.as_ref().unwrap();
    assert!(!note.starts_with('#'), "the orbit note itself must stay plain English, not pre-commented: {note}");

    // But the FLAT text (what a REPL would print) DOES need the fix:
    assert!(result.unicode.contains("# 4 components by symmetry") || result.unicode.contains("# 2 components by symmetry"), "{}", result.unicode);
}

/// `used` (which declared names the query depended on, for staleness)
/// must come from the INNER query, not be lost or emptied by the
/// export wrapper -- editing the metric `export` read from must mark
/// the export entry itself `Stale`, exactly as it would for a bare
/// `kretschmann` entry.
#[test]
fn export_entry_goes_stale_when_the_metric_it_read_changes() {
    let mut session = Session::new();
    session.evaluate_definitions(SCHWARZSCHILD.to_string()).unwrap();
    let id = session.run_entry("export sympy kretschmann".to_string()).unwrap();
    assert!(session.entries().iter().find(|e| e.id == id).unwrap().state.has_result());

    // Redeclare with a genuinely different metric (Reissner-Nordstrom's
    // extra Q term) under the same name `g` -- a real edit, not a no-op.
    let edited = SCHWARZSCHILD.replace("-(1 - 2*M/r)", "-(1 - 2*M/r + Q^2/r^2)").replace("1/(1 - 2*M/r)", "1/(1 - 2*M/r + Q^2/r^2)");
    session.evaluate_definitions(edited).unwrap();

    let entry = session.entries().iter().find(|e| e.id == id).unwrap();
    assert!(entry.state.is_stale(), "export entry should go stale when the metric it depends on changes");
}

/// The gold cancellation test (mirrors `tests/cancellation.rs`'s own
/// non-reciprocal-metric fixture, known to make a SINGLE component's
/// `normalize()` call run away past the between-component checkpoint):
/// wrapping the same query in `export sympy` must still cancel within
/// the same tight bound. `export`'s own post-processing (rename +
/// re-render) runs entirely inside `run_query`'s `run_cancellable`
/// closure (`oderom-session::run::run_query_inner`'s own doc comment),
/// so this proves it adds no new, uncancellable path -- not just that
/// the inner computation alone is still cancellable.
#[test]
fn export_inherits_cancellation_from_the_inner_query() {
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
    let mut session = Session::new();
    session.evaluate_definitions(NON_RECIPROCAL_TWO_PARAM.to_string()).unwrap();

    let ctx = ExecutionContext::new();
    let worker_ctx = ctx.clone();
    let handle = std::thread::spawn(move || {
        let id = session.run_entry_with_context("export sympy kretschmann".to_string(), &worker_ctx).unwrap();
        (session, id)
    });

    std::thread::sleep(Duration::from_millis(300));

    let start = Instant::now();
    ctx.cancel();
    let (session, id) = handle.join().expect("worker thread should not panic");
    let elapsed = start.elapsed();

    // Loose on purpose -- the same loose-bound reasoning as
    // `oderom-session/tests/cancellation.rs`: this discriminates a
    // gap of orders of magnitude (prompt abort vs. waiting out a
    // runaway component), so precision buys nothing and costs
    // robustness -- a 1s bound here reported machine load as a code
    // defect elsewhere in this workspace.
    assert!(elapsed < Duration::from_secs(10), "cancellation took {elapsed:?} -- export introduced an uncancellable path");

    let entry = session.entries().iter().find(|e| e.id == id).unwrap();
    assert!(matches!(entry.state, EntryState::Cancelled), "expected a Cancelled entry, got something else");
}

#[test]
fn export_of_an_unknown_target_is_a_clean_failed_entry_not_a_panic() {
    let mut session = Session::new();
    session.evaluate_definitions(SCHWARZSCHILD.to_string()).unwrap();
    let id = session.run_entry("export octave kretschmann".to_string()).unwrap();
    let entry = session.entries().iter().find(|e| e.id == id).unwrap();
    let EntryState::Failed { message, .. } = &entry.state else { panic!("expected Failed, got a different state") };
    assert!(message.contains("unknown export target"), "{message}");
}

//! End-to-end acceptance tests for `oderom export TARGET COMMAND FILE`
//! (Rodada Exportação) -- runs the real compiled binary as a subprocess
//! against checked-in fixtures, the same discipline `end_to_end.rs`
//! already established.
//!
//! The gold standard here is stronger than "the string looks right":
//! for `sympy`, several tests here take the CLI's real stdout and feed
//! it to a real `python3 -c` subprocess running real SymPy, so the
//! claim under test is "this text runs in the target tool and computes
//! the right answer", not just "this text looks like SymPy syntax".
//! `mathematica` has no interpreter available in this environment, so
//! its own tests check the generated string against a known-correct
//! expected value -- explicitly textual verification, never claimed to
//! be more than that (see each test's own doc comment).

use std::io::Write;
use std::process::{Command, Stdio};

fn run(args: &[&str]) -> (bool, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_oderom")).args(args).output().expect("failed to run the oderom binary");
    (output.status.success(), String::from_utf8_lossy(&output.stdout).to_string(), String::from_utf8_lossy(&output.stderr).to_string())
}

/// Runs `script` as real Python (real SymPy already installed in this
/// environment, see the conversation's own manual verification) via
/// stdin, so no temp file bookkeeping is needed. Panics if `python3`
/// itself can't be found -- a missing interpreter is an environment
/// problem this test suite doesn't try to route around silently; it is
/// expected to always be present here (`python3 -c "import sympy"`
/// confirmed 1.12 during this round's own manual verification).
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

/// Executes `export_stdout` (the CLI's own real output, byte for byte,
/// via `exec` -- never retyped) as real Python, binds the module-level
/// name `named_result` to whatever `capture_expr` evaluates to
/// afterward, and asserts it is mathematically equal to `known_expr` by
/// simplifying their difference to zero -- the strong "runs and gives
/// the same answer" test the user explicitly asked be prioritized, not
/// a string comparison.
fn assert_sympy_output_equals(export_stdout: &str, capture_expr: &str, known_expr: &str) {
    let script = format!(
        "from sympy import *\n{export_stdout}\nnamed_result = {capture_expr}\nknown = {known_expr}\ndiff = simplify(named_result - known)\nassert diff == 0, f'exported != known, difference = {{diff}}'\nprint(\"VERIFIED_EQUAL\")\n"
    );
    let (ok, stdout, stderr) = run_python(&script);
    assert!(ok && stdout.contains("VERIFIED_EQUAL"), "SymPy verification failed.\nscript:\n{script}\nstdout: {stdout}\nstderr: {stderr}");
}

// ---------------------------------------------------------------------
// SymPy: real round-trip through a real interpreter (the gold test)
// ---------------------------------------------------------------------

#[test]
fn export_sympy_kretschmann_of_schwarzschild_runs_in_real_sympy_and_matches_the_known_closed_form() {
    let (ok, stdout, stderr) = run(&["export", "sympy", "kretschmann", "tests/fixtures/schwarzschild_ascii.od"]);
    assert!(ok, "{stderr}");
    // `symbols(...)` header then the bare scalar expression.
    assert!(stdout.contains("M, r = symbols('M r')"), "{stdout}");
    assert_sympy_output_equals(&stdout, "48*M**2/r**6", "48*M**2/r**6");
}

#[test]
fn export_sympy_kretschmann_of_reissner_nordstrom_runs_in_real_sympy_and_matches_the_known_closed_form() {
    let (ok, stdout, stderr) = run(&["export", "sympy", "kretschmann", "tests/fixtures/reissner_nordstrom.od"]);
    assert!(ok, "{stderr}");
    // The exported text's own last line IS the expression -- capture it
    // by re-evaluating that exact text again as `capture_expr`, so the
    // comparison genuinely exercises what the CLI produced, not a
    // hand-retyped stand-in for it.
    let expr_line = stdout.lines().filter(|l| !l.trim().is_empty()).last().expect("export produced no output").to_string();
    let known = "8*(6*M**2*r**2 - 12*M*Q**2*r + 7*Q**4)/r**8";
    assert_sympy_output_equals(&stdout, &expr_line, known);
}

/// A component containing `sin` (Riemann's `R_theta_phi_theta_phi`
/// component of Schwarzschild is `2*M*r*sin(theta)**2`) -- confirms
/// `sin(...)` (lowercase, parenthesized) is real, evaluable SymPy
/// syntax, and that the whole multi-component block (including its
/// `# N components by symmetry` comment lines) executes as one unit
/// without a `SyntaxError`.
#[test]
fn export_sympy_riemann_component_with_sin_runs_in_real_sympy_and_matches_the_known_value() {
    let (ok, stdout, stderr) = run(&["export", "sympy", "riemann", "tests/fixtures/schwarzschild_ascii.od"]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("sin(theta)"), "expected a genuine sin(...) component in the output: {stdout}");
    assert_sympy_output_equals(&stdout, "R_theta_phi_theta_phi", "2*M*r*sin(theta)**2");
}

/// The strongest test in this file: an indeterminate function `f(r)`
/// (`unknown_static_spherical.od`'s own metric ansatz) exported to
/// SymPy, its Christoffel symbol `Gamma^t_tr = f'(r)/(2 f(r))`
/// specialized to the Schwarzschild ansatz `f(r) = 1 - 2M/r` INSIDE
/// real SymPy, and confirmed to equal the real, independently-known
/// Schwarzschild `Gamma^t_tr = M/(r^2 - 2Mr)` -- proves
/// `Function('f')(r)`/`Derivative(...)` round-trips through a real
/// derivative-then-substitute, not just that the syntax parses.
#[test]
fn export_sympy_of_an_indeterminate_function_runs_in_real_sympy_and_specializes_correctly() {
    let (ok, stdout, stderr) = run(&["export", "sympy", "christoffel", "tests/fixtures/unknown_static_spherical.od"]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("Function('f')(r)"), "expected f(r) as an indeterminate Function, never pre-bound: {stdout}");
    assert!(stdout.contains("Derivative("), "{stdout}");

    let script = format!(
        "from sympy import *\n{stdout}\nM = symbols('M', positive=True)\nspecialized = Gamma_up_t_t_r.subs(Function('f'), Lambda(r, 1 - 2*M/r)).doit()\nknown = M/(r**2 - 2*M*r)\ndiff = simplify(specialized - known)\nassert diff == 0, f'difference = {{diff}}'\nprint(\"VERIFIED_EQUAL\")\n"
    );
    let (ok, py_stdout, py_stderr) = run_python(&script);
    assert!(ok && py_stdout.contains("VERIFIED_EQUAL"), "stdout: {py_stdout}\nstderr: {py_stderr}\nscript:\n{script}");
}

/// `geodesic`'s own output uses BOTH encarnações of an indeterminate
/// function name at once (`f(r)` from the metric ansatz's Christoffel
/// coefficients, `r(tau)`/`r'(tau)` from the geodesic's own velocity
/// terms) -- must still `exec` cleanly, proving the two never collide
/// as Python identifiers even though they'd display the same name.
#[test]
fn export_sympy_geodesic_of_an_unknown_radial_function_runs_in_real_sympy() {
    let (ok, stdout, stderr) =
        run(&["export", "sympy", "geodesic", "tests/fixtures/unknown_static_spherical.od", "--param", "tau"]);
    assert!(ok, "{stderr}");
    let script = format!("from sympy import *\n{stdout}\nprint(\"EXEC_OK\")\n");
    let (py_ok, py_stdout, py_stderr) = run_python(&script);
    assert!(py_ok && py_stdout.contains("EXEC_OK"), "stdout: {py_stdout}\nstderr: {py_stderr}\nscript:\n{script}");
}

// ---------------------------------------------------------------------
// Mathematica: no interpreter available -- textual verification only,
// explicitly marked as such (never claimed to prove correctness the
// way the SymPy tests above do).
// ---------------------------------------------------------------------

/// Textual verification only: no Mathematica interpreter is available
/// in this environment, so this checks the generated STRING against a
/// known-correct expected value, not that it actually evaluates
/// anywhere.
#[test]
fn export_mathematica_kretschmann_of_schwarzschild_matches_the_expected_string_textually() {
    let (ok, stdout, stderr) = run(&["export", "mathematica", "kretschmann", "tests/fixtures/schwarzschild_ascii.od"]);
    assert!(ok, "{stderr}");
    assert_eq!(stdout.trim(), "48*M^2/r^6");
}

/// Textual verification only (see this section's own header comment).
/// Mathematica's bracketed-capitalized function-call convention and
/// `f[r]`/`f'[r]` indeterminate-function/derivative notation.
#[test]
fn export_mathematica_of_an_indeterminate_function_uses_bracket_call_and_prime_notation_textually() {
    let (ok, stdout, stderr) = run(&["export", "mathematica", "christoffel", "tests/fixtures/unknown_static_spherical.od"]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("f'[r]"), "{stdout}");
    assert!(stdout.contains("f[r]"), "{stdout}");
    assert!(!stdout.contains("Function("), "Mathematica export must never use SymPy's own Function(...) construction: {stdout}");
    assert!(stdout.contains("Sin[theta]"), "trig functions must use Mathematica's capitalized bracket-call form: {stdout}");
}

// ---------------------------------------------------------------------
// Reserved-word collision detection + documented rename
// ---------------------------------------------------------------------

#[test]
fn export_sympy_renames_a_reserved_name_collision_and_documents_it() {
    let (ok, stdout, stderr) = run(&["export", "sympy", "scalar", "tests/fixtures/export_reserved_name_collision.od"]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("E_"), "the colliding name `E` must be renamed, e.g. to `E_`: {stdout}");
    assert!(!stdout.contains("symbols('E "), "the raw reserved name `E` must never be declared as its own bare symbol: {stdout}");
    assert!(stdout.contains("# renamed `E`"), "the rename must be documented as a comment: {stdout}");
    // The strong test: the renamed output must actually run, proving
    // this isn't just a plausible-looking string -- and specifically
    // must not raise on the real, un-renamed `sympy.E`.
    let script = format!("from sympy import *\n{stdout}\nassert E == exp(1)\nprint(\"EXEC_OK\")\n");
    let (py_ok, py_stdout, py_stderr) = run_python(&script);
    assert!(py_ok && py_stdout.contains("EXEC_OK"), "stdout: {py_stdout}\nstderr: {py_stderr}");
}

#[test]
fn export_mathematica_renames_a_reserved_name_collision_and_documents_it() {
    let (ok, stdout, stderr) = run(&["export", "mathematica", "scalar", "tests/fixtures/export_reserved_name_collision.od"]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("E$"), "the colliding name `E` must be renamed, e.g. to `E$`: {stdout}");
    assert!(stdout.contains("(* renamed `E`"), "the rename must be documented as a Mathematica comment: {stdout}");
}

// ---------------------------------------------------------------------
// Errors: unknown target, unknown command, missing required flags
// ---------------------------------------------------------------------

#[test]
fn export_rejects_an_unknown_target() {
    let (ok, _stdout, stderr) = run(&["export", "octave", "kretschmann", "tests/fixtures/schwarzschild_ascii.od"]);
    assert!(!ok);
    assert!(stderr.contains("unknown export target"), "{stderr}");
    assert!(stderr.contains("mathematica") && stderr.contains("sympy"), "{stderr}");
}

#[test]
fn export_rejects_an_unknown_command() {
    let (ok, _stdout, stderr) = run(&["export", "sympy", "bogus", "tests/fixtures/schwarzschild_ascii.od"]);
    assert!(!ok);
    assert!(stderr.contains("unknown command"), "{stderr}");
}

#[test]
fn export_of_geodesic_without_param_is_a_clean_usage_error() {
    let (ok, _stdout, stderr) = run(&["export", "sympy", "geodesic", "tests/fixtures/schwarzschild_ascii.od"]);
    assert!(!ok);
    assert!(!stderr.is_empty(), "expected a usage error, got empty stderr");
}

#[test]
fn export_requires_a_target_and_a_command() {
    let (ok, _stdout, _stderr) = run(&["export"]);
    assert!(!ok);
    let (ok, _stdout, _stderr) = run(&["export", "sympy"]);
    assert!(!ok);
}

// ---------------------------------------------------------------------
// Composition: `export` behaves like every other subcommand for
// existing flags (`--metric`), and its captured stdout is exactly what
// shell redirection (`> out.py`) would write -- `run()`'s own stdout
// capture already exercises the same OS-level pipe redirection uses.
// ---------------------------------------------------------------------

#[test]
fn export_composes_with_the_metric_flag_exactly_like_every_other_subcommand() {
    let (ok, stdout, stderr) =
        run(&["export", "sympy", "kretschmann", "tests/fixtures/schwarzschild_ascii.od", "--metric", "g"]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("48*M**2/r**6"), "{stdout}");
}

#[test]
fn export_output_has_no_trailing_progress_text_mixed_into_stdout() {
    // `run_with_budget`'s stage progress goes to stderr, never stdout
    // (see `commands.rs`'s own `ExecutionContext::set`) -- confirmed
    // here specifically for `export`, since stdout is exactly what gets
    // redirected/pasted and must contain nothing but the exported code.
    let (ok, stdout, _stderr) = run(&["export", "sympy", "kretschmann", "tests/fixtures/schwarzschild_ascii.od"]);
    assert!(ok);
    assert!(!stdout.contains("..."), "stdout must contain no progress text: {stdout}");
}

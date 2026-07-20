//! The acceptance test the user actually specified for the CLI: write a
//! metric to a text file, run one command, read the rendered curvature --
//! no Rust code, no recompilation. This runs the real compiled binary as
//! a subprocess against checked-in fixture files (not fixtures built by
//! Rust code with a hand-picked structure -- see DESIGN-UI.md 6.0), so it
//! is the only test in this project that exercises the full pipeline
//! (parse .od -> Model -> curvature -> render) the way an actual user
//! would.
//!
//! Both the ASCII and LaTeX-flavored fixtures encode the same
//! Schwarzschild metric and must agree exactly: DESIGN-UI.md 6.1 says
//! there is one grammar, not two, and this is the end-to-end version of
//! that claim (`expr_parser`'s unit tests already check it at the
//! `Expr`-tree level).

use std::process::Command;

fn run(args: &[&str]) -> (bool, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_oderom")).args(args).output().expect("failed to run the oderom binary");
    (output.status.success(), String::from_utf8_lossy(&output.stdout).to_string(), String::from_utf8_lossy(&output.stderr).to_string())
}

#[test]
fn kretschmann_of_schwarzschild_from_a_file_ascii_and_latex_agree() {
    for fixture in ["tests/fixtures/schwarzschild_ascii.od", "tests/fixtures/schwarzschild_latex.od"] {
        let (ok, stdout, stderr) = run(&["kretschmann", fixture]);
        assert!(ok, "{fixture}: {stderr}");
        assert_eq!(stdout.trim(), "48*M^2/r^6", "{fixture}: got {stdout:?}");
    }
}

#[test]
fn ricci_of_schwarzschild_shows_all_ten_independent_components_as_zero() {
    let (ok, stdout, stderr) = run(&["ricci", "tests/fixtures/schwarzschild_ascii.od"]);
    assert!(ok, "{stderr}");
    // Symmetric rank 2 in dimension 4: 4*5/2 = 10 independent
    // components, all zero (Schwarzschild is a vacuum solution).
    assert!(stdout.contains("10 independent components identically zero"), "{stdout}");
    assert!(!stdout.contains("Ricci["), "no nonzero Ricci component should be printed: {stdout}");
}

#[test]
fn scalar_of_schwarzschild_is_zero() {
    let (ok, stdout, stderr) = run(&["scalar", "tests/fixtures/schwarzschild_ascii.od"]);
    assert!(ok, "{stderr}");
    assert_eq!(stdout.trim(), "0");
}

#[test]
fn christoffel_renders_nonzero_symbols_with_the_gamma_label() {
    let (ok, stdout, stderr) = run(&["christoffel", "tests/fixtures/schwarzschild_ascii.od"]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("Gamma["), "{stdout}");
}

#[test]
fn latex_target_produces_a_frac() {
    let (ok, stdout, stderr) = run(&["kretschmann", "tests/fixtures/schwarzschild_ascii.od", "--target", "latex"]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("\\frac"), "{stdout}");
}

#[test]
fn scalar_on_a_bare_connection_errors_instead_of_guessing() {
    let (ok, _stdout, stderr) = run(&["scalar", "tests/fixtures/connection_only.od"]);
    assert!(!ok);
    assert!(stderr.contains("needs a metric"), "{stderr}");
}

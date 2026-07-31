//! Fase 2, item 2.4: the two reduction engines must agree **through the
//! CLI**, not only through the library API
//! (`oderom-components/tests/localized_vs_general_differential.rs`
//! already covers that level). This is the gate that buys the right to
//! make `--engine=auto` the default: for every curvature subcommand and
//! every fixture the general engine already solves, `--engine=general`
//! and `--engine=localized` must print byte-identical output.
//!
//! Note that under `auto` several of these fixtures now run on the
//! localized engine by default -- that is expected and is exactly why
//! this test pins `general` against `localized` explicitly rather than
//! comparing either against a hardcoded string: what is being asserted
//! is *equality of result regardless of path*, not any particular
//! rendering.

use std::process::Command;

fn run(args: &[&str]) -> (bool, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_oderom")).args(args).output().expect("failed to run the oderom binary");
    (output.status.success(), String::from_utf8_lossy(&output.stdout).to_string(), String::from_utf8_lossy(&output.stderr).to_string())
}

/// Every curvature subcommand that derives from a metric. `christoffel`
/// is included even though it is the stage the engine choice is made
/// *at* -- if the two engines disagreed there, everything downstream
/// would inherit it silently.
const CURVATURE_SUBCOMMANDS: &[&str] =
    &["christoffel", "riemann", "ricci", "scalar", "kretschmann", "einstein", "riccisquare", "gaussbonnet", "weyl", "weylsquare"];

/// Fixtures the general engine is known to complete on quickly, so the
/// comparison is cheap and cannot be mistaken for a timeout. Kerr is
/// deliberately absent: the general engine does not finish on it at all
/// (that is the whole premise of this line of work), so there is no
/// general-engine result to compare against -- Kerr's own correctness is
/// asserted against its closed form in `kerr.rs` instead.
const FIXTURES: &[&str] = &["tests/fixtures/schwarzschild_ascii.od", "tests/fixtures/reissner_nordstrom.od"];

fn assert_engines_agree(fixture: &str, subcommand: &str) {
    let (ok_g, out_g, err_g) = run(&[subcommand, fixture, "--engine=general", "--timeout", "120"]);
    assert!(ok_g, "{subcommand} {fixture} --engine=general failed: {err_g}");
    let (ok_l, out_l, err_l) = run(&[subcommand, fixture, "--engine=localized", "--timeout", "120"]);
    assert!(ok_l, "{subcommand} {fixture} --engine=localized failed: {err_l}");

    assert_eq!(out_g, out_l, "engines disagree for `{subcommand} {fixture}`\ngeneral:\n{out_g}\nlocalized:\n{out_l}");

    // The forced choices must actually have been honored -- otherwise
    // this test could pass by running the same engine twice.
    assert!(err_g.contains("motor: geral"), "--engine=general did not report the general engine: {err_g}");
    assert!(err_l.contains("motor: localizado"), "--engine=localized did not report the localized engine: {err_l}");
}

#[test]
fn schwarzschild_agrees_across_engines_for_every_curvature_subcommand() {
    for sub in CURVATURE_SUBCOMMANDS {
        assert_engines_agree(FIXTURES[0], sub);
    }
}

/// Reissner-Nordstrom separately from Schwarzschild, and `#[ignore]`d:
/// its three-term `f(r)` is the fixture that historically stressed the
/// general engine hardest (DESIGN-RATIONAL-FORM.md section 0), so
/// running ten subcommands twice over it is slow enough to belong in an
/// explicit run rather than the default suite.
#[test]
#[ignore]
fn reissner_nordstrom_agrees_across_engines_for_every_curvature_subcommand() {
    for sub in CURVATURE_SUBCOMMANDS {
        assert_engines_agree(FIXTURES[1], sub);
    }
}

/// `--engine=localized` on a metric-backed source must actually report
/// the localized engine, not quietly fall back -- the whole reason the
/// flag has a third value instead of being a boolean is so a test
/// cannot pass through the wrong path without saying so.
#[test]
fn forcing_the_localized_engine_reports_it_rather_than_falling_back() {
    let (ok, _out, err) = run(&["christoffel", "tests/fixtures/schwarzschild_ascii.od", "--engine=localized"]);
    assert!(ok, "{err}");
    assert!(err.contains("motor: localizado"), "a metric-backed source should localize: {err}");
}

/// Both flag spellings reach the same place -- `--engine localized`
/// (matching every other flag in this CLI) and `--engine=localized`
/// (what the spec writes). Accepting only one makes the other look
/// like the feature is missing.
#[test]
fn both_engine_flag_spellings_are_accepted() {
    let (ok_eq, out_eq, err_eq) = run(&["scalar", "tests/fixtures/schwarzschild_ascii.od", "--engine=general"]);
    let (ok_sp, out_sp, err_sp) = run(&["scalar", "tests/fixtures/schwarzschild_ascii.od", "--engine", "general"]);
    assert!(ok_eq && ok_sp, "{err_eq}\n{err_sp}");
    assert_eq!(out_eq, out_sp);
    assert!(err_eq.contains("motor: geral") && err_sp.contains("motor: geral"));
}

/// An unknown `--engine` value is a usage error, never a silent default
/// -- a typo like `--engine=localised` must not quietly run the general
/// engine and look like a performance mystery later.
#[test]
fn an_unknown_engine_value_is_a_usage_error() {
    let (ok, _out, _err) = run(&["kretschmann", "tests/fixtures/schwarzschild_ascii.od", "--engine=localised"]);
    assert!(!ok, "a misspelled --engine value must not be accepted");
}

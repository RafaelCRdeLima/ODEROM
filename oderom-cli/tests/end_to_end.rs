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

/// This round's typography fix (named, space-separated, variance-aware
/// indices; orbit note on its own line, never glued to the formula;
/// the label itself through the same Greek-macro mapping as any other
/// name): `--target latex` on Schwarzschild's real Christoffel symbols
/// must come back as book-style `\Gamma^{t}_{t r}`, never the old
/// `Gamma_{0,0,1}` raw-integer form (bare, non-Greek label; comma
/// instead of a space), and no summary/orbit-note text mixed into the
/// same line as a `\frac`. The space between `t` and `r` is deliberate
/// (see `oderom_components::render::format_indices`'s own doc comment)
/// -- it is what keeps a Greek macro from swallowing a directly
/// following plain letter into its own control-word name, a real bug
/// (`\thetar` instead of `\theta` then `r`) a user caught by eye.
#[test]
fn christoffel_latex_uses_named_variance_aware_indices() {
    let (ok, stdout, stderr) = run(&["christoffel", "tests/fixtures/schwarzschild_ascii.od", "--target", "latex"]);
    assert!(ok, "{stderr}");
    assert!(
        stdout.contains("\\Gamma^{t}_{t r}") || stdout.contains("\\Gamma^{r}_{r r}"),
        "the label must be the Greek macro \\Gamma, never bare \"Gamma\": {stdout}"
    );
    assert!(!stdout.contains(','), "no comma-separated raw index should remain: {stdout}");
    assert!(!stdout.contains("\\thetar") && !stdout.contains("\\phir"), "a Greek index must never swallow a following plain letter: {stdout}");
}

/// Same fix, for `riemann` from a METRIC: fully covariant (the first
/// index was already lowered), so `R_{t r t r}`-style, never a leading
/// `^` anywhere in an index position, and the "N components by
/// symmetry" annotation must be its own line, not appended to a
/// `\frac`-bearing line (the literal bug report: an English
/// parenthetical sharing a line with real LaTeX).
#[test]
fn riemann_from_metric_latex_is_named_covariant_and_keeps_the_orbit_note_off_the_formula_line() {
    let (ok, stdout, stderr) = run(&["riemann", "tests/fixtures/schwarzschild_ascii.od", "--target", "latex"]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("R_{t r t r}"), "{stdout}");
    assert!(!stdout.contains(','), "no comma-separated raw index should remain: {stdout}");
    assert!(!stdout.contains("\\thetar") && !stdout.contains("\\phir"), "a Greek index must never swallow a following plain letter: {stdout}");
    for line in stdout.lines() {
        if line.contains("components by symmetry") {
            assert!(!line.contains('\\'), "the orbit note must never share a line with real LaTeX: {line:?}");
        }
    }
}

/// The other real case this round had to get right: `riemann` from a
/// bare `connection` (no metric to lower an index with) is genuinely
/// mixed, R^a_bcd -- the display must show that real variance, not
/// flatten every result to covariant. Proves the shown variance always
/// follows what was actually computed rather than being fixed.
#[test]
fn riemann_from_a_bare_connection_still_shows_the_real_mixed_variance() {
    let (ok, stdout, stderr) = run(&["riemann", "tests/fixtures/connection_only.od", "--target", "latex"]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("R^{x}_{y x y}") || stdout.contains("R^{x}_{y y x}"), "{stdout}");
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

/// The guardrail: kretschmann of Reissner-Nordstrom terminates and
/// matches the closed form under the production engine (see
/// oderom-components/tests/reissner_nordstrom.rs) -- ~1s in a release
/// build, reliably measured, so a `3`-second budget (this test's
/// original value) does not fire there at all; it only ever "worked" by
/// relying on an unoptimized debug build being slow enough, which broke
/// silently the first time this test ran under `--release` (a metric
/// with `g_tt`/`g_rr` not reciprocal, see DESIGN-RATIONAL-FORM.md, is
/// the real non-terminating case this guardrail exists for now).
/// `200ms` is below RN's own Christoffel stage alone
/// (`oderom-components/tests/diagnostic_cancel_latency.rs`: ~50-65ms) in
/// either build profile, so this fires reliably regardless.
#[test]
fn kretschmann_of_reissner_nordstrom_times_out_cleanly_instead_of_hanging() {
    let start = std::time::Instant::now();
    let (ok, _stdout, stderr) = run(&["kretschmann", "tests/fixtures/reissner_nordstrom.od", "--timeout", "0.2"]);
    assert!(start.elapsed() < std::time::Duration::from_secs(10), "the command must not hang past its own timeout");
    assert!(!ok);
    assert!(stderr.contains("timed out after"), "{stderr}");
}

/// With enough time budget for the denominator-degree check to actually
/// fire (it costs about as much as normalize() itself, see commands.rs),
/// the abort names the real cause precisely instead of a generic
/// timeout. `#[ignore]`d: ~15s in --release, ~60s in the default debug
/// profile this suite normally runs under -- too slow to run by default
/// alongside the 3s generic-timeout test above, which already covers
/// "does not hang" on every run. Run explicitly with
/// `cargo test -p oderom-cli --test end_to_end -- --ignored`.
#[test]
#[ignore]
fn kretschmann_of_reissner_nordstrom_names_denominator_degree_when_given_time_to_check() {
    let (ok, _stdout, stderr) = run(&[
        "kretschmann",
        "tests/fixtures/reissner_nordstrom.od",
        "--timeout",
        "60",
        "--max-denominator-degree",
        "30",
    ]);
    assert!(!ok);
    assert!(stderr.contains("denominator degree exceeded"), "{stderr}");
}

/// christoffel/riemann/ricci on Reissner-Nordstrom are the stages the
/// user's own diagnosis found to be fast and correct (unlike
/// kretschmann/scalar's contraction) -- regression coverage so a future
/// change to the guardrail or the pipeline doesn't quietly slow these
/// back down.
#[test]
fn christoffel_and_riemann_of_reissner_nordstrom_stay_fast() {
    let start = std::time::Instant::now();
    let (ok, stdout, stderr) = run(&["christoffel", "tests/fixtures/reissner_nordstrom.od", "--timeout", "5"]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("Gamma["), "{stdout}");
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
}

/// A user caught, by eye, a real bug in a single screenshot
/// (`christoffel`'s `\thetar`): a Greek LaTeX macro (a *control word* --
/// TeX reads the backslash, then keeps consuming letters) directly
/// followed by a letter or digit with no separator between them parses
/// as one longer, undefined macro instead of the macro then that
/// character. The fix (`oderom_components::render::format_indices`)
/// joins index names with a space, which math mode ignores for layout.
///
/// A single spot check would only prove that particular occurrence is
/// fixed -- exactly the failure mode the user flagged: eyeballing a
/// screenshot confirms what you already expect to see, and this bug
/// lives precisely in what nobody was looking for. This asserts the
/// ABSENCE of the whole bug class instead, over real output: every
/// component-listing query (`christoffel`/`riemann`/`ricci`), against
/// every checked-in fixture that has a Greek-letter coordinate, with
/// `--target latex`, scanned end to end for any recognized Greek macro
/// name directly followed by a letter or digit. A future regression of
/// this exact shape -- anywhere in this family, not just the one
/// component someone happens to look at -- fails this test by itself.
#[test]
fn no_component_query_output_ever_glues_a_greek_macro_to_an_extending_character() {
    // Every fixture in this repo whose chart declares a coordinate name
    // that `oderom_expr::GREEK_LETTERS` recognizes -- found by grepping
    // the fixtures directory for "theta"/"phi"/"chi" once, by hand, not
    // maintained by feel: if a future fixture adds another Greek
    // coordinate and isn't listed here, it simply isn't covered yet,
    // rather than silently assumed covered.
    let fixtures = [
        "tests/fixtures/schwarzschild_ascii.od",
        "tests/fixtures/schwarzschild_latex.od",
        "tests/fixtures/reissner_nordstrom.od",
        "tests/fixtures/reissner_nordstrom_alias.od",
        "tests/fixtures/hyperbolic_plane.od",
    ];
    let mut checked_at_least_one = false;
    for fixture in fixtures {
        for query in ["christoffel", "riemann", "ricci"] {
            let (ok, stdout, stderr) = run(&[query, fixture, "--target", "latex"]);
            assert!(ok, "{query} {fixture}: {stderr}");
            assert_no_glued_greek_macro(&stdout, &format!("{query} {fixture}"));
            checked_at_least_one = true;
        }
    }
    assert!(checked_at_least_one, "the fixture list above must not be empty");
}

/// Scans `output` for any name in `oderom_expr::GREEK_LETTERS` --
/// tried both as the lowercase macro (`\theta`) and the
/// capitalized-first-letter macro `oderom_expr::latex_var` produces for
/// an uppercase source name like `Gamma` (`\Gamma`) -- appearing
/// directly followed by an ASCII letter or digit, with nothing (no
/// space, no `\`, no `{`, no `}`) between them. That adjacency is
/// exactly what makes TeX's control-word reader swallow the next
/// character into the macro's own name instead of starting a new
/// token -- the general shape of the real `\thetar` bug, not a
/// one-off string comparison against that specific case.
fn assert_no_glued_greek_macro(output: &str, context: &str) {
    for name in oderom_expr::GREEK_LETTERS {
        let lower = format!("\\{name}");
        let mut chars = name.chars();
        let capitalized = match chars.next() {
            Some(first) => format!("\\{}{}", first.to_ascii_uppercase(), chars.as_str()),
            None => continue,
        };
        for macro_str in [lower.as_str(), capitalized.as_str()] {
            let mut start = 0;
            while let Some(pos) = output[start..].find(macro_str) {
                let abs = start + pos;
                let after = abs + macro_str.len();
                if let Some(next_char) = output[after..].chars().next() {
                    assert!(
                        !next_char.is_ascii_alphanumeric(),
                        "{context}: found `{macro_str}` directly followed by `{next_char}` with no separator -- \
                         this parses as one undefined macro (e.g. `\\thetar`), not `{macro_str}` then `{next_char}`. \
                         Full output:\n{output}"
                    );
                }
                start = after;
            }
        }
    }
}

/// Marco 6 step 4 (indeterminate functions, DESIGN-M6-PREP.md section
/// 1): the acceptance test that the new atom actually crosses the
/// WHOLE engine, from a metric declaration through a real curvature
/// query, without anything along the way needing to be taught about it
/// specially -- exactly the design document's own prediction
/// (`curvature.rs` calls the same generic `diff`/`normalize` it always
/// did). `f(r)` used to be rejected with `unknown function`; this
/// confirms it is now accepted and REPRESENTED, and that a real
/// `christoffel` query over it produces symbols containing both `f(r)`
/// and `f'(r)` in the expected, legible form -- never that any equation
/// gets solved, which is explicitly out of scope for this round.
#[test]
fn a_metric_with_an_unknown_radial_function_is_accepted_and_christoffel_shows_f_and_f_prime() {
    let (ok, stdout, stderr) = run(&["christoffel", "tests/fixtures/unknown_static_spherical.od"]);
    assert!(ok, "metric with f(r) must be accepted, not rejected as an unknown function: {stderr}");
    assert!(!stdout.contains("unknown function"), "{stdout}");
    assert!(stdout.contains("f(r)"), "expected f(r) itself to appear in the Christoffel output: {stdout}");
    assert!(stdout.contains("f'(r)"), "expected f'(r), f's own derivative, to appear (the metric's r-dependence forces at least one Christoffel symbol to differentiate f): {stdout}");
    // Never a second, unrelated indeterminate function invented out of
    // thin air, and never the literal string "unknown" anywhere --
    // confirms this is a clean representation, not a near-miss.
    assert!(!stdout.to_lowercase().contains("unknown"), "{stdout}");
}

/// The trap this round exists to close: a metric using `tan(theta)`
/// where the real trigonometric tangent was clearly intended must be
/// rejected with a clean, honest error -- never silently accepted as
/// an opaque indeterminate function named "tan", which would have
/// differentiated as an ordinary `tan'` instead of the real `sec^2`
/// and handed back a plausible-looking, silently wrong Christoffel
/// symbol. `f(r)` (no standard meaning) stays valid regardless --
/// confirmed side by side against the SAME kind of query
/// (`christoffel`) the accepted-case test above uses, so the two
/// behaviors are checked under identical conditions, not different
/// commands that could each have their own accidental gap.
#[test]
fn a_metric_using_tan_where_the_real_tangent_was_meant_is_a_clean_error_never_an_opaque_function() {
    let (ok, stdout, stderr) = run(&["christoffel", "tests/fixtures/reserved_name_trap.od"]);
    assert!(!ok, "a reserved elementary function name must never silently succeed: {stdout}");
    assert!(stderr.contains("not yet implemented as callable"), "{stderr}");
    assert!(stderr.contains("reserved names cannot be used as an indeterminate function"), "{stderr}");
    // And the reservation must not somehow disable the general
    // indeterminate-function rule for an unrelated, genuinely unknown
    // name -- checked directly against the accepted-case fixture,
    // through the same `christoffel` command, in the same test run.
    let (ok2, _, stderr2) = run(&["christoffel", "tests/fixtures/unknown_static_spherical.od"]);
    assert!(ok2, "f(r) must remain a valid indeterminate function: {stderr2}");
}

/// Same fixture, `--target latex`: the Greek-macro-gluing regression
/// scan above (`no_component_query_output_ever_glues_a_greek_macro_to_an_extending_character`)
/// only covers fixtures with a Greek coordinate; this checks the
/// analogous, `f`-specific concern directly -- `f'(r)` must render with
/// a real LaTeX prime (`f'(r)`, a literal apostrophe, which KaTeX
/// renders as an actual prime mark, no macro needed) and `\Gamma`
/// (the label) must still be the Greek capital letter, not glued to
/// anything from the `f`/`f'` atoms sitting right next to it in the
/// same formula.
#[test]
fn the_same_metric_in_latex_shows_a_real_prime_mark_and_keeps_gamma_greek() {
    let (ok, stdout, stderr) = run(&["christoffel", "tests/fixtures/unknown_static_spherical.od", "--target", "latex"]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("f'(r)"), "{stdout}");
    // Every line's label must be the macro `\Gamma`, never bare
    // "Gamma" -- checked per line rather than a single `contains`,
    // since "Gamma" is also a substring of "\Gamma" itself.
    for line in stdout.lines().filter(|l| l.contains('=')) {
        assert!(line.trim_start().starts_with("\\Gamma"), "expected the Greek macro \\Gamma, not bare \"Gamma\": {line:?}");
    }
}

// ---------------------------------------------------------------------
// Marco 6 step 4, round B: `geodesic`.
// ---------------------------------------------------------------------

/// Four equations, one per Schwarzschild coordinate, in Unicode's
/// explicit `coord'(param)`/`coord''(param)` form -- the parameter
/// spelled out, since a dot glyph doesn't survive plain text. Which
/// exact combination comes out is already hand-verified against this
/// same engine's own `christoffel` output at the `oderom-components`
/// level (`tests/schwarzschild.rs`); this only checks the CLI's own
/// plumbing gets the same shape of output out to a real subprocess.
#[test]
fn geodesic_of_schwarzschild_unicode_shows_four_equations_with_explicit_derivatives() {
    let (ok, stdout, stderr) = run(&["geodesic", "tests/fixtures/schwarzschild_ascii.od", "--param", "tau"]);
    assert!(ok, "{stderr}");
    let lines: Vec<&str> = stdout.lines().filter(|l| l.trim_end().ends_with("= 0")).collect();
    assert_eq!(lines.len(), 4, "{stdout}");
    assert!(stdout.contains("t''(tau)"), "{stdout}");
    assert!(stdout.contains("r'(tau)"), "{stdout}");
    assert!(stdout.contains("theta'(tau)"), "{stdout}");
    assert!(stdout.contains("phi'(tau)"), "{stdout}");
}

/// Same query, `--target latex`: dot notation, parameter implicit, no
/// leftover prime marks or bare "tau" anywhere -- the SAME four
/// equations as the Unicode test above, only the typography differs
/// (this module's own central design point).
#[test]
fn geodesic_of_schwarzschild_latex_uses_dot_notation_with_the_parameter_implicit() {
    let (ok, stdout, stderr) = run(&["geodesic", "tests/fixtures/schwarzschild_ascii.od", "--param", "tau", "--target", "latex"]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("\\ddot{t}"), "{stdout}");
    assert!(stdout.contains("\\dot{r}"), "{stdout}");
    assert!(stdout.contains("\\dot{\\theta}"), "the Greek coordinate must still use its macro under dot notation: {stdout}");
    assert!(stdout.contains("\\dot{\\phi}"), "{stdout}");
    assert!(!stdout.contains("tau"), "the affine parameter must be implicit in LaTeX dot notation: {stdout}");
    assert!(!stdout.contains('\''), "no leftover prime marks should remain once dot notation applies: {stdout}");
}

/// Mandatory collision rule: an affine parameter equal to a chart
/// coordinate is refused outright, never silently accepted as if it
/// named something else.
#[test]
fn geodesic_refuses_a_parameter_that_collides_with_a_chart_coordinate() {
    let (ok, _stdout, stderr) = run(&["geodesic", "tests/fixtures/schwarzschild_ascii.od", "--param", "r"]);
    assert!(!ok);
    assert!(stderr.contains("collides with chart coordinate"), "{stderr}");
}

/// Same rule, the other collision case: a parameter equal to a free
/// variable already used in the metric itself (`M`, not a coordinate).
#[test]
fn geodesic_refuses_a_parameter_that_collides_with_a_metric_parameter() {
    let (ok, _stdout, stderr) = run(&["geodesic", "tests/fixtures/schwarzschild_ascii.od", "--param", "M"]);
    assert!(!ok);
    assert!(stderr.contains("collides with a free variable"), "{stderr}");
}

/// `geodesic` only ever needs Gamma, never `g^ab` -- unlike
/// `scalar`/`kretschmann` (see `scalar_on_a_bare_connection_errors_instead_of_guessing`
/// above), it must work from a bare connection with no metric at all.
#[test]
fn geodesic_works_from_a_bare_connection_with_no_metric() {
    let (ok, stdout, stderr) = run(&["geodesic", "tests/fixtures/connection_only.od", "--param", "tau"]);
    assert!(ok, "{stderr}");
    let lines: Vec<&str> = stdout.lines().filter(|l| l.trim_end().ends_with("= 0")).collect();
    assert_eq!(lines.len(), 2, "{stdout}");
}

// ---------------------------------------------------------------------
// Marco 6 step 4, round C: `accel` -- the geodesic equation solved for
// each coordinate's own second derivative.
// ---------------------------------------------------------------------

/// Four equations, `coord'' = ...`, never the canonical `= 0` shape --
/// the hand-verified reconstruction against `geodesic`'s own output
/// already lives at the `oderom-components` level
/// (`tests/schwarzschild.rs`, `accel_of_schwarzschild_reproduces_geodesic_when_multiplied_back`);
/// this only checks the CLI's own plumbing produces the same shape of
/// output through a real subprocess.
#[test]
fn accel_of_schwarzschild_unicode_solves_for_each_coordinates_own_second_derivative() {
    let (ok, stdout, stderr) = run(&["accel", "tests/fixtures/schwarzschild_ascii.od", "--param", "tau"]);
    assert!(ok, "{stderr}");
    for coord in ["t", "r", "theta", "phi"] {
        assert!(stdout.contains(&format!("{coord}''(tau) = ")), "{stdout}");
    }
    // Never the canonical "= 0" form -- that's `geodesic`'s own shape,
    // untouched by this command.
    assert!(!stdout.lines().any(|l| l.trim_end().ends_with("= 0")), "{stdout}");
}

/// Same query, `--target latex`: dot notation on BOTH sides of the
/// `=`, parameter implicit, no leftover prime marks -- the same
/// typography split `geodesic` already has, just with the acceleration
/// isolated on the left instead of embedded in the sum.
#[test]
fn accel_of_schwarzschild_latex_uses_dot_notation_on_both_sides() {
    let (ok, stdout, stderr) = run(&["accel", "tests/fixtures/schwarzschild_ascii.od", "--param", "tau", "--target", "latex"]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("\\ddot{t} = "), "{stdout}");
    assert!(stdout.contains("\\ddot{r} = "), "{stdout}");
    assert!(stdout.contains("\\dot{\\theta}"), "the Greek coordinate must still use its macro under dot notation: {stdout}");
    assert!(!stdout.contains("tau"), "the affine parameter must be implicit in LaTeX dot notation: {stdout}");
    assert!(!stdout.contains('\''), "no leftover prime marks should remain once dot notation applies: {stdout}");
}

/// Same mandatory collision rule as `geodesic`, named correctly for
/// THIS command (not a leftover "geodesic" in the error text).
#[test]
fn accel_refuses_a_parameter_that_collides_with_a_chart_coordinate() {
    let (ok, _stdout, stderr) = run(&["accel", "tests/fixtures/schwarzschild_ascii.od", "--param", "r"]);
    assert!(!ok);
    assert!(stderr.contains("collides with chart coordinate"), "{stderr}");
    assert!(stderr.contains("accel needs a distinct name"), "{stderr}");
}

#[test]
fn accel_refuses_a_parameter_that_collides_with_a_metric_parameter() {
    let (ok, _stdout, stderr) = run(&["accel", "tests/fixtures/schwarzschild_ascii.od", "--param", "M"]);
    assert!(!ok);
    assert!(stderr.contains("collides with a free variable"), "{stderr}");
    assert!(stderr.contains("accel needs a distinct name"), "{stderr}");
}

/// `accel` only ever needs Gamma, same as `geodesic` -- must work from
/// a bare connection with no metric at all.
#[test]
fn accel_works_from_a_bare_connection_with_no_metric() {
    let (ok, stdout, stderr) = run(&["accel", "tests/fixtures/connection_only.od", "--param", "tau"]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("x''(tau) = "), "{stdout}");
    assert!(stdout.contains("y''(tau) = "), "{stdout}");
}

// ---------------------------------------------------------------------
// Marco 6 step 5: `einstein` -- G_ab = R_ab - (1/2) g_ab R.
// ---------------------------------------------------------------------

/// The golden check: Schwarzschild is a vacuum solution, so `einstein`
/// must show every one of the ten independent components identically
/// zero -- the same shape `ricci_of_schwarzschild_shows_all_ten_independent_components_as_zero`
/// already established for `ricci` itself, now for the new query.
#[test]
fn einstein_of_schwarzschild_shows_all_ten_independent_components_as_zero() {
    let (ok, stdout, stderr) = run(&["einstein", "tests/fixtures/schwarzschild_ascii.od"]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("10 independent components identically zero"), "{stdout}");
    assert!(!stdout.contains("G["), "no nonzero Einstein component should be printed: {stdout}");
}

/// Reissner-Nordstrom is NOT vacuum (electrovac): `einstein` must show
/// real nonzero components here, never all zero -- otherwise the
/// Schwarzschild test above would be vacuously satisfied by a query that
/// always prints zero regardless of input.
#[test]
fn einstein_of_reissner_nordstrom_shows_nonzero_components() {
    let (ok, stdout, stderr) = run(&["einstein", "tests/fixtures/reissner_nordstrom.od"]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("G[theta,theta]") || stdout.contains("G[t,t]"), "{stdout}");
    assert!(!stdout.contains("4 independent components identically zero"), "{stdout}");
}

/// Reissner-Nordstrom's own Ricci scalar is zero (traceless
/// electromagnetic stress-energy in 4D) -- so `G_ab = R_ab - (1/2)
/// g_ab * 0 = R_ab` exactly. `einstein` and `ricci` must produce
/// byte-identical output here (only the label differs), the sharpest
/// possible confirmation that `einstein` reuses `ricci`'s own real
/// output rather than recomputing something merely similar-looking.
#[test]
fn einstein_of_reissner_nordstrom_matches_ricci_since_the_scalar_vanishes() {
    let (scalar_ok, scalar_stdout, scalar_stderr) = run(&["scalar", "tests/fixtures/reissner_nordstrom.od"]);
    assert!(scalar_ok, "{scalar_stderr}");
    assert_eq!(scalar_stdout.trim(), "0", "this test's whole premise is R=0 for Reissner-Nordstrom: {scalar_stdout}");

    let (ricci_ok, ricci_stdout, ricci_stderr) = run(&["ricci", "tests/fixtures/reissner_nordstrom.od"]);
    assert!(ricci_ok, "{ricci_stderr}");
    let (einstein_ok, einstein_stdout, einstein_stderr) = run(&["einstein", "tests/fixtures/reissner_nordstrom.od"]);
    assert!(einstein_ok, "{einstein_stderr}");

    let ricci_values: Vec<&str> = ricci_stdout.lines().filter_map(|l| l.split_once(" = ").map(|(_, v)| v)).collect();
    let einstein_values: Vec<&str> = einstein_stdout.lines().filter_map(|l| l.split_once(" = ").map(|(_, v)| v)).collect();
    assert_eq!(ricci_values, einstein_values, "ricci:\n{ricci_stdout}\nvs einstein:\n{einstein_stdout}");
}

/// `einstein` needs a real metric -- `g_ab` appears literally in the
/// formula, and `R` itself already needs `g^bd` -- so a bare connection
/// must be refused with the same clean, named error `scalar`/
/// `kretschmann` already give, never a meaningless number.
#[test]
fn einstein_on_a_bare_connection_errors_instead_of_guessing() {
    let (ok, _stdout, stderr) = run(&["einstein", "tests/fixtures/connection_only.od"]);
    assert!(!ok);
    assert!(stderr.contains("needs a metric"), "{stderr}");
}

/// Same typography guarantees every other tensor-listing query already
/// has: named, space-separated LaTeX indices, the real `G` label (no
/// escaping needed, it's already a plain Latin letter, unlike
/// `\Gamma`/`\theta`), no raw comma-separated integer indices.
#[test]
fn einstein_latex_uses_named_variance_aware_indices() {
    let (ok, stdout, stderr) = run(&["einstein", "tests/fixtures/reissner_nordstrom.od", "--target", "latex"]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("G_{t t}") || stdout.contains("G_{\\theta \\theta}"), "{stdout}");
    assert!(!stdout.contains(','), "no comma-separated raw index should remain: {stdout}");
}

// ---------------------------------------------------------------------
// Marco 6 step 6: curvature scalar invariants (`riccisquare`,
// `gaussbonnet`) and the Weyl tensor (`weyl`, `weylsquare`).
// ---------------------------------------------------------------------

/// Schwarzschild is vacuum: `R_ab = 0`, so `R_ab R^ab` must be exactly
/// `0`.
#[test]
fn riccisquare_of_schwarzschild_is_zero() {
    let (ok, stdout, stderr) = run(&["riccisquare", "tests/fixtures/schwarzschild_ascii.od"]);
    assert!(ok, "{stderr}");
    assert_eq!(stdout.trim(), "0");
}

/// Reissner-Nordstrom is NOT vacuum: `R_ab R^ab` must be genuinely
/// nonzero.
#[test]
fn riccisquare_of_reissner_nordstrom_is_nonzero() {
    let (ok, stdout, stderr) = run(&["riccisquare", "tests/fixtures/reissner_nordstrom.od"]);
    assert!(ok, "{stderr}");
    assert_ne!(stdout.trim(), "0");
}

/// `riccisquare` needs a real metric to raise both of Ricci's indices,
/// same refusal shape as `scalar`/`kretschmann`/`einstein`.
#[test]
fn riccisquare_on_a_bare_connection_errors_instead_of_guessing() {
    let (ok, _stdout, stderr) = run(&["riccisquare", "tests/fixtures/connection_only.od"]);
    assert!(!ok);
    assert!(stderr.contains("needs a metric"), "{stderr}");
}

/// Schwarzschild is vacuum, so the Gauss-Bonnet density's other two
/// terms vanish and it must equal `kretschmann` exactly -- the
/// program's own two independently-computed values, not a textbook
/// constant.
#[test]
fn gaussbonnet_of_schwarzschild_equals_kretschmann() {
    let (gb_ok, gb_stdout, gb_stderr) = run(&["gaussbonnet", "tests/fixtures/schwarzschild_ascii.od"]);
    assert!(gb_ok, "{gb_stderr}");
    let (k_ok, k_stdout, k_stderr) = run(&["kretschmann", "tests/fixtures/schwarzschild_ascii.od"]);
    assert!(k_ok, "{k_stderr}");
    assert_eq!(gb_stdout.trim(), k_stdout.trim());
}

#[test]
fn gaussbonnet_on_a_bare_connection_errors_instead_of_guessing() {
    let (ok, _stdout, stderr) = run(&["gaussbonnet", "tests/fixtures/connection_only.od"]);
    assert!(!ok);
    assert!(stderr.contains("needs a metric"), "{stderr}");
}

/// The golden check: Schwarzschild is vacuum, so Weyl's own correction
/// terms vanish and `C_abcd` must equal `R_abcd` byte for byte -- this
/// crate's own `riemann` output for the same file, not a value copied
/// from a textbook.
#[test]
fn weyl_of_schwarzschild_equals_riemann() {
    let (weyl_ok, weyl_stdout, weyl_stderr) = run(&["weyl", "tests/fixtures/schwarzschild_ascii.od"]);
    assert!(weyl_ok, "{weyl_stderr}");
    let (riem_ok, riem_stdout, riem_stderr) = run(&["riemann", "tests/fixtures/schwarzschild_ascii.od"]);
    assert!(riem_ok, "{riem_stderr}");
    let weyl_values: Vec<&str> = weyl_stdout.lines().filter_map(|l| l.split_once(" = ").map(|(_, v)| v)).collect();
    let riem_values: Vec<&str> = riem_stdout.lines().filter_map(|l| l.split_once(" = ").map(|(_, v)| v)).collect();
    assert_eq!(weyl_values, riem_values, "weyl:\n{weyl_stdout}\nvs riemann:\n{riem_stdout}");
}

/// Reissner-Nordstrom is NOT vacuum: Weyl must show real, nonzero
/// components, distinct from a vacuous all-zero case.
#[test]
fn weyl_of_reissner_nordstrom_shows_nonzero_components() {
    let (ok, stdout, stderr) = run(&["weyl", "tests/fixtures/reissner_nordstrom.od"]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("C[t,r,t,r]") || stdout.contains("C[theta,phi,theta,phi]"), "{stdout}");
    assert!(!stdout.contains("21 independent components identically zero"), "{stdout}");
}

/// The mandatory dimension barrier -- this project's own checked-in 2D
/// fixture (`hyperbolic_plane.od`, the direct hyperbolic analogue of
/// the round sphere the manual's own example 3 uses) must refuse
/// cleanly, never dividing by zero or inventing a component.
#[test]
fn weyl_refuses_in_two_dimensions() {
    let (ok, _stdout, stderr) = run(&["weyl", "tests/fixtures/hyperbolic_plane.od"]);
    assert!(!ok);
    assert!(stderr.contains("not defined in 2 dimensions"), "{stderr}");
}

/// `weylsquare` inherits the same dimension-2 refusal.
#[test]
fn weylsquare_refuses_in_two_dimensions() {
    let (ok, _stdout, stderr) = run(&["weylsquare", "tests/fixtures/hyperbolic_plane.od"]);
    assert!(!ok);
    assert!(stderr.contains("not defined in 2 dimensions"), "{stderr}");
}

/// `weyl` needs a real metric (`g_ab`/`R` appear literally in the
/// formula) -- refuses a bare connection like `scalar`/`kretschmann`.
#[test]
fn weyl_on_a_bare_connection_errors_instead_of_guessing() {
    let (ok, _stdout, stderr) = run(&["weyl", "tests/fixtures/connection_only.od"]);
    assert!(!ok);
    assert!(stderr.contains("needs a metric"), "{stderr}");
}

/// The golden check one contraction further: since Weyl equals Riemann
/// in Schwarzschild's vacuum, `weylsquare` must equal `kretschmann`
/// exactly too.
#[test]
fn weylsquare_of_schwarzschild_equals_kretschmann() {
    let (wsq_ok, wsq_stdout, wsq_stderr) = run(&["weylsquare", "tests/fixtures/schwarzschild_ascii.od"]);
    assert!(wsq_ok, "{wsq_stderr}");
    let (k_ok, k_stdout, k_stderr) = run(&["kretschmann", "tests/fixtures/schwarzschild_ascii.od"]);
    assert!(k_ok, "{k_stderr}");
    assert_eq!(wsq_stdout.trim(), k_stdout.trim());
}

/// Reissner-Nordstrom is not vacuum: `weylsquare` must be nonzero and
/// genuinely differ from `kretschmann`.
#[test]
fn weylsquare_of_reissner_nordstrom_differs_from_kretschmann() {
    let (wsq_ok, wsq_stdout, wsq_stderr) = run(&["weylsquare", "tests/fixtures/reissner_nordstrom.od"]);
    assert!(wsq_ok, "{wsq_stderr}");
    assert_ne!(wsq_stdout.trim(), "0");
    let (k_ok, k_stdout, k_stderr) = run(&["kretschmann", "tests/fixtures/reissner_nordstrom.od"]);
    assert!(k_ok, "{k_stderr}");
    assert_ne!(wsq_stdout.trim(), k_stdout.trim());
}

/// Same typography guarantees every other tensor-listing query already
/// has, on `weyl`'s own output: named, space-separated LaTeX indices,
/// the real (unescaped) `C` label, no raw comma-separated integers.
#[test]
fn weyl_latex_uses_named_variance_aware_indices() {
    let (ok, stdout, stderr) = run(&["weyl", "tests/fixtures/reissner_nordstrom.od", "--target", "latex"]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("C_{t r t r}") || stdout.contains("C_{\\theta \\phi \\theta \\phi}"), "{stdout}");
    assert!(!stdout.contains(','), "no comma-separated raw index should remain: {stdout}");
}

// ---------------------------------------------------------------------
// `oderom load NAME` (Rodada Galeria): the CLI's own access to the
// spacetime gallery, alongside the notebook's `load` (which pastes the
// same catalog as editable blocks -- `oderom-notebook/tests/gallery.rs`).
// ---------------------------------------------------------------------

fn temp_od_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("oderom-cli-load-test-{name}-{}-{}.od", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()))
}

#[test]
fn load_prints_the_gallery_entrys_declarations_to_stdout() {
    let (ok, stdout, stderr) = run(&["load", "schwarzschild"]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("manifold M dim 4"));
    assert!(stdout.contains("chart schw on M coords (t, r, theta, phi)"));
    assert!(stdout.contains("[t,t] = -(1 - 2*M/r)"));
}

#[test]
fn load_of_an_unknown_name_fails_and_lists_every_known_entry() {
    let (ok, _stdout, stderr) = run(&["load", "does-not-exist"]);
    assert!(!ok);
    assert!(stderr.contains("unknown gallery entry"), "{stderr}");
    for name in ["desitter", "antidesitter", "frw", "schwarzschild", "reissnernordstrom"] {
        assert!(stderr.contains(name), "expected the known-names list to mention {name:?}: {stderr}");
    }
}

/// The whole point of printing to stdout instead of a notebook-only
/// paste: the output composes with every other subcommand exactly like
/// a hand-written `.od` file, because it *is* one -- redirected to a
/// file here, then fed straight into `scalar`, no editing in between.
#[test]
fn load_output_redirected_to_a_file_is_itself_a_valid_od_file() {
    let (ok, stdout, stderr) = run(&["load", "antidesitter"]);
    assert!(ok, "{stderr}");
    let path = temp_od_path("antidesitter");
    std::fs::write(&path, stdout).unwrap();

    let (ok, stdout, stderr) = run(&["scalar", path.to_str().unwrap()]);
    let _ = std::fs::remove_file(&path);
    assert!(ok, "{stderr}");
    assert_eq!(stdout.trim(), "-12*H^2");
}

#[test]
fn load_with_no_name_and_load_with_extra_arguments_are_both_a_usage_error() {
    let (ok, _stdout, stderr) = run(&["load"]);
    assert!(!ok);
    assert!(stderr.contains("usage:"), "{stderr}");

    let (ok, _stdout, stderr) = run(&["load", "schwarzschild", "extra"]);
    assert!(!ok);
    assert!(stderr.contains("usage:"), "{stderr}");
}

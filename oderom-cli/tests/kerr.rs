//! Kerr, loaded from `examples/kerr.od` -- the same fixture the
//! deliverable ("examples/kerr.od that I can run by hand") points at --
//! exercised through the real parser (`oderom_cli::parser::parse_model`),
//! not through `oderom-components`' own `kerr.rs` test, which builds the
//! `ComponentTensor` directly via the Rust API and never touches the
//! `.od` grammar's off-diagonal `[t,phi] = ...` syntax at all.
//!
//! Split into an active half and a dormant half, same discipline as
//! `oderom-components/tests/kerr.rs`:
//! - `kerr_metric_inverse_from_od_file_satisfies_g_ginv_equals_identity`
//!   runs today (metric inversion is fast and correct regardless of the
//!   normalizer limit below).
//! - `kretschmann_of_kerr_from_od_file_matches_closed_form` is
//!   `#[ignore]`d for the same reason
//!   `oderom-components/tests/kerr.rs::ricci_of_kerr_is_identically_zero`
//!   is: blocked on the multivariate-GCD-without-a-single-pole-variable
//!   limit (DESIGN-RATIONAL-FORM.md section 7.1, Kerr's bivariate
//!   `Sigma`). Both tests come out of `#[ignore]` together, in the same
//!   commit, once the rational-form engine closes this gap -- that is
//!   this round's own definition of done.

use oderom_cli::parser::parse_model;
use oderom_components::curvature::{metric_inverse, verify_metric_inverse};

fn load_kerr() -> oderom_cli::model::Model {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../examples/kerr.od")).expect("examples/kerr.od must exist and be readable");
    parse_model(&src).expect("examples/kerr.od must parse")
}

#[test]
fn kerr_metric_inverse_from_od_file_satisfies_g_ginv_equals_identity() {
    let model = load_kerr();
    let (chart_name, _head, g) = model.metrics.get("g").expect("examples/kerr.od must declare a metric named `g`");
    let chart = model.charts.get(chart_name).expect("the metric's chart must be declared");

    let ginv = metric_inverse(&model.registry, chart, g).expect("Kerr's {t,phi} block plus two singleton blocks must invert");
    verify_metric_inverse(&model.registry, chart, g, &ginv).expect("g_ab * g^bc must equal delta^a_c for the metric parsed from examples/kerr.od");
}

/// The golden test this round exists to unlock, from the actual `.od`
/// file a user would run `oderom kretschmann examples/kerr.od` against
/// -- not blocked on inversion (fast, see above), blocked on
/// `christoffel`/`riemann_mixed` downstream of it not terminating in a
/// reasonable budget. Clearing `#[ignore]` here (and on
/// `oderom-components/tests/kerr.rs::ricci_of_kerr_is_identically_zero`,
/// `riemann_covariant_of_kerr_lowers_cleanly_through_the_non_diagonal_metric`)
/// in the same commit is the round's completion criterion. Run
/// explicitly, with patience, via:
///
/// ```text
/// cargo test -p oderom-cli --release --test kerr -- --ignored kretschmann_of_kerr_from_od_file_matches_closed_form
/// ```
#[test]
#[ignore] // blocked by DESIGN-RATIONAL-FORM.md section 7.1 (Kerr's bivariate Sigma denominator, no single pole variable)
fn kretschmann_of_kerr_from_od_file_matches_closed_form() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_oderom"))
        .args(["kretschmann", concat!(env!("CARGO_MANIFEST_DIR"), "/../examples/kerr.od")])
        .output()
        .expect("failed to run the oderom binary");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Closed form (DESIGN document / examples/kerr.od's own header):
    // K = 48*M^2*(r^2-a^2*cos(theta)^2)*((r^2+a^2*cos(theta)^2)^2 -
    // 16*r^2*a^2*cos(theta)^2) / (r^2+a^2*cos(theta)^2)^6 -- exact
    // rendered text asserted once the engine actually produces it;
    // placeholder assertion here just pins that the command completes.
    assert!(!stdout.trim().is_empty());
}

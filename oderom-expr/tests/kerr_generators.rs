//! Square-free/pairwise-irreducibility check on `{Sigma, Delta, sin(theta)}`
//! -- the generator set the structured-denominator rational-form engine
//! (DESIGN-RATIONAL-FORM.md section 7.1's fix) will localize at for
//! Kerr, per `oderom-cli/tests/diagnostic_kerr_denominators.rs`'s own
//! measurement (every denominator across Kerr's 20 nonzero independent
//! Christoffel components is exactly `Sigma^p * Delta^q * sin(theta)^r`
//! for small `p,q,r`). Canonicity of that representation depends on
//! each generator being irreducible and square-free, and on the three
//! being pairwise coprime -- if any of that failed, the same physical
//! quantity could normalize to two different-looking (factor,exponent)
//! multisets depending on computation order, which is exactly what
//! "canonical form" is supposed to rule out.
//!
//! Checked by hand first (see this session's own report), confirmed
//! here executably against the *current* production engine
//! (`normalize`/`denominator_degree`, already a full recursive
//! multivariate-GCD normalizer per DESIGN-RATIONAL-FORM.md section 6) --
//! not a new check invented for this file: if two expressions share a
//! common polynomial factor, dividing one by the other and normalizing
//! cancels it, so the resulting denominator degree drops below the
//! divisor's own degree. No cancellation (`denominator_degree(a/b) ==
//! denominator_degree(1/b)`, i.e. `degree(b)` alone) is exactly "no
//! shared factor" -- coprimality -- read off the engine that already
//! exists, before any new code is written.
//!
//! **Basis dependency, registered explicitly (not a special case)**:
//! this repository's trigonometric normal form (D-RF.7,
//! `oderom-expr/src/poly.rs`) keeps `sin` as the free-exponent primary
//! generator and reduces `cos` to degree <=1 via `cos^2 -> 1-sin^2`.
//! `sin(theta)` is therefore an ordinary degree-1 ring generator in the
//! *current* basis -- irreducible not as a special case, but because
//! every degree-1 generator is irreducible. Flipping the convention
//! (keeping `cos` free-exponent and reducing `sin` instead) would make
//! `sin(theta)` a composite expression built from `cos`, and
//! `sin(theta)^2 = 1 - cos(theta)^2 = (1-cos(theta))(1+cos(theta))`
//! would become REDUCIBLE -- two proper factors, not one irreducible
//! generator -- breaking this file's whole premise (Kerr's `sin(theta)`
//! pole factor is degree 1 today; in the flipped basis it would need to
//! be tracked as a product of two new poles, `(1-cos(theta))` and
//! `(1+cos(theta))`, or the localization set would need those instead).
//! This is registered in DESIGN-RATIONAL-FORM.md's own text, not just
//! here, so a future basis change is not silently assumed compatible.

use oderom_expr::{denominator_degree, Expr};

fn sigma() -> Expr {
    Expr::var("r").pow(2) + Expr::var("a").pow(2) * Expr::var("theta").cos().pow(2)
}

fn delta() -> Expr {
    Expr::var("r").pow(2) - Expr::int(2) * Expr::var("M") * Expr::var("r") + Expr::var("a").pow(2)
}

fn sin_theta() -> Expr {
    Expr::var("theta").sin()
}

/// `degree(y)` read off the existing public API: `denominator_degree`
/// of `1/y` is the polynomial degree of `y` itself, once `y` is put
/// over a single fraction (trivial here -- none of the three generators
/// contain their own internal fraction).
fn degree_of(y: &Expr) -> i32 {
    denominator_degree(&Expr::Pow(Box::new(y.clone()), -1))
}

/// No cancellation when dividing `x` by `y`: the quotient's denominator
/// degree equals `y`'s own degree, i.e. nothing common was pulled out.
/// This is coprimality, read off the production GCD-reducing
/// `normalize()`/`denominator_degree`, not asserted from hand-derivation
/// alone.
fn assert_coprime(x: &Expr, y: &Expr, label: &str) {
    let expected = degree_of(y);
    let got = denominator_degree(&(x.clone() * Expr::Pow(Box::new(y.clone()), -1)));
    assert_eq!(got, expected, "{label}: expected no cancellation (denominator degree {expected}), got {got} -- a shared factor was cancelled");
}

#[test]
fn sigma_and_delta_are_coprime() {
    assert_coprime(&sigma(), &delta(), "Sigma/Delta");
    assert_coprime(&delta(), &sigma(), "Delta/Sigma");
}

#[test]
fn sigma_and_sin_theta_are_coprime() {
    assert_coprime(&sigma(), &sin_theta(), "Sigma/sin(theta)");
}

#[test]
fn delta_and_sin_theta_are_coprime() {
    // Delta has no theta-dependence at all -- sin(theta) dividing it
    // would mean Delta is identically zero, which it isn't.
    assert_coprime(&delta(), &sin_theta(), "Delta/sin(theta)");
}

/// Square-free check: `gcd(P, dP/dvar) = 1` for a variable `P` actually
/// depends on -- the standard test (over a field of characteristic 0,
/// a nonzero polynomial has a repeated irreducible factor in `var` iff
/// it shares a factor with its own derivative in `var`). `Sigma`
/// depends on `r`; `Delta` depends on `r` (and `M`); `sin(theta)` is
/// checked in its own variable.
#[test]
fn sigma_is_square_free() {
    let d_dr = oderom_expr::diff(&sigma(), "r");
    assert_coprime(&sigma(), &d_dr, "Sigma/d(Sigma)/dr");
}

#[test]
fn delta_is_square_free() {
    let d_dr = oderom_expr::diff(&delta(), "r");
    assert_coprime(&delta(), &d_dr, "Delta/d(Delta)/dr");
    let d_dm = oderom_expr::diff(&delta(), "M");
    assert_coprime(&delta(), &d_dm, "Delta/d(Delta)/dM");
}

#[test]
fn sin_theta_is_square_free() {
    let d_dtheta = oderom_expr::diff(&sin_theta(), "theta");
    // d/dtheta sin(theta) = cos(theta): a bare cos(theta) shares no
    // factor with sin(theta) itself (checked the same way as every
    // other pair above, not assumed from the sin^2+cos^2=1 identity).
    assert_coprime(&sin_theta(), &d_dtheta, "sin(theta)/cos(theta)");
}

/// Sanity check on the harness itself: two expressions that manifestly
/// *do* share a factor (`Sigma` and `Sigma^2`) must NOT be reported
/// coprime -- confirms `assert_coprime`'s failure mode actually fires
/// rather than vacuously passing everything.
#[test]
fn the_coprimality_check_itself_detects_a_real_shared_factor() {
    let sigma_squared = Expr::Pow(Box::new(sigma()), 2);
    let expected_if_coprime = degree_of(&sigma_squared);
    let got = denominator_degree(&(sigma() * Expr::Pow(Box::new(sigma_squared), -1)));
    assert_ne!(got, expected_if_coprime, "Sigma and Sigma^2 manifestly share a factor -- the harness must detect that, not report full-degree coprimality");
    // What should actually happen: Sigma/Sigma^2 = 1/Sigma, degree matches Sigma alone.
    assert_eq!(got, degree_of(&sigma()));
}

//! `oderom simplify` -- abstract-index manipulation of a *sum* of tensor
//! monomials, with no chart and no metric anywhere. The counterpart to
//! `canon`, which canonicalizes a single monomial: sums are where
//! multi-term identities live, and multi-term identities are precisely
//! what pure slot-permutation symmetry provably cannot express (Bianchi's
//! cyclic permutation has order 3, Riemann's slot group has order 8, and
//! 3 does not divide 8 -- DESIGN-M4.md).
//!
//! Every assertion here pins a *mathematical* fact about the result, not
//! a rendering: either the sum collapses to `0`, or it does not collapse
//! at all. The "does not collapse" half matters as much as the other --
//! a simplifier that returns `0` too eagerly is worse than one that
//! returns nothing.

use std::process::Command;

fn simplify(args: &[&str]) -> (bool, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_oderom"))
        .arg("simplify")
        .args(args)
        // The prelude lives at the repo root; tests run with the crate
        // directory as cwd.
        .arg("--prelude")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/../prelude.od"))
        .output()
        .expect("failed to run the oderom binary");
    (output.status.success(), String::from_utf8_lossy(&output.stdout).trim().to_string(), String::from_utf8_lossy(&output.stderr).to_string())
}

/// Riemann's declared antisymmetry in its first pair makes
/// `R[a,b,c,d] + R[b,a,c,d]` identically zero. This is the simplest
/// thing a reader tries first, and it is the case that exposed a real
/// gap: the e-graph canonicalizes both terms and picks the fewest-terms
/// representative, but never *adds coefficients*, so this came back as
/// `R[a,b,c,d] + -1 R[a,b,c,d]` until like-term collection was added on
/// top of extraction.
#[test]
fn antisymmetry_in_the_first_pair_cancels_the_sum() {
    let (ok, out, err) = simplify(&["R[a,b,c,d] + R[b,a,c,d]"]);
    assert!(ok, "{err}");
    assert_eq!(out, "0");
}

/// The pair-swap symmetry: `R[c,d,a,b]` *is* `R[a,b,c,d]`, so
/// subtracting them is zero.
#[test]
fn the_pair_swap_symmetry_makes_the_difference_vanish() {
    let (ok, out, err) = simplify(&["R[a,b,c,d] - R[c,d,a,b]"]);
    assert!(ok, "{err}");
    assert_eq!(out, "0");
}

/// The first Bianchi identity, *with* the axiom registered. This is the
/// capability `canon` structurally cannot have.
#[test]
fn the_cyclic_sum_vanishes_when_bianchi_is_declared() {
    let (ok, out, err) = simplify(&["--bianchi", "R", "R[a,b,c,d] + R[a,c,d,b] + R[a,d,b,c]"]);
    assert!(ok, "{err}");
    assert_eq!(out, "0");
}

/// The other half, and the reason `--bianchi` is an explicit flag rather
/// than something inferred from the declared symmetry: **without** the
/// axiom the very same sum must NOT reduce. A tensor can carry Riemann's
/// pair antisymmetries and pair swap without satisfying the cyclic
/// identity, so inferring Bianchi from slot symmetry would be asserting
/// a theorem the engine cannot check. If this test ever starts printing
/// `0`, some change has begun assuming Bianchi for free.
#[test]
fn the_cyclic_sum_does_not_vanish_without_the_bianchi_axiom() {
    let (ok, out, err) = simplify(&["R[a,b,c,d] + R[a,c,d,b] + R[a,d,b,c]"]);
    assert!(ok, "{err}");
    assert_ne!(out, "0", "the cyclic sum must not reduce without --bianchi");
    assert_eq!(out.matches("R[").count(), 3, "expected three surviving Riemann factors, got: {out}");
}

/// Like terms add rather than merely being listed.
#[test]
fn like_terms_have_their_coefficients_added() {
    let (ok, out, err) = simplify(&["3 R[a,b,c,d] + -1 R[a,b,c,d]"]);
    assert!(ok, "{err}");
    assert_eq!(out, "2 R[a,b,c,d]");
}

/// Two genuinely different index arrangements must survive -- the
/// negative control for every cancellation above. `R[a,b,c,d]` and
/// `R[a,c,b,d]` are not related by any of Riemann's declared symmetries
/// (relating them is exactly what Bianchi would do, and it is not
/// declared here).
#[test]
fn genuinely_distinct_terms_are_left_alone() {
    let (ok, out, err) = simplify(&["R[a,b,c,d] + R[a,c,b,d]"]);
    assert!(ok, "{err}");
    assert_ne!(out, "0");
    assert_eq!(out.matches("R[").count(), 2, "both terms should survive, got: {out}");
}

/// This tool's own output must parse back in unchanged -- a reader will
/// paste a result back to keep working on it. Negative coefficients
/// render as `+ -1 R[...]`, which puts two signs in a row and is exactly
/// the shape that broke the sum splitter the first time.
#[test]
fn the_tools_own_output_parses_back_in() {
    let (ok, out, err) = simplify(&["R[a,b,c,d] + -1 R[a,c,b,d] + R[a,d,b,c]", "--bianchi", "R"]);
    assert!(ok, "{err}");
    assert_eq!(out, "0");
}

/// A single monomial is a legal (one-term) sum -- `simplify` must not
/// require a `+` to be present.
#[test]
fn a_lone_monomial_is_a_valid_sum() {
    let (ok, out, err) = simplify(&["R[b,a,c,d]"]);
    assert!(ok, "{err}");
    assert_eq!(out, "-1 R[a,b,c,d]", "a lone monomial should still be canonicalized");
}

/// A trailing operator is a clean parse error, never a silently dropped
/// term.
#[test]
fn a_trailing_operator_is_a_clean_error() {
    let (ok, _out, err) = simplify(&["R[a,b,c,d] +"]);
    assert!(!ok, "a trailing `+` must not be accepted");
    assert!(err.contains("sem monomio depois dele"), "{err}");
}

/// The index-count rule still applies inside a sum: an index appearing
/// three times in one monomial is neither free nor contracted.
#[test]
fn a_malformed_term_inside_a_sum_is_reported() {
    let (ok, _out, err) = simplify(&["R[a,b,c,d] + R[a,a,a,d]"]);
    assert!(!ok, "a term with a thrice-repeated index must be rejected");
    assert!(err.contains("appears 3 times"), "{err}");
}

/// `--bianchi` naming a head that was never declared is a clean error,
/// not a silent no-op that would look like Bianchi simply failing to
/// apply.
#[test]
fn bianchi_on_an_undeclared_head_is_a_clean_error() {
    let (ok, _out, err) = simplify(&["--bianchi", "Nonexistent", "R[a,b,c,d]"]);
    assert!(!ok, "an undeclared --bianchi head must be an error");
    assert!(!err.is_empty());
}

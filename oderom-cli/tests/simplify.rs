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

// ---------------------------------------------------------------------
// Second (differential) Bianchi identity, `R_{ab[cd;e]} = 0`.
//
// Declared through `--bianchi2`, taking the *base* Riemann head: the
// once-differentiated head is found structurally, so the user never
// writes `R;1`.
// ---------------------------------------------------------------------

const DIFF_SUM: &str = "R[a,b,c,d;e] + R[a,b,d,e;c] + R[a,b,e,c;d]";
const ALG_SUM: &str = "R[a,b,c,d] + R[a,c,d,b] + R[a,d,b,c]";

#[test]
fn second_bianchi_zeroes_the_differential_cyclic_sum() {
    let (ok, out, err) = simplify(&["--bianchi2", "R", DIFF_SUM]);
    assert!(ok, "{err}");
    assert_eq!(out.trim(), "0", "{out}");
}

/// Negative control: without the flag the same sum must survive intact,
/// which is what makes the identity visible as an extra axiom rather
/// than bookkeeping the engine was doing anyway.
#[test]
fn without_the_flag_the_differential_sum_survives() {
    let (ok, out, err) = simplify(&[DIFF_SUM]);
    assert!(ok, "{err}");
    assert_ne!(out.trim(), "0", "the differential sum must not vanish undeclared");
}

/// The two identities are independent axioms: neither implies the
/// other. Declaring one must not silently buy the other, in either
/// direction -- the failure this guards against is a flag that quietly
/// applies "Bianchi" generally and makes the distinction unobservable.
#[test]
fn the_two_bianchi_flags_are_not_interchangeable() {
    let (ok, out, err) = simplify(&["--bianchi", "R", DIFF_SUM]);
    assert!(ok, "{err}");
    assert_ne!(out.trim(), "0", "--bianchi (algebraic) must not reduce a differential sum");

    let (ok, out, err) = simplify(&["--bianchi2", "R", ALG_SUM]);
    assert!(ok, "{err}");
    assert_ne!(out.trim(), "0", "--bianchi2 (differential) must not reduce an algebraic sum");
}

/// Both may be declared at once, and then both reduce.
#[test]
fn both_bianchi_identities_can_be_declared_together() {
    let (ok, out, err) = simplify(&["--bianchi", "R", "--bianchi2", "R", ALG_SUM]);
    assert!(ok, "{err}");
    assert_eq!(out.trim(), "0", "{out}");

    let (ok, out, err) = simplify(&["--bianchi", "R", "--bianchi2", "R", DIFF_SUM]);
    assert!(ok, "{err}");
    assert_eq!(out.trim(), "0", "{out}");
}

#[test]
fn second_bianchi_on_an_undeclared_head_is_a_clean_error() {
    let (ok, _out, err) = simplify(&["--bianchi2", "Nonexistent", DIFF_SUM]);
    assert!(!ok, "an undeclared --bianchi2 head must be an error");
    assert!(!err.is_empty());
}

// ---------------------------------------------------------------------
// Metric elimination (index raising/lowering). This is the operation
// DESIGN.md records as out of Marco 1's reach -- it changes a term's
// factor count, which no permutation-symmetry coset search can do -- and
// it is what makes `simplify` able to manipulate index placement rather
// than only reorder slots.
// ---------------------------------------------------------------------

/// The exact case Marco 1's acceptance table carried as permanently
/// `#[ignore]`d: `R[a,b,c,d] g[a,c] g[b,d]` is `R[a,b,a,b]`. Now
/// reachable from the command line.
#[test]
fn a_metric_contracted_into_riemann_becomes_a_direct_self_contraction() {
    let (ok, out, err) = simplify(&["--metric", "g", "R[a,b,c,d] g[a,c] g[b,d]"]);
    assert!(ok, "{err}");
    assert_eq!(out, "R[a,b,a,b]");
}

/// Index lowering proper: the metric moves a free label onto the slot it
/// was contracted with, rather than joining two slots together.
#[test]
fn a_metric_lowers_a_free_index_onto_the_slot_it_contracts() {
    let (ok, out, err) = simplify(&["--metric", "g", "g[a,b] R[b,c,d,e]"]);
    assert!(ok, "{err}");
    assert_eq!(out, "R[a,c,d,e]");
}

/// Without `--metric` the very same input must keep its metric factors:
/// a symmetric rank-2 head is not automatically a metric, and rewriting
/// through one nobody declared would be using an unstated fact. This is
/// the negative control for the whole feature.
#[test]
fn metrics_are_not_eliminated_unless_declared() {
    let (ok, out, err) = simplify(&["R[a,b,c,d] g[a,c] g[b,d]"]);
    assert!(ok, "{err}");
    assert!(out.contains("g["), "the metric must survive when not declared: {out}");
}

/// A metric with nothing contracted into it has nothing to identify, so
/// it survives even when declared -- `eliminate_metric` is not a rule
/// that simply deletes metric factors.
#[test]
fn a_free_standing_metric_survives_even_when_declared() {
    let (ok, out, err) = simplify(&["--metric", "g", "g[a,b]"]);
    assert!(ok, "{err}");
    assert_eq!(out, "g[a,b]");
}

/// A metric traced against itself is the slot dimension: `g^a_a = n`,
/// and this prelude's `TM` has dimension 4. The result is a bare
/// coefficient with no factors left, which must render without a
/// trailing separator so it still parses back in.
#[test]
fn a_metric_traced_against_itself_is_the_dimension() {
    let (ok, out, err) = simplify(&["--metric", "g", "g[a,a]"]);
    assert!(ok, "{err}");
    assert_eq!(out, "4");
}

/// Metric elimination composes with the symmetry reasoning that follows
/// it: after the metrics go, the two terms are related by Riemann's own
/// pair antisymmetry and cancel.
#[test]
fn elimination_composes_with_the_symmetry_cancellation_that_follows() {
    let (ok, out, err) = simplify(&["--metric", "g", "R[a,b,c,d] g[a,c] g[b,d] + R[b,a,c,d] g[a,c] g[b,d]"]);
    assert!(ok, "{err}");
    assert_eq!(out, "0");
}

// ---------------------------------------------------------------------
// Covariant derivatives, written `T[a,b;c]` -- the standard GR spelling
// (comma for partial, semicolon for covariant), so `R[a,b,c,d;e]` is
// `nabla_e R_abcd`. A derivative adds a trailing slot; the base tensor's
// symmetry still applies to the original slots and to nothing else.
// ---------------------------------------------------------------------

/// The rendering round-trips: what comes out is what goes in.
#[test]
fn a_covariant_derivative_round_trips_through_the_renderer() {
    let (ok, out, err) = simplify(&["R[a,b,c,d;e]"]);
    assert!(ok, "{err}");
    assert_eq!(out, "R[a,b,c,d;e]");
}

/// The base tensor's symmetry still applies underneath the derivative:
/// Riemann's first-pair antisymmetry survives differentiation.
#[test]
fn the_base_symmetry_still_applies_under_a_derivative() {
    let (ok, out, err) = simplify(&["R[b,a,c,d;e]"]);
    assert!(ok, "{err}");
    assert_eq!(out, "-1 R[a,b,c,d;e]");
}

/// ...and therefore cancels in a sum, exactly as it does undifferentiated.
#[test]
fn antisymmetry_under_a_derivative_cancels_the_sum() {
    let (ok, out, err) = simplify(&["R[a,b,c,d;e] + R[b,a,c,d;e]"]);
    assert!(ok, "{err}");
    assert_eq!(out, "0");
}

/// **The load-bearing negative control.** Second covariant derivatives do
/// not commute -- `nabla_e nabla_f - nabla_f nabla_e` applied to a tensor
/// is the Riemann tensor acting on it, which is the whole content of
/// curvature. So this difference must NOT vanish. If it ever starts
/// printing `0`, some change has declared the derivative slots symmetric
/// and thereby silently asserted flat space.
#[test]
fn second_derivatives_do_not_commute() {
    let (ok, out, err) = simplify(&["R[a,b,c,d;e,f] - R[a,b,c,d;f,e]"]);
    assert!(ok, "{err}");
    assert_ne!(out, "0", "declaring derivative slots symmetric would assert flatness");
    assert_eq!(out.matches("R[").count(), 2, "both orderings must survive: {out}");
}

/// A derivative index does not participate in the base tensor's
/// symmetry: swapping a tensor slot with a derivative slot is a
/// different object, not a resymmetrization.
#[test]
fn a_derivative_index_is_not_interchangeable_with_a_tensor_index() {
    let (ok, out, err) = simplify(&["R[a,b,c,d;e] - R[a,b,c,e;d]"]);
    assert!(ok, "{err}");
    assert_ne!(out, "0", "a derivative slot must not be permutable with a tensor slot");
}

/// A `;` with nothing after it is a clean parse error.
#[test]
fn a_semicolon_with_no_derivative_index_is_a_clean_error() {
    let (ok, _out, err) = simplify(&["R[a,b,c,d;]"]);
    assert!(!ok, "a dangling `;` must not be accepted");
    assert!(!err.is_empty());
}

/// `0` is the single most common thing this tool prints, and for three
/// rounds it could not be read back: the parser demanded at least one
/// tensor factor, so a reader who cancelled a sum, got `0`, and pasted
/// it back to keep working got a parse error instead. Found by asking
/// what the round-trip property test could *not* generate -- its
/// generator only produced valid *input*, and this shape was valid
/// output only.
#[test]
fn zero_the_most_common_output_parses_back_in() {
    let (ok, out, err) = simplify(&["0"]);
    assert!(ok, "{err}");
    assert_eq!(out, "0");
}

/// The same for a bare dimension, which is what a traced metric reduces
/// to (`g[a,a]` -> `4`).
#[test]
fn a_bare_coefficient_parses_back_in() {
    let (ok, out, err) = simplify(&["4"]);
    assert!(ok, "{err}");
    assert_eq!(out, "4");
}

/// Scalars add like any other like terms.
#[test]
fn bare_scalars_add() {
    let (ok, out, err) = simplify(&["3 + 2"]);
    assert!(ok, "{err}");
    assert_eq!(out, "5");
}

/// Empty input is still an error -- accepting a bare coefficient must
/// not have made the parser accept nothing at all.
#[test]
fn empty_input_is_still_an_error() {
    let (ok, _out, err) = simplify(&["   "]);
    assert!(!ok, "empty input must not be accepted");
    assert!(!err.is_empty());
}

// ---------------------------------------------------------------------
// The intersection of the two newest features: metric elimination and
// covariant derivatives. Each was tested alone; these are the cases
// where they meet, which is where a wrong assumption in either one
// would show up as a wrong *answer* rather than an error.
// ---------------------------------------------------------------------

/// A metric contracted into a differentiated tensor's ordinary slots
/// still eliminates -- differentiation does not shield the tensor
/// indices from index gymnastics.
#[test]
fn a_metric_still_eliminates_into_a_differentiated_tensor() {
    let (ok, out, err) = simplify(&["--metric", "g", "R[a,b,c,d;e] g[a,c]"]);
    assert!(ok, "{err}");
    assert!(!out.contains("g["), "the metric should be gone: {out}");
    assert!(out.contains(";e"), "the derivative index should survive: {out}");
}

/// A metric contracted into the *derivative* index lowers it like any
/// other index.
#[test]
fn a_metric_lowers_a_derivative_index_too() {
    let (ok, out, err) = simplify(&["--metric", "g", "R[a,b,c,d;e] g[e,f]"]);
    assert!(ok, "{err}");
    assert_eq!(out, "R[a,b,c,d;f]");
}

/// **`nabla_c g_ab` must NOT be eliminated.** It is a different object
/// from `g_ab`: that it vanishes is metric compatibility, a *declared*
/// property of the Levi-Civita connection, not something elimination may
/// assume. Today this holds by construction -- the derivative head is a
/// distinct `HeadId`, so it never matches the metric being eliminated --
/// but that is exactly the kind of correctness-by-accident a later
/// "improvement" (matching on `base_head()`, say) would silently undo,
/// turning a declared physical fact into an unstated assumption. If this
/// test starts printing anything other than the input, elimination has
/// begun asserting metric compatibility on its own.
#[test]
fn a_differentiated_metric_is_never_eliminated() {
    let (ok, out, err) = simplify(&["--metric", "g", "g[a,b;c]"]);
    assert!(ok, "{err}");
    assert_eq!(out, "g[a,b;c]", "nabla_c g_ab is not g_ab and must survive elimination");
}

// ---------------------------------------------------------------------
// Metric compatibility, `nabla_a g_bc = 0` -- the defining property of
// the Levi-Civita connection, declared with `--metric-compatible`.
//
// Deliberately a *separate* flag from `--metric` (index gymnastics).
// `--metric` eliminates an undifferentiated metric; this one says a
// differentiated metric vanishes. Conflating them would let index
// raising silently assume a property of the connection.
// ---------------------------------------------------------------------

#[test]
fn a_differentiated_metric_vanishes_when_compatibility_is_declared() {
    let (ok, out, err) = simplify(&["--metric-compatible", "g", "g[a,b;c]"]);
    assert!(ok, "{err}");
    assert_eq!(out, "0");
}

/// The negative control, and the reason the flag exists: a metric
/// carried by a connection with torsion does not satisfy this, so the
/// engine must not assume it. Without the flag, `nabla_c g_ab` survives.
#[test]
fn a_differentiated_metric_survives_without_the_compatibility_axiom() {
    let (ok, out, err) = simplify(&["g[a,b;c]"]);
    assert!(ok, "{err}");
    assert_eq!(out, "g[a,b;c]");
}

/// A product with a vanishing factor vanishes -- so the rule is useful
/// without any Leibniz machinery: `nabla_a g_bc R[d,e,f,h]` is zero for
/// the same reason `nabla_a g_bc` is.
#[test]
fn a_product_containing_a_differentiated_metric_vanishes() {
    let (ok, out, err) = simplify(&["--metric-compatible", "g", "g[a,b;c] R[d,e,f,h]"]);
    assert!(ok, "{err}");
    assert_eq!(out, "0");
}

/// Higher derivatives vanish too -- `nabla_d nabla_c g_ab = 0` follows
/// from differentiating zero, and the rule matches on *any* derivative
/// count rather than only the first.
#[test]
fn a_second_derivative_of_the_metric_also_vanishes() {
    let (ok, out, err) = simplify(&["--metric-compatible", "g", "g[a,b;c,d]"]);
    assert!(ok, "{err}");
    assert_eq!(out, "0");
}

/// The *undifferentiated* metric is emphatically not zero. This is the
/// control that keeps the rule from degenerating into "delete every
/// metric factor".
#[test]
fn the_undifferentiated_metric_is_not_zero() {
    let (ok, out, err) = simplify(&["--metric-compatible", "g", "g[a,b]"]);
    assert!(ok, "{err}");
    assert_eq!(out, "g[a,b]");
}

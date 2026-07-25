//! `num/den`, reduced by polynomial GCD when there's a single "pole
//! variable" to reduce against (DESIGN-RATIONAL-FORM.md section 2.2/2.3)
//! -- the piece `rationalize()`'s explicit numerator/denominator never
//! had: `RationalFunction::add`/`mul`/`pow` all reduce after combining,
//! instead of letting denominators multiply together unboundedly.
//!
//! Correctness contract (D-RF section 2.6, "reduzido ou não, nunca
//! errado"): whenever reduction isn't attempted (no single pole
//! variable, or the pseudo-division below doesn't come out exact) the
//! result is still `num/den` for the exact same value, just not
//! cancelled -- never wrong, only potentially larger than it needs to be.
//!
//! # Two types, not one (D-RF.1, as revised after a confirmed
//! non-termination bug)
//!
//! An earlier version of this module had exactly one recursive shape:
//! `UPoly` (a polynomial in one chosen pole variable) whose *coefficients
//! were themselves `RationalFunction`s* -- i.e. could have their own
//! denominator, their own pole variable, their own recursive GCD. That
//! meant a coefficient that happened to need reducing could pick a
//! *different* pole variable than its enclosing call, whose own
//! reassembly could reintroduce the *first* variable into a denominator
//! again, and so on: no measure ever strictly decreased, and the
//! recursion did not terminate (confirmed directly: pole choice observed
//! ping-ponging `r -> M -> r -> M -> ...` across recursion depth on the
//! minimal reproducer below, past depth 900 on a related case before the
//! real stack overflowed).
//!
//! The fix is not a base case or a smarter reassembly bolted onto that
//! shape -- it's that the shape itself was wrong. `UPoly`'s coefficients
//! here are plain [`Poly`]: a polynomial ring over the *other* generators
//! (`M`, `Q`, `sin(theta)`, ...), with no denominator field at all, and
//! no operation that could ever produce one -- addition is monomial
//! collection, multiplication is monomial collection, full stop. The
//! coefficient type structurally cannot call `reduce()` or invert
//! anything, because it has no such method: this is enforced by the type
//! system, not by convention. `RationalFunction` (this module's only
//! `num/den`-carrying type) is the *only* place a denominator, a pole
//! variable, or a GCD computation exists at all. Reducing therefore never
//! recurses: pick the one pole variable, run one Euclidean pass over
//! `UPoly<Poly>`, reassemble directly -- there is no second level to
//! reduce *into*.
//!
//! This restricts what this engine can reduce: it assumes the class of
//! expressions this project's metrics produce never need a denominator
//! *inside* a coefficient (D-RF.1's note, confirmed safe for this
//! project's class of problems: `1/(1-2M/r+Q^2/r^2) = r^2/(r^2-2Mr+Q^2)`,
//! a polynomial in `r` with polynomial-in-`M,Q` coefficients, no nested
//! fraction anywhere). If a real case ever needs one, that is a design
//! change, not a bug to patch around here -- see `Poly`'s coefficient
//! arithmetic, which has no division of any kind, deliberately.
//!
//! # Pseudo-division, not field division, for the one GCD that remains
//!
//! `Poly` (the coefficient ring) has no inverse operation, so the
//! univariate Euclidean algorithm over `UPoly<Poly>` cannot divide by a
//! non-constant leading coefficient the way it could when coefficients
//! were fraction-valued. It uses *pseudo-division* instead (a standard
//! technique for polynomial rings that aren't fields -- see e.g.
//! Geddes/Czapor/Labahn, "Algorithms for Computer Algebra", ch. 2): every
//! step only ever multiplies, adds, and subtracts `Poly` values, never
//! divides one. See [`UPoly::pseudo_div_rem`] for the algorithm and the
//! exact-division correctness argument for the two calls that use its
//! result to actually reduce `num`/`den`.

use crate::poly::{atom_rank, AtomId, AtomTable, Poly, Term};
use crate::BigScalar;

#[derive(Clone, Debug)]
pub(crate) struct RationalFunction {
    pub(crate) num: Poly,
    pub(crate) den: Poly,
}

impl RationalFunction {
    pub(crate) fn from_poly(p: Poly) -> Self {
        RationalFunction { num: p, den: Poly::constant(BigScalar::one()) }
    }

    // `add` is no longer reached from production code: `expr_to_rational`'s
    // `Add` case batches a whole sum over one common denominator via raw
    // `Poly` cross-multiplication and reduces once at the end (`from_raw`,
    // below) rather than folding pairwise through `add`. Kept for the test
    // suite (`adding_the_same_fraction_three_times_stays_at_degree_one`
    // exercises it directly, matching what the old per-term-reduce policy
    // used to do, still a meaningful regression check) -- not deleted,
    // `#[cfg(test)]`-gated instead.
    #[cfg(test)]
    pub(crate) fn add(&self, other: &Self, table: &mut AtomTable) -> Self {
        // a/b + c/d = (a*d + c*b) / (b*d)
        let num = self.num.mul(&other.den, table).add(&other.num.mul(&self.den, table));
        let den = self.den.mul(&other.den, table);
        reduce(num, den, table)
    }

    pub(crate) fn mul(&self, other: &Self, table: &mut AtomTable) -> Self {
        let num = self.num.mul(&other.num, table);
        let den = self.den.mul(&other.den, table);
        reduce(num, den, table)
    }

    pub(crate) fn pow(&self, n: i32, table: &mut AtomTable) -> Self {
        if n >= 0 {
            reduce(self.num.pow(n as u32, table), self.den.pow(n as u32, table), table)
        } else {
            let m = (-n) as u32;
            reduce(self.den.pow(m, table), self.num.pow(m, table), table)
        }
    }

    /// Combines a raw `(num, den)` pair (not necessarily reduced) via a
    /// single `reduce()` call -- lets a caller accumulate several terms'
    /// worth of arithmetic (e.g. `expr_to_rational`'s `Add` case,
    /// summing a whole `Expr::Add` over one common denominator) without
    /// reducing after every pairwise step, only once at the end.
    pub(crate) fn from_raw(num: Poly, den: Poly, table: &mut AtomTable) -> Self {
        reduce(num, den, table)
    }
}

thread_local! {
    static REDUCE_DEPTH: std::cell::Cell<u32> = std::cell::Cell::new(0);
    static TOTAL_REDUCE_CALLS: std::cell::Cell<u64> = std::cell::Cell::new(0);
    static GCD_BRANCH_CALLS: std::cell::Cell<u64> = std::cell::Cell::new(0);
    static LITERAL_ONE_RETURNS: std::cell::Cell<u64> = std::cell::Cell::new(0);
}

/// Part of the `ODEROM_REDUCE_STATS` performance investigation
/// (DESIGN-RATIONAL-FORM.md): every `RationalFunction::add`/`mul`/`pow`
/// reduces eagerly (on every operation, not lazily at output/comparison/
/// zero-test time), so if most of those reductions find no common factor
/// at all (`is_literal_one()`), that's the full GCD machinery paying for
/// an answer of "nothing to cancel" far more often than it needs to.
/// Reset/read once per top-level `normalize_via_rational_form` call.
pub(crate) fn reset_reduce_stats() {
    TOTAL_REDUCE_CALLS.with(|c| c.set(0));
    GCD_BRANCH_CALLS.with(|c| c.set(0));
    LITERAL_ONE_RETURNS.with(|c| c.set(0));
}

/// `(total reduce() calls, calls that reached the GCD branch, of those
/// how many found no common factor at all)`.
pub(crate) fn reduce_stats() -> (u64, u64, u64) {
    (TOTAL_REDUCE_CALLS.with(|c| c.get()), GCD_BRANCH_CALLS.with(|c| c.get()), LITERAL_ONE_RETURNS.with(|c| c.get()))
}

/// With the coefficient ring holding no denominators at all,
/// `reduce_inner` below never calls `reduce` again -- there is no second
/// level to recurse into, so this should never exceed depth 1 for any
/// well-formed input. Kept anyway as a loud, cheap defense-in-depth
/// tripwire (redundant on purpose, per the design discussion): if some
/// future change accidentally reintroduces a recursive path, this turns
/// it into an immediate, diagnosable panic instead of a silent hang.
const MAX_REDUCE_DEPTH: u32 = 64;

/// Reduces `num/den` by their GCD when `den` is non-constant (D-RF
/// section 2.3); otherwise returns `num/den` exactly as given -- correct,
/// just not cancelled (D-RF section 2.6).
fn reduce(num: Poly, den: Poly, table: &mut AtomTable) -> RationalFunction {
    TOTAL_REDUCE_CALLS.with(|c| c.set(c.get() + 1));
    let depth = REDUCE_DEPTH.with(|d| {
        let n = d.get() + 1;
        d.set(n);
        n
    });
    if depth > MAX_REDUCE_DEPTH {
        panic!(
            "reduce(): recursion depth exceeded {MAX_REDUCE_DEPTH} -- this should be structurally impossible now that the coefficient ring holds no denominators (see this module's doc comment); a future change likely reintroduced recursion.\nnum = {:#?}\nden = {:#?}",
            num.sorted_terms(table),
            den.sorted_terms(table)
        );
    }
    let result = reduce_inner(num, den, table);
    REDUCE_DEPTH.with(|d| d.set(d.get() - 1));
    result
}

/// `reduce_inner_candidate`'s result, always made primitive (D-RF's
/// content-management invariant, DESIGN-RATIONAL-FORM.md): `num`/`den`
/// share no common polynomial factor at all, full stop, regardless of
/// which of `reduce_inner_candidate`'s return paths produced them. Not a
/// per-case patch on the equalization step specifically -- an earlier
/// version left content-stripping out entirely (misjudged as a pure
/// optimization) and it compounded *across* a chain of dependent
/// `reduce()` calls: `reduce()` call N's leftover content becomes part of
/// call N+1's input, and if N+1 also fails to strip it, whatever
/// exponentiation N+1's own pseudo-division correction applies lands on
/// top of it too -- `(l^k)^k' = l^(k*k')`, multiplicative, not additive.
/// Applying this once here, uniformly, makes that compounding
/// structurally impossible: no call's output can ever hand the next call
/// a content factor to re-amplify.
///
/// Uses the full recursive multivariate `poly_gcd` (below), not a
/// monomial-only shortcut: an earlier version only stripped a shared
/// scalar-times-monomial factor, which fixed the single-variable
/// (Schwarzschild) case but not a genuine *polynomial* shared factor
/// (confirmed present for Reissner-Nordstrom: an `M^2-Q^2`-shaped
/// binomial factor common to `num` and `den`, invisible to a monomial-only
/// check since no single generator's exponent is uniformly shared across
/// its terms).
fn reduce_inner(num: Poly, den: Poly, table: &mut AtomTable) -> RationalFunction {
    let rf = reduce_inner_candidate(num, den, table);
    if rf.num.is_zero() {
        // Content of the zero polynomial is undefined; every path that
        // returns a zero numerator already normalizes den to 1 (or, for
        // the deliberately-opaque 0^-1 case, leaves it as given -- either
        // way there is nothing to strip).
        return rf;
    }
    let content = poly_gcd(&rf.num, &rf.den, table);
    let (num, den) = if content.is_literal_one() {
        (rf.num, rf.den)
    } else {
        let new_num = rf.num.exact_div(&content, table).unwrap_or_else(|| {
            panic!(
                "reduce_inner: poly_gcd(num, den) does not divide num exactly -- an implementation assumption is violated (DESIGN-RATIONAL-FORM.md).\ncontent = {:#?}\nnum = {:#?}",
                content.sorted_terms(table),
                rf.num.sorted_terms(table)
            )
        });
        let new_den = rf.den.exact_div(&content, table).unwrap_or_else(|| {
            panic!(
                "reduce_inner: poly_gcd(num, den) does not divide den exactly -- an implementation assumption is violated (DESIGN-RATIONAL-FORM.md).\ncontent = {:#?}\nden = {:#?}",
                content.sorted_terms(table),
                rf.den.sorted_terms(table)
            )
        });
        (new_num, new_den)
    };
    // Unlike `reduce_inner_candidate`'s own constant-denominator fast
    // path (which only ever sees the *original* `den` passed in), either
    // branch above -- content-stripped or not -- can still leave a
    // *constant* (not necessarily `1`) as the denominator: the full-GCD
    // branch's final reassembly can land there directly, and dividing by
    // poly_gcd's content can too (e.g. `f * f^-1` reducing to `-1/-1`
    // rather than `1/1`, found via `cancels_reciprocal_of_a_sum`: left as
    // the unevaluated `Expr` `-1 * (-1)^-1` instead of folding to the
    // literal `1`). Re-fold unconditionally, one last time, regardless of
    // which branch produced `num`/`den` -- except when `den` is the
    // literal zero polynomial: `den.terms.iter().all(..)` is vacuously
    // true for an empty `Vec`, the exact same trap
    // `reduce_inner_candidate`'s own `den.is_zero()` guard exists to
    // avoid (its comment explains it in full) -- without this guard here
    // too, `0^-1` (deliberately left opaque as `1/0` by that check)
    // reaches this second fold, `.first()` defaults its coefficient to
    // `1`, and `0^-1` silently becomes `1` one level up, e.g. inside
    // `(0^-1)^-1`. Found by `v1_and_v2_agree` on exactly that input.
    if !den.is_zero() && den.terms.iter().all(|t| t.generators.is_empty()) {
        let c = den.terms.first().map(|t| t.coeff.clone()).unwrap_or_else(BigScalar::one);
        if let Some(inv) = c.recip() {
            return RationalFunction { num: num.scale(inv), den: Poly::constant(BigScalar::one()) };
        }
    }
    RationalFunction { num, den }
}

fn reduce_inner_candidate(num: Poly, den: Poly, table: &mut AtomTable) -> RationalFunction {
    if num.is_zero() {
        return RationalFunction { num, den: Poly::constant(BigScalar::one()) };
    }
    // Division by literal zero (`den` the zero polynomial: *empty*
    // terms, not "one term with generators.is_empty()") is left opaque
    // rather than guessed at, same as the old engine's `scalar_pow`
    // ("0^negative: leave opaque"): `den.terms.iter().all(..)` is
    // vacuously true for an empty `Vec`, so without this check first,
    // 0^-1 fell straight into the constant fast path below with
    // `.first()` defaulting to `BigScalar::one()` -- silently turning
    // division by zero into "divide by one", caught by the differential
    // test (`v1_and_v2_agree`) directly.
    if den.is_zero() {
        return RationalFunction { num, den };
    }
    // A constant denominator has no pole variable to speak of -- fold
    // its scalar into num directly, cheaper than routing through the
    // univariate machinery for nothing.
    if den.terms.iter().all(|t| t.generators.is_empty()) {
        let c = den.terms.first().map(|t| t.coeff.clone()).unwrap_or(BigScalar::one());
        return match c.recip() {
            Some(inv) => RationalFunction { num: num.scale(inv), den: Poly::constant(BigScalar::one()) },
            None => RationalFunction { num, den },
        };
    }
    // Full reduction: `poly_gcd` (below) -- the same recursive
    // multivariate GCD `reduce_inner`'s content-stripping invariant uses.
    // An earlier version of this branch had its own bespoke pole-
    // selection-plus-raw-subresultant-PRS logic here, calling `gcd`
    // (the `UPoly`-level function, still used internally by `poly_gcd`)
    // directly on `num`/`den` as given -- exposed to the exact same
    // degree-0-pseudo-division artifact `poly_gcd`'s Gauss's-Lemma fix
    // exists for, but *without* that fix, because `num`/`den` here are
    // not first verified primitive with respect to the chosen pole the
    // way `poly_gcd`'s own recursive calls guarantee. Found via
    // `v1_and_v2_agree` (property-based, not the earlier hand-picked
    // cases): `(M+x)/(rM)` had its numerator's own `reduce_inner`
    // content-stripping panic ("does not divide num exactly") because
    // this branch had already corrupted the value -- pseudo-dividing by
    // a spurious degree-0-in-r "gcd" of `M+x` (which shares nothing with
    // `rM`) and reporting an exact division regardless, the same
    // artifact traced and fixed for `poly_gcd` directly. Delegating to
    // `poly_gcd` here removes the whole bespoke path instead of patching
    // it a second time.
    let content = poly_gcd(&num, &den, table);
    if content.is_literal_one() {
        return RationalFunction { num, den };
    }
    let final_num = num.exact_div(&content, table).unwrap_or_else(|| {
        panic!(
            "reduce_inner_candidate: poly_gcd(num, den) does not divide num exactly -- an implementation assumption is violated (DESIGN-RATIONAL-FORM.md).\ncontent = {:#?}\nnum = {:#?}",
            content.sorted_terms(table),
            num.sorted_terms(table)
        )
    });
    let final_den = den.exact_div(&content, table).unwrap_or_else(|| {
        panic!(
            "reduce_inner_candidate: poly_gcd(num, den) does not divide den exactly -- an implementation assumption is violated (DESIGN-RATIONAL-FORM.md).\ncontent = {:#?}\nden = {:#?}",
            content.sorted_terms(table),
            den.sorted_terms(table)
        )
    });
    RationalFunction { num: final_num, den: final_den }
}

/// A polynomial in one designated generator (the "pole variable"),
/// coefficients plain [`Poly`] -- a polynomial ring over the *other*
/// generators, with no denominator field and no operation that could
/// produce one (see this module's doc comment: this is the type-level
/// fix for the non-termination bug, not just this type's convention).
/// `coeffs[i]` is the coefficient of `pole^i`; no trailing zero entries.
#[derive(Clone, Debug)]
struct UPoly {
    coeffs: Vec<Poly>,
}

impl UPoly {
    fn is_zero(&self) -> bool {
        self.coeffs.is_empty()
    }

    fn degree(&self) -> Option<usize> {
        if self.coeffs.is_empty() {
            None
        } else {
            Some(self.coeffs.len() - 1)
        }
    }

    /// Re-verifies (not just restates `trim()`'s own trivial
    /// postcondition, but checks at the actual point of use) that
    /// `degree()` cannot be lying: no trailing zero coefficient, and
    /// every stored coefficient satisfies `Poly`'s own invariant (no
    /// zero-coefficient term, no nonsensical exponent). A violation here
    /// means some caller passed in (or some earlier operation produced)
    /// a `UPoly` that was mutated without a `trim()` afterward --
    /// DESIGN-RATIONAL-FORM.md investigation: this is exactly the shape
    /// of bug that would make a stale/wrong leading coefficient get read
    /// as `lc_b`, corrupting everything downstream in a way that doesn't
    /// surface until much later (the exponent-overflow panic in
    /// `poly::mul_term` was a late symptom, not the origin).
    fn debug_assert_trimmed(&self, label: &str) {
        if !cfg!(debug_assertions) {
            return;
        }
        debug_assert!(
            !matches!(self.coeffs.last(), Some(c) if c.is_zero()),
            "{label}: UPoly not properly trimmed -- trailing zero coefficient, degree() is lying"
        );
        for (i, c) in self.coeffs.iter().enumerate() {
            for term in &c.terms {
                debug_assert!(
                    !term.coeff.is_zero(),
                    "{label}: UPoly coefficient at pole-degree {i} has a stored zero-coefficient term: {term:?}"
                );
            }
        }
    }

    fn trim(mut self) -> Self {
        while matches!(self.coeffs.last(), Some(c) if c.is_zero()) {
            self.coeffs.pop();
        }
        // Postcondition, checked every time (this is the one place that's
        // supposed to establish it): no trailing zero coefficient left,
        // i.e. `degree()` (which just reads `coeffs.len() - 1`, it doesn't
        // search for the highest nonzero entry) cannot lie about which
        // position is genuinely the leading one. A violation here means
        // some `UPoly`-producing code path mutated `coeffs` directly
        // without going through `trim()` afterward (DESIGN-RATIONAL-FORM.md
        // investigation: this is exactly the failure mode that would make
        // `pseudo_div_rem` read a stale/wrong leading coefficient).
        debug_assert!(
            !matches!(self.coeffs.last(), Some(c) if c.is_zero()),
            "UPoly::trim postcondition violated: a trailing zero coefficient survived"
        );
        self
    }

    fn from_poly(p: &Poly, pole: AtomId) -> Self {
        let mut coeffs: Vec<Poly> = Vec::new();
        for term in &p.terms {
            let mut pole_exp = 0u32;
            let mut rest: Vec<(AtomId, u32)> = Vec::new();
            for &(id, exp) in &term.generators {
                if id == pole {
                    pole_exp = exp;
                } else {
                    rest.push((id, exp));
                }
            }
            let idx = pole_exp as usize;
            if coeffs.len() <= idx {
                coeffs.resize(idx + 1, Poly::zero());
            }
            let contribution = Poly { terms: vec![Term { coeff: term.coeff.clone(), generators: rest }] };
            debug_assert!(
                contribution.terms.iter().all(|t| t.generators.iter().all(|&(g, _)| g != pole)),
                "a coefficient must never contain the pole variable it was just separated from"
            );
            coeffs[idx] = coeffs[idx].add(&contribution);
        }
        UPoly { coeffs }.trim()
    }

    /// Reassembles into a plain [`Poly`] -- always exact, always a pure
    /// polynomial reconstruction (`sum coeffs[i] * pole^i`), because
    /// coefficients never carry a denominator to cross-multiply away.
    /// This is what "the reassembly step disappears" (as a source of
    /// recursion) means concretely: there is nothing left to reduce
    /// after this, so no call back into `reduce` is needed.
    fn to_poly(&self, pole: AtomId, table: &mut AtomTable) -> Poly {
        let mut result = Poly::zero();
        for (exp, coeff) in self.coeffs.iter().enumerate() {
            if coeff.is_zero() {
                continue;
            }
            let term_poly =
                if exp == 0 { coeff.clone() } else { coeff.mul(&Poly::generator(pole).pow(exp as u32, table), table) };
            result = result.add(&term_poly);
        }
        result
    }

    fn scale_by_poly(&self, factor: &Poly, table: &mut AtomTable) -> Self {
        UPoly { coeffs: self.coeffs.iter().map(|c| c.mul(factor, table)).collect() }.trim()
    }

    /// `self / divisor`, dividing every coefficient by `divisor` via
    /// `Poly::exact_div` -- `None` if any single coefficient doesn't
    /// divide exactly (subresultant PRS's own guarantee is that the
    /// *whole* `UPoly` divides exactly, not necessarily coefficient by
    /// coefficient in isolation in every intermediate representation, but
    /// in practice, for the divisors subresultant PRS produces here,
    /// per-coefficient division is what's needed and what the theory
    /// provides -- a `None` here is exactly the "PARE e me diga" signal
    /// the design decision called for).
    fn exact_div_by_poly(&self, divisor: &Poly, table: &mut AtomTable) -> Option<Self> {
        let mut coeffs = Vec::with_capacity(self.coeffs.len());
        for c in &self.coeffs {
            if c.is_zero() {
                coeffs.push(Poly::zero());
            } else {
                coeffs.push(c.exact_div(divisor, table)?);
            }
        }
        Some(UPoly { coeffs }.trim())
    }

    fn add(&self, other: &Self) -> Self {
        let len = self.coeffs.len().max(other.coeffs.len());
        let mut coeffs = Vec::with_capacity(len);
        for i in 0..len {
            let a = self.coeffs.get(i).cloned().unwrap_or_else(Poly::zero);
            let b = other.coeffs.get(i).cloned().unwrap_or_else(Poly::zero);
            coeffs.push(a.add(&b));
        }
        UPoly { coeffs }.trim()
    }

    fn neg(&self) -> Self {
        UPoly { coeffs: self.coeffs.iter().map(|c| c.neg()).collect() }
    }

    fn sub(&self, other: &Self) -> Self {
        self.add(&other.neg())
    }

    /// Pseudo-division: `(quotient, remainder, e)` such that
    /// `lc(other)^e * self = quotient * other + remainder`, with
    /// `deg(remainder) < deg(other)` (or `remainder` zero). Only ever
    /// multiplies, adds, and subtracts `Poly` values -- coefficient
    /// division is never attempted, because `Poly` has no such operation
    /// (see this module's doc comment). Always terminates: bounded by a
    /// fixed number of steps (below), each touching only already-known
    /// positions.
    ///
    /// `e` is **always** `deg(self) - deg(other) + 1` -- the classical,
    /// fixed pseudo-remainder exponent (not "however many steps an
    /// implementation happens to need"). This matters beyond this
    /// function: subresultant PRS (`gcd`, below) derives its `beta`/`psi`
    /// correction factors from exactly this fixed formula; an earlier
    /// version of this function counted only *actual* cancellation steps,
    /// which is fewer whenever `self` has an internal zero coefficient
    /// (routine for sparse polynomials) -- silently mismatched against
    /// what the subresultant theory assumes, which would have made its
    /// exact-division guarantee simply false. When there's nothing to
    /// cancel at a given step (`r`'s degree is already below `other`'s,
    /// but more fixed-formula steps remain), this still scales `r` by
    /// `lc(other)` for bookkeeping consistency -- multiplying a
    /// congruence by `lc(other)` preserves it, so this doesn't change the
    /// mathematical remainder, only keeps the exponent accounting exact.
    ///
    /// Rescaling `r` (the remainder-in-progress) by `lc(other)` every
    /// step is the pseudo-division mechanism itself, required for
    /// correctness -- it cannot be skipped or deferred (this is also
    /// exactly the mechanism behind naive-PRS coefficient growth: same
    /// thing, seen from operation-count and from coefficient-size
    /// respectively, not two separate costs -- subresultant PRS, not this
    /// function, is what controls that growth). What *can* be trimmed,
    /// measured via `ODEROM_REDUCE_STATS`, is redundant work around that
    /// core step, all constant-factor:
    /// - `q` (pure output, unlike `r`, never consulted by a later step)
    ///   is accumulated unscaled and scaled by the right power of
    ///   `lc(other)` once at the end, instead of being rescaled in full
    ///   on every step alongside `r`.
    /// - A step's leading coefficient is not computed and then discovered
    ///   to be zero -- it cancels exactly by construction (`term_coeff =
    ///   lc_r`, `other`'s leading coefficient is `lc_b`, and `r` was just
    ///   scaled by `lc_b`), so it's set directly.
    /// - Zero coefficients of `other` are skipped when forming the
    ///   subtraction (a `Poly::mul` on an empty-terms `Poly` is already
    ///   O(1), so this mainly avoids `Vec` bookkeeping, not arithmetic).
    fn pseudo_div_rem(&self, other: &Self, table: &mut AtomTable) -> Option<(UPoly, UPoly, u32)> {
        self.debug_assert_trimmed("pseudo_div_rem: self");
        other.debug_assert_trimmed("pseudo_div_rem: other");
        let od = other.degree()?;
        let lc_b = other.coeffs[od].clone();
        if self.is_zero() {
            return Some((UPoly { coeffs: vec![] }, UPoly { coeffs: vec![] }, 0));
        }
        let sd = self.degree().expect("checked is_zero above");
        if sd < od {
            return Some((UPoly { coeffs: vec![] }, self.clone(), 0));
        }
        let e = (sd - od + 1) as u32;
        if std::env::var("ODEROM_TRACE_PSEUDO_DIV").is_ok() {
            let lc_b_max_exp = lc_b.terms.iter().flat_map(|t| t.generators.iter().map(|&(_, e)| e)).max();
            eprintln!(
                "pseudo_div_rem: self.degree={:?} other.degree={:?} od={od} e={e} lc_b_terms={} lc_b_max_exp={:?}",
                self.degree(),
                other.degree(),
                lc_b.terms.len(),
                lc_b_max_exp,
            );
        }
        let trace_subres = std::env::var("ODEROM_TRACE_SUBRES").is_ok();
        let mut peak_terms = 0usize;
        let mut peak_bits = 0u64;
        let mut r = self.clone();
        let mut q_terms: Vec<(usize, Poly, u32)> = Vec::new();
        for step in 1..=e {
            let Some(rd) = r.degree() else {
                // `r` collapsed to exactly zero early -- 0 * lc_b = 0,
                // nothing left to do for any remaining fixed-formula step.
                break;
            };
            if rd < od {
                // Nothing of degree >= od to cancel this step, but more
                // fixed-formula steps remain: scale for bookkeeping only
                // (see this function's doc comment).
                r = r.scale_by_poly(&lc_b, table);
                continue;
            }
            let lc_r = r.coeffs[rd].clone();
            r = r.scale_by_poly(&lc_b, table);
            let shift = rd - od;
            // NOT `lc_r * lc_b`: `r` was just scaled by `lc_b` above, so
            // the term that cancels `r`'s new leading coefficient
            // (`lc_r * lc_b`, post-scaling) against `other`'s leading
            // coefficient (`lc_b` itself, unscaled) is `lc_r` alone --
            // `lc_r * lc_b` here would double-count the `lc_b` factor and
            // never actually cancel the leading term, so `r`'s degree
            // would never drop and this would loop forever (found via a
            // hang on the simplest possible single-variable case,
            // `-1536r^-9 + -384r^-7`).
            let term_coeff = lc_r;
            q_terms.push((shift, term_coeff.clone(), step));
            for (i, c) in other.coeffs.iter().enumerate() {
                if i == od || c.is_zero() {
                    // `i == od` is `r`'s leading position, guaranteed to
                    // cancel exactly -- set directly below rather than
                    // computed and subtracted here.
                    continue;
                }
                let pos = shift + i;
                let contribution = c.mul(&term_coeff, table);
                r.coeffs[pos] = r.coeffs[pos].sub(&contribution);
            }
            r.coeffs[rd] = Poly::zero();
            r = r.trim();
            if trace_subres {
                // Measurement 2: the intermediate peak *within* this one
                // pseudo_div_rem call, not just its final result -- if
                // this peak is orders of magnitude above the returned
                // remainder, the swell is happening (and reducing back
                // down) inside pseudo_div_rem itself, not carried forward
                // by subresultant PRS's beta-division.
                let terms_now = upoly_total_terms(&r);
                let bits_now = upoly_max_bits(&r);
                peak_terms = peak_terms.max(terms_now);
                peak_bits = peak_bits.max(bits_now);
            }
        }
        if trace_subres {
            eprintln!("pseudo_div_rem: internal peak during computation: peak_terms={peak_terms} peak_bits={peak_bits}");
        }
        // Powers of `lc_b` from 0 up to `e`, built incrementally (one
        // multiplication per power) and reused for every term below --
        // NOT `lc_b.pow(e - i, table)` computed separately per term
        // (`Poly::pow` is naive repeated multiplication, not fast
        // exponentiation by squaring): that would cost O(e) per term and
        // O(e^2) overall, which regressed one component from ~0.5s to 70s
        // when first tried, exactly the quadratic-in-iteration-count
        // blowup this whole rewrite exists to avoid.
        let mut lc_b_powers = Vec::with_capacity(e as usize + 1);
        lc_b_powers.push(Poly::constant(BigScalar::one()));
        for _ in 0..e {
            lc_b_powers.push(lc_b_powers.last().expect("just pushed").mul(&lc_b, table));
        }
        let max_shift = q_terms.iter().map(|&(s, _, _)| s).max();
        let mut q = UPoly { coeffs: vec![] };
        if let Some(max_shift) = max_shift {
            q.coeffs = vec![Poly::zero(); max_shift + 1];
            for (shift, coeff, step) in q_terms {
                // This term was produced at step `step`; had `q` kept
                // being rescaled by `lc_b` every step the naive way, it
                // would have picked up `e - step` more factors of `lc_b`
                // by the end -- apply that once here instead.
                q.coeffs[shift] = coeff.mul(&lc_b_powers[(e - step) as usize], table);
            }
        }
        Some((q.trim(), r, e))
    }
}

/// Subresultant PRS (Collins/Brown), not primitive PRS: computing
/// `content()` at every step (primitive PRS) would need polynomial GCD
/// *inside* the coefficient ring -- exactly the design divergence D-RF.1
/// forbids, and the wall an earlier, naive pseudo-division-based `gcd`
/// hit directly (confirmed: a chain of dependent `reduce()` calls let
/// pseudo-division's spurious leading-coefficient factor compound
/// multiplicatively, traced as an exact geometric ratio between
/// successive exponents -- not the polynomial naive-PRS-within-one-GCD
/// growth this project already budgets for). Subresultant theory instead
/// *predicts* that spurious factor exactly, from the previous step's
/// leading coefficients and degree differences (`beta`/`psi` below), and
/// divides it out via `Poly::exact_div`/`UPoly::exact_div_by_poly` --
/// exact division only, guaranteed exact by the theory itself, never a
/// search for a common factor. Coefficient growth this way stays
/// polynomial in the input degrees, not exponential.
///
/// Reference: G. E. Collins, "Subresultants and Reduced Polynomial
/// Remainder Sequences" (1967); W. S. Brown, "On Euclid's Algorithm and
/// the Theory of Subresultants" (1971); formula per the standard
/// presentation (e.g. Wikipedia, "Polynomial greatest common divisor" §
/// "Subresultant pseudo-remainder sequences"). Depends on
/// `pseudo_div_rem`'s `e` always being the *fixed* classical exponent
/// `deg(dividend) - deg(divisor) + 1` (see that function's doc comment)
/// -- the `beta`/`psi` recurrence below is derived assuming exactly that
/// convention, not "however many steps a particular implementation
/// happens to take".
///
/// Always terminates: `r_cur`'s degree strictly decreases every
/// iteration (`pseudo_div_rem`'s own contract), a natural number bounded
/// below by "zero, or the loop already exited".
fn upoly_max_bits(u: &UPoly) -> u64 {
    u.coeffs.iter().flat_map(|c| c.terms.iter().map(|t| t.coeff.bit_length_estimate())).max().unwrap_or(0)
}

fn upoly_total_terms(u: &UPoly) -> usize {
    u.coeffs.iter().map(|c| c.terms.len()).sum()
}

thread_local! {
    static GCD_CALL_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn gcd(a: UPoly, b: UPoly, table: &mut AtomTable) -> UPoly {
    let trace = std::env::var("ODEROM_TRACE_SUBRES").is_ok();
    let call_id = GCD_CALL_ID.with(|c| {
        let n = c.get() + 1;
        c.set(n);
        n
    });
    let (mut r_prev, mut r_cur) = if a.degree() >= b.degree() { (a, b) } else { (b, a) };
    let mut gamma_prev: Option<Poly> = None;
    let mut psi_prev: Option<Poly> = None;
    let mut delta_prev: Option<u32> = None;
    let mut first = true;
    let mut iter = 0u32;
    while !r_cur.is_zero() {
        // A single iteration here is the actual expensive step (a
        // pseudo-division plus an exact division, both possibly over
        // polynomials with large coefficients) -- checked once per
        // iteration, not just once per whole `gcd` call, so a metric
        // whose PRS needs many steps stays cancellable throughout, not
        // only at the start.
        crate::cancel::check_cancelled();
        iter += 1;
        let d_prev = r_prev.degree().expect("r_prev is nonzero: either the caller-guaranteed nonzero input, or a former r_cur that just passed the loop condition");
        let d_cur = r_cur.degree().expect("loop condition just checked r_cur is not zero");
        let d = (d_prev - d_cur) as u32;
        let gamma_cur = r_cur.coeffs[d_cur].clone();
        if trace {
            eprintln!(
                "subres[call={call_id} iter={iter}]: d_prev={d_prev} d_cur={d_cur} d={d} r_cur_terms={} r_cur_max_bits={}",
                upoly_total_terms(&r_cur),
                upoly_max_bits(&r_cur)
            );
        }

        let (psi_cur, beta) = if first {
            let beta = Poly::constant(if (d + 1) % 2 == 0 { BigScalar::one() } else { -BigScalar::one() });
            let psi = Poly::constant(-BigScalar::one());
            (psi, beta)
        } else {
            let gamma_prev_v = gamma_prev.expect("set on a prior iteration since `first` is false");
            let psi_prev_v = psi_prev.expect("set on a prior iteration since `first` is false");
            // psi_i = (-gamma_{i-1})^delta_{i-1} * psi_{i-1}^(1-delta_{i-1})
            // -- delta_{i-1} (this iteration's *previous* degree gap), NOT
            // `d` (this iteration's *current* gap, used below for beta_i
            // instead): using `d` here was a real bug (found via
            // property-based fuzzing on a random small expression, not
            // hypothetical) that stayed invisible through every case
            // tried by hand, including Reissner-Nordstrom, because it only
            // produces a wrong answer when two *consecutive* degree gaps
            // differ -- the common case of every gap being 1 (or two equal
            // consecutive gaps of any size) makes `d` and delta_{i-1}
            // coincide, silently masking it. delta_{i-1} == 0 is a real,
            // reachable case (equal-degree inputs on the very first
            // pseudo-division), so the exponent on psi_prev_v is handled
            // as 1-delta_{i-1} directly (never `delta_{i-1} - 1` on the
            // u32, which would underflow at delta_{i-1} == 0) via a
            // multiply-then-divide split on whether delta_{i-1} is 0, 1,
            // or >= 2.
            let delta_prev_v = delta_prev.expect("set on a prior iteration since `first` is false");
            let numerator = gamma_prev_v.neg().pow(delta_prev_v, table);
            let psi = match delta_prev_v {
                0 => psi_prev_v,
                1 => numerator,
                _ => {
                    let denom = psi_prev_v.pow(delta_prev_v - 1, table);
                    numerator.exact_div(&denom, table).unwrap_or_else(|| {
                        panic!(
                            "subresultant PRS: psi_i division not exact -- an implementation assumption is violated, not something to fall back from (DESIGN-RATIONAL-FORM.md).\nnumerator = {:#?}\ndenom = {:#?}",
                            numerator.sorted_terms(table),
                            denom.sorted_terms(table)
                        )
                    })
                }
            };
            let beta = gamma_prev_v.neg().mul(&psi.pow(d, table), table);
            (psi, beta)
        };
        if trace {
            eprintln!(
                "subres[call={call_id} iter={iter}]: beta_terms={} beta_max_bits={} (measurement 3: this is the theory-predicted factor, not a fallback)",
                beta.terms.len(),
                beta.terms.iter().map(|t| t.coeff.bit_length_estimate()).max().unwrap_or(0)
            );
        }

        let __t_prem = std::time::Instant::now();
        let (_, prem, _e) = r_prev
            .pseudo_div_rem(&r_cur, table)
            .expect("r_cur is nonzero (loop condition); pseudo_div_rem only returns None for a zero divisor");
        if trace {
            eprintln!(
                "subres[call={call_id} iter={iter}]: prem computed in {:?}, prem_terms={} prem_max_bits={}",
                __t_prem.elapsed(),
                upoly_total_terms(&prem),
                upoly_max_bits(&prem)
            );
        }

        if prem.is_zero() {
            return r_cur;
        }

        let __t_div = std::time::Instant::now();
        let r_next = prem.exact_div_by_poly(&beta, table).unwrap_or_else(|| {
            panic!(
                "subresultant PRS: remainder division by beta_i not exact -- an implementation assumption is violated, not something to fall back from (DESIGN-RATIONAL-FORM.md).\nbeta = {:#?}",
                beta.sorted_terms(table)
            )
        });
        // Measurement 3, independent check: verify r_next * beta == prem
        // exactly (not just trusting exact_div_by_poly's own internal
        // logic returned Some) -- recompute via a raw multiplication and
        // compare, catching a bug where the division "succeeds" via a
        // wrong factor.
        let check = r_next.scale_by_poly(&beta, table).sub(&prem);
        if !check.is_zero() {
            panic!(
                "subresultant PRS: independent verification failed -- r_next * beta != prem, the exact division was wrong despite exact_div_by_poly reporting success.\ncheck (should be zero) = {:?}",
                check.coeffs.iter().map(|c| c.sorted_terms(table)).collect::<Vec<_>>()
            );
        }
        if trace {
            eprintln!(
                "subres[call={call_id} iter={iter}]: exact_div_by_poly beta done in {:?}, VERIFIED r_next*beta==prem, r_next_terms={} r_next_max_bits={}",
                __t_div.elapsed(),
                upoly_total_terms(&r_next),
                upoly_max_bits(&r_next)
            );
        }

        r_prev = r_cur;
        r_cur = r_next;
        gamma_prev = Some(gamma_cur);
        psi_prev = Some(psi_cur);
        delta_prev = Some(d);
        first = false;
    }
    r_prev
}

/// Recursive multivariate polynomial GCD -- content computed via genuine
/// polynomial GCD in the coefficient ring, now explicitly permitted (only
/// *fraction*-valued coefficients -- a denominator, `reduce()`, `pow(-1)`
/// -- are forbidden there; that was the actual cause of the `r -> M -> r`
/// ping-pong this whole engine was redesigned around, not GCD itself).
///
/// Standard algorithm for a GCD domain (e.g. Geddes/Czapor/Labahn ch. 7):
/// at each level, pick the highest-priority variable present in either
/// input (the single fixed order, `poly::atom_rank`, D-RF.4) as the
/// "main" variable `x`. Split each input into its content in `x` (the GCD
/// of its `UPoly`-in-`x` coefficients -- themselves polynomials with `x`
/// structurally absent, per `UPoly::from_poly`'s own invariant) and its
/// primitive part (the content divided out exactly). The content-GCD is
/// computed by a RECURSIVE call to this same function, one level down;
/// the primitive-part GCD is computed by the existing non-recursive
/// subresultant PRS (`gcd`, above) over `UPoly<Poly>`, which never itself
/// needs a coefficient-ring GCD (only exact division, guaranteed exact by
/// subresultant theory) -- so the recursion here is exactly one level per
/// content computation, not one level per Euclidean step.
///
/// Terminates because the variable count strictly decreases every
/// recursive call: a coefficient of a `UPoly`-in-`x` can never itself
/// contain `x` (checked directly, not just argued -- `max_vars` threads
/// the previous level's count through so every recursive call can
/// `debug_assert` its own count is strictly smaller). Base case: no
/// variables left at all, both inputs are plain `BigScalar` constants --
/// ordinary integer GCD, already implemented (`BigScalar::gcd`).
fn poly_gcd(a: &Poly, b: &Poly, table: &mut AtomTable) -> Poly {
    // Suppressed for the *entire* recursive descent (every nested
    // `poly_gcd_bounded` call, including its own content-GCD recursion,
    // reads the same thread-local) -- see `TrigRewriteSuppressor` and
    // `mul_term`'s doc comment in poly.rs for why mid-computation
    // cos^2 -> 1-sin^2 rewriting breaks subresultant PRS's exactness
    // guarantee. `a`/`b` themselves arrive already in normal form (built
    // via ordinary, unsuppressed arithmetic in `expr_to_rational`), so
    // this only affects new cos-powers the GCD's own arithmetic
    // introduces internally (e.g. squaring a leading coefficient for
    // beta_i). Restored to normal form exactly once, on the result, so
    // every caller outside this function keeps seeing the usual
    // cos-degree<=1 canonical form.
    let result = {
        let _suppress = crate::poly::TrigRewriteSuppressor::new();
        poly_gcd_bounded(a, b, table, None)
    };
    result.normalize_trig(table)
}

fn poly_gcd_bounded(a: &Poly, b: &Poly, table: &mut AtomTable, max_vars: Option<usize>) -> Poly {
    // Once per recursive call (one per eliminated variable, not a tight
    // loop by itself) -- the actually-unbounded cost per level is the
    // `gcd` (subresultant PRS) call below, which checks its own loop
    // directly; this catches the case of many variables each needing a
    // cheap check between levels.
    crate::cancel::check_cancelled();
    if a.is_zero() {
        return b.clone();
    }
    if b.is_zero() {
        return a.clone();
    }
    let mut vars: Vec<AtomId> = Vec::new();
    for t in a.terms.iter().chain(b.terms.iter()) {
        for &(id, exp) in &t.generators {
            if exp != 0 && !vars.contains(&id) {
                vars.push(id);
            }
        }
    }
    if let Some(max) = max_vars {
        debug_assert!(
            vars.len() < max,
            "poly_gcd: recursive call did not strictly decrease variable count ({} >= {}) -- the pole-elimination invariant is violated; this is the r->M->r ping-pong in new clothes (DESIGN-RATIONAL-FORM.md)",
            vars.len(),
            max
        );
    }
    if vars.is_empty() {
        // Both are plain BigScalar constants -- ordinary integer GCD.
        let ca = a.terms.first().map(|t| t.coeff.clone()).unwrap_or_else(BigScalar::zero);
        let cb = b.terms.first().map(|t| t.coeff.clone()).unwrap_or_else(BigScalar::zero);
        return Poly::constant(ca.gcd(&cb));
    }
    vars.sort_by_key(|&id| atom_rank(table.key(id)));
    let x = vars[0];
    let this_level_var_count = vars.len();
    // `ODEROM_TRACE_POLE`: this is the only place a pole variable is
    // chosen now (an earlier version of `reduce_inner_candidate` picked
    // one directly; that whole bespoke path was replaced by a direct
    // `poly_gcd` call after a real correctness bug was found in it -- see
    // this function's other caller). `ODEROM_TRACE_POLYGCD` shows the
    // same choice plus the content/primitive-part values around it.
    let trace_pole = std::env::var("ODEROM_TRACE_POLE").is_ok();
    let trace = trace_pole || std::env::var("ODEROM_TRACE_POLYGCD").is_ok();
    if trace_pole {
        eprintln!("poly_gcd_bounded: pole={:?} (level has {this_level_var_count} variable(s))", table.key(x));
    }
    if trace {
        eprintln!(
            "poly_gcd_bounded: x={:?} a={:?} b={:?}",
            table.key(x),
            a.sorted_terms(table),
            b.sorted_terms(table)
        );
    }

    let a_u = UPoly::from_poly(a, x);
    let b_u = UPoly::from_poly(b, x);

    let content_a =
        a_u.coeffs.iter().fold(Poly::zero(), |acc, c| poly_gcd_bounded(&acc, c, table, Some(this_level_var_count)));
    let content_b =
        b_u.coeffs.iter().fold(Poly::zero(), |acc, c| poly_gcd_bounded(&acc, c, table, Some(this_level_var_count)));
    let content_gcd = poly_gcd_bounded(&content_a, &content_b, table, Some(this_level_var_count));
    if trace {
        eprintln!(
            "poly_gcd_bounded: x={:?} content_a={:?} content_b={:?} content_gcd={:?}",
            table.key(x),
            content_a.sorted_terms(table),
            content_b.sorted_terms(table),
            content_gcd.sorted_terms(table)
        );
    }

    // NOT `a.exact_div(&content_gcd, ...)`: each input must be divided by
    // its OWN full content (`content_a`, `content_b`), not by the
    // smaller shared `content_gcd` -- dividing both by only the shared
    // part leaves a residual factor of `content_a/content_gcd` (resp.
    // `content_b/content_gcd`) sitting in the "primitive" part, which the
    // x-univariate step below then mistakes for a genuine x-independent
    // divisor of the OTHER side (found via the minimal reproducer:
    // gcd(4M^2, 2Mr) computed as 4M^2 -- larger than `2Mr` itself, an
    // impossible result for a GCD -- because primitive_a ended up `2M`
    // instead of fully stripped to `1`, and the subresultant step then
    // treated that leftover constant-in-r as automatically dividing `r`).
    let primitive_a = a.exact_div(&content_a, table).unwrap_or_else(|| {
        panic!(
            "poly_gcd: content_a does not divide a exactly -- an implementation assumption is violated (DESIGN-RATIONAL-FORM.md).\ncontent_a = {:#?}\na = {:#?}",
            content_a.sorted_terms(table),
            a.sorted_terms(table)
        )
    });
    let primitive_b = b.exact_div(&content_b, table).unwrap_or_else(|| {
        panic!(
            "poly_gcd: content_b does not divide b exactly -- an implementation assumption is violated (DESIGN-RATIONAL-FORM.md).\ncontent_b = {:#?}\nb = {:#?}",
            content_b.sorted_terms(table),
            b.sorted_terms(table)
        )
    });

    let primitive_a_u = UPoly::from_poly(&primitive_a, x);
    let primitive_b_u = UPoly::from_poly(&primitive_b, x);
    let primitive_gcd_u = gcd(primitive_a_u, primitive_b_u, table);
    // Gauss's Lemma says the GCD of two polys that are each already
    // primitive in x must itself be primitive in x. But `primitive_gcd_u`
    // is the *raw* subresultant PRS result, which is only guaranteed
    // correct up to a spurious content factor drawn from the coefficient
    // ring (Poly-in-the-other-variables) -- classical subresultant theory
    // scales away the pseudo-division-induced growth via beta/psi, but
    // does not (and cannot, since it works one PRS step at a time)
    // guarantee the final nonzero remainder is itself content-free: found
    // via the minimal reproducer gcd(r^2-2Mr, r^2-4Mr+4M^2) (both already
    // primitive in r), whose PRS chain legitimately produces a last
    // remainder of `-2Mr+4M^2` = `-2M*(r-2M)` -- a genuine remainder, but
    // carrying an extra ring-content factor `2M` that shares nothing
    // with either input. The fix is the same content()/primitive_part()
    // step already applied to the *inputs* (content_a/content_b above),
    // applied once more to this *output*: strip the GCD of its own
    // x-coefficients before accepting it. Degree-0 is the special case
    // of this general rule (a degree-0 poly's only "coefficient" is
    // itself, so stripping its own content always collapses it to 1) --
    // no longer handled separately.
    let raw_gcd_content = primitive_gcd_u
        .coeffs
        .iter()
        .fold(Poly::zero(), |acc, c| poly_gcd_bounded(&acc, c, table, Some(this_level_var_count)));
    let raw_gcd_poly = primitive_gcd_u.to_poly(x, table);
    let primitive_gcd = if raw_gcd_content.is_literal_one() {
        raw_gcd_poly
    } else {
        raw_gcd_poly.exact_div(&raw_gcd_content, table).unwrap_or_else(|| {
            panic!(
                "poly_gcd: raw_gcd_content does not divide the raw subresultant gcd result exactly -- an implementation assumption is violated (DESIGN-RATIONAL-FORM.md).\nraw_gcd_content = {:#?}\nraw_gcd_poly = {:#?}",
                raw_gcd_content.sorted_terms(table),
                raw_gcd_poly.sorted_terms(table)
            )
        })
    };

    let result = content_gcd.mul(&primitive_gcd, table);
    if trace {
        eprintln!(
            "poly_gcd_bounded: x={:?} primitive_a={:?} primitive_b={:?} primitive_gcd={:?} => result={:?}",
            table.key(x),
            primitive_a.sorted_terms(table),
            primitive_b.sorted_terms(table),
            primitive_gcd.sorted_terms(table),
            result.sorted_terms(table)
        );
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Expr;

    /// Builds `num/den` via the real `Expr -> RationalFunction`
    /// converter (not a separate test-only mini-converter): `den` here
    /// routinely contains negative exponents (`1/r`), which only
    /// `expr_to_rational`'s `Pow` handling -- via `RationalFunction::pow`,
    /// which flips num/den for a negative exponent -- represents
    /// correctly; a bare `Poly` cannot hold a negative exponent at all
    /// (an earlier version of this helper cast the negative exponent to
    /// `u32` directly and looped effectively forever).
    fn ratio(num: &Expr, den: &Expr, table: &mut AtomTable) -> RationalFunction {
        let n = crate::canonical::expr_to_rational(num, table);
        let d = crate::canonical::expr_to_rational(den, table);
        n.mul(&d.pow(-1, table), table)
    }

    #[test]
    fn adding_the_same_fraction_three_times_stays_at_degree_one() {
        // 1/f + 1/f + 1/f = 3/f -- degree of f in the denominator must
        // stay 1, not grow to 3 (the bug rationalize() has today).
        let mut t = AtomTable::new();
        let r = Expr::var("r");
        let f = Expr::one() - Expr::int(2) * Expr::var("M") / r.clone();
        let one = Expr::one();
        let one_over_f = ratio(&one, &f, &mut t);
        let sum = one_over_f.add(&one_over_f, &mut t);
        let sum = sum.add(&one_over_f, &mut t);
        assert_eq!(degree_in(&sum.den, &mut t, "r"), 1, "den={:?}", sum.den.sorted_terms(&t));
    }

    fn degree_in(p: &Poly, table: &mut AtomTable, var_name: &str) -> u32 {
        let id = table.var(var_name);
        p.terms.iter().flat_map(|t| t.generators.iter()).filter(|(g, _)| *g == id).map(|(_, e)| *e).max().unwrap_or(0)
    }

    #[test]
    fn reduces_a_simple_common_factor() {
        // (r^2) / r = r
        let mut t = AtomTable::new();
        let r = Expr::var("r");
        let ratio_val = ratio(&r.clone().pow(2), &r, &mut t);
        assert_eq!(degree_in(&ratio_val.den, &mut t, "r"), 0);
        assert_eq!(degree_in(&ratio_val.num, &mut t, "r"), 1);
    }

    #[test]
    fn reduce_of_one_over_one_terminates_immediately() {
        let mut t = AtomTable::new();
        let r = reduce(Poly::constant(BigScalar::one()), Poly::constant(BigScalar::one()), &mut t);
        assert!(!r.num.is_zero());
    }
}

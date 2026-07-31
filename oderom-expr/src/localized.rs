//! Structured-denominator rational form (DESIGN-RATIONAL-FORM.md section
//! 8): a rational function as `(numerator: Poly, multiset of (generator,
//! exponent))`, where each generator is a polynomial factor localized at
//! -- known up front to be worth tracking as its own "pole" -- rather
//! than discovered by the general recursive multivariate GCD
//! (`rational_function.rs`'s `poly_gcd`) on every operation. That
//! general engine is what section 7.1 measured failing to fully collapse
//! for Kerr: its content/primitive-part recursion picks one "pole
//! variable" per level, which never terminates cleanly for a genuinely
//! bivariate denominator like `Sigma = r^2 + a^2*cos(theta)^2`. Here,
//! `Sigma` (or `Delta`, or `sin(theta)`) is not discovered mid-GCD at
//! all -- it is handed in from the metric's own declared denominators
//! (plus `metric_inverse`'s block determinants), so multiplication and
//! division never call GCD (`den` exponents just add/subtract), and
//! addition only ever needs an exact-divisibility test against each
//! *known* factor, not a search for an unknown one.
//!
//! Two hard requirements (per the round that authorized this design):
//!
//! 1. **Canonical, not merely correct**: after every combination that
//!    could introduce new cancellation (every `add`, and every `mul`,
//!    since a numerator on one side can equal a known denominator
//!    generator on the other -- `Sigma/1 * 1/Sigma` must collapse to
//!    `1/1`, not stay `Sigma/Sigma`), the numerator is divided by every
//!    generator in the resulting denominator as many times as it exactly
//!    divides ([`Poly::exact_div`], never a GCD search). Skipping this
//!    is exactly what left Kerr's Ricci merely *smaller* instead of
//!    *zero* before this round.
//! 2. **The generator set is derived, never hardcoded**: [`LocalizationContext`]
//!    starts from whatever `Expr`s a caller seeds it with (this
//!    project's real caller, `oderom-components`, seeds it from a
//!    metric's own declared denominators and `metric_inverse`'s block
//!    determinants -- never a literal `[Sigma, Delta]` written by hand
//!    for Kerr specifically) and can *grow*: a denominator encountered
//!    during computation that isn't yet known is admitted as a new
//!    generator if it is square-free and coprime to every generator
//!    already known (`admit_or_fallback`); otherwise (or if that check
//!    can't be done cheaply) it is folded into an `overflow` factor
//!    handled by the existing general engine
//!    (`RationalFunction::from_raw`, `rational_function.rs`) -- slower,
//!    never wrong (D-RF section 2.6's contract, inherited unchanged).
//!
//! Explicitly out of scope, on purpose (see DESIGN-RATIONAL-FORM.md
//! section 7.2/8.4): Godel's `exp(a)^n` never fusing into `exp(n*a)` is
//! a different mechanism (an exponential-generator identity, not a
//! denominator-localization problem) and is not attacked here.

use crate::canonical::poly_to_expr;
use crate::poly::{AtomTable, Poly};
use crate::rational_function::{poly_gcd, RationalFunction};
use crate::BigScalar;
use crate::Expr;

/// Same shape as `oderom_components::curvature::Checkpoint` -- mirrored,
/// not shared, because `oderom-expr` cannot depend on
/// `oderom-components` (the dependency runs the other way). Checked
/// once per component by a caller's outer loop (coarse -- see
/// `curvature.rs`'s `*_localized_checkpointed` functions) and, inside
/// this module, exactly once more at the one place execution can leave
/// the localized representation for the general engine
/// (`fallback_to_general_engine`) -- never inside `Poly`'s own
/// arithmetic, which is cheap enough (28ms/672ms measured end to end,
/// DESIGN-RATIONAL-FORM.md section 8.5) that finer-grained checking
/// would cost more than it protects.
pub type Checkpoint<'a> = &'a mut dyn FnMut() -> bool;

/// What escaped the localization set and had to fall back to the
/// general engine, at the moment the caller's execution budget had
/// already run out -- named precisely so a real hang has an actionable
/// next step (DESIGN-RATIONAL-FORM.md section 8's own rule: this is
/// exactly the input to "admit this factor as a generator or not").
/// `oderom-components::curvature`'s `ComponentError::LocalizationFallbackBudgetExceeded`
/// wraps this with the one piece of context this crate doesn't have --
/// which tensor component was being computed.
#[derive(Debug, Clone)]
pub struct LocalizationBudgetExceeded {
    /// The denominator expression that did not belong to the known
    /// generator set and forced the general-engine fallback.
    pub denominator: Expr,
    /// Every generator known to `ctx` at the moment the budget ran out
    /// (seeds plus anything admitted mid-computation), in acceptance
    /// order.
    pub generators: Vec<Expr>,
}

impl std::fmt::Display for LocalizationBudgetExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let gens = self.generators.iter().map(|g| g.to_string()).collect::<Vec<_>>().join(", ");
        write!(f, "denominador `{}` saiu do conjunto de geradores localizados [{gens}] e o motor geral não terminou dentro do orçamento", self.denominator)
    }
}

impl std::error::Error for LocalizationBudgetExceeded {}

/// The *only* place `RationalFunction`'s general engine gets invoked
/// from within this module (DESIGN-RATIONAL-FORM.md section 8's
/// "unify the fallback boundary" rule) -- both call sites that used to
/// call `RationalFunction::from_raw`/`.pow()` directly now go through
/// here, so "the fallback boundary always carries the execution budget"
/// is a structural guarantee (one function to audit), not a convention
/// that depends on every future call site remembering to check.
fn fallback_to_general_engine(num: Poly, den: Poly, ctx: &mut LocalizationContext, checkpoint: Checkpoint) -> Result<RationalFunction, LocalizationBudgetExceeded> {
    if checkpoint() {
        let denominator = poly_to_expr(&den, &ctx.table);
        let generators = ctx.generator_sources().into_iter().cloned().collect();
        return Err(LocalizationBudgetExceeded { denominator, generators });
    }
    Ok(RationalFunction::from_raw(num, den, &mut ctx.table))
}

/// One localization generator: its canonical `Poly` form (built once,
/// against `LocalizationContext`'s own persistent [`AtomTable`]) and the
/// `Expr` it came from, kept only for diagnostics/logging.
struct Generator {
    poly: Poly,
    source: Expr,
}

/// Shared, growing state for one localized computation (DESIGN-RATIONAL-
/// FORM.md section 8.4, requirement 2): one metric's worth of
/// Christoffel/Riemann/Ricci/Kretschmann, typically -- never a
/// thread-local or global (see `poly.rs`'s own `TrigRewriteSuppressor`
/// doc comment for exactly the fragility that would reintroduce under
/// future parallelization; this is instead an explicit, owned value
/// threaded through calls, the same convention `Checkpoint` already
/// established in `oderom-components::curvature`).
///
/// Owns its own [`AtomTable`], persistent across every
/// [`normalize_localized`] call made with it -- a deliberate departure
/// from D-RF.5 ("one `AtomTable` per top-level `normalize()` call"),
/// necessary so that `Sigma` discovered while inverting the metric and
/// `Sigma` encountered again three stages later in `Kretschmann` are
/// recognized as the *same* generator (same `AtomId`s) rather than two
/// structurally-equal-but-uncomparable polynomials in two unrelated
/// tables.
pub struct LocalizationContext {
    table: AtomTable,
    generators: Vec<Generator>,
    /// Set once, the first time a generator admission or fallback
    /// happens, so a caller/test can assert "no fallback occurred"
    /// without scraping stderr.
    fallback_log: Vec<String>,
}

impl LocalizationContext {
    /// A new context seeded with `seed_generators` (a metric's own
    /// declared denominators plus `metric_inverse`'s block determinants,
    /// for this project's real caller). Each seed goes through the exact
    /// same check a generator discovered *during* computation does
    /// (`classify_or_admit`, DESIGN-RATIONAL-FORM.md section 8.2:
    /// square-free and coprime to every seed already accepted) rather
    /// than being trusted blindly -- a caller that seeds two
    /// non-coprime denominators (or the same one twice, differently
    /// shaped) gets graceful degradation (the later, non-qualifying seed
    /// is simply not admitted, recorded in `fallback_log`, and handled
    /// via the general engine if it's ever needed as a denominator),
    /// never a broken invariant silently assumed to hold downstream.
    pub fn new(seed_generators: &[Expr]) -> Self {
        let mut ctx = LocalizationContext { table: AtomTable::new(), generators: Vec::new(), fallback_log: Vec::new() };
        for g in seed_generators {
            let poly = plain_poly_of(g, &mut ctx.table);
            classify_or_admit(&poly, &mut ctx);
        }
        ctx
    }

    /// Every fallback/admission event logged so far -- empty means the
    /// known generator set (seed + anything admitted) covered every
    /// denominator encountered; a test asserting "this computation never
    /// fell back to the general engine" should assert this is empty.
    pub fn fallback_log(&self) -> &[String] {
        &self.fallback_log
    }

    pub fn generator_count(&self) -> usize {
        self.generators.len()
    }

    /// Every generator's original source `Expr` (seed or admitted
    /// mid-computation), in the order they were accepted -- for
    /// diagnostics/logging, mirroring `fallback_log`'s role.
    pub fn generator_sources(&self) -> Vec<&Expr> {
        self.generators.iter().map(|g| &g.source).collect()
    }
}

/// Converts `e` to a plain `Poly` (no denominator) via the existing
/// general engine (`rational_function::RationalFunction`) run once
/// against `table` -- correct for every generator this project's metrics
/// actually declare (a metric component's own denominator, or a block
/// determinant, is always already a plain polynomial expression, never
/// itself a fraction). Panics if `e` turns out to carry its own
/// denominator: a seed/admitted generator with a nested fraction is
/// outside this round's scope (DESIGN-RATIONAL-FORM.md section 8 never
/// claims to handle that) and should be caught here, loudly, rather than
/// silently mishandled downstream.
fn plain_poly_of(e: &Expr, table: &mut AtomTable) -> Poly {
    let rf = crate::canonical::expr_to_rational(e, table);
    assert!(
        rf.den.is_literal_one(),
        "localized.rs: generator {e:?} normalized with a nontrivial denominator of its own -- nested-fraction generators are outside this round's scope (DESIGN-RATIONAL-FORM.md section 8)"
    );
    rf.num
}

/// `num / (generators^exponents * overflow)`. `overflow` is `1`
/// (`Poly::constant(BigScalar::one())`) whenever every denominator
/// factor encountered so far was recognized as (or admitted as) a known
/// generator -- the common case for this project's fixtures. Invariant
/// maintained by every operation below: `num` shares no factor with any
/// generator currently listed in `den` (requirement 1) -- `overflow` is
/// exempted from that invariant (reducing against it would mean calling
/// the general GCD on the hot path, exactly what this type exists to
/// avoid), so `overflow` alone can still carry unreduced content; only
/// the final top-level result is reduced against it once, via the
/// existing general engine, in [`normalize_localized`].
#[derive(Debug)]
struct LocalizedRational {
    num: Poly,
    den: Vec<(usize, u32)>,
    overflow: Poly,
}

impl LocalizedRational {
    fn from_plain(p: Poly) -> Self {
        LocalizedRational { num: p, den: Vec::new(), overflow: Poly::constant(BigScalar::one()) }
    }

    fn one() -> Self {
        Self::from_plain(Poly::constant(BigScalar::one()))
    }

    /// Reduces `self.num` against every generator currently listed in
    /// `self.den` (requirement 1): while a generator's `Poly` divides
    /// `num` exactly, replace `num` with the quotient and drop the
    /// generator's exponent by one. No GCD -- only `Poly::exact_div`
    /// against generators already known to be in play for this
    /// combination. Takes `&mut LocalizationContext` (not just the
    /// generator list) because `exact_div` needs `&mut AtomTable` too;
    /// destructuring the two fields directly below gives disjoint
    /// borrows instead of fighting the borrow checker over one `&mut
    /// self`.
    fn reduce_against_own_generators(&mut self, ctx: &mut LocalizationContext) {
        let LocalizationContext { table, generators, .. } = ctx;
        let mut i = 0;
        while i < self.den.len() {
            let (idx, mut exp) = self.den[i];
            while exp > 0 {
                match self.num.exact_div(&generators[idx].poly, table) {
                    Some(q) => {
                        self.num = q;
                        exp -= 1;
                    }
                    None => break,
                }
            }
            if exp == 0 {
                self.den.remove(i);
            } else {
                self.den[i] = (idx, exp);
                i += 1;
            }
        }
    }

    fn mul(&self, other: &Self, ctx: &mut LocalizationContext) -> Self {
        let num = self.num.mul(&other.num, &mut ctx.table);
        let mut den = self.den.clone();
        merge_add(&mut den, &other.den);
        let overflow = self.overflow.mul(&other.overflow, &mut ctx.table);
        let mut result = LocalizedRational { num, den, overflow };
        result.reduce_against_own_generators(ctx);
        result
    }

    /// Nonnegative powers only -- `expr_to_localized`'s `Pow` case routes
    /// every negative exponent through [`reciprocal_pow`] instead, which
    /// is where a genuinely new denominator generator can be discovered
    /// (requirement 2). This function never needs that: `self` is
    /// already reduced (`self.num` coprime to every generator in
    /// `self.den`), and raising both sides to the same power preserves
    /// that (`gcd(a,p)=1 => gcd(a^m,p)=1` for any `m`) -- no re-reduction
    /// step needed here.
    fn pow(&self, m: u32, ctx: &mut LocalizationContext) -> Self {
        let num = self.num.pow(m, &mut ctx.table);
        let den = self.den.iter().map(|&(idx, e)| (idx, e * m)).collect();
        let overflow = self.overflow.pow(m, &mut ctx.table);
        LocalizedRational { num, den, overflow }
    }


    fn add(&self, other: &Self, ctx: &mut LocalizationContext, checkpoint: Checkpoint) -> Result<Self, LocalizationBudgetExceeded> {
        let mut merged_den: Vec<(usize, u32)> = self.den.clone();
        merge_max(&mut merged_den, &other.den);

        let self_missing = missing_factor(&self.den, &merged_den, ctx);
        let other_missing = missing_factor(&other.den, &merged_den, ctx);

        let self_num_scaled = self.num.mul(&self_missing, &mut ctx.table);
        let other_num_scaled = other.num.mul(&other_missing, &mut ctx.table);

        // Overflow denominators combine via the existing general engine
        // (a/b + c/d over unrelated, not-known-to-be-coprime `b`/`d`
        // needs real content management, not a naive cross-multiply --
        // exactly what RationalFunction::from_raw already provides).
        // Every generator-side numerator that isn't already over the
        // *other* side's overflow needs multiplying by it too, and
        // vice versa, before the two overflow-carrying numerators can be
        // added directly.
        //
        // **Performance-critical fast path**: when neither side has any
        // overflow at all (`overflow` still the literal `1` -- the
        // common case for every real fixture, since `overflow` is only
        // ever non-trivial after an actual fallback), skip
        // `RationalFunction::from_raw` entirely instead of calling it
        // with a denominator of `1`. `reduce_inner` always runs a full
        // `poly_gcd` content-strip pass regardless of how trivial `den`
        // is (see its own doc comment: "regardless of which... branch
        // produced them") -- calling it here on every single addition,
        // even when there is nothing whatsoever to reduce, would pay the
        // general engine's full multivariate-GCD cost against a
        // potentially large numerator on *every* `add`, silently
        // reintroducing exactly the cost this type exists to avoid.
        // Found by measurement, not anticipated: the first end-to-end
        // Kerr run under this engine did not return within two minutes
        // until this fast path was added.
        let self_overflow_trivial = self.overflow.is_literal_one();
        let other_overflow_trivial = other.overflow.is_literal_one();
        let (combined_num, combined_overflow) = if self_overflow_trivial && other_overflow_trivial {
            (self_num_scaled.add(&other_num_scaled), Poly::constant(BigScalar::one()))
        } else {
            let self_num_over_other_overflow = self_num_scaled.mul(&other.overflow, &mut ctx.table);
            let other_num_over_self_overflow = other_num_scaled.mul(&self.overflow, &mut ctx.table);
            let combined_overflow_input_num = self_num_over_other_overflow.add(&other_num_over_self_overflow);
            let combined_overflow_input_den = self.overflow.mul(&other.overflow, &mut ctx.table);
            let rf = fallback_to_general_engine(combined_overflow_input_num, combined_overflow_input_den, ctx, checkpoint)?;
            (rf.num, rf.den)
        };

        let mut result = LocalizedRational { num: combined_num, den: merged_den, overflow: combined_overflow };
        result.reduce_against_own_generators(ctx);
        Ok(result)
    }
}

fn merge_add(den: &mut Vec<(usize, u32)>, other: &[(usize, u32)]) {
    for &(idx, exp) in other {
        if let Some(entry) = den.iter_mut().find(|(i, _)| *i == idx) {
            entry.1 += exp;
        } else {
            den.push((idx, exp));
        }
    }
}

fn merge_max(den: &mut Vec<(usize, u32)>, other: &[(usize, u32)]) {
    for &(idx, exp) in other {
        if let Some(entry) = den.iter_mut().find(|(i, _)| *i == idx) {
            entry.1 = entry.1.max(exp);
        } else {
            den.push((idx, exp));
        }
    }
}

/// `product(generator_i ^ (merged_exp_i - own_exp_i))` -- the factor
/// `own`'s side of an addition needs multiplying by so both sides share
/// the same (LCM) denominator. Always a nonnegative exponent by
/// construction (`merged` is the pairwise max), so this is ordinary
/// `Poly::pow`/`mul`, never a division.
fn missing_factor(own: &[(usize, u32)], merged: &[(usize, u32)], ctx: &mut LocalizationContext) -> Poly {
    let mut acc = Poly::constant(BigScalar::one());
    for &(idx, merged_exp) in merged {
        let own_exp = own.iter().find(|(i, _)| *i == idx).map(|&(_, e)| e).unwrap_or(0);
        let missing = merged_exp - own_exp;
        if missing > 0 {
            acc = acc.mul(&ctx.generators[idx].poly.pow(missing, &mut ctx.table), &mut ctx.table);
        }
    }
    acc
}

/// Standard test (char-0 field, Geddes/Czapor/Labahn ch. 8): `p` has a
/// repeated irreducible factor in generator `id` iff `p` and its own
/// formal derivative in `id` share a nontrivial factor. Checked for
/// every generator `p` actually contains -- `p` is square-free (over
/// this whole ring, not just in one variable) iff this holds for all of
/// them.
fn is_square_free(p: &Poly, table: &mut AtomTable) -> bool {
    for id in p.generators_present() {
        let d = p.formal_derivative(id);
        if d.is_zero() {
            continue;
        }
        let g = poly_gcd(p, &d, table);
        if !g.is_literal_one() {
            return false;
        }
    }
    true
}

/// `p` coprime to every generator already known, via the existing
/// general multivariate GCD (`poly_gcd`) -- used only here, once per
/// *newly encountered* candidate factor, never on the per-operation hot
/// path `LocalizedRational::add`/`mul` take.
fn is_coprime_to_all(p: &Poly, ctx: &mut LocalizationContext) -> bool {
    let LocalizationContext { table, generators, .. } = ctx;
    for g in generators.iter() {
        let gcd = poly_gcd(p, &g.poly, table);
        if !gcd.is_literal_one() {
            return false;
        }
    }
    true
}

/// Divides `remainder` by every known generator, as many times as each
/// exactly divides ([`Poly::exact_div`], never a GCD search), repeating
/// until nothing more comes out. Discovered empirically to matter, not
/// merely a nicety: Kerr's own `ginv` naturally produces denominators
/// like `Sigma*Delta` as *one* already-multiplied-out polynomial (not
/// because any `.od` fixture ever writes `1/(Sigma*Delta)` literally --
/// nobody does -- but because combining `g^rr = Delta/Sigma` with other
/// terms during `christoffel`/`riemann_mixed` produces that product as
/// an emergent denominator). Without this decomposition step, `Sigma*Delta`
/// matches neither generator alone and (wrongly) looks like a brand-new,
/// composite candidate -- which correctly fails the coprimality check
/// (it shares `Sigma` with the `Sigma` generator already known) and would
/// fall back to the general engine on nearly every component, which is
/// exactly the regression a first version of this file measured:
/// `christoffel_localized` finished in 393ms, but `riemann_mixed_localized`
/// never returned, because the general-engine fallback path kept
/// re-triggering on this exact composite, over and over.
fn decompose_against_known_generators(mut remainder: Poly, ctx: &mut LocalizationContext) -> (Vec<(usize, u32)>, Poly) {
    let mut decomposition: Vec<(usize, u32)> = Vec::new();
    let mut progress = true;
    while progress {
        progress = false;
        for i in 0..ctx.generators.len() {
            while let Some(q) = remainder.exact_div(&ctx.generators[i].poly, &mut ctx.table) {
                remainder = q;
                progress = true;
                if let Some(entry) = decomposition.iter_mut().find(|(idx, _)| *idx == i) {
                    entry.1 += 1;
                } else {
                    decomposition.push((i, 1));
                }
            }
        }
    }
    (decomposition, remainder)
}

/// The heart of requirement 2 (DESIGN-RATIONAL-FORM.md section 8.4):
/// classifies `p` against `ctx`'s known generators. Always divides out
/// every known generator first ([`decompose_against_known_generators`]),
/// then handles whatever's left:
///
/// - a nonzero constant (including the literal `1`) -- `p` fully
///   decomposed into known generators (plus, possibly, an overall
///   rational scalar); no admission, no fallback.
/// - square-free and coprime to every generator already known -- admits
///   the leftover itself as one new generator.
/// - neither -- the leftover is returned as-is (never silently dropped);
///   the caller folds it into `overflow` and this function has already
///   recorded why in `ctx.fallback_log`.
///
/// Returns `(decomposition, leftover)`: `decomposition` is always valid
/// (every entry is a real, already-known-or-just-admitted generator);
/// `leftover` is `Poly::constant(_)` when nothing needs to fall back,
/// or a genuine residual polynomial when something does.
fn classify_or_admit(p: &Poly, ctx: &mut LocalizationContext) -> (Vec<(usize, u32)>, Poly) {
    let (decomposition, remainder) = decompose_against_known_generators(p.clone(), ctx);
    if remainder.generators_present().is_empty() {
        return (decomposition, remainder);
    }
    if is_square_free(&remainder, &mut ctx.table) && is_coprime_to_all(&remainder, ctx) {
        let idx = ctx.generators.len();
        let source = poly_to_expr(&remainder, &ctx.table);
        ctx.generators.push(Generator { poly: remainder, source });
        let mut decomposition = decomposition;
        decomposition.push((idx, 1));
        return (decomposition, Poly::constant(BigScalar::one()));
    }
    // `remainder` itself isn't square-free -- but *belonging* to the
    // localization is decided by division, not by whether `remainder`
    // happens to already be presented in already-reduced form (the same
    // lesson `decompose_against_known_generators` already applies to
    // known generators). Concretely: `sin(theta)^2` shows up as a
    // denominator before `sin(theta)` itself has ever been admitted (an
    // ordering artifact of which component gets processed first, not a
    // property of the metric) -- `sin(theta)^2` alone correctly fails
    // square-freeness, but the repeated factor inside it is a genuine,
    // smaller generator. Recover it via `gcd(remainder, remainder')`
    // (the standard first step of square-free factorization -- Yun's
    // algorithm; Geddes/Czapor/Labahn ch. 8): if that recovered factor
    // itself qualifies, admit IT and re-decompose `remainder` against
    // the now-larger generator set, instead of giving up on the whole
    // thing.
    if let Some(repeated_factor) = find_repeated_factor(&remainder, ctx) {
        if is_square_free(&repeated_factor, &mut ctx.table) && is_coprime_to_all(&repeated_factor, ctx) {
            // Not read directly -- `decompose_against_known_generators`
            // below re-scans the whole (now one-larger) generator list,
            // including this new entry, and locates it that way.
            let source = poly_to_expr(&repeated_factor, &ctx.table);
            ctx.generators.push(Generator { poly: repeated_factor, source });
            let (extra_decomposition, new_remainder) = decompose_against_known_generators(remainder, ctx);
            let mut decomposition = decomposition;
            decomposition.extend(extra_decomposition);
            if new_remainder.generators_present().is_empty() {
                return (decomposition, new_remainder);
            }
            let rendered = poly_to_expr(&new_remainder, &ctx.table);
            ctx.fallback_log.push(format!(
                "localized: {rendered} left over after extracting a repeated factor -- folding into the general engine for this factor"
            ));
            return (decomposition, new_remainder);
        }
    }
    let rendered = poly_to_expr(&remainder, &ctx.table);
    ctx.fallback_log.push(format!(
        "localized: {rendered} is not square-free and/or not coprime with an existing generator -- folding into the general engine for this factor"
    ));
    (decomposition, remainder)
}

/// `gcd(remainder, d(remainder)/d(id))` for the first generator `id`
/// whose derivative is nonzero and whose gcd with `remainder` is
/// nontrivial -- the repeated irreducible factor a failed square-free
/// check implies must exist (over a field of characteristic 0, `P` has
/// a repeated factor in `id` iff `gcd(P, dP/d(id)) != 1`). Always a
/// *proper* divisor of `remainder` when found: `d(remainder)/d(id)` has
/// strictly lower degree in `id` than `remainder` itself (ordinary
/// polynomial differentiation), so their gcd can never reach
/// `remainder`'s own full degree.
fn find_repeated_factor(remainder: &Poly, ctx: &mut LocalizationContext) -> Option<Poly> {
    for id in remainder.generators_present() {
        let d = remainder.formal_derivative(id);
        if d.is_zero() {
            continue;
        }
        let g = poly_gcd(remainder, &d, &mut ctx.table);
        if !g.is_literal_one() {
            return Some(g);
        }
    }
    None
}

/// `Expr -> LocalizedRational`, mirroring `canonical::expr_to_rational`'s
/// recursive shape exactly, generator-by-`Expr`-variant, but building
/// `LocalizedRational`s (which never call the general `poly_gcd` in
/// `add`/`mul`) instead of `RationalFunction`s (which always do).
fn expr_to_localized(e: &Expr, ctx: &mut LocalizationContext, checkpoint: Checkpoint) -> Result<LocalizedRational, LocalizationBudgetExceeded> {
    match e {
        Expr::Rational(s) => Ok(LocalizedRational::from_plain(Poly::constant(s.clone()))),
        Expr::Var(name) => Ok(LocalizedRational::from_plain(Poly::generator(ctx.table.var(name)))),
        Expr::Add(terms) => {
            let mut acc = LocalizedRational::from_plain(Poly::zero());
            for t in terms {
                let lr = expr_to_localized(t, ctx, checkpoint)?;
                acc = acc.add(&lr, ctx, checkpoint)?;
            }
            Ok(acc)
        }
        Expr::Mul(factors) => {
            let mut acc = LocalizedRational::one();
            for f in factors {
                let lr = expr_to_localized(f, ctx, checkpoint)?;
                acc = acc.mul(&lr, ctx);
            }
            Ok(acc)
        }
        Expr::Pow(base, n) => {
            let base_lr = expr_to_localized(base, ctx, checkpoint)?;
            if *n >= 0 {
                Ok(base_lr.pow(*n as u32, ctx))
            } else {
                reciprocal_pow(&base_lr, (-*n) as u32, ctx, checkpoint)
            }
        }
        // D-RF.6, same as `expr_to_rational`: the argument is
        // canonicalized first (via the ordinary general engine -- an
        // argument like a bare coordinate name never benefits from
        // localization) so two structurally different but equal
        // arguments land on the same atom.
        Expr::Sin(arg) => {
            let canonical_arg = crate::normalize::normalize(arg);
            let id = ctx.table.sin(canonical_arg);
            Ok(LocalizedRational::from_plain(Poly::generator(id)))
        }
        Expr::Cos(arg) => {
            let canonical_arg = crate::normalize::normalize(arg);
            let id = ctx.table.cos(canonical_arg);
            Ok(LocalizedRational::from_plain(Poly::generator(id)))
        }
        Expr::Exp(arg) => {
            let canonical_arg = crate::normalize::normalize(arg);
            let id = ctx.table.exp(canonical_arg);
            Ok(LocalizedRational::from_plain(Poly::generator(id)))
        }
        Expr::Sinh(arg) => {
            let canonical_arg = crate::normalize::normalize(arg);
            let id = ctx.table.sinh(canonical_arg);
            Ok(LocalizedRational::from_plain(Poly::generator(id)))
        }
        Expr::Cosh(arg) => {
            let canonical_arg = crate::normalize::normalize(arg);
            let id = ctx.table.cosh(canonical_arg);
            Ok(LocalizedRational::from_plain(Poly::generator(id)))
        }
        Expr::Func { name, args, order } => {
            let canonical_args: Vec<Expr> = args.iter().map(crate::normalize::normalize).collect();
            let id = ctx.table.func(name.clone(), canonical_args, order.clone());
            Ok(LocalizedRational::from_plain(Poly::generator(id)))
        }
    }
}

/// `base_lr^-m` -- the point a genuinely new denominator generator can
/// enter the picture (requirement 2). Common case (`base_lr` itself
/// carries no denominator of its own -- true for every real fixture:
/// `Sigma`, `Delta`, `sin(theta)` are all written as bare expressions,
/// never as an already-inverted sub-expression): `base_lr.num` is
/// classified via `classify_or_admit`, which divides out every known
/// generator it can (however many -- `Sigma*Delta` decomposes into
/// *two* entries, not one composite blob) before deciding whether
/// anything left over needs admitting or falling back.
fn reciprocal_pow(base_lr: &LocalizedRational, m: u32, ctx: &mut LocalizationContext, checkpoint: Checkpoint) -> Result<LocalizedRational, LocalizationBudgetExceeded> {
    if !base_lr.den.is_empty() {
        // Rare, pathological case: the thing being inverted already had
        // its own known-generator denominator (a nested fraction inside
        // a further negative power). Falls back to the existing general
        // engine wholesale for this one factor -- correctness preserved
        // (D-RF section 2.6), no attempt to keep tracking structured
        // factors through it.
        ctx.fallback_log.push("localized: reciprocal of an expression that itself already had a tracked denominator -- falling back to the general engine for this factor (nested fraction, outside this round's scope)".to_string());
        let full_den = base_lr.den.iter().fold(base_lr.overflow.clone(), |acc, &(idx, exp)| acc.mul(&ctx.generators[idx].poly.pow(exp, &mut ctx.table), &mut ctx.table));
        let rf = fallback_to_general_engine(base_lr.num.clone(), full_den, ctx, checkpoint)?;
        let rf = rf.pow(-(m as i32), &mut ctx.table);
        return Ok(LocalizedRational { num: rf.num, den: Vec::new(), overflow: rf.den });
    }
    let overflow_num_part = base_lr.overflow.pow(m, &mut ctx.table);
    let (decomposition, remainder) = classify_or_admit(&base_lr.num, ctx);
    let den: Vec<(usize, u32)> = decomposition.into_iter().map(|(idx, e)| (idx, e * m)).collect();
    if remainder.generators_present().is_empty() {
        // Fully decomposed into known generators, with at most a
        // constant scalar left over (`1` in the common case) -- that
        // scalar's `m`-th power inverted folds directly into the
        // numerator, no Poly-level denominator tracking needed for a
        // bare number.
        let c = remainder.terms.first().map(|t| t.coeff.clone()).unwrap_or_else(BigScalar::one);
        let mut c_pow = BigScalar::one();
        for _ in 0..m {
            c_pow = c_pow * c.clone();
        }
        let inv = c_pow.recip().expect("a denominator's own leftover scalar is never zero");
        Ok(LocalizedRational { num: overflow_num_part.scale(inv), den, overflow: Poly::constant(BigScalar::one()) })
    } else {
        let remainder_pow = remainder.pow(m, &mut ctx.table);
        Ok(LocalizedRational { num: overflow_num_part, den, overflow: remainder_pow })
    }
}

/// Reassembles `lr` directly into an `Expr` -- deliberately *never*
/// routes back through `expr_to_rational`/the general `normalize()`:
/// doing so would reconvert an already fully-reduced (by construction)
/// numerator/denominator pair through the very GCD machinery this engine
/// exists to avoid calling on the hot path, paying its full cost on a
/// bivariate case like Kerr's `Sigma` for an answer already known.
/// Consequence, accepted deliberately: this engine's canonical *shape*
/// can differ from the general engine's for the same value (same
/// situation `normalize.rs`'s `v1_and_v2_agree` already lives with for
/// legacy vs. current) -- compare by numeric evaluation, not `Expr`
/// equality, when checking this engine's output against the general
/// one's.
/// Multiplies every denominator factor (each generator raised to its
/// exponent, plus `overflow`) into *one* combined `Poly` before ever
/// converting to `Expr` -- `Poly::pow`/`Poly::mul` only (never GCD), so
/// this costs nothing this type exists to avoid, and it happens exactly
/// once per top-level [`normalize_localized`] call, not once per
/// arithmetic operation. The payoff: `poly_to_expr` canonicalizes via
/// `Poly::sorted_terms`, the *same* canonical order the general engine's
/// own `rational_to_expr`/`poly_to_expr` uses for an equal-value `Poly`
/// -- so a denominator like `Sigma^6` comes back fully expanded and
/// term-sorted, matching the general engine's own shape for the same
/// value exactly (checked directly: `kretschmann_of_kerr_matches_the_
/// closed_form_via_the_localized_engine`'s numerator already matched
/// byte-for-byte before this function expanded the denominator too;
/// leaving the denominator as `Pow(Sigma, 6)` instead of its expansion
/// was the one remaining shape mismatch against a `normalize()`-built
/// expected value, found by that test failing on shape while the
/// numeric value already agreed).
fn localized_to_expr(lr: &LocalizedRational, ctx: &mut LocalizationContext) -> Expr {
    let mut num = lr.num.clone();
    let mut combined_den = lr.overflow.clone();
    for &(idx, exp) in &lr.den {
        if exp > 0 {
            combined_den = combined_den.mul(&ctx.generators[idx].poly.pow(exp, &mut ctx.table), &mut ctx.table);
        }
    }
    // Sign convention, matched to the general engine's: a denominator
    // whose leading (canonically first) term is negative gets negated,
    // with the numerator negated to compensate. `num/den` and
    // `-num/-den` are the same value, so this is purely a canonical-form
    // choice -- but it has to be the *same* choice both engines make, or
    // switching engines silently changes rendered output for identical
    // mathematics. Found by the CLI differential test
    // (`oderom-cli/tests/engine_differential.rs`): Schwarzschild's
    // `Gamma^t_tr` came out `-M/(2*M*r - r^2)` from the general engine
    // and `M/(-2*M*r + r^2)` from this one -- equal in value, different
    // byte for byte, which would make any golden CLI test depend on
    // which engine happened to run.
    if combined_den.sorted_terms(&ctx.table).first().is_some_and(|t| t.coeff.is_negative()) {
        combined_den = combined_den.neg();
        num = num.neg();
    }
    let num_expr = poly_to_expr(&num, &ctx.table);
    if combined_den.is_literal_one() {
        return num_expr;
    }
    let den_expr = poly_to_expr(&combined_den, &ctx.table);
    let den_pow = Expr::Pow(Box::new(den_expr), -1);
    if num_expr == Expr::one() {
        den_pow
    } else {
        num_expr * den_pow
    }
}

/// Like [`crate::normalize`], but reduces against `ctx`'s known
/// localization generators after every sum (DESIGN-RATIONAL-FORM.md
/// section 8) instead of relying solely on the general recursive
/// multivariate GCD -- the fix for a denominator with no single pole
/// variable (section 7.1). `ctx` persists and grows across calls (see
/// [`LocalizationContext`]'s own doc comment); pass the *same* context
/// for every component of one metric's Christoffel/Riemann/Ricci/
/// Kretschmann computation so a generator discovered in one stage is
/// recognized in the next.
pub fn normalize_localized(e: &Expr, ctx: &mut LocalizationContext, checkpoint: Checkpoint) -> Result<Expr, LocalizationBudgetExceeded> {
    let lr = expr_to_localized(e, ctx, checkpoint)?;
    Ok(localized_to_expr(&lr, ctx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::normalize::normalize as general_normalize;

    fn sigma_expr() -> Expr {
        Expr::var("r").pow(2) + Expr::var("a").pow(2) * Expr::var("theta").cos().pow(2)
    }

    fn delta_expr() -> Expr {
        Expr::var("r").pow(2) - Expr::int(2) * Expr::var("M") * Expr::var("r") + Expr::var("a").pow(2)
    }

    /// Numeric oracle, same idea as `canonical.rs`'s own private `eval`
    /// test helper -- used to compare this engine's output against the
    /// general engine's by *value*, not `Expr` shape, since the two are
    /// allowed to differ in canonical form (`localized_to_expr`'s own
    /// doc comment).
    fn eval(e: &Expr, vars: &[(&str, f64)]) -> f64 {
        match e {
            Expr::Rational(s) => s.to_f64_lossy(),
            Expr::Var(name) => vars.iter().find(|(n, _)| *n == name).map(|(_, v)| *v).unwrap_or_else(|| panic!("unbound var {name}")),
            Expr::Add(terms) => terms.iter().map(|t| eval(t, vars)).sum(),
            Expr::Mul(factors) => factors.iter().map(|f| eval(f, vars)).product(),
            Expr::Pow(base, n) => eval(base, vars).powi(*n),
            Expr::Sin(arg) => eval(arg, vars).sin(),
            Expr::Cos(arg) => eval(arg, vars).cos(),
            Expr::Exp(arg) => eval(arg, vars).exp(),
            Expr::Sinh(arg) => eval(arg, vars).sinh(),
            Expr::Cosh(arg) => eval(arg, vars).cosh(),
            Expr::Func { name, .. } => panic!("no numeric value for indeterminate function `{name}` in this test"),
        }
    }

    const VARS: &[(&str, f64)] = &[("r", 3.0), ("a", 0.7), ("M", 1.3), ("theta", 0.9)];

    fn assert_matches_general_engine(e: &Expr, ctx: &mut LocalizationContext) {
        let localized = normalize_localized(e, ctx, &mut || false).unwrap();
        let general = general_normalize(e);
        let lv = eval(&localized, VARS);
        let gv = eval(&general, VARS);
        assert!((lv - gv).abs() < 1e-6, "localized={lv} general={gv}\nlocalized_expr={localized:?}\ngeneral_expr={general:?}");
    }

    #[test]
    fn adding_the_same_reciprocal_generator_twice_collects_without_squaring_the_denominator() {
        // 1/Sigma + 1/Sigma = 2/Sigma -- must not become 2*Sigma/Sigma^2
        // (uncancelled): the whole point of reducing after every sum.
        let mut ctx = LocalizationContext::new(&[sigma_expr()]);
        let e = Expr::Pow(Box::new(sigma_expr()), -1) + Expr::Pow(Box::new(sigma_expr()), -1);
        let lr = expr_to_localized(&e, &mut ctx, &mut || false).unwrap();
        assert_eq!(lr.den, vec![(0, 1)], "denominator degree grew instead of staying at Sigma^1: {:?}", lr.den);
        assert!(ctx.fallback_log().is_empty(), "{:?}", ctx.fallback_log());
        assert_matches_general_engine(&e, &mut ctx);
    }

    #[test]
    fn sigma_over_one_times_one_over_sigma_cancels_to_the_literal_one() {
        // The multiplication-side cancellation `reduce_against_own_generators`
        // exists for: Sigma * (1/Sigma) must collapse all the way to the
        // literal polynomial 1, not stay as Sigma/Sigma.
        let mut ctx = LocalizationContext::new(&[sigma_expr()]);
        let e = sigma_expr() * Expr::Pow(Box::new(sigma_expr()), -1);
        let lr = expr_to_localized(&e, &mut ctx, &mut || false).unwrap();
        assert!(lr.den.is_empty(), "{:?}", lr.den);
        assert!(lr.num.is_literal_one(), "expected the literal polynomial 1, got {:?}", lr.num.sorted_terms(&ctx.table));
    }

    #[test]
    fn a_kerr_shaped_cancellation_reduces_to_zero_not_merely_smaller() {
        // The actual failure mode section 7.1 measured: two terms over
        // Sigma^2 and Sigma respectively that are algebraically equal
        // and opposite -- 1/Sigma - (Sigma)/Sigma^2 = 0 exactly. If the
        // post-sum reduction (requirement 1) were skipped, this would
        // normalize to a nonzero-looking (but numerically zero)
        // remainder instead of the literal 0 -- exactly what would leave
        // Kerr's Ricci "smaller" instead of zero.
        let mut ctx = LocalizationContext::new(&[sigma_expr()]);
        let e = Expr::Pow(Box::new(sigma_expr()), -1) - sigma_expr() * Expr::Pow(Box::new(sigma_expr().pow(2)), -1);
        let result = normalize_localized(&e, &mut ctx, &mut || false).unwrap();
        assert_eq!(result, Expr::zero(), "{result:?}");
    }

    #[test]
    fn an_unseeded_denominator_is_auto_admitted_as_a_new_generator() {
        // Context seeded with only Delta; Sigma shows up for the first
        // time inside the expression itself (exactly the "discovered
        // mid-computation" case requirement 2 describes) -- must be
        // admitted (square-free, coprime to Delta), not silently folded
        // into the general-engine overflow path.
        let mut ctx = LocalizationContext::new(&[delta_expr()]);
        assert_eq!(ctx.generator_count(), 1);
        let e = Expr::Pow(Box::new(sigma_expr()), -1);
        let _ = expr_to_localized(&e, &mut ctx, &mut || false).unwrap();
        assert_eq!(ctx.generator_count(), 2, "Sigma should have been auto-admitted as a second generator");
        assert!(ctx.fallback_log().is_empty(), "auto-admission should not itself count as a fallback: {:?}", ctx.fallback_log());
    }

    #[test]
    fn sin_theta_squared_encountered_before_bare_sin_theta_still_decomposes() {
        // The exact ordering artifact this round's real Kerr run hit:
        // some component's denominator is `sin(theta)^2` before any
        // component with a *bare* `sin(theta)` denominator has run --
        // `sin(theta)^2` alone is not square-free, so without repeated-
        // factor recovery this would (correctly, but wastefully) fall
        // back every time. With it, `sin(theta)` itself gets recovered
        // and admitted from the `^2` occurrence directly, no ordering
        // dependency left.
        let mut ctx = LocalizationContext::new(&[]);
        let e = Expr::Pow(Box::new(Expr::var("theta").sin().pow(2)), -1);
        let lr = expr_to_localized(&e, &mut ctx, &mut || false).unwrap();
        assert_eq!(lr.den, vec![(0, 2)], "{:?}", lr.den);
        assert!(ctx.fallback_log().is_empty(), "{:?}", ctx.fallback_log());
    }

    #[test]
    fn a_denominator_that_is_the_product_of_two_known_generators_decomposes_instead_of_falling_back() {
        // The regression this round's own end-to-end Kerr run found:
        // Sigma*Delta shows up as ONE already-multiplied-out denominator
        // (never written that way by hand -- it arises from combining
        // ginv's own Delta/Sigma-shaped entries with other terms), and
        // must decompose into the two ALREADY-KNOWN generators, not be
        // treated as a brand-new composite (which would correctly fail
        // coprimality against Sigma and fall back every time).
        let mut ctx = LocalizationContext::new(&[sigma_expr(), delta_expr()]);
        assert_eq!(ctx.generator_count(), 2);
        let sigma_times_delta = sigma_expr() * delta_expr();
        let e = Expr::Pow(Box::new(sigma_times_delta), -1);
        let lr = expr_to_localized(&e, &mut ctx, &mut || false).unwrap();
        assert_eq!(ctx.generator_count(), 2, "must not have admitted a third, composite generator");
        assert!(ctx.fallback_log().is_empty(), "must not have fallen back: {:?}", ctx.fallback_log());
        let mut den = lr.den.clone();
        den.sort();
        assert_eq!(den, vec![(0, 1), (1, 1)], "expected Sigma^1 * Delta^1, got {den:?}");
        assert!(lr.overflow.is_literal_one());
    }

    #[test]
    fn a_squared_denominator_recovers_its_repeated_factor_instead_of_falling_back() {
        // 1/(x-1)^2, built with the square already baked into the base
        // being inverted -- the SAME shape as `sin(theta)^2` showing up
        // as a Kerr Christoffel denominator before `sin(theta)` itself
        // has been admitted (the regression this test locks in): `(x-1)^2`
        // alone is not square-free, but `gcd((x-1)^2, d/dx[(x-1)^2]) =
        // (x-1)` recovers the genuine, admittable repeated factor, so
        // this decomposes cleanly (`den = [(0, 2)]`) instead of falling
        // back.
        let mut ctx = LocalizationContext::new(&[]);
        let x = Expr::var("x");
        let squared = (x.clone() - Expr::one()).pow(2);
        let e = Expr::Pow(Box::new(squared), -1);
        let lr = expr_to_localized(&e, &mut ctx, &mut || false).unwrap();
        assert_eq!(lr.den, vec![(0, 2)], "expected (x-1) recovered as generator 0 at exponent 2, got {:?}", lr.den);
        assert!(lr.overflow.is_literal_one());
        assert!(ctx.fallback_log().is_empty(), "{:?}", ctx.fallback_log());
        let vars: &[(&str, f64)] = &[("x", 5.0)];
        let localized_val = eval(&localized_to_expr(&lr, &mut ctx), vars);
        assert!((localized_val - 1.0 / 16.0).abs() < 1e-9, "{localized_val}");
    }

    #[test]
    fn a_denominator_not_square_free_even_after_extracting_one_repeated_factor_falls_back() {
        // 1/(x-1)^3 -- one level of repeated-factor recovery
        // (`find_repeated_factor`) only ever extracts
        // `gcd(P, P') = (x-1)^2` here, which is *itself* not square-free
        // (`gcd((x-1)^2, 2(x-1)) = (x-1) != 1`), so it correctly fails
        // its own admission check and this falls all the way back to
        // the general engine -- confirms the recovery doesn't
        // over-claim on a case one extraction genuinely can't resolve
        // (full recursive square-free factorization, Yun's algorithm
        // proper, is out of scope; this is the honest boundary of what
        // one gcd-with-derivative step buys).
        let mut ctx = LocalizationContext::new(&[]);
        let x = Expr::var("x");
        let cubed = (x.clone() - Expr::one()).pow(3);
        let e = Expr::Pow(Box::new(cubed), -1);
        let lr = expr_to_localized(&e, &mut ctx, &mut || false).unwrap();
        assert!(lr.den.is_empty(), "must not have been admitted as a tracked generator: {:?}", lr.den);
        assert!(!lr.overflow.is_literal_one(), "the (x-1)^3 factor must have landed in overflow instead");
        assert!(!ctx.fallback_log().is_empty(), "a fallback must have been logged");
        let vars: &[(&str, f64)] = &[("x", 5.0)];
        let localized_val = eval(&localized_to_expr(&lr, &mut ctx), vars);
        assert!((localized_val - 1.0 / 64.0).abs() < 1e-9, "{localized_val}");
    }

    #[test]
    fn the_execution_budget_actually_fires_at_the_fallback_boundary() {
        // A guardrail that was never seen tripping is not a guardrail
        // (DESIGN-RATIONAL-FORM.md section 8, Phase 1): this
        // deliberately drives execution into the one place this module
        // ever calls the general engine (`add`'s overflow-combination
        // branch, via `fallback_to_general_engine`) with a checkpoint
        // that reports the budget already exhausted, and checks the
        // resulting diagnostic names the actual escaped denominator and
        // the generator set in play -- not just that *some* error came
        // back.
        let mut ctx = LocalizationContext::new(&[sigma_expr()]);
        // (x-1)^3 forces overflow (not square-free even after one
        // repeated-factor extraction -- see the test directly above).
        let x = Expr::var("x");
        let cubed = (x.clone() - Expr::one()).pow(3);
        let lr_with_overflow = expr_to_localized(&Expr::Pow(Box::new(cubed), -1), &mut ctx, &mut || false).unwrap();
        assert!(!lr_with_overflow.overflow.is_literal_one(), "test setup: expected a non-trivial overflow to force add()'s general-engine branch");

        let other = LocalizedRational::from_plain(Poly::constant(BigScalar::one()));
        let err = lr_with_overflow.add(&other, &mut ctx, &mut || true).expect_err("checkpoint reporting the budget exhausted must produce an error, not a value");

        let rendered_denominator = err.denominator.to_string();
        assert!(rendered_denominator.contains('x'), "expected the escaped (x-1)-shaped denominator named in the diagnostic, got {rendered_denominator:?}");
        let rendered_generators: Vec<String> = err.generators.iter().map(|g| g.to_string()).collect();
        assert_eq!(rendered_generators.len(), 1, "expected exactly Sigma (the one seeded generator) listed, got {rendered_generators:?}");

        // Display must actually surface both pieces of information, not
        // just carry them as unused struct fields.
        let message = err.to_string();
        assert!(message.contains(&rendered_denominator), "{message}");
        assert!(message.contains(&rendered_generators[0]), "{message}");
    }

    #[test]
    fn matches_the_general_engine_on_a_sum_of_three_kerr_shaped_terms() {
        // Not a cancellation-to-zero case -- a genuine nonzero rational
        // combination of Sigma/Delta/sin(theta), the same shape
        // Christoffel/Riemann components actually take, checked by
        // numeric value against the general engine rather than assumed
        // to agree from the arithmetic alone.
        let mut ctx = LocalizationContext::new(&[sigma_expr(), delta_expr()]);
        let sin_theta = Expr::var("theta").sin();
        let e = Expr::var("M") * Expr::Pow(Box::new(sigma_expr()), -2)
            + Expr::var("a") * sin_theta * Expr::Pow(Box::new(delta_expr()), -1)
            + Expr::int(3) * Expr::Pow(Box::new(sigma_expr()), -1);
        assert_matches_general_engine(&e, &mut ctx);
        assert!(ctx.fallback_log().is_empty(), "{:?}", ctx.fallback_log());
    }
}

/// Diagnostic (not an acceptance test), same discipline as
/// `oderom-cli/tests/diagnostic_kerr_denominators.rs`: measures this
/// engine against the general one on the exact three-term expression
/// `tests::matches_the_general_engine_on_a_sum_of_three_kerr_shaped_terms`
/// above already checks for agreement. Measured once (release,
/// `cargo test -p oderom-expr --release --lib localized::timing_probe --
/// --ignored --nocapture`): `normalize_localized` 1.97ms, general
/// `normalize()` 2.98s -- roughly 1500x on a expression this small,
/// before ever reaching a full Christoffel/Ricci computation. `#[ignore]`d
/// because it prints instead of asserting a specific number (machine-
/// dependent), not because it's slow.
#[cfg(test)]
mod timing_probe {
    use super::*;
    use crate::normalize::normalize as general_normalize;

    #[test]
    #[ignore]
    fn measure_localized_vs_general_on_the_three_term_sum() {
        let sigma = Expr::var("r").pow(2) + Expr::var("a").pow(2) * Expr::var("theta").cos().pow(2);
        let delta = Expr::var("r").pow(2) - Expr::int(2) * Expr::var("M") * Expr::var("r") + Expr::var("a").pow(2);
        let sin_theta = Expr::var("theta").sin();
        let e = Expr::var("M") * Expr::Pow(Box::new(sigma.clone()), -2)
            + Expr::var("a") * sin_theta * Expr::Pow(Box::new(delta.clone()), -1)
            + Expr::int(3) * Expr::Pow(Box::new(sigma), -1);

        let mut ctx = LocalizationContext::new(&[
            Expr::var("r").pow(2) + Expr::var("a").pow(2) * Expr::var("theta").cos().pow(2),
            Expr::var("r").pow(2) - Expr::int(2) * Expr::var("M") * Expr::var("r") + Expr::var("a").pow(2),
        ]);
        let t0 = std::time::Instant::now();
        let _ = normalize_localized(&e, &mut ctx, &mut || false).unwrap();
        println!("normalize_localized: {:?}", t0.elapsed());

        let t0 = std::time::Instant::now();
        let _ = general_normalize(&e);
        println!("general normalize:   {:?}", t0.elapsed());
    }
}

/// Order-independence of the final generator set (DESIGN-RATIONAL-FORM.md
/// section 8, Phase 1.4): the repeated-factor recovery that fixed
/// `sin(theta)^2`-before-`sin(theta)` established an invariant worth
/// asserting on its own, not just re-testing the one bug that motivated
/// it. Exhaustive (not sampled) over every ordering of a small,
/// genuinely *atomic* candidate set -- see this module's own doc
/// comment below for why "atomic" is doing real work in that sentence.
#[cfg(test)]
mod order_independence {
    use super::*;

    fn sigma() -> Expr {
        Expr::var("r").pow(2) + Expr::var("a").pow(2) * Expr::var("theta").cos().pow(2)
    }
    fn delta() -> Expr {
        Expr::var("r").pow(2) - Expr::int(2) * Expr::var("M") * Expr::var("r") + Expr::var("a").pow(2)
    }

    /// Feeds `order` to a fresh, unseeded context, one candidate at a
    /// time (each as `1/candidate`, the same shape a real denominator
    /// takes), and returns the resulting generator set rendered as a
    /// sorted `Vec<String>` -- a canonical *set* comparison, not a list:
    /// two orderings that admit the same generators in different
    /// sequence must compare equal.
    fn final_generator_set(order: &[Expr]) -> Vec<String> {
        let mut ctx = LocalizationContext::new(&[]);
        for candidate in order {
            let e = Expr::Pow(Box::new(candidate.clone()), -1);
            expr_to_localized(&e, &mut ctx, &mut || false).unwrap();
        }
        let mut set: Vec<String> = ctx.generator_sources().iter().map(|g| g.to_string()).collect();
        set.sort();
        set
    }

    fn assert_all_permutations_agree(candidates: &[Expr]) {
        let reference = final_generator_set(candidates);
        let mut perm = candidates.to_vec();
        permute(&mut perm, 0, &mut |ordering| {
            assert_eq!(final_generator_set(ordering), reference, "ordering {ordering:?} produced a different generator set than {candidates:?}");
        });
    }

    /// Heap's algorithm -- exhaustive, not sampled, deterministic (no
    /// randomness): small enough here (<= 24 orderings) that there is
    /// no reason to settle for a sample.
    fn permute(arr: &mut Vec<Expr>, k: usize, visit: &mut dyn FnMut(&[Expr])) {
        if k == arr.len() {
            visit(arr);
            return;
        }
        for i in k..arr.len() {
            arr.swap(k, i);
            permute(arr, k + 1, visit);
            arr.swap(k, i);
        }
    }

    #[test]
    fn kerr_shaped_atomic_denominators_reach_the_same_generator_set_regardless_of_order() {
        // Sigma, Delta, sin(theta), sin(theta)^2 -- every one of these
        // is atomic in the sense that matters here: none is a *product*
        // of two of the others (contrast the composite case this same
        // module's doc comment documents as a real, separate limit).
        // 4! = 24 orderings, exhausted.
        assert_all_permutations_agree(&[sigma(), delta(), Expr::var("theta").sin(), Expr::var("theta").sin().pow(2)]);
    }

    #[test]
    fn the_exact_ordering_that_caused_the_original_bug_is_named_explicitly() {
        // sin(theta)^2 before bare sin(theta) -- the literal ordering
        // `oderom-cli`'s real Kerr run hit before repeated-factor
        // recovery existed. Named on its own, not just folded into the
        // exhaustive permutation test above, so this specific regression
        // stays visible by name if it ever breaks again.
        let bug_order = [Expr::var("theta").sin().pow(2), Expr::var("theta").sin(), sigma(), delta()];
        let natural_order = [sigma(), delta(), Expr::var("theta").sin(), Expr::var("theta").sin().pow(2)];
        assert_eq!(final_generator_set(&bug_order), final_generator_set(&natural_order));
    }

    /// A composite candidate (`Sigma*Delta`) mixed into the same set as
    /// its own factors, exhausted over all `3! = 6` orderings. Caught by
    /// this test's own construction, worth recording precisely rather
    /// than silently: a first version of this check compared two
    /// *different* sets (`{Sigma*Delta, Sigma}` against `{Sigma, Delta}`)
    /// and misread the mismatch as a genuine order-dependence bug --
    /// once every element of the *same* fixed set gets a chance to be
    /// presented (which is exactly what a real permutation guarantees,
    /// and what that first, invalid comparison never actually did),
    /// `Sigma*Delta` presented first still recovers `Sigma` via
    /// `find_repeated_factor`, and the plain `Delta` that follows later
    /// in the same ordering is admitted cleanly against it -- the final
    /// set converges to `{Sigma, Delta}` regardless of position.
    #[test]
    fn composite_mixed_with_its_own_factors_still_reaches_the_same_generator_set() {
        assert_all_permutations_agree(&[sigma() * delta(), sigma(), delta()]);
    }
}

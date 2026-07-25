//! A canonical multivariate polynomial over "generators" (DESIGN-RATIONAL-FORM.md
//! section 2/5): named variables and, once interned, `sin`/`cos` of an
//! already-canonical argument -- both behave identically in the
//! arithmetic below, one flat monomial ring, no separate "coefficient"
//! level that could itself go uncollected (D-RF.4-D-RF.7).
//!
//! Not exported outside this crate: `Poly`/`AtomTable` only ever live
//! inside one [`crate::normalize`] call, converted from and back to
//! [`crate::Expr`] at the boundary (D-RF.6) -- `diff` never sees them.

use crate::Expr;
use crate::BigScalar;
use rustc_hash::FxHashMap;

thread_local! {
    // See `reduce_trig_powers`'s doc comment for why this exists: never
    // toggled directly, only through `suppress_trig_rewrite_during`
    // (rational_function.rs's `poly_gcd`), whose RAII guard resets it
    // even if the guarded computation panics.
    static SUPPRESS_TRIG_REWRITE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn trig_rewrite_suppressed() -> bool {
    SUPPRESS_TRIG_REWRITE.with(|c| c.get())
}

/// RAII guard: sets the thread-local trig-rewrite suppression flag for
/// its lifetime, restores the previous value on drop (including on
/// unwind, so a panic inside the guarded computation -- this project's
/// standard response to a violated exactness assumption -- can never
/// leave the flag stuck on for whatever runs next on this thread).
/// `pub(crate)` for `rational_function.rs`'s `poly_gcd`; never
/// constructed anywhere else.
///
/// **Correct today only because each `normalize()` call runs start to
/// finish on one thread** (D-RF.5: one `AtomTable` per top-level call,
/// never shared). If component-level curvature computation is ever
/// parallelized (e.g. `christoffel`'s or `riemann_mixed`'s per-component
/// loop farmed out across threads, or a future async/work-stealing
/// executor moving one `normalize()` call's continuation mid-flight
/// between OS threads), a thread-local flag stops being a reliable guard
/// on its own: a child thread spawned *during* a suppressed `poly_gcd`
/// call starts with the flag unset (thread-locals do not inherit across
/// spawn boundaries), or, on an executor that migrates one logical
/// computation across worker threads, a suppressed region could resume
/// on a thread that never had the flag set at all. Either way the
/// failure mode is silent and specific: the D-RF.7 rewrite fires mid-GCD
/// again, exactly the bug this type exists to prevent (DESIGN-RATIONAL-
/// FORM.md section 6) -- no panic, no error, just a wrong answer,
/// because `mul_term` has no way to know it's running inside someone
/// else's suppressed region. Before parallelizing anything that calls
/// through `normalize()`/`poly_gcd`, thread this state explicitly instead
/// (a parameter passed down the call chain, or a value carried on
/// whatever task/context object the parallel executor already threads
/// through) rather than trusting a thread-local to keep tracking it.
pub(crate) struct TrigRewriteSuppressor {
    previous: bool,
}

impl TrigRewriteSuppressor {
    pub(crate) fn new() -> Self {
        let previous = SUPPRESS_TRIG_REWRITE.with(|c| c.replace(true));
        TrigRewriteSuppressor { previous }
    }
}

impl Drop for TrigRewriteSuppressor {
    fn drop(&mut self) {
        SUPPRESS_TRIG_REWRITE.with(|c| c.set(self.previous));
    }
}

/// An interned generator handle: `Copy`, compared/hashed in O(1). Never
/// used for ordering -- see [`AtomTable`]'s docs (D-RF.4).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct AtomId(u32);

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub(crate) enum AtomKey {
    Var(String),
    Sin(Expr),
    Cos(Expr),
    /// `exp(arg)`, `arg` already canonical (D-RF.6, same as `Sin`/`Cos`).
    /// Deliberately never merged with another `Exp` atom via
    /// `exp(a)*exp(b) = exp(a+b)` -- see `AtomTable::exp`'s own doc
    /// comment for why.
    Exp(Expr),
    Sinh(Expr),
    Cosh(Expr),
    /// An indeterminate function call (`f(r)`, `f'(r)`, `h_{t,r}(t,r)`)
    /// -- `args` already canonical (D-RF.6, same as `Sin`/`Cos`/`Exp`).
    /// Like `Exp`, never merged with a differently-argued or
    /// differently-ordered `Func` atom sharing the same name: an
    /// indeterminate function has no algebraic identity relating it to
    /// itself at a different argument (DESIGN-M6-PREP.md section 1) --
    /// this atom needs no cross-rewrite at all, only ordinary
    /// same-generator exponent bookkeeping (`mul_term`) for repeated
    /// powers of the very same call.
    Func(String, Vec<Expr>, Vec<u32>),
}

/// Interns generators for exactly one top-level [`crate::normalize`]
/// call (D-RF.5): never a `static`/global, so nothing here is ever
/// shared between threads or between separate computations -- each
/// `normalize()` call builds one, uses it, drops it.
///
/// [`AtomId`] order is assignment order (whichever generator was seen
/// first), which is meaningless -- two calls with the same expression
/// built in a different order intern in a different order. Any output
/// that must be canonical (equality, the final `Poly` -> `Expr`
/// conversion) sorts by [`AtomKey`] -- name for `Var`, `(discriminant,
/// argument)` for `Sin`/`Cos`, using `Expr`'s own `Ord` on the
/// already-canonical argument -- never by `AtomId` (D-RF.4).
pub(crate) struct AtomTable {
    keys: Vec<AtomKey>,
    index: FxHashMap<AtomKey, AtomId>,
}

impl AtomTable {
    pub(crate) fn new() -> Self {
        AtomTable { keys: Vec::new(), index: FxHashMap::default() }
    }

    fn intern(&mut self, key: AtomKey) -> AtomId {
        if let Some(&id) = self.index.get(&key) {
            return id;
        }
        let id = AtomId(self.keys.len() as u32);
        self.keys.push(key.clone());
        self.index.insert(key, id);
        id
    }

    pub(crate) fn var(&mut self, name: &str) -> AtomId {
        self.intern(AtomKey::Var(name.to_string()))
    }

    pub(crate) fn sin(&mut self, arg: Expr) -> AtomId {
        self.intern(AtomKey::Sin(arg))
    }

    pub(crate) fn cos(&mut self, arg: Expr) -> AtomId {
        self.intern(AtomKey::Cos(arg))
    }

    /// `exp(arg)`. Unlike `sin`/`cos`, `exp` carries no reduction rule:
    /// the candidate identity `exp(a)*exp(b) = exp(a+b)` is a *different
    /// kind* of rewrite than D-RF.7's `cos^2 -> 1-sin^2` -- that one
    /// reduces the POWER of a single already-interned atom (the pattern
    /// `mul_term` already runs on every multiplication, generalized
    /// below to `cosh` too); this one would MERGE two atoms with
    /// genuinely different keys (`Exp(a)` and `Exp(b)`, `a != b`) into a
    /// third, freshly-interned one -- new machinery, not a small
    /// extension of the existing pattern. Deliberately left out this
    /// round: D-RF.6 already collapses `exp(a)` and `exp(b)` to the same
    /// atom whenever `a` and `b` canonicalize equal, and every real use
    /// this project has for `exp` (an FRW-style scale factor
    /// `exp(H*t)`, `oderom-cli/tests/fixtures/frw_desitter.od`) only ever
    /// needs INTEGER POWERS of one such atom -- squaring, inverting --
    /// which `mul_term`'s ordinary same-atom exponent bookkeeping already
    /// handles with no rewrite needed at all. Confirmed empirically, not
    /// just argued: a real Christoffel/Ricci/Kretschmann run against that
    /// fixture never produces a product of two differently-argued `Exp`
    /// atoms left unmerged.
    pub(crate) fn exp(&mut self, arg: Expr) -> AtomId {
        self.intern(AtomKey::Exp(arg))
    }

    pub(crate) fn sinh(&mut self, arg: Expr) -> AtomId {
        self.intern(AtomKey::Sinh(arg))
    }

    pub(crate) fn cosh(&mut self, arg: Expr) -> AtomId {
        self.intern(AtomKey::Cosh(arg))
    }

    /// `name(args...)` at derivative `order` -- `f(r)` (`order=[0]`),
    /// `f'(r)` (`order=[1]`), `h`'s partial wrt its second argument
    /// (`order=[0,1]`), etc. No reduction rule at all, same reasoning as
    /// `exp` above, one step further: an indeterminate function has no
    /// candidate cross-atom identity to even consider skipping.
    pub(crate) fn func(&mut self, name: String, args: Vec<Expr>, order: Vec<u32>) -> AtomId {
        self.intern(AtomKey::Func(name, args, order))
    }

    pub(crate) fn key(&self, id: AtomId) -> &AtomKey {
        &self.keys[id.0 as usize]
    }

    pub(crate) fn is_cos(&self, id: AtomId) -> bool {
        matches!(self.key(id), AtomKey::Cos(_))
    }

    pub(crate) fn is_cosh(&self, id: AtomId) -> bool {
        matches!(self.key(id), AtomKey::Cosh(_))
    }

    /// The `Sin` atom sharing `cos_id`'s argument, interning it if this
    /// is the first time it's needed -- used by the `cos^2 -> 1-sin^2`
    /// reduction (D-RF.7).
    fn sin_for_cos(&mut self, cos_id: AtomId) -> AtomId {
        let AtomKey::Cos(arg) = self.key(cos_id).clone() else {
            unreachable!("sin_for_cos is only ever called with a Cos atom")
        };
        self.sin(arg)
    }

    /// The `Sinh` atom sharing `cosh_id`'s argument -- same role as
    /// `sin_for_cos`, for the `cosh^2 -> 1+sinh^2` reduction.
    fn sinh_for_cosh(&mut self, cosh_id: AtomId) -> AtomId {
        let AtomKey::Cosh(arg) = self.key(cosh_id).clone() else {
            unreachable!("sinh_for_cosh is only ever called with a Cosh atom")
        };
        self.sinh(arg)
    }

    pub(crate) fn to_expr(&self, id: AtomId) -> Expr {
        match self.key(id) {
            AtomKey::Var(name) => Expr::var(name.clone()),
            AtomKey::Sin(arg) => arg.clone().sin(),
            AtomKey::Cos(arg) => arg.clone().cos(),
            AtomKey::Exp(arg) => arg.clone().exp(),
            AtomKey::Sinh(arg) => arg.clone().sinh(),
            AtomKey::Cosh(arg) => arg.clone().cosh(),
            AtomKey::Func(name, args, order) => Expr::Func { name: name.clone(), args: args.clone(), order: order.clone() },
        }
    }
}

/// One monomial: a rational coefficient times generators raised to
/// positive integer exponents. `generators` is sorted by `AtomId` --
/// only to give repeated multiplication of the same monomial a stable
/// hash key (`Poly::mul`'s bookkeeping), never as a claim about
/// canonical order (D-RF.4): nothing reads `generators`' order as
/// meaningful except that internal dedup step.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub(crate) struct Term {
    pub(crate) coeff: BigScalar,
    pub(crate) generators: Vec<(AtomId, u32)>,
}

/// A canonical multivariate polynomial: terms with distinct generator
/// signatures, no zero coefficients. `terms`' own order is not
/// meaningful (arithmetic never needs it) -- call [`Poly::sorted_terms`]
/// for a canonical, content-ordered view (equality checks, final output).
#[derive(Clone, Debug, Default)]
pub(crate) struct Poly {
    pub(crate) terms: Vec<Term>,
}

impl Poly {
    pub(crate) fn zero() -> Self {
        Poly { terms: Vec::new() }
    }

    pub(crate) fn constant(c: BigScalar) -> Self {
        if c.is_zero() {
            Poly::zero()
        } else {
            Poly { terms: vec![Term { coeff: c, generators: Vec::new() }] }
        }
    }

    pub(crate) fn generator(id: AtomId) -> Self {
        Poly { terms: vec![Term { coeff: BigScalar::one(), generators: vec![(id, 1)] }] }
    }

    pub(crate) fn is_zero(&self) -> bool {
        self.terms.is_empty()
    }

    pub(crate) fn neg(&self) -> Poly {
        let result = Poly {
            terms: self.terms.iter().map(|t| Term { coeff: -t.coeff.clone(), generators: t.generators.clone() }).collect(),
        };
        debug_assert_invariant(&result);
        result
    }

    pub(crate) fn add(&self, other: &Poly) -> Poly {
        let mut grouped: FxHashMap<Vec<(AtomId, u32)>, BigScalar> = FxHashMap::default();
        for t in self.terms.iter().chain(other.terms.iter()) {
            let mut gens = t.generators.clone();
            gens.sort_by_key(|(id, _)| id.0);
            let entry = grouped.entry(gens).or_insert(BigScalar::zero());
            *entry = entry.clone() + t.coeff.clone();
        }
        let result = Poly {
            terms: grouped
                .into_iter()
                .filter(|(_, c)| !c.is_zero())
                .map(|(generators, coeff)| Term { coeff, generators })
                .collect(),
        };
        debug_assert_invariant(&result);
        result
    }

    pub(crate) fn sub(&self, other: &Poly) -> Poly {
        self.add(&other.neg())
    }

    pub(crate) fn scale(&self, c: BigScalar) -> Poly {
        if c.is_zero() {
            return Poly::zero();
        }
        let result = Poly {
            terms: self
                .terms
                .iter()
                .map(|t| Term { coeff: t.coeff.clone() * c.clone(), generators: t.generators.clone() })
                .collect(),
        };
        debug_assert_invariant(&result);
        result
    }

    pub(crate) fn mul(&self, other: &Poly, table: &mut AtomTable) -> Poly {
        let mut acc = Poly::zero();
        for a in &self.terms {
            for b in &other.terms {
                acc = acc.add(&mul_term(a, b, table));
            }
        }
        debug_assert_invariant(&acc);
        acc
    }

    pub(crate) fn pow(&self, n: u32, table: &mut AtomTable) -> Poly {
        let mut acc = Poly::constant(BigScalar::one());
        for _ in 0..n {
            acc = acc.mul(self, table);
        }
        acc
    }

    /// `self`'s terms sorted by content (see [`AtomTable`]) -- the
    /// canonical order for equality checks and for converting back to
    /// [`Expr`].
    pub(crate) fn sorted_terms(&self, table: &AtomTable) -> Vec<Term> {
        let mut terms = self.terms.clone();
        terms.sort_by_key(|a| content_key(table, a));
        terms
    }

    /// Whether `self` is *exactly* the constant polynomial `1` -- a
    /// narrow syntactic check (this ring has no notion of "unit" beyond
    /// the literal value, since it has no division/inverse at all).
    pub(crate) fn is_literal_one(&self) -> bool {
        self.terms.len() == 1 && self.terms[0].generators.is_empty() && self.terms[0].coeff == BigScalar::one()
    }

    /// Restores D-RF.7's `cos^2 -> 1-sin^2`/`cosh^2 -> 1+sinh^2` invariant
    /// (cos-degree and cosh-degree both <= 1 everywhere) on a `Poly` that
    /// may have been built entirely or partly under
    /// [`TrigRewriteSuppressor`] -- `poly_gcd`'s own exit point, applied
    /// exactly once to its final result, never mid-GCD. A
    /// ring-homomorphism image, term by term: each stored term is already
    /// in normal form except possibly for cos/cosh-degree, so running it
    /// back through the same reduction `mul_term` uses (factored out as
    /// `reduce_trig_powers`) and summing is enough -- no different from
    /// reducing after a normal multiplication, just applied to terms that
    /// accumulated their exponents across several suppressed multiplies
    /// instead of one.
    pub(crate) fn normalize_trig(&self, table: &mut AtomTable) -> Poly {
        let mut acc = Poly::zero();
        for term in &self.terms {
            let combined: FxHashMap<AtomId, u32> = term.generators.iter().cloned().collect();
            acc = acc.add(&reduce_trig_powers(term.coeff.clone(), combined, table));
        }
        debug_assert_invariant(&acc);
        acc
    }

    /// Multivariate exact division: `self / divisor`, assuming (as every
    /// caller -- subresultant PRS in `rational_function.rs` -- relies on
    /// theory to guarantee) the division really is exact. `None` if it
    /// turns out not to be: a violated assumption or an implementation
    /// bug, not a case to paper over (`rational_function.rs`'s callers
    /// treat `None` here as a hard stop, per the subresultant-PRS design
    /// decision in DESIGN-RATIONAL-FORM.md).
    ///
    /// This is standard multivariate polynomial long division against a
    /// FIXED monomial order, *not* `content_key`/`sorted_terms` (which
    /// only lists a term's *present* generators and is therefore not
    /// multiplicatively compatible in general -- concretely, comparing
    /// `x` against `x*y` by that order does not commute correctly with
    /// multiplying both by `x` again, which breaks the termination
    /// argument below). Instead, every generator appearing anywhere in
    /// `self` or `divisor` is padded to an explicit exponent (`0` if
    /// absent) and compared as a fixed-length vector in the atoms' single
    /// total order (`atom_rank`, D-RF.4) -- ordinary pure lexicographic
    /// order on exponent vectors, which *is* multiplicatively compatible
    /// (standard fact; the same property Gröbner-basis division relies
    /// on). Repeatedly cancels the current remainder's leading term
    /// (under that order) against `divisor`'s leading term: since
    /// multiplying every term of `divisor` by the same monomial preserves
    /// their relative order, the subtraction can only ever introduce
    /// terms strictly below the one just cancelled, so the leading term
    /// strictly decreases every step -- terminates because there is no
    /// infinite strictly-decreasing sequence over a polynomial's own
    /// (finite) set of exponent vectors. Never searches for a common
    /// factor (no GCD) -- only ever verifies/computes a division already
    /// known, by theory, to have zero remainder.
    pub(crate) fn exact_div(&self, divisor: &Poly, table: &mut AtomTable) -> Option<Poly> {
        if divisor.is_zero() {
            return None;
        }
        if self.is_zero() {
            return Some(Poly::zero());
        }
        let mut all_vars: Vec<AtomId> = Vec::new();
        for t in self.terms.iter().chain(divisor.terms.iter()) {
            for &(id, _) in &t.generators {
                if !all_vars.contains(&id) {
                    all_vars.push(id);
                }
            }
        }
        all_vars.sort_by_key(|&id| atom_rank(table.key(id)));
        let monomial_key = |term: &Term| -> Vec<u32> {
            all_vars.iter().map(|&id| term.generators.iter().find(|&&(g, _)| g == id).map(|&(_, e)| e).unwrap_or(0)).collect()
        };

        let lead_divisor = divisor.terms.iter().max_by_key(|t| monomial_key(t)).expect("divisor is nonzero, checked above").clone();

        let mut remainder = self.clone();
        let mut quotient = Poly::zero();
        loop {
            if remainder.is_zero() {
                return Some(quotient);
            }
            let lead_rem = remainder.terms.iter().max_by_key(|t| monomial_key(t)).expect("remainder is nonzero, checked above").clone();

            for &(id, exp) in &lead_divisor.generators {
                let rem_exp = lead_rem.generators.iter().find(|&&(g, _)| g == id).map(|&(_, e)| e).unwrap_or(0);
                if rem_exp < exp {
                    return None;
                }
            }
            let mut new_generators: Vec<(AtomId, u32)> = Vec::new();
            for &(id, exp) in &lead_rem.generators {
                let div_exp = lead_divisor.generators.iter().find(|&&(g, _)| g == id).map(|&(_, e)| e).unwrap_or(0);
                let new_exp = exp - div_exp;
                if new_exp != 0 {
                    new_generators.push((id, new_exp));
                }
            }
            let inv =
                lead_divisor.coeff.recip().expect("a stored term's coefficient is never zero, by Poly's own invariant");
            let term_coeff = lead_rem.coeff.clone() * inv;
            let term_poly = Poly { terms: vec![Term { coeff: term_coeff, generators: new_generators }] };
            quotient = quotient.add(&term_poly);
            let subtrahend = term_poly.mul(divisor, table);
            remainder = remainder.sub(&subtrahend);
        }
    }
}

fn content_key(table: &AtomTable, term: &Term) -> Vec<(AtomKey, u32)> {
    let mut v: Vec<(AtomKey, u32)> = term.generators.iter().map(|&(id, e)| (table.key(id).clone(), e)).collect();
    v.sort();
    v
}

/// A legitimate exponent for this project's class of problems never gets
/// remotely close to this (naive-PRS growth is polynomial in input
/// degree x iterations x depth -- a handful of variables at single-digit
/// degrees reaches hundreds or thousands, not more) -- anything past it
/// is a bug, not a large-but-real result. Panics with the offending
/// generator/value rather than let it propagate silently (a sparse
/// generators list makes a garbage exponent cheap to store and carry for
/// a long time before something notices -- DESIGN-RATIONAL-FORM.md
/// investigation).
const EXPONENT_SANITY_LIMIT: u32 = 1_000_000;

fn check_exponent_sane(table: &AtomTable, id: AtomId, exp: u32) {
    assert!(
        exp <= EXPONENT_SANITY_LIMIT,
        "exponent {exp} for generator {:?} exceeds the sanity limit ({EXPONENT_SANITY_LIMIT}) -- this is a bug (a garbage value from an earlier underflow), not a legitimate degree",
        table.key(id)
    );
}

/// Checked in every build that constructs a new `Poly` via `add`/`mul`/
/// `neg`/`scale` (not just in debug builds -- `debug_assert!` inside this
/// function still only actually panics in a `debug_assertions` build, but
/// the exponent-sanity check runs unconditionally): no term may have a
/// zero coefficient (must have been filtered -- a stored zero coefficient
/// is exactly what would make `Poly::is_zero()`, and therefore
/// `UPoly::trim()`/`degree()`, lie) and no exponent may be nonsensically
/// large.
fn debug_assert_invariant(p: &Poly) {
    if !cfg!(debug_assertions) {
        return;
    }
    for term in &p.terms {
        debug_assert!(!term.coeff.is_zero(), "Poly invariant violated: a stored term has a zero coefficient: {term:?}");
        for &(_, exp) in &term.generators {
            debug_assert!(
                exp <= EXPONENT_SANITY_LIMIT,
                "Poly invariant violated: exponent {exp} exceeds the sanity limit ({EXPONENT_SANITY_LIMIT}): {term:?}"
            );
        }
    }
}

/// Single source of truth for the total order `rational_function.rs`
/// uses to pick a pole variable (D-RF.4: content-derived, never
/// `AtomId`/insertion order). Coordinate-like names (this project's
/// `Chart` coordinates) outrank everything else, since those are the only
/// atoms that legitimately need to be a pole variable for this class of
/// metrics (DESIGN-RATIONAL-FORM.md) -- everything else (`M`, `Q`, `L`,
/// ...) is coefficient-ring by convention, ranked below and sorted
/// alphabetically among itself purely for determinism, not because the
/// order matters there. `Sin`/`Cos` rank lowest of all: a trig generator
/// should essentially never need to be a pole variable for this project's
/// metrics. A *lower* returned rank means *higher* priority (chosen
/// first) -- ascending sort order matches priority order.
///
/// With the coefficient ring (`Poly`, this module) holding no
/// denominators at all (`rational_function.rs`'s `UPoly` coefficients are
/// plain `Poly`, never `RationalFunction`), this order is no longer
/// load-bearing for termination the way an earlier, buggier design needed
/// it to be -- termination now follows from the type separation itself.
/// It stays as a deliberate, redundant invariant (checked by
/// `debug_assert` at the one call site that uses it): the pole choice
/// must always be deterministic and derived from this single, fixed rule.
const KNOWN_COORDINATE_NAMES: &[&str] = &["t", "r", "theta", "phi"];

pub(crate) fn atom_rank(key: &AtomKey) -> (u8, usize, &str) {
    match key {
        AtomKey::Var(name) => match KNOWN_COORDINATE_NAMES.iter().position(|c| c == name) {
            Some(i) => (0, i, ""),
            None => (1, 0, name.as_str()),
        },
        // Same reasoning as Sin/Cos: none of these should ever need to be
        // a pole variable for this project's metrics.
        AtomKey::Sin(_) | AtomKey::Cos(_) | AtomKey::Exp(_) | AtomKey::Sinh(_) | AtomKey::Cosh(_) => (2, 0, ""),
        // An indeterminate function is even less of a pole-variable
        // candidate than a known transcendental (this project's metrics
        // never invert with respect to one) -- same rank bucket as
        // Sin/Cos/Exp/Sinh/Cosh; ties within the bucket don't matter,
        // `sorted_terms`'s `content_key` breaks them via the full
        // `AtomKey` (name/args/order), not this rank alone.
        AtomKey::Func(..) => (2, 0, ""),
    }
}

/// Multiplies two single terms, expanding to more than one term exactly
/// when a `cos` or `cosh` generator's combined exponent would reach 2 or
/// more (D-RF.7, via `reduce_trig_powers`): `cos(arg)^(2k) ->
/// (1-sin(arg)^2)^k`, `cos(arg)^(2k+1) -> cos(arg)*(1-sin(arg)^2)^k`, and
/// the hyperbolic analogue `cosh(arg)^(2k) -> (1+sinh(arg)^2)^k`,
/// `cosh(arg)^(2k+1) -> cosh(arg)*(1+sinh(arg)^2)^k` (sign flipped:
/// `cosh^2 - sinh^2 = 1`, not `+`) -- *unless* [`TrigRewriteSuppressor`]
/// is currently active (`poly_gcd`'s subresultant PRS, DESIGN-RATIONAL-
/// FORM.md section 6: rewriting mid-computation there breaks the
/// algorithm's own exactness guarantee), in which case the combined
/// exponent is kept as-is and `Poly::normalize_trig` restores the
/// invariant once, on the way out. When the rewrite does run, it
/// terminates because expanding `(1-sin^2)^k`/`(1+sinh^2)^k` only ever
/// multiplies terms with zero `cos`/`cosh` exponent (neither generator
/// appears in its own substitute at all), so the recursive `mul`/
/// `mul_term` calls that expansion makes can never re-trigger this same
/// reduction. `exp` gets no analogous treatment here -- see
/// `AtomTable::exp`'s own doc comment for why.
fn mul_term(a: &Term, b: &Term, table: &mut AtomTable) -> Poly {
    let coeff = a.coeff.clone() * b.coeff.clone();
    if coeff.is_zero() {
        return Poly::zero();
    }
    let mut combined: FxHashMap<AtomId, u32> = FxHashMap::default();
    for &(id, e) in a.generators.iter().chain(b.generators.iter()) {
        // Checked, and panics in every build (not just when
        // `overflow-checks` happens to be on): a genuine exponent this
        // large is not a value naive-PRS growth could reach for this
        // project's class of problems (DESIGN-RATIONAL-FORM.md
        // investigation) -- it's a garbage value from an earlier
        // underflow (e.g. an unchecked degree/exponent subtraction
        // wrapping around) that propagated silently until two terms
        // sharing this generator finally collided here. Reported eagerly
        // via EXPONENT_SANITY_LIMIT below too, closer to the true origin.
        let entry = combined.entry(id).or_insert(0);
        *entry = entry.checked_add(e).unwrap_or_else(|| {
            panic!(
                "mul_term: exponent overflow combining generator {:?}: {entry} + {e} -- not a legitimate degree, a garbage value from upstream (DESIGN-RATIONAL-FORM.md investigation)",
                table.key(id)
            )
        });
        check_exponent_sane(table, id, *entry);
    }

    if trig_rewrite_suppressed() {
        // Inside subresultant PRS (DESIGN-RATIONAL-FORM.md section 6):
        // rewriting cos^2 -> 1-sin^2 (or cosh^2 -> 1+sinh^2)
        // *mid-computation* changes the coefficient ring out from under
        // the algorithm's own degree bookkeeping, breaking the
        // exact-division guarantee the theory relies on (found for cos,
        // via property-based fuzzing on `cos(0) + (r+x)^-3 + (r-1)`,
        // confirmed independently: beta divides the pseudo-remainder
        // exactly in the *unreduced* ring Q[cos(t)][x], but leaves a
        // nonzero, non-ideal-member remainder once the rewrite has fired
        // mid-stream -- the same algebraic shape applies to cosh/sinh,
        // so it gets the same suppression rather than assuming it's
        // immune). Leave cos/cosh at whatever exponent they reach here;
        // `Poly::normalize_trig` restores the canonical degree-<=1 form
        // for both, once, on the way back out of `poly_gcd`.
        let generators: Vec<(AtomId, u32)> = combined.into_iter().filter(|&(_, e)| e != 0).collect();
        return Poly { terms: vec![Term { coeff, generators }] };
    }
    reduce_trig_powers(coeff, combined, table)
}

/// D-RF.7's `cos^(2k) -> (1-sin(arg)^2)^k`, `cos^(2k+1) -> cos(arg)*(1-sin(arg)^2)^k`
/// rewrite, and its hyperbolic analogue for `cosh`/`sinh` (sign flipped:
/// `cosh(arg)^(2k) -> (1+sinh(arg)^2)^k`, `cosh(arg)^(2k+1) ->
/// cosh(arg)*(1+sinh(arg)^2)^k`) -- applied to one already-combined
/// coefficient+generators pair (shared by `mul_term`, the normal
/// per-multiplication path, and [`Poly::normalize_trig`], the explicit
/// post-pass used after a computation that ran with the rewrite
/// suppressed). Reduces one atom at a time (whichever of `cos`/`cosh`
/// is found first, with exponent >= 2) and re-multiplies through
/// `Poly::mul` -- which calls back into `mul_term`/this function -- so a
/// term carrying both a reducible `cos` and a reducible `cosh` power
/// (e.g. `cos(x)^3 * cosh(y)^2`) still converges to fully reduced degree
/// <= 1 on both, one reduction per call, same as a term with two
/// distinct `cos` atoms already did before `cosh` existed.
fn reduce_trig_powers(coeff: BigScalar, mut combined: FxHashMap<AtomId, u32>, table: &mut AtomTable) -> Poly {
    enum Kind {
        Cos,
        Cosh,
    }
    let to_reduce = combined.iter().find_map(|(&id, &e)| {
        if e < 2 {
            return None;
        }
        if table.is_cos(id) {
            Some((id, e, Kind::Cos))
        } else if table.is_cosh(id) {
            Some((id, e, Kind::Cosh))
        } else {
            None
        }
    });
    let Some((atom_id, exp, kind)) = to_reduce else {
        let generators: Vec<(AtomId, u32)> = combined.into_iter().filter(|&(_, e)| e != 0).collect();
        return Poly { terms: vec![Term { coeff, generators }] };
    };

    let k = exp / 2;
    let r = exp % 2;
    combined.remove(&atom_id);
    if r == 1 {
        combined.insert(atom_id, 1);
    }

    let one = Poly::constant(BigScalar::one());
    let substitute = match kind {
        Kind::Cos => {
            let sin_id = table.sin_for_cos(atom_id);
            one.sub(&Poly { terms: vec![Term { coeff: BigScalar::one(), generators: vec![(sin_id, 2)] }] })
        }
        Kind::Cosh => {
            let sinh_id = table.sinh_for_cosh(atom_id);
            one.add(&Poly { terms: vec![Term { coeff: BigScalar::one(), generators: vec![(sinh_id, 2)] }] })
        }
    };
    let expanded = substitute.pow(k, table);

    let rest_generators: Vec<(AtomId, u32)> = combined.into_iter().filter(|&(_, e)| e != 0).collect();
    let rest = Poly { terms: vec![Term { coeff, generators: rest_generators }] };
    rest.mul(&expanded, table)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_collects_like_terms() {
        let mut t = AtomTable::new();
        let r = t.var("r");
        let x = Poly::generator(r);
        let sum = x.add(&x);
        let sorted = sum.sorted_terms(&t);
        assert_eq!(sorted.len(), 1);
        assert_eq!(sorted[0].coeff, BigScalar::new(2, 1));
    }

    #[test]
    fn mul_combines_exponents() {
        let mut t = AtomTable::new();
        let r = t.var("r");
        let x = Poly::generator(r);
        let squared = x.mul(&x, &mut t);
        let sorted = squared.sorted_terms(&t);
        assert_eq!(sorted.len(), 1);
        assert_eq!(sorted[0].generators, vec![(r, 2)]);
    }

    #[test]
    fn distributes_over_addition() {
        // r * (r + 1) = r^2 + r
        let mut t = AtomTable::new();
        let r = t.var("r");
        let x = Poly::generator(r);
        let sum = x.add(&Poly::constant(BigScalar::one()));
        let product = x.mul(&sum, &mut t);
        let sorted = product.sorted_terms(&t);
        assert_eq!(sorted.len(), 2);
    }

    #[test]
    fn cos_squared_becomes_one_minus_sin_squared() {
        let mut t = AtomTable::new();
        let theta = Expr::var("theta");
        let cos_id = t.cos(theta.clone());
        let sin_id = t.sin(theta);
        let cos = Poly::generator(cos_id);
        let cos_sq = cos.mul(&cos, &mut t);
        // Expect 1 - sin(theta)^2, i.e. two terms: constant 1 and -sin^2.
        let sorted = cos_sq.sorted_terms(&t);
        assert_eq!(sorted.len(), 2, "{sorted:?}");
        let constant = sorted.iter().find(|term| term.generators.is_empty()).expect("constant term");
        assert_eq!(constant.coeff, BigScalar::one());
        let sin_term = sorted.iter().find(|term| !term.generators.is_empty()).expect("sin^2 term");
        assert_eq!(sin_term.generators, vec![(sin_id, 2)]);
        assert_eq!(sin_term.coeff, BigScalar::new(-1, 1));
    }

    #[test]
    fn cos_cubed_keeps_one_bare_cos_factor() {
        let mut t = AtomTable::new();
        let theta = Expr::var("theta");
        let cos_id = t.cos(theta);
        let cos = Poly::generator(cos_id);
        let cos_sq = cos.mul(&cos, &mut t);
        let cos_cubed = cos_sq.mul(&cos, &mut t);
        let sorted = cos_cubed.sorted_terms(&t);
        // (1 - sin^2) * cos = cos - sin^2*cos -- two terms, each with
        // exactly one bare `cos` factor (exponent 1), never cos^2 or higher.
        for term in &sorted {
            let cos_exp = term.generators.iter().find(|(id, _)| *id == cos_id).map(|(_, e)| *e);
            assert_eq!(cos_exp, Some(1), "{sorted:?}");
        }
    }

    #[test]
    fn cosh_squared_becomes_one_plus_sinh_squared() {
        // Hyperbolic analogue of `cos_squared_becomes_one_minus_sin_squared`
        // -- sign flipped: `cosh^2 -> 1 + sinh^2`, not `1 - sinh^2`.
        let mut t = AtomTable::new();
        let chi = Expr::var("chi");
        let cosh_id = t.cosh(chi.clone());
        let sinh_id = t.sinh(chi);
        let cosh = Poly::generator(cosh_id);
        let cosh_sq = cosh.mul(&cosh, &mut t);
        let sorted = cosh_sq.sorted_terms(&t);
        assert_eq!(sorted.len(), 2, "{sorted:?}");
        let constant = sorted.iter().find(|term| term.generators.is_empty()).expect("constant term");
        assert_eq!(constant.coeff, BigScalar::one());
        let sinh_term = sorted.iter().find(|term| !term.generators.is_empty()).expect("sinh^2 term");
        assert_eq!(sinh_term.generators, vec![(sinh_id, 2)]);
        assert_eq!(sinh_term.coeff, BigScalar::one(), "unlike cos^2, the sinh^2 coefficient is +1, not -1");
    }

    #[test]
    fn cosh_cubed_keeps_one_bare_cosh_factor() {
        // Hyperbolic analogue of `cos_cubed_keeps_one_bare_cos_factor`.
        let mut t = AtomTable::new();
        let chi = Expr::var("chi");
        let cosh_id = t.cosh(chi);
        let cosh = Poly::generator(cosh_id);
        let cosh_sq = cosh.mul(&cosh, &mut t);
        let cosh_cubed = cosh_sq.mul(&cosh, &mut t);
        let sorted = cosh_cubed.sorted_terms(&t);
        for term in &sorted {
            let cosh_exp = term.generators.iter().find(|(id, _)| *id == cosh_id).map(|(_, e)| *e);
            assert_eq!(cosh_exp, Some(1), "{sorted:?}");
        }
    }

    #[test]
    fn a_term_with_both_a_reducible_cos_and_a_reducible_cosh_reduces_both() {
        // `cos(x)^2 * cosh(y)^2` -- exercises `reduce_trig_powers`
        // finding and reducing one, then converging on the other via the
        // recursive `.mul()` call, not just whichever it happens to find
        // first in the (unordered) generator map.
        let mut t = AtomTable::new();
        let x = Expr::var("x");
        let y = Expr::var("y");
        let cos_id = t.cos(x.clone());
        let sin_id = t.sin(x);
        let cosh_id = t.cosh(y.clone());
        let sinh_id = t.sinh(y);
        let cos_sq = Poly::generator(cos_id).mul(&Poly::generator(cos_id), &mut t);
        let cosh_sq = Poly::generator(cosh_id).mul(&Poly::generator(cosh_id), &mut t);
        let product = cos_sq.mul(&cosh_sq, &mut t);
        for term in product.sorted_terms(&t) {
            let cos_exp = term.generators.iter().find(|(id, _)| *id == cos_id).map(|(_, e)| *e).unwrap_or(0);
            let cosh_exp = term.generators.iter().find(|(id, _)| *id == cosh_id).map(|(_, e)| *e).unwrap_or(0);
            assert!(cos_exp <= 1, "cos left at degree {cos_exp}: {term:?}");
            assert!(cosh_exp <= 1, "cosh left at degree {cosh_exp}: {term:?}");
            // Every term's non-cos/cosh generators must be drawn only
            // from {sin, sinh} -- confirms the reduction actually ran
            // (introducing sin/sinh), not just happened to already
            // satisfy the degree bound.
            for &(id, _) in &term.generators {
                assert!(id == cos_id || id == cosh_id || id == sin_id || id == sinh_id, "unexpected generator {:?}", t.key(id));
            }
        }
    }

    #[test]
    fn atom_id_order_does_not_affect_the_canonical_sort() {
        // Intern in one order in one table, the reverse order in
        // another -- sorted_terms must agree regardless (D-RF.4).
        let mut t1 = AtomTable::new();
        let a1 = t1.var("a");
        let b1 = t1.var("b");
        let poly1 = Poly::generator(a1).add(&Poly::generator(b1));

        let mut t2 = AtomTable::new();
        let b2 = t2.var("b");
        let a2 = t2.var("a");
        let poly2 = Poly::generator(a2).add(&Poly::generator(b2));

        let sorted1: Vec<_> = poly1.sorted_terms(&t1).into_iter().map(|term| content_key(&t1, &term)).collect();
        let sorted2: Vec<_> = poly2.sorted_terms(&t2).into_iter().map(|term| content_key(&t2, &term)).collect();
        assert_eq!(sorted1, sorted2);
    }
}

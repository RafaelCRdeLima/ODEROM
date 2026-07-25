//! Arbitrary-precision rational coefficients for [`crate::Expr::Rational`].
//!
//! `oderom_core::Scalar` (i64-backed) is fine for Marco 1's combinatorial
//! signs and small coefficients, which never grow -- it stays exactly as
//! it is, used everywhere else in the workspace. It is the wrong type
//! for a symbolic CAS coefficient: naive polynomial-GCD coefficient
//! growth (DESIGN-RATIONAL-FORM.md) genuinely produces numerators/
//! denominators an i64 cannot hold, and that overflow is *silent* in a
//! release build without `overflow-checks = true` (now set in the
//! workspace `Cargo.toml`, but the real fix is not to depend on a fixed
//! width at all). `BigScalar` is used only inside `Expr::Rational`.
//!
//! # Two representations, not one `BigRational`
//!
//! An earlier version of this type wrapped [`num_rational::BigRational`]
//! directly and used it for every value, including plain small integers.
//! Measured (`ODEROM_REDUCE_STATS`, part of the performance investigation
//! this type is central to): ~1,400-5,500ns per `add`/`mul`, 50-100x more
//! than a `BigRational` holding small values should cost. Root cause,
//! confirmed by reading `num-rational`'s source: `Add` calls `Ratio::new`
//! (which reduces via GCD unconditionally), and `Mul` computes *two* GCDs
//! up front (`numer.gcd(rhs.denom)`, `denom.gcd(rhs.numer)`) plus a third
//! inside its own `Ratio::new` call -- so every multiplication of two
//! plain integers (denominator 1 either side) was computing up to three
//! GCDs whose answer is always "1", millions of times, over Schwarzschild-
//! scale coefficients that fit comfortably in an `i64`.
//!
//! `BigScalar` is now `Repr::Small(i64)` (a plain integer, denominator
//! implicitly 1: `checked_add`/`checked_mul`, no GCD, no allocation) or
//! `Repr::Big(BigRational)` (arbitrary precision, used only once a value
//! genuinely needs it: `i64` overflow, or a real non-unit denominator).
//! `Small op Small` never touches `BigRational` at all; only a promotion
//! (overflow, or a `recip()`/`new()` producing a genuine fraction) pays
//! the GCD-reduction cost, and results are demoted back to `Small`
//! whenever the reduced value turns out to be integer-and-small again --
//! a `Big` intermediate that only mattered transiently (e.g. one
//! `recip()` immediately multiplied back out) shouldn't keep paying
//! `Big`-arithmetic cost for the rest of the computation.

use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{Signed, ToPrimitive, Zero};
use oderom_core::{Render, Target};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::{Add, Mul, Neg, Sub};

#[derive(Clone)]
enum Repr {
    Small(i64),
    Big(BigRational),
}

#[derive(Clone)]
pub struct BigScalar(Repr);

impl BigScalar {
    pub fn zero() -> Self {
        BigScalar(Repr::Small(0))
    }

    pub fn one() -> Self {
        BigScalar(Repr::Small(1))
    }

    pub fn from_i64(n: i64) -> Self {
        BigScalar(Repr::Small(n))
    }

    /// `num/den` in reduced form. Panics if `den == 0`, matching
    /// `oderom_core::Scalar::new`'s contract.
    pub fn new(num: i64, den: i64) -> Self {
        assert!(den != 0, "BigScalar denominator must be nonzero");
        if den == 1 {
            return BigScalar(Repr::Small(num));
        }
        Self::from_big_rational(BigRational::new(BigInt::from(num), BigInt::from(den)))
    }

    /// Demotes back to `Repr::Small` whenever the (already-reduced, since
    /// this is the only place a `BigRational` value is ever produced)
    /// result is a plain integer that fits in an `i64` -- keeps a value
    /// that only transiently needed `Big` (e.g. one `recip()` immediately
    /// multiplied back out) from paying `Big`-arithmetic cost for the
    /// rest of a computation that no longer needs it.
    fn from_big_rational(r: BigRational) -> Self {
        if r.is_integer() {
            if let Some(n) = r.numer().to_i64() {
                return BigScalar(Repr::Small(n));
            }
        }
        BigScalar(Repr::Big(r))
    }

    fn to_big(&self) -> BigRational {
        match &self.0 {
            Repr::Small(n) => BigRational::from_integer(BigInt::from(*n)),
            Repr::Big(r) => r.clone(),
        }
    }

    pub fn is_zero(&self) -> bool {
        match &self.0 {
            Repr::Small(n) => *n == 0,
            Repr::Big(r) => r.is_zero(),
        }
    }

    pub fn is_negative(&self) -> bool {
        match &self.0 {
            Repr::Small(n) => *n < 0,
            Repr::Big(r) => r.is_negative(),
        }
    }

    /// `1/self`, or `None` for zero (which has no reciprocal).
    pub fn recip(&self) -> Option<Self> {
        match &self.0 {
            Repr::Small(0) => None,
            Repr::Small(1) => Some(BigScalar(Repr::Small(1))),
            Repr::Small(-1) => Some(BigScalar(Repr::Small(-1))),
            Repr::Small(n) => Some(BigScalar(Repr::Big(BigRational::new(BigInt::from(1), BigInt::from(*n))))),
            Repr::Big(r) => {
                if r.is_zero() {
                    None
                } else {
                    Some(Self::from_big_rational(r.recip()))
                }
            }
        }
    }

    pub fn is_integer(&self) -> bool {
        match &self.0 {
            Repr::Small(_) => true,
            Repr::Big(r) => r.is_integer(),
        }
    }

    /// Non-negative GCD, by the standard integer-GCD convention (sign is
    /// not part of a GCD; content-stripping divides it back out and each
    /// term's own sign falls out unaffected). `self`/`other` that aren't
    /// integers contribute nothing (their pairwise GCD with anything is
    /// taken as `1`, a safe no-op) -- content-stripping (part of
    /// `Poly::content`, DESIGN-RATIONAL-FORM.md's content-management
    /// invariant) is only meaningful/attempted for the integer
    /// coefficients this project's metrics actually produce; a genuinely
    /// fractional coefficient just means less gets stripped, never a
    /// wrong result.
    pub(crate) fn gcd(&self, other: &Self) -> Self {
        if self.is_zero() {
            return other.clone();
        }
        if other.is_zero() {
            return self.clone();
        }
        if !self.is_integer() || !other.is_integer() {
            return BigScalar::one();
        }
        match (&self.0, &other.0) {
            (Repr::Small(a), Repr::Small(b)) => {
                let mut a = a.unsigned_abs();
                let mut b = b.unsigned_abs();
                while b != 0 {
                    (a, b) = (b, a % b);
                }
                // `a` (the GCD) is always <= min(|a|,|b|), so this only
                // fails to fit for the i64::MIN edge case (whose
                // unsigned_abs is 2^63, one past i64::MAX) -- fall back
                // to the exact BigInt path rather than risk a silent cast
                // issue on that one value.
                match i64::try_from(a) {
                    Ok(n) => BigScalar(Repr::Small(n)),
                    Err(_) => BigScalar::from_big_rational(BigRational::from_integer(BigInt::from(a))),
                }
            }
            _ => {
                let mut a = self.to_big().numer().clone().abs();
                let mut b = other.to_big().numer().clone().abs();
                while !b.is_zero() {
                    let r = &a % &b;
                    a = b;
                    b = r;
                }
                BigScalar::from_big_rational(BigRational::from_integer(a))
            }
        }
    }

    /// Diagnostic only: rough magnitude of numerator/denominator in bits,
    /// for measuring coefficient growth during GCD reduction.
    pub fn bit_length_estimate(&self) -> u64 {
        match &self.0 {
            Repr::Small(n) => 64 - n.unsigned_abs().leading_zeros() as u64,
            Repr::Big(r) => r.numer().bits().max(r.denom().bits()),
        }
    }

    /// Lossy: only used where a numeric approximation is genuinely what's
    /// wanted (the JIT's `f64` interpreter), never for exact comparison.
    pub fn to_f64_lossy(&self) -> f64 {
        match &self.0 {
            Repr::Small(n) => *n as f64,
            Repr::Big(r) => {
                // BigRational has no direct to_f64 in all versions; compute
                // via its numerator/denominator, which does the right thing
                // for magnitudes f64 can represent and saturates to +-inf
                // beyond it.
                let n = r.numer().to_f64().unwrap_or(f64::INFINITY * r.numer().signum().to_f64().unwrap_or(1.0));
                let d = r.denom().to_f64().unwrap_or(f64::INFINITY);
                n / d
            }
        }
    }
}

impl PartialEq for BigScalar {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Repr::Small(a), Repr::Small(b)) => a == b,
            _ => self.to_big() == other.to_big(),
        }
    }
}

impl Eq for BigScalar {}

impl PartialOrd for BigScalar {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for BigScalar {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (&self.0, &other.0) {
            (Repr::Small(a), Repr::Small(b)) => a.cmp(b),
            _ => self.to_big().cmp(&other.to_big()),
        }
    }
}

impl Hash for BigScalar {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Canonical (numer, denom) pair regardless of representation, so
        // Small(3) and a Big value equal to 3/1 hash equal -- required to
        // stay consistent with Eq.
        match &self.0 {
            Repr::Small(n) => {
                n.hash(state);
                1i64.hash(state);
            }
            Repr::Big(r) => {
                r.numer().hash(state);
                r.denom().hash(state);
            }
        }
    }
}

thread_local! {
    static STATS_ENABLED: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
    static ARITH_COUNT: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static ARITH_NANOS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Cached once per thread (an `env::var` call on every single arithmetic
/// op would itself distort the measurement it's trying to take) -- see
/// `ODEROM_REDUCE_STATS`, checked once in `canonical::normalize_via_rational_form`.
fn stats_enabled() -> bool {
    STATS_ENABLED.with(|e| {
        if let Some(v) = e.get() {
            return v;
        }
        let v = std::env::var("ODEROM_REDUCE_STATS").is_ok();
        e.set(Some(v));
        v
    })
}

/// Part of the `ODEROM_REDUCE_STATS` performance investigation
/// (DESIGN-RATIONAL-FORM.md): how much of `reduce()`'s wall-clock cost is
/// arithmetic itself versus everything else (GCD control flow, `Poly`
/// bookkeeping). Reset/read once per top-level `normalize_via_rational_form`
/// call, same cadence as the reduce-call counters in `rational_function.rs`.
pub(crate) fn reset_arith_stats() {
    ARITH_COUNT.with(|c| c.set(0));
    ARITH_NANOS.with(|n| n.set(0));
}

pub(crate) fn arith_stats() -> (u64, u64) {
    (ARITH_COUNT.with(|c| c.get()), ARITH_NANOS.with(|n| n.get()))
}

impl Add for BigScalar {
    type Output = BigScalar;
    fn add(self, rhs: BigScalar) -> BigScalar {
        if stats_enabled() {
            let t0 = std::time::Instant::now();
            let result = add_impl(self, rhs);
            ARITH_COUNT.with(|c| c.set(c.get() + 1));
            ARITH_NANOS.with(|n| n.set(n.get() + t0.elapsed().as_nanos() as u64));
            result
        } else {
            add_impl(self, rhs)
        }
    }
}

fn add_impl(a: BigScalar, b: BigScalar) -> BigScalar {
    match (a.0, b.0) {
        (Repr::Small(x), Repr::Small(y)) => match x.checked_add(y) {
            Some(s) => BigScalar(Repr::Small(s)),
            None => BigScalar::from_big_rational(
                BigRational::from_integer(BigInt::from(x)) + BigRational::from_integer(BigInt::from(y)),
            ),
        },
        (x, y) => {
            let rx = match x {
                Repr::Small(n) => BigRational::from_integer(BigInt::from(n)),
                Repr::Big(r) => r,
            };
            let ry = match y {
                Repr::Small(n) => BigRational::from_integer(BigInt::from(n)),
                Repr::Big(r) => r,
            };
            BigScalar::from_big_rational(rx + ry)
        }
    }
}

impl Sub for BigScalar {
    type Output = BigScalar;
    fn sub(self, rhs: BigScalar) -> BigScalar {
        self + (-rhs)
    }
}

impl Mul for BigScalar {
    type Output = BigScalar;
    fn mul(self, rhs: BigScalar) -> BigScalar {
        if stats_enabled() {
            let t0 = std::time::Instant::now();
            let result = mul_impl(self, rhs);
            ARITH_COUNT.with(|c| c.set(c.get() + 1));
            ARITH_NANOS.with(|n| n.set(n.get() + t0.elapsed().as_nanos() as u64));
            result
        } else {
            mul_impl(self, rhs)
        }
    }
}

fn mul_impl(a: BigScalar, b: BigScalar) -> BigScalar {
    match (a.0, b.0) {
        (Repr::Small(x), Repr::Small(y)) => match x.checked_mul(y) {
            Some(p) => BigScalar(Repr::Small(p)),
            None => BigScalar::from_big_rational(
                BigRational::from_integer(BigInt::from(x)) * BigRational::from_integer(BigInt::from(y)),
            ),
        },
        (x, y) => {
            let rx = match x {
                Repr::Small(n) => BigRational::from_integer(BigInt::from(n)),
                Repr::Big(r) => r,
            };
            let ry = match y {
                Repr::Small(n) => BigRational::from_integer(BigInt::from(n)),
                Repr::Big(r) => r,
            };
            BigScalar::from_big_rational(rx * ry)
        }
    }
}

impl Neg for BigScalar {
    type Output = BigScalar;
    fn neg(self) -> BigScalar {
        match self.0 {
            Repr::Small(n) => match n.checked_neg() {
                Some(m) => BigScalar(Repr::Small(m)),
                None => BigScalar(Repr::Big(-BigRational::from_integer(BigInt::from(n)))),
            },
            Repr::Big(r) => BigScalar::from_big_rational(-r),
        }
    }
}

impl fmt::Display for BigScalar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Repr::Small(n) => write!(f, "{n}"),
            Repr::Big(r) if r.is_integer() => write!(f, "{}", r.numer()),
            Repr::Big(r) => write!(f, "{}/{}", r.numer(), r.denom()),
        }
    }
}

impl fmt::Debug for BigScalar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BigScalar({self})")
    }
}

impl Render for BigScalar {
    fn render(&self, target: Target) -> String {
        match target {
            Target::Unicode => self.to_string(),
            Target::Latex => match &self.0 {
                Repr::Small(n) => format!("{n}"),
                Repr::Big(r) if r.is_integer() => format!("{}", r.numer()),
                Repr::Big(r) if self.is_negative() => format!("-\\frac{{{}}}{{{}}}", -r.numer(), r.denom()),
                Repr::Big(r) => format!("\\frac{{{}}}{{{}}}", r.numer(), r.denom()),
            },
            Target::Json => {
                let r = self.to_big();
                format!(r#"{{"num":{},"den":{}}}"#, r.numer(), r.denom())
            }
            // Same reasoning as `oderom_core::Scalar`'s own Mathematica
            // arm: integer division between two literals is always exact
            // in Mathematica, so the bare `Display`/Unicode text is
            // already valid, correct syntax.
            Target::Mathematica => self.to_string(),
            // Same reasoning as `Scalar`'s own Sympy arm: bare `n/d` is
            // eager, inexact Python float division unless wrapped --
            // `Rational(n, d)` keeps it exact. An integer (`Repr::Small`,
            // or `Repr::Big` that happens to be a whole number) is a
            // plain literal either way, safe unwrapped.
            Target::Sympy => match &self.0 {
                Repr::Small(n) => format!("{n}"),
                Repr::Big(r) if r.is_integer() => format!("{}", r.numer()),
                Repr::Big(r) => format!("Rational({}, {})", r.numer(), r.denom()),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduces_on_construction() {
        assert_eq!(BigScalar::new(2, 4), BigScalar::new(1, 2));
    }

    #[test]
    fn arithmetic() {
        assert_eq!(BigScalar::new(1, 2) + BigScalar::new(1, 3), BigScalar::new(5, 6));
        assert_eq!(BigScalar::new(1, 2) * BigScalar::new(2, 3), BigScalar::new(1, 3));
    }

    #[test]
    fn does_not_overflow_i64_range() {
        // The exact class of value that broke i64 Scalar: something with
        // more than ~19 decimal digits, reachable in a handful of
        // multiplications the way polynomial-GCD coefficient growth does.
        let huge = BigScalar::from_i64(i64::MAX) * BigScalar::from_i64(i64::MAX) * BigScalar::from_i64(1_000_000);
        let doubled = huge.clone() + huge;
        assert!(!doubled.is_zero());
    }

    #[test]
    fn display_matches_scalar_convention() {
        assert_eq!(BigScalar::new(3, 1).to_string(), "3");
        assert_eq!(BigScalar::new(3, 4).to_string(), "3/4");
        assert_eq!(BigScalar::new(-3, 4).to_string(), "-3/4");
    }

    #[test]
    fn small_times_small_stays_small_and_matches_big() {
        // A regression test for the representation split itself: a
        // Small-path result must be indistinguishable (Eq, Ord, Hash, and
        // value) from what the old single-BigRational representation
        // would have produced.
        let a = BigScalar::from_i64(3) * BigScalar::from_i64(5);
        assert_eq!(a, BigScalar::new(15, 1));
    }

    #[test]
    fn recip_then_multiply_back_demotes_to_small() {
        // 1/7 * 7 = 1 -- a Big intermediate (1/7) whose final value is a
        // small integer must demote back to Small, not stay Big forever.
        let seven = BigScalar::from_i64(7);
        let recip = seven.recip().unwrap();
        let back = recip * BigScalar::from_i64(7);
        assert_eq!(back, BigScalar::one());
    }

    #[test]
    fn overflowing_small_arithmetic_promotes_correctly() {
        let big = BigScalar::from_i64(i64::MAX) + BigScalar::from_i64(1);
        assert_eq!(big, BigScalar::new(i64::MAX, 1) + BigScalar::one());
        let big2 = BigScalar::from_i64(i64::MAX) * BigScalar::from_i64(2);
        assert_eq!(big2.to_f64_lossy(), (i64::MAX as f64) * 2.0);
    }
}

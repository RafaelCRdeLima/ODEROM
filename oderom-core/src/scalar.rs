//! Rational scalar coefficients. Marco 1 admits no irrationals, floats, or
//! symbolic constants: `Scalar` is exactly `Q`, represented as a reduced
//! `i64` fraction.

use std::fmt;
use std::ops::{Add, Mul, Neg, Sub};

/// A rational number, always kept in reduced form: `gcd(|num|, den) == 1`,
/// `den > 0`, and `num == 0 => den == 1`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Scalar {
    num: i64,
    den: i64,
}

impl Scalar {
    pub const ZERO: Scalar = Scalar { num: 0, den: 1 };
    pub const ONE: Scalar = Scalar { num: 1, den: 1 };

    /// Builds `num/den` in reduced form. Panics if `den == 0`.
    pub fn new(num: i64, den: i64) -> Self {
        assert!(den != 0, "Scalar denominator must be nonzero");
        Self::reduce(num, den)
    }

    /// Builds the integer `n` as a `Scalar`.
    pub fn from_int(n: i64) -> Self {
        Scalar { num: n, den: 1 }
    }

    fn reduce(num: i64, den: i64) -> Self {
        let flip = if den < 0 { -1 } else { 1 };
        let num = num * flip;
        let den = den * flip;
        if num == 0 {
            return Scalar::ZERO;
        }
        let g = gcd(num.unsigned_abs(), den.unsigned_abs()) as i64;
        Scalar { num: num / g, den: den / g }
    }

    pub fn numerator(&self) -> i64 {
        self.num
    }

    /// Always positive.
    pub fn denominator(&self) -> i64 {
        self.den
    }

    pub fn is_zero(&self) -> bool {
        self.num == 0
    }

    /// `1/self`, or `None` for zero (which has no reciprocal).
    pub fn recip(self) -> Option<Scalar> {
        if self.num == 0 {
            None
        } else {
            Some(Scalar::reduce(self.den, self.num))
        }
    }
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

impl Add for Scalar {
    type Output = Scalar;
    fn add(self, rhs: Scalar) -> Scalar {
        Scalar::reduce(self.num * rhs.den + rhs.num * self.den, self.den * rhs.den)
    }
}

impl Sub for Scalar {
    type Output = Scalar;
    fn sub(self, rhs: Scalar) -> Scalar {
        self + (-rhs)
    }
}

impl Mul for Scalar {
    type Output = Scalar;
    fn mul(self, rhs: Scalar) -> Scalar {
        Scalar::reduce(self.num * rhs.num, self.den * rhs.den)
    }
}

impl Neg for Scalar {
    type Output = Scalar;
    fn neg(self) -> Scalar {
        Scalar { num: -self.num, den: self.den }
    }
}

impl fmt::Display for Scalar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}

impl fmt::Debug for Scalar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Scalar({self})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduces_on_construction() {
        assert_eq!(Scalar::new(2, 4), Scalar::new(1, 2));
        assert_eq!(Scalar::new(-2, 4), Scalar::new(1, -2));
        assert_eq!(Scalar::new(0, 5), Scalar::ZERO);
    }

    #[test]
    fn denominator_always_positive() {
        let s = Scalar::new(3, -7);
        assert_eq!(s.denominator(), 7);
        assert_eq!(s.numerator(), -3);
    }

    #[test]
    fn arithmetic() {
        assert_eq!(Scalar::new(1, 2) + Scalar::new(1, 3), Scalar::new(5, 6));
        assert_eq!(Scalar::new(1, 2) * Scalar::new(2, 3), Scalar::new(1, 3));
        assert_eq!(-Scalar::new(1, 2), Scalar::new(-1, 2));
        assert_eq!(Scalar::new(1, 2) - Scalar::new(1, 2), Scalar::ZERO);
    }

    #[test]
    fn recip() {
        assert_eq!(Scalar::new(2, 3).recip(), Some(Scalar::new(3, 2)));
        assert_eq!(Scalar::new(-2, 3).recip(), Some(Scalar::new(-3, 2)));
        assert_eq!(Scalar::ZERO.recip(), None);
    }

    #[test]
    fn display() {
        assert_eq!(Scalar::new(3, 1).to_string(), "3");
        assert_eq!(Scalar::new(3, 4).to_string(), "3/4");
        assert_eq!(Scalar::new(-3, 4).to_string(), "-3/4");
    }

    #[test]
    #[should_panic]
    fn zero_denominator_panics() {
        Scalar::new(1, 0);
    }
}

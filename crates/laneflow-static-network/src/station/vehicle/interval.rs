//! 向外舍入的几何闭区间。输入的非有限运算只会扩大包络。
use std::ops::{Add, Div, Mul, Neg, Sub};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Interval {
    pub lo: f64,
    pub hi: f64,
}

impl Interval {
    pub const ZERO: Self = Self::point(0.);
    pub const ONE: Self = Self::point(1.);
    pub const UNIT: Self = Self { lo: 0., hi: 1. };
    const ALL: Self = Self {
        lo: f64::NEG_INFINITY,
        hi: f64::INFINITY,
    };

    pub const fn point(v: f64) -> Self {
        Self { lo: v, hi: v }
    }

    pub fn rounded(lo: f64, hi: f64) -> Self {
        if lo.is_nan() || hi.is_nan() {
            Self::ALL
        } else {
            Self {
                lo: lo.next_down(),
                hi: hi.next_up(),
            }
        }
    }

    pub fn from_millimetres(mm: u32) -> Self {
        Self::point(f64::from(mm)) / Self::point(1_000.)
    }

    pub fn meters(nm: u64) -> Self {
        let v = nm as f64;
        let n = if nm <= 1 << 53 {
            Self::point(v)
        } else {
            Self::rounded(v, v)
        };
        n / Self::point(1e9)
    }

    pub fn with_error(self, radius: f64) -> Self {
        if radius == 0. {
            return self;
        }
        Self::rounded(self.lo - radius, self.hi + radius)
    }

    pub fn hull(self, other: Self) -> Self {
        Self {
            lo: self.lo.min(other.lo),
            hi: self.hi.max(other.hi),
        }
    }

    pub fn intersect(self, other: Self) -> Option<Self> {
        let lo = self.lo.max(other.lo);
        let hi = self.hi.min(other.hi);
        (lo <= hi).then_some(Self { lo, hi })
    }

    pub fn square(self) -> Self {
        if self == Self::ZERO {
            return Self::ZERO;
        }
        let lo = if self.lo > 0. {
            self.lo
        } else if self.hi < 0. {
            -self.hi
        } else {
            0.
        };
        let hi = self.lo.abs().max(self.hi.abs());
        let mut value = Self::rounded(lo * lo, hi * hi);
        value.lo = value.lo.max(0.);
        value
    }

    pub fn sqrt(self) -> Self {
        let mut value = Self::rounded(self.lo.max(0.).sqrt(), self.hi.sqrt());
        value.lo = value.lo.max(0.);
        value
    }

    pub fn finite(self) -> bool {
        self.lo.is_finite() && self.hi.is_finite() && self.lo <= self.hi
    }

    pub fn abs(self) -> Self {
        Self {
            lo: self.lo.max(-self.hi).max(0.),
            hi: self.lo.abs().max(self.hi.abs()),
        }
    }

    pub fn bounds(self) -> [f64; 2] {
        [self.lo, self.hi]
    }
}

impl Add for Interval {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        if self == Self::ZERO {
            return rhs;
        }
        if rhs == Self::ZERO {
            return self;
        }
        Self::rounded(self.lo + rhs.lo, self.hi + rhs.hi)
    }
}
impl Neg for Interval {
    type Output = Self;
    fn neg(self) -> Self {
        Self {
            lo: -self.hi,
            hi: -self.lo,
        }
    }
}
impl Sub for Interval {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        self + (-rhs)
    }
}
impl Mul for Interval {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        if self == Self::ZERO || rhs == Self::ZERO {
            return Self::ZERO;
        }
        if self == Self::ONE {
            return rhs;
        }
        if rhs == Self::ONE {
            return self;
        }
        let corners = [
            self.lo * rhs.lo,
            self.lo * rhs.hi,
            self.hi * rhs.lo,
            self.hi * rhs.hi,
        ];
        if corners.iter().any(|x| x.is_nan()) {
            return Self::ALL;
        }
        Self::rounded(
            corners.into_iter().fold(f64::INFINITY, f64::min),
            corners.into_iter().fold(f64::NEG_INFINITY, f64::max),
        )
    }
}
impl Div for Interval {
    type Output = Self;
    fn div(self, rhs: Self) -> Self {
        if rhs.lo <= 0. && rhs.hi >= 0. {
            return Self::ALL;
        }
        if rhs == Self::ONE {
            return self;
        }
        self * Self::rounded(1. / rhs.hi, 1. / rhs.lo)
    }
}

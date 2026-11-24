//! ADR 0022 §6.1。中心值决定规范点，外包区间独立记录相同运算的舍入和积分截断误差。
use std::ops::{Add, Div, Mul, Neg, Sub};

use super::geometry::{CurveSample, NumericFreezeError, NumericFreezeKind, Point3};
use super::model::RoadEditingCurveSegmentGeometry;

const DEGREE: usize = 16;
const TERMS: usize = 2 * DEGREE + 1;
// 0.5^17/17! < 2.2e-20。切向与归一化积分各分量共用此向上界。
const TAIL: f64 = 2.2e-20;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Scalar {
    pub value: f64,
    pub lo: f64,
    pub hi: f64,
}

impl Scalar {
    pub fn exact(value: f64) -> Self {
        Self {
            value,
            lo: value,
            hi: value,
        }
    }
    fn enclose(value: f64, lo: f64, hi: f64) -> Self {
        if lo.is_nan() || hi.is_nan() || value.is_nan() {
            return Self {
                value,
                lo: f64::NEG_INFINITY,
                hi: f64::INFINITY,
            };
        }
        Self {
            value,
            lo: lo.next_down(),
            hi: hi.next_up(),
        }
    }
    pub fn widen(self, error: f64) -> Self {
        Self::enclose(self.value, self.lo - error, self.hi + error)
    }
    pub fn abs_max(self) -> f64 {
        self.lo.abs().max(self.hi.abs())
    }
    pub fn square(self) -> Self {
        let a = self.lo * self.lo;
        let b = self.hi * self.hi;
        let min = if self.lo <= 0.0 && self.hi >= 0.0 {
            0.0
        } else {
            a.min(b)
        };
        Self::enclose(self.value * self.value, min, a.max(b))
    }
    pub fn sqrt(self) -> Self {
        Self::enclose(self.value.sqrt(), self.lo.max(0.0).sqrt(), self.hi.sqrt())
    }
    pub fn finite(self) -> Result<Self, NumericFreezeError> {
        if self.value.is_finite() && self.lo.is_finite() && self.hi.is_finite() {
            Ok(self)
        } else {
            Err(NumericFreezeKind::NonFinite.into())
        }
    }
}

impl Add for Scalar {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::enclose(self.value + rhs.value, self.lo + rhs.lo, self.hi + rhs.hi)
    }
}
impl Sub for Scalar {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self::enclose(self.value - rhs.value, self.lo - rhs.hi, self.hi - rhs.lo)
    }
}
impl Neg for Scalar {
    type Output = Self;
    fn neg(self) -> Self {
        Self {
            value: -self.value,
            lo: -self.hi,
            hi: -self.lo,
        }
    }
}
impl Mul for Scalar {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        let corners = [
            self.lo * rhs.lo,
            self.lo * rhs.hi,
            self.hi * rhs.lo,
            self.hi * rhs.hi,
        ];
        if corners.iter().any(|v| v.is_nan()) {
            return Self::enclose(self.value * rhs.value, f64::NEG_INFINITY, f64::INFINITY);
        }
        Self::enclose(
            self.value * rhs.value,
            corners.iter().copied().fold(f64::INFINITY, f64::min),
            corners.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        )
    }
}
impl Div for Scalar {
    type Output = Self;
    fn div(self, rhs: Self) -> Self {
        let value = self.value / rhs.value;
        if rhs.lo <= 0.0 && rhs.hi >= 0.0 {
            return Self {
                value,
                lo: f64::NEG_INFINITY,
                hi: f64::INFINITY,
            };
        }
        let corners = [
            self.lo / rhs.lo,
            self.lo / rhs.hi,
            self.hi / rhs.lo,
            self.hi / rhs.hi,
        ];
        Self::enclose(
            value,
            corners.iter().copied().fold(f64::INFINITY, f64::min),
            corners.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        )
    }
}

pub(super) type Vector = [Scalar; 3];
pub(super) fn exact_point(p: Point3) -> Vector {
    [p.x, p.y, p.z].map(Scalar::exact)
}
pub(super) fn point(v: Vector) -> Result<Point3, NumericFreezeError> {
    Point3::try_new(
        v[0].finite()?.value,
        v[1].finite()?.value,
        v[2].finite()?.value,
    )
}
pub(super) fn norm(v: Vector) -> Scalar {
    (v[0].square() + v[1].square() + v[2].square()).sqrt()
}
pub(super) fn unit(v: Vector) -> Result<Vector, NumericFreezeError> {
    let scale = v.iter().map(|x| x.value.abs()).fold(0.0, f64::max);
    if scale == 0.0 || !scale.is_finite() {
        return Err(NumericFreezeKind::HorizontalDerivativeZero.into());
    }
    let scaled = v.map(|x| x / Scalar::exact(scale));
    let len = norm(scaled);
    if len.lo <= 0.0 {
        return Err(NumericFreezeKind::HorizontalDerivativeNotProvenNonZero.into());
    }
    Ok(scaled.map(|x| x / len))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Law {
    pub length: Scalar,
    pub k0: Scalar,
    pub k1: Scalar,
    pub grade: Scalar,
}

impl Law {
    pub fn from_input(input: RoadEditingCurveSegmentGeometry) -> Self {
        let (length, k0, k1, grade) = match input {
            RoadEditingCurveSegmentGeometry::CircularArc {
                length_meters,
                signed_radius_meters,
                grade,
            } => {
                let k = Scalar::exact(1.0) / Scalar::exact(signed_radius_meters);
                (length_meters, k, k, grade)
            }
            RoadEditingCurveSegmentGeometry::Clothoid {
                length_meters,
                start_curvature_per_meter,
                end_curvature_per_meter,
                grade,
            } => (
                length_meters,
                Scalar::exact(start_curvature_per_meter),
                Scalar::exact(end_curvature_per_meter),
                grade,
            ),
            _ => unreachable!("only native segments carry an engineering law"),
        };
        Self {
            length: Scalar::exact(length),
            k0,
            k1,
            grade: Scalar::exact(grade),
        }
    }
    pub fn curvature(self, t: f64) -> Scalar {
        if t == 0.0 || self.k0 == self.k1 {
            self.k0
        } else if t == 1.0 {
            self.k1
        } else {
            self.k0 + (self.k1 - self.k0) * Scalar::exact(t)
        }
    }
    pub fn road_length(self) -> Scalar {
        self.length * (Scalar::exact(1.0) + self.grade.square()).sqrt()
    }

    pub fn walk(
        self,
        mut visit: impl FnMut(f64, f64) -> Result<(), NumericFreezeError>,
    ) -> Result<usize, NumericFreezeError> {
        let mut stack = [(0.0, 0.0, 0_u8); 21];
        stack[0] = (0.0, 1.0, 0);
        let mut pending = 1;
        let mut visits = 0;
        let mut leaves = 0;
        while pending > 0 {
            pending -= 1;
            let (a, b, depth) = stack[pending];
            visits += 1;
            if visits > 4095 {
                return Err(NumericFreezeKind::ApproximationNotConverged.into());
            }
            let h = self.length * Scalar::exact(b - a);
            let curvature = self.curvature(a).abs_max().max(self.curvature(b).abs_max());
            if (h * Scalar::exact(curvature)).hi <= 0.5 {
                visit(a, b)?;
                leaves += 1;
            } else {
                if depth == 20 {
                    return Err(NumericFreezeKind::ApproximationNotConverged.into());
                }
                let m = a + (b - a) * 0.5;
                stack[pending] = (m, b, depth + 1);
                stack[pending + 1] = (a, m, depth + 1);
                pending += 2;
            }
        }
        Ok(leaves)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Pair(Scalar, Scalar);

impl Pair {
    fn add(self, other: Self) -> Self {
        Self(self.0 + other.0, self.1 + other.1)
    }
    fn mul(self, factor: Scalar) -> Self {
        Self(self.0 * factor, self.1 * factor)
    }
    fn div(self, factor: Scalar) -> Self {
        Self(self.0 / factor, self.1 / factor)
    }
    fn widen(self) -> Self {
        Self(self.0.widen(TAIL), self.1.widen(TAIL))
    }
    fn in_frame(self, tangent: Self) -> Self {
        Self(
            tangent.0 * self.0 + tangent.1 * self.1,
            tangent.1 * self.0 - tangent.0 * self.1,
        )
    }
}

#[derive(Debug, PartialEq)]
pub(super) struct Block {
    a: f64,
    b: f64,
    h: Scalar,
    position: Pair,
    tangent: Pair,
    coefficients: [Pair; TERMS],
}
impl Block {
    fn new(law: Law, a: f64, b: f64, position: Pair, tangent: Pair) -> Self {
        let h = law.length * Scalar::exact(b - a);
        let linear = h * law.curvature(a);
        let quadratic = h * (law.curvature(b) - law.curvature(a)) / Scalar::exact(2.0);
        let mut term = [Pair::default(); TERMS];
        term[0].0 = Scalar::exact(1.0);
        let mut coefficients = term;
        for j in 1..=DEGREE {
            let divisor = Scalar::exact(j as f64);
            let previous = term;
            for m in 1..=2 * j {
                let first = previous[m - 1].mul(linear);
                let second = if m < 2 {
                    Pair::default()
                } else {
                    previous[m - 2].mul(quadratic)
                };
                let sum = first.add(second);
                term[m] = Pair(-sum.1, sum.0).div(divisor);
                coefficients[m] = coefficients[m].add(term[m]);
            }
            term[0] = Pair::default();
        }
        Self {
            a,
            b,
            h,
            position,
            tangent,
            coefficients,
        }
    }
    fn sample(&self, t: f64) -> (Pair, Pair) {
        if t == self.a {
            return (self.position, self.tangent);
        }
        let v = (Scalar::exact(t) - Scalar::exact(self.a)) / Scalar::exact(self.b - self.a);
        let mut direction = Pair::default();
        let mut displacement = Pair::default();
        for (m, &c) in self.coefficients.iter().enumerate().rev() {
            direction = direction.mul(v).add(c);
            displacement = displacement
                .mul(v)
                .add(c.div(Scalar::exact((m + 1) as f64)));
        }
        (
            self.position.add(
                displacement
                    .mul(v)
                    .widen()
                    .mul(self.h)
                    .in_frame(self.tangent),
            ),
            direction.widen().in_frame(self.tangent),
        )
    }
}

#[derive(Debug, PartialEq)]
pub(crate) struct NativeCurve {
    pub(super) law: Law,
    start: Vector,
    blocks: Box<[Block]>,
}
pub(super) const fn preparation_stack_bytes() -> u64 {
    (4 * std::mem::size_of::<[Pair; TERMS]>() + std::mem::size_of::<[(f64, f64, u8); 21]>()) as u64
}
impl NativeCurve {
    pub(super) fn block_count(&self) -> usize {
        self.blocks.len()
    }
    pub(super) fn new(
        law: Law,
        start: Vector,
        entry: Vector,
        count: usize,
    ) -> Result<Self, NumericFreezeError> {
        let entry = unit([entry[0], Scalar::exact(0.0), entry[2]])?;
        let mut tangent = Pair(entry[0], entry[2]);
        let mut position = Pair(start[0], start[2]);
        let mut blocks = Vec::with_capacity(count);
        law.walk(|a, b| {
            let block = Block::new(law, a, b, position, tangent);
            (position, tangent) = block.sample(b);
            position.0.finite()?;
            position.1.finite()?;
            tangent.0.finite()?;
            tangent.1.finite()?;
            blocks.push(block);
            Ok(())
        })?;
        Ok(Self {
            law,
            start,
            blocks: blocks.into_boxed_slice(),
        })
    }
    pub(super) fn vectors(&self, t: f64) -> Result<(Vector, Vector), NumericFreezeError> {
        if !(0.0..=1.0).contains(&t) {
            return Err(NumericFreezeKind::StationOutOfRange.into());
        }
        let i = self.blocks.partition_point(|block| block.b < t);
        let (p, tangent) = self.blocks[i].sample(t);
        Ok((
            [
                p.0,
                self.start[1] + self.law.grade * (self.law.length * Scalar::exact(t)),
                p.1,
            ],
            [tangent.0, self.law.grade, tangent.1],
        ))
    }
    pub(super) fn sample(&self, t: f64) -> Result<CurveSample, NumericFreezeError> {
        let (p, derivative) = self.vectors(t)?;
        Ok(CurveSample {
            point: point(p)?,
            first: point(derivative.map(|c| c * self.law.length))?,
        })
    }
}

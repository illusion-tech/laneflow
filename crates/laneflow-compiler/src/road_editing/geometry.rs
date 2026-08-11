//! ADR 0022 B1 的确定性曲线数值内核。
//!
//! 本模块只实现固定 scalar-dual 运算图、line/cubic evaluator 和 offset 前置的
//! horizontal-regularity walk。点表细分、station 强制边界和共同 Typed AST 组装由后续
//! 切片组合，避免在数值内核中建立第二套拓扑或资源权威。

const MAX_SUBDIVISION_DEPTH: u8 = 20;
const MAX_REGULARITY_NODE_VISITS: u32 = 4095;
const REGULARITY_STACK_CAPACITY: usize = 21;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NumericFreezeError {
    NonFinite,
    DivisionByZero,
    SquareRootDomain,
    HorizontalDerivativeZero,
    HorizontalDerivativeNotProvenNonZero,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Point3 {
    pub(super) x: f64,
    pub(super) y: f64,
    pub(super) z: f64,
}

impl Point3 {
    pub(super) fn try_new(x: f64, y: f64, z: f64) -> Result<Self, NumericFreezeError> {
        Ok(Self {
            x: finite(x)?,
            y: finite(y)?,
            z: finite(z)?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum CurveSegment {
    Line {
        start: Point3,
        end: Point3,
    },
    CubicBezier {
        start: Point3,
        control_1: Point3,
        control_2: Point3,
        end: Point3,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct CurveSample {
    pub(super) point: Point3,
    pub(super) first: Point3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Dual {
    value: f64,
    first: f64,
}

impl Dual {
    fn constant(value: f64) -> Result<Self, NumericFreezeError> {
        Ok(Self {
            value: finite(value)?,
            first: 0.0,
        })
    }

    fn parameter(value: f64) -> Result<Self, NumericFreezeError> {
        Ok(Self {
            value: finite(value)?,
            first: 1.0,
        })
    }

    fn add(self, other: Self) -> Result<Self, NumericFreezeError> {
        Ok(Self {
            value: finite(self.value + other.value)?,
            first: finite(self.first + other.first)?,
        })
    }

    fn sub(self, other: Self) -> Result<Self, NumericFreezeError> {
        Ok(Self {
            value: finite(self.value - other.value)?,
            first: finite(self.first - other.first)?,
        })
    }

    fn neg(self) -> Result<Self, NumericFreezeError> {
        Ok(Self {
            value: finite(-self.value)?,
            first: finite(-self.first)?,
        })
    }

    fn mul(self, other: Self) -> Result<Self, NumericFreezeError> {
        let value = finite(self.value * other.value)?;
        let left_first = finite(self.first * other.value)?;
        let right_first = finite(self.value * other.first)?;
        Ok(Self {
            value,
            first: finite(left_first + right_first)?,
        })
    }

    fn div(self, other: Self) -> Result<Self, NumericFreezeError> {
        if other.value == 0.0 {
            return Err(NumericFreezeError::DivisionByZero);
        }
        let value = finite(self.value / other.value)?;
        let left_first = finite(self.first * other.value)?;
        let right_first = finite(self.value * other.first)?;
        let numerator = finite(left_first - right_first)?;
        let denominator = finite(other.value * other.value)?;
        if denominator == 0.0 {
            return Err(NumericFreezeError::DivisionByZero);
        }
        Ok(Self {
            value,
            first: finite(numerator / denominator)?,
        })
    }

    fn sqrt(self) -> Result<Self, NumericFreezeError> {
        if self.value <= 0.0 {
            return Err(NumericFreezeError::SquareRootDomain);
        }
        let value = finite(self.value.sqrt())?;
        let denominator = finite(2.0 * value)?;
        Ok(Self {
            value,
            first: finite(self.first / denominator)?,
        })
    }

    fn lerp(self, other: Self, parameter: Self) -> Result<Self, NumericFreezeError> {
        self.add(parameter.mul(other.sub(self)?)?)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct DualPoint3 {
    x: Dual,
    y: Dual,
    z: Dual,
}

impl DualPoint3 {
    fn constant(point: Point3) -> Result<Self, NumericFreezeError> {
        Ok(Self {
            x: Dual::constant(point.x)?,
            y: Dual::constant(point.y)?,
            z: Dual::constant(point.z)?,
        })
    }

    fn add(self, other: Self) -> Result<Self, NumericFreezeError> {
        Ok(Self {
            x: self.x.add(other.x)?,
            y: self.y.add(other.y)?,
            z: self.z.add(other.z)?,
        })
    }

    fn mul(self, scalar: Dual) -> Result<Self, NumericFreezeError> {
        Ok(Self {
            x: self.x.mul(scalar)?,
            y: self.y.mul(scalar)?,
            z: self.z.mul(scalar)?,
        })
    }

    fn lerp(self, other: Self, parameter: Dual) -> Result<Self, NumericFreezeError> {
        Ok(Self {
            x: self.x.lerp(other.x, parameter)?,
            y: self.y.lerp(other.y, parameter)?,
            z: self.z.lerp(other.z, parameter)?,
        })
    }

    fn sample(self) -> Result<CurveSample, NumericFreezeError> {
        Ok(CurveSample {
            point: Point3::try_new(self.x.value, self.y.value, self.z.value)?,
            first: Point3::try_new(self.x.first, self.y.first, self.z.first)?,
        })
    }
}

impl CurveSegment {
    pub(super) fn evaluate(self, parameter: f64) -> Result<CurveSample, NumericFreezeError> {
        self.evaluate_dual(Dual::parameter(parameter)?)?.sample()
    }

    fn horizontal_derivative(self, parameter: Dual) -> Result<(Dual, Dual), NumericFreezeError> {
        match self {
            Self::Line { start, end } => Ok((
                Dual::constant(finite(end.x - start.x)?)?,
                Dual::constant(finite(end.z - start.z)?)?,
            )),
            Self::CubicBezier {
                start,
                control_1,
                control_2,
                end,
            } => {
                let controls = derivative_controls(start, control_1, control_2, end)?;
                let q0_x = Dual::constant(controls[0].x)?;
                let q1_x = Dual::constant(controls[1].x)?;
                let q2_x = Dual::constant(controls[2].x)?;
                let q0_z = Dual::constant(controls[0].z)?;
                let q1_z = Dual::constant(controls[1].z)?;
                let q2_z = Dual::constant(controls[2].z)?;
                let x01 = q0_x.lerp(q1_x, parameter)?;
                let x12 = q1_x.lerp(q2_x, parameter)?;
                let z01 = q0_z.lerp(q1_z, parameter)?;
                let z12 = q1_z.lerp(q2_z, parameter)?;
                Ok((x01.lerp(x12, parameter)?, z01.lerp(z12, parameter)?))
            }
        }
    }

    pub(super) fn evaluate_offset(
        self,
        parameter: f64,
        station: StationInterval,
        offset: OffsetInterval,
    ) -> Result<CurveSample, NumericFreezeError> {
        let parameter = Dual::parameter(parameter)?;
        let base = self.evaluate_dual(parameter)?;
        let (horizontal_x, horizontal_z) = self.horizontal_derivative(parameter)?;
        let horizontal_x_squared = horizontal_x.mul(horizontal_x)?;
        let horizontal_z_squared = horizontal_z.mul(horizontal_z)?;
        let horizontal_norm = horizontal_x_squared.add(horizontal_z_squared)?.sqrt()?;
        let left = DualPoint3 {
            x: horizontal_z.div(horizontal_norm)?,
            y: Dual::constant(0.0)?,
            z: horizontal_x.neg()?.div(horizontal_norm)?,
        };

        let station_t0 = Dual::constant(station.parameter_start)?;
        let station_t1 = Dual::constant(station.parameter_end)?;
        let cumulative_start = Dual::constant(station.cumulative_start_meters)?;
        let cumulative_end = Dual::constant(station.cumulative_end_meters)?;
        let local_parameter = parameter
            .sub(station_t0)?
            .div(station_t1.sub(station_t0)?)?;
        let station_value =
            cumulative_start.add(cumulative_end.sub(cumulative_start)?.mul(local_parameter)?)?;
        let offset_station_start = Dual::constant(offset.station_start_meters)?;
        let offset_station_end = Dual::constant(offset.station_end_meters)?;
        let offset_start = Dual::constant(offset.offset_start_meters)?;
        let offset_end = Dual::constant(offset.offset_end_meters)?;
        let offset_parameter = station_value
            .sub(offset_station_start)?
            .div(offset_station_end.sub(offset_station_start)?)?;
        let offset_value =
            offset_start.add(offset_end.sub(offset_start)?.mul(offset_parameter)?)?;
        base.add(left.mul(offset_value)?)?.sample()
    }

    fn evaluate_dual(self, parameter: Dual) -> Result<DualPoint3, NumericFreezeError> {
        match self {
            Self::Line { start, end } => {
                DualPoint3::constant(start)?.lerp(DualPoint3::constant(end)?, parameter)
            }
            Self::CubicBezier {
                start,
                control_1,
                control_2,
                end,
            } => {
                let p0 = DualPoint3::constant(start)?;
                let p1 = DualPoint3::constant(control_1)?;
                let p2 = DualPoint3::constant(control_2)?;
                let p3 = DualPoint3::constant(end)?;
                let a = p0.lerp(p1, parameter)?;
                let b = p1.lerp(p2, parameter)?;
                let c = p2.lerp(p3, parameter)?;
                let d = a.lerp(b, parameter)?;
                let e = b.lerp(c, parameter)?;
                d.lerp(e, parameter)
            }
        }
    }

    pub(super) fn prove_horizontal_regularity(self) -> Result<u32, NumericFreezeError> {
        match self {
            Self::Line { start, end } => {
                let x = finite(end.x - start.x)?;
                let z = finite(end.z - start.z)?;
                if x == 0.0 && z == 0.0 {
                    Err(NumericFreezeError::HorizontalDerivativeZero)
                } else {
                    Ok(0)
                }
            }
            Self::CubicBezier {
                start,
                control_1,
                control_2,
                end,
            } => {
                let controls = derivative_controls(start, control_1, control_2, end)?;
                if controls
                    .iter()
                    .all(|control| control.x == 0.0 && control.z == 0.0)
                {
                    return Err(NumericFreezeError::HorizontalDerivativeZero);
                }
                regularity_walk(controls)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct StationInterval {
    pub(super) parameter_start: f64,
    pub(super) parameter_end: f64,
    pub(super) cumulative_start_meters: f64,
    pub(super) cumulative_end_meters: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct OffsetInterval {
    pub(super) station_start_meters: f64,
    pub(super) station_end_meters: f64,
    pub(super) offset_start_meters: f64,
    pub(super) offset_end_meters: f64,
}

fn derivative_controls(
    p0: Point3,
    p1: Point3,
    p2: Point3,
    p3: Point3,
) -> Result<[Point3; 3], NumericFreezeError> {
    Ok([
        derivative_control(p0, p1)?,
        derivative_control(p1, p2)?,
        derivative_control(p2, p3)?,
    ])
}

fn derivative_control(start: Point3, end: Point3) -> Result<Point3, NumericFreezeError> {
    let x = finite(end.x - start.x)?;
    let y = finite(end.y - start.y)?;
    let z = finite(end.z - start.z)?;
    Point3::try_new(finite(3.0 * x)?, finite(3.0 * y)?, finite(3.0 * z)?)
}

#[derive(Clone, Copy, Debug)]
struct Interval {
    low: f64,
    high: f64,
}

impl Interval {
    fn point(value: f64) -> Result<Self, NumericFreezeError> {
        let value = finite(value)?;
        Ok(Self {
            low: value,
            high: value,
        })
    }

    fn sub(self, other: Self) -> Result<Self, NumericFreezeError> {
        Ok(Self {
            low: next_down(finite(self.low - other.high)?)?,
            high: next_up(finite(self.high - other.low)?)?,
        })
    }

    fn mul_half(self) -> Result<Self, NumericFreezeError> {
        Ok(Self {
            low: next_down(finite(self.low * 0.5)?)?,
            high: next_up(finite(self.high * 0.5)?)?,
        })
    }

    fn add(self, other: Self) -> Result<Self, NumericFreezeError> {
        Ok(Self {
            low: next_down(finite(self.low + other.low)?)?,
            high: next_up(finite(self.high + other.high)?)?,
        })
    }

    fn midpoint_lerp(self, other: Self) -> Result<Self, NumericFreezeError> {
        self.add(other.sub(self)?.mul_half()?)
    }
}

#[derive(Clone, Copy, Debug)]
struct IntervalPoint2 {
    x: Interval,
    z: Interval,
}

impl IntervalPoint2 {
    fn point(value: Point3) -> Result<Self, NumericFreezeError> {
        Ok(Self {
            x: Interval::point(value.x)?,
            z: Interval::point(value.z)?,
        })
    }

    fn midpoint_lerp(self, other: Self) -> Result<Self, NumericFreezeError> {
        Ok(Self {
            x: self.x.midpoint_lerp(other.x)?,
            z: self.z.midpoint_lerp(other.z)?,
        })
    }
}

#[derive(Clone, Copy, Debug)]
struct RegularityNode {
    controls: [IntervalPoint2; 3],
    depth: u8,
}

fn regularity_walk(controls: [Point3; 3]) -> Result<u32, NumericFreezeError> {
    let root = RegularityNode {
        controls: [
            IntervalPoint2::point(controls[0])?,
            IntervalPoint2::point(controls[1])?,
            IntervalPoint2::point(controls[2])?,
        ],
        depth: 0,
    };
    let mut stack = [None; REGULARITY_STACK_CAPACITY];
    stack[0] = Some(root);
    let mut stack_len = 1_usize;
    let mut visits = 0_u32;

    while stack_len != 0 {
        if visits == MAX_REGULARITY_NODE_VISITS {
            return Err(NumericFreezeError::HorizontalDerivativeNotProvenNonZero);
        }
        stack_len -= 1;
        let node = stack[stack_len]
            .take()
            .expect("fixed regularity stack contains every live node");
        visits += 1;
        if hull_proves_nonzero(node.controls) {
            continue;
        }
        if node.depth == MAX_SUBDIVISION_DEPTH {
            return Err(NumericFreezeError::HorizontalDerivativeNotProvenNonZero);
        }
        let (left, right) = split_regularity_node(node)?;
        debug_assert!(stack_len + 2 <= REGULARITY_STACK_CAPACITY);
        stack[stack_len] = Some(right);
        stack[stack_len + 1] = Some(left);
        stack_len += 2;
    }
    Ok(visits)
}

fn hull_proves_nonzero(controls: [IntervalPoint2; 3]) -> bool {
    let x_low = controls
        .iter()
        .fold(f64::INFINITY, |value, control| value.min(control.x.low));
    let x_high = controls.iter().fold(f64::NEG_INFINITY, |value, control| {
        value.max(control.x.high)
    });
    let z_low = controls
        .iter()
        .fold(f64::INFINITY, |value, control| value.min(control.z.low));
    let z_high = controls.iter().fold(f64::NEG_INFINITY, |value, control| {
        value.max(control.z.high)
    });
    x_low > 0.0 || x_high < 0.0 || z_low > 0.0 || z_high < 0.0
}

fn split_regularity_node(
    node: RegularityNode,
) -> Result<(RegularityNode, RegularityNode), NumericFreezeError> {
    let q0 = node.controls[0];
    let q1 = node.controls[1];
    let q2 = node.controls[2];
    let q01 = q0.midpoint_lerp(q1)?;
    let q12 = q1.midpoint_lerp(q2)?;
    let q012 = q01.midpoint_lerp(q12)?;
    let child_depth = node.depth + 1;
    Ok((
        RegularityNode {
            controls: [q0, q01, q012],
            depth: child_depth,
        },
        RegularityNode {
            controls: [q012, q12, q2],
            depth: child_depth,
        },
    ))
}

fn finite(value: f64) -> Result<f64, NumericFreezeError> {
    if !value.is_finite() {
        return Err(NumericFreezeError::NonFinite);
    }
    Ok(if value == 0.0 { 0.0 } else { value })
}

fn next_up(value: f64) -> Result<f64, NumericFreezeError> {
    let value = finite(value)?;
    let next = if value == 0.0 {
        f64::from_bits(1)
    } else if value > 0.0 {
        f64::from_bits(value.to_bits() + 1)
    } else {
        f64::from_bits(value.to_bits() - 1)
    };
    finite(next)
}

fn next_down(value: f64) -> Result<f64, NumericFreezeError> {
    let value = finite(value)?;
    let next = if value == 0.0 {
        f64::from_bits((1_u64 << 63) | 1)
    } else if value > 0.0 {
        f64::from_bits(value.to_bits() - 1)
    } else {
        f64::from_bits(value.to_bits() + 1)
    };
    finite(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f64, y: f64, z: f64) -> Point3 {
        Point3::try_new(x, y, z).unwrap()
    }

    #[test]
    fn line_and_cubic_use_the_frozen_de_casteljau_dual_graph() {
        let line = CurveSegment::Line {
            start: point(0.0, 1.0, 0.0),
            end: point(8.0, 5.0, 4.0),
        };
        let line_sample = line.evaluate(0.25).unwrap();
        assert_eq!(line_sample.point, point(2.0, 2.0, 1.0));
        assert_eq!(line_sample.first, point(8.0, 4.0, 4.0));

        let cubic = CurveSegment::CubicBezier {
            start: point(0.0, 0.0, 0.0),
            control_1: point(1.0, 2.0, 0.0),
            control_2: point(2.0, 4.0, 0.0),
            end: point(3.0, 6.0, 0.0),
        };
        let cubic_sample = cubic.evaluate(0.5).unwrap();
        assert_eq!(cubic_sample.point, point(1.5, 3.0, 0.0));
        assert_eq!(cubic_sample.first, point(3.0, 6.0, 0.0));
    }

    #[test]
    fn offset_evaluator_uses_station_width_and_left_duals_in_one_graph() {
        let line = CurveSegment::Line {
            start: point(0.0, 0.0, 0.0),
            end: point(8.0, 0.0, 0.0),
        };
        let sample = line
            .evaluate_offset(
                0.5,
                StationInterval {
                    parameter_start: 0.0,
                    parameter_end: 1.0,
                    cumulative_start_meters: 0.0,
                    cumulative_end_meters: 8.0,
                },
                OffsetInterval {
                    station_start_meters: 0.0,
                    station_end_meters: 8.0,
                    offset_start_meters: 2.0,
                    offset_end_meters: 4.0,
                },
            )
            .unwrap();

        assert_eq!(sample.point, point(4.0, 0.0, -3.0));
        assert_eq!(sample.first, point(8.0, 0.0, -2.0));
    }

    #[test]
    fn companion_cubic_regularity_walk_has_exactly_three_visits() {
        let segment = CurveSegment::CubicBezier {
            start: point(0.0, 0.0, 0.0),
            control_1: point(20.0, 0.0, 20.0),
            control_2: point(20.0, 0.0, 0.0),
            end: point(189.5, 0.0, 0.0),
        };

        assert_eq!(segment.prove_horizontal_regularity().unwrap(), 3);
    }

    #[test]
    fn horizontal_zero_and_unproven_cusp_fail_closed() {
        let vertical = CurveSegment::Line {
            start: point(0.0, 0.0, 0.0),
            end: point(0.0, 1.0, 0.0),
        };
        assert_eq!(
            vertical.prove_horizontal_regularity(),
            Err(NumericFreezeError::HorizontalDerivativeZero)
        );

        let cusp = CurveSegment::CubicBezier {
            start: point(0.0, 0.0, 0.0),
            control_1: point(-1.0 / 3.0, 0.0, 0.0),
            control_2: point(-1.0 / 3.0, 0.0, 0.0),
            end: point(0.0, 0.0, 0.0),
        };
        assert_eq!(
            cusp.prove_horizontal_regularity(),
            Err(NumericFreezeError::HorizontalDerivativeNotProvenNonZero)
        );
    }

    #[test]
    fn directed_adjacent_values_handle_signed_zero_and_extremes() {
        assert_eq!(next_up(-0.0).unwrap().to_bits(), 1);
        assert_eq!(next_down(0.0).unwrap().to_bits(), (1_u64 << 63) | 1);
        assert_eq!(next_up(-1.0).unwrap().to_bits(), (-1.0_f64).to_bits() - 1);
        assert_eq!(next_down(1.0).unwrap().to_bits(), 1.0_f64.to_bits() - 1);
        assert_eq!(next_up(f64::MAX), Err(NumericFreezeError::NonFinite));
        assert_eq!(next_down(-f64::MAX), Err(NumericFreezeError::NonFinite));
    }
}

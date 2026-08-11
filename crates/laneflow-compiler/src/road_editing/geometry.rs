//! ADR 0022 B1 的确定性曲线数值内核。
//!
//! 本模块只实现固定 scalar-dual 运算图、line/cubic evaluator 和 offset 前置的
//! horizontal-regularity walk，以及不拥有容器的自适应点表细分。station 强制边界和
//! 共同 Typed AST 组装由后续切片组合，避免在数值内核中建立第二套拓扑或资源权威。

use laneflow_static_contract::{
    CANONICAL_POINT_COMPONENT_MAX_METERS, CANONICAL_POINT_COMPONENT_MIN_METERS,
};

use crate::declaration::CanonicalPoint3F32Input;
use crate::{GeometryAccuracyProfile, GeometryDirectionProfile};

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
    CoordinateOutOfRange,
    ApproximationNotConverged,
    GeometryPointLimit,
    StationRowLimit,
    StationOutOfRange,
    GeometryTopologyMismatch,
    SourceJoinGapExceeded,
    DegenerateCanonicalSegment,
    DirectionDiscontinuity,
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

pub(super) type ApproximationPoint = CanonicalPoint3F32Input;

pub(super) fn quantize_point(value: Point3) -> Result<ApproximationPoint, NumericFreezeError> {
    let minimum = f64::from(CANONICAL_POINT_COMPONENT_MIN_METERS);
    let maximum = f64::from(CANONICAL_POINT_COMPONENT_MAX_METERS);
    if [value.x, value.y, value.z]
        .into_iter()
        .any(|component| !(minimum..=maximum).contains(&component))
    {
        return Err(NumericFreezeError::CoordinateOutOfRange);
    }
    Ok(ApproximationPoint {
        x: canonical_f32(value.x as f32),
        y: canonical_f32(value.y as f32),
        z: canonical_f32(value.z as f32),
    })
}

fn promote_point(value: ApproximationPoint) -> Point3 {
    Point3 {
        x: f64::from(value.x),
        y: f64::from(value.y),
        z: f64::from(value.z),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ApproximationVertex {
    pub(super) parameter: f64,
    pub(super) point: ApproximationPoint,
}

pub(super) trait ApproximationPointSink {
    fn push(&mut self, vertex: ApproximationVertex) -> Result<(), NumericFreezeError>;
}

#[derive(Clone, Copy)]
pub(super) enum SegmentEvaluator {
    Reference(CurveSegment),
    Offset {
        segment: CurveSegment,
        station: StationInterval,
        offset: OffsetInterval,
    },
}

impl SegmentEvaluator {
    pub(super) fn evaluate(self, parameter: f64) -> Result<CurveSample, NumericFreezeError> {
        match self {
            Self::Reference(segment) => segment.evaluate(parameter),
            Self::Offset {
                segment,
                station,
                offset,
            } => segment.evaluate_offset(parameter, station, offset),
        }
    }
}

#[derive(Clone, Copy)]
struct CandidateEndpoint {
    point: ApproximationPoint,
    first: Point3,
}

impl CandidateEndpoint {
    fn evaluate(evaluator: SegmentEvaluator, parameter: f64) -> Result<Self, NumericFreezeError> {
        let sample = evaluator.evaluate(parameter)?;
        Ok(Self {
            point: quantize_point(sample.point)?,
            first: sample.first,
        })
    }
}

#[derive(Clone, Copy)]
struct ApproximationNode {
    parameter_start: f64,
    parameter_end: f64,
    start: CandidateEndpoint,
    end: CandidateEndpoint,
    depth: u8,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ApproximationInterval {
    pub(super) parameter_start: f64,
    pub(super) parameter_end: f64,
    pub(super) welded_start: Option<ApproximationPoint>,
    pub(super) emit_start: bool,
}

pub(super) fn approximate_interval(
    evaluator: SegmentEvaluator,
    interval: ApproximationInterval,
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
    sink: &mut impl ApproximationPointSink,
) -> Result<(), NumericFreezeError> {
    let ApproximationInterval {
        parameter_start,
        parameter_end,
        welded_start,
        emit_start,
    } = interval;
    if !parameter_start.is_finite()
        || !parameter_end.is_finite()
        || parameter_start >= parameter_end
    {
        return Err(NumericFreezeError::ApproximationNotConverged);
    }
    let mut start = CandidateEndpoint::evaluate(evaluator, parameter_start)?;
    if let Some(welded_start) = welded_start {
        start.point = welded_start;
    }
    let end = CandidateEndpoint::evaluate(evaluator, parameter_end)?;
    if emit_start {
        sink.push(ApproximationVertex {
            parameter: parameter_start,
            point: start.point,
        })?;
    }

    let mut stack = [None; REGULARITY_STACK_CAPACITY];
    stack[0] = Some(ApproximationNode {
        parameter_start,
        parameter_end,
        start,
        end,
        depth: 0,
    });
    let mut stack_len = 1_usize;
    while stack_len != 0 {
        stack_len -= 1;
        let node = stack[stack_len]
            .take()
            .expect("fixed approximation stack contains every live node");
        if candidate_accepts(evaluator, node, accuracy, direction)? {
            sink.push(ApproximationVertex {
                parameter: node.parameter_end,
                point: node.end.point,
            })?;
            continue;
        }
        if node.depth == MAX_SUBDIVISION_DEPTH {
            return Err(NumericFreezeError::ApproximationNotConverged);
        }
        let parameter_mid =
            finite(node.parameter_start + (node.parameter_end - node.parameter_start) / 2.0)?;
        if parameter_mid <= node.parameter_start || parameter_mid >= node.parameter_end {
            return Err(NumericFreezeError::ApproximationNotConverged);
        }
        let midpoint = CandidateEndpoint::evaluate(evaluator, parameter_mid)?;
        let child_depth = node.depth + 1;
        debug_assert!(stack_len + 2 <= REGULARITY_STACK_CAPACITY);
        stack[stack_len] = Some(ApproximationNode {
            parameter_start: parameter_mid,
            parameter_end: node.parameter_end,
            start: midpoint,
            end: node.end,
            depth: child_depth,
        });
        stack[stack_len + 1] = Some(ApproximationNode {
            parameter_start: node.parameter_start,
            parameter_end: parameter_mid,
            start: node.start,
            end: midpoint,
            depth: child_depth,
        });
        stack_len += 2;
    }
    Ok(())
}

fn candidate_accepts(
    evaluator: SegmentEvaluator,
    node: ApproximationNode,
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
) -> Result<bool, NumericFreezeError> {
    let parameter_mid =
        finite(node.parameter_start + (node.parameter_end - node.parameter_start) / 2.0)?;
    let parameter_quarter_1 =
        finite(node.parameter_start + (parameter_mid - node.parameter_start) / 2.0)?;
    let parameter_quarter_3 = finite(parameter_mid + (node.parameter_end - parameter_mid) / 2.0)?;
    if parameter_quarter_1 <= node.parameter_start
        || parameter_mid <= parameter_quarter_1
        || parameter_quarter_3 <= parameter_mid
        || parameter_quarter_3 >= node.parameter_end
    {
        return Err(NumericFreezeError::ApproximationNotConverged);
    }

    let start = promote_point(node.start.point);
    let end = promote_point(node.end.point);
    let chord = point_sub(end, start)?;
    let target = position_target(accuracy);
    let target_squared = finite(target * target)?;
    for (parameter, chord_parameter) in [
        (parameter_quarter_1, 0.25_f64),
        (parameter_mid, 0.5),
        (parameter_quarter_3, 0.75),
    ] {
        let sample = evaluator.evaluate(parameter)?;
        let chord_point = point_lerp(start, end, chord_parameter)?;
        let delta = point_sub(sample.point, chord_point)?;
        if norm_squared(delta)? > target_squared {
            return Ok(false);
        }
    }

    let cosine_squared = half_angle_cosine_squared(direction);
    Ok(direction_accepts(node.start.first, chord, cosine_squared)?
        && direction_accepts(chord, node.end.first, cosine_squared)?)
}

fn position_target(profile: GeometryAccuracyProfile) -> f64 {
    f64::from_bits(match profile {
        GeometryAccuracyProfile::Fine2Cm => 0x3f84_7ae1_47ae_147b,
        GeometryAccuracyProfile::Balanced5Cm => 0x3f99_9999_9999_999a,
        GeometryAccuracyProfile::Compact10Cm => 0x3fa9_9999_9999_999a,
    })
}

fn half_angle_cosine_squared(profile: GeometryDirectionProfile) -> f64 {
    f64::from_bits(match profile {
        GeometryDirectionProfile::Smooth1Deg => 0x3fef_ff60_4bfa_d7c5,
        GeometryDirectionProfile::Balanced2Deg => 0x3fef_fd81_3c5f_82b4,
        GeometryDirectionProfile::Compact5Deg => 0x3fef_f069_da0c_0ad2,
    })
}

pub(super) fn full_angle_cosine_squared(profile: GeometryDirectionProfile) -> f64 {
    f64::from_bits(match profile {
        GeometryDirectionProfile::Smooth1Deg => 0x3fef_fd81_3c5f_82b4,
        GeometryDirectionProfile::Balanced2Deg => 0x3fef_f605_b8b8_7ffc,
        GeometryDirectionProfile::Compact5Deg => 0x3fef_c1c5_c640_8e0c,
    })
}

pub(super) fn direction_accepts(
    left: Point3,
    right: Point3,
    cosine_squared: f64,
) -> Result<bool, NumericFreezeError> {
    let dot = dot(left, right)?;
    let left_norm = norm_squared(left)?;
    let right_norm = norm_squared(right)?;
    let lhs = finite(dot * dot)?;
    let weighted_left_norm = finite(cosine_squared * left_norm)?;
    let rhs = finite(weighted_left_norm * right_norm)?;
    Ok(dot > 0.0 && lhs >= rhs)
}

pub(super) fn validate_canonical_polyline(
    points: &[ApproximationPoint],
    direction: GeometryDirectionProfile,
) -> Result<f64, NumericFreezeError> {
    if points.len() < 2 {
        return Err(NumericFreezeError::DegenerateCanonicalSegment);
    }
    let cosine_squared = full_angle_cosine_squared(direction);
    let mut previous_chord = None;
    let mut cumulative_length = 0.0_f64;
    for pair in points.windows(2) {
        let chord = point_sub(promote_point(pair[1]), promote_point(pair[0]))?;
        let chord_norm_squared = norm_squared(chord)?;
        if chord_norm_squared == 0.0 {
            return Err(NumericFreezeError::DegenerateCanonicalSegment);
        }
        if let Some(previous_chord) = previous_chord
            && !direction_accepts(previous_chord, chord, cosine_squared)?
        {
            return Err(NumericFreezeError::DirectionDiscontinuity);
        }
        let chord_length = finite(chord_norm_squared.sqrt())?;
        cumulative_length = finite(cumulative_length + chord_length)?;
        previous_chord = Some(chord);
    }
    Ok(cumulative_length)
}

fn dot(left: Point3, right: Point3) -> Result<f64, NumericFreezeError> {
    let x = finite(left.x * right.x)?;
    let y = finite(left.y * right.y)?;
    let xy = finite(x + y)?;
    let z = finite(left.z * right.z)?;
    finite(xy + z)
}

fn norm_squared(value: Point3) -> Result<f64, NumericFreezeError> {
    let x = finite(value.x * value.x)?;
    let y = finite(value.y * value.y)?;
    let xy = finite(x + y)?;
    let z = finite(value.z * value.z)?;
    finite(xy + z)
}

fn point_sub(left: Point3, right: Point3) -> Result<Point3, NumericFreezeError> {
    Point3::try_new(
        finite(left.x - right.x)?,
        finite(left.y - right.y)?,
        finite(left.z - right.z)?,
    )
}

pub(super) fn point_distance(left: Point3, right: Point3) -> Result<f64, NumericFreezeError> {
    let delta = point_sub(left, right)?;
    finite(norm_squared(delta)?.sqrt())
}

pub(super) fn canonical_point_distance(
    left: ApproximationPoint,
    right: ApproximationPoint,
) -> Result<f64, NumericFreezeError> {
    point_distance(promote_point(left), promote_point(right))
}

fn point_lerp(start: Point3, end: Point3, parameter: f64) -> Result<Point3, NumericFreezeError> {
    fn component(start: f64, end: f64, parameter: f64) -> Result<f64, NumericFreezeError> {
        let delta = finite(end - start)?;
        let scaled = finite(parameter * delta)?;
        finite(start + scaled)
    }
    Point3::try_new(
        component(start.x, end.x, parameter)?,
        component(start.y, end.y, parameter)?,
        component(start.z, end.z, parameter)?,
    )
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

fn canonical_f32(value: f32) -> f32 {
    if value == 0.0 { 0.0 } else { value }
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

    #[derive(Default)]
    struct TestPointSink {
        vertices: Vec<ApproximationVertex>,
        maximum_points: Option<usize>,
    }

    impl ApproximationPointSink for TestPointSink {
        fn push(&mut self, vertex: ApproximationVertex) -> Result<(), NumericFreezeError> {
            if self.maximum_points == Some(self.vertices.len()) {
                return Err(NumericFreezeError::GeometryPointLimit);
            }
            self.vertices.push(vertex);
            Ok(())
        }
    }

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
    fn adaptive_profile_constants_keep_the_frozen_bits() {
        assert_eq!(
            position_target(GeometryAccuracyProfile::Fine2Cm).to_bits(),
            0x3f84_7ae1_47ae_147b
        );
        assert_eq!(
            position_target(GeometryAccuracyProfile::Balanced5Cm).to_bits(),
            0x3f99_9999_9999_999a
        );
        assert_eq!(
            position_target(GeometryAccuracyProfile::Compact10Cm).to_bits(),
            0x3fa9_9999_9999_999a
        );
        assert_eq!(
            half_angle_cosine_squared(GeometryDirectionProfile::Smooth1Deg).to_bits(),
            0x3fef_ff60_4bfa_d7c5
        );
        assert_eq!(
            full_angle_cosine_squared(GeometryDirectionProfile::Compact5Deg).to_bits(),
            0x3fef_c1c5_c640_8e0c
        );
    }

    #[test]
    fn straight_reference_emits_only_the_two_quantized_endpoints_for_every_profile() {
        let evaluator = SegmentEvaluator::Reference(CurveSegment::Line {
            start: point(-0.0, 1.25, 0.0),
            end: point(8.0, 1.25, 4.0),
        });
        for accuracy in [
            GeometryAccuracyProfile::Fine2Cm,
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryAccuracyProfile::Compact10Cm,
        ] {
            for direction in [
                GeometryDirectionProfile::Smooth1Deg,
                GeometryDirectionProfile::Balanced2Deg,
                GeometryDirectionProfile::Compact5Deg,
            ] {
                let mut sink = TestPointSink::default();
                approximate_interval(
                    evaluator,
                    ApproximationInterval {
                        parameter_start: 0.0,
                        parameter_end: 1.0,
                        welded_start: None,
                        emit_start: true,
                    },
                    accuracy,
                    direction,
                    &mut sink,
                )
                .unwrap();
                assert_eq!(
                    sink.vertices,
                    [
                        ApproximationVertex {
                            parameter: 0.0,
                            point: ApproximationPoint {
                                x: 0.0,
                                y: 1.25,
                                z: 0.0,
                            },
                        },
                        ApproximationVertex {
                            parameter: 1.0,
                            point: ApproximationPoint {
                                x: 8.0,
                                y: 1.25,
                                z: 4.0,
                            },
                        },
                    ]
                );
                assert_eq!(sink.vertices[0].point.x.to_bits(), 0);
            }
        }
    }

    #[test]
    fn adjacent_forced_intervals_emit_the_shared_parameter_once() {
        let evaluator = SegmentEvaluator::Reference(CurveSegment::Line {
            start: point(0.0, 0.0, 0.0),
            end: point(8.0, 0.0, 0.0),
        });
        let mut sink = TestPointSink::default();
        approximate_interval(
            evaluator,
            ApproximationInterval {
                parameter_start: 0.0,
                parameter_end: 0.5,
                welded_start: None,
                emit_start: true,
            },
            GeometryAccuracyProfile::Fine2Cm,
            GeometryDirectionProfile::Smooth1Deg,
            &mut sink,
        )
        .unwrap();
        approximate_interval(
            evaluator,
            ApproximationInterval {
                parameter_start: 0.5,
                parameter_end: 1.0,
                welded_start: None,
                emit_start: false,
            },
            GeometryAccuracyProfile::Fine2Cm,
            GeometryDirectionProfile::Smooth1Deg,
            &mut sink,
        )
        .unwrap();

        assert_eq!(
            sink.vertices
                .iter()
                .map(|vertex| (vertex.parameter, vertex.point.x))
                .collect::<Vec<_>>(),
            [(0.0, 0.0), (0.5, 4.0), (1.0, 8.0)]
        );
    }

    #[test]
    fn canonical_polyline_validation_freezes_length_degeneracy_and_full_angle() {
        let straight = [
            ApproximationPoint {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            ApproximationPoint {
                x: 3.0,
                y: 0.0,
                z: 4.0,
            },
        ];
        assert_eq!(
            validate_canonical_polyline(&straight, GeometryDirectionProfile::Smooth1Deg).unwrap(),
            5.0
        );

        let duplicate = [straight[0], straight[0]];
        assert_eq!(
            validate_canonical_polyline(&duplicate, GeometryDirectionProfile::Compact5Deg),
            Err(NumericFreezeError::DegenerateCanonicalSegment)
        );

        let direction_jump = [
            ApproximationPoint {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            ApproximationPoint {
                x: 100.0,
                y: 0.0,
                z: 0.0,
            },
            ApproximationPoint {
                x: 200.0,
                y: 0.0,
                z: 3.5,
            },
        ];
        assert_eq!(
            validate_canonical_polyline(&direction_jump, GeometryDirectionProfile::Smooth1Deg),
            Err(NumericFreezeError::DirectionDiscontinuity)
        );
        assert!(
            validate_canonical_polyline(&direction_jump, GeometryDirectionProfile::Compact5Deg)
                .is_ok()
        );
    }

    #[test]
    fn companion_cubic_subdivides_under_the_strict_profiles() {
        let evaluator = SegmentEvaluator::Reference(CurveSegment::CubicBezier {
            start: point(0.0, 0.0, 0.0),
            control_1: point(20.0, 0.0, 20.0),
            control_2: point(20.0, 0.0, 0.0),
            end: point(189.5, 0.0, 0.0),
        });
        let mut sink = TestPointSink::default();
        approximate_interval(
            evaluator,
            ApproximationInterval {
                parameter_start: 0.0,
                parameter_end: 1.0,
                welded_start: None,
                emit_start: true,
            },
            GeometryAccuracyProfile::Fine2Cm,
            GeometryDirectionProfile::Smooth1Deg,
            &mut sink,
        )
        .unwrap();

        assert_eq!(sink.vertices.len(), 154);
        assert_eq!(
            sink.vertices.first(),
            Some(&ApproximationVertex {
                parameter: 0.0,
                point: ApproximationPoint {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
            })
        );
        assert_eq!(
            sink.vertices.last(),
            Some(&ApproximationVertex {
                parameter: 1.0,
                point: ApproximationPoint {
                    x: 189.5,
                    y: 0.0,
                    z: 0.0,
                },
            })
        );
    }

    #[test]
    fn welded_start_and_sink_point_limit_fail_closed_without_hidden_allocation() {
        let evaluator = SegmentEvaluator::Reference(CurveSegment::Line {
            start: point(0.0, 0.0, 0.0),
            end: point(8.0, 0.0, 0.0),
        });
        let welded = ApproximationPoint {
            x: 0.003,
            y: 0.0,
            z: 0.0,
        };
        let mut sink = TestPointSink {
            vertices: Vec::new(),
            maximum_points: Some(1),
        };
        assert_eq!(
            approximate_interval(
                evaluator,
                ApproximationInterval {
                    parameter_start: 0.0,
                    parameter_end: 1.0,
                    welded_start: Some(welded),
                    emit_start: true,
                },
                GeometryAccuracyProfile::Fine2Cm,
                GeometryDirectionProfile::Smooth1Deg,
                &mut sink,
            ),
            Err(NumericFreezeError::GeometryPointLimit)
        );
        assert_eq!(
            sink.vertices,
            [ApproximationVertex {
                parameter: 0.0,
                point: welded,
            }]
        );
    }

    #[test]
    fn endpoint_quantization_rejects_coordinates_outside_the_canonical_domain() {
        let evaluator = SegmentEvaluator::Reference(CurveSegment::Line {
            start: point(0.0, 0.0, 0.0),
            end: point(
                f64::from(CANONICAL_POINT_COMPONENT_MAX_METERS) + 1.0,
                0.0,
                0.0,
            ),
        });
        let mut sink = TestPointSink::default();
        assert_eq!(
            approximate_interval(
                evaluator,
                ApproximationInterval {
                    parameter_start: 0.0,
                    parameter_end: 1.0,
                    welded_start: None,
                    emit_start: true,
                },
                GeometryAccuracyProfile::Fine2Cm,
                GeometryDirectionProfile::Smooth1Deg,
                &mut sink,
            ),
            Err(NumericFreezeError::CoordinateOutOfRange)
        );
        assert!(sink.vertices.is_empty());
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

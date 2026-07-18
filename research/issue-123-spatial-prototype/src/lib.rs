//! #123 的 research-only engine-neutral spatial prototype。

/// 相邻 polyline 顶点之间允许的最小距离，单位为米。
pub const MIN_SEGMENT_LENGTH_METERS: f64 = 1.0e-4;

/// Core edge length 与 geometry arc length 的绝对一致性 floor，单位为米。
pub const LENGTH_ABSOLUTE_TOLERANCE_METERS: f64 = 1.0e-6;

/// Core edge length 与 geometry arc length 的相对一致性比例。
pub const LENGTH_RELATIVE_TOLERANCE: f64 = 1.0e-9;

/// 相连 edge 端点的最大位置差，单位为米。
pub const JOIN_POSITION_TOLERANCE_METERS: f64 = 1.0e-3;

/// tangent 与 canonical up 过度接近平行时使用的无量纲下限。
pub const BASIS_MIN_PROJECTED_UP_LENGTH: f64 = 1.0e-6;

/// 已构造 canonical pose 的 unit/orthogonal basis 复核阈值。
pub const BASIS_ORTHONORMAL_TOLERANCE: f64 = 1.0e-12;

/// 当前 Core edge boundary 的研究对照值；正式职责由 #125 拆分。
pub const CURRENT_CORE_BOUNDARY_TOLERANCE_METERS: f64 = 1.0e-9;

/// #122 首轮 local presentation envelope，单位为米。
pub const RESEARCH_LOCAL_ENVELOPE_METERS: f64 = 16_384.0;

/// LaneFlow-owned canonical point，单位为米。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalPoint3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl CanonicalPoint3 {
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    pub fn distance(self, other: Self) -> f64 {
        (other - self).length()
    }
}

/// LaneFlow-owned canonical vector。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalVector3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl CanonicalVector3 {
    pub const Y: Self = Self::new(0.0, 1.0, 0.0);

    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub fn dot(self, other: Self) -> f64 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    pub fn length(self) -> f64 {
        self.x.hypot(self.y).hypot(self.z)
    }

    pub fn try_normalize(self) -> Option<Self> {
        let length = self.length();
        if !length.is_finite() || length == 0.0 {
            return None;
        }

        Some(self / length)
    }
}

impl std::ops::Add<CanonicalVector3> for CanonicalPoint3 {
    type Output = Self;

    fn add(self, vector: CanonicalVector3) -> Self::Output {
        Self::new(self.x + vector.x, self.y + vector.y, self.z + vector.z)
    }
}

impl std::ops::Sub for CanonicalPoint3 {
    type Output = CanonicalVector3;

    fn sub(self, other: Self) -> Self::Output {
        CanonicalVector3::new(self.x - other.x, self.y - other.y, self.z - other.z)
    }
}

impl std::ops::Add for CanonicalVector3 {
    type Output = Self;

    fn add(self, other: Self) -> Self::Output {
        Self::new(self.x + other.x, self.y + other.y, self.z + other.z)
    }
}

impl std::ops::Sub for CanonicalVector3 {
    type Output = Self;

    fn sub(self, other: Self) -> Self::Output {
        Self::new(self.x - other.x, self.y - other.y, self.z - other.z)
    }
}

impl std::ops::Mul<f64> for CanonicalVector3 {
    type Output = Self;

    fn mul(self, scalar: f64) -> Self::Output {
        Self::new(self.x * scalar, self.y * scalar, self.z * scalar)
    }
}

impl std::ops::Div<f64> for CanonicalVector3 {
    type Output = Self;

    fn div(self, scalar: f64) -> Self::Output {
        Self::new(self.x / scalar, self.y / scalar, self.z / scalar)
    }
}

/// 由 canonical polyline 采样得到的 LaneFlow-owned pose。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalPose {
    pub position: CanonicalPoint3,
    pub tangent: CanonicalVector3,
    pub up: CanonicalVector3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Segment {
    length: f64,
    tangent: CanonicalVector3,
    up: CanonicalVector3,
}

/// 已验证并预计算累计弧长的 canonical polyline。
#[derive(Clone, Debug, PartialEq)]
pub struct Polyline {
    points: Vec<CanonicalPoint3>,
    segments: Vec<Segment>,
    cumulative_lengths: Vec<f64>,
}

impl Polyline {
    pub fn try_new(points: Vec<CanonicalPoint3>) -> Result<Self, PolylineError> {
        if points.len() < 2 {
            return Err(PolylineError::TooFewPoints {
                actual: points.len(),
            });
        }

        for (index, point) in points.iter().copied().enumerate() {
            if !point.is_finite() {
                return Err(PolylineError::NonFinitePoint { index });
            }
        }

        let mut segments = Vec::with_capacity(points.len() - 1);
        let mut cumulative_lengths = Vec::with_capacity(points.len());
        cumulative_lengths.push(0.0);

        for index in 0..points.len() - 1 {
            let delta = points[index + 1] - points[index];
            let length = delta.length();
            if !length.is_finite() || length <= MIN_SEGMENT_LENGTH_METERS {
                return Err(PolylineError::DegenerateSegment { index, length });
            }

            let tangent = delta / length;
            let projected_up = CanonicalVector3::Y - tangent * CanonicalVector3::Y.dot(tangent);
            let projected_up_length = projected_up.length();
            if projected_up_length <= BASIS_MIN_PROJECTED_UP_LENGTH {
                return Err(PolylineError::DegenerateBasis {
                    index,
                    projected_up_length,
                });
            }

            let up = projected_up / projected_up_length;
            segments.push(Segment {
                length,
                tangent,
                up,
            });
            cumulative_lengths.push(cumulative_lengths[index] + length);
        }

        Ok(Self {
            points,
            segments,
            cumulative_lengths,
        })
    }

    pub fn arc_length(&self) -> f64 {
        *self
            .cumulative_lengths
            .last()
            .expect("validated polyline must contain cumulative lengths")
    }

    pub fn start(&self) -> CanonicalPoint3 {
        self.points[0]
    }

    pub fn end(&self) -> CanonicalPoint3 {
        *self
            .points
            .last()
            .expect("validated polyline must contain points")
    }

    pub fn sample_arc_length(&self, arc_length: f64) -> Result<CanonicalPose, SampleError> {
        let total = self.arc_length();
        if !arc_length.is_finite() || arc_length < 0.0 || arc_length > total {
            return Err(SampleError::ArcLengthOutOfRange { arc_length, total });
        }

        let insertion = self
            .cumulative_lengths
            .partition_point(|candidate| *candidate <= arc_length);
        let segment_index = insertion.saturating_sub(1).min(self.segments.len() - 1);
        let segment = self.segments[segment_index];
        let segment_start = self.cumulative_lengths[segment_index];
        let ratio = ((arc_length - segment_start) / segment.length).clamp(0.0, 1.0);
        let position = self.points[segment_index]
            + (self.points[segment_index + 1] - self.points[segment_index]) * ratio;

        Ok(CanonicalPose {
            position,
            tangent: segment.tangent,
            up: segment.up,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PolylineError {
    TooFewPoints {
        actual: usize,
    },
    NonFinitePoint {
        index: usize,
    },
    DegenerateSegment {
        index: usize,
        length: f64,
    },
    DegenerateBasis {
        index: usize,
        projected_up_length: f64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SampleError {
    ArcLengthOutOfRange { arc_length: f64, total: f64 },
    ProgressOutOfRange { progress: f64, core_length: f64 },
}

/// Core edge length 与 geometry arc length 的 immutable binding。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LengthBinding {
    core_length: f64,
    geometry_arc_length: f64,
}

impl LengthBinding {
    pub fn try_new(core_length: f64, polyline: &Polyline) -> Result<Self, BindingError> {
        let geometry_arc_length = polyline.arc_length();
        if !core_length.is_finite() || core_length <= 0.0 {
            return Err(BindingError::InvalidCoreLength { core_length });
        }
        if !geometry_arc_length.is_finite() || geometry_arc_length <= 0.0 {
            return Err(BindingError::InvalidGeometryLength {
                geometry_arc_length,
            });
        }

        let difference = (core_length - geometry_arc_length).abs();
        let tolerance = length_consistency_tolerance(core_length, geometry_arc_length);
        if difference > tolerance {
            return Err(BindingError::LengthMismatch {
                core_length,
                geometry_arc_length,
                difference,
                tolerance,
            });
        }

        Ok(Self {
            core_length,
            geometry_arc_length,
        })
    }

    pub fn sample_progress(
        self,
        polyline: &Polyline,
        progress: f64,
    ) -> Result<CanonicalPose, SampleError> {
        if !progress.is_finite()
            || progress < 0.0
            || progress > self.core_length + CURRENT_CORE_BOUNDARY_TOLERANCE_METERS
        {
            return Err(SampleError::ProgressOutOfRange {
                progress,
                core_length: self.core_length,
            });
        }

        let snapped_progress = progress.min(self.core_length);
        let ratio = snapped_progress / self.core_length;
        polyline.sample_arc_length(self.geometry_arc_length * ratio)
    }
}

pub fn length_consistency_tolerance(core_length: f64, geometry_arc_length: f64) -> f64 {
    LENGTH_ABSOLUTE_TOLERANCE_METERS
        .max(LENGTH_RELATIVE_TOLERANCE * core_length.abs().max(geometry_arc_length.abs()))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BindingError {
    InvalidCoreLength {
        core_length: f64,
    },
    InvalidGeometryLength {
        geometry_arc_length: f64,
    },
    LengthMismatch {
        core_length: f64,
        geometry_arc_length: f64,
        difference: f64,
        tolerance: f64,
    },
}

pub fn validate_join(from: &Polyline, to: &Polyline) -> Result<(), JoinError> {
    let distance = from.end().distance(to.start());
    if distance > JOIN_POSITION_TOLERANCE_METERS {
        return Err(JoinError {
            distance,
            tolerance: JOIN_POSITION_TOLERANCE_METERS,
        });
    }

    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JoinError {
    pub distance: f64,
    pub tolerance: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalPoint3F32 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalVector3F32 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalPoseF32 {
    pub position: LocalPoint3F32,
    pub tangent: LocalVector3F32,
    pub up: LocalVector3F32,
}

pub fn try_to_local_f32(
    pose: CanonicalPose,
    origin: CanonicalPoint3,
    envelope: f64,
) -> Result<LocalPoseF32, LocalPoseError> {
    if !origin.is_finite() {
        return Err(LocalPoseError::InvalidOrigin);
    }
    if !envelope.is_finite() || envelope <= 0.0 {
        return Err(LocalPoseError::InvalidEnvelope { envelope });
    }
    if !basis_is_orthonormal(pose.tangent, pose.up) {
        return Err(LocalPoseError::InvalidBasis);
    }

    let local = pose.position - origin;
    let position = LocalPoint3F32 {
        x: checked_local_component(local.x, envelope, Axis::X)?,
        y: checked_local_component(local.y, envelope, Axis::Y)?,
        z: checked_local_component(local.z, envelope, Axis::Z)?,
    };

    Ok(LocalPoseF32 {
        position,
        tangent: to_f32_vector(pose.tangent)?,
        up: to_f32_vector(pose.up)?,
    })
}

fn basis_is_orthonormal(tangent: CanonicalVector3, up: CanonicalVector3) -> bool {
    tangent.is_finite()
        && up.is_finite()
        && (tangent.length() - 1.0).abs() <= BASIS_ORTHONORMAL_TOLERANCE
        && (up.length() - 1.0).abs() <= BASIS_ORTHONORMAL_TOLERANCE
        && tangent.dot(up).abs() <= BASIS_ORTHONORMAL_TOLERANCE
}

fn checked_local_component(value: f64, envelope: f64, axis: Axis) -> Result<f32, LocalPoseError> {
    if !value.is_finite() || value.abs() > envelope {
        return Err(LocalPoseError::PositionOutsideEnvelope {
            axis,
            value,
            envelope,
        });
    }

    let converted = value as f32;
    if !converted.is_finite() {
        return Err(LocalPoseError::F32ConversionFailed { axis, value });
    }

    Ok(canonicalize_f32_zero(converted))
}

fn to_f32_vector(vector: CanonicalVector3) -> Result<LocalVector3F32, LocalPoseError> {
    let values = [vector.x as f32, vector.y as f32, vector.z as f32];
    if values.iter().any(|value| !value.is_finite()) {
        return Err(LocalPoseError::InvalidBasis);
    }

    Ok(LocalVector3F32 {
        x: canonicalize_f32_zero(values[0]),
        y: canonicalize_f32_zero(values[1]),
        z: canonicalize_f32_zero(values[2]),
    })
}

fn canonicalize_f32_zero(value: f32) -> f32 {
    if value == 0.0 { 0.0 } else { value }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LocalPoseError {
    InvalidOrigin,
    InvalidEnvelope {
        envelope: f64,
    },
    PositionOutsideEnvelope {
        axis: Axis,
        value: f64,
        envelope: f64,
    },
    F32ConversionFailed {
        axis: Axis,
        value: f64,
    },
    InvalidBasis,
}

pub struct BatchRequest<'a> {
    pub vehicle_id: u64,
    pub polyline: &'a Polyline,
    pub binding: LengthBinding,
    pub progress: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalPoseRecord {
    pub vehicle_id: u64,
    pub pose: LocalPoseF32,
}

pub fn extract_local_batch_into(
    requests: &[BatchRequest<'_>],
    origin: CanonicalPoint3,
    envelope: f64,
    output: &mut Vec<LocalPoseRecord>,
) -> Result<(), BatchError> {
    let mut scratch = Vec::with_capacity(requests.len());
    for (index, request) in requests.iter().enumerate() {
        let pose = request
            .binding
            .sample_progress(request.polyline, request.progress)
            .map_err(|source| BatchError::Sample {
                index,
                vehicle_id: request.vehicle_id,
                source,
            })?;
        let pose =
            try_to_local_f32(pose, origin, envelope).map_err(|source| BatchError::LocalPose {
                index,
                vehicle_id: request.vehicle_id,
                source,
            })?;
        scratch.push(LocalPoseRecord {
            vehicle_id: request.vehicle_id,
            pose,
        });
    }

    output.clear();
    output.append(&mut scratch);
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BatchError {
    Sample {
        index: usize,
        vehicle_id: u64,
        source: SampleError,
    },
    LocalPose {
        index: usize,
        vehicle_id: u64,
        source: LocalPoseError,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn horizontal_polyline() -> Polyline {
        Polyline::try_new(vec![
            CanonicalPoint3::new(0.0, 0.0, 0.0),
            CanonicalPoint3::new(10.0, 0.0, 0.0),
            CanonicalPoint3::new(10.0, 0.0, -10.0),
        ])
        .expect("fixture must be valid")
    }

    #[test]
    fn rejects_non_finite_degenerate_and_vertical_segments() {
        assert!(matches!(
            Polyline::try_new(vec![CanonicalPoint3::new(0.0, 0.0, 0.0)]),
            Err(PolylineError::TooFewPoints { actual: 1 })
        ));
        assert!(matches!(
            Polyline::try_new(vec![
                CanonicalPoint3::new(0.0, 0.0, 0.0),
                CanonicalPoint3::new(f64::NAN, 0.0, 0.0),
            ]),
            Err(PolylineError::NonFinitePoint { index: 1 })
        ));
        assert!(matches!(
            Polyline::try_new(vec![
                CanonicalPoint3::new(0.0, 0.0, 0.0),
                CanonicalPoint3::new(0.0, 0.0, 0.0),
            ]),
            Err(PolylineError::DegenerateSegment { index: 0, .. })
        ));
        assert!(matches!(
            Polyline::try_new(vec![
                CanonicalPoint3::new(0.0, 0.0, 0.0),
                CanonicalPoint3::new(0.0, 1.0, 0.0),
            ]),
            Err(PolylineError::DegenerateBasis { index: 0, .. })
        ));
    }

    #[test]
    fn samples_start_vertex_and_end_with_documented_tangent_rule() {
        let polyline = horizontal_polyline();

        let start = polyline.sample_arc_length(0.0).expect("start sample");
        assert_eq!(start.position, CanonicalPoint3::new(0.0, 0.0, 0.0));
        assert_eq!(start.tangent, CanonicalVector3::new(1.0, 0.0, 0.0));

        let vertex = polyline.sample_arc_length(10.0).expect("vertex sample");
        assert_eq!(vertex.position, CanonicalPoint3::new(10.0, 0.0, 0.0));
        assert_eq!(vertex.tangent, CanonicalVector3::new(0.0, 0.0, -1.0));

        let end = polyline.sample_arc_length(20.0).expect("end sample");
        assert_eq!(end.position, CanonicalPoint3::new(10.0, 0.0, -10.0));
        assert_eq!(end.tangent, CanonicalVector3::new(0.0, 0.0, -1.0));
    }

    #[test]
    fn length_binding_rejects_mismatch_and_maps_core_endpoint_exactly() {
        let polyline = horizontal_polyline();
        let accepted = LengthBinding::try_new(20.0 + 5.0e-7, &polyline)
            .expect("difference inside tolerance must bind");
        let end = accepted
            .sample_progress(&polyline, 20.0 + 5.0e-7)
            .expect("bound endpoint must sample");
        assert_eq!(end.position, polyline.end());
        let snapped_end = accepted
            .sample_progress(
                &polyline,
                20.0 + 5.0e-7 + CURRENT_CORE_BOUNDARY_TOLERANCE_METERS / 2.0,
            )
            .expect("progress inside boundary tolerance must snap");
        assert_eq!(snapped_end.position, polyline.end());
        assert!(matches!(
            accepted.sample_progress(
                &polyline,
                20.0 + 5.0e-7 + CURRENT_CORE_BOUNDARY_TOLERANCE_METERS * 2.0,
            ),
            Err(SampleError::ProgressOutOfRange { .. })
        ));

        assert!(matches!(
            LengthBinding::try_new(20.01, &polyline),
            Err(BindingError::LengthMismatch { .. })
        ));
    }

    #[test]
    fn local_conversion_subtracts_large_origin_before_f32_cast() {
        let pose = CanonicalPose {
            position: CanonicalPoint3::new(1_000_000_000.125, 0.0, 0.0),
            tangent: CanonicalVector3::new(1.0, 0.0, 0.0),
            up: CanonicalVector3::Y,
        };
        let origin = CanonicalPoint3::new(1_000_000_000.0, 0.0, 0.0);
        let local = try_to_local_f32(pose, origin, RESEARCH_LOCAL_ENVELOPE_METERS)
            .expect("local conversion must succeed");

        assert_eq!(local.position.x, 0.125);
        assert_eq!(pose.position.x as f32 - origin.x as f32, 0.0);

        let invalid_basis = CanonicalPose {
            up: CanonicalVector3::new(2.0, 0.0, 0.0),
            ..pose
        };
        assert_eq!(
            try_to_local_f32(invalid_basis, origin, RESEARCH_LOCAL_ENVELOPE_METERS),
            Err(LocalPoseError::InvalidBasis)
        );
    }

    #[test]
    fn failed_batch_does_not_modify_previous_output() {
        let polyline = horizontal_polyline();
        let binding =
            LengthBinding::try_new(polyline.arc_length(), &polyline).expect("fixture must bind");
        let requests = [
            BatchRequest {
                vehicle_id: 1,
                polyline: &polyline,
                binding,
                progress: 5.0,
            },
            BatchRequest {
                vehicle_id: 2,
                polyline: &polyline,
                binding,
                progress: 25.0,
            },
        ];
        let sentinel = LocalPoseRecord {
            vehicle_id: 99,
            pose: LocalPoseF32 {
                position: LocalPoint3F32 {
                    x: 1.0,
                    y: 2.0,
                    z: 3.0,
                },
                tangent: LocalVector3F32 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
                up: LocalVector3F32 {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                },
            },
        };
        let mut output = vec![sentinel];

        assert!(matches!(
            extract_local_batch_into(
                &requests,
                CanonicalPoint3::new(0.0, 0.0, 0.0),
                RESEARCH_LOCAL_ENVELOPE_METERS,
                &mut output,
            ),
            Err(BatchError::Sample {
                index: 1,
                vehicle_id: 2,
                ..
            })
        ));
        assert_eq!(output, vec![sentinel]);
    }

    #[test]
    fn successful_batch_reuses_output_capacity_after_atomic_compute() {
        let polyline = horizontal_polyline();
        let binding =
            LengthBinding::try_new(polyline.arc_length(), &polyline).expect("fixture must bind");
        let requests = [BatchRequest {
            vehicle_id: 1,
            polyline: &polyline,
            binding,
            progress: 5.0,
        }];
        let mut output = Vec::with_capacity(8);
        output.push(LocalPoseRecord {
            vehicle_id: 99,
            pose: LocalPoseF32 {
                position: LocalPoint3F32 {
                    x: 1.0,
                    y: 2.0,
                    z: 3.0,
                },
                tangent: LocalVector3F32 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
                up: LocalVector3F32 {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                },
            },
        });
        let reserved_capacity = output.capacity();

        extract_local_batch_into(
            &requests,
            CanonicalPoint3::new(0.0, 0.0, 0.0),
            RESEARCH_LOCAL_ENVELOPE_METERS,
            &mut output,
        )
        .expect("valid batch must succeed");

        assert_eq!(output.len(), 1);
        assert_eq!(output[0].vehicle_id, 1);
        assert_eq!(output.capacity(), reserved_capacity);
    }

    #[test]
    fn connected_edges_use_position_tolerance_not_tangent_equality() {
        let first = Polyline::try_new(vec![
            CanonicalPoint3::new(0.0, 0.0, 0.0),
            CanonicalPoint3::new(10.0, 0.0, 0.0),
        ])
        .expect("fixture must be valid");
        let joined = Polyline::try_new(vec![
            CanonicalPoint3::new(10.000_5, 0.0, 0.0),
            CanonicalPoint3::new(10.000_5, 0.0, -10.0),
        ])
        .expect("fixture must be valid");
        let disconnected = Polyline::try_new(vec![
            CanonicalPoint3::new(10.01, 0.0, 0.0),
            CanonicalPoint3::new(10.01, 0.0, -10.0),
        ])
        .expect("fixture must be valid");

        validate_join(&first, &joined).expect("sub-millimeter join must pass");
        assert!(validate_join(&first, &disconnected).is_err());
    }
}

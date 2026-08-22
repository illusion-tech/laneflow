use std::sync::Arc;

use laneflow_static_contract::{
    CanonicalFrameOrdinal, LaneEdgeOrdinal, NetworkRevisionId, ParkingSpaceOrdinal,
};
use laneflow_static_network::{LaneGeometryView, SharedNetworkRevision};

use crate::{
    CanonicalPoint3F32, CanonicalPoseF32, CanonicalUnitVector3F32, CanonicalVector3F32,
    FramePlacementToken, PoseRecordId, SpatialError,
};

/// 共享根上的位姿来源。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PoseSource {
    /// 车道进度。
    Lane {
        /// 共享根边序号。
        edge: LaneEdgeOrdinal,
        /// 与共享根边长同域的进度。
        progress: f64,
    },
    /// 停车位。
    Parking {
        /// 共享根停车位序号。
        space: ParkingSpaceOrdinal,
    },
}

/// 一条 pose 批次输入。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PoseInput {
    record: PoseRecordId,
    source: PoseSource,
}

impl PoseInput {
    /// 车道采样输入。
    #[must_use]
    pub const fn lane(record: PoseRecordId, edge: LaneEdgeOrdinal, progress: f64) -> Self {
        Self {
            record,
            source: PoseSource::Lane { edge, progress },
        }
    }

    /// 停车位采样输入。
    #[must_use]
    pub const fn parking(record: PoseRecordId, space: ParkingSpaceOrdinal) -> Self {
        Self {
            record,
            source: PoseSource::Parking { space },
        }
    }

    #[must_use]
    pub const fn record(self) -> PoseRecordId {
        self.record
    }

    #[must_use]
    pub const fn source(self) -> PoseSource {
        self.source
    }
}

/// `SpatialSession::bind` 失败。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpatialBindError {
    /// 有 Spatial component 但缺少 lane-pose，不能做车辆位姿采样。
    MissingLanePose,
}

/// 绑定到一根 `SharedNetworkRevision` 的 Spatial session。
pub struct SpatialSession {
    revision: Arc<SharedNetworkRevision>,
}

impl SpatialSession {
    /// 绑定根 `Arc`。无 Spatial 时返回 `Ok(None)`。有 Spatial 但无 lane-pose 时失败。
    pub fn bind(revision: Arc<SharedNetworkRevision>) -> Result<Option<Self>, SpatialBindError> {
        match revision.spatial() {
            None => Ok(None),
            Some(spatial) if spatial.lane_pose().is_none() => {
                Err(SpatialBindError::MissingLanePose)
            }
            Some(_) => Ok(Some(Self { revision })),
        }
    }

    /// 绑定所用根。
    #[must_use]
    pub fn revision(&self) -> Arc<SharedNetworkRevision> {
        Arc::clone(&self.revision)
    }

    /// 根修订身份。
    #[must_use]
    pub fn network_revision(&self) -> NetworkRevisionId {
        self.revision.network_revision()
    }

    /// 按调用方顺序提取 pose 批次。混 frame 整批失败。
    pub fn extract_pose_batch(
        &self,
        placement_token: FramePlacementToken,
        inputs: &[PoseInput],
        output: &mut CanonicalPoseBatch,
    ) -> Result<(), SpatialError> {
        let mut records = Vec::with_capacity(inputs.len());
        let mut frame: Option<CanonicalFrameOrdinal> = None;
        for (input_index, input) in inputs.iter().copied().enumerate() {
            let (sampled_frame, pose) = self.sample(input.source).map_err(|source| {
                SpatialError::SharedPoseRecordFailed {
                    input_index,
                    record: input.record,
                    source: Box::new(source),
                }
            })?;
            if let Some(expected) = frame {
                if expected != sampled_frame {
                    return Err(SpatialError::BatchFrameMismatch {
                        registry_frame_id: format!("{expected}"),
                        output_frame_id: format!("{sampled_frame}"),
                    });
                }
            } else {
                frame = Some(sampled_frame);
            }
            records.push(CanonicalPoseRecord {
                record: input.record,
                pose,
            });
        }
        output.network_revision = Some(self.network_revision());
        output.canonical_frame = frame;
        output.placement_token = placement_token;
        output.records = records;
        Ok(())
    }

    fn sample(
        &self,
        source: PoseSource,
    ) -> Result<(CanonicalFrameOrdinal, CanonicalPoseF32), SpatialError> {
        match source {
            PoseSource::Lane { edge, progress } => self.sample_lane(edge, progress),
            PoseSource::Parking { space } => self.sample_parking(space),
        }
    }

    fn sample_lane(
        &self,
        edge: LaneEdgeOrdinal,
        progress: f64,
    ) -> Result<(CanonicalFrameOrdinal, CanonicalPoseF32), SpatialError> {
        let traffic_length = *self
            .revision
            .traffic()
            .lane_lengths_meters()
            .get(edge.index())
            .ok_or(SpatialError::UnknownLaneEdge { edge })?;
        if progress < 0.0 || progress > traffic_length {
            return Err(SpatialError::SharedProgressOutOfRange {
                edge,
                progress_meters: progress,
                max_meters: traffic_length,
            });
        }
        let geometry = self
            .revision
            .spatial()
            .and_then(|spatial| spatial.lane_pose())
            .and_then(|network| network.lane_geometry(edge))
            .ok_or(SpatialError::UnknownLaneEdge { edge })?;
        let pose = sample_lane_geometry(geometry, traffic_length, progress)?;
        Ok((geometry.canonical_frame(), pose))
    }

    fn sample_parking(
        &self,
        space: ParkingSpaceOrdinal,
    ) -> Result<(CanonicalFrameOrdinal, CanonicalPoseF32), SpatialError> {
        let relations = self.revision.traffic().relations();
        let (entry_edge, entry_progress) = relations
            .parking_space_entry(space)
            .ok_or(SpatialError::UnknownParkingSpace { space })?;
        let (lateral, heading, _length, _width) = relations
            .parking_space_geometry(space)
            .ok_or(SpatialError::UnknownParkingSpace { space })?;
        let (frame, anchor) = self.sample_lane(entry_edge, entry_progress)?;
        let left = cross(anchor.up(), anchor.tangent())
            .try_normalize()
            .map_err(|source| SpatialError::SharedParkingPoseComputation {
                space,
                operation: "parking left basis",
                source: Box::new(source),
            })?;
        let lateral = lateral as f32;
        if !lateral.is_finite() {
            return Err(SpatialError::UnknownParkingSpace { space });
        }
        let displacement = left.as_vector().checked_scale(lateral).map_err(|source| {
            SpatialError::SharedParkingPoseComputation {
                space,
                operation: "parking position",
                source: Box::new(source),
            }
        })?;
        let position = anchor
            .position()
            .checked_add_vector(displacement)
            .map_err(|source| SpatialError::SharedParkingPoseComputation {
                space,
                operation: "parking position",
                source: Box::new(source),
            })?;
        let heading = heading as f32;
        let (sin_heading, cos_heading) = heading.sin_cos();
        let forward = anchor
            .tangent()
            .as_vector()
            .checked_scale(cos_heading)
            .and_then(|forward| {
                left.as_vector()
                    .checked_scale(sin_heading)
                    .and_then(|lateral| forward.checked_add(lateral))
            })
            .and_then(CanonicalVector3F32::try_normalize)
            .map_err(|source| SpatialError::SharedParkingPoseComputation {
                space,
                operation: "parking heading",
                source: Box::new(source),
            })?;
        Ok((
            frame,
            CanonicalPoseF32::from_parts(position, forward, anchor.up()),
        ))
    }
}

/// 共享根 pose 批次。
#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalPoseBatch {
    network_revision: Option<NetworkRevisionId>,
    canonical_frame: Option<CanonicalFrameOrdinal>,
    placement_token: FramePlacementToken,
    records: Vec<CanonicalPoseRecord>,
}

impl Default for CanonicalPoseBatch {
    fn default() -> Self {
        Self {
            network_revision: None,
            canonical_frame: None,
            placement_token: FramePlacementToken::new(0),
            records: Vec::new(),
        }
    }
}

impl CanonicalPoseBatch {
    /// 空批次。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub const fn network_revision(&self) -> Option<NetworkRevisionId> {
        self.network_revision
    }

    #[must_use]
    pub const fn canonical_frame(&self) -> Option<CanonicalFrameOrdinal> {
        self.canonical_frame
    }

    #[must_use]
    pub const fn placement_token(&self) -> FramePlacementToken {
        self.placement_token
    }

    #[must_use]
    pub fn records(&self) -> &[CanonicalPoseRecord] {
        &self.records
    }
}

/// 单条共享根 pose 记录。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalPoseRecord {
    record: PoseRecordId,
    pose: CanonicalPoseF32,
}

impl CanonicalPoseRecord {
    #[must_use]
    pub const fn record(self) -> PoseRecordId {
        self.record
    }

    #[must_use]
    pub const fn pose(self) -> CanonicalPoseF32 {
        self.pose
    }
}

fn sample_lane_geometry(
    geometry: LaneGeometryView<'_>,
    traffic_length: f64,
    progress: f64,
) -> Result<CanonicalPoseF32, SpatialError> {
    let points = geometry.points();
    let segments = geometry.segments();
    if points.len() < 2 || segments.is_empty() {
        return Err(SpatialError::UnknownLaneEdge {
            edge: LaneEdgeOrdinal::from_raw(0),
        });
    }
    let pose_at =
        |point_index: usize, segment_index: usize| -> Result<CanonicalPoseF32, SpatialError> {
            let point = points[point_index];
            let segment = segments[segment_index];
            Ok(CanonicalPoseF32::from_parts(
                CanonicalPoint3F32::try_new(point.x, point.y, point.z)?,
                unit_from_array(segment.tangent)?,
                unit_from_array(segment.up)?,
            ))
        };
    if progress == 0.0 {
        return pose_at(0, 0);
    }
    if progress >= traffic_length {
        return pose_at(points.len() - 1, segments.len() - 1);
    }
    let arc = geometry.arc_length_meters();
    let geometry_s = ((progress / traffic_length) * f64::from(arc)) as f32;
    if geometry_s >= arc {
        return pose_at(points.len() - 1, segments.len() - 1);
    }
    let segment_index = segments
        .partition_point(|segment| segment.cumulative_end_meters <= geometry_s)
        .min(segments.len() - 1);
    let segment = segments[segment_index];
    let start_s = if segment_index == 0 {
        0.0
    } else {
        segments[segment_index - 1].cumulative_end_meters
    };
    let ratio = (geometry_s - start_s) / segment.length_meters;
    let start = points[segment_index];
    let end = points[segment_index + 1];
    let position = CanonicalPoint3F32::try_new(
        start.x + (end.x - start.x) * ratio,
        start.y + (end.y - start.y) * ratio,
        start.z + (end.z - start.z) * ratio,
    )?;
    Ok(CanonicalPoseF32::from_parts(
        position,
        unit_from_array(segment.tangent)?,
        unit_from_array(segment.up)?,
    ))
}

fn unit_from_array(values: [f32; 3]) -> Result<CanonicalUnitVector3F32, SpatialError> {
    CanonicalVector3F32::try_new(values[0], values[1], values[2])?.try_normalize()
}

fn cross(left: CanonicalUnitVector3F32, right: CanonicalUnitVector3F32) -> CanonicalVector3F32 {
    CanonicalVector3F32::try_new(
        left.y() * right.z() - left.z() * right.y(),
        left.z() * right.x() - left.x() * right.z(),
        left.x() * right.y() - left.y() * right.x(),
    )
    .expect("crossing finite unit directions produces a finite vector")
}

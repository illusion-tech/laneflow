//! 面向 Adapter 的稳定批量位姿提取边界。

use std::mem;

use laneflow_core::{EdgeHandle, EdgeProgress, ParkingRegistry, ParkingSpaceHandle, VehicleHandle};

use crate::{
    CanonicalFrameId, CanonicalPoseF32, CanonicalUnitVector3F32, CanonicalVector3F32, SpatialError,
    SpatialRegistry,
};

const PARKING_LATERAL_OFFSET_FIELD: &str = "lateral_offset";
const PARKING_HEADING_OFFSET_FIELD: &str = "heading_offset_radians";
const PARKING_LEFT_OPERATION: &str = "parking left basis";
const PARKING_POSITION_OPERATION: &str = "parking position";
const PARKING_HEADING_OPERATION: &str = "parking heading";

/// Adapter 为 canonical frame 的某次宿主放置颁发的不透明等值 token。
///
/// token 只在批次头保存一次。Spatial 原样回显它；Adapter 在提交宿主变换前
/// 必须确认该 token 仍对应当前 frame placement。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FramePlacementToken(u64);

impl FramePlacementToken {
    /// 从调用方拥有的稳定值创建 token。
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
}

/// 单条批量输入的位置权威来源。
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum PoseSource {
    /// Active/Stopped 车辆的 lane-relative 位置。
    Lane {
        /// Core edge handle。
        edge: EdgeHandle,
        /// Core 权威 edge progress。
        progress: EdgeProgress,
    },
    /// Parked 车辆的 ParkingSpace-relative 位置。
    Parking {
        /// Core ParkingSpace handle。
        space: ParkingSpaceHandle,
    },
}

/// 调用方按 committed Core snapshot 稳定排序的单条位姿输入。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PoseInputRecord {
    vehicle: VehicleHandle,
    source: PoseSource,
}

impl PoseInputRecord {
    /// 创建 lane-relative 输入。
    pub const fn lane(vehicle: VehicleHandle, edge: EdgeHandle, progress: EdgeProgress) -> Self {
        Self {
            vehicle,
            source: PoseSource::Lane { edge, progress },
        }
    }

    /// 创建 ParkingSpace-relative 输入。
    pub const fn parking(vehicle: VehicleHandle, space: ParkingSpaceHandle) -> Self {
        Self {
            vehicle,
            source: PoseSource::Parking { space },
        }
    }

    /// 返回稳定 vehicle handle。
    pub const fn vehicle(self) -> VehicleHandle {
        self.vehicle
    }

    /// 返回位置权威来源。
    pub const fn source(self) -> PoseSource {
        self.source
    }
}

/// canonical 批量结果中的单条车辆位姿。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalPoseRecordF32 {
    vehicle: VehicleHandle,
    pose: CanonicalPoseF32,
}

impl CanonicalPoseRecordF32 {
    /// 返回稳定 vehicle handle。
    pub const fn vehicle(self) -> VehicleHandle {
        self.vehicle
    }

    /// 返回 canonical `f32` 位姿。
    pub const fn pose(self) -> CanonicalPoseF32 {
        self.pose
    }
}

/// 已提交、可供 Adapter 消费的 canonical `f32` 位姿批次。
///
/// frame ID 在构造后保持不变。切换到另一个 canonical frame 时，调用方必须使用
/// 对应 frame 创建新的 committed batch；同一 frame 的宿主放置切换则使用新的
/// [`FramePlacementToken`] 成功提取后原子更新。
#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalPoseBatchF32 {
    frame_id: CanonicalFrameId,
    placement_token: FramePlacementToken,
    records: Vec<CanonicalPoseRecordF32>,
}

impl CanonicalPoseBatchF32 {
    /// 创建空的 committed batch。
    pub fn new(frame_id: CanonicalFrameId, placement_token: FramePlacementToken) -> Self {
        Self::with_capacity(frame_id, placement_token, 0)
    }

    /// 创建带调用方预留容量的空 committed batch。
    pub fn with_capacity(
        frame_id: CanonicalFrameId,
        placement_token: FramePlacementToken,
        capacity: usize,
    ) -> Self {
        Self {
            frame_id,
            placement_token,
            records: Vec::with_capacity(capacity),
        }
    }

    /// 返回该 batch 唯一的 canonical frame ID。
    pub const fn frame_id(&self) -> &CanonicalFrameId {
        &self.frame_id
    }

    /// 返回生成该 batch 时回显的 frame placement token。
    pub const fn placement_token(&self) -> FramePlacementToken {
        self.placement_token
    }

    /// 返回稳定输入顺序的 committed records。
    pub fn records(&self) -> &[CanonicalPoseRecordF32] {
        &self.records
    }

    /// 返回 committed record 数量。
    pub const fn len(&self) -> usize {
        self.records.len()
    }

    /// 返回 batch 是否为空。
    pub const fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// 返回 committed record buffer 当前容量。
    pub const fn capacity(&self) -> usize {
        self.records.capacity()
    }
}

/// 调用方拥有并可跨 tick 复用的未提交 scratch buffer。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CanonicalPoseBatchScratch {
    records: Vec<CanonicalPoseRecordF32>,
}

impl CanonicalPoseBatchScratch {
    /// 创建空 scratch buffer。
    pub const fn new() -> Self {
        Self {
            records: Vec::new(),
        }
    }

    /// 创建带预留容量的空 scratch buffer。
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            records: Vec::with_capacity(capacity),
        }
    }

    /// 返回 scratch 当前容量。
    pub const fn capacity(&self) -> usize {
        self.records.capacity()
    }

    /// 返回 scratch 是否为空。
    ///
    /// 每次提取返回时，无论成功或失败，scratch 都为空但保留容量。
    pub const fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

impl SpatialRegistry {
    /// 按调用方稳定顺序原子提取 canonical `f32` 位姿批次。
    ///
    /// 全部 records 先写入 `scratch`。只有所有记录成功后才交换到 `output` 并更新
    /// placement token；任何失败都会清空 scratch 且保持旧 output 完全不变。
    ///
    /// # Errors
    ///
    /// output frame 与 registry frame 不同，或任一 lane/Parking 输入无法解析为合法位姿
    /// 时返回结构化 [`SpatialError`]。
    pub fn extract_pose_batch(
        &self,
        parking: &ParkingRegistry,
        placement_token: FramePlacementToken,
        inputs: &[PoseInputRecord],
        output: &mut CanonicalPoseBatchF32,
        scratch: &mut CanonicalPoseBatchScratch,
    ) -> Result<(), SpatialError> {
        scratch.records.clear();

        if output.frame_id != *self.frame_id() {
            return Err(SpatialError::BatchFrameMismatch {
                registry_frame_id: self.frame_id().as_str().to_owned(),
                output_frame_id: output.frame_id.as_str().to_owned(),
            });
        }

        for (input_index, input) in inputs.iter().copied().enumerate() {
            let pose = self
                .sample_pose_source(parking, input.source)
                .map_err(|source| SpatialError::PoseRecordFailed {
                    input_index,
                    vehicle: input.vehicle,
                    source: Box::new(source),
                });
            let pose = match pose {
                Ok(pose) => pose,
                Err(error) => {
                    scratch.records.clear();
                    return Err(error);
                }
            };

            scratch.records.push(CanonicalPoseRecordF32 {
                vehicle: input.vehicle,
                pose,
            });
        }

        mem::swap(&mut output.records, &mut scratch.records);
        scratch.records.clear();
        output.placement_token = placement_token;
        Ok(())
    }

    fn sample_pose_source(
        &self,
        parking: &ParkingRegistry,
        source: PoseSource,
    ) -> Result<CanonicalPoseF32, SpatialError> {
        match source {
            PoseSource::Lane { edge, progress } => self.sample(edge, progress),
            PoseSource::Parking { space } => self.sample_parking_pose(parking, space),
        }
    }

    fn sample_parking_pose(
        &self,
        parking: &ParkingRegistry,
        space: ParkingSpaceHandle,
    ) -> Result<CanonicalPoseF32, SpatialError> {
        let anchor = parking
            .space_entry(space)
            .ok_or(SpatialError::UnknownParkingSpaceHandle { space })?;
        let geometry = parking
            .space_geometry(space)
            .expect("a resolved ParkingSpace always has geometry");
        let progress = EdgeProgress::try_new(anchor.progress())
            .expect("a normalized Parking entry progress is finite and non-negative");
        let anchor_pose = self.sample(anchor.edge(), progress)?;

        let left = cross(anchor_pose.up(), anchor_pose.tangent())
            .try_normalize()
            .map_err(|source| parking_pose_error(space, PARKING_LEFT_OPERATION, source))?;
        let lateral_offset = parking_f64_to_f32(
            space,
            PARKING_LATERAL_OFFSET_FIELD,
            geometry.lateral_offset(),
        )?;
        let displacement = left
            .as_vector()
            .checked_scale(lateral_offset)
            .map_err(|source| parking_pose_error(space, PARKING_POSITION_OPERATION, source))?;
        let position = anchor_pose
            .position()
            .checked_add_vector(displacement)
            .map_err(|source| parking_pose_error(space, PARKING_POSITION_OPERATION, source))?;

        let heading_offset = parking_f64_to_f32(
            space,
            PARKING_HEADING_OFFSET_FIELD,
            geometry.heading_offset_radians(),
        )?;
        let (sin_heading, cos_heading) = heading_offset.sin_cos();
        let forward = anchor_pose
            .tangent()
            .as_vector()
            .checked_scale(cos_heading)
            .and_then(|forward| {
                left.as_vector()
                    .checked_scale(sin_heading)
                    .and_then(|lateral| forward.checked_add(lateral))
            })
            .and_then(CanonicalVector3F32::try_normalize)
            .map_err(|source| parking_pose_error(space, PARKING_HEADING_OPERATION, source))?;

        Ok(CanonicalPoseF32::from_parts(
            position,
            forward,
            anchor_pose.up(),
        ))
    }
}

fn cross(left: CanonicalUnitVector3F32, right: CanonicalUnitVector3F32) -> CanonicalVector3F32 {
    CanonicalVector3F32::try_new(
        left.y() * right.z() - left.z() * right.y(),
        left.z() * right.x() - left.x() * right.z(),
        left.x() * right.y() - left.y() * right.x(),
    )
    .expect("crossing finite unit directions produces a finite vector")
}

fn parking_f64_to_f32(
    space: ParkingSpaceHandle,
    field: &'static str,
    value: f64,
) -> Result<f32, SpatialError> {
    let converted = value as f32;
    if !converted.is_finite() {
        return Err(SpatialError::ParkingGeometryOutOfF32Range {
            space,
            field,
            value,
        });
    }
    Ok(if converted == 0.0 { 0.0 } else { converted })
}

fn parking_pose_error(
    space: ParkingSpaceHandle,
    operation: &'static str,
    source: SpatialError,
) -> SpatialError {
    SpatialError::ParkingPoseComputation {
        space,
        operation,
        source: Box::new(source),
    }
}

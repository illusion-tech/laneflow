use laneflow_static_contract::SignalAspect;
use laneflow_static_contract::{LaneEdgeOrdinal, ParkingSpaceOrdinal, SignalGroupOrdinal};

use crate::VehicleHandle;

/// 已提交 pose 的权威来源。Spatial 批次另行映射为 `PoseRecordId`。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PoseSource {
    /// 车道上的进度。
    Lane {
        /// 共享根边序号。
        edge: LaneEdgeOrdinal,
        /// 当前边上的毫米进度。
        progress_mm: u32,
    },
    /// 停车位。
    Parking {
        /// 共享根停车位序号。
        space: ParkingSpaceOrdinal,
    },
}

/// 稳定顺序的已提交 pose 源。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CommittedPoseSourceBatch {
    pub(crate) items: Vec<(VehicleHandle, PoseSource)>,
}

impl CommittedPoseSourceBatch {
    #[must_use]
    pub fn as_slice(&self) -> &[(VehicleHandle, PoseSource)] {
        &self.items
    }
}

/// 稳定按组序号的已提交信号指示。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CommittedSignalGroupBatch {
    pub(crate) items: Vec<(SignalGroupOrdinal, SignalAspect)>,
}

impl CommittedSignalGroupBatch {
    #[must_use]
    pub fn as_slice(&self) -> &[(SignalGroupOrdinal, SignalAspect)] {
        &self.items
    }
}

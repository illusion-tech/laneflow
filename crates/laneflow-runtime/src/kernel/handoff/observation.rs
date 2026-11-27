use laneflow_static_contract::{CanonicalFrameOrdinal, NetworkRevisionId};
use laneflow_static_network::vehicle::{HandoffFootprint, PoseBounds};
use thiserror::Error;

use crate::{RouteHandle, TrafficWorld, VehicleHandle, WorldGeneration};

/// 同一提交拍的横移几何。位置与方向分量界和两侧占用取自相同的车体求值。
#[derive(Clone, Debug, PartialEq)]
pub struct CommittedHandoffGeometry {
    /// 世界世代。
    pub world_generation: WorldGeneration,
    /// 此拍运行和查询所用的共享路网修订。
    pub revision: NetworkRevisionId,
    /// 后轴参考点与车体三轴的规范坐标框架。
    pub frame: CanonicalFrameOrdinal,
    /// 该状态已成功提交的世界刻度。
    pub tick: u64,
    /// 该车此刻的正式路线。
    pub route: RouteHandle,
    /// 已登记交接的源出现下标。
    pub source_index: u32,
    /// 前保险杠此刻绑定的路线出现。
    pub front_occurrence: u32,
    /// 沿横移轨迹累计的真实后轴行程，单位微米。
    pub rear_travel_um: u64,
    /// 在上述规范框架中，以后轴为参考点的交通姿态界。
    pub pose: PoseBounds,
    /// 前保险杠侧别、所在边和两侧车体投影。
    pub footprint: HandoffFootprint,
}

/// 读取一项已提交横移的几何失败。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum HandoffGeometryReadError {
    /// 句柄不指向本世界的有效车辆。
    #[error("未知或失效车辆句柄")]
    UnknownVehicle,
    /// 无法在提交状态绑定的修订、配置和预算内重建该姿态。
    #[error("无法重建已提交的横移姿态")]
    Unproven,
}

impl TrafficWorld {
    /// 按有效车辆句柄读取本提交拍的横移姿态及占用。未持有横移的车辆返回空。
    ///
    /// # Errors
    ///
    /// 失效句柄、修订或姿态复核失败时返回相应错误。
    ///
    /// # Panics
    ///
    /// 世界因执行 panic 失效后调用会 panic。
    pub fn committed_handoff_geometry(
        &self,
        handle: VehicleHandle,
    ) -> Result<Option<CommittedHandoffGeometry>, HandoffGeometryReadError> {
        self.execution.assert_usable();
        let vehicle = self
            .state
            .vehicle_state(handle)
            .ok_or(HandoffGeometryReadError::UnknownVehicle)?;
        let Some(record) = self
            .state
            .committed
            .handoffs
            .iter()
            .find(|record| record.vehicle == handle)
        else {
            return Ok(None);
        };
        let revision = &self.state.binding.revision;
        let unproven = HandoffGeometryReadError::Unproven;
        let route = self.state.compiled_route(vehicle.route).ok_or(unproven)?;
        let (sample, pose, frame) = record
            .geometry(
                &self.state.binding,
                route,
                vehicle,
                &self.state.derived.handoff_plans,
            )
            .ok_or(unproven)?;
        Ok(Some(CommittedHandoffGeometry {
            world_generation: self.world_generation(),
            revision: revision.network_revision(),
            frame,
            tick: self.tick_index(),
            route: vehicle.route,
            source_index: record.source_index,
            front_occurrence: sample.front_occurrence,
            rear_travel_um: record.rear_travel_um,
            pose,
            footprint: sample.footprint,
        }))
    }
}

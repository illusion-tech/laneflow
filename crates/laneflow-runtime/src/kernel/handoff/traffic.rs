use laneflow_static_network::vehicle::HandoffOutcome;
use thiserror::Error;

use super::{
    BoundRouteHandoff, RouteHandoffSample, RouteHandoffSampleError, RouteHandoffTrial,
    RouteHandoffTrialError,
};
use crate::{StepError, TrafficWorld, VehicleHandle};

/// 进入世界前，以外部候选的拍末车体外包核对本世界当前已提交的活动车辆。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteHandoffTraffic {
    /// 受检候选的拍末位置。车体投影可超出交接路线实际经过的边。
    pub sample: RouteHandoffSample,
    /// 源侧、目标侧各自首个可能相交的车辆。闭端接触也计为可能相交。
    /// 某侧为 `None` 表示其车体外包与已提交的同边毫米占用记录均分离。
    pub side_blockers: [Option<VehicleHandle>; 2],
    /// 实际搜索的物理边数，顺序为源、目标，每侧至多八条。
    pub checked_edges: [u8; 2],
}

impl RouteHandoffTraffic {
    /// 按源侧、目标侧的顺序返回每侧选中的车辆，同一车辆只返回一次。
    pub fn blockers(&self) -> impl Iterator<Item = VehicleHandle> {
        let [source, target] = self.side_blockers;
        source
            .into_iter()
            .chain(target.filter(|v| Some(*v) != source))
    }
}

/// 拟议行程或本世界的已提交占用无法验证。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum RouteHandoffTrafficError {
    /// 已绑定的路线、出现、车型或准入检查失败。
    #[error(transparent)]
    Trial(#[from] RouteHandoffTrialError),
    /// 静态几何未能定位这次交接。
    #[error("候选没有可取样的交接轨迹：{0:?}")]
    Unlocated(HandoffOutcome),
    /// 拟议的累计行程或车体位置未能求证。
    #[error(transparent)]
    Sample(#[from] RouteHandoffSampleError),
    /// 本世界当前占用索引重建失败。
    #[error("无法读取已提交的活动车辆占用：{0}")]
    Occupancy(StepError),
}

impl TrafficWorld {
    /// 从指定窗口、车型和拟议的毫米行程重新求拍末车体，在两侧各找一个可能阻挡。行程求界容差取正整数毫米。
    /// 完整轨迹校验与随后取样各使用 `trial.limits`。每侧按车道行驶方向访问至多
    /// 八条物理边。在首条与车体外包相交的边上，取占用起点最小的车辆，再按终点、
    /// 已提交更新序破平。调用包含所有已提交活动车辆，适用于尚未进入世界的候选。
    /// 占用索引过期时从当前提交态重建；索引有效时每条边只需一次二分与一次后缀读取。
    /// 结果仅比较所取姿态与当前占用。两拍之间的扫过、其他车辆的前进和预约另行裁决。
    ///
    /// # Errors
    ///
    /// 路线、出现下标、车型或几何准入不符，行程取样未证，或占用索引重建失败时
    /// 保留对应错误，世界提交状态与游标保持原值。
    ///
    /// # Panics
    ///
    /// 世界因执行 panic 失效后调用会 panic。
    pub fn preview_route_handoff_traffic(
        &mut self,
        binding: &BoundRouteHandoff,
        source_index: u32,
        trial: &RouteHandoffTrial,
        rear_travel_mm: u64,
        step_travel_mm: u64,
        tolerance_mm: std::num::NonZeroU32,
    ) -> Result<RouteHandoffTraffic, RouteHandoffTrafficError> {
        let preview = self.preview_route_handoff_at(binding, source_index, trial)?;
        let sample = preview
            .sample_after_travel(rear_travel_mm, step_travel_mm, tolerance_mm, trial.limits)
            .ok_or(RouteHandoffTrafficError::Unlocated(preview.report.outcome))?
            .sample?;

        self.state
            .ensure_current_occupancy()
            .map_err(RouteHandoffTrafficError::Occupancy)?;
        let mut checked_edges = [0; 2];
        let mut side_blockers = [None; 2];
        for (i, side) in sample.footprint.sides.iter().enumerate() {
            if let Some(footprint) = side {
                side_blockers[i] = footprint.enclosure.addresses().find_map(|address| {
                    checked_edges[i] += 1;
                    self.state
                        .derived
                        .occupancy
                        .touching_vehicle(address.edge, address.progress_mm)
                });
            }
        }
        Ok(RouteHandoffTraffic {
            sample,
            side_blockers,
            checked_edges,
        })
    }
}

//! 停止线、机动门与等待区视图。

use super::{CanonicalIdentityFieldView, CanonicalSignalControl, impl_stable_entity_view};
use crate::lir::{LirManeuverGate, LirSignalControl, LirStopLine, LirUnit, LirWaitingZone};
use laneflow_static_contract::{
    LaneEdgeOrdinal, ManeuverGateId, ManeuverGateOrdinal, ManeuverPathOrdinal, StopLineId,
    StopLineOrdinal, WaitingZoneId, WaitingZoneOrdinal,
};

impl_stable_entity_view!(
    CanonicalStopLineView,
    LirStopLine,
    StopLineOrdinal,
    StopLineId
);
impl_stable_entity_view!(
    CanonicalManeuverGateView,
    LirManeuverGate,
    ManeuverGateOrdinal,
    ManeuverGateId
);
impl_stable_entity_view!(
    CanonicalWaitingZoneView,
    LirWaitingZone,
    WaitingZoneOrdinal,
    WaitingZoneId
);

impl CanonicalStopLineView<'_> {
    /// 返回停止线所在的车道图边；位置语义固定为该边末端。
    #[must_use]
    pub const fn lane_edge(&self) -> LaneEdgeOrdinal {
        self.record.lane_edge
    }

    /// 返回引用该停止线的机动门，顺序按机动门规范身份冻结。
    #[must_use]
    pub fn maneuver_gates(&self) -> &[ManeuverGateOrdinal] {
        &self.lir.stop_line_maneuver_gates[self.record.maneuver_gates.as_usize_range()]
    }
}

impl CanonicalManeuverGateView<'_> {
    /// 返回唯一拥有本机动门的机动路径。
    #[must_use]
    pub const fn maneuver_path(&self) -> ManeuverPathOrdinal {
        self.record.maneuver_path
    }

    /// 返回路径边序列中受控转换的起始边下标。
    #[must_use]
    pub const fn transition_index(&self) -> u32 {
        self.record.transition_index
    }

    /// 返回位于转换起始边末端的停止线。
    #[must_use]
    pub const fn stop_line(&self) -> StopLineOrdinal {
        self.record.stop_line
    }

    /// 返回信号层控制绑定；`None` 不代表其他通行权约束已经放行。
    #[must_use]
    pub const fn signal_control(&self) -> CanonicalSignalControl {
        match self.record.signal_control {
            LirSignalControl::Group(group) => CanonicalSignalControl::Group(group),
            LirSignalControl::None => CanonicalSignalControl::None,
        }
    }
}

impl CanonicalWaitingZoneView<'_> {
    /// 返回唯一拥有本等待区的机动路径。
    #[must_use]
    pub const fn maneuver_path(&self) -> ManeuverPathOrdinal {
        self.record.maneuver_path
    }

    /// 返回界定等待区起点的入口门。
    #[must_use]
    pub const fn entry_gate(&self) -> ManeuverGateOrdinal {
        self.record.entry_gate
    }

    /// 返回界定等待区终点的释放门。
    #[must_use]
    pub const fn release_gate(&self) -> ManeuverGateOrdinal {
        self.record.release_gate
    }

    /// 返回允许同时占用等待区的最大交通参与单元数；该值已证明大于零。
    #[must_use]
    pub const fn max_occupancy(&self) -> u32 {
        self.record.max_occupancy
    }
}

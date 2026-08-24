//! 固定时制信号组、控制器、相位与灯色赋值视图。

use super::{CanonicalIdentityFieldView, impl_stable_entity_view};
use crate::lir::{
    LirSignalController, LirSignalGroup, LirSignalPhase, LirSignalPhaseState, LirUnit,
};
use laneflow_static_contract::{
    ManeuverGateOrdinal, SignalAspect, SignalControllerId, SignalControllerOrdinal, SignalGroupId,
    SignalGroupOrdinal, SignalPhaseId, SignalPhaseOrdinal,
};

/// 已验证机动门的信号层控制绑定。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CanonicalSignalControl {
    /// 由指定固定时制信号组给出灯色约束。
    Group(SignalGroupOrdinal),
    /// 信号层不对该门施加约束；不等同于最终可通行。
    None,
}

impl_stable_entity_view!(
    CanonicalSignalGroupView,
    LirSignalGroup,
    SignalGroupOrdinal,
    SignalGroupId
);
impl_stable_entity_view!(
    CanonicalSignalControllerView,
    LirSignalController,
    SignalControllerOrdinal,
    SignalControllerId
);
impl_stable_entity_view!(
    CanonicalSignalPhaseView,
    LirSignalPhase,
    SignalPhaseOrdinal,
    SignalPhaseId
);

impl CanonicalSignalGroupView<'_> {
    /// 返回唯一拥有本信号组的固定时制控制器。
    #[must_use]
    pub const fn controller(&self) -> SignalControllerOrdinal {
        self.record.controller
    }

    /// 返回由本组控制的非空机动门集合，按门的规范序号冻结。
    #[must_use]
    pub fn maneuver_gates(&self) -> &[ManeuverGateOrdinal] {
        &self.lir.signal_group_maneuver_gates[self.record.maneuver_gates.as_usize_range()]
    }
}

impl CanonicalSignalControllerView<'_> {
    /// 返回相对世界时间零点的规范循环偏移，单位为毫秒。
    #[must_use]
    pub const fn offset_ms(&self) -> u64 {
        self.record.offset_ms
    }

    /// 返回全部相位持续时间之和，单位为毫秒。
    #[must_use]
    pub const fn cycle_duration_ms(&self) -> u64 {
        self.record.cycle_duration_ms
    }

    /// 返回本控制器唯一拥有的信号组集合，按规范序号冻结。
    #[must_use]
    pub fn signal_groups(&self) -> &[SignalGroupOrdinal] {
        &self.lir.signal_controller_groups[self.record.signal_groups.as_usize_range()]
    }

    /// 返回定义固定时制循环程序的相位序列；该顺序具有执行语义。
    #[must_use]
    pub fn phases(&self) -> &[SignalPhaseOrdinal] {
        &self.lir.signal_controller_phases[self.record.phases.as_usize_range()]
    }
}

impl CanonicalSignalPhaseView<'_> {
    /// 返回唯一拥有本相位的信号控制器。
    #[must_use]
    pub const fn controller(&self) -> SignalControllerOrdinal {
        self.record.controller
    }

    /// 返回相位持续时间，单位为毫秒。
    #[must_use]
    pub const fn duration_ms(&self) -> u64 {
        self.record.duration_ms
    }

    /// 按控制器信号组规范顺序遍历完整灯色赋值。
    pub fn states(&self) -> impl ExactSizeIterator<Item = CanonicalSignalPhaseStateView<'_>> + '_ {
        self.lir.signal_phase_states[self.record.states.as_usize_range()]
            .iter()
            .map(CanonicalSignalPhaseStateView::from_record)
    }
}

/// 固定时制相位对一个信号组的只读状态赋值。
#[derive(Clone, Copy)]
pub struct CanonicalSignalPhaseStateView<'a> {
    record: &'a LirSignalPhaseState,
}

impl<'a> CanonicalSignalPhaseStateView<'a> {
    pub(in crate::compiler) const fn from_record(record: &'a LirSignalPhaseState) -> Self {
        Self { record }
    }
}

impl CanonicalSignalPhaseStateView<'_> {
    /// 返回被赋值的信号组。
    #[must_use]
    pub const fn signal_group(self) -> SignalGroupOrdinal {
        self.record.signal_group
    }

    /// 返回本相位内的灯色指示；它不是最终通行权判定。
    #[must_use]
    pub const fn aspect(self) -> SignalAspect {
        self.record.aspect
    }
}

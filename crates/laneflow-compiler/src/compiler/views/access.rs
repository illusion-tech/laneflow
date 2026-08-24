//! 参与者类别、车辆配置与静态准入规则视图。

use super::{CanonicalIdentityFieldView, impl_stable_entity_view};
use crate::lir::{LirAccessRule, LirAccessTarget, LirParticipantClass, LirUnit, LirVehicleProfile};
use laneflow_static_contract::{
    AccessEffect, AccessRuleId, AccessRuleOrdinal, LaneEdgeOrdinal, LaneGroupOrdinal,
    ManeuverPathOrdinal, ParticipantClassId, ParticipantClassOrdinal, RoadSectionOrdinal,
    VehicleProfileId, VehicleProfileOrdinal,
};

impl_stable_entity_view!(
    CanonicalParticipantClassView,
    LirParticipantClass,
    ParticipantClassOrdinal,
    ParticipantClassId
);
impl_stable_entity_view!(
    CanonicalVehicleProfileView,
    LirVehicleProfile,
    VehicleProfileOrdinal,
    VehicleProfileId
);

impl_stable_entity_view!(
    CanonicalAccessRuleView,
    LirAccessRule,
    AccessRuleOrdinal,
    AccessRuleId
);

impl CanonicalParticipantClassView<'_> {
    /// 返回可选单继承父类；`None` 表示分类法根类别。
    #[must_use]
    pub const fn parent(&self) -> Option<ParticipantClassOrdinal> {
        self.record.parent
    }

    /// 返回继承深度；根类别深度为 `0`。
    #[must_use]
    pub const fn depth(&self) -> u32 {
        self.record.depth
    }

    /// 返回用于常数时间后代判断的 Euler tour 半开区间。
    #[must_use]
    pub const fn subtree_interval(&self) -> (u32, u32) {
        (self.record.subtree_enter, self.record.subtree_exit)
    }

    /// 判断另一个类别序号是否位于本类别的传递子树中（包含自身）。
    #[must_use]
    pub fn contains(&self, other: ParticipantClassOrdinal) -> bool {
        self.lir
            .participant_classes
            .get(other.index())
            .is_some_and(|candidate| {
                self.record.subtree_enter <= candidate.subtree_enter
                    && candidate.subtree_enter < self.record.subtree_exit
            })
    }
}

impl CanonicalVehicleProfileView<'_> {
    /// 返回该车辆配置唯一引用的参与者类别。
    #[must_use]
    pub const fn participant_class(&self) -> ParticipantClassOrdinal {
        self.record.participant_class
    }

    /// 返回车辆长度，单位为米。
    #[must_use]
    pub const fn length_meters(&self) -> f64 {
        self.record.length_meters
    }

    /// 返回自由流期望速度，单位为米每秒。
    #[must_use]
    pub const fn desired_speed_meters_per_second(&self) -> f64 {
        self.record.desired_speed_meters_per_second
    }

    /// 返回行为最小间距，单位为米。
    #[must_use]
    pub const fn min_gap_meters(&self) -> f64 {
        self.record.min_gap_meters
    }

    /// 返回期望时间间隔，单位为秒。
    #[must_use]
    pub const fn time_headway_seconds(&self) -> f64 {
        self.record.time_headway_seconds
    }

    /// 返回最大舒适加速度，单位为米每二次方秒。
    #[must_use]
    pub const fn max_acceleration_meters_per_second_squared(&self) -> f64 {
        self.record.max_acceleration_meters_per_second_squared
    }

    /// 返回舒适减速度幅值，单位为米每二次方秒。
    #[must_use]
    pub const fn comfortable_deceleration_meters_per_second_squared(&self) -> f64 {
        self.record
            .comfortable_deceleration_meters_per_second_squared
    }

    /// 返回紧急减速度幅值，单位为米每二次方秒。
    #[must_use]
    pub const fn emergency_deceleration_meters_per_second_squared(&self) -> f64 {
        self.record.emergency_deceleration_meters_per_second_squared
    }
}

/// Canonical LIR 中一条准入规则的有类型静态目标。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CanonicalAccessTarget {
    /// 单条车道图边。
    LaneEdge(LaneEdgeOrdinal),
    /// 车道组；运行时投影可按预编译覆盖关系展开到边。
    LaneGroup(LaneGroupOrdinal),
    /// 道路区段；运行时投影可按预编译覆盖关系展开到边。
    RoadSection(RoadSectionOrdinal),
    /// 保持独立准入平面的机动路径。
    ManeuverPath(ManeuverPathOrdinal),
}

/// 一条准入规则所携带法规来源的借用视图。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CanonicalAccessRegulationView<'a> {
    jurisdiction: &'a str,
    version: &'a str,
    source: Option<&'a str>,
}

impl<'a> CanonicalAccessRegulationView<'a> {
    /// 返回法域。
    #[must_use]
    pub const fn jurisdiction(self) -> &'a str {
        self.jurisdiction
    }

    /// 返回法规版本。
    #[must_use]
    pub const fn version(self) -> &'a str {
        self.version
    }

    /// 返回可选来源说明。
    #[must_use]
    pub const fn source(self) -> Option<&'a str> {
        self.source
    }
}

impl CanonicalAccessRuleView<'_> {
    /// 返回规则目标；边平面与机动路径平面分别组合，不能跨平面相互覆盖。
    #[must_use]
    pub const fn target(&self) -> CanonicalAccessTarget {
        match self.record.target {
            LirAccessTarget::LaneEdge(target) => CanonicalAccessTarget::LaneEdge(target),
            LirAccessTarget::LaneGroup(target) => CanonicalAccessTarget::LaneGroup(target),
            LirAccessTarget::RoadSection(target) => CanonicalAccessTarget::RoadSection(target),
            LirAccessTarget::ManeuverPath(target) => CanonicalAccessTarget::ManeuverPath(target),
        }
    }

    /// 返回规则在当前准入平面内的允许或拒绝效果。
    #[must_use]
    pub const fn effect(&self) -> AccessEffect {
        self.record.effect
    }

    /// 返回按规范类别序号排序、去重后的非空类别集合。
    #[must_use]
    pub fn participant_classes(&self) -> &[ParticipantClassOrdinal] {
        &self.lir.access_rule_participant_classes[self.record.participant_classes.as_usize_range()]
    }

    /// 返回可选法规来源；该信息不参与规则优先级计算。
    #[must_use]
    pub fn regulation(&self) -> Option<CanonicalAccessRegulationView<'_>> {
        self.record
            .regulation
            .as_ref()
            .map(|regulation| CanonicalAccessRegulationView {
                jurisdiction: &regulation.jurisdiction,
                version: &regulation.version,
                source: regulation.source.as_deref(),
            })
    }

    /// 返回类别与目标具体度相同后的显式优先级。
    #[must_use]
    pub const fn priority(&self) -> i32 {
        self.record.priority
    }
}

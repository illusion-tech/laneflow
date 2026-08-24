//! 准入领域 Canonical LIR 记录。

use laneflow_static_contract::{
    AccessEffect, AccessRuleId, AccessRuleOrdinal, LaneEdgeOrdinal, LaneGroupOrdinal,
    ManeuverPathOrdinal, ParticipantClassId, ParticipantClassOrdinal, RoadSectionOrdinal,
    VehicleProfileId, VehicleProfileOrdinal,
};

use crate::arena::TableRange;

use super::LirIdentityField;

pub(crate) struct LirParticipantClass {
    pub(crate) ordinal: ParticipantClassOrdinal,
    pub(crate) stable_id: ParticipantClassId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) parent: Option<ParticipantClassOrdinal>,
    pub(crate) depth: u32,
    pub(crate) subtree_enter: u32,
    pub(crate) subtree_exit: u32,
}

pub(crate) struct LirVehicleProfile {
    pub(crate) ordinal: VehicleProfileOrdinal,
    pub(crate) stable_id: VehicleProfileId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) participant_class: ParticipantClassOrdinal,
    pub(crate) length_meters: f64,
    pub(crate) desired_speed_meters_per_second: f64,
    pub(crate) min_gap_meters: f64,
    pub(crate) time_headway_seconds: f64,
    pub(crate) max_acceleration_meters_per_second_squared: f64,
    pub(crate) comfortable_deceleration_meters_per_second_squared: f64,
    pub(crate) emergency_deceleration_meters_per_second_squared: f64,
}

#[derive(Clone, Copy)]
pub(crate) enum LirAccessTarget {
    LaneEdge(LaneEdgeOrdinal),
    LaneGroup(LaneGroupOrdinal),
    RoadSection(RoadSectionOrdinal),
    ManeuverPath(ManeuverPathOrdinal),
}

pub(crate) struct LirAccessRegulation {
    pub(crate) jurisdiction: Box<str>,
    pub(crate) version: Box<str>,
    pub(crate) source: Option<Box<str>>,
}

pub(crate) struct LirAccessRule {
    pub(crate) ordinal: AccessRuleOrdinal,
    pub(crate) stable_id: AccessRuleId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) target: LirAccessTarget,
    pub(crate) effect: AccessEffect,
    pub(crate) participant_classes: TableRange<ParticipantClassOrdinal>,
    pub(crate) regulation: Option<LirAccessRegulation>,
    pub(crate) priority: i32,
}

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
    pub(crate) length_mm: u32,
    pub(crate) desired_speed_mm_s: u32,
    pub(crate) min_gap_mm: u32,
    pub(crate) time_headway_seconds: f32,
    pub(crate) max_acceleration_meters_per_second_squared: f32,
    pub(crate) comfortable_deceleration_meters_per_second_squared: f32,
    pub(crate) emergency_deceleration_meters_per_second_squared: f32,
}

#[derive(Clone, Copy)]
pub(crate) enum LirAccessTarget {
    LaneEdge(LaneEdgeOrdinal),
    LaneGroup(LaneGroupOrdinal),
    RoadSection(RoadSectionOrdinal),
    ManeuverPath(ManeuverPathOrdinal),
}

pub(crate) type LirAccessRegulation = crate::RegulationIdentity;

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

use super::{FreezeEnv, LirAccessCounts, push_lir_identity, relation_range};
use crate::DiagnosticBundle;
use crate::mir::MirAccessTarget;
use laneflow_static_contract::FieldTag;

pub(super) struct AccessClassParts {
    pub participant_classes: Vec<LirParticipantClass>,
    pub vehicle_profiles: Vec<LirVehicleProfile>,
}

pub(super) struct AccessRuleParts {
    pub access_rules: Vec<LirAccessRule>,
    pub access_rule_participant_classes: Vec<ParticipantClassOrdinal>,
}

pub(super) fn freeze_classes(
    env: &mut FreezeEnv<'_>,
    counts: &LirAccessCounts,
) -> Result<AccessClassParts, DiagnosticBundle> {
    let mut participant_classes = Vec::with_capacity(env.capacity(counts.participant_classes)?);
    for mir_key in env
        .orders
        .participant_classes
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let participant_class = &env.mir.participant_classes[mir_key.index()];
        let identity_range = push_lir_identity(
            env.identity_fields,
            env.identity_field_bytes,
            FieldTag::ParticipantClassKey,
            &env.mir.modules[participant_class.module.index()].authoring_namespace_id,
            &participant_class.stable_key,
            None,
            env.limits,
            env.primary_span.clone(),
        )?;
        participant_classes.push(LirParticipantClass {
            ordinal: env.orders.participant_classes.ordinal(mir_key),
            stable_id: participant_class.stable_id,
            identity_fields: identity_range,
            parent: participant_class
                .parent
                .map(|parent| env.orders.participant_classes.ordinal(parent)),
            depth: participant_class.depth,
            subtree_enter: participant_class.subtree_enter,
            subtree_exit: participant_class.subtree_exit,
        });
    }
    close_class_intervals_in_lir_order(&mut participant_classes);

    let mut vehicle_profiles = Vec::with_capacity(env.capacity(counts.vehicle_profiles)?);
    for mir_key in env
        .orders
        .vehicle_profiles
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let profile = &env.mir.vehicle_profiles[mir_key.index()];
        let identity_range = push_lir_identity(
            env.identity_fields,
            env.identity_field_bytes,
            FieldTag::VehicleProfileKey,
            &env.mir.modules[profile.module.index()].authoring_namespace_id,
            &profile.stable_key,
            None,
            env.limits,
            env.primary_span.clone(),
        )?;
        vehicle_profiles.push(LirVehicleProfile {
            ordinal: env.orders.vehicle_profiles.ordinal(mir_key),
            stable_id: profile.stable_id,
            identity_fields: identity_range,
            participant_class: env
                .orders
                .participant_classes
                .ordinal(profile.participant_class),
            length_mm: profile.length_mm,
            desired_speed_mm_s: profile.desired_speed_mm_s,
            min_gap_mm: profile.min_gap_mm,
            time_headway_seconds: profile.time_headway_seconds,
            max_acceleration_meters_per_second_squared: profile
                .max_acceleration_meters_per_second_squared,
            comfortable_deceleration_meters_per_second_squared: profile
                .comfortable_deceleration_meters_per_second_squared,
            emergency_deceleration_meters_per_second_squared: profile
                .emergency_deceleration_meters_per_second_squared,
        });
    }

    Ok(AccessClassParts {
        participant_classes,
        vehicle_profiles,
    })
}

/// `ParticipantClass` 的 HIR 闭包基于来源遍历顺序；LIR ordinal 则按稳定身份排序。
/// 父引用完成 ordinal 重映射后必须在最终顺序上重建 DFS 区间，否则合法的多根分类森林
/// 会携带与 LFCA 行顺序不一致的 `depth/subtree` 派生值。
fn close_class_intervals_in_lir_order(classes: &mut [LirParticipantClass]) {
    const NONE: u32 = u32::MAX;

    let mut first_child = vec![NONE; classes.len()];
    let mut next_sibling = vec![NONE; classes.len()];
    let mut first_root = NONE;
    for (index, class) in classes.iter().enumerate() {
        let raw = u32::try_from(index).expect("participant class LIR ordinal is u32-bounded");
        if let Some(parent) = class.parent {
            next_sibling[index] = first_child[parent.index()];
            first_child[parent.index()] = raw;
        } else {
            next_sibling[index] = first_root;
            first_root = raw;
        }
    }

    let mut stack = Vec::with_capacity(classes.len().saturating_mul(2));
    let mut root = first_root;
    while root != NONE {
        stack.push((root, false));
        root = next_sibling[usize::try_from(root).expect("u32 fits usize")];
    }
    let mut euler = 0_u32;
    while let Some((raw, exiting)) = stack.pop() {
        let index = usize::try_from(raw).expect("u32 fits usize");
        if exiting {
            classes[index].subtree_exit = euler;
            continue;
        }
        classes[index].depth = classes[index]
            .parent
            .map_or(0, |parent| classes[parent.index()].depth.saturating_add(1));
        classes[index].subtree_enter = euler;
        euler = euler
            .checked_add(1)
            .expect("participant class count is u32-bounded");
        stack.push((raw, true));
        let mut child = first_child[index];
        while child != NONE {
            stack.push((child, false));
            child = next_sibling[usize::try_from(child).expect("u32 fits usize")];
        }
    }
}

pub(super) fn freeze_rules(
    env: &mut FreezeEnv<'_>,
    counts: &LirAccessCounts,
) -> Result<AccessRuleParts, DiagnosticBundle> {
    let mut access_rules = Vec::with_capacity(env.capacity(counts.access_rules)?);
    let mut access_rule_participant_classes =
        Vec::with_capacity(env.capacity(counts.rule_class_references)?);
    for mir_key in env
        .orders
        .access_rules
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let rule = &env.mir.access_rules[mir_key.index()];
        let identity_range = push_lir_identity(
            env.identity_fields,
            env.identity_field_bytes,
            FieldTag::AccessRuleKey,
            &env.mir.modules[rule.module.index()].authoring_namespace_id,
            &rule.stable_key,
            None,
            env.limits,
            env.primary_span.clone(),
        )?;
        let class_start = access_rule_participant_classes.len();
        access_rule_participant_classes.extend(
            env.mir.access_rule_participant_classes[rule.participant_classes.as_usize_range()]
                .iter()
                .map(|selector| {
                    env.orders
                        .participant_classes
                        .ordinal(selector.participant_class)
                }),
        );
        access_rule_participant_classes[class_start..].sort_unstable();
        let target = match rule.target {
            MirAccessTarget::LaneEdge(target) => {
                LirAccessTarget::LaneEdge(env.orders.lane_edges.ordinal(target))
            }
            MirAccessTarget::LaneGroup(target) => {
                LirAccessTarget::LaneGroup(env.orders.lane_groups.ordinal(target))
            }
            MirAccessTarget::RoadSection(target) => {
                LirAccessTarget::RoadSection(env.orders.road_sections.ordinal(target))
            }
            MirAccessTarget::ManeuverPath(target) => {
                LirAccessTarget::ManeuverPath(env.orders.maneuver_paths.ordinal(target))
            }
        };
        access_rules.push(LirAccessRule {
            ordinal: env.orders.access_rules.ordinal(mir_key),
            stable_id: rule.stable_id,
            identity_fields: identity_range,
            target,
            effect: rule.effect,
            participant_classes: relation_range(
                class_start,
                access_rule_participant_classes.len(),
                env.limits,
                env.primary_span.clone(),
            )?,
            regulation: rule
                .regulation
                .as_ref()
                .map(|regulation| LirAccessRegulation {
                    jurisdiction: regulation.jurisdiction.as_ref().into(),
                    version: regulation.version.as_ref().into(),
                    source: regulation.source.as_deref().map(Into::into),
                }),
            priority: rule.priority,
        });
    }

    Ok(AccessRuleParts {
        access_rules,
        access_rule_participant_classes,
    })
}

//! 信号领域 Canonical LIR 记录。

use laneflow_static_contract::{
    ManeuverGateOrdinal, SignalAspect, SignalControllerId, SignalControllerOrdinal, SignalGroupId,
    SignalGroupOrdinal, SignalPhaseId, SignalPhaseOrdinal,
};

use crate::arena::TableRange;

use super::LirIdentityField;

#[derive(Clone, Copy)]
pub(crate) enum LirSignalControl {
    Group(SignalGroupOrdinal),
    None,
}

pub(crate) struct LirSignalGroup {
    pub(crate) ordinal: SignalGroupOrdinal,
    pub(crate) stable_id: SignalGroupId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) controller: SignalControllerOrdinal,
    pub(crate) maneuver_gates: TableRange<ManeuverGateOrdinal>,
}

pub(crate) struct LirSignalController {
    pub(crate) ordinal: SignalControllerOrdinal,
    pub(crate) stable_id: SignalControllerId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) offset_ms: u64,
    pub(crate) cycle_duration_ms: u64,
    pub(crate) signal_groups: TableRange<SignalGroupOrdinal>,
    pub(crate) phases: TableRange<SignalPhaseOrdinal>,
}

pub(crate) struct LirSignalPhase {
    pub(crate) ordinal: SignalPhaseOrdinal,
    pub(crate) stable_id: SignalPhaseId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) controller: SignalControllerOrdinal,
    pub(crate) duration_ms: u64,
    pub(crate) states: TableRange<LirSignalPhaseState>,
}

pub(crate) struct LirSignalPhaseState {
    pub(crate) signal_group: SignalGroupOrdinal,
    pub(crate) aspect: SignalAspect,
}

use super::{FreezeEnv, LirSignalCounts, push_identity_field, push_lir_identity, relation_range};
use crate::DiagnosticBundle;
use crate::arena::ArenaKey;
use crate::mir::{MirSignalControllerGroup, MirSignalPhaseKey, MirSignalPhaseState};
use laneflow_static_contract::FieldTag;

pub(super) struct SignalParts {
    pub signal_groups: Vec<LirSignalGroup>,
    pub signal_group_maneuver_gates: Vec<ManeuverGateOrdinal>,
    pub signal_controllers: Vec<LirSignalController>,
    pub signal_controller_groups: Vec<SignalGroupOrdinal>,
    pub signal_controller_phases: Vec<SignalPhaseOrdinal>,
    pub signal_phases: Vec<LirSignalPhase>,
    pub signal_phase_states: Vec<LirSignalPhaseState>,
    pub signal_controller_group_mir_rows: Vec<ArenaKey<MirSignalControllerGroup>>,
    pub signal_phase_state_mir_rows: Vec<ArenaKey<MirSignalPhaseState>>,
}

pub(super) fn freeze(
    env: &mut FreezeEnv<'_>,
    counts: &LirSignalCounts,
) -> Result<SignalParts, DiagnosticBundle> {
    let mut signal_groups = Vec::with_capacity(env.capacity(counts.groups)?);
    let mut signal_group_maneuver_gates =
        Vec::with_capacity(env.capacity(counts.controlled_gates)?);
    for mir_key in env
        .orders
        .signal_groups
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let group = &env.mir.signal_groups[mir_key.index()];
        let identity_range = push_lir_identity(
            env.identity_fields,
            env.identity_field_bytes,
            FieldTag::SignalGroupKey,
            &env.mir.modules[group.module.index()].authoring_namespace_id,
            &group.stable_key,
            None,
            env.limits,
            env.primary_span.clone(),
        )?;
        let gate_start = signal_group_maneuver_gates.len();
        signal_group_maneuver_gates.extend(
            env.mir.signal_group_maneuver_gates[group.maneuver_gates.as_usize_range()]
                .iter()
                .map(|member| env.orders.maneuver_gates.ordinal(member.maneuver_gate)),
        );
        signal_group_maneuver_gates[gate_start..].sort_unstable();
        signal_groups.push(LirSignalGroup {
            ordinal: env.orders.signal_groups.ordinal(mir_key),
            stable_id: group.stable_id,
            identity_fields: identity_range,
            controller: env.orders.signal_controllers.ordinal(group.controller),
            maneuver_gates: relation_range(
                gate_start,
                signal_group_maneuver_gates.len(),
                env.limits,
                env.primary_span.clone(),
            )?,
        });
    }

    let mut signal_controllers = Vec::with_capacity(env.capacity(counts.controllers)?);
    let mut signal_controller_groups = Vec::with_capacity(env.capacity(counts.controller_groups)?);
    let mut signal_controller_group_mir_rows: Vec<ArenaKey<MirSignalControllerGroup>> =
        Vec::with_capacity(env.capacity(counts.controller_groups)?);
    let mut signal_controller_phases = Vec::with_capacity(env.capacity(counts.phases)?);
    for mir_key in env
        .orders
        .signal_controllers
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let controller = &env.mir.signal_controllers[mir_key.index()];
        let identity_range = push_lir_identity(
            env.identity_fields,
            env.identity_field_bytes,
            FieldTag::SignalControllerKey,
            &env.mir.modules[controller.module.index()].authoring_namespace_id,
            &controller.stable_key,
            None,
            env.limits,
            env.primary_span.clone(),
        )?;
        let group_start = signal_controller_groups.len();
        let permutation_start = signal_controller_group_mir_rows.len();
        signal_controller_group_mir_rows.extend(controller.signal_groups.as_usize_range().map(
            |index| {
                ArenaKey::from_raw(
                    u32::try_from(index).expect("LIR precheck proved every MIR key fits u32"),
                )
            },
        ));
        signal_controller_group_mir_rows[permutation_start..].sort_unstable_by_key(|mir_row| {
            let member = &env.mir.signal_controller_groups[mir_row.index()];
            (
                env.orders.signal_groups.ordinal(member.signal_group),
                mir_row.raw(),
            )
        });
        // 集合语义只排序这一份 MIR 行地址；语义目标和来源随后都借用此排列。
        signal_controller_groups.extend(
            signal_controller_group_mir_rows[permutation_start..]
                .iter()
                .map(|mir_row| {
                    let member = &env.mir.signal_controller_groups[mir_row.index()];
                    env.orders.signal_groups.ordinal(member.signal_group)
                }),
        );
        debug_assert_eq!(group_start, permutation_start);
        let phase_start = signal_controller_phases.len();
        for phase_index in controller.phases.as_usize_range() {
            signal_controller_phases.push(env.orders.signal_phases.ordinal(
                MirSignalPhaseKey::from_raw(
                    u32::try_from(phase_index).expect("MIR range prevalidated as u32"),
                ),
            ));
        }
        signal_controllers.push(LirSignalController {
            ordinal: env.orders.signal_controllers.ordinal(mir_key),
            stable_id: controller.stable_id,
            identity_fields: identity_range,
            offset_ms: controller.offset_ms,
            cycle_duration_ms: controller.cycle_duration_ms,
            signal_groups: relation_range(
                group_start,
                signal_controller_groups.len(),
                env.limits,
                env.primary_span.clone(),
            )?,
            // 相位顺序就是固定时制程序顺序，不能按 ordinal 再排序。
            phases: relation_range(
                phase_start,
                signal_controller_phases.len(),
                env.limits,
                env.primary_span.clone(),
            )?,
        });
    }

    let mut signal_phases = Vec::with_capacity(env.capacity(counts.phases)?);
    let mut signal_phase_states = Vec::with_capacity(env.capacity(counts.phase_states)?);
    let mut signal_phase_state_mir_rows: Vec<ArenaKey<MirSignalPhaseState>> =
        Vec::with_capacity(env.capacity(counts.phase_states)?);
    for mir_key in env
        .orders
        .signal_phases
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let phase = &env.mir.signal_phases[mir_key.index()];
        let identity_start = env.identity_fields.len();
        for (tag, value) in [
            (
                FieldTag::AuthoringNamespaceId,
                env.mir.modules[phase.module.index()]
                    .authoring_namespace_id
                    .as_bytes(),
            ),
            (
                FieldTag::SignalControllerStableId,
                env.mir.signal_controllers[phase.controller.index()]
                    .stable_id
                    .as_untyped()
                    .as_bytes(),
            ),
            (FieldTag::PhaseKey, phase.stable_key.as_bytes()),
        ] {
            push_identity_field(
                env.identity_fields,
                env.identity_field_bytes,
                tag,
                value,
                env.limits,
                env.primary_span.clone(),
            )?;
        }
        let state_start = signal_phase_states.len();
        let permutation_start = signal_phase_state_mir_rows.len();
        signal_phase_state_mir_rows.extend(phase.states.as_usize_range().map(|index| {
            ArenaKey::from_raw(
                u32::try_from(index).expect("LIR precheck proved every MIR key fits u32"),
            )
        }));
        signal_phase_state_mir_rows[permutation_start..].sort_unstable_by_key(|mir_row| {
            let state = &env.mir.signal_phase_states[mir_row.index()];
            (
                env.orders.signal_groups.ordinal(state.signal_group),
                mir_row.raw(),
            )
        });
        // 相位状态与控制器组表共享 LIR signal-group ordinal 轴。
        signal_phase_states.extend(signal_phase_state_mir_rows[permutation_start..].iter().map(
            |mir_row| {
                let state = &env.mir.signal_phase_states[mir_row.index()];
                LirSignalPhaseState {
                    signal_group: env.orders.signal_groups.ordinal(state.signal_group),
                    aspect: state.aspect,
                }
            },
        ));
        debug_assert_eq!(state_start, permutation_start);
        signal_phases.push(LirSignalPhase {
            ordinal: env.orders.signal_phases.ordinal(mir_key),
            stable_id: phase.stable_id,
            identity_fields: relation_range(
                identity_start,
                env.identity_fields.len(),
                env.limits,
                env.primary_span.clone(),
            )?,
            controller: env.orders.signal_controllers.ordinal(phase.controller),
            duration_ms: phase.duration_ms,
            states: relation_range(
                state_start,
                signal_phase_states.len(),
                env.limits,
                env.primary_span.clone(),
            )?,
        });
    }

    Ok(SignalParts {
        signal_groups,
        signal_group_maneuver_gates,
        signal_controllers,
        signal_controller_groups,
        signal_controller_phases,
        signal_phases,
        signal_phase_states,
        signal_controller_group_mir_rows,
        signal_phase_state_mir_rows,
    })
}

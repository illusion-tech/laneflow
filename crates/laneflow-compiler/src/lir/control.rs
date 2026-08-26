//! 控制领域 Canonical LIR 记录：停止线、机动门与等待区。

use laneflow_static_contract::{
    LaneEdgeOrdinal, ManeuverGateId, ManeuverGateOrdinal, ManeuverPathOrdinal, StopLineId,
    StopLineOrdinal, WaitingZoneId, WaitingZoneOrdinal,
};

use crate::arena::TableRange;

use super::{LirIdentityField, LirSignalControl};

pub(crate) struct LirStopLine {
    pub(crate) ordinal: StopLineOrdinal,
    pub(crate) stable_id: StopLineId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) lane_edge: LaneEdgeOrdinal,
    pub(crate) maneuver_gates: TableRange<ManeuverGateOrdinal>,
}

pub(crate) struct LirManeuverGate {
    pub(crate) ordinal: ManeuverGateOrdinal,
    pub(crate) stable_id: ManeuverGateId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) maneuver_path: ManeuverPathOrdinal,
    pub(crate) transition_index: u32,
    pub(crate) stop_line: StopLineOrdinal,
    pub(crate) signal_control: LirSignalControl,
}

pub(crate) struct LirWaitingZone {
    pub(crate) ordinal: WaitingZoneOrdinal,
    pub(crate) stable_id: WaitingZoneId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) maneuver_path: ManeuverPathOrdinal,
    pub(crate) entry_gate: ManeuverGateOrdinal,
    pub(crate) release_gate: ManeuverGateOrdinal,
    pub(crate) max_occupancy: u32,
}

use super::{FreezeEnv, LirControlCounts, push_identity_field, push_lir_identity, relation_range};
use crate::DiagnosticBundle;
use crate::mir::MirSignalControl;
use laneflow_static_contract::FieldTag;

pub(super) struct ControlParts {
    pub stop_lines: Vec<LirStopLine>,
    pub stop_line_maneuver_gates: Vec<ManeuverGateOrdinal>,
    pub maneuver_gates: Vec<LirManeuverGate>,
    pub waiting_zones: Vec<LirWaitingZone>,
}

pub(super) fn freeze(
    env: &mut FreezeEnv<'_>,
    counts: &LirControlCounts,
) -> Result<ControlParts, DiagnosticBundle> {
    let mut stop_lines = Vec::with_capacity(env.capacity(counts.stop_lines)?);
    let mut stop_line_maneuver_gates =
        Vec::with_capacity(env.capacity(counts.stop_line_maneuver_gates)?);
    for mir_key in env
        .orders
        .stop_lines
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let stop_line = &env.mir.stop_lines[mir_key.index()];
        let identity_range = push_lir_identity(
            env.identity_fields,
            env.identity_field_bytes,
            FieldTag::StopLineKey,
            &env.mir.modules[stop_line.module.index()].authoring_namespace_id,
            &stop_line.stable_key,
            None,
            env.limits,
            env.primary_span.clone(),
        )?;
        let relation_start = stop_line_maneuver_gates.len();
        stop_line_maneuver_gates.extend(
            env.mir.stop_line_maneuver_gates[stop_line.maneuver_gates.as_usize_range()]
                .iter()
                .map(|member| env.orders.maneuver_gates.ordinal(member.maneuver_gate)),
        );
        // 共享静态路网要求同一停止线的门成员按 LIR 序号严格递增；MIR 仍按 stable_id
        // 排列，映射后必须再按序号冻结。
        stop_line_maneuver_gates[relation_start..].sort_unstable();
        stop_lines.push(LirStopLine {
            ordinal: env.orders.stop_lines.ordinal(mir_key),
            stable_id: stop_line.stable_id,
            identity_fields: identity_range,
            lane_edge: env.orders.lane_edges.ordinal(stop_line.lane_edge),
            maneuver_gates: relation_range(
                relation_start,
                stop_line_maneuver_gates.len(),
                env.limits,
                env.primary_span.clone(),
            )?,
        });
    }

    let mut maneuver_gates = Vec::with_capacity(env.capacity(counts.maneuver_gates)?);
    for mir_key in env
        .orders
        .maneuver_gates
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let gate = &env.mir.maneuver_gates[mir_key.index()];
        let identity_start = env.identity_fields.len();
        for (tag, value) in [
            (
                FieldTag::AuthoringNamespaceId,
                env.mir.modules[gate.module.index()]
                    .authoring_namespace_id
                    .as_bytes(),
            ),
            (
                FieldTag::ManeuverPathStableId,
                env.mir.maneuver_paths[gate.maneuver_path.index()]
                    .stable_id
                    .as_untyped()
                    .as_bytes(),
            ),
            (FieldTag::GateKey, gate.stable_key.as_bytes()),
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
        maneuver_gates.push(LirManeuverGate {
            ordinal: env.orders.maneuver_gates.ordinal(mir_key),
            stable_id: gate.stable_id,
            identity_fields: relation_range(
                identity_start,
                env.identity_fields.len(),
                env.limits,
                env.primary_span.clone(),
            )?,
            maneuver_path: env.orders.maneuver_paths.ordinal(gate.maneuver_path),
            transition_index: gate.transition_index,
            stop_line: env.orders.stop_lines.ordinal(gate.stop_line),
            signal_control: match gate.signal_control {
                MirSignalControl::Group { signal_group, .. } => {
                    LirSignalControl::Group(env.orders.signal_groups.ordinal(signal_group))
                }
                MirSignalControl::None => LirSignalControl::None,
            },
        });
    }

    let mut waiting_zones = Vec::with_capacity(env.capacity(counts.waiting_zones)?);
    for mir_key in env
        .orders
        .waiting_zones
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let waiting = &env.mir.waiting_zones[mir_key.index()];
        let identity_start = env.identity_fields.len();
        for (tag, value) in [
            (
                FieldTag::AuthoringNamespaceId,
                env.mir.modules[waiting.module.index()]
                    .authoring_namespace_id
                    .as_bytes(),
            ),
            (
                FieldTag::ManeuverPathStableId,
                env.mir.maneuver_paths[waiting.maneuver_path.index()]
                    .stable_id
                    .as_untyped()
                    .as_bytes(),
            ),
            (FieldTag::WaitingZoneKey, waiting.stable_key.as_bytes()),
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
        waiting_zones.push(LirWaitingZone {
            ordinal: env.orders.waiting_zones.ordinal(mir_key),
            stable_id: waiting.stable_id,
            identity_fields: relation_range(
                identity_start,
                env.identity_fields.len(),
                env.limits,
                env.primary_span.clone(),
            )?,
            maneuver_path: env.orders.maneuver_paths.ordinal(waiting.maneuver_path),
            entry_gate: env.orders.maneuver_gates.ordinal(waiting.entry_gate),
            release_gate: env.orders.maneuver_gates.ordinal(waiting.release_gate),
            max_occupancy: waiting.max_occupancy,
        });
    }

    Ok(ControlParts {
        stop_lines,
        stop_line_maneuver_gates,
        maneuver_gates,
        waiting_zones,
    })
}

//! 冲突领域 Canonical LIR 记录。

use laneflow_static_contract::{
    ConflictZoneId, ConflictZoneOrdinal, FieldTag, JunctionOrdinal, ManeuverGateOrdinal,
    ManeuverPathOrdinal, ParticipantStreamId, ParticipantStreamOrdinal,
};

use crate::DiagnosticBundle;
use crate::arena::{ArenaKey, TableRange};
use crate::mir::{MirConflictPassage, MirPathAnchorReference};

use super::{FreezeEnv, LirConflictCounts, LirIdentityField, push_lir_identity, relation_range};

pub(crate) struct LirConflictZone {
    pub(crate) ordinal: ConflictZoneOrdinal,
    pub(crate) stable_id: ConflictZoneId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) junction: JunctionOrdinal,
}

#[derive(Clone, Copy)]
pub(crate) enum LirPathAnchorReference {
    Gate(ManeuverGateOrdinal),
    EdgeBoundary(u32),
    Interior { path_edge_index: u32 },
}

#[derive(Clone, Copy)]
pub(crate) struct LirPathAnchor {
    pub(crate) reference: LirPathAnchorReference,
    pub(crate) progress_mm: Option<u32>,
}

pub(crate) struct LirConflictPassage {
    pub(crate) conflict_zone: ConflictZoneOrdinal,
    pub(crate) entry: LirPathAnchor,
    pub(crate) exit: LirPathAnchor,
    /// 从 entry 与同一路径 Gate 序列导出的覆盖 Gate；不进入 LFCA authority。
    pub(crate) admission_gate: ManeuverGateOrdinal,
}

pub(crate) struct LirParticipantStream {
    pub(crate) ordinal: ParticipantStreamOrdinal,
    pub(crate) stable_id: ParticipantStreamId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) junction: JunctionOrdinal,
    pub(crate) maneuver_path: ManeuverPathOrdinal,
    pub(crate) passages: TableRange<LirConflictPassage>,
}

pub(super) struct ConflictParts {
    pub conflict_zones: Vec<LirConflictZone>,
    pub participant_streams: Vec<LirParticipantStream>,
    pub conflict_passages: Vec<LirConflictPassage>,
    pub conflict_passage_mir_rows: Vec<ArenaKey<MirConflictPassage>>,
}

pub(super) fn freeze(
    env: &mut FreezeEnv<'_>,
    counts: &LirConflictCounts,
) -> Result<ConflictParts, DiagnosticBundle> {
    let mut participant_streams = Vec::with_capacity(env.capacity(counts.streams)?);
    let mut conflict_passages = Vec::with_capacity(env.capacity(counts.passages)?);
    let mut conflict_passage_mir_rows = Vec::with_capacity(env.capacity(counts.passages)?);

    for mir_key in env
        .orders
        .participant_streams
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let stream = &env.mir.participant_streams[mir_key.index()];
        let junction = &env.mir.junctions[stream.junction.index()];
        let identity_fields = push_lir_identity(
            env.identity_fields,
            env.identity_field_bytes,
            FieldTag::ParticipantStreamKey,
            &env.mir.modules[stream.module.index()].authoring_namespace_id,
            &stream.stable_key,
            Some((
                FieldTag::JunctionStableId,
                junction.stable_id.as_untyped().as_bytes(),
            )),
            env.limits,
            env.primary_span.clone(),
        )?;
        let passage_start = conflict_passages.len();
        for row_index in stream.passages.as_usize_range() {
            let raw = u32::try_from(row_index)
                .expect("LIR precheck proved every MIR relation row fits u32");
            let passage = &env.mir.conflict_passages[row_index];
            conflict_passage_mir_rows.push(ArenaKey::from_raw(raw));
            conflict_passages.push(LirConflictPassage {
                conflict_zone: env.orders.conflict_zones.ordinal(passage.conflict_zone),
                entry: freeze_anchor(env, &passage.entry.reference, passage.entry.progress_mm),
                exit: freeze_anchor(env, &passage.exit.reference, passage.exit.progress_mm),
                admission_gate: env.orders.maneuver_gates.ordinal(passage.admission_gate),
            });
        }
        participant_streams.push(LirParticipantStream {
            ordinal: env.orders.participant_streams.ordinal(mir_key),
            stable_id: stream.stable_id,
            identity_fields,
            junction: env.orders.junctions.ordinal(stream.junction),
            maneuver_path: env.orders.maneuver_paths.ordinal(stream.maneuver_path),
            passages: relation_range(
                passage_start,
                conflict_passages.len(),
                env.limits,
                env.primary_span.clone(),
            )?,
        });
    }

    // 反向表只用于证明 HIR→MIR 降阶未丢失 passage authority。最终
    // Canonical LIR/LFCA 不保存第二份可独立漂移的 zone→stream 表。
    let zone_capacity = env.capacity(counts.zones)?;
    let mut streams_by_zone = Vec::with_capacity(zone_capacity);
    for mir_key in env
        .orders
        .conflict_zones
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let zone = &env.mir.conflict_zones[mir_key.index()];
        streams_by_zone.push(Vec::<ParticipantStreamOrdinal>::with_capacity(
            usize::try_from(zone.participant_streams.len())
                .expect("u32 relation count fits usize on every supported target"),
        ));
    }
    for stream in &participant_streams {
        for passage in &conflict_passages[stream.passages.as_usize_range()] {
            streams_by_zone[passage.conflict_zone.raw() as usize].push(stream.ordinal);
        }
    }
    let mut conflict_zones = Vec::with_capacity(zone_capacity);
    let mut canonical_mir_streams = Vec::with_capacity(env.capacity(counts.max_zone_streams)?);
    for mir_key in env
        .orders
        .conflict_zones
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let zone = &env.mir.conflict_zones[mir_key.index()];
        let junction = &env.mir.junctions[zone.junction.index()];
        let identity_fields = push_lir_identity(
            env.identity_fields,
            env.identity_field_bytes,
            FieldTag::ConflictZoneKey,
            &env.mir.modules[zone.module.index()].authoring_namespace_id,
            &zone.stable_key,
            Some((
                FieldTag::JunctionStableId,
                junction.stable_id.as_untyped().as_bytes(),
            )),
            env.limits,
            env.primary_span.clone(),
        )?;
        let ordinal = env.orders.conflict_zones.ordinal(mir_key);
        let streams = &mut streams_by_zone[ordinal.raw() as usize];
        debug_assert_eq!(
            streams.len(),
            usize::try_from(zone.participant_streams.len())
                .expect("u32 relation count fits usize on every supported target")
        );
        streams.sort_unstable();
        streams.dedup();
        canonical_mir_streams.clear();
        canonical_mir_streams.extend(
            env.mir.conflict_zone_streams[zone.participant_streams.as_usize_range()]
                .iter()
                .map(|member| {
                    env.orders
                        .participant_streams
                        .ordinal(member.participant_stream)
                }),
        );
        canonical_mir_streams.sort_unstable();
        assert_eq!(
            streams.as_slice(),
            canonical_mir_streams.as_slice(),
            "MIR conflict-zone reverse closure diverged from participant-stream passages"
        );
        conflict_zones.push(LirConflictZone {
            ordinal,
            stable_id: zone.stable_id,
            identity_fields,
            junction: env.orders.junctions.ordinal(zone.junction),
        });
    }

    Ok(ConflictParts {
        conflict_zones,
        participant_streams,
        conflict_passages,
        conflict_passage_mir_rows,
    })
}

fn freeze_anchor(
    env: &FreezeEnv<'_>,
    reference: &MirPathAnchorReference,
    progress_mm: Option<u32>,
) -> LirPathAnchor {
    let reference = match *reference {
        MirPathAnchorReference::Gate(gate) => {
            LirPathAnchorReference::Gate(env.orders.maneuver_gates.ordinal(gate))
        }
        MirPathAnchorReference::EdgeBoundary(boundary_index) => {
            LirPathAnchorReference::EdgeBoundary(boundary_index)
        }
        MirPathAnchorReference::Interior { path_edge_index } => {
            LirPathAnchorReference::Interior { path_edge_index }
        }
    };
    LirPathAnchor {
        reference,
        progress_mm,
    }
}

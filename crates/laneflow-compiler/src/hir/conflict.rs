//! 冲突静态 HIR：冲突区、参与者流、规范路径锚点、passage 与反向成员闭包。

use std::collections::HashMap;
use std::sync::Arc;

use laneflow_static_contract::{
    ConflictZoneId, EntityKind, FieldTag, ParticipantStreamId, StableId128,
};

use crate::arena::{ArenaKeyOverflow, TableRange, TypedArena};
use crate::declaration::{PathAnchorDeclaration, TypedAstDeclaration, TypedAstEntityAddress};
use crate::diagnostic::DiagnosticCollector;
use crate::identity::{IdentityFieldInput, IdentityRegistry};
use crate::module::ResolvedSourceLocation;
use crate::{
    CompilationUnit, CompileLimitDimension, Diagnostic, DiagnosticBundle,
    RoadEditingInputViolation, RoadEditingSourceViolation, SourceLocation,
};

use super::{
    ConflictCounts, HirConflictZoneKey, HirConflictZoneTag, HirJunction, HirJunctionKey,
    HirLaneEdge, HirManeuverGate, HirManeuverGateKey, HirManeuverPath, HirManeuverPathEdge,
    HirManeuverPathGate, HirManeuverPathKey, HirModuleKey, HirMovement, HirParticipantStreamKey,
    HirParticipantStreamTag, HirWaitingZone, SymbolTable, arena_overflow, count_to_usize,
    declaration_header, derive_identity, resolve_reference,
};

/// 已闭合所属路口并派生反向参与者流集合的冲突区。
#[derive(Debug, PartialEq)]
pub(crate) struct HirConflictZone {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) source_address: TypedAstEntityAddress,
    pub(crate) stable_id: ConflictZoneId,
    pub(crate) junction: HirJunctionKey,
    pub(crate) junction_source_location: Option<ResolvedSourceLocation>,
    pub(crate) participant_streams: TableRange<HirConflictZoneStream>,
    pub(crate) source_span: SourceLocation,
}

/// 冲突区反向参与者流 CSR 中的一项。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HirConflictZoneStream {
    pub(crate) participant_stream: HirParticipantStreamKey,
}

/// LFCA anchor 的闭合三种引用形状。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HirPathAnchorReference {
    Gate(HirManeuverGateKey),
    EdgeBoundary(u32),
    Interior { path_edge_index: u32 },
}

/// 已解析、可按整数毫米路径位置比较的路径锚点。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HirPathAnchor {
    pub(crate) reference: HirPathAnchorReference,
    pub(crate) progress_mm: Option<u32>,
    pub(crate) position_edge_index: u32,
    pub(crate) position_progress_mm: u32,
    pub(crate) source_location: ResolvedSourceLocation,
}

impl HirPathAnchor {
    fn position(&self) -> (u32, u32) {
        (self.position_edge_index, self.position_progress_mm)
    }
}

/// 一个参与者流穿越一个冲突区的规范 owner-local 行。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HirConflictPassage {
    pub(crate) conflict_zone: HirConflictZoneKey,
    pub(crate) entry: HirPathAnchor,
    pub(crate) exit: HirPathAnchor,
    pub(crate) admission_gate: HirManeuverGateKey,
    pub(crate) conflict_zone_source_location: ResolvedSourceLocation,
    pub(crate) source_location: ResolvedSourceLocation,
}

/// 已闭合 Junction/Path、规范 passages 与派生 admission Gate 的参与者流。
#[derive(Debug, PartialEq)]
pub(crate) struct HirParticipantStream {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: ParticipantStreamId,
    pub(crate) junction: HirJunctionKey,
    pub(crate) junction_source_location: Option<ResolvedSourceLocation>,
    pub(crate) maneuver_path: HirManeuverPathKey,
    pub(crate) maneuver_path_source_location: Option<ResolvedSourceLocation>,
    pub(crate) passages: TableRange<HirConflictPassage>,
    pub(crate) source_span: SourceLocation,
}

#[derive(Default)]
pub(crate) struct ConflictHir {
    pub(crate) conflict_zones: Box<[HirConflictZone]>,
    pub(crate) participant_streams: Box<[HirParticipantStream]>,
    pub(crate) conflict_passages: Box<[HirConflictPassage]>,
    pub(crate) conflict_zone_streams: Box<[HirConflictZoneStream]>,
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(crate) fn build_conflict_hir(
    unit: &CompilationUnit,
    counts: &ConflictCounts,
    module_lookup: &HashMap<Arc<str>, HirModuleKey>,
    junctions: &[HirJunction],
    movements: &[HirMovement],
    maneuver_paths: &[HirManeuverPath],
    maneuver_path_edges: &[HirManeuverPathEdge],
    lane_edges: &[HirLaneEdge],
    maneuver_gates: &[HirManeuverGate],
    maneuver_path_gates: &[HirManeuverPathGate],
    waiting_zones: &[HirWaitingZone],
    identities: &mut IdentityRegistry,
) -> Result<ConflictHir, DiagnosticBundle> {
    if counts.entity_count() == 0 {
        return Ok(ConflictHir::default());
    }

    let zone_capacity = count_to_usize(counts.zones, &unit.limits)?;
    let stream_capacity = count_to_usize(counts.streams, &unit.limits)?;
    let passage_capacity = count_to_usize(counts.passages, &unit.limits)?;
    let mut zones = TypedArena::<HirConflictZoneTag, HirConflictZone>::with_capacity(zone_capacity);
    let mut streams =
        TypedArena::<HirParticipantStreamTag, HirParticipantStream>::with_capacity(stream_capacity);
    let mut zone_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|value| matches!(value, TypedAstDeclaration::ConflictZone(_)))
            .count()
    }));
    let mut zone_sources = Vec::with_capacity(zone_capacity);
    let mut stream_sources = Vec::with_capacity(stream_capacity);

    for (module_index, source_module) in unit.modules.iter().enumerate() {
        let module_key = HirModuleKey::from_raw(
            u32::try_from(module_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        let module_order = u32::try_from(module_index).unwrap_or(u32::MAX);
        let mut indices = source_module
            .declarations
            .iter()
            .enumerate()
            .filter_map(|(index, value)| {
                matches!(
                    value,
                    TypedAstDeclaration::ConflictZone(_)
                        | TypedAstDeclaration::ParticipantStream(_)
                )
                .then_some(index)
            })
            .collect::<Vec<_>>();
        indices.sort_unstable_by(|left, right| {
            let left = declaration_header(&source_module.declarations[*left]);
            let right = declaration_header(&source_module.declarations[*right]);
            (left.entity_kind.code(), &left.source_address)
                .cmp(&(right.entity_kind.code(), &right.source_address))
        });
        for index in indices {
            let declaration_index = u32::try_from(index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?;
            match &source_module.declarations[index] {
                TypedAstDeclaration::ConflictZone(source) => {
                    let key = zones
                        .push(HirConflictZone {
                            module: module_key,
                            stable_key: Arc::clone(&source.header.stable_key),
                            source_address: source.header.source_address.clone(),
                            stable_id: ConflictZoneId::from_untyped(StableId128::ZERO),
                            junction: HirJunctionKey::from_raw(0),
                            junction_source_location: None,
                            participant_streams: TableRange::empty(),
                            source_span: source.header.span.clone(),
                        })
                        .map_err(|overflow| {
                            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                        })?;
                    zone_symbols.insert(module_key, source.header.source_address.clone(), key);
                    zone_sources.push((module_order, declaration_index, key));
                }
                TypedAstDeclaration::ParticipantStream(source) => {
                    let key = streams
                        .push(HirParticipantStream {
                            module: module_key,
                            stable_key: Arc::clone(&source.header.stable_key),
                            stable_id: ParticipantStreamId::from_untyped(StableId128::ZERO),
                            junction: HirJunctionKey::from_raw(0),
                            junction_source_location: None,
                            maneuver_path: HirManeuverPathKey::from_raw(0),
                            maneuver_path_source_location: None,
                            passages: TableRange::empty(),
                            source_span: source.header.span.clone(),
                        })
                        .map_err(|overflow| {
                            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                        })?;
                    stream_sources.push((module_order, declaration_index, key));
                }
                _ => unreachable!("conflict source filter admitted unrelated declaration"),
            }
        }
    }

    let mut junction_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|value| matches!(value, TypedAstDeclaration::Junction(_)))
            .count()
    }));
    for (index, junction) in junctions.iter().enumerate() {
        junction_symbols.insert(
            junction.module,
            TypedAstEntityAddress::module_scoped(Arc::clone(&junction.stable_key)),
            HirJunctionKey::from_raw(u32::try_from(index).map_err(|_| {
                arena_overflow(
                    ArenaKeyOverflow,
                    &unit.limits,
                    Some(junction.source_span.clone()),
                )
            })?),
        );
    }
    let mut path_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|value| matches!(value, TypedAstDeclaration::ManeuverPath(_)))
            .count()
    }));
    for (index, path) in maneuver_paths.iter().enumerate() {
        path_symbols.insert(
            path.module,
            path.source_address.clone(),
            HirManeuverPathKey::from_raw(u32::try_from(index).map_err(|_| {
                arena_overflow(
                    ArenaKeyOverflow,
                    &unit.limits,
                    Some(path.source_span.clone()),
                )
            })?),
        );
    }
    let mut gate_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|value| matches!(value, TypedAstDeclaration::ManeuverGate(_)))
            .count()
    }));
    for (index, gate) in maneuver_gates.iter().enumerate() {
        gate_symbols.insert(
            gate.module,
            gate.source_address.clone(),
            HirManeuverGateKey::from_raw(u32::try_from(index).map_err(|_| {
                arena_overflow(
                    ArenaKeyOverflow,
                    &unit.limits,
                    Some(gate.source_span.clone()),
                )
            })?),
        );
    }

    let mut diagnostics =
        DiagnosticCollector::new(unit.limits.value(CompileLimitDimension::DiagnosticCount));
    for (module_order, declaration_index, zone_key) in &zone_sources {
        let source_module = &unit.modules[*module_order as usize];
        let TypedAstDeclaration::ConflictZone(source) =
            &source_module.declarations[*declaration_index as usize]
        else {
            unreachable!("canonical ConflictZone source changed kind")
        };
        let Some(junction_key) = resolve_reference(
            module_lookup,
            &junction_symbols,
            &source.junction,
            EntityKind::ConflictZone,
            &source.header,
            *module_order,
            &mut diagnostics,
        ) else {
            continue;
        };
        let junction_id = junctions[junction_key.index()].stable_id.into_untyped();
        let fields = [
            IdentityFieldInput::new(
                FieldTag::AuthoringNamespaceId,
                source_module
                    .descriptor()
                    .authoring_namespace_id()
                    .as_bytes(),
            ),
            IdentityFieldInput::new(
                FieldTag::ConflictZoneKey,
                source.header.stable_key.as_bytes(),
            ),
            IdentityFieldInput::new(FieldTag::JunctionStableId, junction_id.as_bytes()),
        ];
        let stable_id = ConflictZoneId::from_untyped(derive_identity(
            unit,
            identities,
            *module_order as usize,
            EntityKind::ConflictZone,
            &source.header.stable_key,
            &source.header.span,
            &fields,
        )?);
        let zone = zones.get_mut(*zone_key);
        zone.stable_id = stable_id;
        zone.junction = junction_key;
        zone.junction_source_location =
            Some(unit.resolve_source_location_for_module(*module_order, &source.junction.span)?);
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    let mut passages = Vec::with_capacity(passage_capacity);
    let mut passage_owners = Vec::with_capacity(passage_capacity);
    for (module_order, declaration_index, stream_key) in &stream_sources {
        let source_module = &unit.modules[*module_order as usize];
        let TypedAstDeclaration::ParticipantStream(source) =
            &source_module.declarations[*declaration_index as usize]
        else {
            unreachable!("canonical ParticipantStream source changed kind")
        };
        let junction = resolve_reference(
            module_lookup,
            &junction_symbols,
            &source.junction,
            EntityKind::ParticipantStream,
            &source.header,
            *module_order,
            &mut diagnostics,
        );
        let path = resolve_reference(
            module_lookup,
            &path_symbols,
            &source.maneuver_path,
            EntityKind::ParticipantStream,
            &source.header,
            *module_order,
            &mut diagnostics,
        );
        let (Some(junction), Some(path)) = (junction, path) else {
            continue;
        };
        let path_junction = movements[maneuver_paths[path.index()].movement.index()].junction;
        if path_junction != junction {
            push_conflict_diagnostic(
                unit,
                *module_order,
                "participantStream.junction",
                source.header.span.clone(),
                &mut diagnostics,
            );
            continue;
        }
        let junction_id = junctions[junction.index()].stable_id.into_untyped();
        let fields = [
            IdentityFieldInput::new(
                FieldTag::AuthoringNamespaceId,
                source_module
                    .descriptor()
                    .authoring_namespace_id()
                    .as_bytes(),
            ),
            IdentityFieldInput::new(
                FieldTag::ParticipantStreamKey,
                source.header.stable_key.as_bytes(),
            ),
            IdentityFieldInput::new(FieldTag::JunctionStableId, junction_id.as_bytes()),
        ];
        let stable_id = ParticipantStreamId::from_untyped(derive_identity(
            unit,
            identities,
            *module_order as usize,
            EntityKind::ParticipantStream,
            &source.header.stable_key,
            &source.header.span,
            &fields,
        )?);
        let start = passages.len();
        let mut seen_zones = Vec::with_capacity(source.passages.len());
        for source_passage in source.passages.iter() {
            let Some(zone) = resolve_reference(
                module_lookup,
                &zone_symbols,
                &source_passage.conflict_zone,
                EntityKind::ParticipantStream,
                &source.header,
                *module_order,
                &mut diagnostics,
            ) else {
                continue;
            };
            if zones.get(zone).junction != junction || seen_zones.contains(&zone) {
                push_conflict_diagnostic(
                    unit,
                    *module_order,
                    "participantStream.passages.conflictZone",
                    source_passage.span.clone(),
                    &mut diagnostics,
                );
                continue;
            }
            seen_zones.push(zone);
            let entry = lower_anchor(
                unit,
                *module_order,
                source,
                path,
                &source_passage.entry,
                maneuver_paths,
                maneuver_path_edges,
                lane_edges,
                maneuver_gates,
                maneuver_path_gates,
                module_lookup,
                &gate_symbols,
                &mut diagnostics,
            )?;
            let exit = lower_anchor(
                unit,
                *module_order,
                source,
                path,
                &source_passage.exit,
                maneuver_paths,
                maneuver_path_edges,
                lane_edges,
                maneuver_gates,
                maneuver_path_gates,
                module_lookup,
                &gate_symbols,
                &mut diagnostics,
            )?;
            let (Some(entry), Some(exit)) = (entry, exit) else {
                continue;
            };
            if entry.position() >= exit.position() {
                push_conflict_diagnostic(
                    unit,
                    *module_order,
                    "participantStream.passages.anchorOrder",
                    source_passage.span.clone(),
                    &mut diagnostics,
                );
                continue;
            }
            let path_gate_slice =
                &maneuver_path_gates[maneuver_paths[path.index()].maneuver_gates.as_usize_range()];
            let admission_index = path_gate_slice
                .partition_point(|item| {
                    let gate = &maneuver_gates[item.maneuver_gate.index()];
                    (gate.transition_index.saturating_add(1), 0) <= entry.position()
                })
                .checked_sub(1);
            let Some(admission_index) = admission_index else {
                push_conflict_diagnostic(
                    unit,
                    *module_order,
                    "participantStream.passages.admissionGate",
                    source_passage.span.clone(),
                    &mut diagnostics,
                );
                continue;
            };
            let admission_gate = path_gate_slice[admission_index].maneuver_gate;
            if let Some(next) = path_gate_slice.get(admission_index.saturating_add(1)) {
                let next_position = (
                    maneuver_gates[next.maneuver_gate.index()]
                        .transition_index
                        .saturating_add(1),
                    0,
                );
                if exit.position() > next_position {
                    push_conflict_diagnostic(
                        unit,
                        *module_order,
                        "participantStream.passages.nextGate",
                        source_passage.span.clone(),
                        &mut diagnostics,
                    );
                    continue;
                }
            }
            let overlaps_waiting = waiting_zones.iter().any(|waiting| {
                if waiting.maneuver_path != path {
                    return false;
                }
                let waiting_entry = (
                    maneuver_gates[waiting.entry_gate.index()]
                        .transition_index
                        .saturating_add(1),
                    0,
                );
                let waiting_exit = (
                    maneuver_gates[waiting.release_gate.index()]
                        .transition_index
                        .saturating_add(1),
                    0,
                );
                entry.position() < waiting_exit && waiting_entry < exit.position()
            });
            if overlaps_waiting {
                push_conflict_diagnostic(
                    unit,
                    *module_order,
                    "participantStream.passages.waitingZoneOverlap",
                    source_passage.span.clone(),
                    &mut diagnostics,
                );
                continue;
            }
            passages.push(HirConflictPassage {
                conflict_zone: zone,
                entry,
                exit,
                admission_gate,
                conflict_zone_source_location: unit.resolve_source_location_for_module(
                    *module_order,
                    &source_passage.conflict_zone.span,
                )?,
                source_location: unit
                    .resolve_source_location_for_module(*module_order, &source_passage.span)?,
            });
            passage_owners.push(*stream_key);
        }
        passages[start..].sort_unstable_by(|left, right| {
            left.entry
                .position()
                .cmp(&right.entry.position())
                .then_with(|| left.exit.position().cmp(&right.exit.position()))
                .then_with(|| {
                    zones
                        .get(left.conflict_zone)
                        .stable_id
                        .cmp(&zones.get(right.conflict_zone).stable_id)
                })
        });
        let count = passages.len().saturating_sub(start);
        let stream = streams.get_mut(*stream_key);
        stream.stable_id = stable_id;
        stream.junction = junction;
        stream.junction_source_location =
            Some(unit.resolve_source_location_for_module(*module_order, &source.junction.span)?);
        stream.maneuver_path = path;
        stream.maneuver_path_source_location = Some(
            unit.resolve_source_location_for_module(*module_order, &source.maneuver_path.span)?,
        );
        stream.passages = TableRange::try_from_usize(start, count).map_err(|overflow| {
            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
        })?;
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    // 同一 zone/path/interval 的不同 stream 是重复行为；不同区间仍保留为显式身份意图。
    let mut behavior = passages
        .iter()
        .enumerate()
        .map(|(index, passage)| {
            let owner = passage_owners[index];
            (
                passage.conflict_zone,
                streams.get(owner).maneuver_path,
                passage.entry.position(),
                passage.exit.position(),
                streams.get(owner).stable_id,
                owner,
            )
        })
        .collect::<Vec<_>>();
    behavior.sort_unstable();
    for pair in behavior.windows(2) {
        if pair[0].0 == pair[1].0
            && pair[0].1 == pair[1].1
            && pair[0].2 == pair[1].2
            && pair[0].3 == pair[1].3
        {
            let duplicate = streams.get(pair[1].5);
            push_conflict_diagnostic(
                unit,
                duplicate.module.raw(),
                "participantStream.duplicateBehavior",
                duplicate.source_span.clone(),
                &mut diagnostics,
            );
        }
    }

    let mut memberships = passages
        .iter()
        .enumerate()
        .map(|(index, passage)| (passage.conflict_zone, passage_owners[index]))
        .collect::<Vec<_>>();
    memberships.sort_unstable_by(|left, right| {
        left.0.cmp(&right.0).then_with(|| {
            streams
                .get(left.1)
                .stable_id
                .cmp(&streams.get(right.1).stable_id)
        })
    });
    let mut zone_streams = Vec::with_capacity(memberships.len());
    let mut cursor = 0_usize;
    for zone_index in 0..zones.len() {
        let zone_key = HirConflictZoneKey::from_raw(
            u32::try_from(zone_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        let start = cursor;
        while cursor < memberships.len() && memberships[cursor].0 == zone_key {
            zone_streams.push(HirConflictZoneStream {
                participant_stream: memberships[cursor].1,
            });
            cursor = cursor.saturating_add(1);
        }
        let count = cursor.saturating_sub(start);
        if count < 2 {
            let zone = zones.get(zone_key);
            push_conflict_diagnostic(
                unit,
                zone.module.raw(),
                "conflictZone.participantStreams",
                zone.source_span.clone(),
                &mut diagnostics,
            );
        }
        zones.get_mut(zone_key).participant_streams = TableRange::try_from_usize(start, count)
            .map_err(|overflow| arena_overflow(overflow, &unit.limits, None))?;
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    debug_assert_eq!(zones.len(), zone_capacity);
    debug_assert_eq!(streams.len(), stream_capacity);
    debug_assert_eq!(passages.len(), passage_capacity);
    Ok(ConflictHir {
        conflict_zones: zones.into_boxed_slice(),
        participant_streams: streams.into_boxed_slice(),
        conflict_passages: passages.into_boxed_slice(),
        conflict_zone_streams: zone_streams.into_boxed_slice(),
    })
}

#[allow(clippy::too_many_arguments)]
fn lower_anchor(
    unit: &CompilationUnit,
    module_order: u32,
    source: &crate::declaration::ParticipantStreamDeclaration,
    path_key: HirManeuverPathKey,
    anchor: &PathAnchorDeclaration,
    maneuver_paths: &[HirManeuverPath],
    maneuver_path_edges: &[HirManeuverPathEdge],
    lane_edges: &[HirLaneEdge],
    maneuver_gates: &[HirManeuverGate],
    maneuver_path_gates: &[HirManeuverPathGate],
    module_lookup: &HashMap<Arc<str>, HirModuleKey>,
    gate_symbols: &SymbolTable<HirManeuverGateKey>,
    diagnostics: &mut DiagnosticCollector,
) -> Result<Option<HirPathAnchor>, DiagnosticBundle> {
    let path_edges = &maneuver_path_edges[maneuver_paths[path_key.index()].edges.as_usize_range()];
    let source_location = unit.resolve_source_location_for_module(module_order, anchor.span())?;
    match anchor {
        PathAnchorDeclaration::Gate { gate, .. } => {
            let Some(gate_key) = resolve_reference(
                module_lookup,
                gate_symbols,
                gate,
                EntityKind::ParticipantStream,
                &source.header,
                module_order,
                diagnostics,
            ) else {
                return Ok(None);
            };
            let resolved = &maneuver_gates[gate_key.index()];
            if resolved.maneuver_path != path_key {
                push_conflict_diagnostic(
                    unit,
                    module_order,
                    "pathAnchor.gate",
                    anchor.span().clone(),
                    diagnostics,
                );
                return Ok(None);
            }
            Ok(Some(HirPathAnchor {
                reference: HirPathAnchorReference::Gate(gate_key),
                progress_mm: None,
                position_edge_index: resolved.transition_index.saturating_add(1),
                position_progress_mm: 0,
                source_location,
            }))
        }
        PathAnchorDeclaration::EdgeBoundary { boundary_index, .. } => {
            let index = usize::try_from(*boundary_index)
                .expect("u32 path edge index fits usize on every supported target");
            if index > path_edges.len() {
                push_conflict_diagnostic(
                    unit,
                    module_order,
                    "pathAnchor.boundaryIndex",
                    anchor.span().clone(),
                    diagnostics,
                );
                return Ok(None);
            }
            let gate_at_boundary = maneuver_path_gates[maneuver_paths[path_key.index()]
                .maneuver_gates
                .as_usize_range()]
            .iter()
            .any(|item| {
                maneuver_gates[item.maneuver_gate.index()]
                    .transition_index
                    .saturating_add(1)
                    == *boundary_index
            });
            if gate_at_boundary {
                push_conflict_diagnostic(
                    unit,
                    module_order,
                    "pathAnchor.boundaryHasGate",
                    anchor.span().clone(),
                    diagnostics,
                );
                return Ok(None);
            }
            Ok(Some(HirPathAnchor {
                reference: HirPathAnchorReference::EdgeBoundary(*boundary_index),
                progress_mm: None,
                position_edge_index: *boundary_index,
                position_progress_mm: 0,
                source_location,
            }))
        }
        PathAnchorDeclaration::Interior {
            path_edge_index,
            progress_mm,
            ..
        } => {
            let index = usize::try_from(*path_edge_index)
                .expect("u32 path edge index fits usize on every supported target");
            let Some(path_edge) = path_edges.get(index) else {
                push_conflict_diagnostic(
                    unit,
                    module_order,
                    "pathAnchor.pathEdgeIndex",
                    anchor.span().clone(),
                    diagnostics,
                );
                return Ok(None);
            };
            let edge_length = lane_edges[path_edge.target.index()].length_mm;
            if *progress_mm == 0 || *progress_mm >= edge_length {
                push_conflict_diagnostic(
                    unit,
                    module_order,
                    "pathAnchor.progressMeters",
                    anchor.span().clone(),
                    diagnostics,
                );
                return Ok(None);
            }
            Ok(Some(HirPathAnchor {
                reference: HirPathAnchorReference::Interior {
                    path_edge_index: *path_edge_index,
                },
                progress_mm: Some(*progress_mm),
                position_edge_index: *path_edge_index,
                position_progress_mm: *progress_mm,
                source_location,
            }))
        }
    }
}

fn push_conflict_diagnostic(
    unit: &CompilationUnit,
    module_order: u32,
    field: &'static str,
    source_span: SourceLocation,
    diagnostics: &mut DiagnosticCollector,
) {
    let module = &unit.modules[module_order as usize];
    let source_document_key = module
        .source_documents
        .first()
        .expect("official module retains a source document")
        .source_document_key();
    let mut diagnostic = Diagnostic::invalid_road_editing_source_at(
        RoadEditingSourceViolation::InvalidSemanticValue(
            RoadEditingInputViolation::InvalidCombination,
        ),
        Some(field),
        source_document_key,
        Some(source_document_key),
        Some(source_span),
    );
    diagnostic.set_canonical_module_order(module_order);
    diagnostics.push(diagnostic);
}

//! 空间领域 HIR：规范坐标框架、车道边几何与几何来源映射的记录与构建。

use std::collections::HashMap;
use std::sync::Arc;

use laneflow_static_contract::{
    CanonicalFrameId, EntityKind, FieldTag, MAX_LANE_EDGE_LENGTH_MM, MIN_LANE_EDGE_LENGTH_MM,
    SPATIAL_JOIN_POSITION_TOLERANCE_METERS, millimetres_from_si,
};

use crate::arena::{ArenaKeyOverflow, TableRange, TypedArena};
use crate::declaration::{
    CanonicalPoint3F32Input, ConflictZoneRegionDeclaration, LaneEdgeDeclaration,
    LaneEdgeGeometryAuthority, OwnedEntityReference, TypedAstDeclaration, TypedAstEntityAddress,
};
use crate::diagnostic::DiagnosticCollector;
use crate::geometry_profile::GeometryCompilationProfiles;
use crate::identity::{IdentityFieldInput, IdentityRegistry};
use crate::spatial_freeze::{
    check_spatial_direction, freeze_canonical_polyline, freeze_conflict_zone_region,
    freeze_spatial_polyline,
};
use crate::{
    CompilationUnit, CompileLimitDimension, Diagnostic, DiagnosticBundle, SourceLocation,
    SpatialGeometryViolation,
};

use super::{
    HirCanonicalFrameKey, HirCanonicalFrameTag, HirConflictZone, HirConflictZoneKey,
    HirFacilityBand, HirFacilityBandKey, HirJunctionInternalEdge, HirLaneEdge, HirLaneEdgeKey,
    HirLaneEdgeReference, HirLaneEdgeTag, HirManeuverPath, HirManeuverPathEdge, HirModuleKey,
    SpatialCounts, SymbolTable, arena_overflow, count_to_usize, declaration_header,
    derive_identity, lane_edge_declaration, resolve_reference,
};

/// 已冻结稳定身份的规范坐标框架。
///
/// 该记录故意不保存轴向、单位或宿主放置：这些语义分别由全局 canonical frame
/// 契约和 Adapter 边界拥有，不能成为同一 `frameId` 下的可变配置。
#[derive(Debug, PartialEq)]
pub(crate) struct HirCanonicalFrame {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: CanonicalFrameId,
    pub(crate) lane_edge_geometries: TableRange<HirLaneEdgeGeometry>,
    pub(crate) facility_band_geometries: TableRange<HirFacilityBandGeometry>,
    pub(crate) source_span: SourceLocation,
}

/// 规范坐标框架内的一条中心线；点与线段区间均按行驶方向排列。
#[derive(Debug, PartialEq)]
pub(crate) struct HirLaneEdgeGeometry {
    pub(crate) source_module: HirModuleKey,
    pub(crate) canonical_frame: HirCanonicalFrameKey,
    pub(crate) lane_edge: HirLaneEdgeKey,
    pub(crate) points: TableRange<HirCanonicalPoint3F32>,
    pub(crate) segments: TableRange<HirSpatialSegment>,
    pub(crate) source_ranges: TableRange<HirGeometrySourceRange>,
    pub(crate) arc_length_meters: f32,
    pub(crate) source_span: SourceLocation,
}

/// 不可遍历 FacilityBand 的规范中心线；与 LaneEdge 几何共享规范点表。
#[derive(Debug, PartialEq)]
pub(crate) struct HirFacilityBandGeometry {
    pub(crate) canonical_frame: HirCanonicalFrameKey,
    pub(crate) facility_band: HirFacilityBandKey,
    pub(crate) points: TableRange<HirCanonicalPoint3F32>,
    pub(crate) source_ranges: TableRange<HirGeometrySourceRange>,
    pub(crate) source_span: SourceLocation,
}

/// 一个 ConflictZone 在规范 frame 中的 owner-local 2.5D 区域。
#[derive(Debug, PartialEq)]
pub(crate) struct HirConflictZoneRegion {
    pub(crate) source_module: HirModuleKey,
    pub(crate) conflict_zone: super::HirConflictZoneKey,
    pub(crate) canonical_frame: HirCanonicalFrameKey,
    pub(crate) min_y: f32,
    pub(crate) max_y: f32,
    pub(crate) ring_xz: TableRange<HirCanonicalPoint2F32>,
    pub(crate) conflict_zone_source_location: crate::module::ResolvedSourceLocation,
    pub(crate) canonical_frame_source_location: crate::module::ResolvedSourceLocation,
    pub(crate) source_location: crate::module::ResolvedSourceLocation,
    pub(crate) source_span: SourceLocation,
}

/// 共享规范点表中一段连续点范围到 authoring source segment 的阶段私有来源映射。
#[derive(Debug, PartialEq)]
pub(crate) struct HirGeometrySourceRange {
    pub(crate) source_module: HirModuleKey,
    pub(crate) points: TableRange<HirCanonicalPoint3F32>,
    pub(crate) source_segment_ordinal: u32,
    pub(crate) source: SourceLocation,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HirCanonicalPoint3F32 {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) z: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HirCanonicalPoint2F32 {
    pub(crate) x: f32,
    pub(crate) z: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HirSpatialSegment {
    pub(crate) length_meters: f32,
    pub(crate) cumulative_end_meters: f32,
    pub(crate) tangent: [f32; 3],
    pub(crate) up: [f32; 3],
}

#[derive(Default)]
pub(crate) struct SpatialHir {
    pub(crate) geometry_profiles: Option<GeometryCompilationProfiles>,
    pub(crate) canonical_frames: Box<[HirCanonicalFrame]>,
    pub(crate) lane_edge_geometries: Box<[HirLaneEdgeGeometry]>,
    pub(crate) facility_band_geometries: Box<[HirFacilityBandGeometry]>,
    pub(crate) conflict_zone_regions: Box<[HirConflictZoneRegion]>,
    pub(crate) geometry_source_ranges: Box<[HirGeometrySourceRange]>,
    pub(crate) canonical_points: Box<[HirCanonicalPoint3F32]>,
    pub(crate) conflict_region_points: Box<[HirCanonicalPoint2F32]>,
    pub(crate) spatial_segments: Box<[HirSpatialSegment]>,
}

pub(crate) struct PendingSpatialGeometry<'a> {
    source_module: HirModuleKey,
    centerline_points: &'a [CanonicalPoint3F32Input],
    expected_length_meters: f64,
    source_ranges: &'a [crate::declaration::CompiledGeometrySourceRange],
    source_span: SourceLocation,
}

#[derive(Clone)]
pub(crate) struct SpatialFrameAssignment {
    frame: HirCanonicalFrameKey,
    source_span: SourceLocation,
}

pub(crate) struct SpatialHirContext<'a> {
    pub(crate) lane_edges: &'a mut TypedArena<HirLaneEdgeTag, HirLaneEdge>,
    pub(crate) lane_edge_references: &'a [HirLaneEdgeReference],
    pub(crate) lane_edge_symbols: &'a SymbolTable<HirLaneEdgeKey>,
    pub(crate) facility_bands: &'a [HirFacilityBand],
    pub(crate) maneuver_paths: &'a [HirManeuverPath],
    pub(crate) maneuver_path_edges: &'a [HirManeuverPathEdge],
    pub(crate) junction_internal_edges: &'a [HirJunctionInternalEdge],
}

pub(crate) struct PendingConflictZoneRegion<'a> {
    module_order: u32,
    source: &'a ConflictZoneRegionDeclaration,
    conflict_zone: HirConflictZoneKey,
    canonical_frame: HirCanonicalFrameKey,
}

pub(crate) fn build_spatial_hir(
    unit: &CompilationUnit,
    counts: &SpatialCounts,
    module_lookup: &HashMap<Arc<str>, HirModuleKey>,
    context: SpatialHirContext<'_>,
    identities: &mut IdentityRegistry,
) -> Result<SpatialHir, DiagnosticBundle> {
    let SpatialHirContext {
        lane_edges,
        lane_edge_references,
        lane_edge_symbols,
        facility_bands,
        maneuver_paths,
        maneuver_path_edges,
        junction_internal_edges,
    } = context;
    if counts.canonical_frames == 0
        && counts.lane_edge_geometries == 0
        && counts.facility_band_geometries == 0
    {
        return Ok(SpatialHir::default());
    }

    let mut frames = TypedArena::<HirCanonicalFrameTag, HirCanonicalFrame>::with_capacity(
        count_to_usize(counts.canonical_frames, &unit.limits)?,
    );
    let mut geometries: Vec<HirLaneEdgeGeometry> =
        Vec::with_capacity(count_to_usize(counts.lane_edge_geometries, &unit.limits)?);
    let mut facility_geometries: Vec<HirFacilityBandGeometry> = Vec::with_capacity(count_to_usize(
        counts.facility_band_geometries,
        &unit.limits,
    )?);
    let mut geometry_source_ranges =
        Vec::with_capacity(count_to_usize(counts.geometry_source_ranges, &unit.limits)?);
    let mut points = Vec::with_capacity(count_to_usize(counts.canonical_points, &unit.limits)?);
    let mut segments = Vec::with_capacity(count_to_usize(counts.spatial_segments, &unit.limits)?);
    let mut pending_geometries: Vec<Option<PendingSpatialGeometry<'_>>> =
        (0..lane_edges.len()).map(|_| None).collect();
    let mut pending_facility_geometries: Vec<Option<PendingSpatialGeometry<'_>>> =
        (0..facility_bands.len()).map(|_| None).collect();
    let mut facility_frame_assignments: Vec<Option<SpatialFrameAssignment>> =
        (0..facility_bands.len()).map(|_| None).collect();
    let mut frame_assignments: Vec<Option<SpatialFrameAssignment>> =
        (0..lane_edges.len()).map(|_| None).collect();
    let mut geometry_index_by_edge = vec![None::<usize>; lane_edges.len()];
    let mut internal_edge_flags = vec![0_u8; lane_edges.len()];
    for relation in junction_internal_edges {
        internal_edge_flags[relation.edge.index()] = 1;
    }
    let mut frame_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|declaration| matches!(declaration, TypedAstDeclaration::CanonicalFrame(_)))
            .count()
    }));
    let mut facility_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|declaration| matches!(declaration, TypedAstDeclaration::FacilityBand(_)))
            .count()
    }));
    for (index, band) in facility_bands.iter().enumerate() {
        facility_symbols.insert(
            band.module,
            band.source_address.clone(),
            HirFacilityBandKey::from_raw(
                u32::try_from(index).expect("FacilityBand arena length is bounded by u32"),
            ),
        );
    }
    let mut diagnostics =
        DiagnosticCollector::new(unit.limits.value(CompileLimitDimension::DiagnosticCount));
    for (module_index, source_module) in unit.modules.iter().enumerate() {
        let module_key = HirModuleKey::from_raw(
            u32::try_from(module_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        let mut declaration_indices: Vec<_> = source_module
            .declarations
            .iter()
            .enumerate()
            .filter_map(|(index, declaration)| {
                matches!(declaration, TypedAstDeclaration::CanonicalFrame(_)).then_some(index)
            })
            .collect();
        declaration_indices.sort_unstable_by_key(|index| {
            &declaration_header(&source_module.declarations[*index]).source_address
        });
        for declaration_index in declaration_indices {
            let TypedAstDeclaration::CanonicalFrame(source) =
                &source_module.declarations[declaration_index]
            else {
                unreachable!("canonical frame source filter admitted unrelated declaration")
            };
            let fields = [
                IdentityFieldInput::new(
                    FieldTag::AuthoringNamespaceId,
                    source_module
                        .descriptor()
                        .authoring_namespace_id()
                        .as_bytes(),
                ),
                IdentityFieldInput::new(
                    FieldTag::CanonicalFrameKey,
                    source.header.stable_key.as_bytes(),
                ),
            ];
            let stable_id = CanonicalFrameId::from_untyped(derive_identity(
                unit,
                identities,
                module_index,
                EntityKind::CanonicalFrame,
                &source.header.stable_key,
                &source.header.span,
                &fields,
            )?);
            let frame_key = frames
                .push(HirCanonicalFrame {
                    module: module_key,
                    stable_key: Arc::clone(&source.header.stable_key),
                    stable_id,
                    lane_edge_geometries: TableRange::empty(),
                    facility_band_geometries: TableRange::empty(),
                    source_span: source.header.span.clone(),
                })
                .map_err(|overflow| {
                    arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                })?;
            frame_symbols.insert(module_key, source.header.source_address.clone(), frame_key);

            for geometry in &source.lane_edge_geometries {
                let target_module = module_lookup[geometry.lane_edge.module_namespace.as_ref()];
                let Some(lane_edge) =
                    lane_edge_symbols.get(target_module, &geometry.lane_edge.target_address)
                else {
                    let mut diagnostic = Diagnostic::unknown_reference_target(
                        EntityKind::LaneEdge,
                        &source.header.stable_key,
                        &geometry.lane_edge.module_namespace,
                        geometry.lane_edge.declaration_key(),
                        geometry.lane_edge.span.clone(),
                        source.header.span.clone(),
                    );
                    diagnostic.set_canonical_module_order(
                        u32::try_from(module_index).unwrap_or(u32::MAX),
                    );
                    diagnostics.push(diagnostic);
                    continue;
                };
                if let Some(existing) = &pending_geometries[lane_edge.index()] {
                    let mut diagnostic = Diagnostic::invalid_spatial_geometry(
                        Some(&source.header.stable_key),
                        geometry.lane_edge.declaration_key(),
                        None,
                        SpatialGeometryViolation::DuplicateEdgeBinding,
                        geometry.lane_edge.span.clone(),
                        Some(existing.source_span.clone()),
                    );
                    diagnostic.set_canonical_module_order(
                        u32::try_from(module_index).unwrap_or(u32::MAX),
                    );
                    diagnostics.push(diagnostic);
                    continue;
                }
                pending_geometries[lane_edge.index()] = Some(PendingSpatialGeometry {
                    source_module: module_key,
                    centerline_points: &geometry.centerline_points,
                    expected_length_meters: f64::from(lane_edges.get(lane_edge).length_mm)
                        / 1_000.0,
                    source_ranges: &[],
                    source_span: geometry.lane_edge.span.clone(),
                });
                frame_assignments[lane_edge.index()] = Some(SpatialFrameAssignment {
                    frame: frame_key,
                    source_span: geometry.lane_edge.span.clone(),
                });
            }
        }
    }

    // RoadEditingSource 点表不回填 CanonicalFrame 声明。先把全部 compiled LaneEdge
    // 登记到与 Synthetic 显式几何相同的 edge-indexed pending 表；section-derived edge
    // 解析显式 frame，junction-internal edge 留给完整 ManeuverPath 图唯一推导。
    let mut compilation_profiles: Option<(GeometryCompilationProfiles, SourceLocation)> = None;
    for (module_index, source_module) in unit.modules.iter().enumerate() {
        let module_key = HirModuleKey::from_raw(
            u32::try_from(module_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        let mut declaration_indices: Vec<_> = source_module
            .declarations
            .iter()
            .enumerate()
            .filter_map(|(index, declaration)| {
                matches!(
                    declaration,
                    TypedAstDeclaration::LaneEdge(LaneEdgeDeclaration {
                        geometry_authority: LaneEdgeGeometryAuthority::Compiled(_),
                        ..
                    })
                )
                .then_some(index)
            })
            .collect();
        declaration_indices.sort_unstable_by_key(|index| {
            &declaration_header(&source_module.declarations[*index]).source_address
        });
        if let Some(first_index) = declaration_indices.first().copied() {
            let first = lane_edge_declaration(&source_module.declarations[first_index])
                .expect("compiled geometry filter must name a LaneEdge");
            match source_module.geometry_profiles {
                None => {
                    let mut diagnostic = Diagnostic::invalid_spatial_geometry(
                        None,
                        &first.header.stable_key,
                        None,
                        SpatialGeometryViolation::MissingGeometryProfiles,
                        first.header.span.clone(),
                        None,
                    );
                    diagnostic.set_canonical_module_order(
                        u32::try_from(module_index).unwrap_or(u32::MAX),
                    );
                    diagnostics.push(diagnostic);
                }
                Some(actual) => {
                    if let Some((expected, expected_span)) = &compilation_profiles {
                        if actual != *expected {
                            let mut diagnostic = Diagnostic::invalid_spatial_geometry(
                                None,
                                &first.header.stable_key,
                                None,
                                SpatialGeometryViolation::GeometryProfileMismatch {
                                    expected_accuracy_code: expected.accuracy as u8,
                                    expected_direction_code: expected.direction as u8,
                                    actual_accuracy_code: actual.accuracy as u8,
                                    actual_direction_code: actual.direction as u8,
                                },
                                first.header.span.clone(),
                                Some(expected_span.clone()),
                            );
                            diagnostic.set_canonical_module_order(
                                u32::try_from(module_index).unwrap_or(u32::MAX),
                            );
                            diagnostics.push(diagnostic);
                        }
                    } else {
                        compilation_profiles = Some((actual, first.header.span.clone()));
                    }
                }
            }
        }

        for declaration_index in declaration_indices {
            let source = lane_edge_declaration(&source_module.declarations[declaration_index])
                .expect("compiled geometry filter must name a LaneEdge");
            let LaneEdgeGeometryAuthority::Compiled(compiled) = &source.geometry_authority else {
                unreachable!("compiled geometry filter changed authority")
            };
            let lane_edge = lane_edge_symbols
                .get(module_key, &source.header.source_address)
                .expect("HIR registered every Typed AST LaneEdge symbol");
            if let Some(existing) = &pending_geometries[lane_edge.index()] {
                let mut diagnostic = Diagnostic::invalid_spatial_geometry(
                    None,
                    &source.header.stable_key,
                    None,
                    SpatialGeometryViolation::DuplicateEdgeBinding,
                    source.header.span.clone(),
                    Some(existing.source_span.clone()),
                );
                diagnostic
                    .set_canonical_module_order(u32::try_from(module_index).unwrap_or(u32::MAX));
                diagnostics.push(diagnostic);
                continue;
            }
            pending_geometries[lane_edge.index()] = Some(PendingSpatialGeometry {
                source_module: module_key,
                centerline_points: &compiled.centerline_points,
                expected_length_meters: compiled.length.observation_metres(),
                source_ranges: &compiled.source_ranges,
                source_span: source.header.span.clone(),
            });
            if compiled.canonical_frame.is_none() && internal_edge_flags[lane_edge.index()] == 0 {
                let mut diagnostic = Diagnostic::invalid_spatial_geometry(
                    None,
                    &source.header.stable_key,
                    None,
                    SpatialGeometryViolation::MissingCanonicalFrame,
                    source.header.span.clone(),
                    None,
                );
                diagnostic
                    .set_canonical_module_order(u32::try_from(module_index).unwrap_or(u32::MAX));
                diagnostics.push(diagnostic);
            }
            if let Some(frame_reference) = &compiled.canonical_frame
                && let Some(frame) = resolve_reference(
                    module_lookup,
                    &frame_symbols,
                    frame_reference,
                    EntityKind::LaneEdge,
                    &source.header,
                    u32::try_from(module_index).unwrap_or(u32::MAX),
                    &mut diagnostics,
                )
            {
                frame_assignments[lane_edge.index()] = Some(SpatialFrameAssignment {
                    frame,
                    source_span: frame_reference.span.clone(),
                });
            }
        }
    }

    // FacilityBand 几何不可遍历，因此不进入 LaneEdge 覆盖或连接图；它仍与车道边共享
    // frame 符号、点冻结器、资源前门以及规范点 backing table。
    for (module_index, source_module) in unit.modules.iter().enumerate() {
        let module_key = HirModuleKey::from_raw(
            u32::try_from(module_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        for declaration in &source_module.declarations {
            let TypedAstDeclaration::FacilityBand(source) = declaration else {
                continue;
            };
            let Some(compiled) = &source.compiled_geometry else {
                continue;
            };
            let band = facility_symbols
                .get(module_key, &source.header.source_address)
                .expect("cross-section HIR registered every FacilityBand symbol");
            pending_facility_geometries[band.index()] = Some(PendingSpatialGeometry {
                source_module: module_key,
                centerline_points: &compiled.centerline_points,
                expected_length_meters: compiled.length.observation_metres(),
                source_ranges: &compiled.source_ranges,
                source_span: source.header.span.clone(),
            });
            if let Some(frame) = resolve_reference(
                module_lookup,
                &frame_symbols,
                &compiled.canonical_frame,
                EntityKind::FacilityBand,
                &source.header,
                u32::try_from(module_index).unwrap_or(u32::MAX),
                &mut diagnostics,
            ) {
                facility_frame_assignments[band.index()] = Some(SpatialFrameAssignment {
                    frame,
                    source_span: compiled.canonical_frame.span.clone(),
                });
            }
        }
    }

    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }
    if pending_geometries.iter().all(Option::is_none)
        && pending_facility_geometries.iter().all(Option::is_none)
    {
        return Ok(SpatialHir {
            geometry_profiles: compilation_profiles.map(|(profiles, _)| profiles),
            canonical_frames: frames.into_boxed_slice(),
            ..SpatialHir::default()
        });
    }

    // entry/exit frame 是 internal edge frame 的唯一来源。共享 internal edge 可被多条 path
    // 使用，但所有路径必须推导出同一 frame；赋值只改变阶段私有索引，不移动任何点。
    for path in maneuver_paths {
        let path_edges = &maneuver_path_edges[path.edges.as_usize_range()];
        let [entry, .., exit] = path_edges else {
            unreachable!("validated ManeuverPath contains entry and exit")
        };
        let Some(entry_assignment) = frame_assignments[entry.target.index()].clone() else {
            let mut diagnostic = Diagnostic::invalid_spatial_geometry(
                None,
                &lane_edges.get(entry.target).stable_key,
                None,
                SpatialGeometryViolation::MissingCanonicalFrame,
                entry.source_span.clone(),
                Some(path.source_span.clone()),
            );
            diagnostic.set_canonical_module_order(path.module.raw());
            diagnostics.push(diagnostic);
            continue;
        };
        let Some(exit_assignment) = frame_assignments[exit.target.index()].clone() else {
            let mut diagnostic = Diagnostic::invalid_spatial_geometry(
                None,
                &lane_edges.get(exit.target).stable_key,
                None,
                SpatialGeometryViolation::MissingCanonicalFrame,
                exit.source_span.clone(),
                Some(path.source_span.clone()),
            );
            diagnostic.set_canonical_module_order(path.module.raw());
            diagnostics.push(diagnostic);
            continue;
        };
        if entry_assignment.frame != exit_assignment.frame {
            let mut diagnostic = Diagnostic::invalid_spatial_geometry(
                Some(&frames.get(entry_assignment.frame).stable_key),
                &lane_edges.get(entry.target).stable_key,
                Some(&lane_edges.get(exit.target).stable_key),
                SpatialGeometryViolation::ManeuverPathFrameMismatch,
                entry.source_span.clone(),
                Some(exit.source_span.clone()),
            );
            diagnostic.set_canonical_module_order(path.module.raw());
            diagnostics.push(diagnostic);
            continue;
        }
        for internal in &path_edges[1..path_edges.len() - 1] {
            debug_assert_ne!(internal_edge_flags[internal.target.index()], 0);
            match &frame_assignments[internal.target.index()] {
                Some(existing) if existing.frame != entry_assignment.frame => {
                    let mut diagnostic = Diagnostic::invalid_spatial_geometry(
                        Some(&frames.get(existing.frame).stable_key),
                        &lane_edges.get(internal.target).stable_key,
                        None,
                        SpatialGeometryViolation::InternalEdgeFrameConflict,
                        internal.source_span.clone(),
                        Some(existing.source_span.clone()),
                    );
                    diagnostic.set_canonical_module_order(path.module.raw());
                    diagnostics.push(diagnostic);
                }
                Some(_) => {}
                None => {
                    frame_assignments[internal.target.index()] = Some(SpatialFrameAssignment {
                        frame: entry_assignment.frame,
                        source_span: internal.source_span.clone(),
                    });
                }
            }
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    if pending_geometries.iter().any(Option::is_some) {
        for (index, pending) in pending_geometries.iter().enumerate() {
            let edge = lane_edges.get(HirLaneEdgeKey::from_raw(
                u32::try_from(index).expect("LaneEdge arena length is bounded by u32"),
            ));
            if pending.is_none() {
                let mut diagnostic = Diagnostic::invalid_spatial_geometry(
                    None,
                    &edge.stable_key,
                    None,
                    SpatialGeometryViolation::MissingEdgeBinding,
                    edge.source_span.clone(),
                    None,
                );
                diagnostic.set_canonical_module_order(edge.module.raw());
                diagnostics.push(diagnostic);
            } else if frame_assignments[index].is_none() {
                let mut diagnostic = Diagnostic::invalid_spatial_geometry(
                    None,
                    &edge.stable_key,
                    None,
                    SpatialGeometryViolation::MissingCanonicalFrame,
                    pending
                        .as_ref()
                        .expect("checked pending geometry")
                        .source_span
                        .clone(),
                    None,
                );
                diagnostic.set_canonical_module_order(edge.module.raw());
                diagnostics.push(diagnostic);
            }
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    let mut geometry_order = pending_geometries
        .iter()
        .enumerate()
        .filter_map(|(index, pending)| {
            pending.as_ref().map(|_| {
                HirLaneEdgeKey::from_raw(
                    u32::try_from(index).expect("LaneEdge arena length is bounded by u32"),
                )
            })
        })
        .collect::<Vec<_>>();
    geometry_order.sort_unstable_by_key(|edge| {
        (
            frame_assignments[edge.index()]
                .as_ref()
                .expect("complete spatial frame coverage")
                .frame
                .raw(),
            edge.raw(),
        )
    });
    let mut facility_geometry_order = pending_facility_geometries
        .iter()
        .enumerate()
        .filter_map(|(index, pending)| {
            pending.as_ref().map(|_| {
                HirFacilityBandKey::from_raw(
                    u32::try_from(index).expect("FacilityBand arena length is bounded by u32"),
                )
            })
        })
        .collect::<Vec<_>>();
    facility_geometry_order.sort_unstable_by_key(|band| {
        (
            facility_frame_assignments[band.index()]
                .as_ref()
                .expect("compiled FacilityBand frame was resolved")
                .frame
                .raw(),
            band.raw(),
        )
    });
    let mut order_cursor = 0_usize;
    let mut facility_order_cursor = 0_usize;
    for frame_index in 0..frames.len() {
        let frame_key = HirCanonicalFrameKey::from_raw(
            u32::try_from(frame_index).expect("compile limits bound CanonicalFrame indexes"),
        );
        let geometry_start = geometries.len();
        while let Some(edge) = geometry_order.get(order_cursor).copied() {
            let assignment = frame_assignments[edge.index()]
                .as_ref()
                .expect("complete spatial frame coverage");
            if assignment.frame != frame_key {
                break;
            }
            let pending = pending_geometries[edge.index()]
                .as_ref()
                .expect("geometry order only contains pending inputs");
            let frozen = match freeze_spatial_polyline(
                pending.centerline_points,
                pending.expected_length_meters,
                &mut points,
                &mut segments,
            ) {
                Ok(frozen) => frozen,
                Err(violation) => {
                    let mut diagnostic = Diagnostic::invalid_spatial_geometry(
                        Some(&frames.get(frame_key).stable_key),
                        &lane_edges.get(edge).stable_key,
                        None,
                        violation,
                        pending.source_span.clone(),
                        None,
                    );
                    diagnostic.set_canonical_module_order(lane_edges.get(edge).module.raw());
                    diagnostics.push(diagnostic);
                    order_cursor = order_cursor.saturating_add(1);
                    continue;
                }
            };
            let Some(committed_mm) = millimetres_from_si(f64::from(frozen.arc_length_meters))
            else {
                let mut diagnostic = Diagnostic::invalid_lane_edge_length(
                    &lane_edges.get(edge).stable_key,
                    f64::from(frozen.arc_length_meters),
                    crate::declaration::ScalarViolation::QuantizeFailed,
                    pending.source_span.clone(),
                );
                diagnostic.set_canonical_module_order(lane_edges.get(edge).module.raw());
                diagnostics.push(diagnostic);
                order_cursor = order_cursor.saturating_add(1);
                continue;
            };
            if !(MIN_LANE_EDGE_LENGTH_MM..=MAX_LANE_EDGE_LENGTH_MM).contains(&committed_mm) {
                let mut diagnostic = Diagnostic::invalid_lane_edge_length(
                    &lane_edges.get(edge).stable_key,
                    f64::from(frozen.arc_length_meters),
                    crate::declaration::ScalarViolation::OutsideClosedMillimetreRange {
                        min_mm: MIN_LANE_EDGE_LENGTH_MM,
                        max_mm: MAX_LANE_EDGE_LENGTH_MM,
                        actual_mm: committed_mm,
                    },
                    pending.source_span.clone(),
                );
                diagnostic.set_canonical_module_order(lane_edges.get(edge).module.raw());
                diagnostics.push(diagnostic);
                order_cursor = order_cursor.saturating_add(1);
                continue;
            }
            lane_edges.get_mut(edge).length_mm = committed_mm;
            let geometry_index = geometries.len();
            let source_range_start = geometry_source_ranges.len();
            push_geometry_source_ranges(
                pending,
                frozen.point_start,
                &mut geometry_source_ranges,
                &unit.limits,
            )?;
            geometries.push(HirLaneEdgeGeometry {
                source_module: pending.source_module,
                canonical_frame: frame_key,
                lane_edge: edge,
                points: TableRange::try_from_usize(frozen.point_start, frozen.point_count)
                    .map_err(|overflow| {
                        arena_overflow(overflow, &unit.limits, Some(pending.source_span.clone()))
                    })?,
                segments: TableRange::try_from_usize(frozen.segment_start, frozen.segment_count)
                    .map_err(|overflow| {
                        arena_overflow(overflow, &unit.limits, Some(pending.source_span.clone()))
                    })?,
                source_ranges: TableRange::try_from_usize(
                    source_range_start,
                    geometry_source_ranges
                        .len()
                        .saturating_sub(source_range_start),
                )
                .map_err(|overflow| {
                    arena_overflow(overflow, &unit.limits, Some(pending.source_span.clone()))
                })?,
                arc_length_meters: frozen.arc_length_meters,
                source_span: pending.source_span.clone(),
            });
            geometry_index_by_edge[edge.index()] = Some(geometry_index);
            order_cursor = order_cursor.saturating_add(1);
        }
        let frame_span = frames.get(frame_key).source_span.clone();
        frames.get_mut(frame_key).lane_edge_geometries = TableRange::try_from_usize(
            geometry_start,
            geometries.len().saturating_sub(geometry_start),
        )
        .map_err(|overflow| arena_overflow(overflow, &unit.limits, Some(frame_span.clone())))?;

        let facility_geometry_start = facility_geometries.len();
        while let Some(band) = facility_geometry_order.get(facility_order_cursor).copied() {
            let assignment = facility_frame_assignments[band.index()]
                .as_ref()
                .expect("compiled FacilityBand frame was resolved");
            if assignment.frame != frame_key {
                break;
            }
            let pending = pending_facility_geometries[band.index()]
                .as_ref()
                .expect("facility geometry order only contains pending inputs");
            let frozen = match freeze_canonical_polyline(
                pending.centerline_points,
                pending.expected_length_meters,
                &mut points,
            ) {
                Ok(frozen) => frozen,
                Err(violation) => {
                    let band_record = &facility_bands[band.index()];
                    let mut diagnostic = Diagnostic::invalid_facility_band_geometry(
                        Some(&frames.get(frame_key).stable_key),
                        &band_record.stable_key,
                        violation,
                        pending.source_span.clone(),
                    );
                    diagnostic.set_canonical_module_order(band_record.module.raw());
                    diagnostics.push(diagnostic);
                    facility_order_cursor = facility_order_cursor.saturating_add(1);
                    continue;
                }
            };
            let source_range_start = geometry_source_ranges.len();
            push_geometry_source_ranges(
                pending,
                frozen.point_start,
                &mut geometry_source_ranges,
                &unit.limits,
            )?;
            facility_geometries.push(HirFacilityBandGeometry {
                canonical_frame: frame_key,
                facility_band: band,
                points: TableRange::try_from_usize(frozen.point_start, frozen.point_count)
                    .map_err(|overflow| {
                        arena_overflow(overflow, &unit.limits, Some(pending.source_span.clone()))
                    })?,
                source_ranges: TableRange::try_from_usize(
                    source_range_start,
                    geometry_source_ranges
                        .len()
                        .saturating_sub(source_range_start),
                )
                .map_err(|overflow| {
                    arena_overflow(overflow, &unit.limits, Some(pending.source_span.clone()))
                })?,
                source_span: pending.source_span.clone(),
            });
            facility_order_cursor = facility_order_cursor.saturating_add(1);
        }
        frames.get_mut(frame_key).facility_band_geometries = TableRange::try_from_usize(
            facility_geometry_start,
            facility_geometries
                .len()
                .saturating_sub(facility_geometry_start),
        )
        .map_err(|overflow| arena_overflow(overflow, &unit.limits, Some(frame_span)))?;
    }
    debug_assert_eq!(order_cursor, geometry_order.len());
    debug_assert_eq!(facility_order_cursor, facility_geometry_order.len());
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    for (edge_key, edge) in lane_edges.iter() {
        for successor in &lane_edge_references[edge.successors.as_usize_range()] {
            validate_spatial_connection(
                unit,
                &frames,
                lane_edges,
                &geometries,
                &points,
                &geometry_index_by_edge,
                edge_key,
                successor.target,
                &successor.source_span,
                compilation_profiles.as_ref().map(|(profiles, _)| *profiles),
                &mut diagnostics,
            );
        }
    }
    // RoadEditing junction-internal edge 不重复声明 successor。对 Synthetic 已由 successor
    // 覆盖的转换跳过，剩余 ManeuverPath 转换仍走完全相同的 frame/间隙/方向权威。
    for path in maneuver_paths {
        let path_edges = &maneuver_path_edges[path.edges.as_usize_range()];
        for pair in path_edges.windows(2) {
            let predecessor = lane_edges.get(pair[0].target);
            if lane_edge_references[predecessor.successors.as_usize_range()]
                .iter()
                .any(|successor| successor.target == pair[1].target)
            {
                continue;
            }
            validate_spatial_connection(
                unit,
                &frames,
                lane_edges,
                &geometries,
                &points,
                &geometry_index_by_edge,
                pair[0].target,
                pair[1].target,
                &pair[1].source_span,
                compilation_profiles.as_ref().map(|(profiles, _)| *profiles),
                &mut diagnostics,
            );
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    Ok(SpatialHir {
        geometry_profiles: compilation_profiles.map(|(profiles, _)| profiles),
        canonical_frames: frames.into_boxed_slice(),
        lane_edge_geometries: geometries.into_boxed_slice(),
        facility_band_geometries: facility_geometries.into_boxed_slice(),
        geometry_source_ranges: geometry_source_ranges.into_boxed_slice(),
        canonical_points: points.into_boxed_slice(),
        spatial_segments: segments.into_boxed_slice(),
        ..SpatialHir::default()
    })
}

/// 在 Traffic conflict HIR 和最终 edge length 均闭合后，把 owner-local region 接到
/// Spatial HIR。该后置步骤只增加可选空间记录，不参与或改写 passage 行为。
pub(crate) fn attach_conflict_zone_regions(
    unit: &CompilationUnit,
    counts: &SpatialCounts,
    module_lookup: &HashMap<Arc<str>, HirModuleKey>,
    conflict_zones: &[HirConflictZone],
    spatial: &mut SpatialHir,
) -> Result<(), DiagnosticBundle> {
    if counts.conflict_zone_regions == 0 {
        return Ok(());
    }
    debug_assert!(spatial.conflict_zone_regions.is_empty());
    debug_assert!(spatial.conflict_region_points.is_empty());

    let mut frame_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|value| matches!(value, TypedAstDeclaration::CanonicalFrame(_)))
            .count()
    }));
    for (index, frame) in spatial.canonical_frames.iter().enumerate() {
        frame_symbols.insert(
            frame.module,
            TypedAstEntityAddress::module_scoped(Arc::clone(&frame.stable_key)),
            HirCanonicalFrameKey::from_raw(u32::try_from(index).map_err(|_| {
                arena_overflow(
                    ArenaKeyOverflow,
                    &unit.limits,
                    Some(frame.source_span.clone()),
                )
            })?),
        );
    }
    let mut zone_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|value| matches!(value, TypedAstDeclaration::ConflictZone(_)))
            .count()
    }));
    for (index, zone) in conflict_zones.iter().enumerate() {
        zone_symbols.insert(
            zone.module,
            zone.source_address.clone(),
            HirConflictZoneKey::from_raw(u32::try_from(index).map_err(|_| {
                arena_overflow(
                    ArenaKeyOverflow,
                    &unit.limits,
                    Some(zone.source_span.clone()),
                )
            })?),
        );
    }

    let mut diagnostics =
        DiagnosticCollector::new(unit.limits.value(CompileLimitDimension::DiagnosticCount));
    let mut pending =
        Vec::with_capacity(count_to_usize(counts.conflict_zone_regions, &unit.limits)?);
    let mut owners = vec![None::<SourceLocation>; conflict_zones.len()];
    for (module_index, source_module) in unit.modules.iter().enumerate() {
        let module_order = u32::try_from(module_index).unwrap_or(u32::MAX);
        for source in source_module.conflict_zone_regions.iter() {
            let conflict_zone = resolve_region_reference(
                module_lookup,
                &zone_symbols,
                &source.conflict_zone,
                EntityKind::ConflictZone,
                source.conflict_zone.declaration_key(),
                &source.span,
                module_order,
                &mut diagnostics,
            );
            let canonical_frame = resolve_region_reference(
                module_lookup,
                &frame_symbols,
                &source.canonical_frame,
                EntityKind::ConflictZone,
                source.conflict_zone.declaration_key(),
                &source.span,
                module_order,
                &mut diagnostics,
            );
            let (Some(conflict_zone), Some(canonical_frame)) = (conflict_zone, canonical_frame)
            else {
                continue;
            };
            if let Some(first_span) = &owners[conflict_zone.index()] {
                let mut diagnostic = Diagnostic::invalid_spatial_geometry(
                    Some(&spatial.canonical_frames[canonical_frame.index()].stable_key),
                    &conflict_zones[conflict_zone.index()].stable_key,
                    None,
                    SpatialGeometryViolation::DuplicateConflictZoneRegion,
                    source.span.clone(),
                    Some(first_span.clone()),
                );
                diagnostic.set_canonical_module_order(module_order);
                diagnostics.push(diagnostic);
                continue;
            }
            owners[conflict_zone.index()] = Some(source.span.clone());
            pending.push(PendingConflictZoneRegion {
                module_order,
                source,
                conflict_zone,
                canonical_frame,
            });
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }
    pending.sort_unstable_by(|left, right| {
        conflict_zones[left.conflict_zone.index()]
            .stable_id
            .cmp(&conflict_zones[right.conflict_zone.index()].stable_id)
            .then_with(|| {
                spatial.canonical_frames[left.canonical_frame.index()]
                    .stable_id
                    .cmp(&spatial.canonical_frames[right.canonical_frame.index()].stable_id)
            })
    });

    let mut regions = Vec::with_capacity(pending.len());
    let mut points =
        Vec::with_capacity(count_to_usize(counts.conflict_region_points, &unit.limits)?);
    for item in pending {
        let zone = &conflict_zones[item.conflict_zone.index()];
        let frame = &spatial.canonical_frames[item.canonical_frame.index()];
        let frozen = match freeze_conflict_zone_region(
            item.source.min_y,
            item.source.max_y,
            &item.source.ring_xz,
            &mut points,
        ) {
            Ok(value) => value,
            Err(violation) => {
                let mut diagnostic = Diagnostic::invalid_spatial_geometry(
                    Some(&frame.stable_key),
                    &zone.stable_key,
                    None,
                    violation,
                    item.source.span.clone(),
                    None,
                );
                diagnostic.set_canonical_module_order(item.module_order);
                diagnostics.push(diagnostic);
                continue;
            }
        };
        regions.push(HirConflictZoneRegion {
            source_module: HirModuleKey::from_raw(item.module_order),
            conflict_zone: item.conflict_zone,
            canonical_frame: item.canonical_frame,
            min_y: frozen.min_y,
            max_y: frozen.max_y,
            ring_xz: TableRange::try_from_usize(frozen.point_start, frozen.point_count).map_err(
                |overflow| arena_overflow(overflow, &unit.limits, Some(item.source.span.clone())),
            )?,
            conflict_zone_source_location: unit.resolve_source_location_for_module(
                item.module_order,
                &item.source.conflict_zone.span,
            )?,
            canonical_frame_source_location: unit.resolve_source_location_for_module(
                item.module_order,
                &item.source.canonical_frame.span,
            )?,
            source_location: unit
                .resolve_source_location_for_module(item.module_order, &item.source.span)?,
            source_span: item.source.span.clone(),
        });
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }
    debug_assert_eq!(regions.len(), counts.conflict_zone_regions as usize);
    debug_assert_eq!(points.len(), counts.conflict_region_points as usize);
    spatial.conflict_zone_regions = regions.into_boxed_slice();
    spatial.conflict_region_points = points.into_boxed_slice();
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn resolve_region_reference<M, K: Copy>(
    module_lookup: &HashMap<Arc<str>, HirModuleKey>,
    symbols: &SymbolTable<K>,
    reference: &OwnedEntityReference<M>,
    source_kind: EntityKind,
    source_stable_key: &str,
    source_span: &SourceLocation,
    module_order: u32,
    diagnostics: &mut DiagnosticCollector,
) -> Option<K>
where
    M: laneflow_static_contract::EntityKindMarker,
{
    let target_module = module_lookup[reference.module_namespace.as_ref()];
    let Some(target) = symbols.get(target_module, &reference.target_address) else {
        let mut diagnostic = Diagnostic::unknown_owner_qualified_reference_target(
            source_kind,
            source_stable_key,
            &reference.module_namespace,
            reference.target_address.owner_local_keys(),
            reference.declaration_key(),
            reference.span.clone(),
            source_span.clone(),
        );
        diagnostic.set_canonical_module_order(module_order);
        diagnostics.push(diagnostic);
        return None;
    };
    Some(target)
}

#[allow(clippy::too_many_arguments)]
fn validate_spatial_connection(
    unit: &CompilationUnit,
    frames: &TypedArena<HirCanonicalFrameTag, HirCanonicalFrame>,
    lane_edges: &TypedArena<HirLaneEdgeTag, HirLaneEdge>,
    geometries: &[HirLaneEdgeGeometry],
    points: &[HirCanonicalPoint3F32],
    geometry_index_by_edge: &[Option<usize>],
    predecessor: HirLaneEdgeKey,
    successor: HirLaneEdgeKey,
    relation_span: &SourceLocation,
    profiles: Option<GeometryCompilationProfiles>,
    diagnostics: &mut DiagnosticCollector,
) {
    let geometry = &geometries[geometry_index_by_edge[predecessor.index()]
        .expect("complete spatial coverage must bind every predecessor")];
    let successor_geometry = &geometries[geometry_index_by_edge[successor.index()]
        .expect("complete spatial coverage must bind every successor")];
    let edge = lane_edges.get(predecessor);
    let successor_edge = lane_edges.get(successor);
    if geometry.canonical_frame != successor_geometry.canonical_frame {
        let mut diagnostic = Diagnostic::invalid_spatial_geometry(
            Some(&frames.get(geometry.canonical_frame).stable_key),
            &edge.stable_key,
            Some(&successor_edge.stable_key),
            SpatialGeometryViolation::ConnectedEdgesUseDifferentFrames,
            geometry.source_span.clone(),
            Some(relation_span.clone()),
        );
        diagnostic.set_canonical_module_order(edge.module.raw());
        diagnostics.push(diagnostic);
        return;
    }
    let end = points[geometry.points.as_usize_range().end - 1];
    let start = points[successor_geometry.points.as_usize_range().start];
    let distance = canonical_point_distance(end, start);
    if distance > SPATIAL_JOIN_POSITION_TOLERANCE_METERS {
        let distance = f64::from(distance);
        let tolerance = f64::from(SPATIAL_JOIN_POSITION_TOLERANCE_METERS);
        let mut diagnostic = Diagnostic::invalid_spatial_geometry(
            Some(&frames.get(geometry.canonical_frame).stable_key),
            &edge.stable_key,
            Some(&successor_edge.stable_key),
            SpatialGeometryViolation::DiscontinuousJoin {
                distance_bits: distance.to_bits(),
                tolerance_bits: tolerance.to_bits(),
            },
            geometry.source_span.clone(),
            Some(relation_span.clone()),
        );
        diagnostic.set_canonical_module_order(edge.module.raw());
        diagnostics.push(diagnostic);
        return;
    }
    let Some(profiles) = profiles.filter(|_| {
        unit.modules[geometry.source_module.index()]
            .geometry_profiles
            .is_some()
            || unit.modules[successor_geometry.source_module.index()]
                .geometry_profiles
                .is_some()
    }) else {
        return;
    };
    let predecessor_points = geometry.points.as_usize_range();
    let predecessor_end = points[predecessor_points.end - 1];
    let predecessor_start = points[predecessor_points.end - 2];
    let successor_points = successor_geometry.points.as_usize_range();
    let successor_start = points[successor_points.start];
    let successor_end = points[successor_points.start + 1];
    let outgoing = [
        f64::from(predecessor_end.x) - f64::from(predecessor_start.x),
        f64::from(predecessor_end.y) - f64::from(predecessor_start.y),
        f64::from(predecessor_end.z) - f64::from(predecessor_start.z),
    ];
    let incoming = [
        f64::from(successor_end.x) - f64::from(successor_start.x),
        f64::from(successor_end.y) - f64::from(successor_start.y),
        f64::from(successor_end.z) - f64::from(successor_start.z),
    ];
    let check = check_spatial_direction(outgoing, incoming, profiles.direction);
    if !check.accepted {
        let mut diagnostic = Diagnostic::invalid_spatial_geometry(
            Some(&frames.get(geometry.canonical_frame).stable_key),
            &edge.stable_key,
            Some(&successor_edge.stable_key),
            SpatialGeometryViolation::DirectionDiscontinuity {
                dot_bits: check.dot_bits,
                lhs_bits: check.lhs_bits,
                rhs_bits: check.rhs_bits,
            },
            geometry.source_span.clone(),
            Some(relation_span.clone()),
        );
        diagnostic.set_canonical_module_order(edge.module.raw());
        diagnostics.push(diagnostic);
    }
}

pub(crate) fn canonical_point_distance(a: HirCanonicalPoint3F32, b: HirCanonicalPoint3F32) -> f32 {
    (b.x - a.x).hypot(b.y - a.y).hypot(b.z - a.z)
}

fn push_geometry_source_ranges(
    pending: &PendingSpatialGeometry<'_>,
    point_start: usize,
    output: &mut Vec<HirGeometrySourceRange>,
    limits: &crate::CompileLimits,
) -> Result<(), DiagnosticBundle> {
    for range in pending.source_ranges {
        let local_start = usize::try_from(range.point_start)
            .expect("u32 source point offset fits usize on supported targets");
        let local_end = usize::try_from(range.point_end_exclusive)
            .expect("u32 source point end fits usize on supported targets");
        let points = TableRange::try_from_usize(
            point_start.saturating_add(local_start),
            local_end.saturating_sub(local_start),
        )
        .map_err(|overflow| arena_overflow(overflow, limits, Some(range.source.clone())))?;
        output.push(HirGeometrySourceRange {
            source_module: pending.source_module,
            points,
            source_segment_ordinal: range.source_segment_ordinal,
            source: range.source.clone(),
        });
    }
    Ok(())
}

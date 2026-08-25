//! 停车领域 HIR：停车区域、停车位、车道锚点与矩形几何的记录与构建。

use std::collections::HashMap;
use std::sync::Arc;

use laneflow_static_contract::{
    EntityKind, FieldTag, MAX_PARKING_LATERAL_OFFSET_ABS_MM, MAX_VEHICLE_LENGTH_MM,
    MIN_PARKING_LATERAL_OFFSET_ABS_MM, MIN_VEHICLE_LENGTH_MM, PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM,
    PARKING_HEADING_OFFSET_MAXIMUM_RADIANS, PARKING_HEADING_OFFSET_MINIMUM_RADIANS, ParkingAreaId,
    ParkingSpaceId, heading_f32_from_si, heading_f32_in_legal_closure, millimetres_from_si,
    millimetres_i32_from_si,
};

use crate::arena::{ArenaKeyOverflow, TableRange, TypedArena};
use crate::declaration::TypedAstDeclaration;
use crate::diagnostic::DiagnosticCollector;
use crate::identity::{IdentityFieldInput, IdentityRegistry};
use crate::module::ResolvedSourceLocation;
use crate::{
    CompilationUnit, CompileLimitDimension, Diagnostic, DiagnosticBundle, ParkingAnchorRole,
    ParkingGeometryField, ParkingGeometryViolation, SourceLocation,
};

use super::{
    CanonicalDeclarationSource, HirLaneEdge, HirLaneEdgeKey, HirLaneEdgeTag, HirModuleKey,
    HirParkingAreaKey, HirParkingAreaTag, HirParkingSpaceKey, HirParkingSpaceTag, ParkingCounts,
    SymbolTable, arena_overflow, count_to_usize, declaration_header, derive_identity,
    resolve_reference,
};

/// 停车区域的一个规范停车位成员。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HirParkingAreaSpace {
    pub(crate) parking_space: HirParkingSpaceKey,
}

/// 已证明至少拥有一个停车位成员的停车区域。
#[derive(Debug, PartialEq)]
pub(crate) struct HirParkingArea {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: ParkingAreaId,
    pub(crate) parking_spaces: TableRange<HirParkingAreaSpace>,
    pub(crate) source_span: SourceLocation,
}

/// 已解析到车道图边严格内部位置的停车锚点。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HirParkingLaneAnchor {
    pub(crate) lane_edge: HirLaneEdgeKey,
    pub(crate) progress_meters: f64,
    pub(crate) source_location: ResolvedSourceLocation,
}

/// 已验证的停车位矩形几何；数值保持来源 `f64` 精度。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HirParkingSpaceGeometry {
    pub(crate) lateral_offset_meters: f64,
    pub(crate) heading_offset_radians: f64,
    pub(crate) length_meters: f64,
    pub(crate) width_meters: f64,
}

/// 已闭合可选区域归属、入口/出口锚点和矩形几何的停车位。
#[derive(Debug, PartialEq)]
pub(crate) struct HirParkingSpace {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: ParkingSpaceId,
    pub(crate) parking_area: Option<HirParkingAreaKey>,
    pub(crate) parking_area_source_location: Option<ResolvedSourceLocation>,
    pub(crate) entry: HirParkingLaneAnchor,
    pub(crate) exit: HirParkingLaneAnchor,
    pub(crate) geometry: HirParkingSpaceGeometry,
    pub(crate) source_span: SourceLocation,
}

#[derive(Default)]
pub(crate) struct ParkingHir {
    pub(crate) parking_areas: Box<[HirParkingArea]>,
    pub(crate) parking_spaces: Box<[HirParkingSpace]>,
    pub(crate) parking_area_spaces: Box<[HirParkingAreaSpace]>,
}

#[allow(clippy::too_many_lines)]
pub(crate) fn build_parking_hir(
    unit: &CompilationUnit,
    counts: &ParkingCounts,
    module_lookup: &HashMap<Arc<str>, HirModuleKey>,
    lane_edges: &TypedArena<HirLaneEdgeTag, HirLaneEdge>,
    lane_edge_symbols: &SymbolTable<HirLaneEdgeKey>,
    identities: &mut IdentityRegistry,
) -> Result<ParkingHir, DiagnosticBundle> {
    if counts.entity_count() == 0 {
        return Ok(ParkingHir::default());
    }

    let mut areas = TypedArena::<HirParkingAreaTag, HirParkingArea>::with_capacity(count_to_usize(
        counts.areas,
        &unit.limits,
    )?);
    let mut spaces = TypedArena::<HirParkingSpaceTag, HirParkingSpace>::with_capacity(
        count_to_usize(counts.spaces, &unit.limits)?,
    );
    let mut area_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|declaration| matches!(declaration, TypedAstDeclaration::ParkingArea(_)))
            .count()
    }));
    let mut area_sources = Vec::with_capacity(count_to_usize(counts.areas, &unit.limits)?);
    let mut space_sources =
        Vec::<(u32, u32)>::with_capacity(count_to_usize(counts.spaces, &unit.limits)?);

    // ParkingArea 必须先完整登记，ParkingSpace 的可选归属因而允许前向和跨模块引用。
    // 两类实体仍分别按模块和稳定键规范排序，来源声明顺序不会进入身份或布局语义。
    for (module_index, source_module) in unit.modules.iter().enumerate() {
        let module_key = HirModuleKey::from_raw(
            u32::try_from(module_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        let module_order = u32::try_from(module_index).unwrap_or(u32::MAX);
        let mut area_indices: Vec<_> = source_module
            .declarations
            .iter()
            .enumerate()
            .filter_map(|(index, declaration)| {
                matches!(declaration, TypedAstDeclaration::ParkingArea(_)).then_some(index)
            })
            .collect();
        area_indices.sort_unstable_by_key(|index| {
            &declaration_header(&source_module.declarations[*index]).source_address
        });
        for declaration_index in area_indices {
            let TypedAstDeclaration::ParkingArea(source) =
                &source_module.declarations[declaration_index]
            else {
                unreachable!("parking area source filter admitted unrelated declaration")
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
                    FieldTag::ParkingAreaKey,
                    source.header.stable_key.as_bytes(),
                ),
            ];
            let stable_id = ParkingAreaId::from_untyped(derive_identity(
                unit,
                identities,
                module_index,
                EntityKind::ParkingArea,
                &source.header.stable_key,
                &source.header.span,
                &fields,
            )?);
            let key = areas
                .push(HirParkingArea {
                    module: module_key,
                    stable_key: Arc::clone(&source.header.stable_key),
                    stable_id,
                    parking_spaces: TableRange::empty(),
                    source_span: source.header.span.clone(),
                })
                .map_err(|overflow| {
                    arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                })?;
            area_symbols.insert(module_key, source.header.source_address.clone(), key);
            area_sources.push(CanonicalDeclarationSource {
                source_module_index: module_order,
                declaration_index: u32::try_from(declaration_index)
                    .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
                hir_key: key,
            });
        }

        let mut indices: Vec<_> = source_module
            .declarations
            .iter()
            .enumerate()
            .filter_map(|(index, declaration)| {
                matches!(declaration, TypedAstDeclaration::ParkingSpace(_)).then_some(index)
            })
            .collect();
        indices.sort_unstable_by_key(|index| {
            &declaration_header(&source_module.declarations[*index]).source_address
        });
        for declaration_index in indices {
            space_sources.push((
                module_order,
                u32::try_from(declaration_index)
                    .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
            ));
        }
    }

    let mut diagnostics =
        DiagnosticCollector::new(unit.limits.value(CompileLimitDimension::DiagnosticCount));
    let mut area_has_member = vec![false; areas.len()];
    let mut memberships = Vec::<(HirParkingAreaKey, HirParkingSpaceKey)>::with_capacity(
        count_to_usize(counts.memberships, &unit.limits)?,
    );

    for (module_order, declaration_index) in space_sources {
        let module_index = module_order as usize;
        let source_module = &unit.modules[module_index];
        let TypedAstDeclaration::ParkingSpace(source) =
            &source_module.declarations[declaration_index as usize]
        else {
            unreachable!("canonical ParkingSpace source changed kind")
        };
        let module_key = HirModuleKey::from_raw(module_order);
        let fields = [
            IdentityFieldInput::new(
                FieldTag::AuthoringNamespaceId,
                source_module
                    .descriptor()
                    .authoring_namespace_id()
                    .as_bytes(),
            ),
            IdentityFieldInput::new(
                FieldTag::ParkingSpaceKey,
                source.header.stable_key.as_bytes(),
            ),
        ];
        let stable_id = ParkingSpaceId::from_untyped(derive_identity(
            unit,
            identities,
            module_index,
            EntityKind::ParkingSpace,
            &source.header.stable_key,
            &source.header.span,
            &fields,
        )?);

        let parking_area = source.parking_area.as_ref().and_then(|reference| {
            let area = resolve_reference(
                module_lookup,
                &area_symbols,
                reference,
                EntityKind::ParkingSpace,
                &source.header,
                module_order,
                &mut diagnostics,
            );
            if let Some(area) = area {
                // 区域孤立性由声明关系判断；成员自己的其他字段失败不应产生级联 orphan。
                area_has_member[area.index()] = true;
            }
            area
        });
        let entry_edge = resolve_reference(
            module_lookup,
            lane_edge_symbols,
            &source.entry.lane_edge,
            EntityKind::ParkingSpace,
            &source.header,
            module_order,
            &mut diagnostics,
        );
        let exit_edge = resolve_reference(
            module_lookup,
            lane_edge_symbols,
            &source.exit.lane_edge,
            EntityKind::ParkingSpace,
            &source.header,
            module_order,
            &mut diagnostics,
        );

        for (role, anchor, edge) in [
            (ParkingAnchorRole::Entry, &source.entry, entry_edge),
            (ParkingAnchorRole::Exit, &source.exit, exit_edge),
        ] {
            let Some(edge) = edge else { continue };
            let edge_length = lane_edges.get(edge).length_meters;
            let progress = anchor.progress_meters;
            let progress_mm = millimetres_from_si(progress);
            let length_mm = millimetres_from_si(edge_length);
            let min_progress_mm = PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM;
            let max_progress_mm = length_mm
                .unwrap_or(0)
                .saturating_sub(PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM);
            if !matches!(
                (progress_mm, length_mm),
                (Some(progress), Some(_))
                    if (min_progress_mm..=max_progress_mm).contains(&progress)
            ) {
                let mut diagnostic = Diagnostic::invalid_parking_anchor_progress(
                    &source.header.stable_key,
                    role,
                    &lane_edges.get(edge).stable_key,
                    progress,
                    edge_length,
                    min_progress_mm,
                    max_progress_mm,
                    anchor.lane_edge.span.clone(),
                );
                diagnostic.set_canonical_module_order(module_order);
                diagnostics.push(diagnostic);
            }
        }

        let geometry = source.geometry;
        for (field, value, violation) in [
            (
                ParkingGeometryField::LateralOffsetMeters,
                geometry.lateral_offset_meters,
                parking_lateral_violation(geometry.lateral_offset_meters),
            ),
            (
                ParkingGeometryField::HeadingOffsetRadians,
                geometry.heading_offset_radians,
                parking_heading_violation(geometry.heading_offset_radians),
            ),
            (
                ParkingGeometryField::LengthMeters,
                geometry.length_meters,
                parking_extent_violation(geometry.length_meters),
            ),
            (
                ParkingGeometryField::WidthMeters,
                geometry.width_meters,
                parking_extent_violation(geometry.width_meters),
            ),
        ] {
            if let Some(violation) = violation {
                let mut diagnostic = Diagnostic::invalid_parking_space_geometry(
                    &source.header.stable_key,
                    field,
                    value,
                    violation,
                    source.header.span.clone(),
                );
                diagnostic.set_canonical_module_order(module_order);
                diagnostics.push(diagnostic);
            }
        }

        let (Some(entry_edge), Some(exit_edge)) = (entry_edge, exit_edge) else {
            continue;
        };
        let parking_area_source_location = match (&source.parking_area, parking_area) {
            (Some(reference), Some(_)) => {
                Some(unit.resolve_source_location_for_module(module_order, &reference.span)?)
            }
            _ => None,
        };
        let space_key = spaces
            .push(HirParkingSpace {
                module: module_key,
                stable_key: Arc::clone(&source.header.stable_key),
                stable_id,
                parking_area,
                parking_area_source_location,
                entry: HirParkingLaneAnchor {
                    lane_edge: entry_edge,
                    progress_meters: source.entry.progress_meters,
                    source_location: unit.resolve_source_location_for_module(
                        module_order,
                        &source.entry.lane_edge.span,
                    )?,
                },
                exit: HirParkingLaneAnchor {
                    lane_edge: exit_edge,
                    progress_meters: source.exit.progress_meters,
                    source_location: unit.resolve_source_location_for_module(
                        module_order,
                        &source.exit.lane_edge.span,
                    )?,
                },
                geometry: HirParkingSpaceGeometry {
                    lateral_offset_meters: geometry.lateral_offset_meters,
                    heading_offset_radians: geometry.heading_offset_radians,
                    length_meters: geometry.length_meters,
                    width_meters: geometry.width_meters,
                },
                source_span: source.header.span.clone(),
            })
            .map_err(|overflow| {
                arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
            })?;
        if let Some(area) = parking_area {
            memberships.push((area, space_key));
        }
    }

    for location in &area_sources {
        if !area_has_member[location.hir_key.index()] {
            let area = areas.get(location.hir_key);
            let mut diagnostic =
                Diagnostic::orphan_parking_area(&area.stable_key, area.source_span.clone());
            diagnostic.set_canonical_module_order(location.source_module_index);
            diagnostics.push(diagnostic);
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    // 归属是集合语义。按区域 StableId、停车位 StableId 冻结反向成员表，避免来源排列
    // 改变 LIR 或语义摘要；独立停车位不会出现在该表中。
    memberships.sort_unstable_by_key(|(area, space)| {
        (areas.get(*area).stable_id, spaces.get(*space).stable_id)
    });
    let area_spaces = memberships
        .iter()
        .map(|(_, parking_space)| HirParkingAreaSpace {
            parking_space: *parking_space,
        })
        .collect::<Vec<_>>();
    let mut area_ranges = vec![(0_usize, 0_usize); areas.len()];
    let mut cursor = 0_usize;
    while cursor < memberships.len() {
        let area = memberships[cursor].0;
        let start = cursor;
        while cursor < memberships.len() && memberships[cursor].0 == area {
            cursor = cursor.saturating_add(1);
        }
        area_ranges[area.index()] = (start, cursor.saturating_sub(start));
    }
    for (area_index, (start, count)) in area_ranges.iter().copied().enumerate() {
        let area_key = HirParkingAreaKey::from_raw(
            u32::try_from(area_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        let source_span = areas.get(area_key).source_span.clone();
        areas.get_mut(area_key).parking_spaces = TableRange::try_from_usize(start, count)
            .map_err(|overflow| arena_overflow(overflow, &unit.limits, Some(source_span)))?;
    }

    Ok(ParkingHir {
        parking_areas: areas.into_boxed_slice(),
        parking_spaces: spaces.into_boxed_slice(),
        parking_area_spaces: area_spaces.into_boxed_slice(),
    })
}

fn parking_extent_violation(value: f64) -> Option<ParkingGeometryViolation> {
    closed_parking_mm(value, MIN_VEHICLE_LENGTH_MM, MAX_VEHICLE_LENGTH_MM)
}

fn parking_lateral_violation(value: f64) -> Option<ParkingGeometryViolation> {
    if !value.is_finite() {
        return Some(ParkingGeometryViolation::NotFinite);
    }
    let Some(actual_mm) = millimetres_i32_from_si(value) else {
        return Some(ParkingGeometryViolation::QuantizeFailed);
    };
    let actual_abs_mm = actual_mm.unsigned_abs();
    if actual_abs_mm < MIN_PARKING_LATERAL_OFFSET_ABS_MM
        || actual_abs_mm > MAX_PARKING_LATERAL_OFFSET_ABS_MM
    {
        return Some(
            ParkingGeometryViolation::AbsoluteOutsideClosedMillimetreRange {
                min_abs_mm: MIN_PARKING_LATERAL_OFFSET_ABS_MM,
                max_abs_mm: MAX_PARKING_LATERAL_OFFSET_ABS_MM,
                actual_abs_mm,
            },
        );
    }
    None
}

fn parking_heading_violation(value: f64) -> Option<ParkingGeometryViolation> {
    let Some(heading) = heading_f32_from_si(value) else {
        return Some(if value.is_finite() {
            ParkingGeometryViolation::QuantizeFailed
        } else {
            ParkingGeometryViolation::NotFinite
        });
    };
    if heading_f32_in_legal_closure(heading) {
        None
    } else {
        Some(ParkingGeometryViolation::OutsideHalfOpenRange {
            minimum_inclusive_bits: PARKING_HEADING_OFFSET_MINIMUM_RADIANS.to_bits(),
            maximum_exclusive_bits: PARKING_HEADING_OFFSET_MAXIMUM_RADIANS.to_bits(),
        })
    }
}

fn closed_parking_mm(value: f64, min_mm: u32, max_mm: u32) -> Option<ParkingGeometryViolation> {
    if !value.is_finite() {
        return Some(ParkingGeometryViolation::NotFinite);
    }
    let Some(actual_mm) = millimetres_from_si(value) else {
        return Some(ParkingGeometryViolation::QuantizeFailed);
    };
    (actual_mm < min_mm || actual_mm > max_mm).then_some(
        ParkingGeometryViolation::OutsideClosedMillimetreRange {
            min_mm,
            max_mm,
            actual_mm,
        },
    )
}

//! 停车领域 HIR：停车区域、停车位、车道锚点与矩形几何的记录与构建。

use std::collections::HashMap;
use std::sync::Arc;

use laneflow_static_contract::{
    EntityKind, FieldTag, PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM, ParkingFacilityId, ParkingSpaceId,
};

use crate::arena::{ArenaKeyOverflow, TableRange, TypedArena};
use crate::declaration::TypedAstDeclaration;
use crate::diagnostic::DiagnosticCollector;
use crate::identity::{IdentityFieldInput, IdentityRegistry};
use crate::module::ResolvedSourceLocation;
use crate::{
    CompilationUnit, CompileLimitDimension, Diagnostic, DiagnosticBundle, ParkingAnchorRole,
    SourceLocation,
};

use super::{
    CanonicalDeclarationSource, HirLaneEdge, HirLaneEdgeKey, HirLaneEdgeTag, HirModuleKey,
    HirParkingFacilityKey, HirParkingFacilityTag, HirParkingSpaceKey, HirParkingSpaceTag,
    ParkingCounts, SymbolTable, arena_overflow, count_to_usize, declaration_header,
    derive_identity, resolve_reference,
};

/// 停车区域的一个规范停车位成员。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HirParkingFacilitySpace {
    pub(crate) parking_space: HirParkingSpaceKey,
}

/// 已证明总容量非零的停车设施。
#[derive(Debug, PartialEq)]
pub(crate) struct HirParkingFacility {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: ParkingFacilityId,
    pub(crate) parking_spaces: TableRange<HirParkingFacilitySpace>,
    pub(crate) virtual_capacity: u32,
    pub(crate) virtual_entries: TableRange<HirParkingLaneAnchor>,
    pub(crate) virtual_exits: TableRange<HirParkingLaneAnchor>,
    pub(crate) source_span: SourceLocation,
}

/// 已解析到车道图边严格内部位置的停车锚点。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HirParkingLaneAnchor {
    pub(crate) lane_edge: HirLaneEdgeKey,
    pub(crate) progress_mm: u32,
    pub(crate) source_location: ResolvedSourceLocation,
}

/// 已验证的停车位矩形几何；交通一维为毫米，朝向为受检 `f32`。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HirParkingSpaceGeometry {
    pub(crate) lateral_offset_mm: i32,
    pub(crate) heading_offset_radians: f32,
    pub(crate) length_mm: u32,
    pub(crate) width_mm: u32,
}

/// 已闭合可选区域归属、入口/出口锚点和矩形几何的停车位。
#[derive(Debug, PartialEq)]
pub(crate) struct HirParkingSpace {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: ParkingSpaceId,
    pub(crate) parking_facility: Option<HirParkingFacilityKey>,
    pub(crate) parking_facility_source_location: Option<ResolvedSourceLocation>,
    pub(crate) entry: HirParkingLaneAnchor,
    pub(crate) exit: HirParkingLaneAnchor,
    pub(crate) geometry: HirParkingSpaceGeometry,
    pub(crate) source_span: SourceLocation,
}

#[derive(Default)]
pub(crate) struct ParkingHir {
    pub(crate) parking_facilities: Box<[HirParkingFacility]>,
    pub(crate) parking_spaces: Box<[HirParkingSpace]>,
    pub(crate) parking_facility_spaces: Box<[HirParkingFacilitySpace]>,
    pub(crate) parking_facility_virtual_entries: Box<[HirParkingLaneAnchor]>,
    pub(crate) parking_facility_virtual_exits: Box<[HirParkingLaneAnchor]>,
}

#[allow(clippy::too_many_lines)]
pub(crate) fn build_parking_hir(
    unit: &CompilationUnit,
    counts: &ParkingCounts,
    module_lookup: &HashMap<Arc<str>, HirModuleKey>,
    lane_edges: &TypedArena<HirLaneEdgeTag, HirLaneEdge>,
    lane_edge_symbols: &SymbolTable<HirLaneEdgeKey>,
    defer_emitted_length_close: bool,
    identities: &mut IdentityRegistry,
) -> Result<ParkingHir, DiagnosticBundle> {
    if counts.entity_count() == 0 {
        return Ok(ParkingHir::default());
    }

    let mut areas = TypedArena::<HirParkingFacilityTag, HirParkingFacility>::with_capacity(
        count_to_usize(counts.areas, &unit.limits)?,
    );
    let mut spaces = TypedArena::<HirParkingSpaceTag, HirParkingSpace>::with_capacity(
        count_to_usize(counts.spaces, &unit.limits)?,
    );
    let mut area_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|declaration| matches!(declaration, TypedAstDeclaration::ParkingFacility(_)))
            .count()
    }));
    let mut area_sources = Vec::with_capacity(count_to_usize(counts.areas, &unit.limits)?);
    let mut space_sources =
        Vec::<(u32, u32)>::with_capacity(count_to_usize(counts.spaces, &unit.limits)?);

    // ParkingFacility 必须先完整登记，ParkingSpace 的可选归属因而允许前向和跨模块引用。
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
                matches!(declaration, TypedAstDeclaration::ParkingFacility(_)).then_some(index)
            })
            .collect();
        area_indices.sort_unstable_by_key(|index| {
            &declaration_header(&source_module.declarations[*index]).source_address
        });
        for declaration_index in area_indices {
            let TypedAstDeclaration::ParkingFacility(source) =
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
                    FieldTag::ParkingFacilityKey,
                    source.header.stable_key.as_bytes(),
                ),
            ];
            let stable_id = ParkingFacilityId::from_untyped(derive_identity(
                unit,
                identities,
                module_index,
                EntityKind::ParkingFacility,
                &source.header.stable_key,
                &source.header.span,
                &fields,
            )?);
            let key = areas
                .push(HirParkingFacility {
                    module: module_key,
                    stable_key: Arc::clone(&source.header.stable_key),
                    stable_id,
                    parking_spaces: TableRange::empty(),
                    virtual_capacity: source.virtual_capacity,
                    virtual_entries: TableRange::empty(),
                    virtual_exits: TableRange::empty(),
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
    let mut virtual_entries =
        Vec::with_capacity(count_to_usize(counts.virtual_entries, &unit.limits)?);
    let mut virtual_exits = Vec::with_capacity(count_to_usize(counts.virtual_exits, &unit.limits)?);
    for location in &area_sources {
        let source_module = &unit.modules[location.source_module_index as usize];
        let TypedAstDeclaration::ParkingFacility(source) =
            &source_module.declarations[location.declaration_index as usize]
        else {
            unreachable!("canonical ParkingFacility source changed kind")
        };
        for (anchors, destination, is_entry) in [
            (&source.virtual_entries, &mut virtual_entries, true),
            (&source.virtual_exits, &mut virtual_exits, false),
        ] {
            let start = destination.len();
            for anchor in anchors.iter() {
                let Some(lane_edge) = resolve_reference(
                    module_lookup,
                    lane_edge_symbols,
                    &anchor.lane_edge,
                    EntityKind::ParkingFacility,
                    &source.header,
                    location.source_module_index,
                    &mut diagnostics,
                ) else {
                    continue;
                };
                destination.push(HirParkingLaneAnchor {
                    lane_edge,
                    progress_mm: anchor.progress_mm,
                    source_location: unit.resolve_source_location_for_module(
                        location.source_module_index,
                        &anchor.lane_edge.span,
                    )?,
                });
            }
            destination[start..].sort_unstable_by_key(|anchor| {
                (
                    lane_edges.get(anchor.lane_edge).stable_id,
                    anchor.progress_mm,
                )
            });
            let role = if is_entry {
                ParkingAnchorRole::VirtualEntry
            } else {
                ParkingAnchorRole::VirtualExit
            };
            let mut last_duplicate = None;
            for pair in destination[start..].windows(2) {
                let first_edge = lane_edges.get(pair[0].lane_edge);
                let duplicate_edge = lane_edges.get(pair[1].lane_edge);
                if first_edge.stable_id == duplicate_edge.stable_id
                    && pair[0].progress_mm == pair[1].progress_mm
                {
                    let duplicate = (duplicate_edge.stable_id.into_untyped(), pair[1].progress_mm);
                    if last_duplicate == Some(duplicate) {
                        continue;
                    }
                    last_duplicate = Some(duplicate);
                    let mut diagnostic = Diagnostic::duplicate_parking_facility_virtual_anchor(
                        &source.header.stable_key,
                        role,
                        duplicate.0,
                        duplicate.1,
                        source.header.span.clone(),
                    );
                    diagnostic.set_canonical_module_order(location.source_module_index);
                    diagnostics.push(diagnostic);
                }
            }
            let count = destination.len().saturating_sub(start);
            let range = TableRange::try_from_usize(start, count).map_err(|overflow| {
                arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
            })?;
            let facility = areas.get_mut(location.hir_key);
            if is_entry {
                facility.virtual_entries = range;
            } else {
                facility.virtual_exits = range;
            }
        }
    }
    let mut area_has_member = areas
        .iter()
        .map(|(_, facility)| facility.virtual_capacity != 0)
        .collect::<Vec<_>>();
    let mut memberships = Vec::<(HirParkingFacilityKey, HirParkingSpaceKey)>::with_capacity(
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

        let parking_facility = source.parking_facility.as_ref().and_then(|reference| {
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

        let geometry = source.geometry;

        let (Some(entry_edge), Some(exit_edge)) = (entry_edge, exit_edge) else {
            continue;
        };
        let parking_facility_source_location = match (&source.parking_facility, parking_facility) {
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
                parking_facility,
                parking_facility_source_location,
                entry: HirParkingLaneAnchor {
                    lane_edge: entry_edge,
                    progress_mm: source.entry.progress_mm,
                    source_location: unit.resolve_source_location_for_module(
                        module_order,
                        &source.entry.lane_edge.span,
                    )?,
                },
                exit: HirParkingLaneAnchor {
                    lane_edge: exit_edge,
                    progress_mm: source.exit.progress_mm,
                    source_location: unit.resolve_source_location_for_module(
                        module_order,
                        &source.exit.lane_edge.span,
                    )?,
                },
                geometry: HirParkingSpaceGeometry {
                    lateral_offset_mm: geometry.lateral_offset_mm,
                    heading_offset_radians: geometry.heading_offset_radians,
                    length_mm: geometry.length_mm,
                    width_mm: geometry.width_mm,
                },
                source_span: source.header.span.clone(),
            })
            .map_err(|overflow| {
                arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
            })?;
        if let Some(area) = parking_facility {
            memberships.push((area, space_key));
        }
    }

    for location in &area_sources {
        if !area_has_member[location.hir_key.index()] {
            let area = areas.get(location.hir_key);
            let mut diagnostic =
                Diagnostic::orphan_parking_facility(&area.stable_key, area.source_span.clone());
            diagnostic.set_canonical_module_order(location.source_module_index);
            diagnostics.push(diagnostic);
        }
    }
    if !defer_emitted_length_close {
        diagnose_parking_anchors_against_emitted_length(
            spaces.iter().map(|(_, space)| space),
            areas.iter().map(|(_, facility)| facility),
            &virtual_entries,
            &virtual_exits,
            lane_edges,
            &mut diagnostics,
        );
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
        .map(|(_, parking_space)| HirParkingFacilitySpace {
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
        let area_key = HirParkingFacilityKey::from_raw(
            u32::try_from(area_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        let source_span = areas.get(area_key).source_span.clone();
        areas.get_mut(area_key).parking_spaces = TableRange::try_from_usize(start, count)
            .map_err(|overflow| arena_overflow(overflow, &unit.limits, Some(source_span)))?;
    }

    Ok(ParkingHir {
        parking_facilities: areas.into_boxed_slice(),
        parking_spaces: spaces.into_boxed_slice(),
        parking_facility_spaces: area_spaces.into_boxed_slice(),
        parking_facility_virtual_entries: virtual_entries.into_boxed_slice(),
        parking_facility_virtual_exits: virtual_exits.into_boxed_slice(),
    })
}

/// 相对空间冻结提交后的 IR `length_mm` 关闭停车锚点。
pub(super) fn close_parking_anchors_to_emitted_length_mm(
    parking: &ParkingHir,
    lane_edges: &TypedArena<HirLaneEdgeTag, HirLaneEdge>,
    diagnostic_limit: u64,
) -> Result<(), DiagnosticBundle> {
    if parking.parking_spaces.is_empty()
        && parking.parking_facility_virtual_entries.is_empty()
        && parking.parking_facility_virtual_exits.is_empty()
    {
        return Ok(());
    }
    let mut diagnostics = DiagnosticCollector::new(diagnostic_limit);
    diagnose_parking_anchors_against_emitted_length(
        parking.parking_spaces.iter(),
        parking.parking_facilities.iter(),
        &parking.parking_facility_virtual_entries,
        &parking.parking_facility_virtual_exits,
        lane_edges,
        &mut diagnostics,
    );
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics.finish())
    }
}

fn diagnose_parking_anchors_against_emitted_length<'a>(
    parking_spaces: impl IntoIterator<Item = &'a HirParkingSpace>,
    parking_facilities: impl IntoIterator<Item = &'a HirParkingFacility>,
    virtual_entries: &[HirParkingLaneAnchor],
    virtual_exits: &[HirParkingLaneAnchor],
    lane_edges: &TypedArena<HirLaneEdgeTag, HirLaneEdge>,
    diagnostics: &mut DiagnosticCollector,
) {
    for space in parking_spaces {
        for (role, anchor) in [
            (ParkingAnchorRole::Entry, &space.entry),
            (ParkingAnchorRole::Exit, &space.exit),
        ] {
            let edge = lane_edges.get(anchor.lane_edge);
            let length_mm = edge.length_mm;
            let progress_mm = anchor.progress_mm;
            let min_progress_mm = PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM;
            let max_progress_mm = length_mm.saturating_sub(PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM);
            if !(min_progress_mm..=max_progress_mm).contains(&progress_mm) {
                let mut diagnostic = Diagnostic::invalid_parking_anchor_progress(
                    &space.stable_key,
                    role,
                    &edge.stable_key,
                    f64::from(progress_mm) / 1_000.0,
                    f64::from(length_mm) / 1_000.0,
                    min_progress_mm,
                    max_progress_mm,
                    space.source_span.clone(),
                );
                diagnostic.set_canonical_module_order(space.module.raw());
                diagnostics.push(diagnostic);
            }
        }
    }
    for facility in parking_facilities {
        for (role, anchors) in [
            (
                ParkingAnchorRole::VirtualEntry,
                &virtual_entries[facility.virtual_entries.as_usize_range()],
            ),
            (
                ParkingAnchorRole::VirtualExit,
                &virtual_exits[facility.virtual_exits.as_usize_range()],
            ),
        ] {
            for anchor in anchors {
                let edge = lane_edges.get(anchor.lane_edge);
                let min_progress_mm = PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM;
                let max_progress_mm = edge
                    .length_mm
                    .saturating_sub(PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM);
                if !(min_progress_mm..=max_progress_mm).contains(&anchor.progress_mm) {
                    let mut diagnostic = Diagnostic::invalid_parking_anchor_progress(
                        &facility.stable_key,
                        role,
                        &edge.stable_key,
                        f64::from(anchor.progress_mm) / 1_000.0,
                        f64::from(edge.length_mm) / 1_000.0,
                        min_progress_mm,
                        max_progress_mm,
                        facility.source_span.clone(),
                    );
                    diagnostic.set_canonical_module_order(facility.module.raw());
                    diagnostics.push(diagnostic);
                }
            }
        }
    }
}

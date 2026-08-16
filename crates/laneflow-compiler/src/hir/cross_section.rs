//! 横断面（cross section）领域 HIR：道路走廊、道路区段、编制车道、车道组与设施带的
//! 记录与构建。

use std::collections::HashMap;
use std::sync::Arc;

use laneflow_static_contract::{
    AuthoringLaneId, EntityKind, FacilityBandId, FieldTag, LaneGroupId, RoadCorridorId,
    RoadSectionId, StableId128,
};

use crate::arena::{ArenaKeyOverflow, TableRange, TypedArena};
use crate::declaration::{
    OwnedCorridorElementReference, TypedAstDeclaration, TypedAstEntityAddress,
};
use crate::diagnostic::DiagnosticCollector;
use crate::identity::{IdentityFieldInput, IdentityRegistry};
use crate::module::ResolvedSourceLocation;
use crate::{CompilationUnit, CompileLimitDimension, Diagnostic, DiagnosticBundle, SourceLocation};

use super::{
    CanonicalDeclarationSource, CrossSectionCounts, HirAuthoringLaneKey, HirAuthoringLaneTag,
    HirFacilityBandKey, HirFacilityBandTag, HirLaneEdge, HirLaneEdgeKey, HirLaneEdgeReference,
    HirLaneEdgeTag, HirLaneGroupKey, HirLaneGroupTag, HirModuleKey, HirRoadCorridorKey,
    HirRoadCorridorTag, HirRoadSectionKey, HirRoadSectionTag, SymbolTable, arena_overflow,
    count_to_usize, declaration_header, derive_identity, register_owner, resolve_reference,
};

/// 道路走廊有序横断面中的已解析异构成员。
#[derive(Debug, PartialEq)]
pub(crate) enum HirCorridorElement {
    RoadSection {
        road_section: HirRoadSectionKey,
        source_location: ResolvedSourceLocation,
    },
    FacilityBand {
        facility_band: HirFacilityBandKey,
        source_location: ResolvedSourceLocation,
    },
}

/// 已证明参考区段成员性与成员唯一所有权的道路走廊。
#[derive(Debug, PartialEq)]
pub(crate) struct HirRoadCorridor {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: RoadCorridorId,
    pub(crate) reference_section: HirRoadSectionKey,
    pub(crate) elements: TableRange<HirCorridorElement>,
    pub(crate) source_span: SourceLocation,
}

/// 已闭合到唯一道路走廊父项的道路区段。
#[derive(Debug, PartialEq)]
pub(crate) struct HirRoadSection {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) source_address: TypedAstEntityAddress,
    pub(crate) stable_id: RoadSectionId,
    pub(crate) road_corridor: HirRoadCorridorKey,
    pub(crate) kind_id: Arc<str>,
    pub(crate) lanes: TableRange<HirAuthoringLane>,
    pub(crate) source_span: SourceLocation,
}

/// 编制车道覆盖链中的一项已解析车道图边及其来源位置。
#[derive(Debug, PartialEq)]
pub(crate) struct HirAuthoringLaneEdge {
    pub(crate) target: HirLaneEdgeKey,
    pub(crate) source_span: SourceLocation,
}

/// 已解析父区段、覆盖链和可选车道组的编制车道。
#[derive(Debug, PartialEq)]
pub(crate) struct HirAuthoringLane {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: AuthoringLaneId,
    pub(crate) road_section: HirRoadSectionKey,
    pub(crate) edge_chain: TableRange<HirAuthoringLaneEdge>,
    pub(crate) lane_group: Option<HirLaneGroupKey>,
    pub(crate) lane_group_source_location: Option<ResolvedSourceLocation>,
    pub(crate) source_span: SourceLocation,
}

/// 车道组成员表中的一条编制车道引用。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HirLaneGroupMember {
    pub(crate) lane: HirAuthoringLaneKey,
}

/// 已证明所有成员与父区段一致且非空的车道组。
#[derive(Debug, PartialEq)]
pub(crate) struct HirLaneGroup {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) source_address: TypedAstEntityAddress,
    pub(crate) stable_id: LaneGroupId,
    pub(crate) road_section: HirRoadSectionKey,
    pub(crate) members: TableRange<HirLaneGroupMember>,
    pub(crate) source_span: SourceLocation,
}

/// 已闭合到唯一道路走廊父项的非遍历设施带。
#[derive(Debug, PartialEq)]
pub(crate) struct HirFacilityBand {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) source_address: TypedAstEntityAddress,
    pub(crate) stable_id: FacilityBandId,
    pub(crate) road_corridor: HirRoadCorridorKey,
    pub(crate) kind_id: Arc<str>,
    pub(crate) source_span: SourceLocation,
}

#[derive(Clone, Copy)]
pub(crate) struct CanonicalAuthoringLaneSource {
    source_module_index: u32,
    declaration_index: u32,
    lane_index: u32,
    hir_key: HirAuthoringLaneKey,
}

#[derive(Default)]
pub(crate) struct CrossSectionHir {
    pub(crate) road_corridors: Box<[HirRoadCorridor]>,
    pub(crate) corridor_elements: Box<[HirCorridorElement]>,
    pub(crate) road_sections: Box<[HirRoadSection]>,
    pub(crate) authoring_lanes: Box<[HirAuthoringLane]>,
    pub(crate) authoring_lane_edges: Box<[HirAuthoringLaneEdge]>,
    pub(crate) lane_groups: Box<[HirLaneGroup]>,
    pub(crate) lane_group_members: Box<[HirLaneGroupMember]>,
    pub(crate) facility_bands: Box<[HirFacilityBand]>,
}

#[allow(clippy::too_many_lines)]
pub(crate) fn build_cross_section_hir(
    unit: &CompilationUnit,
    counts: &CrossSectionCounts,
    module_lookup: &HashMap<Arc<str>, HirModuleKey>,
    lane_edges: &TypedArena<HirLaneEdgeTag, HirLaneEdge>,
    lane_edge_references: &[HirLaneEdgeReference],
    lane_edge_symbols: &SymbolTable<HirLaneEdgeKey>,
    identities: &mut IdentityRegistry,
) -> Result<CrossSectionHir, DiagnosticBundle> {
    if counts.entity_count() == 0 {
        return Ok(CrossSectionHir::default());
    }
    // 只为会被引用解析访问的实体建立符号表，并按实体类别精确预留容量。RoadCorridor
    // 与 AuthoringLane 在本切片中没有按键引用消费者；为它们建立查找表只会增加峰值内存。
    let mut section_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|declaration| matches!(declaration, TypedAstDeclaration::RoadSection(_)))
            .count()
    }));
    let mut group_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|declaration| matches!(declaration, TypedAstDeclaration::LaneGroup(_)))
            .count()
    }));
    let mut band_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|declaration| matches!(declaration, TypedAstDeclaration::FacilityBand(_)))
            .count()
    }));

    let mut corridors = TypedArena::<HirRoadCorridorTag, HirRoadCorridor>::with_capacity(
        count_to_usize(counts.road_corridors, &unit.limits)?,
    );
    let mut sections = TypedArena::<HirRoadSectionTag, HirRoadSection>::with_capacity(
        count_to_usize(counts.road_sections, &unit.limits)?,
    );
    let mut lanes = TypedArena::<HirAuthoringLaneTag, HirAuthoringLane>::with_capacity(
        count_to_usize(counts.authoring_lanes, &unit.limits)?,
    );
    let mut groups = TypedArena::<HirLaneGroupTag, HirLaneGroup>::with_capacity(count_to_usize(
        counts.lane_groups,
        &unit.limits,
    )?);
    let mut bands = TypedArena::<HirFacilityBandTag, HirFacilityBand>::with_capacity(
        count_to_usize(counts.facility_bands, &unit.limits)?,
    );
    let mut corridor_sources = Vec::with_capacity(corridors_capacity(counts, &unit.limits)?);
    let mut section_sources = Vec::with_capacity(sections_capacity(counts, &unit.limits)?);
    let mut lane_sources = Vec::with_capacity(lanes_capacity(counts, &unit.limits)?);
    let mut group_sources = Vec::with_capacity(groups_capacity(counts, &unit.limits)?);
    let mut band_sources = Vec::with_capacity(bands_capacity(counts, &unit.limits)?);

    // 首遍只登记符号与不依赖父项的 RoadCorridor identity。其余实体先保留零值占位，
    // 但在所有者/引用错误存在时不会逃逸出本函数；父项闭合后才写入真实 ID。
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
                    TypedAstDeclaration::RoadCorridor(_)
                        | TypedAstDeclaration::RoadSection(_)
                        | TypedAstDeclaration::LaneGroup(_)
                        | TypedAstDeclaration::FacilityBand(_)
                )
                .then_some(index)
            })
            .collect();
        declaration_indices.sort_unstable_by(|left, right| {
            let left = declaration_header(&source_module.declarations[*left]);
            let right = declaration_header(&source_module.declarations[*right]);
            (left.entity_kind.code(), &left.source_address)
                .cmp(&(right.entity_kind.code(), &right.source_address))
        });
        for source_declaration_index in declaration_indices {
            let source_module_index = u32::try_from(module_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?;
            let declaration_index = u32::try_from(source_declaration_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?;
            match &source_module.declarations[source_declaration_index] {
                TypedAstDeclaration::LaneEdge(_) => {
                    unreachable!("cross-section source filter admitted LaneEdge")
                }
                TypedAstDeclaration::RoadCorridor(source) => {
                    let fields = [
                        IdentityFieldInput::new(
                            FieldTag::AuthoringNamespaceId,
                            source_module
                                .descriptor()
                                .authoring_namespace_id()
                                .as_bytes(),
                        ),
                        IdentityFieldInput::new(
                            FieldTag::CorridorKey,
                            source.header.stable_key.as_bytes(),
                        ),
                    ];
                    let stable_id = RoadCorridorId::from_untyped(derive_identity(
                        unit,
                        identities,
                        module_index,
                        EntityKind::RoadCorridor,
                        &source.header.stable_key,
                        &source.header.span,
                        &fields,
                    )?);
                    let key = corridors
                        .push(HirRoadCorridor {
                            module: module_key,
                            stable_key: Arc::clone(&source.header.stable_key),
                            stable_id,
                            reference_section: HirRoadSectionKey::from_raw(0),
                            elements: TableRange::empty(),
                            source_span: source.header.span.clone(),
                        })
                        .map_err(|overflow| {
                            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                        })?;
                    corridor_sources.push(CanonicalDeclarationSource {
                        source_module_index,
                        declaration_index,
                        hir_key: key,
                    });
                }
                TypedAstDeclaration::RoadSection(source) => {
                    let lane_start = lanes.len();
                    let section_key = sections
                        .push(HirRoadSection {
                            module: module_key,
                            stable_key: Arc::clone(&source.header.stable_key),
                            source_address: source.header.source_address.clone(),
                            stable_id: RoadSectionId::from_untyped(StableId128::ZERO),
                            road_corridor: HirRoadCorridorKey::from_raw(0),
                            kind_id: Arc::clone(&source.kind_id),
                            lanes: TableRange::empty(),
                            source_span: source.header.span.clone(),
                        })
                        .map_err(|overflow| {
                            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                        })?;
                    section_symbols.insert(
                        module_key,
                        source.header.source_address.clone(),
                        section_key,
                    );
                    section_sources.push(CanonicalDeclarationSource {
                        source_module_index,
                        declaration_index,
                        hir_key: section_key,
                    });
                    for (lane_index, lane) in source.lanes.iter().enumerate() {
                        let lane_key = lanes
                            .push(HirAuthoringLane {
                                module: module_key,
                                stable_key: Arc::clone(&lane.header.stable_key),
                                stable_id: AuthoringLaneId::from_untyped(StableId128::ZERO),
                                road_section: section_key,
                                edge_chain: TableRange::empty(),
                                lane_group: None,
                                lane_group_source_location: None,
                                source_span: lane.header.span.clone(),
                            })
                            .map_err(|overflow| {
                                arena_overflow(
                                    overflow,
                                    &unit.limits,
                                    Some(lane.header.span.clone()),
                                )
                            })?;
                        lane_sources.push(CanonicalAuthoringLaneSource {
                            source_module_index,
                            declaration_index,
                            lane_index: u32::try_from(lane_index).map_err(|_| {
                                arena_overflow(
                                    ArenaKeyOverflow,
                                    &unit.limits,
                                    Some(lane.header.span.clone()),
                                )
                            })?,
                            hir_key: lane_key,
                        });
                    }
                    sections.get_mut(section_key).lanes = TableRange::try_from_usize(
                        lane_start,
                        lanes.len().saturating_sub(lane_start),
                    )
                    .map_err(|overflow| {
                        arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                    })?;
                }
                TypedAstDeclaration::LaneGroup(source) => {
                    let key = groups
                        .push(HirLaneGroup {
                            module: module_key,
                            stable_key: Arc::clone(&source.header.stable_key),
                            source_address: source.header.source_address.clone(),
                            stable_id: LaneGroupId::from_untyped(StableId128::ZERO),
                            road_section: HirRoadSectionKey::from_raw(0),
                            members: TableRange::empty(),
                            source_span: source.header.span.clone(),
                        })
                        .map_err(|overflow| {
                            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                        })?;
                    group_symbols.insert(module_key, source.header.source_address.clone(), key);
                    group_sources.push(CanonicalDeclarationSource {
                        source_module_index,
                        declaration_index,
                        hir_key: key,
                    });
                }
                TypedAstDeclaration::FacilityBand(source) => {
                    let key = bands
                        .push(HirFacilityBand {
                            module: module_key,
                            stable_key: Arc::clone(&source.header.stable_key),
                            source_address: source.header.source_address.clone(),
                            stable_id: FacilityBandId::from_untyped(StableId128::ZERO),
                            road_corridor: HirRoadCorridorKey::from_raw(0),
                            kind_id: Arc::clone(&source.kind_id),
                            source_span: source.header.span.clone(),
                        })
                        .map_err(|overflow| {
                            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                        })?;
                    band_symbols.insert(module_key, source.header.source_address.clone(), key);
                    band_sources.push(CanonicalDeclarationSource {
                        source_module_index,
                        declaration_index,
                        hir_key: key,
                    });
                }
                TypedAstDeclaration::Junction(_)
                | TypedAstDeclaration::Movement(_)
                | TypedAstDeclaration::ManeuverPath(_)
                | TypedAstDeclaration::StopLine(_)
                | TypedAstDeclaration::ManeuverGate(_)
                | TypedAstDeclaration::WaitingZone(_)
                | TypedAstDeclaration::StaticRoute(_)
                | TypedAstDeclaration::SignalGroup(_)
                | TypedAstDeclaration::SignalController(_)
                | TypedAstDeclaration::ParkingArea(_)
                | TypedAstDeclaration::ParkingSpace(_)
                | TypedAstDeclaration::ParticipantClass(_)
                | TypedAstDeclaration::VehicleProfile(_)
                | TypedAstDeclaration::CanonicalFrame(_)
                | TypedAstDeclaration::AccessRule(_) => {
                    unreachable!("cross-section source filter admitted junction declaration")
                }
            }
        }
    }

    let mut diagnostics =
        DiagnosticCollector::new(unit.limits.value(CompileLimitDimension::DiagnosticCount));
    let mut corridor_elements =
        Vec::with_capacity(count_to_usize(counts.corridor_elements, &unit.limits)?);
    let mut section_owners: Vec<Option<(HirRoadCorridorKey, SourceLocation)>> =
        vec![None; sections.len()];
    let mut band_owners: Vec<Option<(HirRoadCorridorKey, SourceLocation)>> =
        vec![None; bands.len()];

    for location in &corridor_sources {
        let source_module = &unit.modules[location.source_module_index as usize];
        let TypedAstDeclaration::RoadCorridor(source) =
            &source_module.declarations[location.declaration_index as usize]
        else {
            unreachable!("canonical RoadCorridor source changed kind")
        };
        let reference_section = resolve_reference(
            module_lookup,
            &section_symbols,
            &source.reference_section,
            EntityKind::RoadCorridor,
            &source.header,
            location.source_module_index,
            &mut diagnostics,
        );
        let start = corridor_elements.len();
        let mut reference_is_member = false;
        for element in &source.elements {
            match element {
                OwnedCorridorElementReference::RoadSection(reference) => {
                    if let Some(target) = resolve_reference(
                        module_lookup,
                        &section_symbols,
                        reference,
                        EntityKind::RoadCorridor,
                        &source.header,
                        location.source_module_index,
                        &mut diagnostics,
                    ) {
                        reference_is_member |= reference_section == Some(target);
                        register_owner(
                            EntityKind::RoadSection,
                            target.index(),
                            &sections.get(target).stable_key,
                            location.hir_key,
                            &source.header,
                            &mut section_owners,
                            &corridors,
                            location.source_module_index,
                            &mut diagnostics,
                        );
                        corridor_elements.push(HirCorridorElement::RoadSection {
                            road_section: target,
                            source_location: unit.resolve_source_location_for_module(
                                location.source_module_index,
                                &reference.span,
                            )?,
                        });
                    }
                }
                OwnedCorridorElementReference::FacilityBand(reference) => {
                    if let Some(target) = resolve_reference(
                        module_lookup,
                        &band_symbols,
                        reference,
                        EntityKind::RoadCorridor,
                        &source.header,
                        location.source_module_index,
                        &mut diagnostics,
                    ) {
                        register_owner(
                            EntityKind::FacilityBand,
                            target.index(),
                            &bands.get(target).stable_key,
                            location.hir_key,
                            &source.header,
                            &mut band_owners,
                            &corridors,
                            location.source_module_index,
                            &mut diagnostics,
                        );
                        corridor_elements.push(HirCorridorElement::FacilityBand {
                            facility_band: target,
                            source_location: unit.resolve_source_location_for_module(
                                location.source_module_index,
                                &reference.span,
                            )?,
                        });
                    }
                }
            }
        }
        if let Some(reference_section) = reference_section {
            corridors.get_mut(location.hir_key).reference_section = reference_section;
            if !reference_is_member {
                let mut diagnostic = Diagnostic::invalid_corridor_reference_section(
                    &source.header.stable_key,
                    &source.reference_section.module_namespace,
                    source.reference_section.declaration_key(),
                    source.reference_section.span.clone(),
                    source.header.span.clone(),
                );
                diagnostic.set_canonical_module_order(location.source_module_index);
                diagnostics.push(diagnostic);
            }
        }
        corridors.get_mut(location.hir_key).elements =
            TableRange::try_from_usize(start, corridor_elements.len().saturating_sub(start))
                .map_err(|overflow| {
                    arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                })?;
    }

    for (key, section) in sections.iter() {
        if section_owners[key.index()].is_none() {
            let mut diagnostic = Diagnostic::missing_cross_section_owner(
                EntityKind::RoadSection,
                &section.stable_key,
                section.source_span.clone(),
            );
            diagnostic.set_canonical_module_order(section.module.raw());
            diagnostics.push(diagnostic);
        }
    }
    for (key, band) in bands.iter() {
        if band_owners[key.index()].is_none() {
            let mut diagnostic = Diagnostic::missing_cross_section_owner(
                EntityKind::FacilityBand,
                &band.stable_key,
                band.source_span.clone(),
            );
            diagnostic.set_canonical_module_order(band.module.raw());
            diagnostics.push(diagnostic);
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    // 父走廊已唯一闭合，此时才派生 RoadSection / FacilityBand identity。
    for location in &section_sources {
        let source_module = &unit.modules[location.source_module_index as usize];
        let TypedAstDeclaration::RoadSection(source) =
            &source_module.declarations[location.declaration_index as usize]
        else {
            unreachable!("canonical RoadSection source changed kind")
        };
        let owner = section_owners[location.hir_key.index()]
            .as_ref()
            .expect("owner diagnostics already rejected missing sections")
            .0;
        let owner_id = corridors.get(owner).stable_id;
        let fields = [
            IdentityFieldInput::new(
                FieldTag::AuthoringNamespaceId,
                source_module
                    .descriptor()
                    .authoring_namespace_id()
                    .as_bytes(),
            ),
            IdentityFieldInput::new(FieldTag::SectionKey, source.header.stable_key.as_bytes()),
            IdentityFieldInput::new(
                FieldTag::RoadCorridorStableId,
                owner_id.as_untyped().as_bytes(),
            ),
        ];
        let stable_id = RoadSectionId::from_untyped(derive_identity(
            unit,
            identities,
            location.source_module_index as usize,
            EntityKind::RoadSection,
            &source.header.stable_key,
            &source.header.span,
            &fields,
        )?);
        let section = sections.get_mut(location.hir_key);
        section.road_corridor = owner;
        section.stable_id = stable_id;
    }
    for location in &band_sources {
        let source_module = &unit.modules[location.source_module_index as usize];
        let TypedAstDeclaration::FacilityBand(source) =
            &source_module.declarations[location.declaration_index as usize]
        else {
            unreachable!("canonical FacilityBand source changed kind")
        };
        let owner = band_owners[location.hir_key.index()]
            .as_ref()
            .expect("owner diagnostics already rejected missing bands")
            .0;
        let owner_id = corridors.get(owner).stable_id;
        let fields = [
            IdentityFieldInput::new(
                FieldTag::AuthoringNamespaceId,
                source_module
                    .descriptor()
                    .authoring_namespace_id()
                    .as_bytes(),
            ),
            IdentityFieldInput::new(
                FieldTag::FacilityBandKey,
                source.header.stable_key.as_bytes(),
            ),
            IdentityFieldInput::new(
                FieldTag::RoadCorridorStableId,
                owner_id.as_untyped().as_bytes(),
            ),
        ];
        let stable_id = FacilityBandId::from_untyped(derive_identity(
            unit,
            identities,
            location.source_module_index as usize,
            EntityKind::FacilityBand,
            &source.header.stable_key,
            &source.header.span,
            &fields,
        )?);
        let band = bands.get_mut(location.hir_key);
        band.road_corridor = owner;
        band.stable_id = stable_id;
    }

    // LaneGroup 的父区段是其 identity 输入，必须先解析再处理引用它的编制车道。
    for location in &group_sources {
        let source_module = &unit.modules[location.source_module_index as usize];
        let TypedAstDeclaration::LaneGroup(source) =
            &source_module.declarations[location.declaration_index as usize]
        else {
            unreachable!("canonical LaneGroup source changed kind")
        };
        let Some(parent) = resolve_reference(
            module_lookup,
            &section_symbols,
            &source.road_section,
            EntityKind::LaneGroup,
            &source.header,
            location.source_module_index,
            &mut diagnostics,
        ) else {
            continue;
        };
        let parent_id = sections.get(parent).stable_id;
        let fields = [
            IdentityFieldInput::new(
                FieldTag::AuthoringNamespaceId,
                source_module
                    .descriptor()
                    .authoring_namespace_id()
                    .as_bytes(),
            ),
            IdentityFieldInput::new(FieldTag::LaneGroupKey, source.header.stable_key.as_bytes()),
            IdentityFieldInput::new(
                FieldTag::RoadSectionStableId,
                parent_id.as_untyped().as_bytes(),
            ),
        ];
        let stable_id = LaneGroupId::from_untyped(derive_identity(
            unit,
            identities,
            location.source_module_index as usize,
            EntityKind::LaneGroup,
            &source.header.stable_key,
            &source.header.span,
            &fields,
        )?);
        let group = groups.get_mut(location.hir_key);
        group.road_section = parent;
        group.stable_id = stable_id;
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    let mut lane_edges_flat =
        Vec::with_capacity(count_to_usize(counts.authoring_lane_edges, &unit.limits)?);
    let mut edge_owners: Vec<Option<HirAuthoringLaneKey>> = vec![None; lane_edges.len()];
    let mut group_member_counts = vec![0_usize; groups.len()];
    for location in &lane_sources {
        let source_module = &unit.modules[location.source_module_index as usize];
        let TypedAstDeclaration::RoadSection(section_source) =
            &source_module.declarations[location.declaration_index as usize]
        else {
            unreachable!("canonical AuthoringLane source parent changed kind")
        };
        let lane_source = &section_source.lanes[location.lane_index as usize];
        let parent = lanes.get(location.hir_key).road_section;
        let parent_id = sections.get(parent).stable_id;
        let fields = [
            IdentityFieldInput::new(
                FieldTag::AuthoringNamespaceId,
                source_module
                    .descriptor()
                    .authoring_namespace_id()
                    .as_bytes(),
            ),
            IdentityFieldInput::new(FieldTag::LaneKey, lane_source.header.stable_key.as_bytes()),
            IdentityFieldInput::new(
                FieldTag::RoadSectionStableId,
                parent_id.as_untyped().as_bytes(),
            ),
        ];
        let stable_id = AuthoringLaneId::from_untyped(derive_identity(
            unit,
            identities,
            location.source_module_index as usize,
            EntityKind::AuthoringLane,
            &lane_source.header.stable_key,
            &lane_source.header.span,
            &fields,
        )?);
        let start = lane_edges_flat.len();
        let mut predecessor = None;
        for reference in &lane_source.edge_chain {
            let Some(target) = resolve_reference(
                module_lookup,
                lane_edge_symbols,
                reference,
                EntityKind::AuthoringLane,
                &lane_source.header,
                location.source_module_index,
                &mut diagnostics,
            ) else {
                // 未知引用保留自身诊断，但不能把其两侧原本不相邻的边拼接后再检查连通性。
                predecessor = None;
                continue;
            };
            if let Some(first_owner) = edge_owners[target.index()] {
                let mut diagnostic = Diagnostic::multiple_authoring_lane_owners(
                    &lane_edges.get(target).stable_key,
                    &lanes.get(first_owner).stable_key,
                    &lane_source.header.stable_key,
                    reference.span.clone(),
                    lanes.get(first_owner).source_span.clone(),
                );
                diagnostic.set_canonical_module_order(location.source_module_index);
                diagnostics.push(diagnostic);
            } else {
                edge_owners[target.index()] = Some(location.hir_key);
            }
            if let Some((predecessor_key, predecessor_span)) = predecessor {
                let predecessor_record = lane_edges.get(predecessor_key);
                let connected = lane_edge_references
                    [predecessor_record.successors.as_usize_range()]
                .iter()
                .any(|candidate| candidate.target == target);
                if !connected {
                    let mut diagnostic = Diagnostic::disconnected_authoring_lane_edge_chain(
                        &lane_source.header.stable_key,
                        &predecessor_record.stable_key,
                        &lane_edges.get(target).stable_key,
                        reference.span.clone(),
                        predecessor_span,
                    );
                    diagnostic.set_canonical_module_order(location.source_module_index);
                    diagnostics.push(diagnostic);
                }
            }
            predecessor = Some((target, reference.span.clone()));
            lane_edges_flat.push(HirAuthoringLaneEdge {
                target,
                source_span: reference.span.clone(),
            });
        }

        let lane_group = lane_source.lane_group.as_ref().and_then(|reference| {
            resolve_reference(
                module_lookup,
                &group_symbols,
                reference,
                EntityKind::AuthoringLane,
                &lane_source.header,
                location.source_module_index,
                &mut diagnostics,
            )
        });
        if let Some(group_key) = lane_group {
            let group = groups.get(group_key);
            if group.road_section != parent {
                let mut diagnostic = Diagnostic::lane_group_parent_mismatch(
                    &lane_source.header.stable_key,
                    &group.stable_key,
                    &sections.get(parent).stable_key,
                    &sections.get(group.road_section).stable_key,
                    lane_source
                        .lane_group
                        .as_ref()
                        .expect("resolved lane group has source reference")
                        .span
                        .clone(),
                    group.source_span.clone(),
                );
                diagnostic.set_canonical_module_order(location.source_module_index);
                diagnostics.push(diagnostic);
            } else {
                group_member_counts[group_key.index()] =
                    group_member_counts[group_key.index()].saturating_add(1);
            }
        }
        let lane = lanes.get_mut(location.hir_key);
        lane.stable_id = stable_id;
        lane.edge_chain =
            TableRange::try_from_usize(start, lane_edges_flat.len().saturating_sub(start))
                .map_err(|overflow| {
                    arena_overflow(
                        overflow,
                        &unit.limits,
                        Some(lane_source.header.span.clone()),
                    )
                })?;
        lane.lane_group = lane_group;
        lane.lane_group_source_location = match (&lane_source.lane_group, lane_group) {
            (Some(reference), Some(_)) => Some(unit.resolve_source_location_for_module(
                location.source_module_index,
                &reference.span,
            )?),
            _ => None,
        };
    }

    for (group_key, group) in groups.iter() {
        if group_member_counts[group_key.index()] == 0 {
            diagnostics.push(Diagnostic::empty_lane_group(
                &group.stable_key,
                group.source_span.clone(),
            ));
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    // 先按 group key 计算连续范围，再按 lane key 递增顺序填充。这样维持与原车道遍历
    // 一致的成员顺序，同时避免为每个 LaneGroup 单独分配一个临时 Vec。
    let mut next_group_member = Vec::with_capacity(groups.len());
    let mut member_count = 0_usize;
    for (group_index, count) in group_member_counts.iter().copied().enumerate() {
        next_group_member.push(member_count);
        let group_key = HirLaneGroupKey::from_raw(
            u32::try_from(group_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        groups.get_mut(group_key).members = TableRange::try_from_usize(member_count, count)
            .map_err(|overflow| arena_overflow(overflow, &unit.limits, None))?;
        member_count = member_count.saturating_add(count);
    }
    let mut lane_group_members = if member_count == 0 {
        Vec::new()
    } else {
        let first_member = lanes
            .iter()
            .find_map(|(key, lane)| lane.lane_group.map(|_| key))
            .expect("positive validated group member count must name a lane");
        vec![HirLaneGroupMember { lane: first_member }; member_count]
    };
    for (lane_key, lane) in lanes.iter() {
        let Some(group_key) = lane.lane_group else {
            continue;
        };
        let destination = &mut next_group_member[group_key.index()];
        lane_group_members[*destination] = HirLaneGroupMember { lane: lane_key };
        *destination += 1;
    }
    debug_assert!(groups.iter().all(|(key, group)| {
        next_group_member[key.index()] == group.members.as_usize_range().end
    }));

    Ok(CrossSectionHir {
        road_corridors: corridors.into_boxed_slice(),
        corridor_elements: corridor_elements.into_boxed_slice(),
        road_sections: sections.into_boxed_slice(),
        authoring_lanes: lanes.into_boxed_slice(),
        authoring_lane_edges: lane_edges_flat.into_boxed_slice(),
        lane_groups: groups.into_boxed_slice(),
        lane_group_members: lane_group_members.into_boxed_slice(),
        facility_bands: bands.into_boxed_slice(),
    })
}

fn corridors_capacity(
    counts: &CrossSectionCounts,
    limits: &crate::CompileLimits,
) -> Result<usize, DiagnosticBundle> {
    count_to_usize(counts.road_corridors, limits)
}

fn sections_capacity(
    counts: &CrossSectionCounts,
    limits: &crate::CompileLimits,
) -> Result<usize, DiagnosticBundle> {
    count_to_usize(counts.road_sections, limits)
}

fn lanes_capacity(
    counts: &CrossSectionCounts,
    limits: &crate::CompileLimits,
) -> Result<usize, DiagnosticBundle> {
    count_to_usize(counts.authoring_lanes, limits)
}

fn groups_capacity(
    counts: &CrossSectionCounts,
    limits: &crate::CompileLimits,
) -> Result<usize, DiagnosticBundle> {
    count_to_usize(counts.lane_groups, limits)
}

fn bands_capacity(
    counts: &CrossSectionCounts,
    limits: &crate::CompileLimits,
) -> Result<usize, DiagnosticBundle> {
    count_to_usize(counts.facility_bands, limits)
}

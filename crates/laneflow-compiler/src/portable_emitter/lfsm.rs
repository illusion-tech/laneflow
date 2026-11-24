use super::budget::Accounting;
use super::tree::{ObjectTree, RowRef, SectionRef, TreeBuilder, Value, build_tree, field};
use super::*;
use crate::ledger::{BoundedVec, Plan};
mod policy_check;
mod policy_projection;
#[cfg(test)]
mod policy_tests;
pub(super) use policy_check::check_policy_sources;
pub use policy_check::check_portable_policy_sources;

fn road_editing_relation_code(value: crate::RoadEditingRelationKind) -> u8 {
    match value {
        crate::RoadEditingRelationKind::Import => 0,
        crate::RoadEditingRelationKind::CurveSegment => 1,
        crate::RoadEditingRelationKind::CorridorElement => 2,
        crate::RoadEditingRelationKind::RoadSectionAuthoringLane => 3,
        crate::RoadEditingRelationKind::LaneEdgeSuccessor => 4,
        crate::RoadEditingRelationKind::JunctionApproachEdge => 5,
        crate::RoadEditingRelationKind::JunctionInternalEdge => 6,
        crate::RoadEditingRelationKind::ManeuverPathInternalEdge => 7,
        crate::RoadEditingRelationKind::SignalControllerGroup => 8,
        crate::RoadEditingRelationKind::SignalControllerPhase => 9,
        crate::RoadEditingRelationKind::SignalPhaseState => 10,
        crate::RoadEditingRelationKind::AccessRuleParticipantClass => 11,
        crate::RoadEditingRelationKind::ParkingFacilityVirtualEntry => 12,
        crate::RoadEditingRelationKind::ParkingFacilityVirtualExit => 13,
        crate::RoadEditingRelationKind::ParticipantStreamPassage => 14,
        crate::RoadEditingRelationKind::ConflictZoneRegion => 15,
        crate::RoadEditingRelationKind::PolicyEvidence => 16,
        crate::RoadEditingRelationKind::PolicyGapProfile => 17,
        crate::RoadEditingRelationKind::PolicyStreamRule => 18,
        crate::RoadEditingRelationKind::PolicyGateRule => 19,
        crate::RoadEditingRelationKind::AuthoringLaneEdge => 20,
        crate::RoadEditingRelationKind::InteriorCut => 21,
    }
}

fn source_language_code(value: crate::SourceLanguage) -> u16 {
    match value {
        crate::SourceLanguage::SyntheticDsl => 1,
        crate::SourceLanguage::RoadEditingSource => 2,
    }
}

/// 来源文档键到（模块序号, 文档序号）的有序查找表，由发射账本授予。
pub(super) struct DocumentOrdinals<'a> {
    entries: BoundedVec<(&'a str, u32, u32)>,
}

impl DocumentOrdinals<'_> {
    fn get(&self, key: &str) -> Option<(u32, u32)> {
        self.entries
            .binary_search_by(|entry| entry.0.cmp(key))
            .ok()
            .map(|index| (self.entries[index].1, self.entries[index].2))
    }

    const fn granted_bytes(&self) -> u64 {
        self.entries.granted_bytes()
    }
}

#[cfg(test)]
impl<'a> DocumentOrdinals<'a> {
    /// 测试用：由（文档键, 模块序号, 文档序号）构造查找表。
    pub(super) fn for_test(entries: &[(&'a str, u32, u32)]) -> Self {
        let mut granted = super::budget::test_pass().vec(entries.len()).unwrap();
        granted.extend_from_slice(entries).unwrap();
        granted.sort_unstable();
        Self { entries: granted }
    }
}

type AddressProjection<'a> = (&'a str, Option<u16>, [Option<&'a str>; 3], &'a str);

fn address_projection<'a>(
    address: &'a crate::RoadEditingSourceAddress,
    context: &'a crate::RoadEditingLocationContext,
) -> AddressProjection<'a> {
    let mut owner_local_keys = [None, None, None];
    for (slot, key) in owner_local_keys
        .iter_mut()
        .zip(address.owner_local_keys(context))
    {
        *slot = Some(key);
    }
    (
        address.module_namespace(context),
        address.entity_kind().map(EntityKind::code),
        owner_local_keys,
        address.local_key(context),
    )
}

fn location_value<'a>(
    view: crate::SourceLocationView<'a>,
    documents: &DocumentOrdinals<'_>,
) -> Result<LocationValue<'a>, PortableEmissionError> {
    let (source_module_ordinal, source_document_ordinal) = documents
        .get(view.source_document_key())
        .ok_or(PortableEmissionError::InternalBindingMismatch)?;
    match view {
        crate::SourceLocationView::Text { start, end, .. } => Ok(LocationValue::Text {
            source_module_ordinal,
            source_document_ordinal,
            start_line: start.line(),
            start_column: start.column(),
            end_line: end.line(),
            end_column: end.column(),
        }),
        crate::SourceLocationView::RoadEditing(location) => {
            if location.byte_range().is_some() {
                return Err(PortableEmissionError::InternalBindingMismatch);
            }
            let context = location.context();
            let mut module_namespace = None;
            let mut entity_kind = None;
            let mut owner_local_keys = [None, None, None];
            let mut local_key = None;
            let mut owner_kind = None;
            let mut relation_kind = None;
            let mut occurrence_kind = None;
            let mut occurrence_ordinal = None;
            let subject_kind = match location.subject() {
                crate::RoadEditingSubject::ModuleHeader => 0,
                crate::RoadEditingSubject::RoadAlignment { address } => {
                    let (namespace, entity, owners, key) = address_projection(address, context);
                    module_namespace = Some(namespace);
                    entity_kind = entity;
                    owner_local_keys = owners;
                    local_key = Some(key);
                    1
                }
                crate::RoadEditingSubject::Declaration { address } => {
                    let (namespace, entity, owners, key) = address_projection(address, context);
                    module_namespace = Some(namespace);
                    entity_kind = entity;
                    owner_local_keys = owners;
                    local_key = Some(key);
                    2
                }
                crate::RoadEditingSubject::OwnerLocal {
                    owner,
                    relation,
                    occurrence,
                } => {
                    match owner {
                        crate::RoadEditingOwner::ModuleHeader => owner_kind = Some(0),
                        crate::RoadEditingOwner::Address(address) => {
                            owner_kind = Some(1);
                            let (namespace, entity, owners, key) =
                                address_projection(address, context);
                            module_namespace = Some(namespace);
                            entity_kind = entity;
                            owner_local_keys = owners;
                            local_key = Some(key);
                        }
                    }
                    relation_kind = Some(road_editing_relation_code(*relation));
                    let (kind, ordinal) = match *occurrence {
                        crate::RoadEditingRelationOccurrence::OrderedProductOrdinal(ordinal) => {
                            (0, ordinal)
                        }
                        crate::RoadEditingRelationOccurrence::CanonicalSetOrdinal(ordinal) => {
                            (1, ordinal)
                        }
                    };
                    occurrence_kind = Some(kind);
                    occurrence_ordinal = Some(ordinal);
                    3
                }
                crate::RoadEditingSubject::Wire { .. } => {
                    return Err(PortableEmissionError::InternalBindingMismatch);
                }
            };
            Ok(LocationValue::RoadEditing {
                source_module_ordinal,
                source_document_ordinal,
                subject_kind,
                module_namespace,
                entity_kind,
                owner_local_keys,
                local_key,
                owner_kind,
                relation_kind,
                occurrence_kind,
                occurrence_ordinal,
                property_steps: location
                    .property_path()
                    .map(|path| PropertySteps(path.steps())),
                canvas_selection: location.canvas_selection(),
            })
        }
    }
}

fn location_ordinal(
    locations: &[LocationValue<'_>],
    location: &LocationValue<'_>,
) -> Result<u32, PortableEmissionError> {
    let index = locations
        .binary_search(location)
        .map_err(|_| PortableEmissionError::InternalBindingMismatch)?;
    u32::try_from(index).map_err(|_| PortableEmissionError::ArithmeticOverflow)
}

/// 把一组来源位置的序号去重后按升序追加到序号集合存储，返回其区间。位置池按位置值
/// 排序，序号升序即位置值升序。
fn push_location_set<'l, 'a: 'l>(
    sets: &mut BoundedVec<u32>,
    locations: &[LocationValue<'_>],
    values: impl IntoIterator<Item = &'l LocationValue<'a>>,
) -> Result<Slots, PortableEmissionError> {
    let start = sets.len();
    for value in values {
        sets.push(location_ordinal(locations, value)?)?;
    }
    sets[start..].sort_unstable();
    let mut end = start;
    for read in start..sets.len() {
        if end == start || sets[read] != sets[end - 1] {
            sets[end] = sets[read];
            end += 1;
        }
    }
    sets.truncate(end);
    Ok(Slots { start, end })
}

fn location_row(t: &TreeBuilder, ordinal: u32, location: &LocationValue<'_>) -> RowRef {
    match *location {
        LocationValue::Text {
            source_module_ordinal,
            source_document_ordinal,
            start_line,
            start_column,
            end_line,
            end_column,
        } => t.row([
            field(1, Value::U32(ordinal)),
            field(2, Value::U8(0)),
            field(3, Value::U32(source_module_ordinal)),
            field(4, Value::U32(source_document_ordinal)),
            field(5, Value::U32(start_line)),
            field(6, Value::U32(start_column)),
            field(7, Value::U32(end_line)),
            field(8, Value::U32(end_column)),
        ]),
        LocationValue::RoadEditing {
            source_module_ordinal,
            source_document_ordinal,
            subject_kind,
            module_namespace,
            entity_kind,
            owner_local_keys,
            local_key,
            owner_kind,
            relation_kind,
            occurrence_kind,
            occurrence_ordinal,
            property_steps,
            canvas_selection,
        } => {
            let [first, second, third] = owner_local_keys.map(|key| key.map(|key| t.utf8(key)));
            t.row([
                Some(field(1, Value::U32(ordinal))),
                Some(field(2, Value::U8(1))),
                Some(field(3, Value::U32(source_module_ordinal))),
                Some(field(4, Value::U32(source_document_ordinal))),
                Some(field(9, Value::U8(subject_kind))),
                module_namespace.map(|value| field(10, t.utf8(value))),
                entity_kind.map(|value| field(11, Value::U16(value))),
                first.map(|value| field(12, value)),
                second.map(|value| field(13, value)),
                third.map(|value| field(14, value)),
                local_key.map(|value| field(15, t.utf8(value))),
                owner_kind.map(|value| field(16, Value::U8(value))),
                relation_kind.map(|value| field(17, Value::U8(value))),
                occurrence_kind.map(|value| field(18, Value::U8(value))),
                occurrence_ordinal.map(|value| field(19, Value::U32(value))),
                property_steps.map(|steps| {
                    field(
                        20,
                        t.records(steps.codes().map(|(kind, container, member)| {
                            t.record([
                                field(1, Value::U8(kind)),
                                field(2, Value::U16(container)),
                                field(3, Value::U16(member)),
                            ])
                        })),
                    )
                }),
                canvas_selection.map(|value| field(21, t.utf8(value))),
            ])
        }
    }
}

/// 对每个稳定实体来源族展开 `$apply!(实体种类, 来源迭代器)`。
macro_rules! for_each_stable_family {
    ($source_map:expr, $apply:ident) => {
        $apply!(
            EntityKind::RoadCorridor,
            $source_map.road_corridor_sources()
        );
        $apply!(EntityKind::RoadSection, $source_map.road_section_sources());
        $apply!(
            EntityKind::AuthoringLane,
            $source_map.authoring_lane_sources()
        );
        $apply!(EntityKind::LaneEdge, $source_map.lane_edge_sources());
        $apply!(EntityKind::Junction, $source_map.junction_sources());
        $apply!(EntityKind::Movement, $source_map.movement_sources());
        $apply!(
            EntityKind::ManeuverPath,
            $source_map.maneuver_path_sources()
        );
        $apply!(
            EntityKind::ManeuverGate,
            $source_map.maneuver_gate_sources()
        );
        $apply!(EntityKind::WaitingZone, $source_map.waiting_zone_sources());
        $apply!(EntityKind::StopLine, $source_map.stop_line_sources());
        $apply!(EntityKind::SignalGroup, $source_map.signal_group_sources());
        $apply!(
            EntityKind::SignalController,
            $source_map.signal_controller_sources()
        );
        $apply!(EntityKind::SignalPhase, $source_map.signal_phase_sources());
        $apply!(
            EntityKind::ConflictZone,
            $source_map.conflict_zone_sources()
        );
        $apply!(
            EntityKind::ParticipantStream,
            $source_map.participant_stream_sources()
        );
        $apply!(
            EntityKind::ParkingFacility,
            $source_map.parking_facility_sources()
        );
        $apply!(
            EntityKind::ParkingSpace,
            $source_map.parking_space_sources()
        );
        $apply!(EntityKind::LaneGroup, $source_map.lane_group_sources());
        $apply!(
            EntityKind::FacilityBand,
            $source_map.facility_band_sources()
        );
        $apply!(
            EntityKind::ParticipantClass,
            $source_map.participant_class_sources()
        );
        $apply!(EntityKind::AccessRule, $source_map.access_rule_sources());
        $apply!(
            EntityKind::VehicleProfile,
            $source_map.vehicle_profile_sources()
        );
        $apply!(
            EntityKind::CanonicalFrame,
            $source_map.canonical_frame_sources()
        );
    };
}

/// 对每个所有者局部来源族（不含 successor 与空间关系）展开
/// `$apply!(来源迭代器, 来源 => 所有者种类与稳定标识)`。
macro_rules! for_each_owner_family {
    ($source_map:expr, $apply:ident) => {
        $apply!($source_map.cross_section_relation_sources(), source => match source.owner() {
            crate::CrossSectionRelationOwner::RoadCorridor(_, id) => {
                (EntityKind::RoadCorridor, stable_id_bytes(id))
            }
            crate::CrossSectionRelationOwner::RoadSection(_, id) => {
                (EntityKind::RoadSection, stable_id_bytes(id))
            }
            crate::CrossSectionRelationOwner::AuthoringLane(_, id) => {
                (EntityKind::AuthoringLane, stable_id_bytes(id))
            }
            crate::CrossSectionRelationOwner::LaneGroup(_, id) => {
                (EntityKind::LaneGroup, stable_id_bytes(id))
            }
        });
        $apply!($source_map.junction_relation_sources(), source => match source.owner() {
            crate::JunctionRelationOwner::Junction(_, id) => {
                (EntityKind::Junction, stable_id_bytes(id))
            }
            crate::JunctionRelationOwner::Movement(_, id) => {
                (EntityKind::Movement, stable_id_bytes(id))
            }
            crate::JunctionRelationOwner::ManeuverPath(_, id) => {
                (EntityKind::ManeuverPath, stable_id_bytes(id))
            }
            crate::JunctionRelationOwner::StopLine(_, id) => {
                (EntityKind::StopLine, stable_id_bytes(id))
            }
        });
        $apply!($source_map.signal_relation_sources(), source => match source.owner() {
            crate::SignalRelationOwner::SignalController(_, id) => {
                (EntityKind::SignalController, stable_id_bytes(id))
            }
            crate::SignalRelationOwner::SignalPhase(_, id) => {
                (EntityKind::SignalPhase, stable_id_bytes(id))
            }
            crate::SignalRelationOwner::ManeuverGate(_, id) => {
                (EntityKind::ManeuverGate, stable_id_bytes(id))
            }
        });
        $apply!($source_map.parking_relation_sources(), source => (
            EntityKind::ParkingSpace,
            stable_id_bytes(source.owner_stable_id())
        ));
        $apply!($source_map.parking_facility_relation_sources(), source => (
            EntityKind::ParkingFacility,
            stable_id_bytes(source.owner_stable_id())
        ));
        $apply!($source_map.conflict_relation_sources(), source => (
            EntityKind::ParticipantStream,
            stable_id_bytes(source.owner_stable_id())
        ));
        $apply!($source_map.access_relation_sources(), source => match source.owner() {
            crate::AccessRelationOwner::ParticipantClass(_, id) => {
                (EntityKind::ParticipantClass, stable_id_bytes(id))
            }
            crate::AccessRelationOwner::VehicleProfile(_, id) => {
                (EntityKind::VehicleProfile, stable_id_bytes(id))
            }
            crate::AccessRelationOwner::AccessRule(_, id) => {
                (EntityKind::AccessRule, stable_id_bytes(id))
            }
        });
    };
}

/// 来源映射各类来源的计数，由不分配的遍历求得，是 LFSM 来源投影的计划输入。
#[derive(Clone, Copy, Debug, Default)]
struct SourceCounts {
    stable: usize,
    owner: usize,
    spatial_ranges: usize,
    contributing: usize,
    policy: usize,
    lir_entities: usize,
    expected_owner: usize,
}

fn source_counts(output: &CompilationOutput) -> SourceCounts {
    let source_map = output.source_map_input();
    let lir = output.lir().unit();
    let mut counts = SourceCounts::default();
    macro_rules! stable_family {
        ($kind:expr, $iterator:expr) => {
            for source in $iterator {
                counts.stable += 1;
                counts.contributing += source.contributing_sources().len();
            }
        };
    }
    for_each_stable_family!(source_map, stable_family);
    for source in source_map.lane_edge_successor_sources() {
        counts.owner += 1;
        counts.contributing += source.contributing_sources().len();
    }
    macro_rules! owner_family {
        ($iterator:expr, $source:ident => $owner:expr) => {
            for $source in $iterator {
                counts.owner += 1;
                counts.contributing += $source.contributing_sources().len();
            }
        };
    }
    for_each_owner_family!(source_map, owner_family);
    for source in source_map.spatial_relation_sources() {
        counts.owner += 1;
        counts.contributing += source.contributing_sources().len();
        counts.spatial_ranges += source.geometry_source_ranges().len();
    }
    for source in source_map.policy_sources() {
        counts.policy += 1;
        counts.contributing += source.contributing_sources().len();
        match source.target() {
            crate::PolicySourceTarget::Declaration { .. } => counts.stable += 1,
            crate::PolicySourceTarget::Member { .. } => counts.owner += 1,
            crate::PolicySourceTarget::MovementDirection { .. } => {}
        }
    }
    macro_rules! lir_entities {
        ($kind:expr, $records:expr) => {
            counts.lir_entities += $records.len();
        };
    }
    crate::lir::for_each_lir_entity_table!(lir, lir_entities);
    counts.expected_owner = super::relations::canonical_relation_count(lir)
        + lir
            .signal_phases
            .iter()
            .map(|phase| phase.states.as_usize_range().len())
            .sum::<usize>()
        + lir.lane_edge_geometries.len()
        + lir.facility_band_geometries.len()
        + lir.conflict_zone_regions.len();
    counts
}

/// LFSM 构造前一次求出、两遍构造共用的来源投影，全部由发射账本授予。
struct LfsmSources<'a> {
    documents: BoundedVec<(u32, &'a str, crate::SourceDocumentView<'a>)>,
    document_ordinals: DocumentOrdinals<'a>,
    stable: BoundedVec<StableSourceProjection<'a>>,
    owner_local: BoundedVec<OwnerLocalProjection<'a>>,
    spatial_ranges: BoundedVec<SpatialRangeProjection<'a>>,
    contributing: BoundedVec<LocationValue<'a>>,
    locations: BoundedVec<LocationValue<'a>>,
    sets: BoundedVec<u32>,
}

impl LfsmSources<'_> {
    const fn granted_bytes(&self) -> u64 {
        self.documents.granted_bytes()
            + self.document_ordinals.granted_bytes()
            + self.stable.granted_bytes()
            + self.owner_local.granted_bytes()
            + self.spatial_ranges.granted_bytes()
            + self.contributing.granted_bytes()
            + self.locations.granted_bytes()
            + self.sets.granted_bytes()
    }
}

/// 按计数求 LFSM 来源投影的计划峰值：输出投影与位置池同时存续；模块序号表、策略来源
/// 排序表、successor 局部下标计数器与两份期望键表只在本遍内存续，期望键表先后使用取峰值。
fn sources_plan(output: &CompilationOutput, counts: SourceCounts) -> Plan {
    let source_map = output.source_map_input();
    let lir = output.lir().unit();
    let modules = source_map.source_modules().len();
    let documents = source_map.source_documents().len();
    let location_capacity = modules
        + counts.stable
        + counts.owner
        + counts.spatial_ranges
        + counts.contributing
        + counts.policy;
    let set_capacity = 2 * counts.contributing + counts.policy + counts.owner;
    let stable_keys = Plan::ZERO.array::<(EntityKind, [u8; 16], u32)>(counts.lir_entities);
    let owner_keys = Plan::ZERO
        .array::<(EntityKind, [u8; 16], u8, u32)>(counts.expected_owner)
        .array::<u32>(super::relations::canonical_relation_scratch_len(lir))
        .array::<u32>(lir.canonical_frames.len());
    Plan::ZERO
        .array::<(u32, &str, crate::SourceDocumentView<'_>)>(documents)
        .array::<(&str, u32, u32)>(documents)
        .array::<StableSourceProjection<'_>>(counts.stable)
        .array::<OwnerLocalProjection<'_>>(counts.owner)
        .array::<SpatialRangeProjection<'_>>(counts.spatial_ranges)
        .array::<LocationValue<'_>>(counts.contributing)
        .array::<LocationValue<'_>>(location_capacity)
        .array::<u32>(set_capacity)
        .array::<(&str, u32)>(modules)
        .array::<crate::PolicySourceView<'_>>(counts.policy)
        .array::<u32>(lir.lane_edges.len())
        .plus(stable_keys.max(owner_keys))
}

fn prepare_sources<'a>(
    output: &'a CompilationOutput,
    accounting: Accounting<'_>,
) -> Result<LfsmSources<'a>, PortableEmissionError> {
    let source_map = output.source_map_input();
    let lir = output.lir().unit();
    let counts = source_counts(output);
    let pass = accounting.admit("lfsm-sources", sources_plan(output, counts))?;

    let mut module_ordinals = pass.vec::<(&str, u32)>(source_map.source_modules().len())?;
    for (ordinal, module) in source_map.source_modules().enumerate() {
        module_ordinals.push((
            module.authoring_namespace_id(),
            u32::try_from(ordinal).map_err(|_| PortableEmissionError::ArithmeticOverflow)?,
        ))?;
    }
    module_ordinals.sort_unstable();
    let mut documents = pass.vec(source_map.source_documents().len())?;
    for document in source_map.source_documents() {
        let module = module_ordinals
            .binary_search_by(|entry| entry.0.cmp(document.authoring_namespace_id()))
            .map(|index| module_ordinals[index].1)
            .map_err(|_| PortableEmissionError::InternalBindingMismatch)?;
        documents.push((module, document.source_document_key(), document))?;
    }
    drop(module_ordinals);
    documents.sort_unstable_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.as_bytes().cmp(right.1.as_bytes()))
    });
    let mut entries = pass.vec(documents.len())?;
    for (ordinal, (module, key, _)) in documents.iter().enumerate() {
        entries.push((
            *key,
            *module,
            u32::try_from(ordinal).map_err(|_| PortableEmissionError::ArithmeticOverflow)?,
        ))?;
    }
    entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
    let document_ordinals = DocumentOrdinals { entries };

    let mut contributing = pass.vec::<LocationValue<'a>>(counts.contributing)?;
    let mut stable = pass.vec::<StableSourceProjection<'a>>(counts.stable)?;
    macro_rules! stable_family {
        ($kind:expr, $iterator:expr) => {
            for source in $iterator {
                let primary = location_value(source.primary_source(), &document_ordinals)?;
                let start = contributing.len();
                for location in source.contributing_sources() {
                    contributing.push(location_value(location, &document_ordinals)?)?;
                }
                stable.push(StableSourceProjection {
                    entity_kind: $kind,
                    stable_id: stable_id_bytes(source.stable_id()),
                    typed_ordinal: source.ordinal().raw(),
                    primary,
                    contributing: Slots {
                        start,
                        end: contributing.len(),
                    },
                    direction: None,
                    set: Slots::default(),
                })?;
            }
        };
    }
    for_each_stable_family!(source_map, stable_family);
    stable.sort_unstable_by_key(|source| {
        (source.entity_kind, source.stable_id, source.typed_ordinal)
    });
    {
        let mut expected = pass.vec::<(EntityKind, [u8; 16], u32)>(counts.lir_entities)?;
        macro_rules! expected_keys {
            ($kind:expr, $records:expr) => {
                for record in &$records {
                    expected.push((
                        $kind,
                        stable_id_bytes(record.stable_id),
                        record.ordinal.raw(),
                    ))?;
                }
            };
        }
        crate::lir::for_each_lir_entity_table!(lir, expected_keys);
        expected.sort_unstable();
        if !stable
            .iter()
            .map(|source| (source.entity_kind, source.stable_id, source.typed_ordinal))
            .eq(expected.iter().copied())
        {
            return Err(PortableEmissionError::InternalBindingMismatch);
        }
    }

    let mut owner_local = pass.vec::<OwnerLocalProjection<'a>>(counts.owner)?;
    let mut spatial_ranges = pass.vec::<SpatialRangeProjection<'a>>(counts.spatial_ranges)?;
    {
        let mut next_successor_index_by_owner = pass.vec::<u32>(lir.lane_edges.len())?;
        next_successor_index_by_owner.resize(lir.lane_edges.len(), 0)?;
        for source in source_map.lane_edge_successor_sources() {
            let owner = source.owner_ordinal();
            let edge = &lir.lane_edges[owner.index()];
            let target = lir.lane_edge_successors[edge.successors.as_usize_range()]
                [usize::try_from(source.local_index())
                    .expect("supported compiler targets can index a validated local relation")];
            if super::relations::is_internal_edge(lir, owner.raw())
                || super::relations::is_internal_edge(lir, target.raw())
            {
                continue;
            }
            let local_index = next_successor_index_by_owner[owner.index()];
            next_successor_index_by_owner[owner.index()] += 1;
            let primary = location_value(source.primary_source(), &document_ordinals)?;
            let start = contributing.len();
            for location in source.contributing_sources() {
                contributing.push(location_value(location, &document_ordinals)?)?;
            }
            owner_local.push(OwnerLocalProjection {
                owner_entity_kind: EntityKind::LaneEdge,
                owner_stable_id: stable_id_bytes(source.owner_stable_id()),
                role: source.role().code(),
                local_index,
                primary,
                contributing: Slots {
                    start,
                    end: contributing.len(),
                },
                set: Slots::default(),
                derived_set: Slots::default(),
            })?;
        }
    }
    macro_rules! owner_family {
        ($iterator:expr, $source:ident => $owner:expr) => {
            for $source in $iterator {
                let (owner_entity_kind, owner_stable_id) = $owner;
                let primary = location_value($source.primary_source(), &document_ordinals)?;
                let start = contributing.len();
                for location in $source.contributing_sources() {
                    contributing.push(location_value(location, &document_ordinals)?)?;
                }
                owner_local.push(OwnerLocalProjection {
                    owner_entity_kind,
                    owner_stable_id,
                    role: $source.role().code(),
                    local_index: $source.local_index(),
                    primary,
                    contributing: Slots {
                        start,
                        end: contributing.len(),
                    },
                    set: Slots::default(),
                    derived_set: Slots::default(),
                })?;
            }
        };
    }
    for_each_owner_family!(source_map, owner_family);
    for source in source_map.spatial_relation_sources() {
        let owner_entity_kind = EntityKind::CanonicalFrame;
        let owner_stable_id = stable_id_bytes(source.owner_stable_id());
        let role = source.role().code();
        let local_index = source.local_index();
        let primary = location_value(source.primary_source(), &document_ordinals)?;
        let start = contributing.len();
        for location in source.contributing_sources() {
            contributing.push(location_value(location, &document_ordinals)?)?;
        }
        for range in source.geometry_source_ranges() {
            let points = range.point_range();
            spatial_ranges.push(SpatialRangeProjection {
                owner_entity_kind,
                owner_stable_id,
                role,
                local_index,
                point_start: points.start,
                point_end_exclusive: points.end,
                source_segment_ordinal: range.source_segment_ordinal(),
                source: location_value(range.source(), &document_ordinals)?,
            })?;
        }
        owner_local.push(OwnerLocalProjection {
            owner_entity_kind,
            owner_stable_id,
            role,
            local_index,
            primary,
            contributing: Slots {
                start,
                end: contributing.len(),
            },
            set: Slots::default(),
            derived_set: Slots::default(),
        })?;
    }
    owner_local.sort_unstable_by_key(|source| {
        (
            source.owner_entity_kind,
            source.owner_stable_id,
            source.role,
            source.local_index,
        )
    });
    verify_owner_local_keys(lir, &pass, counts.expected_owner, &owner_local)?;

    {
        let mut policy_sources = pass.vec(counts.policy)?;
        policy_projection::append_policy_sources(
            source_map,
            &document_ordinals,
            &mut policy_sources,
            &mut stable,
            &mut owner_local,
            &mut contributing,
        )?;
    }
    stable.sort_unstable_by_key(|s| (s.entity_kind, s.stable_id, s.typed_ordinal));
    owner_local.sort_unstable_by_key(|s| {
        (
            s.owner_entity_kind,
            s.owner_stable_id,
            s.role,
            s.local_index,
        )
    });
    spatial_ranges.sort_unstable_by_key(|source| {
        (
            source.owner_entity_kind,
            source.owner_stable_id,
            source.role,
            source.local_index,
            source.point_start,
        )
    });

    if source_map.source_module_sources().len() != source_map.source_modules().len() {
        return Err(PortableEmissionError::InternalBindingMismatch);
    }
    let location_capacity = source_map.source_modules().len()
        + counts.stable
        + counts.owner
        + counts.spatial_ranges
        + counts.contributing
        + counts.policy;
    let mut locations = pass.vec::<LocationValue<'a>>(location_capacity)?;
    for source in source_map.source_module_sources() {
        locations.push(location_value(source.primary_source(), &document_ordinals)?)?;
    }
    for source in &stable {
        locations.push(source.primary)?;
        locations.extend_from_slice(source.contributing.of(&contributing))?;
        if let Some(direction) = source.direction {
            locations.push(direction)?;
        }
    }
    for source in &owner_local {
        locations.push(source.primary)?;
        locations.extend_from_slice(source.contributing.of(&contributing))?;
    }
    locations.try_extend(spatial_ranges.iter().map(|range| range.source))?;
    locations.sort_unstable();
    locations.dedup();

    let mut sets = pass.vec::<u32>(2 * counts.contributing + counts.policy + counts.owner)?;
    for source in stable.iter_mut() {
        source.set = push_location_set(
            &mut sets,
            &locations,
            source
                .contributing
                .of(&contributing)
                .iter()
                .chain(source.direction.as_ref()),
        )?;
    }
    for source in owner_local.iter_mut() {
        source.set =
            push_location_set(&mut sets, &locations, source.contributing.of(&contributing))?;
        if source.role == 9 {
            source.derived_set = push_location_set(
                &mut sets,
                &locations,
                source
                    .contributing
                    .of(&contributing)
                    .iter()
                    .chain([&source.primary]),
            )?;
        }
    }

    let sources = LfsmSources {
        documents,
        document_ordinals,
        stable,
        owner_local,
        spatial_ranges,
        contributing,
        locations,
        sets,
    };
    pass.finish(sources.granted_bytes())?;
    Ok(sources)
}

/// 核对来源映射给出的所有者局部来源键与 LIR 规范关系、信号相位状态与规范坐标系几何
/// 推出的键完全一致。期望键在本编译遍内授予，核对后释放。
fn verify_owner_local_keys(
    lir: &crate::lir::LirUnit,
    pass: &crate::ledger::PassBudget<'_>,
    expected_count: usize,
    owner_local: &[OwnerLocalProjection<'_>],
) -> Result<(), PortableEmissionError> {
    let mut expected = pass.vec::<(EntityKind, [u8; 16], u8, u32)>(expected_count)?;
    let mut overflow = None;
    super::relations::for_each_canonical_relation_tuple(lir, pass, &mut |relation| {
        if let Err(error) = expected.push((
            relation.owner_entity_kind,
            relation.owner_stable_id,
            relation.role,
            relation.local_index,
        )) {
            overflow.get_or_insert(error);
        }
    })?;
    if let Some(error) = overflow {
        return Err(error.into());
    }
    for phase in &lir.signal_phases {
        for local_index in 0..phase.states.as_usize_range().len() {
            expected.push((
                EntityKind::SignalPhase,
                stable_id_bytes(phase.stable_id),
                19,
                u32::try_from(local_index).expect("compile limits cap relation counts at u32"),
            ))?;
        }
    }
    let mut next_index = pass.vec::<u32>(lir.canonical_frames.len())?;
    macro_rules! frame_keys {
        ($records:expr, $role:expr) => {
            next_index.resize(0, 0)?;
            next_index.resize(lir.canonical_frames.len(), 0)?;
            for record in &$records {
                let frame = record.canonical_frame;
                let local_index = next_index[frame.index()];
                next_index[frame.index()] += 1;
                expected.push((
                    EntityKind::CanonicalFrame,
                    stable_id_bytes(lir.canonical_frames[frame.index()].stable_id),
                    $role,
                    local_index,
                ))?;
            }
        };
    }
    frame_keys!(lir.lane_edge_geometries, 28);
    frame_keys!(lir.facility_band_geometries, 29);
    frame_keys!(lir.conflict_zone_regions, 32);
    drop(next_index);
    expected.sort_unstable();
    if owner_local
        .iter()
        .map(|source| {
            (
                source.owner_entity_kind,
                source.owner_stable_id,
                source.role,
                source.local_index,
            )
        })
        .eq(expected.iter().copied())
    {
        Ok(())
    } else {
        Err(PortableEmissionError::InternalBindingMismatch)
    }
}

/// 从受检来源映射输入、发射 provenance、来源集合摘要与已关闭的 LFCA 候选构造 LFSM
/// 源映射封套的对象树。来源投影与对象树均由发射账本授予。
pub(super) fn build_lfsm(
    output: &CompilationOutput,
    provenance: &PortableEmissionProvenance,
    source_collection_digest: [u8; 32],
    network_revision: NetworkRevisionId,
    artifact: &PortableObjectCandidate,
    accounting: Accounting<'_>,
) -> Result<ObjectTree, PortableEmissionError> {
    let sources = prepare_sources(output, accounting)?;
    let tree = build_tree(
        accounting,
        "lfsm-tree",
        PortableObjectKind::SourceMap,
        |t| {
            emit_lfsm(
                t,
                output,
                provenance,
                source_collection_digest,
                network_revision,
                artifact,
                &sources,
            )
        },
    )?;
    drop(sources);
    Ok(tree)
}

/// 测试用：以不受限的测试账本构造 LFSM，并还原为拥有型嵌套对象。
#[cfg(test)]
pub(super) fn build_lfsm_owned(
    output: &CompilationOutput,
    provenance: &PortableEmissionProvenance,
    source_collection_digest: [u8; 32],
    network_revision: NetworkRevisionId,
    artifact: &PortableObjectCandidate,
) -> Result<OwnedObject, PortableEmissionError> {
    Ok(build_lfsm(
        output,
        provenance,
        source_collection_digest,
        network_revision,
        artifact,
        super::budget::test_accounting(u64::MAX),
    )?
    .to_owned_object())
}

fn emit_lfsm(
    t: &TreeBuilder,
    output: &CompilationOutput,
    provenance: &PortableEmissionProvenance,
    source_collection_digest: [u8; 32],
    network_revision: NetworkRevisionId,
    artifact: &PortableObjectCandidate,
    sources: &LfsmSources<'_>,
) -> Result<[SectionRef; 5], PortableEmissionError> {
    let source_map = output.source_map_input();
    let locations = &sources.locations;
    let set = |slots: Slots| t.ordinals(slots.of(&sources.sets).iter().copied());

    let bindings = t.section(
        1,
        [t.table(
            1,
            [t.row([
                field(1, Value::U16(NETWORK_REVISION_DERIVATION_VERSION)),
                field(2, t.sha256(network_revision.into_digest().into_bytes())),
                field(3, Value::U16(CANONICAL_ARTIFACT_FORMAT_VERSION)),
                field(4, t.sha256(artifact.digest().into_bytes())),
                field(5, Value::U64(artifact.byte_length().get())),
                field(6, t.utf8(&provenance.compiler_build_id)),
                field(7, Value::U16(SOURCE_COLLECTION_DIGEST_VERSION_V1)),
                field(8, t.sha256(source_collection_digest)),
            ])],
        )],
    );
    let documents = t.section_with(2, || {
        t.table_with(1, || {
            for (ordinal, source) in source_map.source_module_sources().enumerate() {
                let descriptor = source.descriptor();
                let primary = location_value(source.primary_source(), &sources.document_ordinals)?;
                let primary = location_ordinal(locations, &primary)?;
                let imports = t.records(
                    descriptor
                        .imports()
                        .map(|namespace| t.record([field(1, t.utf8(namespace))])),
                );
                t.row([
                    Some(field(
                        1,
                        Value::U32(
                            u32::try_from(ordinal)
                                .expect("compile limits cap source modules at u32"),
                        ),
                    )),
                    Some(field(2, t.utf8(descriptor.authoring_namespace_id()))),
                    Some(field(
                        3,
                        Value::U16(source_language_code(descriptor.source_language())),
                    )),
                    Some(field(4, t.sha256(*descriptor.source_document_set_digest()))),
                    Some(field(
                        5,
                        Value::U32(descriptor.source_document_set_digest_version()),
                    )),
                    Some(field(6, Value::U32(descriptor.frontend_version()))),
                    Some(field(7, t.sha256(*descriptor.frontend_options_digest()))),
                    Some(field(8, t.utf8(descriptor.generator_build_id()))),
                    Some(field(
                        9,
                        t.sha256(*descriptor.parameters_and_inputs_digest()),
                    )),
                    descriptor
                        .random_seed()
                        .map(|seed| field(10, Value::U64(seed))),
                    Some(field(11, t.utf8(descriptor.provenance()))),
                    Some(field(12, imports)),
                    Some(field(13, Value::U32(primary))),
                ]);
            }
            Ok::<_, PortableEmissionError>(())
        })?;
        t.table(
            2,
            sources
                .documents
                .iter()
                .enumerate()
                .map(|(ordinal, (module_ordinal, _, document))| {
                    t.row([
                        Some(field(
                            1,
                            Value::U32(
                                u32::try_from(ordinal)
                                    .expect("compile limits cap source documents at u32"),
                            ),
                        )),
                        Some(field(2, Value::U32(*module_ordinal))),
                        Some(field(3, t.utf8(document.source_document_key()))),
                        Some(field(4, t.sha256(*document.source_document_digest()))),
                        Some(field(5, Value::U32(document.source_record_byte_len()))),
                        document
                            .origin()
                            .display_source()
                            .map(|display_source| field(6, t.utf8(display_source))),
                    ])
                }),
        );
        t.table(
            3,
            locations.iter().enumerate().map(|(ordinal, location)| {
                location_row(
                    t,
                    u32::try_from(ordinal).expect("compile limits cap source locations at u32"),
                    location,
                )
            }),
        );
        Ok::<_, PortableEmissionError>(())
    })?;
    let stable = t.section_with(3, || {
        t.table_with(1, || {
            for source in &sources.stable {
                let primary = location_ordinal(locations, &source.primary)?;
                t.row([
                    field(1, Value::U16(source.entity_kind.code())),
                    field(2, Value::StableId128(source.stable_id)),
                    field(3, Value::U32(source.typed_ordinal)),
                    field(4, Value::U32(primary)),
                    field(5, set(source.set)),
                ]);
            }
            Ok::<_, PortableEmissionError>(())
        })?;
        Ok::<_, PortableEmissionError>(())
    })?;
    let owner_local = t.section_with(4, || {
        t.table_with(1, || {
            for source in &sources.owner_local {
                let primary = location_ordinal(locations, &source.primary)?;
                t.row([
                    field(1, Value::U16(source.owner_entity_kind.code())),
                    field(2, Value::StableId128(source.owner_stable_id)),
                    field(3, Value::U8(source.role)),
                    field(4, Value::U32(source.local_index)),
                    field(5, Value::U32(primary)),
                    field(6, set(source.set)),
                ]);
            }
            Ok::<_, PortableEmissionError>(())
        })?;
        t.table_with(2, || {
            for range in &sources.spatial_ranges {
                let source = location_ordinal(locations, &range.source)?;
                t.row([
                    field(1, Value::U16(range.owner_entity_kind.code())),
                    field(2, Value::StableId128(range.owner_stable_id)),
                    field(3, Value::U8(range.role)),
                    field(4, Value::U32(range.local_index)),
                    field(5, Value::U32(range.point_start)),
                    field(6, Value::U32(range.point_end_exclusive)),
                    field(7, Value::U32(range.source_segment_ordinal)),
                    field(8, Value::U32(source)),
                ]);
            }
            Ok::<_, PortableEmissionError>(())
        })?;
        Ok::<_, PortableEmissionError>(())
    })?;
    let derived = t.section(
        5,
        [t.table(
            1,
            sources
                .owner_local
                .iter()
                .filter(|source| source.role == 9)
                .map(|source| {
                    t.row([
                        field(1, Value::U16(source.owner_entity_kind.code())),
                        field(2, Value::StableId128(source.owner_stable_id)),
                        field(3, Value::U8(source.role)),
                        field(4, Value::U32(source.local_index)),
                        field(5, Value::U16(1)),
                        field(6, Value::U16(CONSTRAINT_CONTRACT_VERSION)),
                        field(7, set(source.derived_set)),
                    ])
                }),
        )],
    );
    Ok([bindings, documents, stable, owner_local, derived])
}

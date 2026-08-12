use std::sync::Arc;

use laneflow_road_editing_wire::generated::lane_flow::road_editing::v1 as wire;
use laneflow_static_contract::EntityKind;

use crate::{
    RoadEditingAddressKind, RoadEditingByteRange, RoadEditingDocumentIdentity,
    RoadEditingLocationContext, RoadEditingOwner, RoadEditingPropertyPath, RoadEditingPropertyStep,
    RoadEditingRelationKind, RoadEditingRelationOccurrence, RoadEditingRootVectorKind,
    RoadEditingSourceAddress, RoadEditingSourceLocation, RoadEditingStructKind, RoadEditingSubject,
    RoadEditingTableKind, RoadEditingUnionKind, SourceLocation,
};

use super::rules::validate_wire_reference;

/// 语义预检失败前只保存在栈上的闭合来源位置种子。
///
/// `physical_index` 只用于从已经通过 verifier 的 root 中重新取得稳定身份字段；它不会
/// 进入最终 [`RoadEditingSubject`]，也不会参与诊断排序。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SemanticPreflightSubjectSite {
    ModuleHeader,
    ModuleOwnerLocal {
        relation: RoadEditingRelationKind,
        occurrence: RoadEditingRelationOccurrence,
    },
    Root {
        vector: RoadEditingRootVectorKind,
        physical_index: u32,
    },
    OwnerLocal {
        owner_vector: RoadEditingRootVectorKind,
        owner_physical_index: u32,
        relation: RoadEditingRelationKind,
        occurrence: RoadEditingRelationOccurrence,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SemanticPreflightPropertyPath {
    None,
    One([RoadEditingPropertyStep; 1]),
    Two([RoadEditingPropertyStep; 2]),
    Three([RoadEditingPropertyStep; 3]),
    Four([RoadEditingPropertyStep; 4]),
}

impl SemanticPreflightPropertyPath {
    fn as_slice(&self) -> &[RoadEditingPropertyStep] {
        match self {
            Self::None => &[],
            Self::One(value) => value,
            Self::Two(value) => value,
            Self::Three(value) => value,
            Self::Four(value) => value,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct SemanticPreflightSite {
    subject: SemanticPreflightSubjectSite,
    property: SemanticPreflightPropertyPath,
}

impl SemanticPreflightSite {
    pub(super) fn module_header(field: Option<&str>) -> Self {
        Self {
            subject: SemanticPreflightSubjectSite::ModuleHeader,
            property: semantic_property_path(field),
        }
    }

    pub(super) fn root(
        vector: RoadEditingRootVectorKind,
        physical_index: usize,
        field: Option<&str>,
    ) -> Self {
        Self {
            subject: SemanticPreflightSubjectSite::Root {
                vector,
                physical_index: u32::try_from(physical_index).unwrap_or(u32::MAX),
            },
            property: semantic_root_property_path(vector, field),
        }
    }

    pub(super) fn module_owner_local(
        relation: RoadEditingRelationKind,
        occurrence: RoadEditingRelationOccurrence,
        field: Option<&str>,
    ) -> Self {
        Self {
            subject: SemanticPreflightSubjectSite::ModuleOwnerLocal {
                relation,
                occurrence,
            },
            property: semantic_owner_local_property_path(relation, field),
        }
    }

    pub(super) fn owner_local(
        owner_vector: RoadEditingRootVectorKind,
        owner_physical_index: usize,
        relation: RoadEditingRelationKind,
        occurrence: RoadEditingRelationOccurrence,
        field: Option<&str>,
    ) -> Self {
        Self {
            subject: SemanticPreflightSubjectSite::OwnerLocal {
                owner_vector,
                owner_physical_index: u32::try_from(owner_physical_index).unwrap_or(u32::MAX),
                relation,
                occurrence,
            },
            property: semantic_owner_local_property_path(relation, field),
        }
    }
}

#[derive(Clone, Copy)]
enum MinimalOwnerKeys<'a> {
    None,
    One([&'a str; 1]),
    Two([&'a str; 2]),
    Three([&'a str; 3]),
}

impl MinimalOwnerKeys<'_> {
    fn as_slice(&self) -> &[&str] {
        match self {
            Self::None => &[],
            Self::One(value) => value,
            Self::Two(value) => value,
            Self::Three(value) => value,
        }
    }
}

#[derive(Clone, Copy)]
enum MinimalRootSubject<'a> {
    RoadAlignment {
        key: &'a str,
        canvas_selection: Option<&'a str>,
    },
    Declaration {
        entity_kind: EntityKind,
        owner_local_keys: MinimalOwnerKeys<'a>,
        local_key: &'a str,
        canvas_selection: Option<&'a str>,
    },
}

/// verifier 后的第二遍只收集来源位置需要保留的唯一 token；不复制完整 wire 字符串集。
pub(crate) struct RoadEditingLocationFactory {
    context: Arc<RoadEditingLocationContext>,
    document_identity: RoadEditingDocumentIdentity,
}

impl RoadEditingLocationFactory {
    pub(crate) fn input_module_header(expected_source_document_key: &str) -> SourceLocation {
        Self::input_module_header_with_range(expected_source_document_key, None)
    }

    pub(crate) fn input_module_header_with_range(
        expected_source_document_key: &str,
        byte_range: Option<RoadEditingByteRange>,
    ) -> SourceLocation {
        SourceLocation::RoadEditing(RoadEditingSourceLocation::new(
            empty_context(),
            RoadEditingDocumentIdentity::input(Arc::from(expected_source_document_key)),
            RoadEditingSubject::ModuleHeader,
            None,
            None,
            byte_range,
        ))
    }

    pub(crate) fn input_wire(
        expected_source_document_key: &str,
        root_vector: RoadEditingRootVectorKind,
        physical_index: u32,
        table: RoadEditingTableKind,
        byte_range: Option<RoadEditingByteRange>,
    ) -> SourceLocation {
        SourceLocation::RoadEditing(RoadEditingSourceLocation::new(
            empty_context(),
            RoadEditingDocumentIdentity::input(Arc::from(expected_source_document_key)),
            RoadEditingSubject::Wire {
                root_vector,
                physical_index,
                table,
            },
            None,
            None,
            byte_range,
        ))
    }

    pub(crate) fn verified_module_header(
        module_namespace: &str,
        source_document_key: &str,
    ) -> SourceLocation {
        Self {
            context: empty_context(),
            document_identity: RoadEditingDocumentIdentity::verified(
                Arc::from(module_namespace),
                Arc::from(source_document_key),
            ),
        }
        .module_header()
    }

    /// 只在语义预检失败出口把栈上 site 物化为最小来源上下文。
    ///
    /// `physical_index` 只重新读取 verifier 已确认安全的表；若 owner 引用自身不满足
    /// 语法约束，就返回 `None`，由 reader 诚实回退到模块头，而不是伪造声明地址。
    pub(super) fn semantic_preflight(
        root: wire::RoadEditingSource<'_>,
        expected_source_document_key: &str,
        site: SemanticPreflightSite,
    ) -> Option<SourceLocation> {
        let steps = site.property.as_slice();
        match site.subject {
            SemanticPreflightSubjectSite::ModuleHeader => {
                let factory = Self::minimal(
                    root,
                    expected_source_document_key,
                    MinimalOwnerKeys::None,
                    None,
                    None,
                    steps,
                );
                Some(if steps.is_empty() {
                    factory.module_header()
                } else {
                    factory.module_header_property(steps)
                })
            }
            SemanticPreflightSubjectSite::ModuleOwnerLocal {
                relation,
                occurrence,
            } => {
                let factory = Self::minimal(
                    root,
                    expected_source_document_key,
                    MinimalOwnerKeys::None,
                    None,
                    None,
                    steps,
                );
                Some(factory.module_owner_local(relation, occurrence, steps))
            }
            SemanticPreflightSubjectSite::Root {
                vector,
                physical_index,
            } => {
                let Some(subject) = minimal_root_subject(root, vector, physical_index) else {
                    return Some(verified_wire_fallback(
                        root,
                        expected_source_document_key,
                        vector,
                        physical_index,
                    ));
                };
                Some(materialize_minimal_root(
                    root,
                    expected_source_document_key,
                    subject,
                    steps,
                ))
            }
            SemanticPreflightSubjectSite::OwnerLocal {
                owner_vector,
                owner_physical_index,
                relation,
                occurrence,
            } => {
                let Some(subject) = minimal_root_subject(root, owner_vector, owner_physical_index)
                else {
                    return Some(verified_wire_fallback(
                        root,
                        expected_source_document_key,
                        owner_vector,
                        owner_physical_index,
                    ));
                };
                Some(materialize_minimal_owner_local(
                    root,
                    expected_source_document_key,
                    subject,
                    relation,
                    occurrence,
                    steps,
                ))
            }
        }
    }

    pub(crate) fn from_verified_root(root: wire::RoadEditingSource<'_>) -> Self {
        let header = root.module_header();
        let mut strings = Vec::with_capacity(location_string_occurrence_count(root));
        strings.push(Arc::<str>::from(header.authoring_namespace_id()));
        let mut canvas_selections = Vec::<Arc<str>>::with_capacity(canvas_occurrence_count(root));

        macro_rules! collect_root {
            ($values:expr, $key:ident) => {
                for value in $values {
                    strings.push(Arc::from(value.$key()));
                    collect_canvas(&mut canvas_selections, value.canvas_selection());
                }
            };
        }

        for alignment in root.road_alignments() {
            strings.push(Arc::from(alignment.road_alignment_key()));
            collect_canvas(&mut canvas_selections, alignment.canvas_selection());
            collect_curve_canvas(&mut canvas_selections, alignment.reference_line());
        }
        collect_root!(root.road_corridors(), road_corridor_key);
        collect_root!(root.road_sections(), road_section_key);
        collect_root!(root.authoring_lanes(), authoring_lane_key);
        for edge in root.lane_edges() {
            strings.push(Arc::from(edge.lane_edge_key()));
            collect_canvas(&mut canvas_selections, edge.canvas_selection());
            if let Some(curve) = edge.explicit_geometry() {
                collect_curve_canvas(&mut canvas_selections, curve);
            }
        }
        collect_root!(root.junctions(), junction_key);
        collect_root!(root.movements(), movement_key);
        collect_root!(root.maneuver_paths(), maneuver_path_key);
        collect_root!(root.maneuver_gates(), maneuver_gate_key);
        collect_root!(root.waiting_zones(), waiting_zone_key);
        collect_root!(root.stop_lines(), stop_line_key);
        collect_root!(root.signal_groups(), signal_group_key);
        collect_root!(root.signal_controllers(), signal_controller_key);
        collect_root!(root.signal_phases(), signal_phase_key);
        collect_root!(root.parking_areas(), parking_area_key);
        collect_root!(root.parking_spaces(), parking_space_key);
        collect_root!(root.lane_groups(), lane_group_key);
        collect_root!(root.facility_bands(), facility_band_key);
        collect_root!(root.participant_classes(), participant_class_key);
        collect_root!(root.access_rules(), access_rule_key);
        collect_root!(root.vehicle_profiles(), vehicle_profile_key);
        collect_root!(root.static_routes(), static_route_key);
        collect_root!(root.canonical_frames(), canonical_frame_key);

        strings.sort_unstable_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
        strings.dedup_by(|left, right| left.as_bytes() == right.as_bytes());
        canvas_selections.sort_unstable_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
        canvas_selections.dedup_by(|left, right| left.as_bytes() == right.as_bytes());
        let mut property_paths = closed_property_paths();
        property_paths.sort_unstable();
        property_paths.dedup();

        let namespace: Arc<str> = Arc::from(header.authoring_namespace_id());
        let document_key: Arc<str> = Arc::from(header.source_document_key());
        Self {
            context: Arc::new(RoadEditingLocationContext::new(
                strings.into_boxed_slice(),
                property_paths.into_boxed_slice(),
                canvas_selections.into_boxed_slice(),
            )),
            document_identity: RoadEditingDocumentIdentity::verified(namespace, document_key),
        }
    }

    pub(crate) fn declaration(
        &self,
        entity_kind: EntityKind,
        owner_local_keys: &[&str],
        local_key: &str,
        canvas_selection: Option<&str>,
    ) -> SourceLocation {
        self.location(
            RoadEditingSubject::Declaration {
                address: self.address(
                    RoadEditingAddressKind::Declaration(entity_kind),
                    owner_local_keys,
                    local_key,
                ),
            },
            None,
            canvas_selection,
        )
    }

    pub(crate) fn road_alignment(
        &self,
        road_alignment_key: &str,
        canvas_selection: Option<&str>,
    ) -> SourceLocation {
        self.location(
            RoadEditingSubject::RoadAlignment {
                address: self.address(
                    RoadEditingAddressKind::RoadAlignment,
                    &[],
                    road_alignment_key,
                ),
            },
            None,
            canvas_selection,
        )
    }

    pub(crate) fn road_alignment_property(
        &self,
        road_alignment_key: &str,
        steps: &[RoadEditingPropertyStep],
        canvas_selection: Option<&str>,
    ) -> SourceLocation {
        self.location(
            RoadEditingSubject::RoadAlignment {
                address: self.address(
                    RoadEditingAddressKind::RoadAlignment,
                    &[],
                    road_alignment_key,
                ),
            },
            Some(steps),
            canvas_selection,
        )
    }

    pub(crate) fn road_alignment_owner_local(
        &self,
        road_alignment_key: &str,
        relation: RoadEditingRelationKind,
        occurrence: RoadEditingRelationOccurrence,
        steps: &[RoadEditingPropertyStep],
        canvas_selection: Option<&str>,
    ) -> SourceLocation {
        self.owner_local_address(
            RoadEditingAddressKind::RoadAlignment,
            &[],
            road_alignment_key,
            relation,
            occurrence,
            steps,
            canvas_selection,
        )
    }

    pub(crate) fn property(
        &self,
        entity_kind: EntityKind,
        owner_local_keys: &[&str],
        local_key: &str,
        steps: &[RoadEditingPropertyStep],
        canvas_selection: Option<&str>,
    ) -> SourceLocation {
        self.location(
            RoadEditingSubject::Declaration {
                address: self.address(
                    RoadEditingAddressKind::Declaration(entity_kind),
                    owner_local_keys,
                    local_key,
                ),
            },
            Some(steps),
            canvas_selection,
        )
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "closed typed location fields remain explicit at relation call sites"
    )]
    pub(crate) fn owner_local(
        &self,
        owner_kind: EntityKind,
        owner_local_keys: &[&str],
        owner_key: &str,
        relation: RoadEditingRelationKind,
        occurrence: RoadEditingRelationOccurrence,
        steps: &[RoadEditingPropertyStep],
        canvas_selection: Option<&str>,
    ) -> SourceLocation {
        self.owner_local_address(
            RoadEditingAddressKind::Declaration(owner_kind),
            owner_local_keys,
            owner_key,
            relation,
            occurrence,
            steps,
            canvas_selection,
        )
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "closed typed location fields remain explicit at relation call sites"
    )]
    fn owner_local_address(
        &self,
        owner_kind: RoadEditingAddressKind,
        owner_local_keys: &[&str],
        owner_key: &str,
        relation: RoadEditingRelationKind,
        occurrence: RoadEditingRelationOccurrence,
        steps: &[RoadEditingPropertyStep],
        canvas_selection: Option<&str>,
    ) -> SourceLocation {
        self.location(
            RoadEditingSubject::OwnerLocal {
                owner: RoadEditingOwner::Address(self.address(
                    owner_kind,
                    owner_local_keys,
                    owner_key,
                )),
                relation,
                occurrence,
            },
            Some(steps),
            canvas_selection,
        )
    }

    pub(crate) fn module_owner_local(
        &self,
        relation: RoadEditingRelationKind,
        occurrence: RoadEditingRelationOccurrence,
        steps: &[RoadEditingPropertyStep],
    ) -> SourceLocation {
        self.location(
            RoadEditingSubject::OwnerLocal {
                owner: RoadEditingOwner::ModuleHeader,
                relation,
                occurrence,
            },
            Some(steps),
            None,
        )
    }

    pub(crate) fn module_header(&self) -> SourceLocation {
        self.location(RoadEditingSubject::ModuleHeader, None, None)
    }

    fn module_header_property(&self, steps: &[RoadEditingPropertyStep]) -> SourceLocation {
        self.location(RoadEditingSubject::ModuleHeader, Some(steps), None)
    }

    pub(crate) fn controlled_live_bytes(&self) -> u64 {
        self.context.controlled_live_bytes()
    }

    fn address(
        &self,
        kind: RoadEditingAddressKind,
        owner_local_keys: &[&str],
        local_key: &str,
    ) -> RoadEditingSourceAddress {
        let module_namespace = self
            .document_identity
            .module_namespace()
            .expect("verified road-editing identity retains namespace");
        RoadEditingSourceAddress::new(
            self.context.string_ordinal_for(module_namespace),
            kind,
            owner_local_keys
                .iter()
                .map(|key| self.context.string_ordinal_for(key)),
            self.context.string_ordinal_for(local_key),
        )
    }

    fn location(
        &self,
        subject: RoadEditingSubject,
        property_steps: Option<&[RoadEditingPropertyStep]>,
        canvas_selection: Option<&str>,
    ) -> SourceLocation {
        let property_path = property_steps.map(|steps| {
            let path = RoadEditingPropertyPath::new(steps.to_vec().into_boxed_slice());
            self.context.property_path_ordinal_for(&path)
        });
        let canvas_selection =
            canvas_selection.map(|value| self.context.canvas_selection_ordinal_for(value));
        SourceLocation::RoadEditing(RoadEditingSourceLocation::new(
            Arc::clone(&self.context),
            self.document_identity.clone(),
            subject,
            property_path,
            canvas_selection,
            None,
        ))
    }

    fn minimal(
        root: wire::RoadEditingSource<'_>,
        expected_source_document_key: &str,
        owner_local_keys: MinimalOwnerKeys<'_>,
        local_key: Option<&str>,
        canvas_selection: Option<&str>,
        property_steps: &[RoadEditingPropertyStep],
    ) -> Self {
        let header = root.module_header();
        let mut strings = Vec::with_capacity(
            1_usize
                .saturating_add(owner_local_keys.as_slice().len())
                .saturating_add(usize::from(local_key.is_some())),
        );
        strings.push(Arc::<str>::from(header.authoring_namespace_id()));
        strings.extend(
            owner_local_keys
                .as_slice()
                .iter()
                .map(|value| Arc::from(*value)),
        );
        if let Some(local_key) = local_key {
            strings.push(Arc::from(local_key));
        }
        strings.sort_unstable_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
        strings.dedup_by(|left, right| left.as_bytes() == right.as_bytes());

        let property_paths: Box<[RoadEditingPropertyPath]> = if property_steps.is_empty() {
            Box::default()
        } else {
            Box::new([RoadEditingPropertyPath::new(
                property_steps.to_vec().into_boxed_slice(),
            )]) as Box<[RoadEditingPropertyPath]>
        };
        let canvas_selections: Box<[Arc<str>]> = canvas_selection
            .map(|value| Box::new([Arc::<str>::from(value)]) as Box<[Arc<str>]>)
            .unwrap_or_default();
        Self {
            context: Arc::new(RoadEditingLocationContext::new(
                strings.into_boxed_slice(),
                property_paths,
                canvas_selections,
            )),
            document_identity: RoadEditingDocumentIdentity::verified(
                Arc::from(header.authoring_namespace_id()),
                Arc::from(expected_source_document_key),
            ),
        }
    }
}

fn materialize_minimal_root(
    root: wire::RoadEditingSource<'_>,
    expected_source_document_key: &str,
    subject: MinimalRootSubject<'_>,
    steps: &[RoadEditingPropertyStep],
) -> SourceLocation {
    match subject {
        MinimalRootSubject::RoadAlignment {
            key,
            canvas_selection,
        } => {
            let factory = RoadEditingLocationFactory::minimal(
                root,
                expected_source_document_key,
                MinimalOwnerKeys::None,
                Some(key),
                canvas_selection,
                steps,
            );
            if steps.is_empty() {
                factory.road_alignment(key, canvas_selection)
            } else {
                factory.road_alignment_property(key, steps, canvas_selection)
            }
        }
        MinimalRootSubject::Declaration {
            entity_kind,
            owner_local_keys,
            local_key,
            canvas_selection,
        } => {
            let factory = RoadEditingLocationFactory::minimal(
                root,
                expected_source_document_key,
                owner_local_keys,
                Some(local_key),
                canvas_selection,
                steps,
            );
            if steps.is_empty() {
                factory.declaration(
                    entity_kind,
                    owner_local_keys.as_slice(),
                    local_key,
                    canvas_selection,
                )
            } else {
                factory.property(
                    entity_kind,
                    owner_local_keys.as_slice(),
                    local_key,
                    steps,
                    canvas_selection,
                )
            }
        }
    }
}

fn materialize_minimal_owner_local(
    root: wire::RoadEditingSource<'_>,
    expected_source_document_key: &str,
    subject: MinimalRootSubject<'_>,
    relation: RoadEditingRelationKind,
    occurrence: RoadEditingRelationOccurrence,
    steps: &[RoadEditingPropertyStep],
) -> SourceLocation {
    match subject {
        MinimalRootSubject::RoadAlignment {
            key,
            canvas_selection,
        } => {
            let factory = RoadEditingLocationFactory::minimal(
                root,
                expected_source_document_key,
                MinimalOwnerKeys::None,
                Some(key),
                canvas_selection,
                steps,
            );
            factory.road_alignment_owner_local(key, relation, occurrence, steps, canvas_selection)
        }
        MinimalRootSubject::Declaration {
            entity_kind,
            owner_local_keys,
            local_key,
            canvas_selection,
        } => {
            let factory = RoadEditingLocationFactory::minimal(
                root,
                expected_source_document_key,
                owner_local_keys,
                Some(local_key),
                canvas_selection,
                steps,
            );
            factory.owner_local(
                entity_kind,
                owner_local_keys.as_slice(),
                local_key,
                relation,
                occurrence,
                steps,
                canvas_selection,
            )
        }
    }
}

fn verified_wire_fallback(
    root: wire::RoadEditingSource<'_>,
    expected_source_document_key: &str,
    root_vector: RoadEditingRootVectorKind,
    physical_index: u32,
) -> SourceLocation {
    let factory = RoadEditingLocationFactory::minimal(
        root,
        expected_source_document_key,
        MinimalOwnerKeys::None,
        None,
        None,
        &[],
    );
    factory.location(
        RoadEditingSubject::Wire {
            root_vector,
            physical_index,
            table: root_table(root_vector),
        },
        None,
        None,
    )
}

fn root_table(root_vector: RoadEditingRootVectorKind) -> RoadEditingTableKind {
    match root_vector {
        RoadEditingRootVectorKind::RoadAlignment => RoadEditingTableKind::RoadAlignment,
        RoadEditingRootVectorKind::RoadCorridor => RoadEditingTableKind::RoadCorridor,
        RoadEditingRootVectorKind::RoadSection => RoadEditingTableKind::RoadSection,
        RoadEditingRootVectorKind::AuthoringLane => RoadEditingTableKind::AuthoringLane,
        RoadEditingRootVectorKind::LaneEdge => RoadEditingTableKind::LaneEdge,
        RoadEditingRootVectorKind::Junction => RoadEditingTableKind::Junction,
        RoadEditingRootVectorKind::Movement => RoadEditingTableKind::Movement,
        RoadEditingRootVectorKind::ManeuverPath => RoadEditingTableKind::ManeuverPath,
        RoadEditingRootVectorKind::ManeuverGate => RoadEditingTableKind::ManeuverGate,
        RoadEditingRootVectorKind::WaitingZone => RoadEditingTableKind::WaitingZone,
        RoadEditingRootVectorKind::StopLine => RoadEditingTableKind::StopLine,
        RoadEditingRootVectorKind::SignalGroup => RoadEditingTableKind::SignalGroup,
        RoadEditingRootVectorKind::SignalController => RoadEditingTableKind::SignalController,
        RoadEditingRootVectorKind::SignalPhase => RoadEditingTableKind::SignalPhase,
        RoadEditingRootVectorKind::ParkingArea => RoadEditingTableKind::ParkingArea,
        RoadEditingRootVectorKind::ParkingSpace => RoadEditingTableKind::ParkingSpace,
        RoadEditingRootVectorKind::LaneGroup => RoadEditingTableKind::LaneGroup,
        RoadEditingRootVectorKind::FacilityBand => RoadEditingTableKind::FacilityBand,
        RoadEditingRootVectorKind::ParticipantClass => RoadEditingTableKind::ParticipantClass,
        RoadEditingRootVectorKind::AccessRule => RoadEditingTableKind::AccessRule,
        RoadEditingRootVectorKind::VehicleProfile => RoadEditingTableKind::VehicleProfile,
        RoadEditingRootVectorKind::StaticRoute => RoadEditingTableKind::StaticRoute,
        RoadEditingRootVectorKind::CanonicalFrame => RoadEditingTableKind::CanonicalFrame,
    }
}

fn minimal_root_subject(
    root: wire::RoadEditingSource<'_>,
    vector: RoadEditingRootVectorKind,
    physical_index: u32,
) -> Option<MinimalRootSubject<'_>> {
    let index = usize::try_from(physical_index).ok()?;
    macro_rules! module_declaration {
        ($values:expr, $kind:expr, $key:ident) => {{
            let values = $values;
            if index >= values.len() {
                None
            } else {
                let value = values.get(index);
                Some(MinimalRootSubject::Declaration {
                    entity_kind: $kind,
                    owner_local_keys: MinimalOwnerKeys::None,
                    local_key: valid_local_key(value.$key())?,
                    canvas_selection: value.canvas_selection().and_then(valid_local_key),
                })
            }
        }};
    }
    macro_rules! owner_declaration {
        ($values:expr, $kind:expr, $key:ident, $owner:ident, $depth:expr) => {{
            let values = $values;
            if index >= values.len() {
                None
            } else {
                let value = values.get(index);
                Some(MinimalRootSubject::Declaration {
                    entity_kind: $kind,
                    owner_local_keys: minimal_owner_keys(value.$owner(), $depth)?,
                    local_key: valid_local_key(value.$key())?,
                    canvas_selection: value.canvas_selection().and_then(valid_local_key),
                })
            }
        }};
    }

    match vector {
        RoadEditingRootVectorKind::RoadAlignment => {
            let values = root.road_alignments();
            if index >= values.len() {
                None
            } else {
                let value = values.get(index);
                Some(MinimalRootSubject::RoadAlignment {
                    key: valid_local_key(value.road_alignment_key())?,
                    canvas_selection: value.canvas_selection().and_then(valid_local_key),
                })
            }
        }
        RoadEditingRootVectorKind::RoadCorridor => module_declaration!(
            root.road_corridors(),
            EntityKind::RoadCorridor,
            road_corridor_key
        ),
        RoadEditingRootVectorKind::RoadSection => owner_declaration!(
            root.road_sections(),
            EntityKind::RoadSection,
            road_section_key,
            road_corridor,
            1
        ),
        RoadEditingRootVectorKind::AuthoringLane => owner_declaration!(
            root.authoring_lanes(),
            EntityKind::AuthoringLane,
            authoring_lane_key,
            road_section,
            2
        ),
        RoadEditingRootVectorKind::LaneEdge => {
            module_declaration!(root.lane_edges(), EntityKind::LaneEdge, lane_edge_key)
        }
        RoadEditingRootVectorKind::Junction => {
            module_declaration!(root.junctions(), EntityKind::Junction, junction_key)
        }
        RoadEditingRootVectorKind::Movement => owner_declaration!(
            root.movements(),
            EntityKind::Movement,
            movement_key,
            junction,
            1
        ),
        RoadEditingRootVectorKind::ManeuverPath => owner_declaration!(
            root.maneuver_paths(),
            EntityKind::ManeuverPath,
            maneuver_path_key,
            movement,
            2
        ),
        RoadEditingRootVectorKind::ManeuverGate => owner_declaration!(
            root.maneuver_gates(),
            EntityKind::ManeuverGate,
            maneuver_gate_key,
            maneuver_path,
            3
        ),
        RoadEditingRootVectorKind::WaitingZone => owner_declaration!(
            root.waiting_zones(),
            EntityKind::WaitingZone,
            waiting_zone_key,
            maneuver_path,
            3
        ),
        RoadEditingRootVectorKind::StopLine => {
            module_declaration!(root.stop_lines(), EntityKind::StopLine, stop_line_key)
        }
        RoadEditingRootVectorKind::SignalGroup => module_declaration!(
            root.signal_groups(),
            EntityKind::SignalGroup,
            signal_group_key
        ),
        RoadEditingRootVectorKind::SignalController => module_declaration!(
            root.signal_controllers(),
            EntityKind::SignalController,
            signal_controller_key
        ),
        RoadEditingRootVectorKind::SignalPhase => owner_declaration!(
            root.signal_phases(),
            EntityKind::SignalPhase,
            signal_phase_key,
            signal_controller,
            1
        ),
        RoadEditingRootVectorKind::ParkingArea => module_declaration!(
            root.parking_areas(),
            EntityKind::ParkingArea,
            parking_area_key
        ),
        RoadEditingRootVectorKind::ParkingSpace => module_declaration!(
            root.parking_spaces(),
            EntityKind::ParkingSpace,
            parking_space_key
        ),
        RoadEditingRootVectorKind::LaneGroup => owner_declaration!(
            root.lane_groups(),
            EntityKind::LaneGroup,
            lane_group_key,
            road_section,
            2
        ),
        RoadEditingRootVectorKind::FacilityBand => owner_declaration!(
            root.facility_bands(),
            EntityKind::FacilityBand,
            facility_band_key,
            road_corridor,
            1
        ),
        RoadEditingRootVectorKind::ParticipantClass => module_declaration!(
            root.participant_classes(),
            EntityKind::ParticipantClass,
            participant_class_key
        ),
        RoadEditingRootVectorKind::AccessRule => {
            module_declaration!(root.access_rules(), EntityKind::AccessRule, access_rule_key)
        }
        RoadEditingRootVectorKind::VehicleProfile => module_declaration!(
            root.vehicle_profiles(),
            EntityKind::VehicleProfile,
            vehicle_profile_key
        ),
        RoadEditingRootVectorKind::StaticRoute => module_declaration!(
            root.static_routes(),
            EntityKind::StaticRoute,
            static_route_key
        ),
        RoadEditingRootVectorKind::CanonicalFrame => module_declaration!(
            root.canonical_frames(),
            EntityKind::CanonicalFrame,
            canonical_frame_key
        ),
    }
}

fn valid_local_key(value: &str) -> Option<&str> {
    validate_wire_reference(value, 1, false).ok()?;
    Some(value)
}

fn minimal_owner_keys(value: &str, component_count: u8) -> Option<MinimalOwnerKeys<'_>> {
    let parsed = validate_wire_reference(value, component_count, false).ok()?;
    if parsed.namespace().is_some() {
        return None;
    }
    let mut components = parsed.key_components();
    match component_count {
        1 => Some(MinimalOwnerKeys::One([components.next()?])),
        2 => Some(MinimalOwnerKeys::Two([
            components.next()?,
            components.next()?,
        ])),
        3 => Some(MinimalOwnerKeys::Three([
            components.next()?,
            components.next()?,
            components.next()?,
        ])),
        _ => None,
    }
}

fn semantic_root_property_path(
    vector: RoadEditingRootVectorKind,
    field: Option<&str>,
) -> SemanticPreflightPropertyPath {
    if field != Some("canvasSelection") {
        return semantic_property_path(field);
    }
    let (table, field_id) = match vector {
        RoadEditingRootVectorKind::RoadAlignment => (RoadEditingTableKind::RoadAlignment, 3),
        RoadEditingRootVectorKind::RoadCorridor => (RoadEditingTableKind::RoadCorridor, 8),
        RoadEditingRootVectorKind::RoadSection => (RoadEditingTableKind::RoadSection, 3),
        RoadEditingRootVectorKind::AuthoringLane => (RoadEditingTableKind::AuthoringLane, 5),
        RoadEditingRootVectorKind::LaneEdge => (RoadEditingTableKind::LaneEdge, 4),
        RoadEditingRootVectorKind::Junction => (RoadEditingTableKind::Junction, 3),
        RoadEditingRootVectorKind::Movement => (RoadEditingTableKind::Movement, 4),
        RoadEditingRootVectorKind::ManeuverPath => (RoadEditingTableKind::ManeuverPath, 5),
        RoadEditingRootVectorKind::ManeuverGate => (RoadEditingTableKind::ManeuverGate, 6),
        RoadEditingRootVectorKind::WaitingZone => (RoadEditingTableKind::WaitingZone, 5),
        RoadEditingRootVectorKind::StopLine => (RoadEditingTableKind::StopLine, 2),
        RoadEditingRootVectorKind::SignalGroup => (RoadEditingTableKind::SignalGroup, 1),
        RoadEditingRootVectorKind::SignalController => (RoadEditingTableKind::SignalController, 4),
        RoadEditingRootVectorKind::SignalPhase => (RoadEditingTableKind::SignalPhase, 3),
        RoadEditingRootVectorKind::ParkingArea => (RoadEditingTableKind::ParkingArea, 1),
        RoadEditingRootVectorKind::ParkingSpace => (RoadEditingTableKind::ParkingSpace, 5),
        RoadEditingRootVectorKind::LaneGroup => (RoadEditingTableKind::LaneGroup, 2),
        RoadEditingRootVectorKind::FacilityBand => (RoadEditingTableKind::FacilityBand, 3),
        RoadEditingRootVectorKind::ParticipantClass => (RoadEditingTableKind::ParticipantClass, 2),
        RoadEditingRootVectorKind::AccessRule => (RoadEditingTableKind::AccessRule, 7),
        RoadEditingRootVectorKind::VehicleProfile => (RoadEditingTableKind::VehicleProfile, 3),
        RoadEditingRootVectorKind::StaticRoute => (RoadEditingTableKind::StaticRoute, 2),
        RoadEditingRootVectorKind::CanonicalFrame => (RoadEditingTableKind::CanonicalFrame, 1),
    };
    table_field(table, field_id)
}

fn semantic_owner_local_property_path(
    relation: RoadEditingRelationKind,
    field: Option<&str>,
) -> SemanticPreflightPropertyPath {
    if field == Some("canvasSelection") && relation == RoadEditingRelationKind::CurveSegment {
        table_field(RoadEditingTableKind::CurveSegment, 2)
    } else {
        semantic_property_path(field)
    }
}

fn semantic_property_path(field: Option<&str>) -> SemanticPreflightPropertyPath {
    let Some(field) = field else {
        return SemanticPreflightPropertyPath::None;
    };
    let direct = match field {
        "roadEditingSource.geometryAccuracyProfile" => {
            Some((RoadEditingTableKind::RoadEditingSource, 2))
        }
        "roadEditingSource.geometryDirectionProfile" => {
            Some((RoadEditingTableKind::RoadEditingSource, 3))
        }
        "moduleHeader.authoringNamespaceId" => Some((RoadEditingTableKind::ModuleHeader, 0)),
        "moduleHeader.sourceDocumentKey" => Some((RoadEditingTableKind::ModuleHeader, 1)),
        "moduleHeader.imports" => Some((RoadEditingTableKind::ModuleHeader, 2)),
        "moduleHeader.provenance" => Some((RoadEditingTableKind::ModuleHeader, 3)),
        "curveProgram.start" => Some((RoadEditingTableKind::CurveProgram, 0)),
        "curveProgram.segments" => Some((RoadEditingTableKind::CurveProgram, 1)),
        "curveSegment.geometry" => Some((RoadEditingTableKind::CurveSegment, 1)),
        "roadAlignment.roadAlignmentKey" | "roadAlignments.roadAlignmentKey" => {
            Some((RoadEditingTableKind::RoadAlignment, 0))
        }
        "roadAlignment.canonicalFrame" => Some((RoadEditingTableKind::RoadAlignment, 1)),
        "roadCorridor.roadCorridorKey" | "roadCorridors.roadCorridorKey" => {
            Some((RoadEditingTableKind::RoadCorridor, 0))
        }
        "roadCorridor.roadAlignmentKey" => Some((RoadEditingTableKind::RoadCorridor, 1)),
        "roadCorridor.startStationMeters" => Some((RoadEditingTableKind::RoadCorridor, 2)),
        "roadCorridor.endStationKind" => Some((RoadEditingTableKind::RoadCorridor, 3)),
        "roadCorridor.endStationMeters" => Some((RoadEditingTableKind::RoadCorridor, 4)),
        "roadCorridor.referenceSection" => Some((RoadEditingTableKind::RoadCorridor, 5)),
        "roadCorridor.referenceLane" => Some((RoadEditingTableKind::RoadCorridor, 6)),
        "roadCorridor.elements" => Some((RoadEditingTableKind::RoadCorridor, 7)),
        "roadSection.roadSectionKey" | "roadSections.address" => {
            Some((RoadEditingTableKind::RoadSection, 0))
        }
        "roadSection.kindId" => Some((RoadEditingTableKind::RoadSection, 1)),
        "roadSection.authoringLanes" => Some((RoadEditingTableKind::RoadSection, 2)),
        "roadSection.roadCorridor" => Some((RoadEditingTableKind::RoadSection, 4)),
        "authoringLane.authoringLaneKey" | "authoringLanes.address" => {
            Some((RoadEditingTableKind::AuthoringLane, 0))
        }
        "authoringLane.laneEdge" => Some((RoadEditingTableKind::AuthoringLane, 1)),
        "authoringLane.direction" => Some((RoadEditingTableKind::AuthoringLane, 2)),
        "authoringLane.widthProfile" => Some((RoadEditingTableKind::AuthoringLane, 3)),
        "authoringLane.laneGroup" => Some((RoadEditingTableKind::AuthoringLane, 4)),
        "authoringLane.roadSection" => Some((RoadEditingTableKind::AuthoringLane, 6)),
        "laneEdge.laneEdgeKey" | "laneEdges.laneEdgeKey" => {
            Some((RoadEditingTableKind::LaneEdge, 0))
        }
        "laneEdge.speedLimitMetersPerSecond" => Some((RoadEditingTableKind::LaneEdge, 1)),
        "laneEdge.successors" => Some((RoadEditingTableKind::LaneEdge, 2)),
        "junction.junctionKey" | "junctions.junctionKey" => {
            Some((RoadEditingTableKind::Junction, 0))
        }
        "junction.approachEdges" => Some((RoadEditingTableKind::Junction, 1)),
        "junction.internalEdges" | "junction.edgeRoles" => {
            Some((RoadEditingTableKind::Junction, 2))
        }
        "movement.movementKey" | "movements.address" => Some((RoadEditingTableKind::Movement, 0)),
        "movement.junction" => Some((RoadEditingTableKind::Movement, 1)),
        "movement.directedEntryApproachKey" => Some((RoadEditingTableKind::Movement, 2)),
        "movement.directedExitApproachKey" => Some((RoadEditingTableKind::Movement, 3)),
        "maneuverPath.maneuverPathKey" | "maneuverPaths.address" => {
            Some((RoadEditingTableKind::ManeuverPath, 0))
        }
        "maneuverPath.movement" => Some((RoadEditingTableKind::ManeuverPath, 1)),
        "maneuverPath.entryEdge" => Some((RoadEditingTableKind::ManeuverPath, 2)),
        "maneuverPath.internalEdges" => Some((RoadEditingTableKind::ManeuverPath, 3)),
        "maneuverPath.exitEdge" => Some((RoadEditingTableKind::ManeuverPath, 4)),
        "maneuverGate.maneuverGateKey" | "maneuverGates.address" => {
            Some((RoadEditingTableKind::ManeuverGate, 0))
        }
        "maneuverGate.maneuverPath" => Some((RoadEditingTableKind::ManeuverGate, 1)),
        "maneuverGate.stopLine" => Some((RoadEditingTableKind::ManeuverGate, 3)),
        "maneuverGate.signalControl" => Some((RoadEditingTableKind::ManeuverGate, 4)),
        "maneuverGate.signalGroup" => Some((RoadEditingTableKind::ManeuverGate, 5)),
        "waitingZone.waitingZoneKey" | "waitingZones.address" => {
            Some((RoadEditingTableKind::WaitingZone, 0))
        }
        "waitingZone.maneuverPath" => Some((RoadEditingTableKind::WaitingZone, 1)),
        "waitingZone.entryGate" => Some((RoadEditingTableKind::WaitingZone, 2)),
        "waitingZone.releaseGate" => Some((RoadEditingTableKind::WaitingZone, 3)),
        "waitingZone.maxOccupancy" => Some((RoadEditingTableKind::WaitingZone, 4)),
        "stopLine.stopLineKey" | "stopLines.stopLineKey" => {
            Some((RoadEditingTableKind::StopLine, 0))
        }
        "stopLine.laneEdge" => Some((RoadEditingTableKind::StopLine, 1)),
        "signalGroup.signalGroupKey" | "signalGroups.signalGroupKey" => {
            Some((RoadEditingTableKind::SignalGroup, 0))
        }
        "signalController.signalControllerKey" | "signalControllers.signalControllerKey" => {
            Some((RoadEditingTableKind::SignalController, 0))
        }
        "signalController.signalGroups" => Some((RoadEditingTableKind::SignalController, 2)),
        "signalController.signalPhases" => Some((RoadEditingTableKind::SignalController, 3)),
        "signalPhase.signalPhaseKey" | "signalPhases.address" => {
            Some((RoadEditingTableKind::SignalPhase, 0))
        }
        "signalPhase.durationMilliseconds" => Some((RoadEditingTableKind::SignalPhase, 1)),
        "signalPhase.states" => Some((RoadEditingTableKind::SignalPhase, 2)),
        "signalPhase.signalController" => Some((RoadEditingTableKind::SignalPhase, 4)),
        "parkingArea.parkingAreaKey" | "parkingAreas.parkingAreaKey" => {
            Some((RoadEditingTableKind::ParkingArea, 0))
        }
        "parkingSpace.parkingSpaceKey" | "parkingSpaces.parkingSpaceKey" => {
            Some((RoadEditingTableKind::ParkingSpace, 0))
        }
        "parkingSpace.parkingArea" => Some((RoadEditingTableKind::ParkingSpace, 1)),
        "parkingSpace.entry" => Some((RoadEditingTableKind::ParkingSpace, 2)),
        "parkingSpace.exit" => Some((RoadEditingTableKind::ParkingSpace, 3)),
        "laneGroup.laneGroupKey" | "laneGroups.address" => {
            Some((RoadEditingTableKind::LaneGroup, 0))
        }
        "laneGroup.roadSection" => Some((RoadEditingTableKind::LaneGroup, 1)),
        "facilityBand.facilityBandKey" | "facilityBands.address" => {
            Some((RoadEditingTableKind::FacilityBand, 0))
        }
        "facilityBand.kindId" => Some((RoadEditingTableKind::FacilityBand, 1)),
        "facilityBand.widthProfile" => Some((RoadEditingTableKind::FacilityBand, 2)),
        "facilityBand.roadCorridor" => Some((RoadEditingTableKind::FacilityBand, 4)),
        "participantClass.participantClassKey" | "participantClasses.participantClassKey" => {
            Some((RoadEditingTableKind::ParticipantClass, 0))
        }
        "participantClass.extends" => Some((RoadEditingTableKind::ParticipantClass, 1)),
        "accessRule.accessRuleKey" | "accessRules.accessRuleKey" => {
            Some((RoadEditingTableKind::AccessRule, 0))
        }
        "accessRule.targetKind" => Some((RoadEditingTableKind::AccessRule, 1)),
        "accessRule.targetReference" => Some((RoadEditingTableKind::AccessRule, 2)),
        "accessRule.effect" => Some((RoadEditingTableKind::AccessRule, 3)),
        "accessRule.participantClasses" => Some((RoadEditingTableKind::AccessRule, 4)),
        "vehicleProfile.vehicleProfileKey" | "vehicleProfiles.vehicleProfileKey" => {
            Some((RoadEditingTableKind::VehicleProfile, 0))
        }
        "vehicleProfile.participantClass" => Some((RoadEditingTableKind::VehicleProfile, 1)),
        "staticRoute.staticRouteKey" | "staticRoutes.staticRouteKey" => {
            Some((RoadEditingTableKind::StaticRoute, 0))
        }
        "staticRoute.edgeSequence" => Some((RoadEditingTableKind::StaticRoute, 1)),
        "canonicalFrame.canonicalFrameKey" | "canonicalFrames.canonicalFrameKey" => {
            Some((RoadEditingTableKind::CanonicalFrame, 0))
        }
        _ => None,
    };
    if let Some((table, field_id)) = direct {
        return table_field(table, field_id);
    }

    match field {
        "moduleHeader.provenance.kind" => nested_table_field(
            RoadEditingTableKind::ModuleHeader,
            3,
            RoadEditingTableKind::Provenance,
            0,
        ),
        "moduleHeader.provenance.generatorBuildId" => nested_table_field(
            RoadEditingTableKind::ModuleHeader,
            3,
            RoadEditingTableKind::Provenance,
            1,
        ),
        "moduleHeader.provenance.description" => nested_table_field(
            RoadEditingTableKind::ModuleHeader,
            3,
            RoadEditingTableKind::Provenance,
            5,
        ),
        "roadCorridor.elements.kind" => nested_table_field(
            RoadEditingTableKind::RoadCorridor,
            7,
            RoadEditingTableKind::CorridorElement,
            0,
        ),
        "roadCorridor.elements.entityReference" => nested_table_field(
            RoadEditingTableKind::RoadCorridor,
            7,
            RoadEditingTableKind::CorridorElement,
            1,
        ),
        "signalPhase.states.signalGroup" => nested_table_field(
            RoadEditingTableKind::SignalPhase,
            2,
            RoadEditingTableKind::SignalPhaseState,
            0,
        ),
        "signalPhase.states.aspect" => nested_table_field(
            RoadEditingTableKind::SignalPhase,
            2,
            RoadEditingTableKind::SignalPhaseState,
            1,
        ),
        "parkingSpace.geometry.lateralOffsetMeters" => nested_table_field(
            RoadEditingTableKind::ParkingSpace,
            4,
            RoadEditingTableKind::ParkingSpaceGeometry,
            0,
        ),
        "parkingSpace.geometry.headingOffsetRadians" => nested_table_field(
            RoadEditingTableKind::ParkingSpace,
            4,
            RoadEditingTableKind::ParkingSpaceGeometry,
            1,
        ),
        "parkingSpace.geometry.lengthMeters" => nested_table_field(
            RoadEditingTableKind::ParkingSpace,
            4,
            RoadEditingTableKind::ParkingSpaceGeometry,
            2,
        ),
        "parkingSpace.geometry.widthMeters" => nested_table_field(
            RoadEditingTableKind::ParkingSpace,
            4,
            RoadEditingTableKind::ParkingSpaceGeometry,
            3,
        ),
        "accessRegulation.jurisdiction" => nested_table_field(
            RoadEditingTableKind::AccessRule,
            5,
            RoadEditingTableKind::AccessRegulation,
            0,
        ),
        "accessRegulation.version" => nested_table_field(
            RoadEditingTableKind::AccessRule,
            5,
            RoadEditingTableKind::AccessRegulation,
            1,
        ),
        "accessRegulation.source" => nested_table_field(
            RoadEditingTableKind::AccessRule,
            5,
            RoadEditingTableKind::AccessRegulation,
            2,
        ),
        "vehicleProfile.iidm.lengthMeters" => nested_table_field(
            RoadEditingTableKind::VehicleProfile,
            2,
            RoadEditingTableKind::IidmVehicleProfile,
            0,
        ),
        "vehicleProfile.iidm.desiredSpeedMetersPerSecond" => nested_table_field(
            RoadEditingTableKind::VehicleProfile,
            2,
            RoadEditingTableKind::IidmVehicleProfile,
            1,
        ),
        "vehicleProfile.iidm.timeHeadwaySeconds" => nested_table_field(
            RoadEditingTableKind::VehicleProfile,
            2,
            RoadEditingTableKind::IidmVehicleProfile,
            3,
        ),
        "vehicleProfile.iidm.minGapMeters" => nested_table_field(
            RoadEditingTableKind::VehicleProfile,
            2,
            RoadEditingTableKind::IidmVehicleProfile,
            2,
        ),
        "vehicleProfile.iidm.maxAccelerationMetersPerSecondSquared" => nested_table_field(
            RoadEditingTableKind::VehicleProfile,
            2,
            RoadEditingTableKind::IidmVehicleProfile,
            4,
        ),
        "vehicleProfile.iidm.comfortableDecelerationMetersPerSecondSquared" => nested_table_field(
            RoadEditingTableKind::VehicleProfile,
            2,
            RoadEditingTableKind::IidmVehicleProfile,
            5,
        ),
        "vehicleProfile.iidm.emergencyDecelerationMetersPerSecondSquared" => nested_table_field(
            RoadEditingTableKind::VehicleProfile,
            2,
            RoadEditingTableKind::IidmVehicleProfile,
            6,
        ),
        "curveSegment.geometry.line.end" => {
            curve_geometry_property(1, RoadEditingTableKind::LineSegment, 0)
        }
        "curveSegment.geometry.cubic.control1" => {
            curve_geometry_property(2, RoadEditingTableKind::CubicBezierSegment, 0)
        }
        "curveSegment.geometry.cubic.control2" => {
            curve_geometry_property(2, RoadEditingTableKind::CubicBezierSegment, 1)
        }
        "curveSegment.geometry.cubic.end" => {
            curve_geometry_property(2, RoadEditingTableKind::CubicBezierSegment, 2)
        }
        "roadAlignment.referenceLine.start.x" => {
            curve_start_property(RoadEditingTableKind::RoadAlignment, 2, 0)
        }
        "roadAlignment.referenceLine.start.y" => {
            curve_start_property(RoadEditingTableKind::RoadAlignment, 2, 1)
        }
        "roadAlignment.referenceLine.start.z" => {
            curve_start_property(RoadEditingTableKind::RoadAlignment, 2, 2)
        }
        "laneEdge.explicitGeometry.start.x" => {
            curve_start_property(RoadEditingTableKind::LaneEdge, 3, 0)
        }
        "laneEdge.explicitGeometry.start.y" => {
            curve_start_property(RoadEditingTableKind::LaneEdge, 3, 1)
        }
        "laneEdge.explicitGeometry.start.z" => {
            curve_start_property(RoadEditingTableKind::LaneEdge, 3, 2)
        }
        "curveSegment.geometry.line.end.x" => {
            curve_geometry_member_property(1, RoadEditingTableKind::LineSegment, 0, 0)
        }
        "curveSegment.geometry.line.end.y" => {
            curve_geometry_member_property(1, RoadEditingTableKind::LineSegment, 0, 1)
        }
        "curveSegment.geometry.line.end.z" => {
            curve_geometry_member_property(1, RoadEditingTableKind::LineSegment, 0, 2)
        }
        "curveSegment.geometry.cubic.control1.x" => {
            curve_geometry_member_property(2, RoadEditingTableKind::CubicBezierSegment, 0, 0)
        }
        "curveSegment.geometry.cubic.control1.y" => {
            curve_geometry_member_property(2, RoadEditingTableKind::CubicBezierSegment, 0, 1)
        }
        "curveSegment.geometry.cubic.control1.z" => {
            curve_geometry_member_property(2, RoadEditingTableKind::CubicBezierSegment, 0, 2)
        }
        "curveSegment.geometry.cubic.control2.x" => {
            curve_geometry_member_property(2, RoadEditingTableKind::CubicBezierSegment, 1, 0)
        }
        "curveSegment.geometry.cubic.control2.y" => {
            curve_geometry_member_property(2, RoadEditingTableKind::CubicBezierSegment, 1, 1)
        }
        "curveSegment.geometry.cubic.control2.z" => {
            curve_geometry_member_property(2, RoadEditingTableKind::CubicBezierSegment, 1, 2)
        }
        "curveSegment.geometry.cubic.end.x" => {
            curve_geometry_member_property(2, RoadEditingTableKind::CubicBezierSegment, 2, 0)
        }
        "curveSegment.geometry.cubic.end.y" => {
            curve_geometry_member_property(2, RoadEditingTableKind::CubicBezierSegment, 2, 1)
        }
        "curveSegment.geometry.cubic.end.z" => {
            curve_geometry_member_property(2, RoadEditingTableKind::CubicBezierSegment, 2, 2)
        }
        "authoringLane.widthProfile.startWidthMeters" => {
            width_member_property(RoadEditingTableKind::AuthoringLane, 3, 0)
        }
        "authoringLane.widthProfile.endWidthMeters" => {
            width_member_property(RoadEditingTableKind::AuthoringLane, 3, 1)
        }
        "facilityBand.widthProfile.startWidthMeters" => {
            width_member_property(RoadEditingTableKind::FacilityBand, 2, 0)
        }
        "facilityBand.widthProfile.endWidthMeters" => {
            width_member_property(RoadEditingTableKind::FacilityBand, 2, 1)
        }
        "parkingSpace.entry.laneEdge" => parking_anchor_property(2, 0),
        "parkingSpace.entry.progressMeters" => parking_anchor_property(2, 1),
        "parkingSpace.exit.laneEdge" => parking_anchor_property(3, 0),
        "parkingSpace.exit.progressMeters" => parking_anchor_property(3, 1),
        _ => SemanticPreflightPropertyPath::None,
    }
}

fn curve_start_property(
    owner_table: RoadEditingTableKind,
    owner_field_id: u16,
    member_id: u8,
) -> SemanticPreflightPropertyPath {
    SemanticPreflightPropertyPath::Three([
        RoadEditingPropertyStep::TableField {
            table: owner_table,
            field_id: owner_field_id,
        },
        RoadEditingPropertyStep::TableField {
            table: RoadEditingTableKind::CurveProgram,
            field_id: 0,
        },
        RoadEditingPropertyStep::StructMember {
            structure: RoadEditingStructKind::Vec3F64,
            member_id,
        },
    ])
}

fn curve_geometry_member_property(
    discriminant: u8,
    table: RoadEditingTableKind,
    field_id: u16,
    member_id: u8,
) -> SemanticPreflightPropertyPath {
    let SemanticPreflightPropertyPath::Three(prefix) =
        curve_geometry_property(discriminant, table, field_id)
    else {
        unreachable!("curve geometry property depth is fixed")
    };
    SemanticPreflightPropertyPath::Four([
        prefix[0],
        prefix[1],
        prefix[2],
        RoadEditingPropertyStep::StructMember {
            structure: RoadEditingStructKind::Vec3F64,
            member_id,
        },
    ])
}

fn width_member_property(
    table: RoadEditingTableKind,
    field_id: u16,
    member_id: u8,
) -> SemanticPreflightPropertyPath {
    SemanticPreflightPropertyPath::Two([
        RoadEditingPropertyStep::TableField { table, field_id },
        RoadEditingPropertyStep::StructMember {
            structure: RoadEditingStructKind::LinearWidthProfile,
            member_id,
        },
    ])
}

fn parking_anchor_property(
    outer_field_id: u16,
    inner_field_id: u16,
) -> SemanticPreflightPropertyPath {
    nested_table_field(
        RoadEditingTableKind::ParkingSpace,
        outer_field_id,
        RoadEditingTableKind::ParkingLaneAnchor,
        inner_field_id,
    )
}

fn table_field(table: RoadEditingTableKind, field_id: u16) -> SemanticPreflightPropertyPath {
    SemanticPreflightPropertyPath::One([RoadEditingPropertyStep::TableField { table, field_id }])
}

fn nested_table_field(
    outer_table: RoadEditingTableKind,
    outer_field_id: u16,
    inner_table: RoadEditingTableKind,
    inner_field_id: u16,
) -> SemanticPreflightPropertyPath {
    SemanticPreflightPropertyPath::Two([
        RoadEditingPropertyStep::TableField {
            table: outer_table,
            field_id: outer_field_id,
        },
        RoadEditingPropertyStep::TableField {
            table: inner_table,
            field_id: inner_field_id,
        },
    ])
}

fn curve_geometry_property(
    discriminant: u8,
    table: RoadEditingTableKind,
    field_id: u16,
) -> SemanticPreflightPropertyPath {
    SemanticPreflightPropertyPath::Three([
        RoadEditingPropertyStep::TableField {
            table: RoadEditingTableKind::CurveSegment,
            field_id: 1,
        },
        RoadEditingPropertyStep::UnionVariant {
            union: RoadEditingUnionKind::CurveSegmentGeometry,
            discriminant,
        },
        RoadEditingPropertyStep::TableField { table, field_id },
    ])
}

fn empty_context() -> Arc<RoadEditingLocationContext> {
    Arc::new(RoadEditingLocationContext::new(
        Box::default(),
        Box::default(),
        Box::default(),
    ))
}

fn collect_canvas(output: &mut Vec<Arc<str>>, value: Option<&str>) {
    if let Some(value) = value {
        output.push(Arc::from(value));
    }
}

fn collect_curve_canvas(output: &mut Vec<Arc<str>>, curve: wire::CurveProgram<'_>) {
    for segment in curve.segments() {
        collect_canvas(output, segment.canvas_selection());
    }
}

fn location_string_occurrence_count(root: wire::RoadEditingSource<'_>) -> usize {
    1_usize
        .saturating_add(root.road_alignments().len())
        .saturating_add(root.road_corridors().len())
        .saturating_add(root.road_sections().len())
        .saturating_add(root.authoring_lanes().len())
        .saturating_add(root.lane_edges().len())
        .saturating_add(root.junctions().len())
        .saturating_add(root.movements().len())
        .saturating_add(root.maneuver_paths().len())
        .saturating_add(root.maneuver_gates().len())
        .saturating_add(root.waiting_zones().len())
        .saturating_add(root.stop_lines().len())
        .saturating_add(root.signal_groups().len())
        .saturating_add(root.signal_controllers().len())
        .saturating_add(root.signal_phases().len())
        .saturating_add(root.parking_areas().len())
        .saturating_add(root.parking_spaces().len())
        .saturating_add(root.lane_groups().len())
        .saturating_add(root.facility_bands().len())
        .saturating_add(root.participant_classes().len())
        .saturating_add(root.access_rules().len())
        .saturating_add(root.vehicle_profiles().len())
        .saturating_add(root.static_routes().len())
        .saturating_add(root.canonical_frames().len())
}

fn canvas_occurrence_count(root: wire::RoadEditingSource<'_>) -> usize {
    let mut count = 0_usize;
    macro_rules! charge_root {
        ($values:expr) => {
            count = count.saturating_add(
                $values
                    .iter()
                    .filter(|value| value.canvas_selection().is_some())
                    .count(),
            );
        };
    }
    charge_root!(root.road_alignments());
    charge_root!(root.road_corridors());
    charge_root!(root.road_sections());
    charge_root!(root.authoring_lanes());
    charge_root!(root.lane_edges());
    charge_root!(root.junctions());
    charge_root!(root.movements());
    charge_root!(root.maneuver_paths());
    charge_root!(root.maneuver_gates());
    charge_root!(root.waiting_zones());
    charge_root!(root.stop_lines());
    charge_root!(root.signal_groups());
    charge_root!(root.signal_controllers());
    charge_root!(root.signal_phases());
    charge_root!(root.parking_areas());
    charge_root!(root.parking_spaces());
    charge_root!(root.lane_groups());
    charge_root!(root.facility_bands());
    charge_root!(root.participant_classes());
    charge_root!(root.access_rules());
    charge_root!(root.vehicle_profiles());
    charge_root!(root.static_routes());
    charge_root!(root.canonical_frames());
    for alignment in root.road_alignments() {
        count = count.saturating_add(
            alignment
                .reference_line()
                .segments()
                .iter()
                .filter(|segment| segment.canvas_selection().is_some())
                .count(),
        );
    }
    for edge in root.lane_edges() {
        if let Some(curve) = edge.explicit_geometry() {
            count = count.saturating_add(
                curve
                    .segments()
                    .iter()
                    .filter(|segment| segment.canvas_selection().is_some())
                    .count(),
            );
        }
    }
    count
}

fn closed_property_paths() -> Vec<RoadEditingPropertyPath> {
    let tables = [
        (RoadEditingTableKind::RoadEditingSource, 26_u16),
        (RoadEditingTableKind::ModuleHeader, 3),
        (RoadEditingTableKind::Provenance, 5),
        (RoadEditingTableKind::LineSegment, 0),
        (RoadEditingTableKind::CubicBezierSegment, 2),
        (RoadEditingTableKind::CurveSegment, 2),
        (RoadEditingTableKind::CurveProgram, 1),
        (RoadEditingTableKind::RoadAlignment, 3),
        (RoadEditingTableKind::CorridorElement, 1),
        (RoadEditingTableKind::RoadCorridor, 8),
        (RoadEditingTableKind::RoadSection, 4),
        (RoadEditingTableKind::AuthoringLane, 6),
        (RoadEditingTableKind::LaneEdge, 4),
        (RoadEditingTableKind::Junction, 3),
        (RoadEditingTableKind::Movement, 4),
        (RoadEditingTableKind::ManeuverPath, 5),
        (RoadEditingTableKind::ManeuverGate, 6),
        (RoadEditingTableKind::WaitingZone, 5),
        (RoadEditingTableKind::StopLine, 2),
        (RoadEditingTableKind::SignalGroup, 1),
        (RoadEditingTableKind::SignalController, 4),
        (RoadEditingTableKind::SignalPhaseState, 1),
        (RoadEditingTableKind::SignalPhase, 4),
        (RoadEditingTableKind::ParkingArea, 1),
        (RoadEditingTableKind::ParkingLaneAnchor, 1),
        (RoadEditingTableKind::ParkingSpaceGeometry, 3),
        (RoadEditingTableKind::ParkingSpace, 5),
        (RoadEditingTableKind::LaneGroup, 2),
        (RoadEditingTableKind::FacilityBand, 4),
        (RoadEditingTableKind::ParticipantClass, 2),
        (RoadEditingTableKind::AccessRegulation, 2),
        (RoadEditingTableKind::AccessRule, 7),
        (RoadEditingTableKind::IidmVehicleProfile, 6),
        (RoadEditingTableKind::VehicleProfile, 3),
        (RoadEditingTableKind::StaticRoute, 2),
        (RoadEditingTableKind::CanonicalFrame, 1),
    ];
    let mut paths = Vec::with_capacity(512);
    for (table, last_field_id) in tables {
        for field_id in 0..=last_field_id {
            paths.push(RoadEditingPropertyPath::new(Box::new([
                RoadEditingPropertyStep::TableField { table, field_id },
            ])));
        }
    }
    for (table, field_id, structure, members) in [
        (
            RoadEditingTableKind::Provenance,
            2,
            RoadEditingStructKind::Digest256,
            1_u8,
        ),
        (
            RoadEditingTableKind::Provenance,
            3,
            RoadEditingStructKind::Digest256,
            1,
        ),
        (
            RoadEditingTableKind::Provenance,
            4,
            RoadEditingStructKind::OptionalU64,
            1,
        ),
        (
            RoadEditingTableKind::CurveProgram,
            0,
            RoadEditingStructKind::Vec3F64,
            3,
        ),
        (
            RoadEditingTableKind::LineSegment,
            0,
            RoadEditingStructKind::Vec3F64,
            3,
        ),
        (
            RoadEditingTableKind::AuthoringLane,
            3,
            RoadEditingStructKind::LinearWidthProfile,
            2,
        ),
        (
            RoadEditingTableKind::FacilityBand,
            2,
            RoadEditingStructKind::LinearWidthProfile,
            2,
        ),
    ] {
        for member_id in 0..members {
            paths.push(RoadEditingPropertyPath::new(Box::new([
                RoadEditingPropertyStep::TableField { table, field_id },
                RoadEditingPropertyStep::StructMember {
                    structure,
                    member_id,
                },
            ])));
        }
    }
    for (outer_table, outer_field_id, inner_table, inner_last_field_id) in [
        (
            RoadEditingTableKind::ModuleHeader,
            3,
            RoadEditingTableKind::Provenance,
            5_u16,
        ),
        (
            RoadEditingTableKind::RoadAlignment,
            2,
            RoadEditingTableKind::CurveProgram,
            1,
        ),
        (
            RoadEditingTableKind::LaneEdge,
            3,
            RoadEditingTableKind::CurveProgram,
            1,
        ),
        (
            RoadEditingTableKind::RoadCorridor,
            7,
            RoadEditingTableKind::CorridorElement,
            1,
        ),
        (
            RoadEditingTableKind::SignalPhase,
            2,
            RoadEditingTableKind::SignalPhaseState,
            1,
        ),
        (
            RoadEditingTableKind::ParkingSpace,
            2,
            RoadEditingTableKind::ParkingLaneAnchor,
            1,
        ),
        (
            RoadEditingTableKind::ParkingSpace,
            3,
            RoadEditingTableKind::ParkingLaneAnchor,
            1,
        ),
        (
            RoadEditingTableKind::ParkingSpace,
            4,
            RoadEditingTableKind::ParkingSpaceGeometry,
            3,
        ),
        (
            RoadEditingTableKind::AccessRule,
            5,
            RoadEditingTableKind::AccessRegulation,
            2,
        ),
        (
            RoadEditingTableKind::VehicleProfile,
            2,
            RoadEditingTableKind::IidmVehicleProfile,
            6,
        ),
    ] {
        for inner_field_id in 0..=inner_last_field_id {
            paths.push(RoadEditingPropertyPath::new(Box::new([
                RoadEditingPropertyStep::TableField {
                    table: outer_table,
                    field_id: outer_field_id,
                },
                RoadEditingPropertyStep::TableField {
                    table: inner_table,
                    field_id: inner_field_id,
                },
            ])));
        }
    }
    for (outer_table, outer_field_id) in [
        (RoadEditingTableKind::RoadAlignment, 2_u16),
        (RoadEditingTableKind::LaneEdge, 3),
    ] {
        for member_id in 0..3_u8 {
            paths.push(RoadEditingPropertyPath::new(Box::new([
                RoadEditingPropertyStep::TableField {
                    table: outer_table,
                    field_id: outer_field_id,
                },
                RoadEditingPropertyStep::TableField {
                    table: RoadEditingTableKind::CurveProgram,
                    field_id: 0,
                },
                RoadEditingPropertyStep::StructMember {
                    structure: RoadEditingStructKind::Vec3F64,
                    member_id,
                },
            ])));
        }
    }
    for (field_id, structure, members) in [
        (2_u16, RoadEditingStructKind::Digest256, 1_u8),
        (3, RoadEditingStructKind::Digest256, 1),
        (4, RoadEditingStructKind::OptionalU64, 1),
    ] {
        for member_id in 0..members {
            paths.push(RoadEditingPropertyPath::new(Box::new([
                RoadEditingPropertyStep::TableField {
                    table: RoadEditingTableKind::ModuleHeader,
                    field_id: 3,
                },
                RoadEditingPropertyStep::TableField {
                    table: RoadEditingTableKind::Provenance,
                    field_id,
                },
                RoadEditingPropertyStep::StructMember {
                    structure,
                    member_id,
                },
            ])));
        }
    }
    for (field_id, members) in [(0_u16, 3_u8), (1, 3), (2, 3)] {
        for member_id in 0..members {
            paths.push(RoadEditingPropertyPath::new(Box::new([
                RoadEditingPropertyStep::TableField {
                    table: RoadEditingTableKind::CubicBezierSegment,
                    field_id,
                },
                RoadEditingPropertyStep::StructMember {
                    structure: RoadEditingStructKind::Vec3F64,
                    member_id,
                },
            ])));
        }
    }
    for (outer_table, outer_field_id, inner_table, inner_field_id) in [
        (
            RoadEditingTableKind::RoadCorridor,
            7,
            RoadEditingTableKind::CorridorElement,
            1,
        ),
        (
            RoadEditingTableKind::SignalPhase,
            2,
            RoadEditingTableKind::SignalPhaseState,
            0,
        ),
        (
            RoadEditingTableKind::ParkingSpace,
            2,
            RoadEditingTableKind::ParkingLaneAnchor,
            0,
        ),
        (
            RoadEditingTableKind::ParkingSpace,
            3,
            RoadEditingTableKind::ParkingLaneAnchor,
            0,
        ),
    ] {
        paths.push(RoadEditingPropertyPath::new(Box::new([
            RoadEditingPropertyStep::TableField {
                table: outer_table,
                field_id: outer_field_id,
            },
            RoadEditingPropertyStep::TableField {
                table: inner_table,
                field_id: inner_field_id,
            },
        ])));
    }
    for (variant, table, field_count) in [
        (1_u8, RoadEditingTableKind::LineSegment, 1_u16),
        (2, RoadEditingTableKind::CubicBezierSegment, 3),
    ] {
        for field_id in 0..field_count {
            for member_id in 0..3_u8 {
                paths.push(RoadEditingPropertyPath::new(Box::new([
                    RoadEditingPropertyStep::TableField {
                        table: RoadEditingTableKind::CurveSegment,
                        field_id: 1,
                    },
                    RoadEditingPropertyStep::UnionVariant {
                        union: RoadEditingUnionKind::CurveSegmentGeometry,
                        discriminant: variant,
                    },
                    RoadEditingPropertyStep::TableField { table, field_id },
                    RoadEditingPropertyStep::StructMember {
                        structure: RoadEditingStructKind::Vec3F64,
                        member_id,
                    },
                ])));
            }
        }
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::road_editing::{
        CanonicalFrameInput, RoadEditingDeclaration, RoadEditingModuleHeader,
        RoadEditingModuleInput, RoadEditingProvenance, RoadEditingSourceModuleBuilder,
        RoadEditingSourceWriter,
    };
    use crate::{CompileLimits, GeometryAccuracyProfile, GeometryDirectionProfile};

    #[test]
    fn factory_resolves_owner_address_property_and_canvas_after_wire_order_changes() {
        let limits = CompileLimits::p100_initial_v2();
        let header = RoadEditingModuleHeader::try_new(
            "city/main",
            "roads/main",
            Vec::new(),
            RoadEditingProvenance::direct("test").unwrap(),
        )
        .unwrap();
        let mut builder = RoadEditingSourceModuleBuilder::new(
            header,
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            &limits,
        )
        .unwrap();
        builder
            .add_declaration(RoadEditingDeclaration::CanonicalFrame(
                CanonicalFrameInput::try_new("frame-main")
                    .unwrap()
                    .with_canvas_selection("canvas/frame-main")
                    .unwrap(),
            ))
            .unwrap();
        let module = builder.finish().unwrap();
        let bytes = RoadEditingSourceWriter::new(&limits).write(module).unwrap();
        let input = RoadEditingModuleInput::try_new("roads/main", bytes.as_bytes(), None).unwrap();
        let verified = super::super::reader::verify_source(input, &limits, 0, 0).unwrap();
        let factory = RoadEditingLocationFactory::from_verified_root(verified.root());
        let location = factory.property(
            EntityKind::CanonicalFrame,
            &[],
            "frame-main",
            &[RoadEditingPropertyStep::TableField {
                table: RoadEditingTableKind::CanonicalFrame,
                field_id: 0,
            }],
            Some("canvas/frame-main"),
        );
        let road = location.road_editing().unwrap();
        let RoadEditingSubject::Declaration { address } = road.subject() else {
            panic!("expected declaration subject");
        };
        assert_eq!(address.module_namespace(road.context()), "city/main");
        assert_eq!(address.local_key(road.context()), "frame-main");
        assert_eq!(road.canvas_selection(), Some("canvas/frame-main"));
        assert_eq!(road.property_path().unwrap().steps().len(), 1);
    }

    #[test]
    fn factory_represents_module_import_as_module_owned_relation() {
        let limits = CompileLimits::p100_initial_v2();
        let header = RoadEditingModuleHeader::try_new(
            "city/main",
            "roads/main",
            vec!["city/base".to_owned()],
            RoadEditingProvenance::direct("test").unwrap(),
        )
        .unwrap();
        let module = RoadEditingSourceModuleBuilder::new(
            header,
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            &limits,
        )
        .unwrap()
        .finish()
        .unwrap();
        let bytes = RoadEditingSourceWriter::new(&limits).write(module).unwrap();
        let input = RoadEditingModuleInput::try_new("roads/main", bytes.as_bytes(), None).unwrap();
        let verified = super::super::reader::verify_source(input, &limits, 0, 0).unwrap();
        let factory = RoadEditingLocationFactory::from_verified_root(verified.root());
        let location = factory.module_owner_local(
            RoadEditingRelationKind::Import,
            RoadEditingRelationOccurrence::CanonicalSetOrdinal(0),
            &[RoadEditingPropertyStep::TableField {
                table: RoadEditingTableKind::ModuleHeader,
                field_id: 2,
            }],
        );
        let road = location.road_editing().unwrap();

        assert!(matches!(
            road.subject(),
            RoadEditingSubject::OwnerLocal {
                owner: RoadEditingOwner::ModuleHeader,
                relation: RoadEditingRelationKind::Import,
                occurrence: RoadEditingRelationOccurrence::CanonicalSetOrdinal(0),
            }
        ));
    }

    #[test]
    fn closed_paths_cover_nested_table_leaves() {
        let paths = closed_property_paths();
        for expected in [
            RoadEditingPropertyPath::new(Box::new([
                RoadEditingPropertyStep::TableField {
                    table: RoadEditingTableKind::ParkingSpace,
                    field_id: 2,
                },
                RoadEditingPropertyStep::TableField {
                    table: RoadEditingTableKind::ParkingLaneAnchor,
                    field_id: 1,
                },
            ])),
            RoadEditingPropertyPath::new(Box::new([
                RoadEditingPropertyStep::TableField {
                    table: RoadEditingTableKind::VehicleProfile,
                    field_id: 2,
                },
                RoadEditingPropertyStep::TableField {
                    table: RoadEditingTableKind::IidmVehicleProfile,
                    field_id: 6,
                },
            ])),
            RoadEditingPropertyPath::new(Box::new([
                RoadEditingPropertyStep::TableField {
                    table: RoadEditingTableKind::ModuleHeader,
                    field_id: 3,
                },
                RoadEditingPropertyStep::TableField {
                    table: RoadEditingTableKind::Provenance,
                    field_id: 2,
                },
                RoadEditingPropertyStep::StructMember {
                    structure: RoadEditingStructKind::Digest256,
                    member_id: 0,
                },
            ])),
        ] {
            assert!(
                paths.contains(&expected),
                "missing nested path: {expected:?}"
            );
        }
    }

    #[test]
    fn semantic_preflight_maps_scalar_leaf_paths_exactly() {
        assert_eq!(
            semantic_property_path(Some("curveSegment.geometry.cubic.control2.z")).as_slice(),
            &[
                RoadEditingPropertyStep::TableField {
                    table: RoadEditingTableKind::CurveSegment,
                    field_id: 1,
                },
                RoadEditingPropertyStep::UnionVariant {
                    union: RoadEditingUnionKind::CurveSegmentGeometry,
                    discriminant: 2,
                },
                RoadEditingPropertyStep::TableField {
                    table: RoadEditingTableKind::CubicBezierSegment,
                    field_id: 1,
                },
                RoadEditingPropertyStep::StructMember {
                    structure: RoadEditingStructKind::Vec3F64,
                    member_id: 2,
                },
            ]
        );
        assert_eq!(
            semantic_property_path(Some("authoringLane.widthProfile.endWidthMeters")).as_slice(),
            &[
                RoadEditingPropertyStep::TableField {
                    table: RoadEditingTableKind::AuthoringLane,
                    field_id: 3,
                },
                RoadEditingPropertyStep::StructMember {
                    structure: RoadEditingStructKind::LinearWidthProfile,
                    member_id: 1,
                },
            ]
        );
        assert_eq!(
            semantic_property_path(Some("parkingSpace.entry.progressMeters")).as_slice(),
            &[
                RoadEditingPropertyStep::TableField {
                    table: RoadEditingTableKind::ParkingSpace,
                    field_id: 2,
                },
                RoadEditingPropertyStep::TableField {
                    table: RoadEditingTableKind::ParkingLaneAnchor,
                    field_id: 1,
                },
            ]
        );
    }

    #[test]
    fn semantic_preflight_materializes_minimal_owner_scoped_declaration_context() {
        let limits = CompileLimits::p100_initial_v2();
        let module = super::super::writer::tests::module_with_every_declaration(&limits);
        let bytes = RoadEditingSourceWriter::new(&limits).write(module).unwrap();
        let root = wire::size_prefixed_root_as_road_editing_source(bytes.as_bytes()).unwrap();

        let location = RoadEditingLocationFactory::semantic_preflight(
            root,
            "road-editing",
            SemanticPreflightSite::root(
                RoadEditingRootVectorKind::AuthoringLane,
                0,
                Some("authoringLane.direction"),
            ),
        )
        .expect("stable owner-scoped location");
        let road = location.road_editing().unwrap();
        let RoadEditingSubject::Declaration { address } = road.subject() else {
            panic!("expected declaration subject");
        };
        assert_eq!(address.entity_kind(), Some(EntityKind::AuthoringLane));
        assert_eq!(
            address.owner_local_keys(road.context()).collect::<Vec<_>>(),
            ["corridor", "section"]
        );
        assert_eq!(address.local_key(road.context()), "lane");
        assert_eq!(
            road.property_path().unwrap().steps(),
            &[RoadEditingPropertyStep::TableField {
                table: RoadEditingTableKind::AuthoringLane,
                field_id: 2,
            }]
        );

        let full = RoadEditingLocationFactory::from_verified_root(root);
        assert!(road.context().controlled_live_bytes() < full.controlled_live_bytes());
    }

    #[test]
    fn semantic_preflight_materializes_curve_segment_owner_local_site() {
        let limits = CompileLimits::p100_initial_v2();
        let module = super::super::writer::tests::module_with_every_declaration(&limits);
        let bytes = RoadEditingSourceWriter::new(&limits).write(module).unwrap();
        let root = wire::size_prefixed_root_as_road_editing_source(bytes.as_bytes()).unwrap();

        let location = RoadEditingLocationFactory::semantic_preflight(
            root,
            "road-editing",
            SemanticPreflightSite::owner_local(
                RoadEditingRootVectorKind::RoadAlignment,
                0,
                RoadEditingRelationKind::CurveSegment,
                RoadEditingRelationOccurrence::OrderedProductOrdinal(0),
                Some("curveSegment.geometry.line.end"),
            ),
        )
        .expect("curve segment location");
        let road = location.road_editing().unwrap();
        assert!(matches!(
            road.subject(),
            RoadEditingSubject::OwnerLocal {
                owner: RoadEditingOwner::Address(_),
                relation: RoadEditingRelationKind::CurveSegment,
                occurrence: RoadEditingRelationOccurrence::OrderedProductOrdinal(0),
            }
        ));
        assert_eq!(road.property_path().unwrap().steps().len(), 3);
    }

    #[test]
    fn semantic_preflight_maps_iidm_sibling_fields_exactly() {
        let limits = CompileLimits::p100_initial_v2();
        let module = super::super::writer::tests::module_with_every_declaration(&limits);
        let bytes = RoadEditingSourceWriter::new(&limits).write(module).unwrap();
        let root = wire::size_prefixed_root_as_road_editing_source(bytes.as_bytes()).unwrap();

        for (field, inner_field_id) in [
            ("vehicleProfile.iidm.minGapMeters", 2),
            ("vehicleProfile.iidm.timeHeadwaySeconds", 3),
        ] {
            let location = RoadEditingLocationFactory::semantic_preflight(
                root,
                "road-editing",
                SemanticPreflightSite::root(
                    RoadEditingRootVectorKind::VehicleProfile,
                    0,
                    Some(field),
                ),
            )
            .expect("vehicle profile property location");
            assert_eq!(
                location
                    .road_editing()
                    .unwrap()
                    .property_path()
                    .unwrap()
                    .steps(),
                &[
                    RoadEditingPropertyStep::TableField {
                        table: RoadEditingTableKind::VehicleProfile,
                        field_id: 2,
                    },
                    RoadEditingPropertyStep::TableField {
                        table: RoadEditingTableKind::IidmVehicleProfile,
                        field_id: inner_field_id,
                    },
                ]
            );
        }
    }

    #[test]
    fn road_alignment_segment_location_keeps_address_and_canvas() {
        let path = RoadEditingPropertyPath::new(Box::new([RoadEditingPropertyStep::TableField {
            table: RoadEditingTableKind::CurveSegment,
            field_id: 1,
        }]));
        let context = Arc::new(RoadEditingLocationContext::new(
            Box::new([Arc::from("alignment-main"), Arc::from("city/main")]),
            Box::new([path]),
            Box::new([Arc::from("canvas/segment-0")]),
        ));
        let factory = RoadEditingLocationFactory {
            context,
            document_identity: RoadEditingDocumentIdentity::verified(
                Arc::from("city/main"),
                Arc::from("roads/main"),
            ),
        };

        let location = factory.road_alignment_owner_local(
            "alignment-main",
            RoadEditingRelationKind::CurveSegment,
            RoadEditingRelationOccurrence::OrderedProductOrdinal(0),
            &[RoadEditingPropertyStep::TableField {
                table: RoadEditingTableKind::CurveSegment,
                field_id: 1,
            }],
            Some("canvas/segment-0"),
        );
        let road = location.road_editing().expect("road-editing location");

        assert_eq!(road.canvas_selection(), Some("canvas/segment-0"));
        let RoadEditingSubject::OwnerLocal {
            owner: RoadEditingOwner::Address(address),
            ..
        } = road.subject()
        else {
            panic!("road-alignment owner-local subject expected");
        };
        assert_eq!(address.kind(), RoadEditingAddressKind::RoadAlignment);
        assert_eq!(address.local_key(road.context()), "alignment-main");
    }

    #[test]
    fn semantic_preflight_uses_wire_fallback_for_invalid_owner_reference() {
        let limits = CompileLimits::p100_initial_v2();
        let module = super::super::writer::tests::module_with_every_declaration(&limits);
        let bytes = RoadEditingSourceWriter::new(&limits).write(module).unwrap();
        let mut corrupted = bytes.as_bytes().to_vec();
        let root = wire::size_prefixed_root_as_road_editing_source(&corrupted).unwrap();
        let owner = root.authoring_lanes().get(0).road_section();
        let owner_offset = (owner.as_ptr() as usize)
            .checked_sub(corrupted.as_ptr() as usize)
            .expect("owner string lies in buffer");
        corrupted[owner_offset] = b'_';
        let root = wire::size_prefixed_root_as_road_editing_source(&corrupted).unwrap();

        let location = RoadEditingLocationFactory::semantic_preflight(
            root,
            "road-editing",
            SemanticPreflightSite::root(
                RoadEditingRootVectorKind::AuthoringLane,
                0,
                Some("authoringLane.roadSection"),
            ),
        )
        .expect("verified wire fallback");
        let road = location.road_editing().unwrap();
        assert!(matches!(
            road.subject(),
            RoadEditingSubject::Wire {
                root_vector: RoadEditingRootVectorKind::AuthoringLane,
                physical_index: 0,
                table: RoadEditingTableKind::AuthoringLane,
            }
        ));
        assert!(road.property_path().is_none());
    }
}

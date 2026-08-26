use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use laneflow_static_contract::{
    CANONICAL_POINT_COMPONENT_MAX_METERS, CANONICAL_POINT_COMPONENT_MIN_METERS, EntityKind,
    FieldTag, HEADING_MINUS_PI_F32_BITS, HEADING_PLUS_PI_F32_BITS, JunctionKind, LaneEdgeKind,
    LaneGroupKind, MAX_ACCEL_METERS_PER_SECOND_SQUARED, MAX_MIN_GAP_MM,
    MAX_PARKING_LATERAL_OFFSET_ABS_MM, MAX_TIME_HEADWAY_SECONDS, MAX_VEHICLE_LENGTH_MM,
    MIN_ACCEL_METERS_PER_SECOND_SQUARED, MIN_PARKING_LATERAL_OFFSET_ABS_MM, MIN_VEHICLE_LENGTH_MM,
    ManeuverGateKind, ManeuverPathKind, MovementKind, ParticipantClassKind, RoadSectionKind,
    SignalGroupKind, StopLineKind, heading_f32_from_si, heading_f32_in_legal_closure,
    millimetres_from_si, millimetres_i32_from_si,
};

use crate::declaration::{
    AccessRuleDeclaration, AccessRuleInput, AccessRuleTargetInput, AdmittedIidmProfile,
    AdmittedParkingGeometry, AuthoringLaneDeclaration, CanonicalFrameDeclaration,
    CanonicalFrameInput, CanonicalPoint3F32Input, CorridorElementReference, DeclarationHeader,
    EdgeLength, FacilityBandDeclaration, FacilityBandInput, FacilityKindCategory,
    FacilityKindViolation, JunctionDeclaration, JunctionInput, LaneEdgeDeclaration,
    LaneEdgeGeometryAuthority, LaneEdgeGeometryDeclaration, LaneEdgeInput, LaneGroupDeclaration,
    LaneGroupInput, ManeuverGateDeclaration, ManeuverGateInput, ManeuverPathDeclaration,
    ManeuverPathInput, MovementDeclaration, MovementInput, OwnedAccessRegulation,
    OwnedAccessRuleTarget, OwnedCorridorElementReference, OwnedEntityReference, OwnedSignalControl,
    ParkingAreaDeclaration, ParkingAreaInput, ParkingLaneAnchorDeclaration,
    ParkingSpaceDeclaration, ParkingSpaceInput, ParticipantClassDeclaration, ParticipantClassInput,
    RoadCorridorDeclaration, RoadCorridorInput, RoadSectionDeclaration, RoadSectionInput,
    ScalarViolation, SignalControlInput, SignalControllerDeclaration, SignalControllerInput,
    SignalGroupDeclaration, SignalGroupInput, SignalGroupStateDeclaration, SignalPhaseDeclaration,
    SpeedLimit, StaticRouteDeclaration, StaticRouteInput, StopLineDeclaration, StopLineInput,
    TypedAstDeclaration, VehicleProfileDeclaration, VehicleProfileInput, WaitingZoneDeclaration,
    WaitingZoneInput, closed_millimetres, facility_kind_category,
};
use crate::diagnostic::DiagnosticCollector;
use crate::source::external_token_violation;
use crate::{
    CompileLimitDimension, CompileLimits, Diagnostic, DiagnosticBundle, ParkingAnchorRole,
    ParkingGeometryField, ParkingGeometryViolation, SourceLocation, SourceModuleHeader, SourceSpan,
    SpatialAxis, SpatialGeometryViolation,
};

use super::admission::{AdmittedOfficialModule, ImportRecord, TypedAstModule};
use super::descriptor::{
    SOURCE_DOCUMENT_SET_DIGEST_VERSION, SourceDocumentDescriptor, SourceDocumentOrigin,
    SourceLanguage, SourceModuleDescriptor, freeze_source_documents, source_document_digest,
};
use super::resources::{ModuleResourceCounts, size_bytes};
use super::synthetic_record::{
    access_rule_input_len, access_target_input_parts, canonical_frame_input_len,
    declaration_header_len, encode_source_record, encoded_reference_len, encoded_source_record_len,
    facility_band_declaration_len, lane_edge_declaration_base_len, lane_group_declaration_len,
    maneuver_gate_declaration_len, maneuver_path_declaration_len, movement_declaration_len,
    parking_space_input_len, participant_class_declaration_len, road_corridor_declaration_len,
    road_section_declaration_len, signal_controller_input_len, stop_line_declaration_len,
    vehicle_profile_declaration_len, waiting_zone_declaration_len,
};

/// 当前合成领域专用语言 `LFSOURCE` 来源记录编码版本。
///
/// `3`：准入后交通一维以整数毫米 / 受检 `f32` SI 写入来源记录，不再写编制 `f64`（#500）。
/// 拒绝 `StaticRoute` 声明不另升此值；只有记录布局变化才升（ADR 0029）。
pub const SYNTHETIC_FRONTEND_VERSION: u32 = 3;

pub struct SyntheticModuleBuilder {
    header: SourceModuleHeader,
    limits: CompileLimits,
    imports: Vec<ImportRecord>,
    import_index: HashMap<Arc<str>, usize>,
    declarations: Vec<TypedAstDeclaration>,
    declaration_index: HashMap<EntityKind, HashMap<Arc<str>, SourceLocation>>,
    declaration_count: u64,
    typed_ast_record_count: u64,
    reference_count: u64,
    relation_occurrence_count: u64,
    identity_field_occurrence_count: u64,
    symbol_count: u64,
    string_item_count: u64,
    string_bytes: u64,
    controlled_string_bytes: u64,
    controlled_structural_bytes: u64,
    source_record_byte_len: u64,
    maneuver_gate_count: u64,
    waiting_zone_count: u64,
    route_occurrence_count: u64,
    geometry_point_count: u64,
}

#[derive(Default)]
struct DeclarationResourceDelta {
    declarations: u64,
    typed_ast_records: u64,
    references: u64,
    relations: u64,
    identity_fields: u64,
    symbols: u64,
    string_items: u64,
    string_bytes: u64,
    controlled_string_bytes: u64,
    controlled_structural_bytes: u64,
    source_bytes: u64,
    maneuver_gates: u64,
    waiting_zones: u64,
    route_occurrences: u64,
    geometry_points: u64,
}

struct DeclarationResourceState {
    declaration_count: u64,
    typed_ast_record_count: u64,
    reference_count: u64,
    relation_occurrence_count: u64,
    identity_field_occurrence_count: u64,
    symbol_count: u64,
    string_item_count: u64,
    string_bytes: u64,
    controlled_string_bytes: u64,
    controlled_structural_bytes: u64,
    source_record_byte_len: u64,
    maneuver_gate_count: u64,
    waiting_zone_count: u64,
    route_occurrence_count: u64,
    geometry_point_count: u64,
}

impl SyntheticModuleBuilder {
    /// 建立一个只允许官方合成领域构造的来源模块构建器。
    ///
    /// # Errors
    ///
    /// 若空模块的基础 `LFSOURCE` 记录、逻辑字符串或编译器控制存续字节已经超过
    /// `limits`，返回资源上限诊断且不建立构建器。
    pub fn new(
        header: SourceModuleHeader,
        limits: &CompileLimits,
    ) -> Result<Self, DiagnosticBundle> {
        let string_bytes = header_resident_string_bytes(&header);
        let controlled_string_bytes = header_controlled_string_bytes(&header);
        let string_item_count = 2;
        let base_source_bytes = encoded_source_record_len(&header, &[], &[]).unwrap_or(u64::MAX);
        let mut diagnostics =
            DiagnosticCollector::new(limits.value(CompileLimitDimension::DiagnosticCount));
        push_limit_if_exceeded(
            &mut diagnostics,
            limits,
            CompileLimitDimension::StringItemCount,
            string_item_count,
            Some(header.declaration_span.clone()),
            Some(header.authoring_namespace_id.as_ref().into()),
        );
        let controlled_live_bytes = controlled_string_bytes.saturating_add(base_source_bytes);
        push_limit_if_exceeded(
            &mut diagnostics,
            limits,
            CompileLimitDimension::CompilerControlledLiveBytes,
            controlled_live_bytes,
            Some(header.declaration_span.clone()),
            Some(header.authoring_namespace_id.as_ref().into()),
        );
        push_limit_if_exceeded(
            &mut diagnostics,
            limits,
            CompileLimitDimension::TotalStringBytes,
            string_bytes,
            Some(header.declaration_span.clone()),
            Some(header.authoring_namespace_id.as_ref().into()),
        );
        push_limit_if_exceeded(
            &mut diagnostics,
            limits,
            CompileLimitDimension::SourceBytesPerModule,
            base_source_bytes,
            Some(header.declaration_span.clone()),
            Some(header.authoring_namespace_id.as_ref().into()),
        );
        if !diagnostics.is_empty() {
            return Err(diagnostics.finish());
        }

        Ok(Self {
            header,
            limits: limits.clone(),
            imports: Vec::new(),
            import_index: HashMap::new(),
            declarations: Vec::new(),
            declaration_index: HashMap::new(),
            declaration_count: 0,
            typed_ast_record_count: 1,
            reference_count: 0,
            relation_occurrence_count: 0,
            identity_field_occurrence_count: 0,
            symbol_count: 0,
            string_item_count,
            string_bytes,
            controlled_string_bytes,
            controlled_structural_bytes: 0,
            source_record_byte_len: base_source_bytes,
            maneuver_gate_count: 0,
            waiting_zone_count: 0,
            route_occurrence_count: 0,
            geometry_point_count: 0,
        })
    }

    fn validate_declaration_key(
        &self,
        entity_kind: EntityKind,
        stable_key: &str,
        span: &SourceSpan,
    ) -> Result<(), DiagnosticBundle> {
        if let Some(violation) = external_token_violation(
            stable_key,
            self.limits.value(CompileLimitDimension::SingleStringBytes),
        ) {
            return Err(DiagnosticBundle::single(
                Diagnostic::invalid_declaration_key(entity_kind, violation, span.clone()),
            ));
        }
        if let Some(existing_span) = self
            .declaration_index
            .get(&entity_kind)
            .and_then(|index| index.get(stable_key))
        {
            return Err(DiagnosticBundle::single(Diagnostic::duplicate_declaration(
                entity_kind,
                stable_key,
                span.clone(),
                existing_span.clone(),
            )));
        }
        Ok(())
    }

    fn validate_facility_kind(
        &self,
        entity_kind: EntityKind,
        stable_key: &str,
        kind_id: &str,
        expected_category: FacilityKindCategory,
        span: &SourceSpan,
    ) -> Result<(), DiagnosticBundle> {
        let violation = match external_token_violation(
            kind_id,
            self.limits.value(CompileLimitDimension::SingleStringBytes),
        ) {
            Some(violation) => Some(FacilityKindViolation::InvalidToken(violation)),
            None => match facility_kind_category(kind_id) {
                None => Some(FacilityKindViolation::Unknown),
                Some(actual) if actual != expected_category => {
                    Some(FacilityKindViolation::CategoryMismatch { actual })
                }
                Some(_) => None,
            },
        };
        if let Some(violation) = violation {
            return Err(DiagnosticBundle::single(Diagnostic::invalid_facility_kind(
                entity_kind,
                stable_key,
                kind_id,
                expected_category,
                violation,
                span.clone(),
            )));
        }
        Ok(())
    }

    fn validate_identity_ascii_field(
        &self,
        entity_kind: EntityKind,
        stable_key: &str,
        field_tag: FieldTag,
        value: &str,
        span: &SourceSpan,
    ) -> Result<(), DiagnosticBundle> {
        if let Some(violation) = external_token_violation(
            value,
            self.limits.value(CompileLimitDimension::SingleStringBytes),
        ) {
            return Err(DiagnosticBundle::single(
                Diagnostic::invalid_identity_ascii_field(
                    entity_kind,
                    stable_key,
                    field_tag,
                    violation,
                    span.clone(),
                ),
            ));
        }
        Ok(())
    }

    fn validate_reference<K: laneflow_static_contract::EntityKindMarker>(
        &self,
        target_kind: EntityKind,
        reference: crate::EntityReference<'_, K>,
        span: &SourceSpan,
    ) -> Result<(), DiagnosticBundle> {
        if let Some(namespace) = reference.module_namespace()
            && let Some(violation) = external_token_violation(
                namespace,
                self.limits.value(CompileLimitDimension::SingleStringBytes),
            )
        {
            return Err(DiagnosticBundle::single(
                Diagnostic::invalid_reference_namespace(violation, span.clone()),
            ));
        }
        self.reference_namespace(reference.module_namespace(), span)?;
        if let Some(violation) = external_token_violation(
            reference.declaration_key(),
            self.limits.value(CompileLimitDimension::SingleStringBytes),
        ) {
            return Err(DiagnosticBundle::single(Diagnostic::invalid_reference_key(
                target_kind,
                violation,
                span.clone(),
            )));
        }
        Ok(())
    }

    fn own_reference<K: laneflow_static_contract::EntityKindMarker>(
        &self,
        target_kind: EntityKind,
        reference: crate::EntityReference<'_, K>,
        span: &SourceSpan,
    ) -> Result<OwnedEntityReference<K>, DiagnosticBundle> {
        self.validate_reference(target_kind, reference, span)?;
        Ok(OwnedEntityReference::new(
            self.reference_namespace_arc(reference.module_namespace(), span)?,
            reference.declaration_key().into(),
            span.clone(),
        ))
    }

    fn check_declaration_resources(
        &self,
        delta: DeclarationResourceDelta,
        stable_key: &str,
        span: &SourceSpan,
    ) -> Result<DeclarationResourceState, DiagnosticBundle> {
        let state = DeclarationResourceState {
            declaration_count: self.declaration_count.saturating_add(delta.declarations),
            typed_ast_record_count: self
                .typed_ast_record_count
                .saturating_add(delta.typed_ast_records),
            reference_count: self.reference_count.saturating_add(delta.references),
            relation_occurrence_count: self
                .relation_occurrence_count
                .saturating_add(delta.relations),
            identity_field_occurrence_count: self
                .identity_field_occurrence_count
                .saturating_add(delta.identity_fields),
            symbol_count: self.symbol_count.saturating_add(delta.symbols),
            string_item_count: self.string_item_count.saturating_add(delta.string_items),
            string_bytes: self.string_bytes.saturating_add(delta.string_bytes),
            controlled_string_bytes: self
                .controlled_string_bytes
                .saturating_add(delta.controlled_string_bytes),
            controlled_structural_bytes: self
                .controlled_structural_bytes
                .saturating_add(delta.controlled_structural_bytes),
            source_record_byte_len: self
                .source_record_byte_len
                .saturating_add(delta.source_bytes),
            maneuver_gate_count: self
                .maneuver_gate_count
                .saturating_add(delta.maneuver_gates),
            waiting_zone_count: self.waiting_zone_count.saturating_add(delta.waiting_zones),
            route_occurrence_count: self
                .route_occurrence_count
                .saturating_add(delta.route_occurrences),
            geometry_point_count: self
                .geometry_point_count
                .saturating_add(delta.geometry_points),
        };
        let controlled_live_bytes = state
            .controlled_string_bytes
            .saturating_add(state.controlled_structural_bytes)
            .saturating_add(state.source_record_byte_len);
        for (dimension, observed) in [
            (
                CompileLimitDimension::DeclarationCount,
                state.declaration_count,
            ),
            (
                CompileLimitDimension::TypedAstRecordCount,
                state.typed_ast_record_count,
            ),
            (CompileLimitDimension::ReferenceCount, state.reference_count),
            (
                CompileLimitDimension::RelationOccurrenceCount,
                state.relation_occurrence_count,
            ),
            (
                CompileLimitDimension::IdentityFieldOccurrenceCount,
                state.identity_field_occurrence_count,
            ),
            (CompileLimitDimension::SymbolCount, state.symbol_count),
            (
                CompileLimitDimension::StringItemCount,
                state.string_item_count,
            ),
            (CompileLimitDimension::TotalStringBytes, state.string_bytes),
            (
                CompileLimitDimension::SourceBytesPerModule,
                state.source_record_byte_len,
            ),
            (
                CompileLimitDimension::CompilerControlledLiveBytes,
                controlled_live_bytes,
            ),
            (
                CompileLimitDimension::ManeuverGateCount,
                state.maneuver_gate_count,
            ),
            (
                CompileLimitDimension::WaitingZoneCount,
                state.waiting_zone_count,
            ),
            (
                CompileLimitDimension::RouteOccurrenceCount,
                state.route_occurrence_count,
            ),
            (
                CompileLimitDimension::GeometryPointCount,
                state.geometry_point_count,
            ),
        ] {
            if let Some(diagnostic) = limit_diagnostic(
                &self.limits,
                dimension,
                observed,
                Some(span.clone()),
                Some(stable_key.into()),
            ) {
                return Err(DiagnosticBundle::single(diagnostic));
            }
        }
        Ok(state)
    }

    fn commit_declaration_resources(&mut self, state: DeclarationResourceState) {
        self.declaration_count = state.declaration_count;
        self.typed_ast_record_count = state.typed_ast_record_count;
        self.reference_count = state.reference_count;
        self.relation_occurrence_count = state.relation_occurrence_count;
        self.identity_field_occurrence_count = state.identity_field_occurrence_count;
        self.symbol_count = state.symbol_count;
        self.string_item_count = state.string_item_count;
        self.string_bytes = state.string_bytes;
        self.controlled_string_bytes = state.controlled_string_bytes;
        self.controlled_structural_bytes = state.controlled_structural_bytes;
        self.source_record_byte_len = state.source_record_byte_len;
        self.maneuver_gate_count = state.maneuver_gate_count;
        self.waiting_zone_count = state.waiting_zone_count;
        self.route_occurrence_count = state.route_occurrence_count;
        self.geometry_point_count = state.geometry_point_count;
    }

    /// 声明显式模块导入；网络或文件系统发现不属于该操作。
    ///
    /// `namespace` 只建立图边，不要求目标模块已加入 `CompilationUnitBuilder`；目标
    /// 存在性和全图循环在构建编译单元时验证。
    ///
    /// # Errors
    ///
    /// 当命名空间非法、等于当前模块、已经导入，或加入后任一资源计数超限时失败。
    /// 失败不会修改导入集合、索引或累计计数。
    #[track_caller]
    pub fn add_import(&mut self, namespace: &str) -> Result<&mut Self, DiagnosticBundle> {
        let span = SourceSpan::at_caller(
            Arc::clone(&self.header.source_document_key),
            std::panic::Location::caller(),
        );
        let single_string_limit = self.limits.value(CompileLimitDimension::SingleStringBytes);
        if let Some(violation) = external_token_violation(namespace, single_string_limit) {
            return Err(DiagnosticBundle::single(
                Diagnostic::invalid_import_namespace(violation, span),
            ));
        }
        if namespace == self.header.authoring_namespace_id.as_ref() {
            return Err(DiagnosticBundle::single(Diagnostic::import_cycle(
                &[namespace],
                Box::new([span]),
            )));
        }
        if let Some(existing_index) = self.import_index.get(namespace).copied() {
            return Err(DiagnosticBundle::single(Diagnostic::duplicate_import(
                namespace,
                span,
                self.imports[existing_index].span.clone(),
            )));
        }

        let observed_imports = u64::try_from(self.imports.len())
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        let observed_typed_ast_records = self.typed_ast_record_count.saturating_add(1);
        if let Some(diagnostic) = limit_diagnostic(
            &self.limits,
            CompileLimitDimension::ImportEdgeCount,
            observed_imports,
            Some(span.clone()),
            Some(namespace.into()),
        ) {
            return Err(DiagnosticBundle::single(diagnostic));
        }
        let namespace_bytes = u64::try_from(namespace.len()).unwrap_or(u64::MAX);
        let observed_string_items = self.string_item_count.saturating_add(1);
        let observed_string_bytes = self.string_bytes.saturating_add(namespace_bytes);
        let observed_controlled_string_bytes =
            self.controlled_string_bytes.saturating_add(namespace_bytes);
        let observed_source_bytes = self
            .source_record_byte_len
            .checked_add(4 + 16)
            .and_then(|length| length.checked_add(namespace_bytes))
            .unwrap_or(u64::MAX);
        for (dimension, observed) in [
            (
                CompileLimitDimension::StringItemCount,
                observed_string_items,
            ),
            (
                CompileLimitDimension::TotalStringBytes,
                observed_string_bytes,
            ),
            (
                CompileLimitDimension::SourceBytesPerModule,
                observed_source_bytes,
            ),
            (
                CompileLimitDimension::TypedAstRecordCount,
                observed_typed_ast_records,
            ),
            (
                CompileLimitDimension::CompilerControlledLiveBytes,
                observed_controlled_string_bytes
                    .saturating_add(self.controlled_structural_bytes)
                    .saturating_add(observed_source_bytes),
            ),
        ] {
            if let Some(diagnostic) = limit_diagnostic(
                &self.limits,
                dimension,
                observed,
                Some(span.clone()),
                Some(namespace.into()),
            ) {
                return Err(DiagnosticBundle::single(diagnostic));
            }
        }

        let namespace: Arc<str> = namespace.into();
        self.imports.push(ImportRecord {
            namespace: Arc::clone(&namespace),
            span: span.into(),
        });
        self.import_index.insert(namespace, self.imports.len() - 1);
        self.string_item_count = observed_string_items;
        self.string_bytes = observed_string_bytes;
        self.controlled_string_bytes = observed_controlled_string_bytes;
        self.source_record_byte_len = observed_source_bytes;
        self.typed_ast_record_count = observed_typed_ast_records;

        Ok(self)
    }

    /// 声明车道图边、基础道路限速和无序显式下游连接。
    ///
    /// 目标允许后置声明、自环或跨显式导入模块；目标存在性在 HIR 阶段解析。传入的
    /// `successors` 会按 `(module namespace, declaration key)` 排序，调用顺序不进入
    /// 来源身份。
    ///
    /// # Errors
    ///
    /// 稳定键或引用 token 非法、引用未导入模块、键/连接重复、长度或限速违反数值
    /// 约束，或候选声明导致资源上限超限时失败。失败不会插入部分声明或改变计数。
    ///
    /// # Examples
    ///
    /// 空 `successors` 明确表示终止边：
    ///
    /// ```
    /// use laneflow_compiler::{
    ///     CompileLimits, DiagnosticBundle, LaneEdgeInput, SourceModuleHeader,
    ///     SourceModuleHeaderInput, SyntheticModuleBuilder,
    /// };
    ///
    /// let limits = CompileLimits::p100_initial_v1();
    /// let header = SourceModuleHeader::new(
    ///     SourceModuleHeaderInput {
    ///         authoring_namespace_id: "example",
    ///         source_document_key: "example/source",
    ///         generator_build_id: "example-generator-v1",
    ///         parameters_and_inputs_digest: [0; 32],
    ///         frontend_options_digest: [0; 32],
    ///         random_seed: None,
    ///         provenance: "rustdoc example",
    ///     },
    ///     &limits,
    /// )?;
    /// let mut module = SyntheticModuleBuilder::new(header, &limits)?;
    /// module.add_lane_edge(LaneEdgeInput {
    ///     lane_edge_key: "terminal",
    ///     length_meters: 12.0,
    ///     speed_limit_meters_per_second: 8.0,
    ///     successors: &[],
    /// })?;
    /// let module = module.finish()?;
    /// assert_eq!(module.descriptor().authoring_namespace_id(), "example");
    ///
    /// # Ok::<(), DiagnosticBundle>(())
    /// ```
    #[track_caller]
    pub fn add_lane_edge(
        &mut self,
        input: LaneEdgeInput<'_>,
    ) -> Result<&mut Self, DiagnosticBundle> {
        let span = SourceSpan::at_caller(
            Arc::clone(&self.header.source_document_key),
            std::panic::Location::caller(),
        );
        self.add_lane_edge_at(input, span)
    }

    pub(super) fn add_lane_edge_at(
        &mut self,
        input: LaneEdgeInput<'_>,
        span: SourceSpan,
    ) -> Result<&mut Self, DiagnosticBundle> {
        let single_string_limit = self.limits.value(CompileLimitDimension::SingleStringBytes);
        if let Some(violation) = external_token_violation(input.lane_edge_key, single_string_limit)
        {
            return Err(DiagnosticBundle::single(
                Diagnostic::invalid_declaration_key(EntityKind::LaneEdge, violation, span),
            ));
        }
        if let Some(existing_span) = self
            .declaration_index
            .get(&EntityKind::LaneEdge)
            .and_then(|index| index.get(input.lane_edge_key))
        {
            return Err(DiagnosticBundle::single(Diagnostic::duplicate_declaration(
                EntityKind::LaneEdge,
                input.lane_edge_key,
                span,
                existing_span.clone(),
            )));
        }
        let length = EdgeLength::try_new(input.length_meters).map_err(|violation| {
            DiagnosticBundle::single(Diagnostic::invalid_lane_edge_length(
                input.lane_edge_key,
                input.length_meters,
                violation,
                span.clone(),
            ))
        })?;
        let speed_limit =
            SpeedLimit::try_new(input.speed_limit_meters_per_second).map_err(|violation| {
                DiagnosticBundle::single(Diagnostic::invalid_lane_edge_speed_limit(
                    input.lane_edge_key,
                    input.speed_limit_meters_per_second,
                    violation,
                    span.clone(),
                ))
            })?;

        let successor_count = u64::try_from(input.successors.len()).unwrap_or(u64::MAX);
        let next_declaration_count = self.declaration_count.saturating_add(1);
        let next_reference_count = self.reference_count.saturating_add(successor_count);
        let next_relation_occurrence_count = self
            .relation_occurrence_count
            .saturating_add(successor_count);
        let next_identity_field_occurrence_count =
            self.identity_field_occurrence_count.saturating_add(2);
        let next_symbol_count = self.symbol_count.saturating_add(1);
        let typed_ast_delta = 3_u64.saturating_add(successor_count.saturating_mul(2));
        let next_typed_ast_record_count =
            self.typed_ast_record_count.saturating_add(typed_ast_delta);

        for (dimension, observed) in [
            (
                CompileLimitDimension::DeclarationCount,
                next_declaration_count,
            ),
            (CompileLimitDimension::ReferenceCount, next_reference_count),
            (
                CompileLimitDimension::RelationOccurrenceCount,
                next_relation_occurrence_count,
            ),
            (
                CompileLimitDimension::IdentityFieldOccurrenceCount,
                next_identity_field_occurrence_count,
            ),
            (CompileLimitDimension::SymbolCount, next_symbol_count),
            (
                CompileLimitDimension::TypedAstRecordCount,
                next_typed_ast_record_count,
            ),
        ] {
            if let Some(diagnostic) = limit_diagnostic(
                &self.limits,
                dimension,
                observed,
                Some(span.clone()),
                Some(input.lane_edge_key.into()),
            ) {
                return Err(DiagnosticBundle::single(diagnostic));
            }
        }

        let mut logical_string_item_delta = 2_u64;
        let mut logical_string_byte_delta = u64::try_from(self.header.authoring_namespace_id.len())
            .unwrap_or(u64::MAX)
            .saturating_add(u64::try_from(input.lane_edge_key.len()).unwrap_or(u64::MAX));
        let mut controlled_string_byte_delta =
            u64::try_from(input.lane_edge_key.len()).unwrap_or(u64::MAX);
        let mut declaration_source_bytes = lane_edge_declaration_base_len(input.lane_edge_key);
        for successor in input.successors {
            if let Some(namespace) = successor.module_namespace()
                && let Some(violation) = external_token_violation(namespace, single_string_limit)
            {
                return Err(DiagnosticBundle::single(
                    Diagnostic::invalid_reference_namespace(violation, span.clone()),
                ));
            }
            let namespace = self.reference_namespace(successor.module_namespace(), &span)?;
            if let Some(violation) =
                external_token_violation(successor.declaration_key(), single_string_limit)
            {
                return Err(DiagnosticBundle::single(Diagnostic::invalid_reference_key(
                    EntityKind::LaneEdge,
                    violation,
                    span.clone(),
                )));
            }
            logical_string_item_delta = logical_string_item_delta.saturating_add(1);
            let reference_spelling_bytes = namespace
                .len()
                .saturating_add(1)
                .saturating_add(successor.declaration_key().len());
            logical_string_byte_delta = logical_string_byte_delta
                .saturating_add(u64::try_from(reference_spelling_bytes).unwrap_or(u64::MAX));
            controlled_string_byte_delta = controlled_string_byte_delta.saturating_add(
                u64::try_from(successor.declaration_key().len()).unwrap_or(u64::MAX),
            );
            declaration_source_bytes = declaration_source_bytes.saturating_add(
                encoded_reference_len(namespace, successor.declaration_key()),
            );
        }

        let next_string_item_count = self
            .string_item_count
            .saturating_add(logical_string_item_delta);
        let next_string_bytes = self.string_bytes.saturating_add(logical_string_byte_delta);
        let next_controlled_string_bytes = self
            .controlled_string_bytes
            .saturating_add(controlled_string_byte_delta);
        let next_source_record_byte_len = self
            .source_record_byte_len
            .saturating_add(declaration_source_bytes);
        let structural_bytes = u64::try_from(std::mem::size_of::<LaneEdgeDeclaration>())
            .unwrap_or(u64::MAX)
            .saturating_add(
                successor_count.saturating_mul(
                    u64::try_from(std::mem::size_of::<OwnedEntityReference<LaneEdgeKind>>())
                        .unwrap_or(u64::MAX),
                ),
            );
        let next_controlled_structural_bytes = self
            .controlled_structural_bytes
            .saturating_add(structural_bytes);
        let next_controlled_live_bytes = next_controlled_string_bytes
            .saturating_add(next_source_record_byte_len)
            .saturating_add(next_controlled_structural_bytes);
        for (dimension, observed) in [
            (
                CompileLimitDimension::StringItemCount,
                next_string_item_count,
            ),
            (CompileLimitDimension::TotalStringBytes, next_string_bytes),
            (
                CompileLimitDimension::SourceBytesPerModule,
                next_source_record_byte_len,
            ),
            (
                CompileLimitDimension::CompilerControlledLiveBytes,
                next_controlled_live_bytes,
            ),
        ] {
            if let Some(diagnostic) = limit_diagnostic(
                &self.limits,
                dimension,
                observed,
                Some(span.clone()),
                Some(input.lane_edge_key.into()),
            ) {
                return Err(DiagnosticBundle::single(diagnostic));
            }
        }

        // 前面的验证只计算候选计数。到这里仍不修改构建器；先复制并规范化完整连接集，
        // 让重复检查也保持失败原子性。
        let mut successors = Vec::with_capacity(input.successors.len());
        for successor in input.successors {
            let namespace = self.reference_namespace_arc(successor.module_namespace(), &span)?;
            successors.push(OwnedEntityReference::new(
                namespace,
                successor.declaration_key().into(),
                span.clone(),
            ));
        }
        successors.sort_unstable_by(|left, right| {
            (&left.module_namespace, &left.declaration_key())
                .cmp(&(&right.module_namespace, &right.declaration_key()))
        });
        if let Some(duplicate) = successors.windows(2).find(|pair| {
            pair[0].module_namespace == pair[1].module_namespace
                && pair[0].declaration_key() == pair[1].declaration_key()
        }) {
            return Err(DiagnosticBundle::single(
                Diagnostic::duplicate_lane_edge_successor(
                    input.lane_edge_key,
                    &duplicate[1].module_namespace,
                    duplicate[1].declaration_key(),
                    span.clone(),
                ),
            ));
        }

        let stable_key: Arc<str> = input.lane_edge_key.into();
        let declaration = TypedAstDeclaration::LaneEdge(LaneEdgeDeclaration {
            header: DeclarationHeader::module_scoped(
                EntityKind::LaneEdge,
                Arc::clone(&stable_key),
                span.clone().into(),
            ),
            geometry_authority: LaneEdgeGeometryAuthority::DirectLength(length),
            speed_limit,
            successors: successors.into_boxed_slice(),
        });
        // 所有可能失败的检查已经完成；从索引开始一次性提交声明及其累计计数。
        self.declaration_index
            .entry(EntityKind::LaneEdge)
            .or_default()
            .insert(Arc::clone(&stable_key), span.into());
        self.declarations.push(declaration);
        self.declaration_count = next_declaration_count;
        self.reference_count = next_reference_count;
        self.relation_occurrence_count = next_relation_occurrence_count;
        self.identity_field_occurrence_count = next_identity_field_occurrence_count;
        self.symbol_count = next_symbol_count;
        self.typed_ast_record_count = next_typed_ast_record_count;
        self.string_item_count = next_string_item_count;
        self.string_bytes = next_string_bytes;
        self.controlled_string_bytes = next_controlled_string_bytes;
        self.controlled_structural_bytes = next_controlled_structural_bytes;
        self.source_record_byte_len = next_source_record_byte_len;
        Ok(self)
    }

    /// 声明一个路口；通行流向成员由 `MovementInput::junction` 反向形成。
    ///
    /// # Errors
    ///
    /// 稳定键非法、声明重复或资源上限超限时失败。非空成员约束在完整模块图建立后
    /// 验证；失败不会改变构建器。
    #[track_caller]
    pub fn add_junction(
        &mut self,
        input: JunctionInput<'_>,
    ) -> Result<&mut Self, DiagnosticBundle> {
        let span = SourceSpan::at_caller(
            Arc::clone(&self.header.source_document_key),
            std::panic::Location::caller(),
        );
        self.validate_declaration_key(EntityKind::Junction, input.junction_key, &span)?;
        let namespace_bytes =
            u64::try_from(self.header.authoring_namespace_id.len()).unwrap_or(u64::MAX);
        let key_bytes = u64::try_from(input.junction_key.len()).unwrap_or(u64::MAX);
        let state = self.check_declaration_resources(
            DeclarationResourceDelta {
                declarations: 1,
                typed_ast_records: 3,
                identity_fields: 2,
                symbols: 1,
                string_items: 2,
                string_bytes: namespace_bytes.saturating_add(key_bytes),
                controlled_string_bytes: key_bytes,
                controlled_structural_bytes: size_bytes::<JunctionDeclaration>(1),
                source_bytes: declaration_header_len(input.junction_key),
                ..DeclarationResourceDelta::default()
            },
            input.junction_key,
            &span,
        )?;

        let stable_key: Arc<str> = input.junction_key.into();
        let declaration = TypedAstDeclaration::Junction(JunctionDeclaration {
            header: DeclarationHeader::module_scoped(
                EntityKind::Junction,
                Arc::clone(&stable_key),
                span.clone().into(),
            ),
            approach_edges: Box::default(),
            internal_edges: Box::default(),
        });
        self.declaration_index
            .entry(EntityKind::Junction)
            .or_default()
            .insert(Arc::clone(&stable_key), span.into());
        self.declarations.push(declaration);
        self.commit_declaration_resources(state);
        Ok(self)
    }

    /// 声明一个通行流向及其唯一路口父项和两个稳定有向引道键。
    ///
    /// # Errors
    ///
    /// 稳定键、有向引道键或父项引用非法，跨模块引用未显式导入，声明重复，或资源
    /// 上限超限时失败。父项存在性与非空路径成员约束在 HIR 阶段验证。
    #[track_caller]
    pub fn add_movement(
        &mut self,
        input: MovementInput<'_>,
    ) -> Result<&mut Self, DiagnosticBundle> {
        let span = SourceSpan::at_caller(
            Arc::clone(&self.header.source_document_key),
            std::panic::Location::caller(),
        );
        self.validate_declaration_key(EntityKind::Movement, input.movement_key, &span)?;
        self.validate_identity_ascii_field(
            EntityKind::Movement,
            input.movement_key,
            FieldTag::DirectedEntryApproachKey,
            input.directed_entry_approach_key,
            &span,
        )?;
        self.validate_identity_ascii_field(
            EntityKind::Movement,
            input.movement_key,
            FieldTag::DirectedExitApproachKey,
            input.directed_exit_approach_key,
            &span,
        )?;
        let junction = self.own_reference(EntityKind::Junction, input.junction, &span)?;
        let namespace_bytes =
            u64::try_from(self.header.authoring_namespace_id.len()).unwrap_or(u64::MAX);
        let key_bytes = u64::try_from(input.movement_key.len()).unwrap_or(u64::MAX);
        let entry_bytes =
            u64::try_from(input.directed_entry_approach_key.len()).unwrap_or(u64::MAX);
        let exit_bytes = u64::try_from(input.directed_exit_approach_key.len()).unwrap_or(u64::MAX);
        let state = self.check_declaration_resources(
            DeclarationResourceDelta {
                declarations: 1,
                typed_ast_records: 7,
                references: 1,
                relations: 1,
                identity_fields: 5,
                symbols: 1,
                string_items: 5,
                string_bytes: namespace_bytes
                    .saturating_add(key_bytes)
                    .saturating_add(reference_spelling_bytes(&junction))
                    .saturating_add(entry_bytes)
                    .saturating_add(exit_bytes),
                controlled_string_bytes: key_bytes
                    .saturating_add(
                        u64::try_from(junction.declaration_key().len()).unwrap_or(u64::MAX),
                    )
                    .saturating_add(entry_bytes)
                    .saturating_add(exit_bytes),
                controlled_structural_bytes: size_bytes::<MovementDeclaration>(1)
                    .saturating_add(size_bytes::<OwnedEntityReference<JunctionKind>>(1)),
                source_bytes: movement_declaration_len(
                    input.movement_key,
                    &junction,
                    input.directed_entry_approach_key,
                    input.directed_exit_approach_key,
                ),
                ..DeclarationResourceDelta::default()
            },
            input.movement_key,
            &span,
        )?;

        let stable_key: Arc<str> = input.movement_key.into();
        let declaration = TypedAstDeclaration::Movement(MovementDeclaration {
            header: DeclarationHeader::module_scoped(
                EntityKind::Movement,
                Arc::clone(&stable_key),
                span.clone().into(),
            ),
            junction,
            directed_entry_approach_key: input.directed_entry_approach_key.into(),
            directed_exit_approach_key: input.directed_exit_approach_key.into(),
        });
        self.declaration_index
            .entry(EntityKind::Movement)
            .or_default()
            .insert(Arc::clone(&stable_key), span.into());
        self.declarations.push(declaration);
        self.commit_declaration_resources(state);
        Ok(self)
    }

    /// 声明一条机动路径及其完整入口、内部、出口边序列。
    ///
    /// # Errors
    ///
    /// 稳定键或任一引用非法、跨模块引用未显式导入、声明重复，或资源上限超限时
    /// 失败。目标存在性、路径连通性、遍历序列唯一性和内部边角色排他性在 HIR 阶段
    /// 验证；内部边数组允许为空。
    #[track_caller]
    pub fn add_maneuver_path(
        &mut self,
        input: ManeuverPathInput<'_>,
    ) -> Result<&mut Self, DiagnosticBundle> {
        let span = SourceSpan::at_caller(
            Arc::clone(&self.header.source_document_key),
            std::panic::Location::caller(),
        );
        self.validate_declaration_key(EntityKind::ManeuverPath, input.maneuver_path_key, &span)?;
        let movement = self.own_reference(EntityKind::Movement, input.movement, &span)?;
        let entry_edge = self.own_reference(EntityKind::LaneEdge, input.entry_edge, &span)?;
        let mut internal_edges = Vec::with_capacity(input.internal_edges.len());
        for reference in input.internal_edges {
            internal_edges.push(self.own_reference(EntityKind::LaneEdge, *reference, &span)?);
        }
        let exit_edge = self.own_reference(EntityKind::LaneEdge, input.exit_edge, &span)?;
        let internal_count = u64::try_from(internal_edges.len()).unwrap_or(u64::MAX);
        let reference_count = internal_count.saturating_add(3);
        let namespace_bytes =
            u64::try_from(self.header.authoring_namespace_id.len()).unwrap_or(u64::MAX);
        let key_bytes = u64::try_from(input.maneuver_path_key.len()).unwrap_or(u64::MAX);
        let references_logical_bytes = [movement.declaration_key(), entry_edge.declaration_key()]
            .into_iter()
            .chain(
                internal_edges
                    .iter()
                    .map(OwnedEntityReference::declaration_key),
            )
            .chain([exit_edge.declaration_key()])
            .fold(0_u64, |total, key| {
                total.saturating_add(u64::try_from(key.len()).unwrap_or(u64::MAX))
            });
        let references_spelling_bytes = reference_spelling_bytes(&movement)
            .saturating_add(reference_spelling_bytes(&entry_edge))
            .saturating_add(internal_edges.iter().fold(0_u64, |total, edge| {
                total.saturating_add(reference_spelling_bytes(edge))
            }))
            .saturating_add(reference_spelling_bytes(&exit_edge));
        let state = self.check_declaration_resources(
            DeclarationResourceDelta {
                declarations: 1,
                typed_ast_records: 3_u64.saturating_add(reference_count.saturating_mul(2)),
                references: reference_count,
                relations: 1_u64.saturating_add(internal_count.saturating_add(2)),
                identity_fields: 5,
                symbols: 1,
                string_items: 2_u64.saturating_add(reference_count),
                string_bytes: namespace_bytes
                    .saturating_add(key_bytes)
                    .saturating_add(references_spelling_bytes),
                controlled_string_bytes: key_bytes.saturating_add(references_logical_bytes),
                controlled_structural_bytes: size_bytes::<ManeuverPathDeclaration>(1)
                    .saturating_add(size_bytes::<OwnedEntityReference<MovementKind>>(1))
                    .saturating_add(size_bytes::<OwnedEntityReference<LaneEdgeKind>>(
                        internal_count.saturating_add(2),
                    )),
                source_bytes: maneuver_path_declaration_len(
                    input.maneuver_path_key,
                    &movement,
                    &entry_edge,
                    &internal_edges,
                    &exit_edge,
                ),
                ..DeclarationResourceDelta::default()
            },
            input.maneuver_path_key,
            &span,
        )?;

        let stable_key: Arc<str> = input.maneuver_path_key.into();
        let declaration = TypedAstDeclaration::ManeuverPath(ManeuverPathDeclaration {
            header: DeclarationHeader::module_scoped(
                EntityKind::ManeuverPath,
                Arc::clone(&stable_key),
                span.clone().into(),
            ),
            movement,
            entry_edge,
            internal_edges: internal_edges.into_boxed_slice(),
            exit_edge,
        });
        self.declaration_index
            .entry(EntityKind::ManeuverPath)
            .or_default()
            .insert(Arc::clone(&stable_key), span.into());
        self.declarations.push(declaration);
        self.commit_declaration_resources(state);
        Ok(self)
    }

    /// 声明一条位于车道图边末端的停止线。
    ///
    /// # Errors
    ///
    /// 稳定键或边引用非法、跨模块引用未显式导入、声明重复，或资源上限超限时失败。
    /// 目标存在性、停止线与机动门转换起始边的一致性，以及停止线必须被使用的闭包在
    /// HIR 阶段验证。
    #[track_caller]
    pub fn add_stop_line(
        &mut self,
        input: StopLineInput<'_>,
    ) -> Result<&mut Self, DiagnosticBundle> {
        let span = SourceSpan::at_caller(
            Arc::clone(&self.header.source_document_key),
            std::panic::Location::caller(),
        );
        self.validate_declaration_key(EntityKind::StopLine, input.stop_line_key, &span)?;
        let lane_edge = self.own_reference(EntityKind::LaneEdge, input.lane_edge, &span)?;
        let namespace_bytes =
            u64::try_from(self.header.authoring_namespace_id.len()).unwrap_or(u64::MAX);
        let key_bytes = u64::try_from(input.stop_line_key.len()).unwrap_or(u64::MAX);
        let state = self.check_declaration_resources(
            DeclarationResourceDelta {
                declarations: 1,
                typed_ast_records: 5,
                references: 1,
                relations: 1,
                identity_fields: 2,
                symbols: 1,
                string_items: 3,
                string_bytes: namespace_bytes
                    .saturating_add(key_bytes)
                    .saturating_add(reference_spelling_bytes(&lane_edge)),
                controlled_string_bytes: key_bytes.saturating_add(
                    u64::try_from(lane_edge.declaration_key().len()).unwrap_or(u64::MAX),
                ),
                controlled_structural_bytes: size_bytes::<StopLineDeclaration>(1)
                    .saturating_add(size_bytes::<OwnedEntityReference<LaneEdgeKind>>(1)),
                source_bytes: stop_line_declaration_len(input.stop_line_key, &lane_edge),
                ..DeclarationResourceDelta::default()
            },
            input.stop_line_key,
            &span,
        )?;

        let stable_key: Arc<str> = input.stop_line_key.into();
        self.declaration_index
            .entry(EntityKind::StopLine)
            .or_default()
            .insert(Arc::clone(&stable_key), span.clone().into());
        self.declarations
            .push(TypedAstDeclaration::StopLine(StopLineDeclaration {
                header: DeclarationHeader::module_scoped(
                    EntityKind::StopLine,
                    stable_key,
                    span.clone().into(),
                ),
                lane_edge,
            }));
        self.commit_declaration_resources(state);
        Ok(self)
    }

    /// 声明一个由固定时制控制器唯一拥有的信号组。
    ///
    /// # Errors
    ///
    /// 稳定键非法、声明重复或资源上限超限时失败。控制器所有权和至少一个
    /// `ManeuverGate` 使用关系在 HIR 阶段闭合。
    #[track_caller]
    pub fn add_signal_group(
        &mut self,
        input: SignalGroupInput<'_>,
    ) -> Result<&mut Self, DiagnosticBundle> {
        let span = SourceSpan::at_caller(
            Arc::clone(&self.header.source_document_key),
            std::panic::Location::caller(),
        );
        self.validate_declaration_key(EntityKind::SignalGroup, input.signal_group_key, &span)?;
        let namespace_bytes =
            u64::try_from(self.header.authoring_namespace_id.len()).unwrap_or(u64::MAX);
        let key_bytes = u64::try_from(input.signal_group_key.len()).unwrap_or(u64::MAX);
        let state = self.check_declaration_resources(
            DeclarationResourceDelta {
                declarations: 1,
                typed_ast_records: 3,
                identity_fields: 2,
                symbols: 1,
                string_items: 2,
                string_bytes: namespace_bytes.saturating_add(key_bytes),
                controlled_string_bytes: key_bytes,
                controlled_structural_bytes: size_bytes::<SignalGroupDeclaration>(1),
                source_bytes: declaration_header_len(input.signal_group_key),
                ..DeclarationResourceDelta::default()
            },
            input.signal_group_key,
            &span,
        )?;

        let stable_key: Arc<str> = input.signal_group_key.into();
        self.declaration_index
            .entry(EntityKind::SignalGroup)
            .or_default()
            .insert(Arc::clone(&stable_key), span.clone().into());
        self.declarations
            .push(TypedAstDeclaration::SignalGroup(SignalGroupDeclaration {
                header: DeclarationHeader::module_scoped(
                    EntityKind::SignalGroup,
                    stable_key,
                    span.clone().into(),
                ),
            }));
        self.commit_declaration_resources(state);
        Ok(self)
    }

    /// 声明一个不可变固定时制（immutable fixed-time）信号控制器及其有序相位程序。
    ///
    /// 输入顺序只对 `phases` 具有程序语义；`signal_groups` 与每个相位的 `states`
    /// 作为完备集合在 HIR 中规范化。相位键只在本控制器内唯一。
    ///
    /// # Errors
    ///
    /// 控制器/相位稳定键或信号组引用非法，跨模块引用未显式导入，或资源上限超限时
    /// 失败。非空、唯一所有权、状态完备性、时间范围、cycle 与 offset 约束在 HIR
    /// 阶段统一验证。
    #[track_caller]
    pub fn add_signal_controller(
        &mut self,
        input: SignalControllerInput<'_>,
    ) -> Result<&mut Self, DiagnosticBundle> {
        let span = SourceSpan::at_caller(
            Arc::clone(&self.header.source_document_key),
            std::panic::Location::caller(),
        );
        self.validate_declaration_key(
            EntityKind::SignalController,
            input.signal_controller_key,
            &span,
        )?;
        for phase in input.phases {
            self.validate_identity_ascii_field(
                EntityKind::SignalPhase,
                phase.signal_phase_key,
                FieldTag::PhaseKey,
                phase.signal_phase_key,
                &span,
            )?;
        }
        for group in input.signal_groups {
            self.validate_reference(EntityKind::SignalGroup, *group, &span)?;
        }
        for phase in input.phases {
            for state in phase.states {
                self.validate_reference(EntityKind::SignalGroup, state.signal_group, &span)?;
            }
        }

        let phase_count = u64::try_from(input.phases.len()).unwrap_or(u64::MAX);
        let group_count = u64::try_from(input.signal_groups.len()).unwrap_or(u64::MAX);
        let state_count = input.phases.iter().fold(0_u64, |total, phase| {
            total.saturating_add(u64::try_from(phase.states.len()).unwrap_or(u64::MAX))
        });
        let reference_count = group_count.saturating_add(state_count);
        let namespace_bytes =
            u64::try_from(self.header.authoring_namespace_id.len()).unwrap_or(u64::MAX);
        let key_bytes = u64::try_from(input.signal_controller_key.len()).unwrap_or(u64::MAX);
        let mut logical_string_bytes = namespace_bytes
            .saturating_mul(1_u64.saturating_add(phase_count))
            .saturating_add(key_bytes);
        let mut controlled_string_bytes = key_bytes;
        for group in input.signal_groups {
            let namespace = group
                .module_namespace()
                .unwrap_or(&self.header.authoring_namespace_id);
            logical_string_bytes = logical_string_bytes.saturating_add(
                reference_spelling_parts_bytes(namespace, group.declaration_key()),
            );
            controlled_string_bytes = controlled_string_bytes
                .saturating_add(u64::try_from(group.declaration_key().len()).unwrap_or(u64::MAX));
        }
        for phase in input.phases {
            let phase_key_bytes = u64::try_from(phase.signal_phase_key.len()).unwrap_or(u64::MAX);
            logical_string_bytes = logical_string_bytes.saturating_add(phase_key_bytes);
            controlled_string_bytes = controlled_string_bytes.saturating_add(phase_key_bytes);
            for state in phase.states {
                let namespace = state
                    .signal_group
                    .module_namespace()
                    .unwrap_or(&self.header.authoring_namespace_id);
                logical_string_bytes = logical_string_bytes.saturating_add(
                    reference_spelling_parts_bytes(namespace, state.signal_group.declaration_key()),
                );
                controlled_string_bytes = controlled_string_bytes.saturating_add(
                    u64::try_from(state.signal_group.declaration_key().len()).unwrap_or(u64::MAX),
                );
            }
        }
        let structural_bytes = size_bytes::<SignalControllerDeclaration>(1)
            .saturating_add(size_bytes::<OwnedEntityReference<SignalGroupKind>>(
                group_count,
            ))
            .saturating_add(size_bytes::<SignalPhaseDeclaration>(phase_count))
            .saturating_add(size_bytes::<SignalGroupStateDeclaration>(state_count))
            .saturating_add(size_bytes::<OwnedEntityReference<SignalGroupKind>>(
                state_count,
            ));
        let source_bytes = signal_controller_input_len(
            input.signal_controller_key,
            input.offset_ms,
            input.signal_groups,
            input.phases,
            &self.header.authoring_namespace_id,
        );
        let state = self.check_declaration_resources(
            DeclarationResourceDelta {
                declarations: 1_u64.saturating_add(phase_count),
                typed_ast_records: 3_u64
                    .saturating_add(phase_count.saturating_mul(3))
                    .saturating_add(group_count.saturating_mul(2))
                    .saturating_add(state_count.saturating_mul(3)),
                references: reference_count,
                relations: group_count
                    .saturating_add(phase_count)
                    .saturating_add(state_count),
                identity_fields: 2_u64.saturating_add(phase_count.saturating_mul(3)),
                symbols: 1_u64.saturating_add(phase_count),
                string_items: 2_u64
                    .saturating_add(phase_count.saturating_mul(2))
                    .saturating_add(reference_count),
                string_bytes: logical_string_bytes,
                controlled_string_bytes,
                controlled_structural_bytes: structural_bytes,
                source_bytes,
                ..DeclarationResourceDelta::default()
            },
            input.signal_controller_key,
            &span,
        )?;

        let mut signal_groups = Vec::with_capacity(input.signal_groups.len());
        for group in input.signal_groups {
            signal_groups.push(self.own_reference(EntityKind::SignalGroup, *group, &span)?);
        }
        let mut phases = Vec::with_capacity(input.phases.len());
        for phase in input.phases {
            let mut states = Vec::with_capacity(phase.states.len());
            for phase_state in phase.states {
                states.push(SignalGroupStateDeclaration {
                    signal_group: self.own_reference(
                        EntityKind::SignalGroup,
                        phase_state.signal_group,
                        &span,
                    )?,
                    aspect: phase_state.aspect,
                });
            }
            phases.push(SignalPhaseDeclaration {
                header: DeclarationHeader::module_scoped(
                    EntityKind::SignalPhase,
                    phase.signal_phase_key.into(),
                    span.clone().into(),
                ),
                controller_relation_span: span.clone().into(),
                duration_ms: phase.duration_ms,
                states: states.into_boxed_slice(),
            });
        }

        let stable_key: Arc<str> = input.signal_controller_key.into();
        self.declaration_index
            .entry(EntityKind::SignalController)
            .or_default()
            .insert(Arc::clone(&stable_key), span.clone().into());
        self.declarations
            .push(TypedAstDeclaration::SignalController(
                SignalControllerDeclaration {
                    header: DeclarationHeader::module_scoped(
                        EntityKind::SignalController,
                        stable_key,
                        span.clone().into(),
                    ),
                    offset_ms: input.offset_ms,
                    signal_groups: signal_groups.into_boxed_slice(),
                    phases: phases.into_boxed_slice(),
                },
            ));
        self.commit_declaration_resources(state);
        Ok(self)
    }

    /// 声明一个可选组织停车位的不可变停车区域。
    ///
    /// # Errors
    ///
    /// 稳定键非法、声明重复或资源上限超限时失败。区域至少拥有一个停车位的闭包在
    /// HIR 阶段验证。
    #[track_caller]
    pub fn add_parking_area(
        &mut self,
        input: ParkingAreaInput<'_>,
    ) -> Result<&mut Self, DiagnosticBundle> {
        let span = SourceSpan::at_caller(
            Arc::clone(&self.header.source_document_key),
            std::panic::Location::caller(),
        );
        self.validate_declaration_key(EntityKind::ParkingArea, input.parking_area_key, &span)?;
        let namespace_bytes =
            u64::try_from(self.header.authoring_namespace_id.len()).unwrap_or(u64::MAX);
        let key_bytes = u64::try_from(input.parking_area_key.len()).unwrap_or(u64::MAX);
        let state = self.check_declaration_resources(
            DeclarationResourceDelta {
                declarations: 1,
                typed_ast_records: 3,
                identity_fields: 2,
                symbols: 1,
                string_items: 2,
                string_bytes: namespace_bytes.saturating_add(key_bytes),
                controlled_string_bytes: key_bytes,
                controlled_structural_bytes: size_bytes::<ParkingAreaDeclaration>(1),
                source_bytes: declaration_header_len(input.parking_area_key),
                ..DeclarationResourceDelta::default()
            },
            input.parking_area_key,
            &span,
        )?;

        let stable_key: Arc<str> = input.parking_area_key.into();
        self.declaration_index
            .entry(EntityKind::ParkingArea)
            .or_default()
            .insert(Arc::clone(&stable_key), span.clone().into());
        self.declarations
            .push(TypedAstDeclaration::ParkingArea(ParkingAreaDeclaration {
                header: DeclarationHeader::module_scoped(
                    EntityKind::ParkingArea,
                    stable_key,
                    span.clone().into(),
                ),
            }));
        self.commit_declaration_resources(state);
        Ok(self)
    }

    /// 声明一个带可选区域归属、入口/出口锚点和矩形几何的不可变停车位。
    ///
    /// # Errors
    ///
    /// 稳定键或引用非法、跨模块引用未显式导入、声明重复，或资源上限超限时失败。
    /// 区域存在性、锚点边界、几何范围和区域非孤立约束在 HIR 阶段统一验证。
    #[track_caller]
    pub fn add_parking_space(
        &mut self,
        input: ParkingSpaceInput<'_>,
    ) -> Result<&mut Self, DiagnosticBundle> {
        let span = SourceSpan::at_caller(
            Arc::clone(&self.header.source_document_key),
            std::panic::Location::caller(),
        );
        self.validate_declaration_key(EntityKind::ParkingSpace, input.parking_space_key, &span)?;
        if let Some(area) = input.parking_area {
            self.validate_reference(EntityKind::ParkingArea, area, &span)?;
        }
        self.validate_reference(EntityKind::LaneEdge, input.entry.lane_edge, &span)?;
        self.validate_reference(EntityKind::LaneEdge, input.exit.lane_edge, &span)?;
        let (entry_progress_mm, exit_progress_mm, geometry) = admit_parking_space_scalars(
            input,
            &span,
            self.limits.value(CompileLimitDimension::DiagnosticCount),
        )?;

        let reference_count = 2_u64.saturating_add(u64::from(input.parking_area.is_some()));
        let namespace_bytes =
            u64::try_from(self.header.authoring_namespace_id.len()).unwrap_or(u64::MAX);
        let key_bytes = u64::try_from(input.parking_space_key.len()).unwrap_or(u64::MAX);
        let mut logical_string_bytes = namespace_bytes.saturating_add(key_bytes);
        let mut controlled_string_bytes = key_bytes;
        if let Some(area) = input.parking_area {
            logical_string_bytes =
                logical_string_bytes.saturating_add(reference_spelling_parts_bytes(
                    area.module_namespace()
                        .unwrap_or(&self.header.authoring_namespace_id),
                    area.declaration_key(),
                ));
            controlled_string_bytes = controlled_string_bytes
                .saturating_add(u64::try_from(area.declaration_key().len()).unwrap_or(u64::MAX));
        }
        for edge in [input.entry.lane_edge, input.exit.lane_edge] {
            logical_string_bytes =
                logical_string_bytes.saturating_add(reference_spelling_parts_bytes(
                    edge.module_namespace()
                        .unwrap_or(&self.header.authoring_namespace_id),
                    edge.declaration_key(),
                ));
            controlled_string_bytes = controlled_string_bytes
                .saturating_add(u64::try_from(edge.declaration_key().len()).unwrap_or(u64::MAX));
        }
        let source_bytes = parking_space_input_len(&input, &self.header.authoring_namespace_id);
        let state = self.check_declaration_resources(
            DeclarationResourceDelta {
                declarations: 1,
                typed_ast_records: 7_u64.saturating_add(reference_count.saturating_mul(2)),
                references: reference_count,
                relations: reference_count,
                identity_fields: 2,
                symbols: 1,
                string_items: 2_u64.saturating_add(reference_count),
                string_bytes: logical_string_bytes,
                controlled_string_bytes,
                controlled_structural_bytes: size_bytes::<ParkingSpaceDeclaration>(1),
                source_bytes,
                ..DeclarationResourceDelta::default()
            },
            input.parking_space_key,
            &span,
        )?;

        let parking_area = match input.parking_area {
            Some(area) => Some(self.own_reference(EntityKind::ParkingArea, area, &span)?),
            None => None,
        };
        let entry = ParkingLaneAnchorDeclaration {
            lane_edge: self.own_reference(EntityKind::LaneEdge, input.entry.lane_edge, &span)?,
            progress_mm: entry_progress_mm,
        };
        let exit = ParkingLaneAnchorDeclaration {
            lane_edge: self.own_reference(EntityKind::LaneEdge, input.exit.lane_edge, &span)?,
            progress_mm: exit_progress_mm,
        };
        let stable_key: Arc<str> = input.parking_space_key.into();
        self.declaration_index
            .entry(EntityKind::ParkingSpace)
            .or_default()
            .insert(Arc::clone(&stable_key), span.clone().into());
        self.declarations
            .push(TypedAstDeclaration::ParkingSpace(ParkingSpaceDeclaration {
                header: DeclarationHeader::module_scoped(
                    EntityKind::ParkingSpace,
                    stable_key,
                    span.clone().into(),
                ),
                parking_area,
                entry,
                exit,
                geometry,
            }));
        self.commit_declaration_resources(state);
        Ok(self)
    }

    /// 声明一个只用于静态准入分类法的参与者类别。
    ///
    /// # Errors
    ///
    /// 稳定键或父类引用非法、跨模块引用未显式导入、声明重复，或资源上限超限时
    /// 失败。父类存在性和继承环在完整模块图建立后的 HIR 阶段验证。
    #[track_caller]
    pub fn add_participant_class(
        &mut self,
        input: ParticipantClassInput<'_>,
    ) -> Result<&mut Self, DiagnosticBundle> {
        let span = SourceSpan::at_caller(
            Arc::clone(&self.header.source_document_key),
            std::panic::Location::caller(),
        );
        self.validate_declaration_key(
            EntityKind::ParticipantClass,
            input.participant_class_key,
            &span,
        )?;
        let extends = input
            .extends
            .map(|parent| self.own_reference(EntityKind::ParticipantClass, parent, &span))
            .transpose()?;
        let namespace_bytes =
            u64::try_from(self.header.authoring_namespace_id.len()).unwrap_or(u64::MAX);
        let key_bytes = u64::try_from(input.participant_class_key.len()).unwrap_or(u64::MAX);
        let reference_count = u64::from(extends.is_some());
        let reference_bytes = extends.as_ref().map_or(0, reference_spelling_bytes);
        let controlled_reference_bytes = extends.as_ref().map_or(0, |reference| {
            u64::try_from(reference.declaration_key().len()).unwrap_or(u64::MAX)
        });
        let state = self.check_declaration_resources(
            DeclarationResourceDelta {
                declarations: 1,
                typed_ast_records: 3_u64.saturating_add(reference_count.saturating_mul(2)),
                references: reference_count,
                relations: reference_count,
                identity_fields: 2,
                symbols: 1,
                string_items: 2_u64.saturating_add(reference_count),
                string_bytes: namespace_bytes
                    .saturating_add(key_bytes)
                    .saturating_add(reference_bytes),
                controlled_string_bytes: key_bytes.saturating_add(controlled_reference_bytes),
                controlled_structural_bytes: size_bytes::<ParticipantClassDeclaration>(1)
                    .saturating_add(size_bytes::<OwnedEntityReference<ParticipantClassKind>>(
                        reference_count,
                    )),
                source_bytes: participant_class_declaration_len(
                    input.participant_class_key,
                    extends.as_ref(),
                ),
                ..DeclarationResourceDelta::default()
            },
            input.participant_class_key,
            &span,
        )?;

        let stable_key: Arc<str> = input.participant_class_key.into();
        self.declaration_index
            .entry(EntityKind::ParticipantClass)
            .or_default()
            .insert(Arc::clone(&stable_key), span.clone().into());
        self.declarations
            .push(TypedAstDeclaration::ParticipantClass(
                ParticipantClassDeclaration {
                    header: DeclarationHeader::module_scoped(
                        EntityKind::ParticipantClass,
                        stable_key,
                        span.clone().into(),
                    ),
                    extends,
                },
            ));
        self.commit_declaration_resources(state);
        Ok(self)
    }

    /// 声明一个当前道路机动车执行域使用的 IIDM 车辆配置。
    ///
    /// # Errors
    ///
    /// 稳定键或参与者类别引用非法、任一 IIDM 数值违反 current Core 约束、减速度
    /// 顺序非法、跨模块引用未显式导入、声明重复，或资源上限超限时失败。类别目标
    /// 存在性在完整模块图建立后的 HIR 阶段验证。
    #[track_caller]
    pub fn add_vehicle_profile(
        &mut self,
        input: VehicleProfileInput<'_>,
    ) -> Result<&mut Self, DiagnosticBundle> {
        let span = SourceSpan::at_caller(
            Arc::clone(&self.header.source_document_key),
            std::panic::Location::caller(),
        );
        self.validate_declaration_key(
            EntityKind::VehicleProfile,
            input.vehicle_profile_key,
            &span,
        )?;
        self.validate_reference(EntityKind::ParticipantClass, input.participant_class, &span)?;
        let iidm = admit_vehicle_profile_scalars(input, &span)?;

        let participant_class =
            self.own_reference(EntityKind::ParticipantClass, input.participant_class, &span)?;
        let namespace_bytes =
            u64::try_from(self.header.authoring_namespace_id.len()).unwrap_or(u64::MAX);
        let key_bytes = u64::try_from(input.vehicle_profile_key.len()).unwrap_or(u64::MAX);
        let reference_bytes = reference_spelling_bytes(&participant_class);
        let controlled_reference_bytes =
            u64::try_from(participant_class.declaration_key().len()).unwrap_or(u64::MAX);
        let state = self.check_declaration_resources(
            DeclarationResourceDelta {
                declarations: 1,
                typed_ast_records: 5,
                references: 1,
                relations: 1,
                identity_fields: 2,
                symbols: 1,
                string_items: 3,
                string_bytes: namespace_bytes
                    .saturating_add(key_bytes)
                    .saturating_add(reference_bytes),
                controlled_string_bytes: key_bytes.saturating_add(controlled_reference_bytes),
                controlled_structural_bytes: size_bytes::<VehicleProfileDeclaration>(1)
                    .saturating_add(size_bytes::<OwnedEntityReference<ParticipantClassKind>>(1)),
                source_bytes: vehicle_profile_declaration_len(
                    input.vehicle_profile_key,
                    &participant_class,
                ),
                ..DeclarationResourceDelta::default()
            },
            input.vehicle_profile_key,
            &span,
        )?;

        let stable_key: Arc<str> = input.vehicle_profile_key.into();
        self.declaration_index
            .entry(EntityKind::VehicleProfile)
            .or_default()
            .insert(Arc::clone(&stable_key), span.clone().into());
        self.declarations.push(TypedAstDeclaration::VehicleProfile(
            VehicleProfileDeclaration {
                header: DeclarationHeader::module_scoped(
                    EntityKind::VehicleProfile,
                    stable_key,
                    span.clone().into(),
                ),
                participant_class,
                iidm,
            },
        ));
        self.commit_declaration_resources(state);
        Ok(self)
    }

    /// 声明一个 SpatialPackage v0.1 使用的规范坐标框架及其车道边中心线。
    ///
    /// 坐标单位、轴向、手性和范围沿用全局空间契约，调用方不得借此编码 CRS、宿主
    /// 放置或可变原点。几何集合顺序不参与语义；每条中心线内部的点顺序按行驶方向
    /// 保留。
    ///
    /// # Errors
    ///
    /// 稳定键或边引用非法、同一 frame 重复绑定边、点数不足、坐标非法、声明重复，
    /// 或资源上限超限时失败。跨 frame 重复、全图覆盖、长度与连接连续性在 HIR 阶段
    /// 统一验证。
    #[track_caller]
    pub fn add_canonical_frame(
        &mut self,
        input: CanonicalFrameInput<'_>,
    ) -> Result<&mut Self, DiagnosticBundle> {
        let span = SourceSpan::at_caller(
            Arc::clone(&self.header.source_document_key),
            std::panic::Location::caller(),
        );
        self.validate_declaration_key(
            EntityKind::CanonicalFrame,
            input.canonical_frame_key,
            &span,
        )?;

        for geometry in input.lane_edge_geometries {
            self.validate_reference(EntityKind::LaneEdge, geometry.lane_edge, &span)?;
            if geometry.centerline_points.len() < 2 {
                return Err(DiagnosticBundle::single(
                    Diagnostic::invalid_spatial_geometry(
                        Some(input.canonical_frame_key),
                        geometry.lane_edge.declaration_key(),
                        None,
                        SpatialGeometryViolation::InsufficientPoints {
                            minimum: 2,
                            actual: u32::try_from(geometry.centerline_points.len())
                                .unwrap_or(u32::MAX),
                        },
                        span,
                        None,
                    ),
                ));
            }
            for (point_index, point) in geometry.centerline_points.iter().enumerate() {
                for (axis, value) in [
                    (SpatialAxis::X, point.x),
                    (SpatialAxis::Y, point.y),
                    (SpatialAxis::Z, point.z),
                ] {
                    let point_index = u32::try_from(point_index).unwrap_or(u32::MAX);
                    let violation = if !value.is_finite() {
                        Some(SpatialGeometryViolation::NonFiniteCoordinate {
                            point_index,
                            axis,
                            value_bits: value.to_bits(),
                        })
                    } else if !(CANONICAL_POINT_COMPONENT_MIN_METERS
                        ..=CANONICAL_POINT_COMPONENT_MAX_METERS)
                        .contains(&value)
                    {
                        Some(SpatialGeometryViolation::CoordinateOutOfRange {
                            point_index,
                            axis,
                            value_bits: value.to_bits(),
                            minimum_bits: CANONICAL_POINT_COMPONENT_MIN_METERS.to_bits(),
                            maximum_bits: CANONICAL_POINT_COMPONENT_MAX_METERS.to_bits(),
                        })
                    } else {
                        None
                    };
                    if let Some(violation) = violation {
                        return Err(DiagnosticBundle::single(
                            Diagnostic::invalid_spatial_geometry(
                                Some(input.canonical_frame_key),
                                geometry.lane_edge.declaration_key(),
                                None,
                                violation,
                                span,
                                None,
                            ),
                        ));
                    }
                }
            }
        }

        let namespace_bytes =
            u64::try_from(self.header.authoring_namespace_id.len()).unwrap_or(u64::MAX);
        let key_bytes = u64::try_from(input.canonical_frame_key.len()).unwrap_or(u64::MAX);
        let geometry_count = u64::try_from(input.lane_edge_geometries.len()).unwrap_or(u64::MAX);
        let point_count = input
            .lane_edge_geometries
            .iter()
            .fold(0_u64, |total, geometry| {
                total.saturating_add(
                    u64::try_from(geometry.centerline_points.len()).unwrap_or(u64::MAX),
                )
            });
        let reference_string_bytes =
            input
                .lane_edge_geometries
                .iter()
                .fold(0_u64, |total, geometry| {
                    total.saturating_add(reference_spelling_parts_bytes(
                        geometry
                            .lane_edge
                            .module_namespace()
                            .unwrap_or(&self.header.authoring_namespace_id),
                        geometry.lane_edge.declaration_key(),
                    ))
                });
        let controlled_reference_bytes =
            input
                .lane_edge_geometries
                .iter()
                .fold(0_u64, |total, geometry| {
                    total.saturating_add(
                        u64::try_from(geometry.lane_edge.declaration_key().len())
                            .unwrap_or(u64::MAX),
                    )
                });
        let state = self.check_declaration_resources(
            DeclarationResourceDelta {
                declarations: 1,
                typed_ast_records: 3_u64
                    .saturating_add(geometry_count.saturating_mul(2))
                    .saturating_add(point_count),
                references: geometry_count,
                relations: geometry_count,
                identity_fields: 2,
                symbols: 1,
                string_items: 2_u64.saturating_add(geometry_count),
                string_bytes: namespace_bytes
                    .saturating_add(key_bytes)
                    .saturating_add(reference_string_bytes),
                controlled_string_bytes: key_bytes.saturating_add(controlled_reference_bytes),
                controlled_structural_bytes: size_bytes::<CanonicalFrameDeclaration>(1)
                    .saturating_add(size_bytes::<LaneEdgeGeometryDeclaration>(geometry_count))
                    .saturating_add(size_bytes::<CanonicalPoint3F32Input>(point_count)),
                source_bytes: canonical_frame_input_len(
                    input.canonical_frame_key,
                    input.lane_edge_geometries,
                    &self.header.authoring_namespace_id,
                ),
                geometry_points: point_count,
                ..DeclarationResourceDelta::default()
            },
            input.canonical_frame_key,
            &span,
        )?;

        // 只有资源预检成功后才为规范集合顺序分配暂存索引；不可信几何数量不能抢在
        // `max_geometry_point_count` / live-byte 检查之前触发线性分配。
        let mut ordered_geometries = input.lane_edge_geometries.iter().collect::<Vec<_>>();
        ordered_geometries.sort_unstable_by(|left, right| {
            left.lane_edge
                .module_namespace()
                .unwrap_or(&self.header.authoring_namespace_id)
                .cmp(
                    right
                        .lane_edge
                        .module_namespace()
                        .unwrap_or(&self.header.authoring_namespace_id),
                )
                .then_with(|| {
                    left.lane_edge
                        .declaration_key()
                        .cmp(right.lane_edge.declaration_key())
                })
        });
        if let Some(duplicate) = ordered_geometries.windows(2).find_map(|pair| {
            let left_namespace = pair[0]
                .lane_edge
                .module_namespace()
                .unwrap_or(&self.header.authoring_namespace_id);
            let right_namespace = pair[1]
                .lane_edge
                .module_namespace()
                .unwrap_or(&self.header.authoring_namespace_id);
            (left_namespace == right_namespace
                && pair[0].lane_edge.declaration_key() == pair[1].lane_edge.declaration_key())
            .then_some(pair[1])
        }) {
            return Err(DiagnosticBundle::single(
                Diagnostic::invalid_spatial_geometry(
                    Some(input.canonical_frame_key),
                    duplicate.lane_edge.declaration_key(),
                    None,
                    SpatialGeometryViolation::DuplicateEdgeBinding,
                    span.clone(),
                    None,
                ),
            ));
        }

        let lane_edge_geometries = ordered_geometries
            .into_iter()
            .map(|geometry| {
                let lane_edge =
                    self.own_reference(EntityKind::LaneEdge, geometry.lane_edge, &span)?;
                let centerline_points = geometry
                    .centerline_points
                    .iter()
                    .map(|point| CanonicalPoint3F32Input {
                        x: normalize_spatial_zero(point.x),
                        y: normalize_spatial_zero(point.y),
                        z: normalize_spatial_zero(point.z),
                    })
                    .collect::<Vec<_>>()
                    .into_boxed_slice();
                Ok(LaneEdgeGeometryDeclaration {
                    lane_edge,
                    centerline_points,
                })
            })
            .collect::<Result<Vec<_>, DiagnosticBundle>>()?
            .into_boxed_slice();

        let stable_key: Arc<str> = input.canonical_frame_key.into();
        self.declaration_index
            .entry(EntityKind::CanonicalFrame)
            .or_default()
            .insert(Arc::clone(&stable_key), span.clone().into());
        self.declarations.push(TypedAstDeclaration::CanonicalFrame(
            CanonicalFrameDeclaration {
                header: DeclarationHeader::module_scoped(
                    EntityKind::CanonicalFrame,
                    stable_key,
                    span.clone().into(),
                ),
                lane_edge_geometries,
            },
        ));
        self.commit_declaration_resources(state);
        Ok(self)
    }

    /// 声明一条永远适用的静态准入规则。
    ///
    /// # Errors
    ///
    /// 稳定键或任一引用非法、跨模块引用未显式导入、声明重复，或资源上限超限时
    /// 失败。空类别集合、未知目标/类别、FacilityBand capability guard、法规来源
    /// 一致性和组合歧义在 HIR 阶段按确定顺序验证。
    #[track_caller]
    pub fn add_access_rule(
        &mut self,
        input: AccessRuleInput<'_>,
    ) -> Result<&mut Self, DiagnosticBundle> {
        let span = SourceSpan::at_caller(
            Arc::clone(&self.header.source_document_key),
            std::panic::Location::caller(),
        );
        self.validate_declaration_key(EntityKind::AccessRule, input.access_rule_key, &span)?;
        match input.target {
            AccessRuleTargetInput::LaneEdge(reference) => {
                self.validate_reference(EntityKind::LaneEdge, reference, &span)?;
            }
            AccessRuleTargetInput::LaneGroup(reference) => {
                self.validate_reference(EntityKind::LaneGroup, reference, &span)?;
            }
            AccessRuleTargetInput::RoadSection(reference) => {
                self.validate_reference(EntityKind::RoadSection, reference, &span)?;
            }
            AccessRuleTargetInput::ManeuverPath(reference) => {
                self.validate_reference(EntityKind::ManeuverPath, reference, &span)?;
            }
            AccessRuleTargetInput::FacilityBand(reference) => {
                self.validate_reference(EntityKind::FacilityBand, reference, &span)?;
            }
        }
        for reference in input.participant_classes {
            self.validate_reference(EntityKind::ParticipantClass, *reference, &span)?;
        }

        let reference_count = 1_u64
            .saturating_add(u64::try_from(input.participant_classes.len()).unwrap_or(u64::MAX));
        let regulation_string_count = input.regulation.map_or(0_u64, |regulation| {
            2_u64.saturating_add(u64::from(regulation.source.is_some()))
        });
        let namespace_bytes =
            u64::try_from(self.header.authoring_namespace_id.len()).unwrap_or(u64::MAX);
        let key_bytes = u64::try_from(input.access_rule_key.len()).unwrap_or(u64::MAX);
        let (_, target_namespace, target_key) = access_target_input_parts(input.target);
        let target_namespace = target_namespace.unwrap_or(&self.header.authoring_namespace_id);
        let target_bytes = reference_spelling_parts_bytes(target_namespace, target_key);
        let class_reference_bytes =
            input
                .participant_classes
                .iter()
                .fold(0_u64, |total, reference| {
                    let namespace = reference
                        .module_namespace()
                        .unwrap_or(&self.header.authoring_namespace_id);
                    total.saturating_add(reference_spelling_parts_bytes(
                        namespace,
                        reference.declaration_key(),
                    ))
                });
        let controlled_reference_bytes = input.participant_classes.iter().fold(
            u64::try_from(target_key.len()).unwrap_or(u64::MAX),
            |total, reference| {
                total.saturating_add(
                    u64::try_from(reference.declaration_key().len()).unwrap_or(u64::MAX),
                )
            },
        );
        let regulation_bytes = input.regulation.map_or(0_u64, |regulation| {
            u64::try_from(regulation.jurisdiction.len())
                .unwrap_or(u64::MAX)
                .saturating_add(u64::try_from(regulation.version.len()).unwrap_or(u64::MAX))
                .saturating_add(
                    regulation
                        .source
                        .as_ref()
                        .map_or(0, |source| u64::try_from(source.len()).unwrap_or(u64::MAX)),
                )
        });
        let state = self.check_declaration_resources(
            DeclarationResourceDelta {
                declarations: 1,
                typed_ast_records: 8_u64
                    .saturating_add(reference_count.saturating_mul(2))
                    .saturating_add(regulation_string_count),
                references: reference_count,
                relations: reference_count,
                identity_fields: 2,
                symbols: 1,
                string_items: 2_u64
                    .saturating_add(reference_count)
                    .saturating_add(regulation_string_count),
                string_bytes: namespace_bytes
                    .saturating_add(key_bytes)
                    .saturating_add(target_bytes)
                    .saturating_add(class_reference_bytes)
                    .saturating_add(regulation_bytes),
                controlled_string_bytes: key_bytes
                    .saturating_add(controlled_reference_bytes)
                    .saturating_add(regulation_bytes),
                controlled_structural_bytes: size_bytes::<AccessRuleDeclaration>(1)
                    .saturating_add(size_bytes::<OwnedAccessRuleTarget>(1))
                    .saturating_add(size_bytes::<OwnedEntityReference<ParticipantClassKind>>(
                        u64::try_from(input.participant_classes.len()).unwrap_or(u64::MAX),
                    ))
                    .saturating_add(size_bytes::<OwnedAccessRegulation>(u64::from(
                        input.regulation.is_some(),
                    ))),
                source_bytes: access_rule_input_len(
                    input.access_rule_key,
                    input.target,
                    input.participant_classes,
                    input.regulation,
                    &self.header.authoring_namespace_id,
                ),
                ..DeclarationResourceDelta::default()
            },
            input.access_rule_key,
            &span,
        )?;

        // 所有计数、字符串和来源长度门禁均已通过后才复制可变长输入，保证超限调用不会
        // 先按不受信任的 slice 长度申请内存，也不会留下部分声明。
        let target = match input.target {
            AccessRuleTargetInput::LaneEdge(reference) => OwnedAccessRuleTarget::LaneEdge(
                self.own_reference(EntityKind::LaneEdge, reference, &span)?,
            ),
            AccessRuleTargetInput::LaneGroup(reference) => OwnedAccessRuleTarget::LaneGroup(
                self.own_reference(EntityKind::LaneGroup, reference, &span)?,
            ),
            AccessRuleTargetInput::RoadSection(reference) => OwnedAccessRuleTarget::RoadSection(
                self.own_reference(EntityKind::RoadSection, reference, &span)?,
            ),
            AccessRuleTargetInput::ManeuverPath(reference) => OwnedAccessRuleTarget::ManeuverPath(
                self.own_reference(EntityKind::ManeuverPath, reference, &span)?,
            ),
            AccessRuleTargetInput::FacilityBand(reference) => OwnedAccessRuleTarget::FacilityBand(
                self.own_reference(EntityKind::FacilityBand, reference, &span)?,
            ),
        };
        let mut participant_classes = Vec::with_capacity(input.participant_classes.len());
        for reference in input.participant_classes {
            participant_classes.push(self.own_reference(
                EntityKind::ParticipantClass,
                *reference,
                &span,
            )?);
        }
        let regulation = input.regulation.map(|regulation| OwnedAccessRegulation {
            jurisdiction: regulation.jurisdiction.into(),
            version: regulation.version.into(),
            source: regulation.source.map(Into::into),
        });

        let stable_key: Arc<str> = input.access_rule_key.into();
        self.declaration_index
            .entry(EntityKind::AccessRule)
            .or_default()
            .insert(Arc::clone(&stable_key), span.clone().into());
        self.declarations
            .push(TypedAstDeclaration::AccessRule(AccessRuleDeclaration {
                header: DeclarationHeader::module_scoped(
                    EntityKind::AccessRule,
                    stable_key,
                    span.clone().into(),
                ),
                target,
                effect: input.effect,
                participant_classes: participant_classes.into_boxed_slice(),
                regulation,
                priority: input.priority,
            }));
        self.commit_declaration_resources(state);
        Ok(self)
    }

    /// 声明一个位于机动路径转换上的机动门。
    ///
    /// # Errors
    ///
    /// 稳定键或引用非法、跨模块引用未显式导入、声明重复，或资源上限超限时失败。
    /// 转换下标、同转换唯一性和停止线位置在 HIR 阶段验证。
    #[track_caller]
    pub fn add_maneuver_gate(
        &mut self,
        input: ManeuverGateInput<'_>,
    ) -> Result<&mut Self, DiagnosticBundle> {
        let span = SourceSpan::at_caller(
            Arc::clone(&self.header.source_document_key),
            std::panic::Location::caller(),
        );
        self.validate_declaration_key(EntityKind::ManeuverGate, input.maneuver_gate_key, &span)?;
        let maneuver_path =
            self.own_reference(EntityKind::ManeuverPath, input.maneuver_path, &span)?;
        let stop_line = self.own_reference(EntityKind::StopLine, input.stop_line, &span)?;
        let signal_control = match input.signal_control {
            SignalControlInput::Group(group) => OwnedSignalControl::Group(self.own_reference(
                EntityKind::SignalGroup,
                group,
                &span,
            )?),
            SignalControlInput::None => OwnedSignalControl::None,
        };
        let signal_group_reference = match &signal_control {
            OwnedSignalControl::Group(group) => Some(group),
            OwnedSignalControl::None => None,
        };
        let namespace_bytes =
            u64::try_from(self.header.authoring_namespace_id.len()).unwrap_or(u64::MAX);
        let key_bytes = u64::try_from(input.maneuver_gate_key.len()).unwrap_or(u64::MAX);
        let state = self.check_declaration_resources(
            DeclarationResourceDelta {
                declarations: 1,
                typed_ast_records: 8_u64
                    .saturating_add(u64::from(signal_group_reference.is_some()).saturating_mul(2)),
                references: 2_u64.saturating_add(u64::from(signal_group_reference.is_some())),
                relations: 2_u64.saturating_add(u64::from(signal_group_reference.is_some())),
                identity_fields: 3,
                symbols: 1,
                string_items: 4_u64.saturating_add(u64::from(signal_group_reference.is_some())),
                string_bytes: namespace_bytes
                    .saturating_add(key_bytes)
                    .saturating_add(reference_spelling_bytes(&maneuver_path))
                    .saturating_add(reference_spelling_bytes(&stop_line))
                    .saturating_add(signal_group_reference.map_or(0, reference_spelling_bytes)),
                controlled_string_bytes: key_bytes
                    .saturating_add(
                        u64::try_from(maneuver_path.declaration_key().len()).unwrap_or(u64::MAX),
                    )
                    .saturating_add(
                        u64::try_from(stop_line.declaration_key().len()).unwrap_or(u64::MAX),
                    )
                    .saturating_add(signal_group_reference.map_or(0, |group| {
                        u64::try_from(group.declaration_key().len()).unwrap_or(u64::MAX)
                    })),
                controlled_structural_bytes: size_bytes::<ManeuverGateDeclaration>(1)
                    .saturating_add(size_bytes::<OwnedEntityReference<ManeuverPathKind>>(1))
                    .saturating_add(size_bytes::<OwnedEntityReference<StopLineKind>>(1))
                    .saturating_add(size_bytes::<OwnedEntityReference<SignalGroupKind>>(
                        u64::from(signal_group_reference.is_some()),
                    )),
                source_bytes: maneuver_gate_declaration_len(
                    input.maneuver_gate_key,
                    &maneuver_path,
                    input.transition_index,
                    &stop_line,
                    &signal_control,
                ),
                maneuver_gates: 1,
                ..DeclarationResourceDelta::default()
            },
            input.maneuver_gate_key,
            &span,
        )?;

        let stable_key: Arc<str> = input.maneuver_gate_key.into();
        self.declaration_index
            .entry(EntityKind::ManeuverGate)
            .or_default()
            .insert(Arc::clone(&stable_key), span.clone().into());
        self.declarations
            .push(TypedAstDeclaration::ManeuverGate(ManeuverGateDeclaration {
                header: DeclarationHeader::module_scoped(
                    EntityKind::ManeuverGate,
                    stable_key,
                    span.clone().into(),
                ),
                maneuver_path,
                transition_index: input.transition_index,
                stop_line,
                signal_control,
            }));
        self.commit_declaration_resources(state);
        Ok(self)
    }

    /// 声明一个由同一路径入口门和释放门界定的等待区。
    ///
    /// # Errors
    ///
    /// `max_occupancy` 为零，稳定键或引用非法、跨模块引用未显式导入、声明重复，或
    /// 资源上限超限时失败。门所有权、严格顺序和等待区内部不重叠约束在 HIR 阶段验证。
    #[track_caller]
    pub fn add_waiting_zone(
        &mut self,
        input: WaitingZoneInput<'_>,
    ) -> Result<&mut Self, DiagnosticBundle> {
        let span = SourceSpan::at_caller(
            Arc::clone(&self.header.source_document_key),
            std::panic::Location::caller(),
        );
        self.validate_declaration_key(EntityKind::WaitingZone, input.waiting_zone_key, &span)?;
        if input.max_occupancy == 0 {
            return Err(DiagnosticBundle::single(
                Diagnostic::invalid_waiting_zone_capacity(input.waiting_zone_key, span),
            ));
        }
        let maneuver_path =
            self.own_reference(EntityKind::ManeuverPath, input.maneuver_path, &span)?;
        let entry_gate = self.own_reference(EntityKind::ManeuverGate, input.entry_gate, &span)?;
        let release_gate =
            self.own_reference(EntityKind::ManeuverGate, input.release_gate, &span)?;
        let namespace_bytes =
            u64::try_from(self.header.authoring_namespace_id.len()).unwrap_or(u64::MAX);
        let key_bytes = u64::try_from(input.waiting_zone_key.len()).unwrap_or(u64::MAX);
        let references = [
            &maneuver_path.declaration_key(),
            &entry_gate.declaration_key(),
            &release_gate.declaration_key(),
        ];
        let state = self.check_declaration_resources(
            DeclarationResourceDelta {
                declarations: 1,
                typed_ast_records: 9,
                references: 3,
                relations: 3,
                identity_fields: 3,
                symbols: 1,
                string_items: 5,
                string_bytes: namespace_bytes
                    .saturating_add(key_bytes)
                    .saturating_add(reference_spelling_bytes(&maneuver_path))
                    .saturating_add(reference_spelling_bytes(&entry_gate))
                    .saturating_add(reference_spelling_bytes(&release_gate)),
                controlled_string_bytes: references.iter().fold(key_bytes, |total, value| {
                    total.saturating_add(u64::try_from(value.len()).unwrap_or(u64::MAX))
                }),
                controlled_structural_bytes: size_bytes::<WaitingZoneDeclaration>(1)
                    .saturating_add(size_bytes::<OwnedEntityReference<ManeuverPathKind>>(1))
                    .saturating_add(size_bytes::<OwnedEntityReference<ManeuverGateKind>>(2)),
                source_bytes: waiting_zone_declaration_len(
                    input.waiting_zone_key,
                    &maneuver_path,
                    &entry_gate,
                    &release_gate,
                ),
                waiting_zones: 1,
                ..DeclarationResourceDelta::default()
            },
            input.waiting_zone_key,
            &span,
        )?;

        let stable_key: Arc<str> = input.waiting_zone_key.into();
        self.declaration_index
            .entry(EntityKind::WaitingZone)
            .or_default()
            .insert(Arc::clone(&stable_key), span.clone().into());
        self.declarations
            .push(TypedAstDeclaration::WaitingZone(WaitingZoneDeclaration {
                header: DeclarationHeader::module_scoped(
                    EntityKind::WaitingZone,
                    stable_key,
                    span.clone().into(),
                ),
                maneuver_path,
                entry_gate,
                release_gate,
                max_occupancy: input.max_occupancy,
            }));
        self.commit_declaration_resources(state);
        Ok(self)
    }

    /// 声明一条编制期静态路线并保留其有序车道图边出现序列。
    ///
    /// 同一边可以重复出现；调用方和后续表必须使用路线内下标区分每次出现，不能按
    /// `LaneEdge` 身份去重。相邻连通性、路口边界和控制出现项闭包在 HIR 阶段验证。
    ///
    /// # Errors
    ///
    /// 路线为空，稳定键或边引用非法，跨模块引用未显式导入，声明重复，或资源上限
    /// 超限时失败。失败不会插入部分声明或改变累计计数。
    #[track_caller]
    pub fn add_static_route(
        &mut self,
        input: StaticRouteInput<'_>,
    ) -> Result<&mut Self, DiagnosticBundle> {
        let span = SourceSpan::at_caller(
            Arc::clone(&self.header.source_document_key),
            std::panic::Location::caller(),
        );
        self.validate_declaration_key(EntityKind::StaticRoute, input.static_route_key, &span)?;
        if input.edge_sequence.is_empty() {
            return Err(DiagnosticBundle::single(Diagnostic::empty_static_route(
                input.static_route_key,
                span,
            )));
        }

        // 先只借用调用方切片完成校验和精确资源预检；超大不可信序列不得通过
        // Vec::with_capacity 或字符串复制抢在 max_route_occurrence_count 前分配。
        for reference in input.edge_sequence {
            self.validate_reference(EntityKind::LaneEdge, *reference, &span)?;
        }
        let occurrence_count = u64::try_from(input.edge_sequence.len()).unwrap_or(u64::MAX);
        let namespace_bytes =
            u64::try_from(self.header.authoring_namespace_id.len()).unwrap_or(u64::MAX);
        let key_bytes = u64::try_from(input.static_route_key.len()).unwrap_or(u64::MAX);
        let reference_string_bytes = input.edge_sequence.iter().fold(0_u64, |total, edge| {
            let namespace = edge
                .module_namespace()
                .unwrap_or(&self.header.authoring_namespace_id);
            total.saturating_add(reference_spelling_parts_bytes(
                namespace,
                edge.declaration_key(),
            ))
        });
        let controlled_reference_bytes = input.edge_sequence.iter().fold(0_u64, |total, edge| {
            total.saturating_add(u64::try_from(edge.declaration_key().len()).unwrap_or(u64::MAX))
        });
        let source_bytes = input.edge_sequence.iter().fold(
            declaration_header_len(input.static_route_key).saturating_add(4),
            |total, edge| {
                let namespace = edge
                    .module_namespace()
                    .unwrap_or(&self.header.authoring_namespace_id);
                total.saturating_add(encoded_reference_len(namespace, edge.declaration_key()))
            },
        );
        let state = self.check_declaration_resources(
            DeclarationResourceDelta {
                declarations: 1,
                typed_ast_records: 3_u64.saturating_add(occurrence_count.saturating_mul(2)),
                references: occurrence_count,
                relations: occurrence_count,
                identity_fields: 2,
                symbols: 1,
                string_items: 2_u64.saturating_add(occurrence_count),
                string_bytes: namespace_bytes
                    .saturating_add(key_bytes)
                    .saturating_add(reference_string_bytes),
                controlled_string_bytes: key_bytes.saturating_add(controlled_reference_bytes),
                controlled_structural_bytes: size_bytes::<StaticRouteDeclaration>(1)
                    .saturating_add(size_bytes::<OwnedEntityReference<LaneEdgeKind>>(
                        occurrence_count,
                    )),
                source_bytes,
                route_occurrences: occurrence_count,
                ..DeclarationResourceDelta::default()
            },
            input.static_route_key,
            &span,
        )?;

        let mut edge_sequence = Vec::with_capacity(input.edge_sequence.len());
        for reference in input.edge_sequence {
            edge_sequence.push(self.own_reference(EntityKind::LaneEdge, *reference, &span)?);
        }

        let stable_key: Arc<str> = input.static_route_key.into();
        self.declaration_index
            .entry(EntityKind::StaticRoute)
            .or_default()
            .insert(Arc::clone(&stable_key), span.clone().into());
        self.declarations
            .push(TypedAstDeclaration::StaticRoute(StaticRouteDeclaration {
                header: DeclarationHeader::module_scoped(
                    EntityKind::StaticRoute,
                    stable_key,
                    span.into(),
                ),
                edge_sequence: edge_sequence.into_boxed_slice(),
            }));
        self.commit_declaration_resources(state);
        Ok(self)
    }

    /// 声明一个非遍历设施带；唯一走廊所有者在完整模块图中解析。
    ///
    /// # Errors
    ///
    /// 稳定键、`kind_id` 或其类别非法，声明重复，或资源上限超限时失败。失败不会改变
    /// 构建器。
    #[track_caller]
    pub fn add_facility_band(
        &mut self,
        input: FacilityBandInput<'_>,
    ) -> Result<&mut Self, DiagnosticBundle> {
        let span = SourceSpan::at_caller(
            Arc::clone(&self.header.source_document_key),
            std::panic::Location::caller(),
        );
        self.validate_declaration_key(EntityKind::FacilityBand, input.facility_band_key, &span)?;
        self.validate_facility_kind(
            EntityKind::FacilityBand,
            input.facility_band_key,
            input.kind_id,
            FacilityKindCategory::NonTraversable,
            &span,
        )?;
        let namespace_bytes =
            u64::try_from(self.header.authoring_namespace_id.len()).unwrap_or(u64::MAX);
        let key_bytes = u64::try_from(input.facility_band_key.len()).unwrap_or(u64::MAX);
        let kind_bytes = u64::try_from(input.kind_id.len()).unwrap_or(u64::MAX);
        let state = self.check_declaration_resources(
            DeclarationResourceDelta {
                declarations: 1,
                typed_ast_records: 3,
                identity_fields: 3,
                symbols: 1,
                string_items: 3,
                string_bytes: namespace_bytes
                    .saturating_add(key_bytes)
                    .saturating_add(kind_bytes),
                controlled_string_bytes: key_bytes.saturating_add(kind_bytes),
                controlled_structural_bytes: size_bytes::<FacilityBandDeclaration>(1),
                source_bytes: facility_band_declaration_len(input.facility_band_key, input.kind_id),
                ..DeclarationResourceDelta::default()
            },
            input.facility_band_key,
            &span,
        )?;

        let stable_key: Arc<str> = input.facility_band_key.into();
        let declaration = TypedAstDeclaration::FacilityBand(FacilityBandDeclaration {
            header: DeclarationHeader::module_scoped(
                EntityKind::FacilityBand,
                Arc::clone(&stable_key),
                span.clone().into(),
            ),
            kind_id: input.kind_id.into(),
            authoring_width_profile: None,
            compiled_geometry: None,
        });
        self.declaration_index
            .entry(EntityKind::FacilityBand)
            .or_default()
            .insert(Arc::clone(&stable_key), span.into());
        self.declarations.push(declaration);
        self.commit_declaration_resources(state);
        Ok(self)
    }

    /// 声明一个车道组及其唯一道路区段父项。
    ///
    /// 车道成员由道路区段内各 `AuthoringLaneInput::lane_group` 反向形成；本操作不会
    /// 接受第二份成员数组。
    ///
    /// # Errors
    ///
    /// 稳定键或父项引用非法、跨模块引用未显式导入、声明重复，或资源上限超限时
    /// 失败。父项存在性与非空成员约束在完整模块图建立后验证。
    #[track_caller]
    pub fn add_lane_group(
        &mut self,
        input: LaneGroupInput<'_>,
    ) -> Result<&mut Self, DiagnosticBundle> {
        let span = SourceSpan::at_caller(
            Arc::clone(&self.header.source_document_key),
            std::panic::Location::caller(),
        );
        self.validate_declaration_key(EntityKind::LaneGroup, input.lane_group_key, &span)?;
        let road_section =
            self.own_reference(EntityKind::RoadSection, input.road_section, &span)?;
        let namespace_bytes =
            u64::try_from(self.header.authoring_namespace_id.len()).unwrap_or(u64::MAX);
        let key_bytes = u64::try_from(input.lane_group_key.len()).unwrap_or(u64::MAX);
        let reference_bytes = reference_spelling_bytes(&road_section);
        let state = self.check_declaration_resources(
            DeclarationResourceDelta {
                declarations: 1,
                typed_ast_records: 5,
                references: 1,
                relations: 1,
                identity_fields: 3,
                symbols: 1,
                string_items: 3,
                string_bytes: namespace_bytes
                    .saturating_add(key_bytes)
                    .saturating_add(reference_bytes),
                controlled_string_bytes: key_bytes.saturating_add(
                    u64::try_from(road_section.declaration_key().len()).unwrap_or(u64::MAX),
                ),
                controlled_structural_bytes: size_bytes::<LaneGroupDeclaration>(1)
                    .saturating_add(size_bytes::<OwnedEntityReference<RoadSectionKind>>(1)),
                source_bytes: lane_group_declaration_len(input.lane_group_key, &road_section),
                ..DeclarationResourceDelta::default()
            },
            input.lane_group_key,
            &span,
        )?;

        let stable_key: Arc<str> = input.lane_group_key.into();
        let declaration = TypedAstDeclaration::LaneGroup(LaneGroupDeclaration {
            header: DeclarationHeader::module_scoped(
                EntityKind::LaneGroup,
                Arc::clone(&stable_key),
                span.clone().into(),
            ),
            road_section,
        });
        self.declaration_index
            .entry(EntityKind::LaneGroup)
            .or_default()
            .insert(Arc::clone(&stable_key), span.into());
        self.declarations.push(declaration);
        self.commit_declaration_resources(state);
        Ok(self)
    }

    /// 声明道路区段及其按走廊参考方向排列的编制车道。
    ///
    /// # Errors
    ///
    /// 区段/车道稳定键、设施类别或引用非法，区段或车道链为空，同一车道链重复覆盖
    /// 车道图边，声明重复，或资源上限超限时失败。引用存在性、链连通性、跨车道覆盖
    /// 冲突和车道组父项一致性在完整模块图建立后验证。失败不会插入区段或任何嵌套
    /// 编制车道。
    #[track_caller]
    pub fn add_road_section(
        &mut self,
        input: RoadSectionInput<'_>,
    ) -> Result<&mut Self, DiagnosticBundle> {
        let span = SourceSpan::at_caller(
            Arc::clone(&self.header.source_document_key),
            std::panic::Location::caller(),
        );
        self.validate_declaration_key(EntityKind::RoadSection, input.road_section_key, &span)?;
        self.validate_facility_kind(
            EntityKind::RoadSection,
            input.road_section_key,
            input.kind_id,
            FacilityKindCategory::LaneBearing,
            &span,
        )?;
        if input.lanes.is_empty() {
            return Err(DiagnosticBundle::single(
                Diagnostic::empty_road_section_lanes(input.road_section_key, span),
            ));
        }

        let mut lane_keys = BTreeSet::new();
        let mut lanes = Vec::with_capacity(input.lanes.len());
        let mut edge_reference_count = 0_u64;
        let mut lane_group_reference_count = 0_u64;
        for lane in input.lanes {
            self.validate_declaration_key(
                EntityKind::AuthoringLane,
                lane.authoring_lane_key,
                &span,
            )?;
            if !lane_keys.insert(lane.authoring_lane_key) {
                return Err(DiagnosticBundle::single(Diagnostic::duplicate_declaration(
                    EntityKind::AuthoringLane,
                    lane.authoring_lane_key,
                    span.clone(),
                    span.clone(),
                )));
            }
            if lane.edge_chain.is_empty() {
                return Err(DiagnosticBundle::single(
                    Diagnostic::empty_authoring_lane_edge_chain(
                        lane.authoring_lane_key,
                        span.clone(),
                    ),
                ));
            }
            let mut edge_chain = Vec::with_capacity(lane.edge_chain.len());
            let mut seen_edges = BTreeSet::new();
            for edge in lane.edge_chain {
                let edge = self.own_reference(EntityKind::LaneEdge, *edge, &span)?;
                let edge_key = (
                    Arc::clone(&edge.module_namespace),
                    Arc::clone(edge.declaration_key()),
                );
                if !seen_edges.insert(edge_key) {
                    return Err(DiagnosticBundle::single(
                        Diagnostic::duplicate_authoring_lane_edge(
                            lane.authoring_lane_key,
                            &edge.module_namespace,
                            edge.declaration_key(),
                            span.clone(),
                        ),
                    ));
                }
                edge_chain.push(edge);
            }
            edge_reference_count = edge_reference_count
                .saturating_add(u64::try_from(edge_chain.len()).unwrap_or(u64::MAX));
            let lane_group = lane
                .lane_group
                .map(|reference| self.own_reference(EntityKind::LaneGroup, reference, &span))
                .transpose()?;
            lane_group_reference_count =
                lane_group_reference_count.saturating_add(u64::from(lane_group.is_some()));
            lanes.push(AuthoringLaneDeclaration {
                header: DeclarationHeader::module_scoped(
                    EntityKind::AuthoringLane,
                    lane.authoring_lane_key.into(),
                    span.clone().into(),
                ),
                section_relation_span: span.clone().into(),
                edge_chain: edge_chain.into_boxed_slice(),
                lane_group,
                authoring_geometry: None,
            });
        }

        let lane_count = u64::try_from(lanes.len()).unwrap_or(u64::MAX);
        let reference_count = edge_reference_count.saturating_add(lane_group_reference_count);
        let mut logical_string_bytes = u64::try_from(self.header.authoring_namespace_id.len())
            .unwrap_or(u64::MAX)
            .saturating_add(u64::try_from(input.road_section_key.len()).unwrap_or(u64::MAX))
            .saturating_add(u64::try_from(input.kind_id.len()).unwrap_or(u64::MAX));
        let mut controlled_string_bytes = u64::try_from(input.road_section_key.len())
            .unwrap_or(u64::MAX)
            .saturating_add(u64::try_from(input.kind_id.len()).unwrap_or(u64::MAX));
        for lane in &lanes {
            logical_string_bytes = logical_string_bytes
                .saturating_add(
                    u64::try_from(self.header.authoring_namespace_id.len()).unwrap_or(u64::MAX),
                )
                .saturating_add(u64::try_from(lane.header.stable_key.len()).unwrap_or(u64::MAX));
            controlled_string_bytes = controlled_string_bytes
                .saturating_add(u64::try_from(lane.header.stable_key.len()).unwrap_or(u64::MAX));
            for edge in &lane.edge_chain {
                logical_string_bytes =
                    logical_string_bytes.saturating_add(reference_spelling_bytes(edge));
                controlled_string_bytes = controlled_string_bytes.saturating_add(
                    u64::try_from(edge.declaration_key().len()).unwrap_or(u64::MAX),
                );
            }
            if let Some(group) = &lane.lane_group {
                logical_string_bytes =
                    logical_string_bytes.saturating_add(reference_spelling_bytes(group));
                controlled_string_bytes = controlled_string_bytes.saturating_add(
                    u64::try_from(group.declaration_key().len()).unwrap_or(u64::MAX),
                );
            }
        }
        let structural_bytes = size_bytes::<RoadSectionDeclaration>(1)
            .saturating_add(size_bytes::<AuthoringLaneDeclaration>(lane_count))
            .saturating_add(size_bytes::<OwnedEntityReference<LaneEdgeKind>>(
                edge_reference_count,
            ))
            .saturating_add(size_bytes::<OwnedEntityReference<LaneGroupKind>>(
                lane_group_reference_count,
            ));
        let source_bytes =
            road_section_declaration_len(input.road_section_key, input.kind_id, &lanes);
        let state = self.check_declaration_resources(
            DeclarationResourceDelta {
                declarations: 1_u64.saturating_add(lane_count),
                typed_ast_records: 3_u64
                    .saturating_add(lane_count.saturating_mul(3))
                    .saturating_add(reference_count.saturating_mul(2)),
                references: reference_count,
                relations: lane_count.saturating_add(reference_count),
                identity_fields: 3_u64.saturating_mul(1_u64.saturating_add(lane_count)),
                symbols: 1_u64.saturating_add(lane_count),
                string_items: 3_u64
                    .saturating_add(lane_count.saturating_mul(2))
                    .saturating_add(reference_count),
                string_bytes: logical_string_bytes,
                controlled_string_bytes,
                controlled_structural_bytes: structural_bytes,
                source_bytes,
                ..DeclarationResourceDelta::default()
            },
            input.road_section_key,
            &span,
        )?;

        let stable_key: Arc<str> = input.road_section_key.into();
        for lane in &lanes {
            self.declaration_index
                .entry(EntityKind::AuthoringLane)
                .or_default()
                .insert(
                    Arc::clone(&lane.header.stable_key),
                    lane.header.span.clone(),
                );
        }
        let declaration = TypedAstDeclaration::RoadSection(RoadSectionDeclaration {
            header: DeclarationHeader::module_scoped(
                EntityKind::RoadSection,
                Arc::clone(&stable_key),
                span.clone().into(),
            ),
            kind_id: input.kind_id.into(),
            lanes: lanes.into_boxed_slice(),
        });
        self.declaration_index
            .entry(EntityKind::RoadSection)
            .or_default()
            .insert(Arc::clone(&stable_key), span.into());
        self.declarations.push(declaration);
        self.commit_declaration_resources(state);
        Ok(self)
    }

    /// 声明道路走廊、参考道路区段和有序异构横断面成员。
    ///
    /// # Errors
    ///
    /// 稳定键或引用非法，成员为空或重复，声明重复，或资源上限超限时失败。成员目标
    /// 存在性、完备唯一所有者树和参考区段成员性在完整模块图建立后验证。
    #[track_caller]
    pub fn add_road_corridor(
        &mut self,
        input: RoadCorridorInput<'_>,
    ) -> Result<&mut Self, DiagnosticBundle> {
        let span = SourceSpan::at_caller(
            Arc::clone(&self.header.source_document_key),
            std::panic::Location::caller(),
        );
        self.validate_declaration_key(EntityKind::RoadCorridor, input.road_corridor_key, &span)?;
        if input.elements.is_empty() {
            return Err(DiagnosticBundle::single(
                Diagnostic::empty_road_corridor_elements(input.road_corridor_key, span),
            ));
        }
        let reference_section =
            self.own_reference(EntityKind::RoadSection, input.reference_section, &span)?;
        let mut elements = Vec::with_capacity(input.elements.len());
        let mut seen_elements = BTreeSet::new();
        for element in input.elements {
            let (target_kind, target_namespace, target_key, owned) = match *element {
                CorridorElementReference::RoadSection(reference) => {
                    let owned = self.own_reference(EntityKind::RoadSection, reference, &span)?;
                    (
                        EntityKind::RoadSection,
                        Arc::clone(&owned.module_namespace),
                        Arc::clone(owned.declaration_key()),
                        OwnedCorridorElementReference::RoadSection(owned),
                    )
                }
                CorridorElementReference::FacilityBand(reference) => {
                    let owned = self.own_reference(EntityKind::FacilityBand, reference, &span)?;
                    (
                        EntityKind::FacilityBand,
                        Arc::clone(&owned.module_namespace),
                        Arc::clone(owned.declaration_key()),
                        OwnedCorridorElementReference::FacilityBand(owned),
                    )
                }
            };
            if !seen_elements.insert((
                target_kind,
                Arc::clone(&target_namespace),
                Arc::clone(&target_key),
            )) {
                return Err(DiagnosticBundle::single(
                    Diagnostic::duplicate_road_corridor_element(
                        input.road_corridor_key,
                        target_kind,
                        &target_namespace,
                        &target_key,
                        span.clone(),
                    ),
                ));
            }
            elements.push(owned);
        }

        let element_count = u64::try_from(elements.len()).unwrap_or(u64::MAX);
        let reference_count = 1_u64.saturating_add(element_count);
        let key_bytes = u64::try_from(input.road_corridor_key.len()).unwrap_or(u64::MAX);
        let mut logical_string_bytes = u64::try_from(self.header.authoring_namespace_id.len())
            .unwrap_or(u64::MAX)
            .saturating_add(key_bytes)
            .saturating_add(reference_spelling_bytes(&reference_section));
        let mut controlled_string_bytes = key_bytes.saturating_add(
            u64::try_from(reference_section.declaration_key().len()).unwrap_or(u64::MAX),
        );
        for element in &elements {
            let (namespace, key) = match element {
                OwnedCorridorElementReference::RoadSection(reference) => {
                    (&reference.module_namespace, &reference.declaration_key())
                }
                OwnedCorridorElementReference::FacilityBand(reference) => {
                    (&reference.module_namespace, &reference.declaration_key())
                }
            };
            logical_string_bytes =
                logical_string_bytes.saturating_add(reference_spelling_parts_bytes(namespace, key));
            controlled_string_bytes = controlled_string_bytes
                .saturating_add(u64::try_from(key.len()).unwrap_or(u64::MAX));
        }
        let state = self.check_declaration_resources(
            DeclarationResourceDelta {
                declarations: 1,
                typed_ast_records: 3_u64.saturating_add(reference_count.saturating_mul(2)),
                references: reference_count,
                relations: reference_count,
                identity_fields: 2,
                symbols: 1,
                string_items: 2_u64.saturating_add(reference_count),
                string_bytes: logical_string_bytes,
                controlled_string_bytes,
                controlled_structural_bytes: size_bytes::<RoadCorridorDeclaration>(1)
                    .saturating_add(size_bytes::<OwnedEntityReference<RoadSectionKind>>(1))
                    .saturating_add(size_bytes::<OwnedCorridorElementReference>(element_count)),
                source_bytes: road_corridor_declaration_len(
                    input.road_corridor_key,
                    &reference_section,
                    &elements,
                ),
                ..DeclarationResourceDelta::default()
            },
            input.road_corridor_key,
            &span,
        )?;

        let stable_key: Arc<str> = input.road_corridor_key.into();
        let declaration = TypedAstDeclaration::RoadCorridor(RoadCorridorDeclaration {
            header: DeclarationHeader::module_scoped(
                EntityKind::RoadCorridor,
                Arc::clone(&stable_key),
                span.clone().into(),
            ),
            reference_section,
            elements: elements.into_boxed_slice(),
            authoring_geometry: None,
        });
        self.declaration_index
            .entry(EntityKind::RoadCorridor)
            .or_default()
            .insert(Arc::clone(&stable_key), span.into());
        self.declarations.push(declaration);
        self.commit_declaration_resources(state);
        Ok(self)
    }

    fn reference_namespace<'a>(
        &'a self,
        explicit_namespace: Option<&'a str>,
        span: &SourceSpan,
    ) -> Result<&'a str, DiagnosticBundle> {
        let Some(namespace) = explicit_namespace else {
            return Ok(&self.header.authoring_namespace_id);
        };
        if namespace == self.header.authoring_namespace_id.as_ref() {
            return Ok(&self.header.authoring_namespace_id);
        }
        let Some(import_index) = self.import_index.get(namespace).copied() else {
            return Err(DiagnosticBundle::single(
                Diagnostic::unimported_reference_module(namespace, span.clone()),
            ));
        };
        Ok(&self.imports[import_index].namespace)
    }

    fn reference_namespace_arc(
        &self,
        explicit_namespace: Option<&str>,
        span: &SourceSpan,
    ) -> Result<Arc<str>, DiagnosticBundle> {
        let namespace = self.reference_namespace(explicit_namespace, span)?;
        if namespace == self.header.authoring_namespace_id.as_ref() {
            return Ok(Arc::clone(&self.header.authoring_namespace_id));
        }
        let import_index = self.import_index[namespace];
        Ok(Arc::clone(&self.imports[import_index].namespace))
    }

    /// 原子派生来源记录、SHA-256 内容摘要与不可配错的模块描述符。
    ///
    /// `LFSOURCE` 记录保留受检调用顺序、每条声明内已规范化的 successors 与来源位置；
    /// 描述符的 imports 另按命名空间排序供模块图使用。成功会消费构建器，避免摘要
    /// 派生后继续修改内容。
    ///
    /// # Errors
    ///
    /// 若最终记录长度溢出 `u32` 或超过单模块来源字节上限，则返回资源诊断，不返回
    /// 描述符或部分模块。该方法按值取得 `self`，因此失败也会消费构建器；调用方不能
    /// 在失败后继续追加声明。
    pub fn finish(self) -> Result<SyntheticModule, DiagnosticBundle> {
        let source_record = encode_source_record(
            &self.header,
            &self.imports,
            &self.declarations,
            self.limits
                .value(CompileLimitDimension::SourceBytesPerModule),
        )?;
        let source_record_byte_len = u32::try_from(source_record.len()).map_err(|_| {
            DiagnosticBundle::single(Diagnostic::compile_limit_exceeded(
                CompileLimitDimension::SourceBytesPerModule,
                self.limits
                    .value(CompileLimitDimension::SourceBytesPerModule),
                u64::try_from(source_record.len()).unwrap_or(u64::MAX),
            ))
        })?;
        let source_document_digest = source_document_digest(&source_record);

        let mut canonical_imports: Vec<_> = self
            .imports
            .iter()
            .map(|record| Arc::clone(&record.namespace))
            .collect();
        canonical_imports.sort_unstable();

        let authoring_namespace_id = self.header.authoring_namespace_id;
        let source_document = SourceDocumentDescriptor {
            source_document_key: self.header.source_document_key,
            source_document_digest,
            source_record_byte_len,
            authoring_namespace_id: Arc::clone(&authoring_namespace_id),
            origin: SourceDocumentOrigin::synthetic(),
        };
        let (source_documents, source_document_set_digest) =
            freeze_source_documents(&authoring_namespace_id, source_document, Vec::new());
        let descriptor = SourceModuleDescriptor {
            authoring_namespace_id,
            source_language: SourceLanguage::SyntheticDsl,
            source_document_set_digest,
            source_document_set_digest_version: SOURCE_DOCUMENT_SET_DIGEST_VERSION,
            frontend_version: SYNTHETIC_FRONTEND_VERSION,
            frontend_options_digest: self.header.frontend_options_digest,
            generator_build_id: self.header.generator_build_id,
            parameters_and_inputs_digest: self.header.parameters_and_inputs_digest,
            random_seed: self.header.random_seed,
            provenance: self.header.provenance,
            imports: canonical_imports.into_boxed_slice(),
        };

        Ok(SyntheticModule {
            admitted: AdmittedOfficialModule::new(
                TypedAstModule {
                    descriptor,
                    declaration_span: self.header.declaration_span.into(),
                    source_documents,
                    imports: self.imports.into_boxed_slice(),
                    geometry_profiles: None,
                    road_alignments: Box::default(),
                    declarations: self.declarations.into_boxed_slice(),
                },
                ModuleResourceCounts {
                    source_bytes: u64::from(source_record_byte_len),
                    declaration_count: self.declaration_count,
                    typed_ast_record_count: self.typed_ast_record_count,
                    reference_count: self.reference_count,
                    relation_occurrence_count: self.relation_occurrence_count,
                    identity_field_occurrence_count: self.identity_field_occurrence_count,
                    symbol_count: self.symbol_count,
                    string_item_count: self.string_item_count,
                    string_bytes: self.string_bytes,
                    maneuver_gate_count: self.maneuver_gate_count,
                    waiting_zone_count: self.waiting_zone_count,
                    route_occurrence_count: self.route_occurrence_count,
                    geometry_point_count: self.geometry_point_count,
                    geometry_source_range_count: 0,
                    controlled_live_bytes: self
                        .controlled_string_bytes
                        .saturating_add(self.controlled_structural_bytes)
                        .saturating_add(size_bytes::<SourceDocumentDescriptor>(1)),
                    admission_peak_live_bytes: self
                        .controlled_string_bytes
                        .saturating_add(self.controlled_structural_bytes)
                        .saturating_add(size_bytes::<SourceDocumentDescriptor>(1)),
                },
            ),
        })
    }
}

pub struct SyntheticModule {
    pub(super) admitted: AdmittedOfficialModule,
}

impl SyntheticModule {
    /// 返回由同一模块内容原子派生的只读描述符。
    #[must_use]
    pub const fn descriptor(&self) -> &SourceModuleDescriptor {
        &self.admitted.typed_ast().descriptor
    }

    /// 按文档键 UTF-8 字节序遍历该逻辑模块的来源文档描述符。
    pub fn source_documents(&self) -> impl ExactSizeIterator<Item = &SourceDocumentDescriptor> {
        self.admitted.source_documents.iter()
    }
}

fn header_resident_string_bytes(header: &SourceModuleHeader) -> u64 {
    [
        header.authoring_namespace_id.len(),
        header.source_document_key.len(),
    ]
    .into_iter()
    .try_fold(0_u64, |total, value| {
        total.checked_add(u64::try_from(value).ok()?)
    })
    .unwrap_or(u64::MAX)
}

fn header_controlled_string_bytes(header: &SourceModuleHeader) -> u64 {
    [
        header.authoring_namespace_id.len(),
        header.source_document_key.len(),
        header.generator_build_id.len(),
        header.provenance.len(),
    ]
    .into_iter()
    .try_fold(0_u64, |total, value| {
        total.checked_add(u64::try_from(value).ok()?)
    })
    .unwrap_or(u64::MAX)
}

pub(super) fn limit_diagnostic(
    limits: &CompileLimits,
    dimension: CompileLimitDimension,
    observed: u64,
    primary_span: Option<SourceSpan>,
    stable_key: Option<Box<str>>,
) -> Option<Diagnostic> {
    let limit = limits.value(dimension);
    (observed > limit).then(|| {
        Diagnostic::compile_limit_exceeded_at(dimension, limit, observed, primary_span, stable_key)
    })
}

fn push_limit_if_exceeded(
    diagnostics: &mut DiagnosticCollector,
    limits: &CompileLimits,
    dimension: CompileLimitDimension,
    observed: u64,
    primary_span: Option<SourceSpan>,
    stable_key: Option<Box<str>>,
) {
    if let Some(diagnostic) =
        limit_diagnostic(limits, dimension, observed, primary_span, stable_key)
    {
        diagnostics.push(diagnostic);
    }
}

fn normalize_spatial_zero(value: f32) -> f32 {
    if value == 0.0 { 0.0 } else { value }
}

fn reference_spelling_bytes<K: laneflow_static_contract::EntityKindMarker>(
    reference: &OwnedEntityReference<K>,
) -> u64 {
    reference_spelling_parts_bytes(&reference.module_namespace, reference.declaration_key())
}

fn reference_spelling_parts_bytes(module_namespace: &str, declaration_key: &str) -> u64 {
    u64::try_from(module_namespace.len())
        .unwrap_or(u64::MAX)
        .saturating_add(1)
        .saturating_add(u64::try_from(declaration_key.len()).unwrap_or(u64::MAX))
}

fn admit_vehicle_profile_scalars(
    input: VehicleProfileInput<'_>,
    span: &SourceSpan,
) -> Result<AdmittedIidmProfile, DiagnosticBundle> {
    let iidm = input.iidm;
    let length_mm = closed_millimetres(
        iidm.length_meters,
        MIN_VEHICLE_LENGTH_MM,
        MAX_VEHICLE_LENGTH_MM,
    )
    .map_err(|violation| {
        DiagnosticBundle::single(Diagnostic::invalid_vehicle_profile_value(
            input.vehicle_profile_key,
            "length",
            iidm.length_meters,
            violation,
            span.clone(),
        ))
    })?;
    let desired_speed_mm_s = SpeedLimit::try_new(iidm.desired_speed_meters_per_second)
        .map(|limit| limit.millimetres_per_second())
        .map_err(|violation| {
            DiagnosticBundle::single(Diagnostic::invalid_vehicle_profile_value(
                input.vehicle_profile_key,
                "desiredSpeed",
                iidm.desired_speed_meters_per_second,
                violation,
                span.clone(),
            ))
        })?;
    let min_gap_mm =
        closed_millimetres(iidm.min_gap_meters, 0, MAX_MIN_GAP_MM).map_err(|violation| {
            DiagnosticBundle::single(Diagnostic::invalid_vehicle_profile_value(
                input.vehicle_profile_key,
                "minGap",
                iidm.min_gap_meters,
                violation,
                span.clone(),
            ))
        })?;
    let time_headway_seconds =
        admit_time_headway(iidm.time_headway_seconds).map_err(|violation| {
            DiagnosticBundle::single(Diagnostic::invalid_vehicle_profile_value(
                input.vehicle_profile_key,
                "timeHeadway",
                iidm.time_headway_seconds,
                violation,
                span.clone(),
            ))
        })?;
    let max_acceleration_meters_per_second_squared =
        admit_accel(iidm.max_acceleration_meters_per_second_squared).map_err(|violation| {
            DiagnosticBundle::single(Diagnostic::invalid_vehicle_profile_value(
                input.vehicle_profile_key,
                "maxAcceleration",
                iidm.max_acceleration_meters_per_second_squared,
                violation,
                span.clone(),
            ))
        })?;
    let comfortable_deceleration_meters_per_second_squared = admit_accel(
        iidm.comfortable_deceleration_meters_per_second_squared,
    )
    .map_err(|violation| {
        DiagnosticBundle::single(Diagnostic::invalid_vehicle_profile_value(
            input.vehicle_profile_key,
            "comfortableDeceleration",
            iidm.comfortable_deceleration_meters_per_second_squared,
            violation,
            span.clone(),
        ))
    })?;
    let emergency_deceleration_meters_per_second_squared = admit_accel(
        iidm.emergency_deceleration_meters_per_second_squared,
    )
    .map_err(|violation| {
        DiagnosticBundle::single(Diagnostic::invalid_vehicle_profile_value(
            input.vehicle_profile_key,
            "emergencyDeceleration",
            iidm.emergency_deceleration_meters_per_second_squared,
            violation,
            span.clone(),
        ))
    })?;
    if emergency_deceleration_meters_per_second_squared
        < comfortable_deceleration_meters_per_second_squared
    {
        return Err(DiagnosticBundle::single(
            Diagnostic::invalid_vehicle_profile_deceleration_order(
                input.vehicle_profile_key,
                iidm.comfortable_deceleration_meters_per_second_squared,
                iidm.emergency_deceleration_meters_per_second_squared,
                span.clone(),
            ),
        ));
    }
    Ok(AdmittedIidmProfile {
        length_mm,
        desired_speed_mm_s,
        min_gap_mm,
        time_headway_seconds,
        max_acceleration_meters_per_second_squared,
        comfortable_deceleration_meters_per_second_squared,
        emergency_deceleration_meters_per_second_squared,
    })
}

fn admit_parking_space_scalars(
    input: ParkingSpaceInput<'_>,
    span: &SourceSpan,
    diagnostic_limit: u64,
) -> Result<(u32, u32, AdmittedParkingGeometry), DiagnosticBundle> {
    let mut diagnostics = DiagnosticCollector::new(diagnostic_limit);
    let entry_progress_mm = match admit_parking_progress(input.entry.progress_meters) {
        Ok(progress_mm) => Some(progress_mm),
        Err(_) => {
            diagnostics.push(Diagnostic::invalid_parking_anchor_progress(
                input.parking_space_key,
                ParkingAnchorRole::Entry,
                input.entry.lane_edge.declaration_key(),
                input.entry.progress_meters,
                0.0,
                1,
                0,
                span.clone(),
            ));
            None
        }
    };
    let exit_progress_mm = match admit_parking_progress(input.exit.progress_meters) {
        Ok(progress_mm) => Some(progress_mm),
        Err(_) => {
            diagnostics.push(Diagnostic::invalid_parking_anchor_progress(
                input.parking_space_key,
                ParkingAnchorRole::Exit,
                input.exit.lane_edge.declaration_key(),
                input.exit.progress_meters,
                0.0,
                1,
                0,
                span.clone(),
            ));
            None
        }
    };
    let geometry = input.geometry;
    let mut admitted = AdmittedParkingGeometry {
        lateral_offset_mm: 0,
        heading_offset_radians: 0.0,
        length_mm: 0,
        width_mm: 0,
    };
    let mut geometry_ok = true;
    for (field, value, result) in [
        (
            ParkingGeometryField::LateralOffsetMeters,
            geometry.lateral_offset_meters,
            parking_lateral_mm(geometry.lateral_offset_meters).map(|mm| {
                admitted.lateral_offset_mm = mm;
            }),
        ),
        (
            ParkingGeometryField::HeadingOffsetRadians,
            geometry.heading_offset_radians,
            parking_heading_f32(geometry.heading_offset_radians).map(|heading| {
                admitted.heading_offset_radians = heading;
            }),
        ),
        (
            ParkingGeometryField::LengthMeters,
            geometry.length_meters,
            parking_extent_mm(geometry.length_meters).map(|mm| {
                admitted.length_mm = mm;
            }),
        ),
        (
            ParkingGeometryField::WidthMeters,
            geometry.width_meters,
            parking_extent_mm(geometry.width_meters).map(|mm| {
                admitted.width_mm = mm;
            }),
        ),
    ] {
        if let Err(violation) = result {
            geometry_ok = false;
            diagnostics.push(Diagnostic::invalid_parking_space_geometry(
                input.parking_space_key,
                field,
                value,
                violation,
                span.clone(),
            ));
        }
    }
    if diagnostics.is_empty() {
        Ok((
            entry_progress_mm.expect("progress admitted"),
            exit_progress_mm.expect("progress admitted"),
            admitted,
        ))
    } else {
        let _ = (geometry_ok, entry_progress_mm, exit_progress_mm);
        Err(diagnostics.finish())
    }
}

fn admit_parking_progress(value: f64) -> Result<u32, ScalarViolation> {
    millimetres_from_si(value).ok_or(if value.is_finite() {
        ScalarViolation::QuantizeFailed
    } else {
        ScalarViolation::NotFinite
    })
}

fn parking_extent_mm(value: f64) -> Result<u32, ParkingGeometryViolation> {
    if !value.is_finite() {
        return Err(ParkingGeometryViolation::NotFinite);
    }
    let Some(actual_mm) = millimetres_from_si(value) else {
        return Err(ParkingGeometryViolation::QuantizeFailed);
    };
    if actual_mm < MIN_VEHICLE_LENGTH_MM || actual_mm > MAX_VEHICLE_LENGTH_MM {
        return Err(ParkingGeometryViolation::OutsideClosedMillimetreRange {
            min_mm: MIN_VEHICLE_LENGTH_MM,
            max_mm: MAX_VEHICLE_LENGTH_MM,
            actual_mm,
        });
    }
    Ok(actual_mm)
}

fn parking_lateral_mm(value: f64) -> Result<i32, ParkingGeometryViolation> {
    if !value.is_finite() {
        return Err(ParkingGeometryViolation::NotFinite);
    }
    let Some(actual_mm) = millimetres_i32_from_si(value) else {
        return Err(ParkingGeometryViolation::QuantizeFailed);
    };
    let actual_abs_mm = actual_mm.unsigned_abs();
    if actual_abs_mm < MIN_PARKING_LATERAL_OFFSET_ABS_MM
        || actual_abs_mm > MAX_PARKING_LATERAL_OFFSET_ABS_MM
    {
        return Err(
            ParkingGeometryViolation::AbsoluteOutsideClosedMillimetreRange {
                min_abs_mm: MIN_PARKING_LATERAL_OFFSET_ABS_MM,
                max_abs_mm: MAX_PARKING_LATERAL_OFFSET_ABS_MM,
                actual_abs_mm,
            },
        );
    }
    Ok(actual_mm)
}

fn parking_heading_f32(value: f64) -> Result<f32, ParkingGeometryViolation> {
    let Some(heading) = heading_f32_from_si(value) else {
        return Err(if value.is_finite() {
            ParkingGeometryViolation::QuantizeFailed
        } else {
            ParkingGeometryViolation::NotFinite
        });
    };
    if heading_f32_in_legal_closure(heading) {
        Ok(heading)
    } else {
        Err(ParkingGeometryViolation::OutsideHalfOpenRange {
            minimum_inclusive_bits: HEADING_MINUS_PI_F32_BITS,
            maximum_exclusive_bits: HEADING_PLUS_PI_F32_BITS,
        })
    }
}

fn admit_time_headway(value: f64) -> Result<f32, ScalarViolation> {
    if !value.is_finite() {
        return Err(ScalarViolation::NotFinite);
    }
    let quantized = value as f32;
    if !quantized.is_finite() || quantized <= 0.0 {
        return Err(ScalarViolation::NotGreaterThan {
            exclusive_minimum_bits: 0.0_f64.to_bits(),
        });
    }
    if quantized > MAX_TIME_HEADWAY_SECONDS {
        return Err(ScalarViolation::NotAtMost {
            inclusive_maximum_bits: f64::from(MAX_TIME_HEADWAY_SECONDS).to_bits(),
        });
    }
    Ok(quantized)
}

fn admit_accel(value: f64) -> Result<f32, ScalarViolation> {
    if !value.is_finite() {
        return Err(ScalarViolation::NotFinite);
    }
    let quantized = value as f32;
    if !quantized.is_finite()
        || quantized < MIN_ACCEL_METERS_PER_SECOND_SQUARED
        || quantized > MAX_ACCEL_METERS_PER_SECOND_SQUARED
    {
        return Err(ScalarViolation::OutsideClosedF32Range {
            min_bits: MIN_ACCEL_METERS_PER_SECOND_SQUARED.to_bits(),
            max_bits: MAX_ACCEL_METERS_PER_SECOND_SQUARED.to_bits(),
        });
    }
    Ok(quantized)
}

//! Geometry 文档前端的封闭生产配置与借用输入。
//!
//! 本模块只建立 #296 G1 冻结的公共构造面基础。解析器、numeric freeze 与共同
//! admission 继续保持 crate 私有，并且不得从这些配置档之外接受任意浮点容差。

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use laneflow_static_contract::{
    AccessEffect, CanonicalFrameKind, EntityKind, EntityKindMarker, FacilityBandKind, JunctionKind,
    LaneEdgeKind, LaneGroupKind, ManeuverGateKind, ManeuverPathKind, MovementKind, ParkingAreaKind,
    ParticipantClassKind, RoadCorridorKind, RoadSectionKind, SignalAspect, SignalGroupKind,
    StopLineKind,
};
use sha2::{Digest, Sha256};

use crate::declaration::{
    AccessRuleDeclaration, AuthoringLaneDeclaration, CanonicalFrameDeclaration, DeclarationHeader,
    EdgeLength, FacilityBandDeclaration, FacilityKindCategory, FacilityKindViolation,
    GeometryConnectionIntent, GeometryCrossSectionSpanIntent, GeometryInternalEdgeIntent,
    GeometryOffsetIntent, GeometryOffsetIntentKind, GeometryReferenceLineIntent,
    IidmVehicleProfileInput, JunctionDeclaration, LaneEdgeDeclaration, LaneEdgeGeometryDeclaration,
    LaneGroupDeclaration, ManeuverGateDeclaration, ManeuverPathDeclaration, MovementDeclaration,
    OwnedAccessRegulation, OwnedAccessRuleTarget, OwnedCorridorElementReference,
    OwnedEntityReference, OwnedSignalControl, ParkingAreaDeclaration, ParkingLaneAnchorDeclaration,
    ParkingSpaceDeclaration, ParkingSpaceGeometryInput, ParticipantClassDeclaration,
    RoadCorridorDeclaration, RoadSectionDeclaration, SignalControllerDeclaration,
    SignalGroupDeclaration, SignalGroupStateDeclaration, SignalPhaseDeclaration, SpeedLimit,
    StaticRouteDeclaration, StopLineDeclaration, TypedAstDeclaration, VehicleProfileDeclaration,
    WaitingZoneDeclaration, facility_kind_category, validate_vehicle_profile_scalars,
};
use crate::source::external_token_violation;
use crate::{
    CompileLimitDimension, CompileLimits, Diagnostic, DiagnosticBundle, GeometryDocumentViolation,
    SourceHeaderField, SourceSpan,
};

use self::json::{ByteSpan, JsonErrorKind, LineIndex, StageScratchMeter};
use self::schema::{
    ParsedCurveSegment, ParsedGeometryDocument, RawNumber, SchemaError, SchemaErrorKind,
    SpannedString,
};
use super::admission::{AdmittedOfficialModule, ImportRecord, TypedAstModule};
use super::descriptor::{
    SOURCE_DOCUMENT_SET_DIGEST_VERSION, SourceDocumentDescriptor, SourceDocumentOrigin,
    SourceLanguage, SourceModuleDescriptor, freeze_source_documents, source_document_digest,
};
use super::resources::{ModuleResourceCounts, size_bytes};

mod json;
mod schema;

// 几何 MIR 切片（crate::hir）需要命名冻结载荷曲线与方向档；schema 模块本身保持私有。
pub(crate) use schema::{FrozenCanonicalPoint, LateralIntentKind};

#[allow(
    dead_code,
    reason = "consumed by the following Geometry descriptor slice"
)]
const DIRECT_INPUTS_PREIMAGE: &[u8] = b"laneflow.geometry.direct.inputs.v1\0";
#[allow(
    dead_code,
    reason = "consumed by the following Geometry descriptor slice"
)]
const DIRECT_FRONTEND_OPTIONS_PREIMAGE: &[u8] = b"laneflow.geometry.direct.frontend-options.v1\0";
#[allow(
    dead_code,
    reason = "consumed by the following Geometry descriptor slice"
)]
const FRONTEND_OPTIONS_MAGIC: &[u8] = b"laneflow.geometry.frontend-options.v1\0";

/// 首版 Geometry 文档前端版本。
pub const GEOMETRY_FRONTEND_VERSION: u32 = 1;

/// Geometry authoring 曲线到规范运行时折线的总位置误差配置档。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(u8)]
pub enum GeometryAccuracyProfile {
    Fine2Cm = 1,
    Balanced5Cm = 2,
    Compact10Cm = 3,
}

impl GeometryAccuracyProfile {
    /// 返回进入描述符、诊断与校准工件的稳定 ASCII 名称。
    #[must_use]
    pub const fn stable_name(self) -> &'static str {
        match self {
            Self::Fine2Cm => "fine-2cm-v1",
            Self::Balanced5Cm => "balanced-5cm-v1",
            Self::Compact10Cm => "compact-10cm-v1",
        }
    }

    /// 返回 authoring/offset evaluator 到最终规范折线的总位置误差上限。
    #[must_use]
    pub const fn max_position_error_meters(self) -> f64 {
        match self {
            Self::Fine2Cm => 0.02,
            Self::Balanced5Cm => 0.05,
            Self::Compact10Cm => 0.10,
        }
    }

    #[allow(
        dead_code,
        reason = "consumed by the following Geometry descriptor slice"
    )]
    pub(super) const fn code(self) -> u8 {
        self as u8
    }

    #[allow(dead_code, reason = "used by the following numeric-freeze slice")]
    pub(super) const fn subdivision_budget_meters(self) -> f64 {
        match self {
            Self::Fine2Cm => 0.01,
            Self::Balanced5Cm => 0.025,
            Self::Compact10Cm => 0.05,
        }
    }
}

/// 最终规范 `f32` 折线的方向跳变配置档。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(u8)]
pub enum GeometryDirectionProfile {
    Smooth1Deg = 1,
    Balanced2Deg = 2,
    Compact5Deg = 3,
}

impl GeometryDirectionProfile {
    /// 返回进入描述符、诊断与校准工件的稳定 ASCII 名称。
    #[must_use]
    pub const fn stable_name(self) -> &'static str {
        match self {
            Self::Smooth1Deg => "smooth-1deg-v1",
            Self::Balanced2Deg => "balanced-2deg-v1",
            Self::Compact5Deg => "compact-5deg-v1",
        }
    }

    /// 返回最终规范 `f32` 相邻弦和相连 edge 首尾弦允许的最大方向跳变。
    #[must_use]
    pub const fn max_runtime_direction_jump_degrees(self) -> f64 {
        match self {
            Self::Smooth1Deg => 1.0,
            Self::Balanced2Deg => 2.0,
            Self::Compact5Deg => 5.0,
        }
    }

    #[allow(
        dead_code,
        reason = "consumed by the following Geometry descriptor slice"
    )]
    pub(super) const fn code(self) -> u8 {
        self as u8
    }

    #[allow(dead_code, reason = "used by the following numeric-freeze slice")]
    pub(super) const fn candidate_cos_squared(self) -> f64 {
        match self {
            Self::Smooth1Deg => f64::from_bits(0x3fef_ff60_4bfa_d7c5),
            Self::Balanced2Deg => f64::from_bits(0x3fef_fd81_3c5f_82b4),
            Self::Compact5Deg => f64::from_bits(0x3fef_f069_da0c_0ad2),
        }
    }

    /// 返回最终规范 `f32` 相邻弦与连接首尾弦方向检查的冻结 `cos²` 阈值（§6.1 最终全角档）。
    pub(crate) const fn runtime_cos_squared(self) -> f64 {
        match self {
            Self::Smooth1Deg => f64::from_bits(0x3fef_fd81_3c5f_82b4),
            Self::Balanced2Deg => f64::from_bits(0x3fef_f605_b8b8_7ffc),
            Self::Compact5Deg => f64::from_bits(0x3fef_c1c5_c640_8e0c),
        }
    }
}

#[allow(
    dead_code,
    reason = "consumed by the following Geometry descriptor slice"
)]
pub(super) fn direct_parameters_and_inputs_digest() -> [u8; 32] {
    Sha256::digest(DIRECT_INPUTS_PREIMAGE).into()
}

#[allow(
    dead_code,
    reason = "consumed by the following Geometry descriptor slice"
)]
pub(super) fn direct_source_frontend_options_digest() -> [u8; 32] {
    Sha256::digest(DIRECT_FRONTEND_OPTIONS_PREIMAGE).into()
}

#[allow(
    dead_code,
    reason = "consumed by the following Geometry descriptor slice"
)]
pub(super) fn frontend_options_digest(
    accuracy_profile: GeometryAccuracyProfile,
    direction_profile: GeometryDirectionProfile,
    source_frontend_options_digest: &[u8; 32],
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(FRONTEND_OPTIONS_MAGIC);
    hasher.update([accuracy_profile.code()]);
    hasher.update([direction_profile.code()]);
    hasher.update(source_frontend_options_digest);
    hasher.finalize().into()
}

/// 一份 Geometry v1 来源文档的借用输入。
///
/// `new` 只组装借用，不解析、不分配、不哈希。调用方提供的显示来源不参与稳定身份、
/// 来源摘要或 LIR 语义。
#[allow(dead_code, reason = "consumed by the following bounded-parser slice")]
pub struct GeometryDocumentInput<'a> {
    source_document_key: &'a str,
    source_bytes: &'a [u8],
    display_source: Option<&'a str>,
}

/// 已完成 Geometry v1 有界 parse/build、尚待 numeric freeze 的模块构建器。
///
/// 构建器只保留紧凑 wire records、byte→行列索引和内容摘要，不保留调用方来源全文。
#[allow(
    dead_code,
    reason = "fields are consumed by the following numeric-freeze slice"
)]
pub struct GeometryModuleBuilder {
    source_document_key: Arc<str>,
    source_document_digest: [u8; 32],
    source_record_byte_len: u32,
    display_source: Option<Arc<str>>,
    accuracy_profile: GeometryAccuracyProfile,
    direction_profile: GeometryDirectionProfile,
    limits: CompileLimits,
    line_index: LineIndex,
    parsed: ParsedGeometryDocument,
}

impl<'a> GeometryDocumentInput<'a> {
    /// 组装一份待解析的 Geometry v1 文档借用。
    #[must_use]
    pub fn new(
        source_document_key: &'a str,
        source_bytes: &'a [u8],
        display_source: Option<&'a str>,
    ) -> Self {
        Self {
            source_document_key,
            source_bytes,
            display_source,
        }
    }

    #[allow(dead_code, reason = "consumed by the following bounded-parser slice")]
    pub(super) const fn source_document_key(&self) -> &'a str {
        self.source_document_key
    }

    #[allow(dead_code, reason = "consumed by the following bounded-parser slice")]
    pub(super) const fn source_bytes(&self) -> &'a [u8] {
        self.source_bytes
    }

    #[allow(dead_code, reason = "consumed by the following bounded-parser slice")]
    pub(super) const fn display_source(&self) -> Option<&'a str> {
        self.display_source
    }
}

impl GeometryModuleBuilder {
    /// 验证来源键与字节边界，精确哈希一次来源全文并完成有界 closed-shape 解析。
    ///
    /// # Errors
    ///
    /// 来源键非法、来源字节超过配置档或 `u32` 描述符边界、JSON/schema 非法，或文档内
    /// `module.documentKey` 与调用方预期键不一致时，返回规范结构化诊断且不保留部分构建器。
    pub fn new(
        input: GeometryDocumentInput<'_>,
        accuracy_profile: GeometryAccuracyProfile,
        direction_profile: GeometryDirectionProfile,
        limits: &CompileLimits,
    ) -> Result<Self, DiagnosticBundle> {
        if let Some(violation) = external_token_violation(
            input.source_document_key(),
            limits.value(CompileLimitDimension::SingleStringBytes),
        ) {
            return Err(DiagnosticBundle::single(
                Diagnostic::invalid_source_header_field(
                    SourceHeaderField::SourceDocumentKey,
                    violation,
                ),
            ));
        }

        let source_byte_len = u64::try_from(input.source_bytes().len()).unwrap_or(u64::MAX);
        let configured_limit = limits.value(CompileLimitDimension::SourceBytesPerModule);
        let effective_limit = configured_limit.min(u64::from(u32::MAX));
        if source_byte_len > effective_limit {
            return Err(DiagnosticBundle::single(
                Diagnostic::compile_limit_exceeded(
                    CompileLimitDimension::SourceBytesPerModule,
                    effective_limit,
                    source_byte_len,
                ),
            ));
        }
        let source_record_byte_len = u32::try_from(source_byte_len)
            .expect("Geometry source byte precheck guarantees u32 descriptor length");

        let source_document_key: Arc<str> = input.source_document_key().into();
        let source_document_digest = source_document_digest(input.source_bytes());
        let line_index = LineIndex::new(input.source_bytes()).map_err(|error| {
            schema_diagnostic(SchemaError::from(error), &source_document_key, None)
        })?;
        let parsed = schema::parse_geometry_document_with_scratch(
            input.source_bytes(),
            limits.value(CompileLimitDimension::StageScratchBytes),
        )
        .map_err(|error| schema_diagnostic(error, &source_document_key, Some(&line_index)))?;

        if parsed.module.document_key.value.as_ref() != input.source_document_key() {
            let primary_span =
                line_index.source_span(&source_document_key, parsed.module.document_key.span);
            return Err(DiagnosticBundle::single(
                Diagnostic::invalid_geometry_document(
                    GeometryDocumentViolation::DocumentKeyMismatch,
                    Some("module.documentKey"),
                    Some(&parsed.module.document_key.value),
                    Some(input.source_document_key()),
                    primary_span,
                ),
            ));
        }

        Ok(Self {
            source_document_key,
            source_document_digest,
            source_record_byte_len,
            display_source: input.display_source().map(Arc::from),
            accuracy_profile,
            direction_profile,
            limits: limits.clone(),
            line_index,
            parsed,
        })
    }

    #[allow(dead_code, reason = "called by the following public finish slice")]
    fn freeze_reference_lines(
        &self,
    ) -> Result<Box<[schema::FrozenRoadReference]>, DiagnosticBundle> {
        let mut scratch =
            StageScratchMeter::new(self.limits.value(CompileLimitDimension::StageScratchBytes));
        schema::freeze_reference_lines(
            &self.parsed,
            self.accuracy_profile,
            self.direction_profile,
            &mut scratch,
        )
        .map_err(|error| {
            let primary_span = self
                .line_index
                .source_span(&self.source_document_key, error.span);
            DiagnosticBundle::single(Diagnostic::invalid_geometry_document(
                GeometryDocumentViolation::FieldValue,
                Some(error.field),
                Some(&format!("{:?}", error.violation)),
                Some("finite deterministic Geometry v1 reference curve"),
                primary_span,
            ))
        })
    }

    #[allow(dead_code, reason = "called by the following public finish slice")]
    fn freeze_stationing(&self) -> Result<Box<[schema::FrozenRoadStationing]>, DiagnosticBundle> {
        let mut scratch =
            StageScratchMeter::new(self.limits.value(CompileLimitDimension::StageScratchBytes));
        schema::freeze_stationing(&self.parsed, self.direction_profile, &mut scratch).map_err(
            |error| {
                let primary_span = self
                    .line_index
                    .source_span(&self.source_document_key, error.span);
                DiagnosticBundle::single(Diagnostic::invalid_geometry_document(
                    GeometryDocumentViolation::FieldValue,
                    Some(error.field),
                    Some(&format!("{:?}", error.violation)),
                    Some("complete deterministic Geometry v1 station coverage"),
                    primary_span,
                ))
            },
        )
    }

    #[allow(dead_code, reason = "called by the following public finish slice")]
    fn freeze_cross_section_layouts(
        &self,
    ) -> Result<Box<[schema::FrozenCrossSectionLayout]>, DiagnosticBundle> {
        let mut scratch =
            StageScratchMeter::new(self.limits.value(CompileLimitDimension::StageScratchBytes));
        schema::freeze_cross_section_layouts(&self.parsed, &mut scratch).map_err(|error| {
            let primary_span = self
                .line_index
                .source_span(&self.source_document_key, error.span);
            DiagnosticBundle::single(Diagnostic::invalid_geometry_document(
                GeometryDocumentViolation::FieldValue,
                Some(error.field),
                Some(&format!("{:?}", error.violation)),
                Some("valid Geometry v1 reference lane and lateral prefix layout"),
                primary_span,
            ))
        })
    }

    #[allow(dead_code, reason = "called by the following public finish slice")]
    fn freeze_lateral_curves(
        &self,
        stationing: &[schema::FrozenRoadStationing],
        layouts: &[schema::FrozenCrossSectionLayout],
    ) -> Result<Box<[schema::FrozenLateralCurve]>, DiagnosticBundle> {
        let mut scratch =
            StageScratchMeter::new(self.limits.value(CompileLimitDimension::StageScratchBytes));
        schema::freeze_lateral_curves(
            &self.parsed,
            stationing,
            layouts,
            self.accuracy_profile,
            self.direction_profile,
            &mut scratch,
        )
        .map_err(|error| {
            let primary_span = self
                .line_index
                .source_span(&self.source_document_key, error.span);
            DiagnosticBundle::single(Diagnostic::invalid_geometry_document(
                GeometryDocumentViolation::FieldValue,
                Some(error.field),
                Some(&format!("{:?}", error.violation)),
                Some("bounded deterministic Geometry v1 canonical lateral curve"),
                primary_span,
            ))
        })
    }

    fn freeze_geometry_payload(&self) -> Result<schema::FrozenGeometryPayload, DiagnosticBundle> {
        let point_limit = self.limits.value(CompileLimitDimension::GeometryPointCount);
        let scratch_limit = self.limits.value(CompileLimitDimension::StageScratchBytes);
        let mut scratch = StageScratchMeter::new(scratch_limit);
        schema::freeze_geometry_payload(
            &self.parsed,
            self.accuracy_profile,
            self.direction_profile,
            point_limit,
            &mut scratch,
        )
        .map_err(|error| {
            if error.violation == schema::NumericFreezeViolation::GeometryPointLimitExceeded {
                return DiagnosticBundle::single(Diagnostic::compile_limit_exceeded(
                    CompileLimitDimension::GeometryPointCount,
                    point_limit,
                    point_limit.saturating_add(1),
                ));
            }
            if error.violation == schema::NumericFreezeViolation::StageScratchExceeded {
                return DiagnosticBundle::single(Diagnostic::compile_limit_exceeded(
                    CompileLimitDimension::StageScratchBytes,
                    scratch_limit,
                    scratch_limit.saturating_add(1),
                ));
            }
            let primary_span = self
                .line_index
                .source_span(&self.source_document_key, error.span);
            DiagnosticBundle::single(Diagnostic::invalid_geometry_document(
                GeometryDocumentViolation::FieldValue,
                Some(error.field),
                Some(&format!("{:?}", error.violation)),
                Some("complete bounded Geometry v1 canonical geometry payload"),
                primary_span,
            ))
        })
    }

    /// 原子完成模块头与键检查、numeric freeze、Typed AST 降阶与资源上限校验，
    /// 产出可交给共同 admission 的 Geometry 模块。
    ///
    /// 成功会消费构建器，避免摘要派生后继续修改内容；失败不返回部分模块，
    /// 也不把尚未受检的声明或点数据交给调用方。
    ///
    /// # Errors
    ///
    /// 模块头非法、键非法或重复、引用拼写或导入边界非法、junction internal edge
    /// 声明或连接引用不满足 §4.4、numeric freeze 失败，或任一资源维度超过配置档时，
    /// 返回规范结构化诊断。
    pub fn finish(self) -> Result<GeometryModule, DiagnosticBundle> {
        let single_string_limit = self.limits.value(CompileLimitDimension::SingleStringBytes);
        let span_of = |span: ByteSpan| self.line_index.source_span(&self.source_document_key, span);

        // ① 模块头、导入、键分组与 approach 引用检查；先到先得单诊断。
        let header = self.finish_header(single_string_limit, &span_of)?;
        // ② numeric freeze 恰好一次；GeometryPointCount 单模块上限在 freeze 内闭合。
        let payload = self.freeze_geometry_payload()?;
        // ③ Typed AST 降阶；长度从冻结折线按 §6.1 的固定约定派生，§4.4 的
        // internal edge 声明校验与连接引用解析在此闭合。
        let resolver = ReferenceResolver {
            namespace: &header.namespace,
            imports: &header.imports,
            import_index: &header.import_index,
            single_string_limit,
        };
        let declarations = lower_typed_ast(
            &self.parsed,
            &header,
            &payload,
            &resolver,
            single_string_limit,
            &span_of,
        )?;
        // ④ 模块级资源计数与各维度上限。
        let counts = finish_resource_counts(
            &self.parsed,
            &payload,
            &declarations,
            &header,
            &self.source_document_key,
            self.source_record_byte_len,
        );
        let declaration_span = span_of(self.parsed.module.span);
        check_finish_limits(
            &self.limits,
            &counts,
            u64::try_from(header.imports.len()).unwrap_or(u64::MAX),
            &header.namespace,
            &declaration_span,
        )?;

        // §9.2 前端计数与描述符同批原子冻结；随后解析树与载荷按所有权移交。
        let module_counts = finish_module_counts(&self.parsed, &payload, &counts);

        // ⑤ 组装描述符、来源文档与不可分的冻结几何载荷。
        let frontend_options_digest = frontend_options_digest(
            self.accuracy_profile,
            self.direction_profile,
            &header.provenance.source_frontend_options_digest,
        );
        let Self {
            source_document_key,
            source_document_digest,
            source_record_byte_len,
            display_source,
            accuracy_profile,
            direction_profile,
            limits: _,
            line_index: _,
            parsed: _,
        } = self;
        let FinishHeader {
            namespace,
            imports,
            provenance,
            import_index: _,
            approaches: _,
        } = header;
        let source_document = SourceDocumentDescriptor {
            source_document_key,
            source_document_digest,
            source_record_byte_len,
            authoring_namespace_id: Arc::clone(&namespace),
            origin: SourceDocumentOrigin::geometry(display_source),
        };
        let (source_documents, source_document_set_digest) =
            freeze_source_documents(&namespace, source_document, Vec::new());
        let mut canonical_imports: Vec<_> = imports
            .iter()
            .map(|record| Arc::clone(&record.namespace))
            .collect();
        canonical_imports.sort_unstable();
        let descriptor = SourceModuleDescriptor {
            authoring_namespace_id: namespace,
            source_language: SourceLanguage::GeometryDocument,
            source_document_set_digest,
            source_document_set_digest_version: SOURCE_DOCUMENT_SET_DIGEST_VERSION,
            frontend_version: GEOMETRY_FRONTEND_VERSION,
            frontend_options_digest,
            generator_build_id: provenance.generator_build_id,
            parameters_and_inputs_digest: provenance.parameters_and_inputs_digest,
            random_seed: provenance.random_seed,
            provenance: provenance.description,
            imports: canonical_imports.into_boxed_slice(),
        };
        Ok(GeometryModule {
            admitted: AdmittedOfficialModule::new_geometry(
                TypedAstModule {
                    descriptor,
                    declaration_span,
                    source_documents,
                    imports,
                    declarations: declarations.into_boxed_slice(),
                },
                counts,
                FrozenGeometryModulePayload {
                    frozen: payload,
                    accuracy_profile,
                    direction_profile,
                },
            ),
            accuracy_profile,
            direction_profile,
            counts: module_counts,
        })
    }

    /// 完成 ① 的模块头、导入、键分组查重与 junction approach 引用解析。
    fn finish_header(
        &self,
        single_string_limit: u64,
        span_of: &dyn Fn(ByteSpan) -> SourceSpan,
    ) -> Result<FinishHeader, DiagnosticBundle> {
        let module = &self.parsed.module;
        let namespace_value = module.namespace.value.as_ref();
        if let Some(_violation) = external_token_violation(namespace_value, single_string_limit) {
            return Err(DiagnosticBundle::single(
                Diagnostic::invalid_geometry_document(
                    GeometryDocumentViolation::FieldValue,
                    Some("module.namespace"),
                    Some(namespace_value),
                    Some("valid external token"),
                    span_of(module.namespace.span),
                ),
            ));
        }
        let namespace: Arc<str> = Arc::from(namespace_value);

        let mut imports = Vec::with_capacity(module.imports.len());
        let mut import_index = HashMap::with_capacity(module.imports.len());
        for import in &module.imports {
            let value = import.value.as_ref();
            if let Some(_violation) = external_token_violation(value, single_string_limit) {
                return Err(DiagnosticBundle::single(
                    Diagnostic::invalid_geometry_document(
                        GeometryDocumentViolation::FieldValue,
                        Some("module.imports"),
                        Some(value),
                        Some("valid external token"),
                        span_of(import.span),
                    ),
                ));
            }
            if value == namespace_value {
                return Err(DiagnosticBundle::single(Diagnostic::import_cycle(
                    &[value],
                    Box::new([span_of(import.span)]),
                )));
            }
            import_index.insert(Arc::from(value), imports.len());
            imports.push(ImportRecord {
                namespace: Arc::from(value),
                span: span_of(import.span),
            });
        }

        validate_finish_keys(&self.parsed, single_string_limit, span_of)?;
        let resolver = ReferenceResolver {
            namespace: &namespace,
            imports: &imports,
            import_index: &import_index,
            single_string_limit,
        };
        let mut approaches = Vec::with_capacity(self.parsed.junctions.len());
        for junction in &self.parsed.junctions {
            let mut approach_set = BTreeSet::new();
            for approach in &junction.approach_edges {
                let (namespace, key) =
                    resolver.resolve_parts(approach, "junctions[].approachEdges", span_of)?;
                approach_set.insert((namespace, Arc::from(key)));
            }
            approaches.push(approach_set);
        }
        let provenance = self.finish_provenance(single_string_limit, span_of)?;
        Ok(FinishHeader {
            namespace,
            imports: imports.into_boxed_slice(),
            import_index,
            approaches: approaches.into_boxed_slice(),
            provenance,
        })
    }

    /// 校验并映射 §2.3 的 provenance 登记字段。
    fn finish_provenance(
        &self,
        single_string_limit: u64,
        span_of: &dyn Fn(ByteSpan) -> SourceSpan,
    ) -> Result<FinishProvenance, DiagnosticBundle> {
        match &self.parsed.module.provenance {
            schema::ParsedProvenance::Direct { description } => Ok(FinishProvenance {
                generator_build_id: Arc::from("laneflow-geometry-direct-v1"),
                parameters_and_inputs_digest: direct_parameters_and_inputs_digest(),
                random_seed: None,
                source_frontend_options_digest: direct_source_frontend_options_digest(),
                description: Arc::from(description.value.as_ref()),
            }),
            schema::ParsedProvenance::Generated {
                generator_build_id,
                parameters_and_inputs_digest,
                frontend_options_digest,
                random_seed,
                description,
            } => {
                let value = generator_build_id.value.as_ref();
                if let Some(_violation) = external_token_violation(value, single_string_limit) {
                    return Err(DiagnosticBundle::single(
                        Diagnostic::invalid_geometry_document(
                            GeometryDocumentViolation::FieldValue,
                            Some("module.provenance.generatorBuildId"),
                            Some(value),
                            Some("valid external token"),
                            span_of(generator_build_id.span),
                        ),
                    ));
                }
                Ok(FinishProvenance {
                    generator_build_id: Arc::from(value),
                    parameters_and_inputs_digest: *parameters_and_inputs_digest,
                    random_seed: *random_seed,
                    source_frontend_options_digest: *frontend_options_digest,
                    description: Arc::from(description.value.as_ref()),
                })
            }
        }
    }
}

/// Geometry 前端随模块冻结的几何载荷；与 Typed AST 在共同准入前不可分。
pub(crate) struct FrozenGeometryModulePayload {
    /// 冻结的 lane/facility 规范折线集合与精确点数。
    pub(crate) frozen: schema::FrozenGeometryPayload,
    /// 冻结所用的总位置误差配置档。
    pub(crate) accuracy_profile: GeometryAccuracyProfile,
    /// 冻结所用的方向跳变配置档。
    pub(crate) direction_profile: GeometryDirectionProfile,
}

/// 已完成受检构造、numeric freeze 与 Typed AST 降阶的 Geometry 来源模块。
pub struct GeometryModule {
    pub(in crate::module) admitted: AdmittedOfficialModule,
    accuracy_profile: GeometryAccuracyProfile,
    direction_profile: GeometryDirectionProfile,
    counts: GeometryModuleCounts,
}

impl GeometryModule {
    /// 返回由同一模块内容原子派生的只读描述符。
    #[must_use]
    pub const fn descriptor(&self) -> &SourceModuleDescriptor {
        &self.admitted.typed_ast().descriptor
    }

    /// 按文档键 UTF-8 字节序遍历该逻辑模块的来源文档描述符。
    pub fn source_documents(&self) -> impl ExactSizeIterator<Item = &SourceDocumentDescriptor> {
        self.admitted.source_documents.iter()
    }

    /// 返回冻结该模块几何载荷的总位置误差配置档。
    #[must_use]
    pub const fn accuracy_profile(&self) -> GeometryAccuracyProfile {
        self.accuracy_profile
    }

    /// 返回冻结该模块几何载荷的方向跳变配置档。
    #[must_use]
    pub const fn direction_profile(&self) -> GeometryDirectionProfile {
        self.direction_profile
    }

    /// 返回 `finish` 随 numeric freeze 与 Typed AST 降阶原子冻结的只读前端计数。
    #[must_use]
    pub const fn counts(&self) -> &GeometryModuleCounts {
        &self.counts
    }
}

/// §9.2 workload manifest 消费的横向 offset 曲线 |中心偏移| 分布桶。
///
/// 同一 |中心偏移| 的 f64 位模式共享一桶；桶序为位模式升序，与声明顺序无关。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GeometryOffsetCurveBucket {
    absolute_offset_meters_bits: u64,
    curve_count: u64,
}

impl GeometryOffsetCurveBucket {
    /// 返回 |中心偏移|（米）的 `f64` 位模式。
    #[must_use]
    pub const fn absolute_offset_meters_bits(self) -> u64 {
        self.absolute_offset_meters_bits
    }

    /// 返回该位模式覆盖的横向 offset 曲线数。
    #[must_use]
    pub const fn curve_count(self) -> u64 {
        self.curve_count
    }
}

/// Geometry 模块在 `finish` 原子冻结的只读前端计数。
///
/// 计数来自封闭解析、numeric freeze 与 Typed AST 降阶的精确结果，不从 HIR/LIR 反推；
/// 它只服务 §9.2 的 workload manifest 生成与独立重算核对，不改变模块语义。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeometryModuleCounts {
    declaration_count: u64,
    reference_count: u64,
    relation_occurrence_count: u64,
    line_segment_count: u64,
    cubic_segment_count: u64,
    control_point_count: u64,
    offset_curve_count: u64,
    canonical_point_count: u64,
    absolute_offset_distribution: Box<[GeometryOffsetCurveBucket]>,
}

impl GeometryModuleCounts {
    /// 返回该模块降阶后的共同 Typed AST 声明数。
    #[must_use]
    pub const fn declaration_count(&self) -> u64 {
        self.declaration_count
    }

    /// 返回该模块 Typed AST 的有类型引用数。
    #[must_use]
    pub const fn reference_count(&self) -> u64 {
        self.reference_count
    }

    /// 返回该模块 Typed AST 的关系出现项数。
    #[must_use]
    pub const fn relation_occurrence_count(&self) -> u64 {
        self.relation_occurrence_count
    }

    /// 返回全部 reference line 与 junction internal edge geometry 的 line 段总数。
    #[must_use]
    pub const fn line_segment_count(&self) -> u64 {
        self.line_segment_count
    }

    /// 返回全部 reference line 与 junction internal edge geometry 的 cubic Bézier 段总数。
    #[must_use]
    pub const fn cubic_segment_count(&self) -> u64 {
        self.cubic_segment_count
    }

    /// 返回 cubic Bézier 段的内部控制点总数；每段恰好两个。
    #[must_use]
    pub const fn control_point_count(&self) -> u64 {
        self.control_point_count
    }

    /// 返回由 lane 与 FacilityBand offset intent 派生的横向 offset 曲线总数。
    #[must_use]
    pub const fn offset_curve_count(&self) -> u64 {
        self.offset_curve_count
    }

    /// 返回 numeric freeze 生成的规范 `f32` 点总数，与 GeometryPointCount 资源维度一致。
    #[must_use]
    pub const fn canonical_point_count(&self) -> u64 {
        self.canonical_point_count
    }

    /// 返回横向 offset 曲线按 |中心偏移| 位模式分组的升序分布。
    #[must_use]
    pub fn absolute_offset_distribution(&self) -> &[GeometryOffsetCurveBucket] {
        &self.absolute_offset_distribution
    }
}

/// finish ① 的模块头校验结果；approach 集合只服务连接 entry/exit 成员检查，
/// 不进入 Typed AST。
struct FinishHeader {
    namespace: Arc<str>,
    imports: Box<[ImportRecord]>,
    import_index: HashMap<Arc<str>, usize>,
    approaches: Box<[ResolvedKeySet]>,
    provenance: FinishProvenance,
}

/// 解析后 `(namespace, key)` 的键集合；字面不同但解析相同的目标在此去重。
type ResolvedKeySet = BTreeSet<(Arc<str>, Arc<str>)>;

/// §2.3 provenance 登记字段的拥有型映射结果。
struct FinishProvenance {
    generator_build_id: Arc<str>,
    parameters_and_inputs_digest: [u8; 32],
    random_seed: Option<u64>,
    source_frontend_options_digest: [u8; 32],
    description: Arc<str>,
}

/// finish ① 的键查重分组；跨组允许重名，对齐 per-`EntityKind` 语义。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum FinishKeyGroup {
    Entity(EntityKind),
    RoadKey,
    SpanKey,
}

/// 由 finish 已闭合的解析树、冻结载荷与模块资源计数派生 §9.2 前端计数。
///
/// 曲线段计数覆盖全部 road reference line 与 junction internal edge geometry；
/// cubic Bézier 段每段恰好贡献两个内部控制点。
fn finish_module_counts(
    document: &ParsedGeometryDocument,
    payload: &schema::FrozenGeometryPayload,
    resource_counts: &ModuleResourceCounts,
) -> GeometryModuleCounts {
    let mut line_segment_count = 0_u64;
    let mut cubic_segment_count = 0_u64;
    let curves = document
        .roads
        .iter()
        .map(|road| &road.reference_line)
        .chain(
            document
                .junctions
                .iter()
                .flat_map(|junction| junction.internal_edges.iter().map(|edge| &edge.geometry)),
        );
    for curve in curves {
        for segment in &curve.segments {
            match segment {
                ParsedCurveSegment::Line { .. } => line_segment_count += 1,
                ParsedCurveSegment::CubicBezier { .. } => cubic_segment_count += 1,
            }
        }
    }
    let offset_curve_count = payload
        .offset_curve_distribution
        .iter()
        .fold(0_u64, |total, bucket| {
            total.saturating_add(bucket.curve_count)
        });
    GeometryModuleCounts {
        declaration_count: resource_counts.declaration_count,
        reference_count: resource_counts.reference_count,
        relation_occurrence_count: resource_counts.relation_occurrence_count,
        line_segment_count,
        cubic_segment_count,
        control_point_count: cubic_segment_count.saturating_mul(2),
        offset_curve_count,
        canonical_point_count: payload.geometry_point_count,
        absolute_offset_distribution: payload
            .offset_curve_distribution
            .iter()
            .map(|bucket| GeometryOffsetCurveBucket {
                absolute_offset_meters_bits: bucket.absolute_offset_meters_bits,
                curve_count: bucket.curve_count,
            })
            .collect(),
    }
}

/// 登记一个键定义：token 合法性与同组重复检查，先到先得单诊断。
fn register_finish_key<'a>(
    index: &mut HashMap<FinishKeyGroup, HashMap<&'a str, SourceSpan>>,
    group: FinishKeyGroup,
    field: &'static str,
    value: &'a SpannedString,
    single_string_limit: u64,
    span_of: &dyn Fn(ByteSpan) -> SourceSpan,
) -> Result<(), DiagnosticBundle> {
    let span = span_of(value.span);
    if let Some(violation) = external_token_violation(&value.value, single_string_limit) {
        return Err(match group {
            FinishKeyGroup::Entity(entity_kind) => DiagnosticBundle::single(
                Diagnostic::invalid_declaration_key(entity_kind, violation, span),
            ),
            FinishKeyGroup::RoadKey | FinishKeyGroup::SpanKey => {
                DiagnosticBundle::single(Diagnostic::invalid_geometry_document(
                    GeometryDocumentViolation::FieldValue,
                    Some(field),
                    Some(&value.value),
                    Some("valid external token"),
                    span,
                ))
            }
        });
    }
    if let Some(existing) = index
        .entry(group)
        .or_default()
        .insert(&value.value, span.clone())
    {
        return Err(match group {
            FinishKeyGroup::Entity(entity_kind) => DiagnosticBundle::single(
                Diagnostic::duplicate_declaration(entity_kind, &value.value, span, existing),
            ),
            FinishKeyGroup::RoadKey | FinishKeyGroup::SpanKey => {
                DiagnosticBundle::single(Diagnostic::invalid_geometry_document(
                    GeometryDocumentViolation::FieldValue,
                    Some(field),
                    Some(&value.value),
                    Some("unique key within the document"),
                    span,
                ))
            }
        });
    }
    Ok(())
}

/// 扫描全部键定义：token 合法性 + 同组重复，先到先得单诊断。
fn validate_finish_keys(
    parsed: &ParsedGeometryDocument,
    single_string_limit: u64,
    span_of: &dyn Fn(ByteSpan) -> SourceSpan,
) -> Result<(), DiagnosticBundle> {
    let mut index: HashMap<FinishKeyGroup, HashMap<&str, SourceSpan>> = HashMap::new();
    for frame in &parsed.frames {
        register_finish_key(
            &mut index,
            FinishKeyGroup::Entity(EntityKind::CanonicalFrame),
            "frames[].frameKey",
            &frame.frame_key,
            single_string_limit,
            span_of,
        )?;
    }
    for road in &parsed.roads {
        register_finish_key(
            &mut index,
            FinishKeyGroup::RoadKey,
            "roads[].roadKey",
            &road.road_key,
            single_string_limit,
            span_of,
        )?;
        for span in &road.cross_section_spans {
            register_finish_key(
                &mut index,
                FinishKeyGroup::SpanKey,
                "roads[].crossSectionSpans[].spanKey",
                &span.span_key,
                single_string_limit,
                span_of,
            )?;
            register_finish_key(
                &mut index,
                FinishKeyGroup::Entity(EntityKind::RoadCorridor),
                "roads[].crossSectionSpans[].corridorKey",
                &span.corridor_key,
                single_string_limit,
                span_of,
            )?;
            for section in &span.road_sections {
                register_finish_key(
                    &mut index,
                    FinishKeyGroup::Entity(EntityKind::RoadSection),
                    "roadSections[].sectionKey",
                    &section.section_key,
                    single_string_limit,
                    span_of,
                )?;
                for lane in &section.lanes {
                    register_finish_key(
                        &mut index,
                        FinishKeyGroup::Entity(EntityKind::AuthoringLane),
                        "lanes[].laneKey",
                        &lane.lane_key,
                        single_string_limit,
                        span_of,
                    )?;
                    register_finish_key(
                        &mut index,
                        FinishKeyGroup::Entity(EntityKind::LaneEdge),
                        "lanes[].laneEdgeKey",
                        &lane.lane_edge_key,
                        single_string_limit,
                        span_of,
                    )?;
                }
                for group in &section.lane_groups {
                    register_finish_key(
                        &mut index,
                        FinishKeyGroup::Entity(EntityKind::LaneGroup),
                        "laneGroups[].laneGroupKey",
                        &group.lane_group_key,
                        single_string_limit,
                        span_of,
                    )?;
                }
            }
            for facility in &span.facility_bands {
                register_finish_key(
                    &mut index,
                    FinishKeyGroup::Entity(EntityKind::FacilityBand),
                    "facilityBands[].facilityBandKey",
                    &facility.facility_band_key,
                    single_string_limit,
                    span_of,
                )?;
            }
        }
    }
    for junction in &parsed.junctions {
        register_finish_key(
            &mut index,
            FinishKeyGroup::Entity(EntityKind::Junction),
            "junctions[].junctionKey",
            &junction.junction_key,
            single_string_limit,
            span_of,
        )?;
        // §4.4：internal edge 的 laneEdgeKey 与 road lane 同一 LaneEdge 键分组。
        for internal_edge in &junction.internal_edges {
            register_finish_key(
                &mut index,
                FinishKeyGroup::Entity(EntityKind::LaneEdge),
                "junctions[].internalEdges[].laneEdgeKey",
                &internal_edge.lane_edge_key,
                single_string_limit,
                span_of,
            )?;
        }
        for connection in &junction.connections {
            register_finish_key(
                &mut index,
                FinishKeyGroup::Entity(EntityKind::Movement),
                "connections[].movementKey",
                &connection.movement_key,
                single_string_limit,
                span_of,
            )?;
            register_finish_key(
                &mut index,
                FinishKeyGroup::Entity(EntityKind::ManeuverPath),
                "connections[].maneuverPathKey",
                &connection.maneuver_path_key,
                single_string_limit,
                span_of,
            )?;
            for (field, approach_key) in [
                (
                    "connections[].directedEntryApproachKey",
                    &connection.directed_entry_approach_key,
                ),
                (
                    "connections[].directedExitApproachKey",
                    &connection.directed_exit_approach_key,
                ),
            ] {
                if let Some(_violation) =
                    external_token_violation(&approach_key.value, single_string_limit)
                {
                    return Err(DiagnosticBundle::single(
                        Diagnostic::invalid_geometry_document(
                            GeometryDocumentViolation::FieldValue,
                            Some(field),
                            Some(&approach_key.value),
                            Some("valid external token"),
                            span_of(approach_key.span),
                        ),
                    ));
                }
            }
        }
    }
    let overlays = &parsed.overlays;
    for group in &overlays.signal_groups {
        register_finish_key(
            &mut index,
            FinishKeyGroup::Entity(EntityKind::SignalGroup),
            "signalGroups[].signalGroupKey",
            &group.signal_group_key,
            single_string_limit,
            span_of,
        )?;
    }
    for controller in &overlays.signal_controllers {
        register_finish_key(
            &mut index,
            FinishKeyGroup::Entity(EntityKind::SignalController),
            "signalControllers[].signalControllerKey",
            &controller.signal_controller_key,
            single_string_limit,
            span_of,
        )?;
        for phase in &controller.phases {
            // 共同约束不要求 signalPhaseKey 全局唯一；只做 token 检查。
            if let Some(_violation) =
                external_token_violation(&phase.signal_phase_key.value, single_string_limit)
            {
                return Err(DiagnosticBundle::single(
                    Diagnostic::invalid_geometry_document(
                        GeometryDocumentViolation::FieldValue,
                        Some("signalControllers[].phases[].signalPhaseKey"),
                        Some(&phase.signal_phase_key.value),
                        Some("valid external token"),
                        span_of(phase.signal_phase_key.span),
                    ),
                ));
            }
        }
    }
    for area in &overlays.parking_areas {
        register_finish_key(
            &mut index,
            FinishKeyGroup::Entity(EntityKind::ParkingArea),
            "parkingAreas[].parkingAreaKey",
            &area.parking_area_key,
            single_string_limit,
            span_of,
        )?;
    }
    for space in &overlays.parking_spaces {
        register_finish_key(
            &mut index,
            FinishKeyGroup::Entity(EntityKind::ParkingSpace),
            "parkingSpaces[].parkingSpaceKey",
            &space.parking_space_key,
            single_string_limit,
            span_of,
        )?;
    }
    for class in &overlays.participant_classes {
        register_finish_key(
            &mut index,
            FinishKeyGroup::Entity(EntityKind::ParticipantClass),
            "participantClasses[].participantClassKey",
            &class.participant_class_key,
            single_string_limit,
            span_of,
        )?;
    }
    for profile in &overlays.vehicle_profiles {
        register_finish_key(
            &mut index,
            FinishKeyGroup::Entity(EntityKind::VehicleProfile),
            "vehicleProfiles[].vehicleProfileKey",
            &profile.vehicle_profile_key,
            single_string_limit,
            span_of,
        )?;
    }
    for rule in &overlays.access_rules {
        register_finish_key(
            &mut index,
            FinishKeyGroup::Entity(EntityKind::AccessRule),
            "accessRules[].accessRuleKey",
            &rule.access_rule_key,
            single_string_limit,
            span_of,
        )?;
    }
    for route in &overlays.static_routes {
        register_finish_key(
            &mut index,
            FinishKeyGroup::Entity(EntityKind::StaticRoute),
            "staticRoutes[].staticRouteKey",
            &route.static_route_key,
            single_string_limit,
            span_of,
        )?;
    }
    for stop_line in &overlays.stop_lines {
        register_finish_key(
            &mut index,
            FinishKeyGroup::Entity(EntityKind::StopLine),
            "stopLines[].stopLineKey",
            &stop_line.stop_line_key,
            single_string_limit,
            span_of,
        )?;
    }
    for gate in &overlays.maneuver_gates {
        register_finish_key(
            &mut index,
            FinishKeyGroup::Entity(EntityKind::ManeuverGate),
            "maneuverGates[].maneuverGateKey",
            &gate.maneuver_gate_key,
            single_string_limit,
            span_of,
        )?;
    }
    for zone in &overlays.waiting_zones {
        register_finish_key(
            &mut index,
            FinishKeyGroup::Entity(EntityKind::WaitingZone),
            "waitingZones[].waitingZoneKey",
            &zone.waiting_zone_key,
            single_string_limit,
            span_of,
        )?;
    }
    Ok(())
}

/// 文档内引用的拼写切分与命名空间分类。
struct ReferenceResolver<'a> {
    namespace: &'a Arc<str>,
    imports: &'a [ImportRecord],
    import_index: &'a HashMap<Arc<str>, usize>,
    single_string_limit: u64,
}

impl ReferenceResolver<'_> {
    /// 把引用值切分为已解析的模块命名空间与声明键。
    ///
    /// 含 `::` 的值按最后一个 `::` 切分；无前缀的值绑定本模块。显式前缀必须等于
    /// 本模块或某个显式导入，否则结构化失败。
    fn resolve_parts<'a>(
        &self,
        value: &'a SpannedString,
        field: &'static str,
        span_of: &dyn Fn(ByteSpan) -> SourceSpan,
    ) -> Result<(Arc<str>, &'a str), DiagnosticBundle> {
        let span = span_of(value.span);
        let (explicit_namespace, key) = match value.value.rsplit_once("::") {
            Some((namespace, key)) => (Some(namespace), key),
            None => (None, value.value.as_ref()),
        };
        if let Some(namespace) = explicit_namespace
            && let Some(violation) = external_token_violation(namespace, self.single_string_limit)
        {
            return Err(DiagnosticBundle::single(
                Diagnostic::invalid_reference_namespace(violation, span),
            ));
        }
        if let Some(_violation) = external_token_violation(key, self.single_string_limit) {
            return Err(DiagnosticBundle::single(
                Diagnostic::invalid_geometry_document(
                    GeometryDocumentViolation::FieldValue,
                    Some(field),
                    Some(&value.value),
                    Some("valid external token"),
                    span,
                ),
            ));
        }
        let namespace = match explicit_namespace {
            None => Arc::clone(self.namespace),
            Some(explicit) if explicit == self.namespace.as_ref() => Arc::clone(self.namespace),
            Some(explicit) => {
                let Some(import_index) = self.import_index.get(explicit).copied() else {
                    return Err(DiagnosticBundle::single(
                        Diagnostic::unimported_reference_module(explicit, span),
                    ));
                };
                Arc::clone(&self.imports[import_index].namespace)
            }
        };
        Ok((namespace, key))
    }

    /// 解析并拥有一个指向 `K` 类声明的 Typed AST 引用。
    fn resolve<K: EntityKindMarker>(
        &self,
        value: &SpannedString,
        field: &'static str,
        span_of: &dyn Fn(ByteSpan) -> SourceSpan,
    ) -> Result<OwnedEntityReference<K>, DiagnosticBundle> {
        let span = span_of(value.span);
        let (namespace, key) = self.resolve_parts(value, field, span_of)?;
        Ok(OwnedEntityReference::new(namespace, Arc::from(key), span))
    }

    /// 构造一个指向本模块声明的引用；wire 嵌套结构的隐式归属关系使用此路径。
    fn local_reference<K: EntityKindMarker>(
        &self,
        key: &SpannedString,
        span: SourceSpan,
    ) -> OwnedEntityReference<K> {
        OwnedEntityReference::new(
            Arc::clone(self.namespace),
            Arc::from(key.value.as_ref()),
            span,
        )
    }
}

/// 把 numeric freeze 错误包装为 Geometry 文档字段诊断；全部 `parse_finite`
/// 调用点共用同一包装。
fn numeric_field_diagnostic(
    error: schema::NumericFreezeError,
    span_of: &dyn Fn(ByteSpan) -> SourceSpan,
) -> DiagnosticBundle {
    DiagnosticBundle::single(Diagnostic::invalid_geometry_document(
        GeometryDocumentViolation::FieldValue,
        Some(error.field),
        Some(&format!("{:?}", error.violation)),
        Some("finite deterministic numeric field"),
        span_of(error.span),
    ))
}

/// 解析数值字段为有限 `f64`。
fn parse_finite_field(
    value: &RawNumber,
    field: &'static str,
    span_of: &dyn Fn(ByteSpan) -> SourceSpan,
) -> Result<f64, DiagnosticBundle> {
    schema::parse_finite(value, field).map_err(|error| numeric_field_diagnostic(error, span_of))
}

/// §4.5 的秒到毫秒换算：乘积必须有限、无小数且可无损收窄为 `u64`，
/// 不做四舍五入。
fn seconds_to_milliseconds(
    value: &RawNumber,
    field: &'static str,
    span_of: &dyn Fn(ByteSpan) -> SourceSpan,
) -> Result<u64, DiagnosticBundle> {
    let seconds = parse_finite_field(value, field, span_of)?;
    let milliseconds = seconds * 1000.0;
    if milliseconds.is_finite()
        && milliseconds.fract() == 0.0
        && (0.0..18446744073709551616.0).contains(&milliseconds)
    {
        #[expect(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        return Ok(milliseconds as u64);
    }
    Err(DiagnosticBundle::single(
        Diagnostic::invalid_geometry_document(
            GeometryDocumentViolation::FieldValue,
            Some(field),
            Some(&value.token),
            Some("non-negative whole milliseconds representable as u64"),
            span_of(value.span),
        ),
    ))
}

/// 解析无小数、可无损收窄为 `u32` 的非负整数字段。
fn parse_u32_field(
    value: &RawNumber,
    field: &'static str,
    span_of: &dyn Fn(ByteSpan) -> SourceSpan,
) -> Result<u32, DiagnosticBundle> {
    let parsed = parse_finite_field(value, field, span_of)?;
    if parsed.fract() == 0.0 && (0.0..=f64::from(u32::MAX)).contains(&parsed) {
        #[expect(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        return Ok(parsed as u32);
    }
    Err(DiagnosticBundle::single(
        Diagnostic::invalid_geometry_document(
            GeometryDocumentViolation::FieldValue,
            Some(field),
            Some(&value.token),
            Some("non-negative whole number representable as u32"),
            span_of(value.span),
        ),
    ))
}

/// 解析无小数、可无损收窄为 `i32` 的整数字段。
fn parse_i32_field(
    value: &RawNumber,
    field: &'static str,
    span_of: &dyn Fn(ByteSpan) -> SourceSpan,
) -> Result<i32, DiagnosticBundle> {
    let parsed = parse_finite_field(value, field, span_of)?;
    if parsed.fract() == 0.0 && (f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&parsed) {
        #[expect(clippy::cast_possible_truncation)]
        return Ok(parsed as i32);
    }
    Err(DiagnosticBundle::single(
        Diagnostic::invalid_geometry_document(
            GeometryDocumentViolation::FieldValue,
            Some(field),
            Some(&value.token),
            Some("whole number representable as i32"),
            span_of(value.span),
        ),
    ))
}

/// 校验 `kindId` 的共同 lane-bearing/non-traversable 分类约束。
fn validate_geometry_kind(
    entity_kind: EntityKind,
    key: &SpannedString,
    kind_id: &SpannedString,
    expected_category: FacilityKindCategory,
    single_string_limit: u64,
    span_of: &dyn Fn(ByteSpan) -> SourceSpan,
) -> Result<(), DiagnosticBundle> {
    let violation = match external_token_violation(&kind_id.value, single_string_limit) {
        Some(violation) => Some(FacilityKindViolation::InvalidToken(violation)),
        None => match facility_kind_category(&kind_id.value) {
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
            &key.value,
            &kind_id.value,
            expected_category,
            violation,
            span_of(kind_id.span),
        )));
    }
    Ok(())
}

/// ④ Typed AST 降阶：frames、roads、junctions 与 overlay 记录按 wire 文档顺序产出
/// 声明；确定性由文档顺序与显式排序共同保证。
fn lower_typed_ast(
    parsed: &ParsedGeometryDocument,
    header: &FinishHeader,
    payload: &schema::FrozenGeometryPayload,
    resolver: &ReferenceResolver,
    single_string_limit: u64,
    span_of: &dyn Fn(ByteSpan) -> SourceSpan,
) -> Result<Vec<TypedAstDeclaration>, DiagnosticBundle> {
    let mut lane_curves: HashMap<(&str, &str), &schema::FrozenLateralCurve> =
        HashMap::with_capacity(payload.lateral_curves.len());
    for curve in &payload.lateral_curves {
        if curve.kind != schema::LateralIntentKind::FacilityBand {
            lane_curves.insert((&curve.span_key, &curve.key), curve);
        }
    }
    let mut internal_edge_curves: HashMap<(&str, &str), &schema::FrozenInternalEdgeCurve> =
        HashMap::with_capacity(payload.internal_edge_curves.len());
    for curve in &payload.internal_edge_curves {
        internal_edge_curves.insert((&curve.junction_key, &curve.lane_edge_key), curve);
    }
    let mut declarations = Vec::with_capacity(lowered_declaration_capacity(parsed));
    lower_frames_and_roads(
        parsed,
        resolver,
        &lane_curves,
        single_string_limit,
        span_of,
        &mut declarations,
    )?;
    lower_junctions(
        parsed,
        header,
        resolver,
        &internal_edge_curves,
        span_of,
        &mut declarations,
    )?;
    lower_overlays(
        parsed,
        resolver,
        single_string_limit,
        span_of,
        &mut declarations,
    )?;
    Ok(declarations)
}

/// 顶层声明数的精确预分配上界。
fn lowered_declaration_capacity(parsed: &ParsedGeometryDocument) -> usize {
    let mut total = parsed
        .frames
        .len()
        .saturating_add(parsed.roads.len())
        .saturating_add(parsed.junctions.len());
    for road in &parsed.roads {
        for span in &road.cross_section_spans {
            total = total
                .saturating_add(2)
                .saturating_add(span.road_sections.len())
                .saturating_add(span.facility_bands.len());
            for section in &span.road_sections {
                total = total
                    .saturating_add(section.lane_groups.len())
                    .saturating_add(section.lanes.len());
            }
        }
    }
    for junction in &parsed.junctions {
        total = total
            .saturating_add(junction.connections.len().saturating_mul(3))
            .saturating_add(junction.internal_edges.len().saturating_mul(2));
    }
    let overlays = &parsed.overlays;
    total
        .saturating_add(overlays.signal_groups.len())
        .saturating_add(overlays.signal_controllers.len())
        .saturating_add(overlays.parking_areas.len())
        .saturating_add(overlays.parking_spaces.len())
        .saturating_add(overlays.participant_classes.len())
        .saturating_add(overlays.vehicle_profiles.len())
        .saturating_add(overlays.access_rules.len())
        .saturating_add(overlays.static_routes.len())
        .saturating_add(overlays.stop_lines.len())
        .saturating_add(overlays.maneuver_gates.len())
        .saturating_add(overlays.waiting_zones.len())
}

fn lower_frames_and_roads(
    parsed: &ParsedGeometryDocument,
    resolver: &ReferenceResolver,
    lane_curves: &HashMap<(&str, &str), &schema::FrozenLateralCurve>,
    single_string_limit: u64,
    span_of: &dyn Fn(ByteSpan) -> SourceSpan,
    declarations: &mut Vec<TypedAstDeclaration>,
) -> Result<(), DiagnosticBundle> {
    for frame in &parsed.frames {
        // §5.1 反向闭合：geometry 的边几何由 intent 绑定，frame 声明不携带显式中心线。
        declarations.push(TypedAstDeclaration::CanonicalFrame(
            CanonicalFrameDeclaration {
                header: DeclarationHeader {
                    entity_kind: EntityKind::CanonicalFrame,
                    stable_key: Arc::from(frame.frame_key.value.as_ref()),
                    span: span_of(frame.frame_key.span),
                },
                lane_edge_geometries: Box::default(),
            },
        ));
    }
    for road in &parsed.roads {
        let frame =
            resolver.resolve::<CanonicalFrameKind>(&road.frame, "roads[].frame", span_of)?;
        declarations.push(TypedAstDeclaration::GeometryReferenceLine(
            GeometryReferenceLineIntent {
                road_key: Arc::from(road.road_key.value.as_ref()),
                frame: frame.clone(),
                span: span_of(road.span),
            },
        ));
        for span in &road.cross_section_spans {
            lower_cross_section_span(
                span,
                &frame,
                resolver,
                lane_curves,
                single_string_limit,
                span_of,
                declarations,
            )?;
        }
    }
    Ok(())
}

fn lower_cross_section_span(
    span: &schema::ParsedCrossSectionSpan,
    frame: &OwnedEntityReference<CanonicalFrameKind>,
    resolver: &ReferenceResolver,
    lane_curves: &HashMap<(&str, &str), &schema::FrozenLateralCurve>,
    single_string_limit: u64,
    span_of: &dyn Fn(ByteSpan) -> SourceSpan,
    declarations: &mut Vec<TypedAstDeclaration>,
) -> Result<(), DiagnosticBundle> {
    let mut section_index: HashMap<&str, &schema::ParsedRoadSection> =
        HashMap::with_capacity(span.road_sections.len());
    for section in &span.road_sections {
        section_index.insert(&section.section_key.value, section);
    }
    let mut facility_index: HashMap<&str, &schema::ParsedFacilityBand> =
        HashMap::with_capacity(span.facility_bands.len());
    for facility in &span.facility_bands {
        facility_index.insert(&facility.facility_band_key.value, facility);
    }

    // 显式 `elements` 从左到右展开 offset 意图；每个 RoadSection/FacilityBand 恰好一次。
    let mut offsets = Vec::new();
    let mut elements = Vec::with_capacity(span.elements.len());
    let mut consumed: BTreeSet<(&str, &str)> = BTreeSet::new();
    let mut road_section_element_keys: BTreeSet<&str> = BTreeSet::new();
    for element in &span.elements {
        match element {
            schema::ParsedCorridorElement::RoadSection {
                section_key,
                span: element_span,
            } => {
                road_section_element_keys.insert(&section_key.value);
                if !consumed.insert(("roadSection", &section_key.value)) {
                    return Err(DiagnosticBundle::single(
                        Diagnostic::invalid_geometry_document(
                            GeometryDocumentViolation::FieldValue,
                            Some("crossSectionSpans[].elements"),
                            Some(&section_key.value),
                            Some("each RoadSection/FacilityBand exactly once"),
                            span_of(*element_span),
                        ),
                    ));
                }
                let Some(section) = section_index.get(section_key.value.as_ref()).copied() else {
                    return Err(DiagnosticBundle::single(
                        Diagnostic::invalid_geometry_document(
                            GeometryDocumentViolation::FieldValue,
                            Some("crossSectionSpans[].elements"),
                            Some(&section_key.value),
                            Some("member declared in the same span roadSections"),
                            span_of(*element_span),
                        ),
                    ));
                };
                for lane in &section.lanes {
                    offsets.push(GeometryOffsetIntent {
                        key: Arc::from(lane.lane_key.value.as_ref()),
                        kind: match lane.direction {
                            schema::ParsedLaneDirection::Forward => {
                                GeometryOffsetIntentKind::ForwardLane
                            }
                            schema::ParsedLaneDirection::Backward => {
                                GeometryOffsetIntentKind::BackwardLane
                            }
                        },
                        width_meters: parse_finite_field(
                            &lane.width_meters,
                            "lanes[].widthMeters",
                            span_of,
                        )?,
                        span: span_of(lane.span),
                    });
                }
                elements.push(OwnedCorridorElementReference::RoadSection(
                    resolver.resolve::<RoadSectionKind>(
                        section_key,
                        "crossSectionSpans[].elements",
                        span_of,
                    )?,
                ));
            }
            schema::ParsedCorridorElement::FacilityBand {
                facility_band_key,
                span: element_span,
            } => {
                if !consumed.insert(("facilityBand", &facility_band_key.value)) {
                    return Err(DiagnosticBundle::single(
                        Diagnostic::invalid_geometry_document(
                            GeometryDocumentViolation::FieldValue,
                            Some("crossSectionSpans[].elements"),
                            Some(&facility_band_key.value),
                            Some("each RoadSection/FacilityBand exactly once"),
                            span_of(*element_span),
                        ),
                    ));
                }
                let Some(facility) = facility_index
                    .get(facility_band_key.value.as_ref())
                    .copied()
                else {
                    return Err(DiagnosticBundle::single(
                        Diagnostic::invalid_geometry_document(
                            GeometryDocumentViolation::FieldValue,
                            Some("crossSectionSpans[].elements"),
                            Some(&facility_band_key.value),
                            Some("member declared in the same span facilityBands"),
                            span_of(*element_span),
                        ),
                    ));
                };
                offsets.push(GeometryOffsetIntent {
                    key: Arc::from(facility.facility_band_key.value.as_ref()),
                    kind: GeometryOffsetIntentKind::FacilityBand,
                    width_meters: parse_finite_field(
                        &facility.width_meters,
                        "facilityBands[].widthMeters",
                        span_of,
                    )?,
                    span: span_of(facility.span),
                });
                elements.push(OwnedCorridorElementReference::FacilityBand(
                    resolver.resolve::<FacilityBandKind>(
                        facility_band_key,
                        "crossSectionSpans[].elements",
                        span_of,
                    )?,
                ));
            }
        }
    }
    // §4.3：`referenceSectionKey` 必须引用其中一个 RoadSectionElement。
    if !road_section_element_keys.contains(span.reference_section_key.value.as_ref()) {
        return Err(DiagnosticBundle::single(
            Diagnostic::invalid_geometry_document(
                GeometryDocumentViolation::FieldValue,
                Some("crossSectionSpans[].referenceSectionKey"),
                Some(&span.reference_section_key.value),
                Some("one of the span elements roadSection members"),
                span_of(span.reference_section_key.span),
            ),
        ));
    }

    declarations.push(TypedAstDeclaration::GeometryCrossSectionSpan(
        GeometryCrossSectionSpanIntent {
            span_key: Arc::from(span.span_key.value.as_ref()),
            frame: frame.clone(),
            corridor: resolver.resolve::<RoadCorridorKind>(
                &span.corridor_key,
                "crossSectionSpans[].corridorKey",
                span_of,
            )?,
            offsets: offsets.into_boxed_slice(),
            span: span_of(span.span),
        },
    ));
    declarations.push(TypedAstDeclaration::RoadCorridor(RoadCorridorDeclaration {
        header: DeclarationHeader {
            entity_kind: EntityKind::RoadCorridor,
            stable_key: Arc::from(span.corridor_key.value.as_ref()),
            span: span_of(span.corridor_key.span),
        },
        reference_section: resolver.resolve::<RoadSectionKind>(
            &span.reference_section_key,
            "crossSectionSpans[].referenceSectionKey",
            span_of,
        )?,
        elements: elements.into_boxed_slice(),
    }));

    for section in &span.road_sections {
        // §4.3：同一 RoadSection 的全部 lane 必须有相同 direction。
        if let Some(first) = section.lanes.first() {
            for lane in &section.lanes[1..] {
                if lane.direction != first.direction {
                    return Err(DiagnosticBundle::single(
                        Diagnostic::invalid_geometry_document(
                            GeometryDocumentViolation::FieldValue,
                            Some("roadSections[].lanes[].direction"),
                            Some(match lane.direction {
                                schema::ParsedLaneDirection::Forward => "forward",
                                schema::ParsedLaneDirection::Backward => "backward",
                            }),
                            Some("same direction for all lanes of one RoadSection"),
                            span_of(lane.span),
                        ),
                    ));
                }
            }
        }
        validate_geometry_kind(
            EntityKind::RoadSection,
            &section.section_key,
            &section.kind_id,
            FacilityKindCategory::LaneBearing,
            single_string_limit,
            span_of,
        )?;
        let mut lanes = Vec::with_capacity(section.lanes.len());
        for lane in &section.lanes {
            lanes.push(AuthoringLaneDeclaration {
                header: DeclarationHeader {
                    entity_kind: EntityKind::AuthoringLane,
                    stable_key: Arc::from(lane.lane_key.value.as_ref()),
                    span: span_of(lane.lane_key.span),
                },
                edge_chain: Box::new([resolver.resolve::<LaneEdgeKind>(
                    &lane.lane_edge_key,
                    "lanes[].laneEdgeKey",
                    span_of,
                )?]),
                lane_group: lane
                    .lane_group_key
                    .as_ref()
                    .map(|key| {
                        resolver.resolve::<LaneGroupKind>(key, "lanes[].laneGroupKey", span_of)
                    })
                    .transpose()?,
            });
        }
        declarations.push(TypedAstDeclaration::RoadSection(RoadSectionDeclaration {
            header: DeclarationHeader {
                entity_kind: EntityKind::RoadSection,
                stable_key: Arc::from(section.section_key.value.as_ref()),
                span: span_of(section.section_key.span),
            },
            kind_id: Arc::from(section.kind_id.value.as_ref()),
            lanes: lanes.into_boxed_slice(),
        }));
        for group in &section.lane_groups {
            declarations.push(TypedAstDeclaration::LaneGroup(LaneGroupDeclaration {
                header: DeclarationHeader {
                    entity_kind: EntityKind::LaneGroup,
                    stable_key: Arc::from(group.lane_group_key.value.as_ref()),
                    span: span_of(group.lane_group_key.span),
                },
                road_section: resolver.local_reference(&section.section_key, span_of(group.span)),
            }));
        }
        for lane in &section.lanes {
            let curve = lane_curves
                .get(&(span.span_key.value.as_ref(), lane.lane_key.value.as_ref()))
                .expect("numeric freeze emits one curve per lane");
            let length_meters = schema::frozen_polyline_length_meters(&curve.points);
            let length = EdgeLength::try_new(length_meters).map_err(|violation| {
                DiagnosticBundle::single(Diagnostic::invalid_lane_edge_length(
                    &lane.lane_edge_key.value,
                    length_meters,
                    violation,
                    span_of(lane.lane_edge_key.span),
                ))
            })?;
            let speed = parse_finite_field(
                &lane.speed_limit_meters_per_second,
                "lanes[].speedLimitMetersPerSecond",
                span_of,
            )?;
            let speed_limit = SpeedLimit::try_new(speed).map_err(|violation| {
                DiagnosticBundle::single(Diagnostic::invalid_lane_edge_speed_limit(
                    &lane.lane_edge_key.value,
                    speed,
                    violation,
                    span_of(lane.lane_edge_key.span),
                ))
            })?;
            let mut successors = Vec::with_capacity(lane.successors.len());
            for successor in &lane.successors {
                successors.push(resolver.resolve::<LaneEdgeKind>(
                    successor,
                    "lanes[].successors",
                    span_of,
                )?);
            }
            // 与 Synthetic 前端一致：successors 按 (module namespace, declaration key)
            // 规范化排序，文档内书写顺序不进入来源身份。
            successors.sort_unstable_by(|left, right| {
                (&left.module_namespace, &left.declaration_key)
                    .cmp(&(&right.module_namespace, &right.declaration_key))
            });
            declarations.push(TypedAstDeclaration::LaneEdge(LaneEdgeDeclaration {
                header: DeclarationHeader {
                    entity_kind: EntityKind::LaneEdge,
                    stable_key: Arc::from(lane.lane_edge_key.value.as_ref()),
                    span: span_of(lane.lane_edge_key.span),
                },
                length,
                speed_limit,
                successors: successors.into_boxed_slice(),
            }));
        }
    }
    for facility in &span.facility_bands {
        validate_geometry_kind(
            EntityKind::FacilityBand,
            &facility.facility_band_key,
            &facility.kind_id,
            FacilityKindCategory::NonTraversable,
            single_string_limit,
            span_of,
        )?;
        declarations.push(TypedAstDeclaration::FacilityBand(FacilityBandDeclaration {
            header: DeclarationHeader {
                entity_kind: EntityKind::FacilityBand,
                stable_key: Arc::from(facility.facility_band_key.value.as_ref()),
                span: span_of(facility.facility_band_key.span),
            },
            kind_id: Arc::from(facility.kind_id.value.as_ref()),
        }));
    }
    Ok(())
}

fn lower_junctions(
    parsed: &ParsedGeometryDocument,
    header: &FinishHeader,
    resolver: &ReferenceResolver,
    internal_edge_curves: &HashMap<(&str, &str), &schema::FrozenInternalEdgeCurve>,
    span_of: &dyn Fn(ByteSpan) -> SourceSpan,
    declarations: &mut Vec<TypedAstDeclaration>,
) -> Result<(), DiagnosticBundle> {
    for (junction_index, junction) in parsed.junctions.iter().enumerate() {
        declarations.push(TypedAstDeclaration::Junction(JunctionDeclaration {
            header: DeclarationHeader {
                entity_kind: EntityKind::Junction,
                stable_key: Arc::from(junction.junction_key.value.as_ref()),
                span: span_of(junction.junction_key.span),
            },
        }));
        // §4.4/§5.1：每条 internal record 由当前 Junction 唯一拥有，产出带显式
        // speed、successors 为空的 `LaneEdge` 共同声明与显式 geometry intent；
        // length 从冻结中心线按 §6.2 的规范 `f64` 累计派生，与 road lane 同一约定。
        let mut internal_edge_index: HashMap<&str, &schema::ParsedInternalEdge> =
            HashMap::with_capacity(junction.internal_edges.len());
        for internal_edge in &junction.internal_edges {
            internal_edge_index.insert(&internal_edge.lane_edge_key.value, internal_edge);
            let curve = internal_edge_curves
                .get(&(
                    junction.junction_key.value.as_ref(),
                    internal_edge.lane_edge_key.value.as_ref(),
                ))
                .expect("numeric freeze emits one curve per internal edge");
            let length_meters = schema::frozen_polyline_length_meters(&curve.points);
            let length = EdgeLength::try_new(length_meters).map_err(|violation| {
                DiagnosticBundle::single(Diagnostic::invalid_lane_edge_length(
                    &internal_edge.lane_edge_key.value,
                    length_meters,
                    violation,
                    span_of(internal_edge.lane_edge_key.span),
                ))
            })?;
            let speed = parse_finite_field(
                &internal_edge.speed_limit_meters_per_second,
                "junctions[].internalEdges[].speedLimitMetersPerSecond",
                span_of,
            )?;
            let speed_limit = SpeedLimit::try_new(speed).map_err(|violation| {
                DiagnosticBundle::single(Diagnostic::invalid_lane_edge_speed_limit(
                    &internal_edge.lane_edge_key.value,
                    speed,
                    violation,
                    span_of(internal_edge.lane_edge_key.span),
                ))
            })?;
            declarations.push(TypedAstDeclaration::LaneEdge(LaneEdgeDeclaration {
                header: DeclarationHeader {
                    entity_kind: EntityKind::LaneEdge,
                    stable_key: Arc::from(internal_edge.lane_edge_key.value.as_ref()),
                    span: span_of(internal_edge.lane_edge_key.span),
                },
                length,
                speed_limit,
                successors: Box::default(),
            }));
            declarations.push(TypedAstDeclaration::GeometryInternalEdge(
                GeometryInternalEdgeIntent {
                    key: Arc::from(internal_edge.lane_edge_key.value.as_ref()),
                    junction: resolver
                        .local_reference(&junction.junction_key, span_of(internal_edge.span)),
                    span: span_of(internal_edge.span),
                },
            ));
        }
        let approaches = &header.approaches[junction_index];
        let mut referenced_internal_edges: BTreeSet<&str> = BTreeSet::new();
        for connection in &junction.connections {
            let entry_edge = resolver.resolve::<LaneEdgeKind>(
                &connection.entry_edge,
                "connections[].entryEdge",
                span_of,
            )?;
            if !approaches.contains(&(
                Arc::clone(&entry_edge.module_namespace),
                Arc::clone(&entry_edge.declaration_key),
            )) {
                return Err(DiagnosticBundle::single(
                    Diagnostic::invalid_geometry_document(
                        GeometryDocumentViolation::FieldValue,
                        Some("connections[].entryEdge"),
                        Some(&connection.entry_edge.value),
                        Some("one of the owning junction approachEdges"),
                        span_of(connection.entry_edge.span),
                    ),
                ));
            }
            let exit_edge = resolver.resolve::<LaneEdgeKind>(
                &connection.exit_edge,
                "connections[].exitEdge",
                span_of,
            )?;
            if !approaches.contains(&(
                Arc::clone(&exit_edge.module_namespace),
                Arc::clone(&exit_edge.declaration_key),
            )) {
                return Err(DiagnosticBundle::single(
                    Diagnostic::invalid_geometry_document(
                        GeometryDocumentViolation::FieldValue,
                        Some("connections[].exitEdge"),
                        Some(&connection.exit_edge.value),
                        Some("one of the owning junction approachEdges"),
                        span_of(connection.exit_edge.span),
                    ),
                ));
            }
            // §4.4：internalEdgeSequence 是连接内有序引用，每项必须解析到当前
            // Junction 的 internalEdges；解析后的 (namespace, key) 在同一连接内
            // 不得重复（parser 只保证字面 token 不重复）。
            let mut internal_edges = Vec::with_capacity(connection.internal_edge_sequence.len());
            let mut connection_internal_keys: ResolvedKeySet = BTreeSet::new();
            for token in &connection.internal_edge_sequence {
                let reference = resolver.resolve::<LaneEdgeKind>(
                    token,
                    "junctions[].connections[].internalEdgeSequence",
                    span_of,
                )?;
                let internal_edge =
                    if reference.module_namespace.as_ref() == resolver.namespace.as_ref() {
                        internal_edge_index
                            .get(reference.declaration_key.as_ref())
                            .copied()
                    } else {
                        None
                    };
                let Some(internal_edge) = internal_edge else {
                    return Err(DiagnosticBundle::single(
                        Diagnostic::invalid_geometry_document(
                            GeometryDocumentViolation::FieldValue,
                            Some("junctions[].connections[].internalEdgeSequence"),
                            Some(&token.value),
                            Some("one of the owning junction internalEdges"),
                            span_of(token.span),
                        ),
                    ));
                };
                referenced_internal_edges.insert(internal_edge.lane_edge_key.value.as_ref());
                if !connection_internal_keys.insert((
                    Arc::clone(&reference.module_namespace),
                    Arc::clone(&reference.declaration_key),
                )) {
                    return Err(DiagnosticBundle::single(
                        Diagnostic::invalid_geometry_document(
                            GeometryDocumentViolation::FieldValue,
                            Some("junctions[].connections[].internalEdgeSequence"),
                            Some(&token.value),
                            Some("distinct internal edge references within one connection"),
                            span_of(token.span),
                        ),
                    ));
                }
                internal_edges.push(reference);
            }
            declarations.push(TypedAstDeclaration::Movement(MovementDeclaration {
                header: DeclarationHeader {
                    entity_kind: EntityKind::Movement,
                    stable_key: Arc::from(connection.movement_key.value.as_ref()),
                    span: span_of(connection.movement_key.span),
                },
                junction: resolver
                    .local_reference(&junction.junction_key, span_of(connection.span)),
                directed_entry_approach_key: Arc::from(
                    connection.directed_entry_approach_key.value.as_ref(),
                ),
                directed_exit_approach_key: Arc::from(
                    connection.directed_exit_approach_key.value.as_ref(),
                ),
            }));
            declarations.push(TypedAstDeclaration::ManeuverPath(ManeuverPathDeclaration {
                header: DeclarationHeader {
                    entity_kind: EntityKind::ManeuverPath,
                    stable_key: Arc::from(connection.maneuver_path_key.value.as_ref()),
                    span: span_of(connection.maneuver_path_key.span),
                },
                movement: resolver
                    .local_reference(&connection.movement_key, span_of(connection.span)),
                entry_edge: entry_edge.clone(),
                internal_edges: internal_edges.clone().into_boxed_slice(),
                exit_edge: exit_edge.clone(),
            }));
            declarations.push(TypedAstDeclaration::GeometryConnection(
                GeometryConnectionIntent {
                    junction: resolver
                        .local_reference(&junction.junction_key, span_of(connection.span)),
                    maneuver_path: resolver
                        .local_reference(&connection.maneuver_path_key, span_of(connection.span)),
                    entry_edge,
                    internal_edges: internal_edges.into_boxed_slice(),
                    exit_edge,
                    span: span_of(connection.span),
                },
            ));
        }
        // §4.4 失败关闭：Junction 中没有被任何 connection 引用的 internal edge。
        for internal_edge in &junction.internal_edges {
            if !referenced_internal_edges.contains(internal_edge.lane_edge_key.value.as_ref()) {
                return Err(DiagnosticBundle::single(
                    Diagnostic::invalid_geometry_document(
                        GeometryDocumentViolation::FieldValue,
                        Some("junctions[].internalEdges"),
                        Some(&internal_edge.lane_edge_key.value),
                        Some("referenced by at least one connection internalEdgeSequence"),
                        span_of(internal_edge.lane_edge_key.span),
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn lower_overlays(
    parsed: &ParsedGeometryDocument,
    resolver: &ReferenceResolver,
    single_string_limit: u64,
    span_of: &dyn Fn(ByteSpan) -> SourceSpan,
    declarations: &mut Vec<TypedAstDeclaration>,
) -> Result<(), DiagnosticBundle> {
    let _ = single_string_limit;
    let overlays = &parsed.overlays;
    for group in &overlays.signal_groups {
        declarations.push(TypedAstDeclaration::SignalGroup(SignalGroupDeclaration {
            header: DeclarationHeader {
                entity_kind: EntityKind::SignalGroup,
                stable_key: Arc::from(group.signal_group_key.value.as_ref()),
                span: span_of(group.signal_group_key.span),
            },
        }));
    }
    for controller in &overlays.signal_controllers {
        let offset_ms = seconds_to_milliseconds(
            &controller.offset_seconds,
            "signalControllers[].offsetSeconds",
            span_of,
        )?;
        let mut signal_groups = Vec::with_capacity(controller.signal_groups.len());
        for group in &controller.signal_groups {
            signal_groups.push(resolver.resolve::<SignalGroupKind>(
                group,
                "signalControllers[].signalGroups",
                span_of,
            )?);
        }
        let mut phases = Vec::with_capacity(controller.phases.len());
        for phase in &controller.phases {
            let duration_ms = seconds_to_milliseconds(
                &phase.duration_seconds,
                "signalControllers[].phases[].durationSeconds",
                span_of,
            )?;
            let mut states = Vec::with_capacity(phase.states.len());
            for state in &phase.states {
                states.push(SignalGroupStateDeclaration {
                    signal_group: resolver.resolve::<SignalGroupKind>(
                        &state.signal_group,
                        "signalControllers[].phases[].states[].signalGroup",
                        span_of,
                    )?,
                    aspect: match state.aspect {
                        schema::ParsedSignalAspect::Red => SignalAspect::Red,
                        schema::ParsedSignalAspect::Yellow => SignalAspect::Yellow,
                        schema::ParsedSignalAspect::Green => SignalAspect::Green,
                    },
                });
            }
            phases.push(SignalPhaseDeclaration {
                header: DeclarationHeader {
                    entity_kind: EntityKind::SignalPhase,
                    stable_key: Arc::from(phase.signal_phase_key.value.as_ref()),
                    span: span_of(phase.signal_phase_key.span),
                },
                duration_ms,
                states: states.into_boxed_slice(),
            });
        }
        declarations.push(TypedAstDeclaration::SignalController(
            SignalControllerDeclaration {
                header: DeclarationHeader {
                    entity_kind: EntityKind::SignalController,
                    stable_key: Arc::from(controller.signal_controller_key.value.as_ref()),
                    span: span_of(controller.signal_controller_key.span),
                },
                offset_ms,
                signal_groups: signal_groups.into_boxed_slice(),
                phases: phases.into_boxed_slice(),
            },
        ));
    }
    for area in &overlays.parking_areas {
        declarations.push(TypedAstDeclaration::ParkingArea(ParkingAreaDeclaration {
            header: DeclarationHeader {
                entity_kind: EntityKind::ParkingArea,
                stable_key: Arc::from(area.parking_area_key.value.as_ref()),
                span: span_of(area.parking_area_key.span),
            },
        }));
    }
    for space in &overlays.parking_spaces {
        let parking_area = space
            .parking_area
            .as_ref()
            .map(|area| {
                resolver.resolve::<ParkingAreaKind>(area, "parkingSpaces[].parkingArea", span_of)
            })
            .transpose()?;
        let anchor = |anchor: &schema::ParsedParkingAnchor,
                      field: &'static str|
         -> Result<ParkingLaneAnchorDeclaration, DiagnosticBundle> {
            Ok(ParkingLaneAnchorDeclaration {
                lane_edge: resolver.resolve::<LaneEdgeKind>(&anchor.lane_edge, field, span_of)?,
                progress_meters: parse_finite_field(&anchor.progress_meters, field, span_of)?,
            })
        };
        let geometry = &space.geometry;
        declarations.push(TypedAstDeclaration::ParkingSpace(ParkingSpaceDeclaration {
            header: DeclarationHeader {
                entity_kind: EntityKind::ParkingSpace,
                stable_key: Arc::from(space.parking_space_key.value.as_ref()),
                span: span_of(space.parking_space_key.span),
            },
            parking_area,
            entry: anchor(&space.entry, "parkingSpaces[].entry")?,
            exit: anchor(&space.exit, "parkingSpaces[].exit")?,
            geometry: ParkingSpaceGeometryInput {
                lateral_offset_meters: parse_finite_field(
                    &geometry.lateral_offset_meters,
                    "parkingSpaces[].geometry.lateralOffsetMeters",
                    span_of,
                )?,
                heading_offset_radians: parse_finite_field(
                    &geometry.heading_offset_radians,
                    "parkingSpaces[].geometry.headingOffsetRadians",
                    span_of,
                )?,
                length_meters: parse_finite_field(
                    &geometry.length_meters,
                    "parkingSpaces[].geometry.lengthMeters",
                    span_of,
                )?,
                width_meters: parse_finite_field(
                    &geometry.width_meters,
                    "parkingSpaces[].geometry.widthMeters",
                    span_of,
                )?,
            },
        }));
    }
    for class in &overlays.participant_classes {
        let extends = class
            .extends
            .as_ref()
            .map(|parent| {
                resolver.resolve::<ParticipantClassKind>(
                    parent,
                    "participantClasses[].extends",
                    span_of,
                )
            })
            .transpose()?;
        declarations.push(TypedAstDeclaration::ParticipantClass(
            ParticipantClassDeclaration {
                header: DeclarationHeader {
                    entity_kind: EntityKind::ParticipantClass,
                    stable_key: Arc::from(class.participant_class_key.value.as_ref()),
                    span: span_of(class.participant_class_key.span),
                },
                extends,
            },
        ));
    }
    for profile in &overlays.vehicle_profiles {
        const IIDM_FIELDS: [&str; 7] = [
            "iidm.lengthMeters",
            "iidm.desiredSpeedMetersPerSecond",
            "iidm.minGapMeters",
            "iidm.timeHeadwaySeconds",
            "iidm.maxAccelerationMetersPerSecondSquared",
            "iidm.comfortableDecelerationMetersPerSecondSquared",
            "iidm.emergencyDecelerationMetersPerSecondSquared",
        ];
        let mut values = [0.0_f64; 7];
        for (index, field) in IIDM_FIELDS.iter().enumerate() {
            values[index] = parse_finite_field(&profile.iidm[index], field, span_of)?;
        }
        let iidm = IidmVehicleProfileInput {
            length_meters: values[0],
            desired_speed_meters_per_second: values[1],
            min_gap_meters: values[2],
            time_headway_seconds: values[3],
            max_acceleration_meters_per_second_squared: values[4],
            comfortable_deceleration_meters_per_second_squared: values[5],
            emergency_deceleration_meters_per_second_squared: values[6],
        };
        validate_vehicle_profile_scalars(
            &profile.vehicle_profile_key.value,
            iidm,
            &span_of(profile.span),
        )?;
        declarations.push(TypedAstDeclaration::VehicleProfile(
            VehicleProfileDeclaration {
                header: DeclarationHeader {
                    entity_kind: EntityKind::VehicleProfile,
                    stable_key: Arc::from(profile.vehicle_profile_key.value.as_ref()),
                    span: span_of(profile.vehicle_profile_key.span),
                },
                participant_class: resolver.resolve::<ParticipantClassKind>(
                    &profile.participant_class,
                    "vehicleProfiles[].participantClass",
                    span_of,
                )?,
                iidm,
            },
        ));
    }
    for rule in &overlays.access_rules {
        let target = match &rule.target {
            schema::ParsedAccessTarget::LaneEdge(value) => OwnedAccessRuleTarget::LaneEdge(
                resolver.resolve::<LaneEdgeKind>(value, "accessRules[].target", span_of)?,
            ),
            schema::ParsedAccessTarget::LaneGroup(value) => OwnedAccessRuleTarget::LaneGroup(
                resolver.resolve::<LaneGroupKind>(value, "accessRules[].target", span_of)?,
            ),
            schema::ParsedAccessTarget::RoadSection(value) => OwnedAccessRuleTarget::RoadSection(
                resolver.resolve::<RoadSectionKind>(value, "accessRules[].target", span_of)?,
            ),
            schema::ParsedAccessTarget::ManeuverPath(value) => OwnedAccessRuleTarget::ManeuverPath(
                resolver.resolve::<ManeuverPathKind>(value, "accessRules[].target", span_of)?,
            ),
            schema::ParsedAccessTarget::FacilityBand(value) => OwnedAccessRuleTarget::FacilityBand(
                resolver.resolve::<FacilityBandKind>(value, "accessRules[].target", span_of)?,
            ),
        };
        let mut participant_classes = Vec::with_capacity(rule.participant_classes.len());
        for class in &rule.participant_classes {
            participant_classes.push(resolver.resolve::<ParticipantClassKind>(
                class,
                "accessRules[].participantClasses",
                span_of,
            )?);
        }
        let regulation = rule
            .regulation
            .as_ref()
            .map(|regulation| OwnedAccessRegulation {
                jurisdiction: Arc::from(regulation.jurisdiction.value.as_ref()),
                version: Arc::from(regulation.version.value.as_ref()),
                source: regulation
                    .source
                    .as_ref()
                    .map(|source| Arc::from(source.value.as_ref())),
            });
        let priority = parse_i32_field(&rule.priority, "accessRules[].priority", span_of)?;
        declarations.push(TypedAstDeclaration::AccessRule(AccessRuleDeclaration {
            header: DeclarationHeader {
                entity_kind: EntityKind::AccessRule,
                stable_key: Arc::from(rule.access_rule_key.value.as_ref()),
                span: span_of(rule.access_rule_key.span),
            },
            target,
            effect: match rule.effect {
                schema::ParsedAccessEffect::Allow => AccessEffect::Allow,
                schema::ParsedAccessEffect::Deny => AccessEffect::Deny,
            },
            participant_classes: participant_classes.into_boxed_slice(),
            regulation,
            priority,
        }));
    }
    for route in &overlays.static_routes {
        let mut edge_sequence = Vec::with_capacity(route.edge_sequence.len());
        for edge in &route.edge_sequence {
            edge_sequence.push(resolver.resolve::<LaneEdgeKind>(
                edge,
                "staticRoutes[].edgeSequence",
                span_of,
            )?);
        }
        declarations.push(TypedAstDeclaration::StaticRoute(StaticRouteDeclaration {
            header: DeclarationHeader {
                entity_kind: EntityKind::StaticRoute,
                stable_key: Arc::from(route.static_route_key.value.as_ref()),
                span: span_of(route.static_route_key.span),
            },
            edge_sequence: edge_sequence.into_boxed_slice(),
        }));
    }
    for stop_line in &overlays.stop_lines {
        declarations.push(TypedAstDeclaration::StopLine(StopLineDeclaration {
            header: DeclarationHeader {
                entity_kind: EntityKind::StopLine,
                stable_key: Arc::from(stop_line.stop_line_key.value.as_ref()),
                span: span_of(stop_line.stop_line_key.span),
            },
            lane_edge: resolver.resolve::<LaneEdgeKind>(
                &stop_line.lane_edge,
                "stopLines[].laneEdge",
                span_of,
            )?,
        }));
    }
    for gate in &overlays.maneuver_gates {
        let transition_index = parse_u32_field(
            &gate.transition_index,
            "maneuverGates[].transitionIndex",
            span_of,
        )?;
        let signal_control = gate
            .signal_control
            .as_ref()
            .map(|group| {
                resolver.resolve::<SignalGroupKind>(group, "maneuverGates[].signalControl", span_of)
            })
            .transpose()?
            .map_or(OwnedSignalControl::None, OwnedSignalControl::Group);
        declarations.push(TypedAstDeclaration::ManeuverGate(ManeuverGateDeclaration {
            header: DeclarationHeader {
                entity_kind: EntityKind::ManeuverGate,
                stable_key: Arc::from(gate.maneuver_gate_key.value.as_ref()),
                span: span_of(gate.maneuver_gate_key.span),
            },
            maneuver_path: resolver.resolve::<ManeuverPathKind>(
                &gate.maneuver_path,
                "maneuverGates[].maneuverPath",
                span_of,
            )?,
            transition_index,
            stop_line: resolver.resolve::<StopLineKind>(
                &gate.stop_line,
                "maneuverGates[].stopLine",
                span_of,
            )?,
            signal_control,
        }));
    }
    for zone in &overlays.waiting_zones {
        let max_occupancy =
            parse_u32_field(&zone.max_occupancy, "waitingZones[].maxOccupancy", span_of)?;
        if max_occupancy == 0 {
            return Err(DiagnosticBundle::single(
                Diagnostic::invalid_waiting_zone_capacity(
                    &zone.waiting_zone_key.value,
                    span_of(zone.waiting_zone_key.span),
                ),
            ));
        }
        declarations.push(TypedAstDeclaration::WaitingZone(WaitingZoneDeclaration {
            header: DeclarationHeader {
                entity_kind: EntityKind::WaitingZone,
                stable_key: Arc::from(zone.waiting_zone_key.value.as_ref()),
                span: span_of(zone.waiting_zone_key.span),
            },
            maneuver_path: resolver.resolve::<ManeuverPathKind>(
                &zone.maneuver_path,
                "waitingZones[].maneuverPath",
                span_of,
            )?,
            entry_gate: resolver.resolve::<ManeuverGateKind>(
                &zone.entry_gate,
                "waitingZones[].entryGate",
                span_of,
            )?,
            release_gate: resolver.resolve::<ManeuverGateKind>(
                &zone.release_gate,
                "waitingZones[].releaseGate",
                span_of,
            )?,
            max_occupancy,
        }));
    }
    Ok(())
}

/// finish ⑤ 的声明维度累计器；公式与 Synthetic 前端增量计数逐类对齐。
#[derive(Default)]
struct FinishCounters {
    declaration_count: u64,
    reference_count: u64,
    relation_occurrence_count: u64,
    identity_field_occurrence_count: u64,
    symbol_count: u64,
    string_item_count: u64,
    string_bytes: u64,
    controlled_string_bytes: u64,
    controlled_structural_bytes: u64,
    maneuver_gate_count: u64,
    waiting_zone_count: u64,
    route_occurrence_count: u64,
}

#[inline]
fn count_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[inline]
fn len_u64(value: &str) -> u64 {
    u64::try_from(value.len()).unwrap_or(u64::MAX)
}

/// 引用规范化拼写字节：namespace + 分隔符 + declaration key。
#[inline]
fn reference_spelling_bytes<K: EntityKindMarker>(reference: &OwnedEntityReference<K>) -> u64 {
    len_u64(&reference.module_namespace)
        .saturating_add(1)
        .saturating_add(len_u64(&reference.declaration_key))
}

impl FinishCounters {
    /// 声明头（namespace + stable key）的字符串维度。
    fn add_header_strings(&mut self, namespace_bytes: u64, stable_key: &Arc<str>) {
        self.string_item_count = self.string_item_count.saturating_add(2);
        self.string_bytes = self
            .string_bytes
            .saturating_add(namespace_bytes)
            .saturating_add(len_u64(stable_key));
        self.controlled_string_bytes = self
            .controlled_string_bytes
            .saturating_add(len_u64(stable_key));
    }

    /// 一次引用出现的字符串维度；引用拼写整体计一个字符串项。
    fn add_reference_strings<K: EntityKindMarker>(&mut self, reference: &OwnedEntityReference<K>) {
        self.string_item_count = self.string_item_count.saturating_add(1);
        self.string_bytes = self
            .string_bytes
            .saturating_add(reference_spelling_bytes(reference));
        self.controlled_string_bytes = self
            .controlled_string_bytes
            .saturating_add(len_u64(&reference.declaration_key));
    }

    /// `kindId` 等本地分类字段的字符串维度。
    fn add_kind_strings(&mut self, kind_id: &Arc<str>) {
        self.string_item_count = self.string_item_count.saturating_add(1);
        self.string_bytes = self.string_bytes.saturating_add(len_u64(kind_id));
        self.controlled_string_bytes = self
            .controlled_string_bytes
            .saturating_add(len_u64(kind_id));
    }

    fn add_structural<T>(&mut self, count: u64) {
        self.controlled_structural_bytes = self
            .controlled_structural_bytes
            .saturating_add(size_bytes::<T>(count));
    }

    /// 只含声明头的同构实体（Junction/SignalGroup/ParkingArea）计数。
    fn add_header_only_entity(
        &mut self,
        namespace_bytes: u64,
        stable_key: &Arc<str>,
        structural_size: u64,
    ) {
        self.identity_field_occurrence_count =
            self.identity_field_occurrence_count.saturating_add(2);
        self.symbol_count = self.symbol_count.saturating_add(1);
        self.add_header_strings(namespace_bytes, stable_key);
        self.controlled_structural_bytes = self
            .controlled_structural_bytes
            .saturating_add(structural_size);
    }

    fn add_declaration(&mut self, declaration: &TypedAstDeclaration, namespace_bytes: u64) {
        self.declaration_count = self.declaration_count.saturating_add(1);
        match declaration {
            TypedAstDeclaration::LaneEdge(declaration) => {
                let successors = count_u64(declaration.successors.len());
                self.reference_count = self.reference_count.saturating_add(successors);
                self.relation_occurrence_count =
                    self.relation_occurrence_count.saturating_add(successors);
                self.identity_field_occurrence_count =
                    self.identity_field_occurrence_count.saturating_add(2);
                self.symbol_count = self.symbol_count.saturating_add(1);
                self.add_header_strings(namespace_bytes, &declaration.header.stable_key);
                for successor in &declaration.successors {
                    self.add_reference_strings(successor);
                }
                self.add_structural::<LaneEdgeDeclaration>(1);
                self.add_structural::<OwnedEntityReference<LaneEdgeKind>>(successors);
            }
            TypedAstDeclaration::RoadCorridor(declaration) => {
                let elements = count_u64(declaration.elements.len());
                let references = elements.saturating_add(1);
                self.reference_count = self.reference_count.saturating_add(references);
                self.relation_occurrence_count =
                    self.relation_occurrence_count.saturating_add(references);
                self.identity_field_occurrence_count =
                    self.identity_field_occurrence_count.saturating_add(2);
                self.symbol_count = self.symbol_count.saturating_add(1);
                self.add_header_strings(namespace_bytes, &declaration.header.stable_key);
                self.add_reference_strings(&declaration.reference_section);
                for element in &declaration.elements {
                    match element {
                        OwnedCorridorElementReference::RoadSection(reference) => {
                            self.add_reference_strings(reference);
                        }
                        OwnedCorridorElementReference::FacilityBand(reference) => {
                            self.add_reference_strings(reference);
                        }
                    }
                }
                self.add_structural::<RoadCorridorDeclaration>(1);
                self.add_structural::<OwnedEntityReference<RoadSectionKind>>(1);
                self.add_structural::<OwnedCorridorElementReference>(elements);
            }
            TypedAstDeclaration::RoadSection(declaration) => {
                let lanes = count_u64(declaration.lanes.len());
                let mut edges = 0_u64;
                let mut groups = 0_u64;
                for lane in &declaration.lanes {
                    edges = edges.saturating_add(count_u64(lane.edge_chain.len()));
                    groups = groups.saturating_add(u64::from(lane.lane_group.is_some()));
                }
                let references = edges.saturating_add(groups);
                self.declaration_count = self.declaration_count.saturating_add(lanes);
                self.reference_count = self.reference_count.saturating_add(references);
                self.relation_occurrence_count = self
                    .relation_occurrence_count
                    .saturating_add(lanes.saturating_add(references));
                self.identity_field_occurrence_count = self
                    .identity_field_occurrence_count
                    .saturating_add(3_u64.saturating_mul(lanes.saturating_add(1)));
                self.symbol_count = self.symbol_count.saturating_add(lanes.saturating_add(1));
                self.add_header_strings(namespace_bytes, &declaration.header.stable_key);
                self.add_kind_strings(&declaration.kind_id);
                for lane in &declaration.lanes {
                    self.add_header_strings(namespace_bytes, &lane.header.stable_key);
                    for edge in &lane.edge_chain {
                        self.add_reference_strings(edge);
                    }
                    if let Some(lane_group) = &lane.lane_group {
                        self.add_reference_strings(lane_group);
                    }
                }
                self.add_structural::<RoadSectionDeclaration>(1);
                self.add_structural::<AuthoringLaneDeclaration>(lanes);
                self.add_structural::<OwnedEntityReference<LaneEdgeKind>>(edges);
                self.add_structural::<OwnedEntityReference<LaneGroupKind>>(groups);
            }
            TypedAstDeclaration::LaneGroup(declaration) => {
                self.reference_count = self.reference_count.saturating_add(1);
                self.relation_occurrence_count = self.relation_occurrence_count.saturating_add(1);
                self.identity_field_occurrence_count =
                    self.identity_field_occurrence_count.saturating_add(3);
                self.symbol_count = self.symbol_count.saturating_add(1);
                self.add_header_strings(namespace_bytes, &declaration.header.stable_key);
                self.add_reference_strings(&declaration.road_section);
                self.add_structural::<LaneGroupDeclaration>(1);
                self.add_structural::<OwnedEntityReference<RoadSectionKind>>(1);
            }
            TypedAstDeclaration::FacilityBand(declaration) => {
                self.identity_field_occurrence_count =
                    self.identity_field_occurrence_count.saturating_add(3);
                self.symbol_count = self.symbol_count.saturating_add(1);
                self.add_header_strings(namespace_bytes, &declaration.header.stable_key);
                self.add_kind_strings(&declaration.kind_id);
                self.add_structural::<FacilityBandDeclaration>(1);
            }
            TypedAstDeclaration::Junction(declaration) => {
                self.add_header_only_entity(
                    namespace_bytes,
                    &declaration.header.stable_key,
                    size_bytes::<JunctionDeclaration>(1),
                );
            }
            TypedAstDeclaration::Movement(declaration) => {
                self.reference_count = self.reference_count.saturating_add(1);
                self.relation_occurrence_count = self.relation_occurrence_count.saturating_add(1);
                self.identity_field_occurrence_count =
                    self.identity_field_occurrence_count.saturating_add(5);
                self.symbol_count = self.symbol_count.saturating_add(1);
                self.add_header_strings(namespace_bytes, &declaration.header.stable_key);
                self.add_reference_strings(&declaration.junction);
                self.string_item_count = self.string_item_count.saturating_add(2);
                let approach_bytes = len_u64(&declaration.directed_entry_approach_key)
                    .saturating_add(len_u64(&declaration.directed_exit_approach_key));
                self.string_bytes = self.string_bytes.saturating_add(approach_bytes);
                self.controlled_string_bytes =
                    self.controlled_string_bytes.saturating_add(approach_bytes);
                self.add_structural::<MovementDeclaration>(1);
                self.add_structural::<OwnedEntityReference<JunctionKind>>(1);
            }
            TypedAstDeclaration::ManeuverPath(declaration) => {
                let internal = count_u64(declaration.internal_edges.len());
                let references = internal.saturating_add(3);
                self.reference_count = self.reference_count.saturating_add(references);
                self.relation_occurrence_count = self
                    .relation_occurrence_count
                    .saturating_add(internal.saturating_add(3));
                self.identity_field_occurrence_count =
                    self.identity_field_occurrence_count.saturating_add(5);
                self.symbol_count = self.symbol_count.saturating_add(1);
                self.add_header_strings(namespace_bytes, &declaration.header.stable_key);
                self.add_reference_strings(&declaration.movement);
                self.add_reference_strings(&declaration.entry_edge);
                for edge in &declaration.internal_edges {
                    self.add_reference_strings(edge);
                }
                self.add_reference_strings(&declaration.exit_edge);
                self.add_structural::<ManeuverPathDeclaration>(1);
                self.add_structural::<OwnedEntityReference<MovementKind>>(1);
                self.add_structural::<OwnedEntityReference<LaneEdgeKind>>(
                    internal.saturating_add(2),
                );
            }
            TypedAstDeclaration::StopLine(declaration) => {
                self.reference_count = self.reference_count.saturating_add(1);
                self.relation_occurrence_count = self.relation_occurrence_count.saturating_add(1);
                self.identity_field_occurrence_count =
                    self.identity_field_occurrence_count.saturating_add(2);
                self.symbol_count = self.symbol_count.saturating_add(1);
                self.add_header_strings(namespace_bytes, &declaration.header.stable_key);
                self.add_reference_strings(&declaration.lane_edge);
                self.add_structural::<StopLineDeclaration>(1);
                self.add_structural::<OwnedEntityReference<LaneEdgeKind>>(1);
            }
            TypedAstDeclaration::ManeuverGate(declaration) => {
                let has_group = u64::from(matches!(
                    declaration.signal_control,
                    OwnedSignalControl::Group(_)
                ));
                let references = has_group.saturating_add(2);
                self.reference_count = self.reference_count.saturating_add(references);
                self.relation_occurrence_count =
                    self.relation_occurrence_count.saturating_add(references);
                self.identity_field_occurrence_count =
                    self.identity_field_occurrence_count.saturating_add(3);
                self.symbol_count = self.symbol_count.saturating_add(1);
                self.maneuver_gate_count = self.maneuver_gate_count.saturating_add(1);
                self.add_header_strings(namespace_bytes, &declaration.header.stable_key);
                self.add_reference_strings(&declaration.maneuver_path);
                self.add_reference_strings(&declaration.stop_line);
                if let OwnedSignalControl::Group(group) = &declaration.signal_control {
                    self.add_reference_strings(group);
                }
                self.add_structural::<ManeuverGateDeclaration>(1);
                self.add_structural::<OwnedEntityReference<ManeuverPathKind>>(1);
                self.add_structural::<OwnedEntityReference<StopLineKind>>(1);
                self.add_structural::<OwnedEntityReference<SignalGroupKind>>(has_group);
            }
            TypedAstDeclaration::WaitingZone(declaration) => {
                self.reference_count = self.reference_count.saturating_add(3);
                self.relation_occurrence_count = self.relation_occurrence_count.saturating_add(3);
                self.identity_field_occurrence_count =
                    self.identity_field_occurrence_count.saturating_add(3);
                self.symbol_count = self.symbol_count.saturating_add(1);
                self.waiting_zone_count = self.waiting_zone_count.saturating_add(1);
                self.add_header_strings(namespace_bytes, &declaration.header.stable_key);
                self.add_reference_strings(&declaration.maneuver_path);
                self.add_reference_strings(&declaration.entry_gate);
                self.add_reference_strings(&declaration.release_gate);
                self.add_structural::<WaitingZoneDeclaration>(1);
                self.add_structural::<OwnedEntityReference<ManeuverPathKind>>(1);
                self.add_structural::<OwnedEntityReference<ManeuverGateKind>>(2);
            }
            TypedAstDeclaration::StaticRoute(declaration) => {
                let occurrences = count_u64(declaration.edge_sequence.len());
                self.reference_count = self.reference_count.saturating_add(occurrences);
                self.relation_occurrence_count =
                    self.relation_occurrence_count.saturating_add(occurrences);
                self.identity_field_occurrence_count =
                    self.identity_field_occurrence_count.saturating_add(2);
                self.symbol_count = self.symbol_count.saturating_add(1);
                self.route_occurrence_count =
                    self.route_occurrence_count.saturating_add(occurrences);
                self.add_header_strings(namespace_bytes, &declaration.header.stable_key);
                for edge in &declaration.edge_sequence {
                    self.add_reference_strings(edge);
                }
                self.add_structural::<StaticRouteDeclaration>(1);
                self.add_structural::<OwnedEntityReference<LaneEdgeKind>>(occurrences);
            }
            TypedAstDeclaration::SignalGroup(declaration) => {
                self.add_header_only_entity(
                    namespace_bytes,
                    &declaration.header.stable_key,
                    size_bytes::<SignalGroupDeclaration>(1),
                );
            }
            TypedAstDeclaration::SignalController(declaration) => {
                let phases = count_u64(declaration.phases.len());
                let groups = count_u64(declaration.signal_groups.len());
                let states = declaration.phases.iter().fold(0_u64, |total, phase| {
                    total.saturating_add(count_u64(phase.states.len()))
                });
                let references = groups.saturating_add(states);
                self.declaration_count = self.declaration_count.saturating_add(phases);
                self.reference_count = self.reference_count.saturating_add(references);
                self.relation_occurrence_count = self
                    .relation_occurrence_count
                    .saturating_add(groups.saturating_add(phases).saturating_add(states));
                self.identity_field_occurrence_count = self
                    .identity_field_occurrence_count
                    .saturating_add(2_u64.saturating_add(3_u64.saturating_mul(phases)));
                self.symbol_count = self.symbol_count.saturating_add(phases.saturating_add(1));
                self.add_header_strings(namespace_bytes, &declaration.header.stable_key);
                for group in &declaration.signal_groups {
                    self.add_reference_strings(group);
                }
                for phase in &declaration.phases {
                    self.add_header_strings(namespace_bytes, &phase.header.stable_key);
                    for state in &phase.states {
                        self.add_reference_strings(&state.signal_group);
                    }
                }
                self.add_structural::<SignalControllerDeclaration>(1);
                self.add_structural::<OwnedEntityReference<SignalGroupKind>>(
                    groups.saturating_add(states),
                );
                self.add_structural::<SignalPhaseDeclaration>(phases);
                self.add_structural::<SignalGroupStateDeclaration>(states);
            }
            TypedAstDeclaration::ParkingArea(declaration) => {
                self.add_header_only_entity(
                    namespace_bytes,
                    &declaration.header.stable_key,
                    size_bytes::<ParkingAreaDeclaration>(1),
                );
            }
            TypedAstDeclaration::ParkingSpace(declaration) => {
                let has_area = u64::from(declaration.parking_area.is_some());
                let references = has_area.saturating_add(2);
                self.reference_count = self.reference_count.saturating_add(references);
                self.relation_occurrence_count =
                    self.relation_occurrence_count.saturating_add(references);
                self.identity_field_occurrence_count =
                    self.identity_field_occurrence_count.saturating_add(2);
                self.symbol_count = self.symbol_count.saturating_add(1);
                self.add_header_strings(namespace_bytes, &declaration.header.stable_key);
                if let Some(area) = &declaration.parking_area {
                    self.add_reference_strings(area);
                }
                self.add_reference_strings(&declaration.entry.lane_edge);
                self.add_reference_strings(&declaration.exit.lane_edge);
                self.add_structural::<ParkingSpaceDeclaration>(1);
            }
            TypedAstDeclaration::ParticipantClass(declaration) => {
                let has_extends = u64::from(declaration.extends.is_some());
                self.reference_count = self.reference_count.saturating_add(has_extends);
                self.relation_occurrence_count =
                    self.relation_occurrence_count.saturating_add(has_extends);
                self.identity_field_occurrence_count =
                    self.identity_field_occurrence_count.saturating_add(2);
                self.symbol_count = self.symbol_count.saturating_add(1);
                self.add_header_strings(namespace_bytes, &declaration.header.stable_key);
                if let Some(parent) = &declaration.extends {
                    self.add_reference_strings(parent);
                }
                self.add_structural::<ParticipantClassDeclaration>(1);
                self.add_structural::<OwnedEntityReference<ParticipantClassKind>>(has_extends);
            }
            TypedAstDeclaration::VehicleProfile(declaration) => {
                self.reference_count = self.reference_count.saturating_add(1);
                self.relation_occurrence_count = self.relation_occurrence_count.saturating_add(1);
                self.identity_field_occurrence_count =
                    self.identity_field_occurrence_count.saturating_add(2);
                self.symbol_count = self.symbol_count.saturating_add(1);
                self.add_header_strings(namespace_bytes, &declaration.header.stable_key);
                self.add_reference_strings(&declaration.participant_class);
                self.add_structural::<VehicleProfileDeclaration>(1);
                self.add_structural::<OwnedEntityReference<ParticipantClassKind>>(1);
            }
            TypedAstDeclaration::CanonicalFrame(declaration) => {
                let geometries = count_u64(declaration.lane_edge_geometries.len());
                let points = declaration
                    .lane_edge_geometries
                    .iter()
                    .fold(0_u64, |total, g| {
                        total.saturating_add(count_u64(g.centerline_points.len()))
                    });
                self.reference_count = self.reference_count.saturating_add(geometries);
                self.relation_occurrence_count =
                    self.relation_occurrence_count.saturating_add(geometries);
                self.identity_field_occurrence_count =
                    self.identity_field_occurrence_count.saturating_add(2);
                self.symbol_count = self.symbol_count.saturating_add(1);
                self.add_header_strings(namespace_bytes, &declaration.header.stable_key);
                for geometry in &declaration.lane_edge_geometries {
                    self.add_reference_strings(&geometry.lane_edge);
                }
                self.add_structural::<CanonicalFrameDeclaration>(1);
                self.add_structural::<LaneEdgeGeometryDeclaration>(geometries);
                self.add_structural::<crate::declaration::CanonicalPoint3F32Input>(points);
            }
            TypedAstDeclaration::AccessRule(declaration) => {
                let classes = count_u64(declaration.participant_classes.len());
                let references = classes.saturating_add(1);
                let (regulation_strings, regulation_bytes) = match &declaration.regulation {
                    Some(regulation) => {
                        let bytes = len_u64(&regulation.jurisdiction)
                            .saturating_add(len_u64(&regulation.version))
                            .saturating_add(regulation.source.as_deref().map_or(0, len_u64));
                        (
                            2_u64.saturating_add(u64::from(regulation.source.is_some())),
                            bytes,
                        )
                    }
                    None => (0, 0),
                };
                self.reference_count = self.reference_count.saturating_add(references);
                self.relation_occurrence_count =
                    self.relation_occurrence_count.saturating_add(references);
                self.identity_field_occurrence_count =
                    self.identity_field_occurrence_count.saturating_add(2);
                self.symbol_count = self.symbol_count.saturating_add(1);
                self.add_header_strings(namespace_bytes, &declaration.header.stable_key);
                match &declaration.target {
                    OwnedAccessRuleTarget::LaneEdge(reference) => {
                        self.add_reference_strings(reference);
                    }
                    OwnedAccessRuleTarget::LaneGroup(reference) => {
                        self.add_reference_strings(reference);
                    }
                    OwnedAccessRuleTarget::RoadSection(reference) => {
                        self.add_reference_strings(reference);
                    }
                    OwnedAccessRuleTarget::ManeuverPath(reference) => {
                        self.add_reference_strings(reference);
                    }
                    OwnedAccessRuleTarget::FacilityBand(reference) => {
                        self.add_reference_strings(reference);
                    }
                }
                for class in &declaration.participant_classes {
                    self.add_reference_strings(class);
                }
                self.string_item_count = self.string_item_count.saturating_add(regulation_strings);
                self.string_bytes = self.string_bytes.saturating_add(regulation_bytes);
                self.controlled_string_bytes = self
                    .controlled_string_bytes
                    .saturating_add(regulation_bytes);
                self.add_structural::<AccessRuleDeclaration>(1);
                self.add_structural::<OwnedAccessRuleTarget>(1);
                self.add_structural::<OwnedEntityReference<ParticipantClassKind>>(classes);
                self.add_structural::<OwnedAccessRegulation>(u64::from(
                    declaration.regulation.is_some(),
                ));
            }
            TypedAstDeclaration::GeometryReferenceLine(intent) => {
                // intent 不派生身份、不进入符号表；引用计入 reference 维度，
                // 字符串与结构字节仿同形状声明。
                self.reference_count = self.reference_count.saturating_add(1);
                self.add_header_strings(namespace_bytes, &intent.road_key);
                self.add_reference_strings(&intent.frame);
                self.add_structural::<GeometryReferenceLineIntent>(1);
            }
            TypedAstDeclaration::GeometryCrossSectionSpan(intent) => {
                let offsets = count_u64(intent.offsets.len());
                self.reference_count = self.reference_count.saturating_add(2);
                self.add_header_strings(namespace_bytes, &intent.span_key);
                self.add_reference_strings(&intent.frame);
                self.add_reference_strings(&intent.corridor);
                for offset in &intent.offsets {
                    self.string_item_count = self.string_item_count.saturating_add(1);
                    self.string_bytes = self.string_bytes.saturating_add(len_u64(&offset.key));
                    self.controlled_string_bytes = self
                        .controlled_string_bytes
                        .saturating_add(len_u64(&offset.key));
                }
                self.add_structural::<GeometryCrossSectionSpanIntent>(1);
                self.add_structural::<GeometryOffsetIntent>(offsets);
                self.add_structural::<OwnedEntityReference<CanonicalFrameKind>>(1);
            }
            TypedAstDeclaration::GeometryConnection(intent) => {
                let internal = count_u64(intent.internal_edges.len());
                self.reference_count = self
                    .reference_count
                    .saturating_add(internal.saturating_add(4));
                self.add_reference_strings(&intent.junction);
                self.add_reference_strings(&intent.maneuver_path);
                self.add_reference_strings(&intent.entry_edge);
                for edge in &intent.internal_edges {
                    self.add_reference_strings(edge);
                }
                self.add_reference_strings(&intent.exit_edge);
                self.add_structural::<GeometryConnectionIntent>(1);
                self.add_structural::<OwnedEntityReference<LaneEdgeKind>>(internal);
            }
            TypedAstDeclaration::GeometryInternalEdge(intent) => {
                // intent 不派生身份、不进入符号表；junction 引用计入 reference 维度，
                // 键字符串与结构字节仿同形状 intent。
                self.reference_count = self.reference_count.saturating_add(1);
                self.add_header_strings(namespace_bytes, &intent.key);
                self.add_reference_strings(&intent.junction);
                self.add_structural::<GeometryInternalEdgeIntent>(1);
                self.add_structural::<OwnedEntityReference<JunctionKind>>(1);
            }
        }
    }
}

/// §7.1 的 wire 记录口径 `typed_ast_record_count`：模块头 1 + import + 各 wire 记录；
/// 控制点不算记录。
fn finish_typed_ast_record_count(parsed: &ParsedGeometryDocument) -> u64 {
    let mut total = 1_u64
        .saturating_add(count_u64(parsed.module.imports.len()))
        .saturating_add(count_u64(parsed.frames.len()))
        .saturating_add(count_u64(parsed.roads.len()));
    for road in &parsed.roads {
        total = total
            .saturating_add(count_u64(road.reference_line.segments.len()))
            .saturating_add(count_u64(road.cross_section_spans.len()));
        for span in &road.cross_section_spans {
            total = total
                .saturating_add(count_u64(span.road_sections.len()))
                .saturating_add(count_u64(span.facility_bands.len()));
            for section in &span.road_sections {
                total = total
                    .saturating_add(count_u64(section.lanes.len()))
                    .saturating_add(count_u64(section.lane_groups.len()));
            }
        }
    }
    for junction in &parsed.junctions {
        total = total
            .saturating_add(1)
            .saturating_add(count_u64(junction.connections.len()))
            .saturating_add(count_u64(junction.internal_edges.len()));
    }
    let overlays = &parsed.overlays;
    total = total
        .saturating_add(count_u64(overlays.signal_groups.len()))
        .saturating_add(count_u64(overlays.signal_controllers.len()))
        .saturating_add(count_u64(overlays.parking_areas.len()))
        .saturating_add(count_u64(overlays.parking_spaces.len()))
        .saturating_add(count_u64(overlays.participant_classes.len()))
        .saturating_add(count_u64(overlays.vehicle_profiles.len()))
        .saturating_add(count_u64(overlays.access_rules.len()))
        .saturating_add(count_u64(overlays.static_routes.len()))
        .saturating_add(count_u64(overlays.stop_lines.len()))
        .saturating_add(count_u64(overlays.maneuver_gates.len()))
        .saturating_add(count_u64(overlays.waiting_zones.len()));
    for controller in &overlays.signal_controllers {
        total = total.saturating_add(count_u64(controller.phases.len()));
        for phase in &controller.phases {
            total = total.saturating_add(count_u64(phase.states.len()));
        }
    }
    total
}

/// finish ⑤：按 Synthetic 同形状公式遍历冻结声明计数，并叠加 Geometry 特有
/// 的 payload 点字节与单文档来源字节。
fn finish_resource_counts(
    parsed: &ParsedGeometryDocument,
    payload: &schema::FrozenGeometryPayload,
    declarations: &[TypedAstDeclaration],
    header: &FinishHeader,
    source_document_key: &Arc<str>,
    source_record_byte_len: u32,
) -> ModuleResourceCounts {
    let namespace_bytes = len_u64(&header.namespace);
    let mut counters = FinishCounters::default();
    // 模块头 resident：ns 与 document key 进入 string 维度，generator/provenance
    // 只进入 controlled 字符串（对齐 Synthetic builder 初始化）。
    counters.string_item_count = 2;
    counters.string_bytes = namespace_bytes.saturating_add(len_u64(source_document_key));
    counters.controlled_string_bytes = counters
        .string_bytes
        .saturating_add(len_u64(&header.provenance.generator_build_id))
        .saturating_add(len_u64(&header.provenance.description));
    for import in &header.imports {
        counters.string_item_count = counters.string_item_count.saturating_add(1);
        let bytes = len_u64(&import.namespace);
        counters.string_bytes = counters.string_bytes.saturating_add(bytes);
        counters.controlled_string_bytes = counters.controlled_string_bytes.saturating_add(bytes);
    }
    for declaration in declarations {
        counters.add_declaration(declaration, namespace_bytes);
    }
    let payload_bytes =
        size_bytes::<schema::FrozenLateralCurve>(count_u64(payload.lateral_curves.len()))
            .saturating_add(payload.lateral_curves.iter().fold(0_u64, |total, curve| {
                total.saturating_add(size_bytes::<schema::FrozenCanonicalPoint>(count_u64(
                    curve.points.len(),
                )))
            }))
            .saturating_add(size_bytes::<schema::FrozenInternalEdgeCurve>(count_u64(
                payload.internal_edge_curves.len(),
            )))
            .saturating_add(
                payload
                    .internal_edge_curves
                    .iter()
                    .fold(0_u64, |total, curve| {
                        total.saturating_add(size_bytes::<schema::FrozenCanonicalPoint>(count_u64(
                            curve.points.len(),
                        )))
                    }),
            );
    let controlled_live_bytes = counters
        .controlled_string_bytes
        .saturating_add(counters.controlled_structural_bytes)
        .saturating_add(size_bytes::<SourceDocumentDescriptor>(1))
        .saturating_add(payload_bytes);
    ModuleResourceCounts {
        source_bytes: u64::from(source_record_byte_len),
        declaration_count: counters.declaration_count,
        typed_ast_record_count: finish_typed_ast_record_count(parsed),
        reference_count: counters.reference_count,
        relation_occurrence_count: counters.relation_occurrence_count,
        identity_field_occurrence_count: counters.identity_field_occurrence_count,
        symbol_count: counters.symbol_count,
        string_item_count: counters.string_item_count,
        string_bytes: counters.string_bytes,
        maneuver_gate_count: counters.maneuver_gate_count,
        waiting_zone_count: counters.waiting_zone_count,
        route_occurrence_count: counters.route_occurrence_count,
        geometry_point_count: payload.geometry_point_count,
        controlled_live_bytes,
    }
}

/// finish ⑤ 的模块级上限检查；`SourceBytesPerModule` 与 `GeometryPointCount`
/// 已在 builder 构造与 numeric freeze 内闭合，其余维度在此先到先得单诊断。
fn check_finish_limits(
    limits: &CompileLimits,
    counts: &ModuleResourceCounts,
    import_count: u64,
    namespace: &Arc<str>,
    declaration_span: &SourceSpan,
) -> Result<(), DiagnosticBundle> {
    let namespace: Box<str> = namespace.as_ref().into();
    for (dimension, observed) in [
        (CompileLimitDimension::ImportEdgeCount, import_count),
        (
            CompileLimitDimension::DeclarationCount,
            counts.declaration_count,
        ),
        (
            CompileLimitDimension::TypedAstRecordCount,
            counts.typed_ast_record_count,
        ),
        (
            CompileLimitDimension::ReferenceCount,
            counts.reference_count,
        ),
        (
            CompileLimitDimension::RelationOccurrenceCount,
            counts.relation_occurrence_count,
        ),
        (
            CompileLimitDimension::IdentityFieldOccurrenceCount,
            counts.identity_field_occurrence_count,
        ),
        (CompileLimitDimension::SymbolCount, counts.symbol_count),
        (
            CompileLimitDimension::StringItemCount,
            counts.string_item_count,
        ),
        (CompileLimitDimension::TotalStringBytes, counts.string_bytes),
        (
            CompileLimitDimension::ManeuverGateCount,
            counts.maneuver_gate_count,
        ),
        (
            CompileLimitDimension::WaitingZoneCount,
            counts.waiting_zone_count,
        ),
        (
            CompileLimitDimension::RouteOccurrenceCount,
            counts.route_occurrence_count,
        ),
        (
            CompileLimitDimension::CompilerControlledLiveBytes,
            counts.controlled_live_bytes,
        ),
    ] {
        let limit = limits.value(dimension);
        if observed > limit {
            return Err(DiagnosticBundle::single(
                Diagnostic::compile_limit_exceeded_at(
                    dimension,
                    limit,
                    observed,
                    Some(declaration_span.clone()),
                    Some(namespace.clone()),
                ),
            ));
        }
    }
    Ok(())
}

fn schema_diagnostic(
    error: SchemaError,
    source_document_key: &Arc<str>,
    line_index: Option<&LineIndex>,
) -> DiagnosticBundle {
    // §7.2 阶段 1 parser 资源错误：StageScratchBytes 超限在 generic 映射前单独
    // 映射为资源上限诊断。
    if let SchemaErrorKind::Json(json_error) = &error.kind
        && let JsonErrorKind::StageScratchExceeded(exceeded) = json_error.kind
    {
        return DiagnosticBundle::single(Diagnostic::compile_limit_exceeded(
            CompileLimitDimension::StageScratchBytes,
            exceeded.limit,
            exceeded.observed,
        ));
    }
    let primary_span = line_index.map_or_else(
        || crate::SourceSpan::point(Arc::clone(source_document_key), 1, 1),
        |index| index.source_span(source_document_key, error.span),
    );
    let (violation, field, actual, expected) = schema_error_parts(error.kind);
    DiagnosticBundle::single(Diagnostic::invalid_geometry_document(
        violation,
        field.as_deref(),
        actual.as_deref(),
        expected.as_deref(),
        primary_span,
    ))
}

type SchemaDiagnosticParts = (
    GeometryDocumentViolation,
    Option<Box<str>>,
    Option<Box<str>>,
    Option<Box<str>>,
);

fn schema_error_parts(kind: SchemaErrorKind) -> SchemaDiagnosticParts {
    use GeometryDocumentViolation::{ClosedShape, Encoding, FieldValue, NestingDepth, Syntax};

    match kind {
        SchemaErrorKind::Json(error) => {
            let violation = match error.kind {
                JsonErrorKind::Utf8Bom
                | JsonErrorKind::InvalidUtf8
                | JsonErrorKind::SourcePositionOverflow => Encoding,
                JsonErrorKind::NestingDepthExceeded => NestingDepth,
                // schema_diagnostic 已把 StageScratchExceeded 特判为资源上限诊断，
                // 此臂仅为 match 穷尽性兜底。
                JsonErrorKind::StageScratchExceeded(_)
                | JsonErrorKind::UnexpectedEnd
                | JsonErrorKind::UnexpectedByte
                | JsonErrorKind::InvalidStringEscape
                | JsonErrorKind::InvalidUnicodeEscape
                | JsonErrorKind::UnescapedControlCharacter
                | JsonErrorKind::InvalidNumber
                | JsonErrorKind::TrailingBytes => Syntax,
            };
            (
                violation,
                None,
                Some(format!("{:?}", error.kind).into()),
                None,
            )
        }
        SchemaErrorKind::UnknownField(field) => {
            (ClosedShape, None, Some(field), Some("known field".into()))
        }
        SchemaErrorKind::DuplicateField(field) => (
            ClosedShape,
            Some(field.into()),
            Some("duplicate".into()),
            Some("single occurrence".into()),
        ),
        SchemaErrorKind::MissingField(field) => (
            ClosedShape,
            Some(field.into()),
            Some("missing".into()),
            Some("required".into()),
        ),
        SchemaErrorKind::UnexpectedConstant { field, expected } => {
            (FieldValue, Some(field.into()), None, Some(expected.into()))
        }
        SchemaErrorKind::InvalidToken(field) => (
            FieldValue,
            Some(field.into()),
            Some("invalid token".into()),
            None,
        ),
        SchemaErrorKind::EmptyString(field) => (
            FieldValue,
            Some(field.into()),
            Some("empty string".into()),
            Some("non-empty string".into()),
        ),
        SchemaErrorKind::DuplicateImport(value) => (
            FieldValue,
            Some("module.imports".into()),
            Some(value),
            Some("unique import".into()),
        ),
        SchemaErrorKind::DuplicateArrayItem { field, value } => (
            FieldValue,
            Some(field.into()),
            Some(value),
            Some("unique item".into()),
        ),
        SchemaErrorKind::InvalidDigest(field) => (
            FieldValue,
            Some(field.into()),
            Some("invalid digest".into()),
            Some("64 lowercase hexadecimal characters".into()),
        ),
        SchemaErrorKind::InvalidRandomSeed => (
            FieldValue,
            Some("randomSeed".into()),
            Some("invalid unsigned integer string".into()),
            None,
        ),
        SchemaErrorKind::InvalidProvenanceKind(value) => (
            FieldValue,
            Some("provenance.kind".into()),
            Some(value),
            Some("direct or generated".into()),
        ),
        SchemaErrorKind::FieldNotAllowedForProvenance { field, kind } => (
            ClosedShape,
            Some(field.into()),
            Some(kind.into()),
            Some("field allowed for provenance variant".into()),
        ),
        SchemaErrorKind::FieldNotAllowedForVariant { field, variant } => (
            ClosedShape,
            Some(field.into()),
            Some(variant.into()),
            Some("field allowed for variant".into()),
        ),
        SchemaErrorKind::InvalidEnum { field, value } => (
            FieldValue,
            Some(field.into()),
            Some(value),
            Some("known enum value".into()),
        ),
        SchemaErrorKind::EmptyArray(field) => (
            FieldValue,
            Some(field.into()),
            Some("empty array".into()),
            Some("non-empty array".into()),
        ),
        SchemaErrorKind::WrongArrayLength {
            field,
            expected,
            actual,
        } => (
            FieldValue,
            Some(field.into()),
            Some(actual.to_string().into()),
            Some(expected.to_string().into()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use crate::declaration::{
        AuthoringLaneDeclaration, CanonicalFrameDeclaration, FacilityKindCategory,
        FacilityKindViolation, GeometryConnectionIntent, GeometryCrossSectionSpanIntent,
        GeometryInternalEdgeIntent, GeometryOffsetIntent, GeometryOffsetIntentKind,
        GeometryReferenceLineIntent, JunctionDeclaration, LaneEdgeDeclaration,
        ManeuverPathDeclaration, MovementDeclaration, OwnedAccessRuleTarget,
        OwnedCorridorElementReference, OwnedEntityReference, OwnedSignalControl,
        RoadCorridorDeclaration, RoadSectionDeclaration, TypedAstDeclaration,
    };
    use crate::{
        AccessEffect, CompileLimitDimension, CompileLimits, DiagnosticBundle, DiagnosticCode,
        DiagnosticPayload, GeometryDocumentViolation, SignalAspect, SourceLanguage,
    };
    use laneflow_static_contract::{
        CanonicalFrameKind, EntityKind, JunctionKind, LaneEdgeKind, LaneGroupKind, MovementKind,
        RoadSectionKind,
    };

    use super::super::resources::size_bytes;
    use super::schema::{FrozenCanonicalPoint, FrozenInternalEdgeCurve, FrozenLateralCurve};
    use super::{
        GEOMETRY_FRONTEND_VERSION, GeometryAccuracyProfile, GeometryDirectionProfile,
        GeometryDocumentInput, GeometryModule, GeometryModuleBuilder,
        direct_parameters_and_inputs_digest, direct_source_frontend_options_digest,
        frontend_options_digest,
    };

    #[test]
    fn accuracy_profiles_freeze_names_codes_and_budgets() {
        let cases = [
            (
                GeometryAccuracyProfile::Fine2Cm,
                "fine-2cm-v1",
                1,
                0.02_f64,
                0.01_f64,
            ),
            (
                GeometryAccuracyProfile::Balanced5Cm,
                "balanced-5cm-v1",
                2,
                0.05_f64,
                0.025_f64,
            ),
            (
                GeometryAccuracyProfile::Compact10Cm,
                "compact-10cm-v1",
                3,
                0.10_f64,
                0.05_f64,
            ),
        ];

        for (profile, name, code, maximum, subdivision) in cases {
            assert_eq!(profile.stable_name(), name);
            assert_eq!(profile.code(), code);
            assert_eq!(
                profile.max_position_error_meters().to_bits(),
                maximum.to_bits()
            );
            assert_eq!(
                profile.subdivision_budget_meters().to_bits(),
                subdivision.to_bits()
            );
        }
    }

    #[test]
    fn direction_profiles_freeze_names_codes_and_threshold_bits() {
        let cases = [
            (
                GeometryDirectionProfile::Smooth1Deg,
                "smooth-1deg-v1",
                1,
                1.0_f64,
                0x3fef_ff60_4bfa_d7c5,
                0x3fef_fd81_3c5f_82b4,
            ),
            (
                GeometryDirectionProfile::Balanced2Deg,
                "balanced-2deg-v1",
                2,
                2.0_f64,
                0x3fef_fd81_3c5f_82b4,
                0x3fef_f605_b8b8_7ffc,
            ),
            (
                GeometryDirectionProfile::Compact5Deg,
                "compact-5deg-v1",
                3,
                5.0_f64,
                0x3fef_f069_da0c_0ad2,
                0x3fef_c1c5_c640_8e0c,
            ),
        ];

        for (profile, name, code, maximum, candidate_bits, runtime_bits) in cases {
            assert_eq!(profile.stable_name(), name);
            assert_eq!(profile.code(), code);
            assert_eq!(
                profile.max_runtime_direction_jump_degrees().to_bits(),
                maximum.to_bits()
            );
            assert_eq!(profile.candidate_cos_squared().to_bits(), candidate_bits);
            assert_eq!(profile.runtime_cos_squared().to_bits(), runtime_bits);
        }
    }

    #[test]
    fn document_input_only_borrows_exact_caller_values() {
        let bytes = br#"{"geometryVersion":"1"}"#;
        let input = GeometryDocumentInput::new("source/main", bytes, Some("authoring/main.json"));

        assert_eq!(input.source_document_key(), "source/main");
        assert_eq!(input.source_bytes(), bytes);
        assert_eq!(input.display_source(), Some("authoring/main.json"));
    }

    #[test]
    fn direct_provenance_digests_match_frozen_known_vectors() {
        assert_eq!(
            direct_parameters_and_inputs_digest(),
            decode_digest("b5e975087623986587468bcbb2df36fa17fe6aaf4367d5faf55a01aded4c544b")
        );
        assert_eq!(
            direct_source_frontend_options_digest(),
            decode_digest("4cc9e978718828ddf14adcd1c2d5887021bfeacaf922e700b47e0778e7d638a9")
        );
    }

    #[test]
    fn direct_frontend_options_digest_covers_all_nine_profile_combinations() {
        let source_digest = direct_source_frontend_options_digest();
        let cases = [
            (
                GeometryAccuracyProfile::Fine2Cm,
                GeometryDirectionProfile::Smooth1Deg,
                "2cf7baa3e8ffd532fc8def1511d681935a3c8a67ccc96a3222de3c4fbff52c35",
            ),
            (
                GeometryAccuracyProfile::Fine2Cm,
                GeometryDirectionProfile::Balanced2Deg,
                "e17fc963d368d0dd9184975d7473b64a6bcc320c3bbce87e2bd7f6a9cf55d0da",
            ),
            (
                GeometryAccuracyProfile::Fine2Cm,
                GeometryDirectionProfile::Compact5Deg,
                "cf77c05e19c19d5956b9e5bb2b28539735557a5fbe56200810b3103e2bb615c1",
            ),
            (
                GeometryAccuracyProfile::Balanced5Cm,
                GeometryDirectionProfile::Smooth1Deg,
                "890d3928a67002c9a3d780666e7f0d6cbef6a67ecc561bdb6fefa31d57120545",
            ),
            (
                GeometryAccuracyProfile::Balanced5Cm,
                GeometryDirectionProfile::Balanced2Deg,
                "f920e44897beadf8d33fa48fc59dafcc3e15bc39c1428e21aa12efac3078b563",
            ),
            (
                GeometryAccuracyProfile::Balanced5Cm,
                GeometryDirectionProfile::Compact5Deg,
                "a83765fa49f6bcc1ee4f83bae2653029dc886edd257560e476bebcd3a6ad6e27",
            ),
            (
                GeometryAccuracyProfile::Compact10Cm,
                GeometryDirectionProfile::Smooth1Deg,
                "fe20db07f4f009f4effc43356720eb04e1cf3454074ab5701529d6fa32eb0cd5",
            ),
            (
                GeometryAccuracyProfile::Compact10Cm,
                GeometryDirectionProfile::Balanced2Deg,
                "ef6e9dba63188e4b4ab8ad3e071cbf3f64943d4fd095af5aa64fad1dae8555f1",
            ),
            (
                GeometryAccuracyProfile::Compact10Cm,
                GeometryDirectionProfile::Compact5Deg,
                "94a20f577254d890382b04f9ef2ccc56c816b1544d0bbfe6eb3eabe67bd723ba",
            ),
        ];

        for (accuracy, direction, expected) in cases {
            assert_eq!(
                frontend_options_digest(accuracy, direction, &source_digest),
                decode_digest(expected)
            );
        }
    }

    /// §8 geometry 行九组合 golden 共用的 cubic 文档：半径 45 m、圆心角 20° 的圆弧
    /// 三次贝塞尔逼近（k = 4/3·tan(5°)）。曲率平缓使九个配置档组合全部可冻结，
    /// 且精度档与方向档各自都能在部分组合上成为细分点数的约束，点数随组合变化。
    const NINE_COMBO_CUBIC_DOCUMENT: &str = concat!(
        "{\"geometryVersion\":\"1\",\"module\":{\"namespace\":\"city/main\",\"documentKey\":\"source/main\",",
        "\"imports\":[],\"provenance\":{\"kind\":\"direct\",\"description\":\"nine combo golden\"}},",
        "\"units\":{\"distance\":\"meter\",\"angle\":\"radian\",\"speed\":\"meter-per-second\",\"time\":\"second\"},",
        "\"frames\":[{\"frameKey\":\"frame.main\"}],",
        "\"roads\":[{\"roadKey\":\"road.main\",\"frame\":\"frame.main\",",
        "\"referenceLine\":{\"start\":[100.5,1.25,-50.75],\"segments\":[{\"kind\":\"cubicBezier\",",
        "\"control1\":[105.74931981,1.25,-50.75],\"control2\":[110.95815175,1.25,-49.83151381],",
        "\"end\":[115.89090645,1.25,-48.03616794]}]},",
        "\"crossSectionSpans\":[{\"spanKey\":\"span.main\",\"corridorKey\":\"corridor.main\",",
        "\"startStationMeters\":0,\"endStationMeters\":\"end\",\"referenceSectionKey\":\"section.main\",",
        "\"referenceLaneKey\":\"lane.main\",\"elements\":[{\"kind\":\"roadSection\",\"sectionKey\":\"section.main\"}],",
        "\"roadSections\":[{\"sectionKey\":\"section.main\",\"kindId\":\"motorLane\",\"lanes\":[",
        "{\"laneKey\":\"lane.main\",\"laneEdgeKey\":\"edge.main\",\"direction\":\"forward\",\"widthMeters\":3.5,",
        "\"speedLimitMetersPerSecond\":10,\"successors\":[]}],\"laneGroups\":[]}],\"facilityBands\":[]}]}],",
        "\"junctions\":[],\"overlays\":{\"signalGroups\":[],\"signalControllers\":[],\"parkingAreas\":[],",
        "\"parkingSpaces\":[],\"participantClasses\":[],\"vehicleProfiles\":[],\"accessRules\":[],",
        "\"staticRoutes\":[],\"stopLines\":[],\"maneuverGates\":[],\"waitingZones\":[]}}"
    );

    #[test]
    fn geometry_payload_golden_covers_all_nine_profile_combinations() {
        // §8 geometry 行的九组合 golden：同一份含 cubic 的 Geometry 文档在全部 3×3
        // (accuracy, direction) 组合下 freeze_geometry_payload，固定每组合的点数与
        // 首末点 f32 bit 模式。方向档主导 Smooth/Balanced 列（33/17 点）；精度档在
        // Compact 方向列成为约束（Fine 17、Balanced 9、Compact 7 点）。
        let cases = [
            (
                GeometryAccuracyProfile::Fine2Cm,
                GeometryDirectionProfile::Smooth1Deg,
                33_u64,
                [0x42c9_0000, 0x3fa0_0000, 0xc24b_0000],
                [0x42e7_c825, 0x3fa0_0000, 0xc240_2509],
            ),
            (
                GeometryAccuracyProfile::Fine2Cm,
                GeometryDirectionProfile::Balanced2Deg,
                17,
                [0x42c9_0000, 0x3fa0_0000, 0xc24b_0000],
                [0x42e7_c825, 0x3fa0_0000, 0xc240_2509],
            ),
            (
                GeometryAccuracyProfile::Fine2Cm,
                GeometryDirectionProfile::Compact5Deg,
                17,
                [0x42c9_0000, 0x3fa0_0000, 0xc24b_0000],
                [0x42e7_c825, 0x3fa0_0000, 0xc240_2509],
            ),
            (
                GeometryAccuracyProfile::Balanced5Cm,
                GeometryDirectionProfile::Smooth1Deg,
                33,
                [0x42c9_0000, 0x3fa0_0000, 0xc24b_0000],
                [0x42e7_c825, 0x3fa0_0000, 0xc240_2509],
            ),
            (
                GeometryAccuracyProfile::Balanced5Cm,
                GeometryDirectionProfile::Balanced2Deg,
                17,
                [0x42c9_0000, 0x3fa0_0000, 0xc24b_0000],
                [0x42e7_c825, 0x3fa0_0000, 0xc240_2509],
            ),
            (
                GeometryAccuracyProfile::Balanced5Cm,
                GeometryDirectionProfile::Compact5Deg,
                9,
                [0x42c9_0000, 0x3fa0_0000, 0xc24b_0000],
                [0x42e7_c825, 0x3fa0_0000, 0xc240_2509],
            ),
            (
                GeometryAccuracyProfile::Compact10Cm,
                GeometryDirectionProfile::Smooth1Deg,
                33,
                [0x42c9_0000, 0x3fa0_0000, 0xc24b_0000],
                [0x42e7_c825, 0x3fa0_0000, 0xc240_2509],
            ),
            (
                GeometryAccuracyProfile::Compact10Cm,
                GeometryDirectionProfile::Balanced2Deg,
                17,
                [0x42c9_0000, 0x3fa0_0000, 0xc24b_0000],
                [0x42e7_c825, 0x3fa0_0000, 0xc240_2509],
            ),
            (
                GeometryAccuracyProfile::Compact10Cm,
                GeometryDirectionProfile::Compact5Deg,
                7,
                [0x42c9_0000, 0x3fa0_0000, 0xc24b_0000],
                [0x42e7_c825, 0x3fa0_0000, 0xc240_2509],
            ),
        ];

        for (accuracy, direction, expected_point_count, expected_first, expected_last) in cases {
            let context = || {
                format!(
                    "组合 {}/{}",
                    accuracy.stable_name(),
                    direction.stable_name()
                )
            };
            let freeze = || {
                GeometryModuleBuilder::new(
                    GeometryDocumentInput::new(
                        "source/main",
                        NINE_COMBO_CUBIC_DOCUMENT.as_bytes(),
                        None,
                    ),
                    accuracy,
                    direction,
                    &CompileLimits::p100_initial_v1(),
                )
                .unwrap_or_else(|error| {
                    panic!("九组合 golden 文档必须解析成功（{}）：{error:?}", context())
                })
                .freeze_geometry_payload()
                .unwrap_or_else(|error| {
                    panic!("九组合 golden 文档必须冻结成功（{}）：{error:?}", context())
                })
            };
            let payload = freeze();

            assert_eq!(
                payload.geometry_point_count,
                expected_point_count,
                "geometry_point_count（{}）",
                context()
            );
            assert_eq!(
                payload.lateral_curves.len(),
                1,
                "单车道文档只产生一条 lateral curve（{}）",
                context()
            );
            assert!(
                payload.internal_edge_curves.is_empty(),
                "无路口文档不产生 internal edge curve（{}）",
                context()
            );
            let points = &payload.lateral_curves[0].points;
            assert_eq!(
                points.len() as u64,
                expected_point_count,
                "lateral curve 点数必须与 geometry_point_count 一致（{}）",
                context()
            );
            let first = points.first().expect("golden 曲线至少两个点");
            assert_eq!(
                [first.x.to_bits(), first.y.to_bits(), first.z.to_bits()],
                expected_first,
                "首点 f32 bit 模式（{}）",
                context()
            );
            let last = points.last().expect("golden 曲线至少两个点");
            assert_eq!(
                [last.x.to_bits(), last.y.to_bits(), last.z.to_bits()],
                expected_last,
                "末点 f32 bit 模式（{}）",
                context()
            );

            // 确定性：同一组合独立重建再 freeze 一次，载荷逐位一致。
            let repeat = freeze();
            assert_eq!(
                repeat.geometry_point_count,
                payload.geometry_point_count,
                "重复 freeze 点数必须逐位一致（{}）",
                context()
            );
            assert_eq!(
                repeat.lateral_curves,
                payload.lateral_curves,
                "重复 freeze lateral curves 必须逐位一致（{}）",
                context()
            );
            assert_eq!(
                repeat.internal_edge_curves,
                payload.internal_edge_curves,
                "重复 freeze internal edge curves 必须逐位一致（{}）",
                context()
            );
        }
    }

    #[test]
    fn module_builder_owns_compact_parse_state_without_source_borrow() {
        super::super::descriptor::SOURCE_DOCUMENT_DIGEST_CALL_COUNT.with(|count| count.set(0));
        let source = super::schema::MINIMAL_DOCUMENT.to_vec();
        let expected_digest: [u8; 32] = Sha256::digest(&source).into();
        let builder = GeometryModuleBuilder::new(
            GeometryDocumentInput::new("source/main", &source, Some("authoring/main.json")),
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            &CompileLimits::p100_initial_v1(),
        )
        .unwrap();
        drop(source);

        assert_eq!(builder.source_document_key.as_ref(), "source/main");
        assert_eq!(builder.source_document_digest, expected_digest);
        assert_eq!(
            builder.source_record_byte_len as usize,
            super::schema::MINIMAL_DOCUMENT.len()
        );
        assert_eq!(
            builder.display_source.as_deref(),
            Some("authoring/main.json")
        );
        assert_eq!(builder.parsed.frames.len(), 1);
        assert_eq!(
            builder.freeze_reference_lines().unwrap()[0].samples.len(),
            2
        );
        let stationing = builder.freeze_stationing().unwrap();
        assert_eq!(stationing[0].intervals.len(), 1);
        assert_eq!(stationing[0].spans.len(), 1);
        assert_eq!(stationing[0].spans[0].start.parameter, 0.0);
        assert_eq!(stationing[0].spans[0].end.parameter, 1.0);
        let layouts = builder.freeze_cross_section_layouts().unwrap();
        assert_eq!(layouts.len(), 1);
        assert_eq!(layouts[0].items.len(), 1);
        assert_eq!(layouts[0].items[0].center_offset_meters, 0.0);
        let curves = builder
            .freeze_lateral_curves(&stationing, &layouts)
            .unwrap();
        assert_eq!(curves.len(), 1);
        assert_eq!(curves[0].key.as_ref(), "lane.main");
        assert_eq!(curves[0].points.len(), 2);
        assert_eq!(curves[0].points[0].x, 0.0);
        assert_eq!(curves[0].points[1].x, 10.0);
        let payload = builder.freeze_geometry_payload().unwrap();
        assert_eq!(payload.geometry_point_count, 2);
        assert_eq!(payload.lateral_curves.len(), 1);
        assert_eq!(
            payload.geometry_point_count,
            payload
                .lateral_curves
                .iter()
                .map(|curve| u64::try_from(curve.points.len()).unwrap())
                .sum::<u64>()
        );
        super::super::descriptor::SOURCE_DOCUMENT_DIGEST_CALL_COUNT
            .with(|count| assert_eq!(count.get(), 1));
    }

    #[test]
    fn geometry_payload_checks_exact_point_limit_before_returning_partial_payload() {
        fn freeze_with_limit(limit: u32) -> Result<u64, crate::DiagnosticBundle> {
            let limits = CompileLimits::p100_initial_v1()
                .with_test_admission_limit(crate::CompileLimitDimension::GeometryPointCount, limit);
            let builder = GeometryModuleBuilder::new(
                GeometryDocumentInput::new("source/main", super::schema::MINIMAL_DOCUMENT, None),
                GeometryAccuracyProfile::Balanced5Cm,
                GeometryDirectionProfile::Balanced2Deg,
                &limits,
            )?;
            Ok(builder.freeze_geometry_payload()?.geometry_point_count)
        }

        assert_eq!(freeze_with_limit(2).unwrap(), 2);
        let error = freeze_with_limit(1).err().unwrap();
        assert!(matches!(
            error.diagnostics()[0].payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: crate::CompileLimitDimension::GeometryPointCount,
                limit: 1,
                observed: 2,
            }
        ));
    }

    #[test]
    fn module_builder_rejects_source_limit_before_hashing_or_parsing() {
        super::super::descriptor::SOURCE_DOCUMENT_DIGEST_CALL_COUNT.with(|count| count.set(0));
        let limits = CompileLimits::p100_initial_v1();
        let observed = limits.value(crate::CompileLimitDimension::SourceBytesPerModule) + 1;
        let source = vec![b' '; usize::try_from(observed).unwrap()];
        let error = GeometryModuleBuilder::new(
            GeometryDocumentInput::new("source/main", &source, None),
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            &limits,
        )
        .err()
        .unwrap();

        assert!(matches!(
            error.diagnostics()[0].payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: crate::CompileLimitDimension::SourceBytesPerModule,
                observed: actual,
                ..
            } if *actual == observed
        ));
        super::super::descriptor::SOURCE_DOCUMENT_DIGEST_CALL_COUNT
            .with(|count| assert_eq!(count.get(), 0));
    }

    #[test]
    fn module_builder_rejects_document_key_mismatch_at_value_span() {
        let source = String::from_utf8(super::schema::MINIMAL_DOCUMENT.to_vec())
            .unwrap()
            .replace("source/main", "source/other");
        let error = GeometryModuleBuilder::new(
            GeometryDocumentInput::new("source/main", source.as_bytes(), None),
            GeometryAccuracyProfile::Fine2Cm,
            GeometryDirectionProfile::Smooth1Deg,
            &CompileLimits::p100_initial_v1(),
        )
        .err()
        .unwrap();
        let diagnostic = &error.diagnostics()[0];

        assert_eq!(diagnostic.code(), DiagnosticCode::InvalidGeometryDocument);
        assert!(matches!(
            diagnostic.payload(),
            DiagnosticPayload::InvalidGeometryDocument {
                violation: GeometryDocumentViolation::DocumentKeyMismatch,
                actual: Some(actual),
                expected: Some(expected),
                ..
            } if actual.as_ref() == "source/other" && expected.as_ref() == "source/main"
        ));
        assert_eq!(
            diagnostic.primary_span().unwrap().source_document_key(),
            "source/main"
        );
    }

    #[test]
    fn module_builder_maps_schema_errors_to_true_byte_positions() {
        let error = GeometryModuleBuilder::new(
            GeometryDocumentInput::new("source/main", b"{\r\n\t\"unknown\": 1}", None),
            GeometryAccuracyProfile::Compact10Cm,
            GeometryDirectionProfile::Compact5Deg,
            &CompileLimits::p100_initial_v1(),
        )
        .err()
        .unwrap();
        let diagnostic = &error.diagnostics()[0];
        let span = diagnostic.primary_span().unwrap();

        assert_eq!(diagnostic.code(), DiagnosticCode::InvalidGeometryDocument);
        assert!(matches!(
            diagnostic.payload(),
            DiagnosticPayload::InvalidGeometryDocument {
                violation: GeometryDocumentViolation::ClosedShape,
                actual: Some(actual),
                ..
            } if actual.as_ref() == "unknown"
        ));
        assert_eq!((span.start().line(), span.start().column()), (2, 2));
        assert_eq!((span.end().line(), span.end().column()), (2, 10));
    }

    fn valid_minimal_document() -> String {
        String::from_utf8(super::schema::MINIMAL_DOCUMENT.to_vec())
            .unwrap()
            .replace("\"kindId\":\"road.vehicle\"", "\"kindId\":\"motorLane\"")
    }

    fn finish_document(
        source: &[u8],
        limits: &CompileLimits,
    ) -> Result<GeometryModule, DiagnosticBundle> {
        GeometryModuleBuilder::new(
            GeometryDocumentInput::new("source/main", source, None),
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            limits,
        )?
        .finish()
    }

    fn declaration_name(declaration: &TypedAstDeclaration) -> &'static str {
        match declaration {
            TypedAstDeclaration::LaneEdge(_) => "LaneEdge",
            TypedAstDeclaration::RoadCorridor(_) => "RoadCorridor",
            TypedAstDeclaration::RoadSection(_) => "RoadSection",
            TypedAstDeclaration::LaneGroup(_) => "LaneGroup",
            TypedAstDeclaration::FacilityBand(_) => "FacilityBand",
            TypedAstDeclaration::Junction(_) => "Junction",
            TypedAstDeclaration::Movement(_) => "Movement",
            TypedAstDeclaration::ManeuverPath(_) => "ManeuverPath",
            TypedAstDeclaration::StopLine(_) => "StopLine",
            TypedAstDeclaration::ManeuverGate(_) => "ManeuverGate",
            TypedAstDeclaration::WaitingZone(_) => "WaitingZone",
            TypedAstDeclaration::StaticRoute(_) => "StaticRoute",
            TypedAstDeclaration::SignalGroup(_) => "SignalGroup",
            TypedAstDeclaration::SignalController(_) => "SignalController",
            TypedAstDeclaration::ParkingArea(_) => "ParkingArea",
            TypedAstDeclaration::ParkingSpace(_) => "ParkingSpace",
            TypedAstDeclaration::ParticipantClass(_) => "ParticipantClass",
            TypedAstDeclaration::VehicleProfile(_) => "VehicleProfile",
            TypedAstDeclaration::CanonicalFrame(_) => "CanonicalFrame",
            TypedAstDeclaration::AccessRule(_) => "AccessRule",
            TypedAstDeclaration::GeometryReferenceLine(_) => "GeometryReferenceLine",
            TypedAstDeclaration::GeometryCrossSectionSpan(_) => "GeometryCrossSectionSpan",
            TypedAstDeclaration::GeometryConnection(_) => "GeometryConnection",
            TypedAstDeclaration::GeometryInternalEdge(_) => "GeometryInternalEdge",
        }
    }

    const FULL_DOCUMENT: &str = r#"{
        "geometryVersion": "1",
        "module": {
            "namespace": "city/main",
            "documentKey": "source/main",
            "imports": [],
            "provenance": { "kind": "direct", "description": "full coverage" }
        },
        "units": {"distance":"meter","angle":"radian","speed":"meter-per-second","time":"second"},
        "frames": [{"frameKey":"frame.main"}],
        "roads": [{
            "roadKey":"road.main",
            "frame":"frame.main",
            "referenceLine":{"start":[0,0,0],"segments":[{"kind":"line","end":[10,0,0]}]},
            "crossSectionSpans":[{
                "spanKey":"span.main",
                "corridorKey":"corridor.main",
                "startStationMeters":0,
                "endStationMeters":"end",
                "referenceSectionKey":"section.forward",
                "referenceLaneKey":"lane.f1",
                "elements":[
                    {"kind":"roadSection","sectionKey":"section.forward"},
                    {"kind":"facilityBand","facilityBandKey":"band.walk"},
                    {"kind":"roadSection","sectionKey":"section.backward"}
                ],
                "roadSections":[
                    {"sectionKey":"section.forward","kindId":"motorLane","lanes":[
                        {"laneKey":"lane.f1","laneEdgeKey":"edge.f1","direction":"forward","widthMeters":3.5,"speedLimitMetersPerSecond":10,"laneGroupKey":"group.f","successors":["edge.b1"]},
                        {"laneKey":"lane.f2","laneEdgeKey":"edge.f2","direction":"forward","widthMeters":3.5,"speedLimitMetersPerSecond":10,"successors":[]}
                    ],"laneGroups":[{"laneGroupKey":"group.f"}]},
                    {"sectionKey":"section.backward","kindId":"motorLane","lanes":[
                        {"laneKey":"lane.b1","laneEdgeKey":"edge.b1","direction":"backward","widthMeters":3.25,"speedLimitMetersPerSecond":8,"successors":[]}
                    ],"laneGroups":[]}
                ],
                "facilityBands":[{"facilityBandKey":"band.walk","kindId":"sidewalk","widthMeters":2}]
            }]
        }],
        "junctions": [{
            "junctionKey":"junction.main",
            "approachEdges":["edge.f1","edge.b1"],
            "internalEdges":[],
            "connections":[{
                "movementKey":"movement.main",
                "directedEntryApproachKey":"approach.in",
                "directedExitApproachKey":"approach.out",
                "maneuverPathKey":"path.main",
                "entryEdge":"edge.f1",
                "internalEdgeSequence":[],
                "exitEdge":"edge.b1"
            }]
        }],
        "overlays": {
            "signalGroups":[{"signalGroupKey":"signal.group.main"}],
            "signalControllers":[{"signalControllerKey":"controller.main","offsetSeconds":1.5,"signalGroups":["signal.group.main"],"phases":[{"signalPhaseKey":"phase.a","durationSeconds":30,"states":[{"signalGroup":"signal.group.main","aspect":"green"}]}]}],
            "parkingAreas":[{"parkingAreaKey":"parking.area.main"}],
            "parkingSpaces":[{"parkingSpaceKey":"parking.space.main","parkingArea":"parking.area.main","entry":{"laneEdge":"edge.f1","progressMeters":1},"exit":{"laneEdge":"edge.f1","progressMeters":9},"geometry":{"lateralOffsetMeters":0.5,"headingOffsetRadians":0,"lengthMeters":5,"widthMeters":2.2}}],
            "participantClasses":[{"participantClassKey":"class.car"}],
            "vehicleProfiles":[{"vehicleProfileKey":"profile.car","participantClass":"class.car","iidm":{"lengthMeters":4.5,"desiredSpeedMetersPerSecond":12,"minGapMeters":2,"timeHeadwaySeconds":1.2,"maxAccelerationMetersPerSecondSquared":2.6,"comfortableDecelerationMetersPerSecondSquared":3,"emergencyDecelerationMetersPerSecondSquared":6}}],
            "accessRules":[{"accessRuleKey":"rule.main","target":{"kind":"laneEdge","laneEdge":"edge.f1"},"effect":"allow","participantClasses":["class.car"],"regulation":{"jurisdiction":"cn","version":"2024"},"priority":0}],
            "staticRoutes":[{"staticRouteKey":"route.main","edgeSequence":["edge.f1","edge.b1"]}],
            "stopLines":[{"stopLineKey":"stop.line.main","laneEdge":"edge.f1"}],
            "maneuverGates":[
                {"maneuverGateKey":"gate.entry","maneuverPath":"path.main","transitionIndex":0,"stopLine":"stop.line.main","signalControl":"signal.group.main"},
                {"maneuverGateKey":"gate.release","maneuverPath":"path.main","transitionIndex":1,"stopLine":"stop.line.main","signalControl":null}
            ],
            "waitingZones":[{"waitingZoneKey":"zone.main","maneuverPath":"path.main","entryGate":"gate.entry","releaseGate":"gate.release","maxOccupancy":2}]
        }
    }"#;

    #[test]
    fn finish_lowers_the_full_wire_document_in_declaration_order() {
        let module =
            finish_document(FULL_DOCUMENT.as_bytes(), &CompileLimits::p100_initial_v1()).unwrap();
        let declarations = &module.admitted.declarations;
        let names: Vec<&str> = declarations.iter().map(declaration_name).collect();
        assert_eq!(
            names,
            [
                "CanonicalFrame",
                "GeometryReferenceLine",
                "GeometryCrossSectionSpan",
                "RoadCorridor",
                "RoadSection",
                "LaneGroup",
                "LaneEdge",
                "LaneEdge",
                "RoadSection",
                "LaneEdge",
                "FacilityBand",
                "Junction",
                "Movement",
                "ManeuverPath",
                "GeometryConnection",
                "SignalGroup",
                "SignalController",
                "ParkingArea",
                "ParkingSpace",
                "ParticipantClass",
                "VehicleProfile",
                "AccessRule",
                "StaticRoute",
                "StopLine",
                "ManeuverGate",
                "ManeuverGate",
                "WaitingZone",
            ]
        );

        // §5.1 反向闭合：frame 声明不携带显式中心线。
        let TypedAstDeclaration::CanonicalFrame(frame) = &declarations[0] else {
            panic!("checked above");
        };
        assert_eq!(frame.header.stable_key.as_ref(), "frame.main");
        assert!(frame.lane_edge_geometries.is_empty());

        let TypedAstDeclaration::GeometryReferenceLine(intent) = &declarations[1] else {
            panic!("checked above");
        };
        assert_eq!(intent.road_key.as_ref(), "road.main");
        assert_eq!(intent.frame.declaration_key.as_ref(), "frame.main");
        assert_eq!(intent.frame.module_namespace.as_ref(), "city/main");

        let TypedAstDeclaration::GeometryCrossSectionSpan(intent) = &declarations[2] else {
            panic!("checked above");
        };
        assert_eq!(intent.span_key.as_ref(), "span.main");
        assert_eq!(intent.corridor.declaration_key.as_ref(), "corridor.main");
        let offsets: Vec<(&str, GeometryOffsetIntentKind, f64)> = intent
            .offsets
            .iter()
            .map(|offset| (offset.key.as_ref(), offset.kind, offset.width_meters))
            .collect();
        assert_eq!(
            offsets,
            [
                ("lane.f1", GeometryOffsetIntentKind::ForwardLane, 3.5),
                ("lane.f2", GeometryOffsetIntentKind::ForwardLane, 3.5),
                ("band.walk", GeometryOffsetIntentKind::FacilityBand, 2.0),
                ("lane.b1", GeometryOffsetIntentKind::BackwardLane, 3.25),
            ]
        );

        let TypedAstDeclaration::RoadCorridor(corridor) = &declarations[3] else {
            panic!("checked above");
        };
        assert_eq!(
            corridor.reference_section.declaration_key.as_ref(),
            "section.forward"
        );
        let elements: Vec<&str> = corridor
            .elements
            .iter()
            .map(|element| match element {
                OwnedCorridorElementReference::RoadSection(reference) => {
                    reference.declaration_key.as_ref()
                }
                OwnedCorridorElementReference::FacilityBand(reference) => {
                    reference.declaration_key.as_ref()
                }
            })
            .collect();
        assert_eq!(
            elements,
            ["section.forward", "band.walk", "section.backward"]
        );

        let TypedAstDeclaration::RoadSection(section) = &declarations[4] else {
            panic!("checked above");
        };
        assert_eq!(section.kind_id.as_ref(), "motorLane");
        assert_eq!(section.lanes.len(), 2);
        assert_eq!(
            section.lanes[0].edge_chain[0].declaration_key.as_ref(),
            "edge.f1"
        );
        assert_eq!(
            section.lanes[0]
                .lane_group
                .as_ref()
                .unwrap()
                .declaration_key
                .as_ref(),
            "group.f"
        );
        assert!(section.lanes[1].lane_group.is_none());

        let TypedAstDeclaration::LaneGroup(group) = &declarations[5] else {
            panic!("checked above");
        };
        assert_eq!(
            group.road_section.declaration_key.as_ref(),
            "section.forward"
        );

        // 直线 10m 参考线覆盖整个站区：长度按 §6.1 由冻结折线精确派生。
        let TypedAstDeclaration::LaneEdge(edge) = &declarations[6] else {
            panic!("checked above");
        };
        assert_eq!(edge.header.stable_key.as_ref(), "edge.f1");
        assert_eq!(edge.length.value(), 10.0);
        assert_eq!(edge.speed_limit.value(), 10.0);
        assert_eq!(edge.successors.len(), 1);
        assert_eq!(edge.successors[0].declaration_key.as_ref(), "edge.b1");

        let TypedAstDeclaration::LaneEdge(edge) = &declarations[9] else {
            panic!("checked above");
        };
        assert_eq!(edge.header.stable_key.as_ref(), "edge.b1");
        assert_eq!(edge.length.value(), 10.0);
        assert_eq!(edge.speed_limit.value(), 8.0);

        let TypedAstDeclaration::FacilityBand(band) = &declarations[10] else {
            panic!("checked above");
        };
        assert_eq!(band.kind_id.as_ref(), "sidewalk");

        let TypedAstDeclaration::Movement(movement) = &declarations[12] else {
            panic!("checked above");
        };
        assert_eq!(movement.junction.declaration_key.as_ref(), "junction.main");
        assert_eq!(movement.junction.module_namespace.as_ref(), "city/main");
        assert_eq!(movement.directed_entry_approach_key.as_ref(), "approach.in");
        assert_eq!(movement.directed_exit_approach_key.as_ref(), "approach.out");

        let TypedAstDeclaration::ManeuverPath(path) = &declarations[13] else {
            panic!("checked above");
        };
        assert_eq!(path.movement.declaration_key.as_ref(), "movement.main");
        assert_eq!(path.entry_edge.declaration_key.as_ref(), "edge.f1");
        assert!(path.internal_edges.is_empty());
        assert_eq!(path.exit_edge.declaration_key.as_ref(), "edge.b1");

        let TypedAstDeclaration::GeometryConnection(intent) = &declarations[14] else {
            panic!("checked above");
        };
        assert_eq!(intent.junction.declaration_key.as_ref(), "junction.main");
        assert_eq!(intent.maneuver_path.declaration_key.as_ref(), "path.main");
        assert_eq!(intent.entry_edge.declaration_key.as_ref(), "edge.f1");
        assert!(intent.internal_edges.is_empty());
        assert_eq!(intent.exit_edge.declaration_key.as_ref(), "edge.b1");

        let TypedAstDeclaration::SignalController(controller) = &declarations[16] else {
            panic!("checked above");
        };
        assert_eq!(controller.offset_ms, 1_500);
        assert_eq!(controller.signal_groups.len(), 1);
        assert_eq!(controller.phases.len(), 1);
        assert_eq!(controller.phases[0].duration_ms, 30_000);
        assert_eq!(controller.phases[0].states.len(), 1);
        assert_eq!(controller.phases[0].states[0].aspect, SignalAspect::Green);

        let TypedAstDeclaration::ParkingSpace(space) = &declarations[18] else {
            panic!("checked above");
        };
        assert_eq!(
            space
                .parking_area
                .as_ref()
                .unwrap()
                .declaration_key
                .as_ref(),
            "parking.area.main"
        );
        assert_eq!(space.entry.progress_meters, 1.0);
        assert_eq!(space.exit.progress_meters, 9.0);
        assert_eq!(space.geometry.length_meters, 5.0);

        let TypedAstDeclaration::VehicleProfile(profile) = &declarations[20] else {
            panic!("checked above");
        };
        assert_eq!(
            profile.participant_class.declaration_key.as_ref(),
            "class.car"
        );
        assert_eq!(profile.iidm.length_meters, 4.5);

        let TypedAstDeclaration::AccessRule(rule) = &declarations[21] else {
            panic!("checked above");
        };
        let OwnedAccessRuleTarget::LaneEdge(target) = &rule.target else {
            panic!("lane edge target checked in the document");
        };
        assert_eq!(target.declaration_key.as_ref(), "edge.f1");
        assert_eq!(rule.effect, AccessEffect::Allow);
        let regulation = rule.regulation.as_ref().unwrap();
        assert_eq!(regulation.jurisdiction.as_ref(), "cn");
        assert_eq!(regulation.version.as_ref(), "2024");
        assert!(regulation.source.is_none());

        let TypedAstDeclaration::StaticRoute(route) = &declarations[22] else {
            panic!("checked above");
        };
        let route_edges: Vec<&str> = route
            .edge_sequence
            .iter()
            .map(|edge| edge.declaration_key.as_ref())
            .collect();
        assert_eq!(route_edges, ["edge.f1", "edge.b1"]);

        let TypedAstDeclaration::ManeuverGate(gate) = &declarations[24] else {
            panic!("checked above");
        };
        assert_eq!(gate.header.stable_key.as_ref(), "gate.entry");
        assert_eq!(gate.transition_index, 0);
        let OwnedSignalControl::Group(group) = &gate.signal_control else {
            panic!("signal control checked in the document");
        };
        assert_eq!(group.declaration_key.as_ref(), "signal.group.main");

        let TypedAstDeclaration::ManeuverGate(gate) = &declarations[25] else {
            panic!("checked above");
        };
        assert!(matches!(gate.signal_control, OwnedSignalControl::None));

        let TypedAstDeclaration::WaitingZone(zone) = &declarations[26] else {
            panic!("checked above");
        };
        assert_eq!(zone.max_occupancy, 2);
        assert_eq!(zone.entry_gate.declaration_key.as_ref(), "gate.entry");
        assert_eq!(zone.release_gate.declaration_key.as_ref(), "gate.release");

        // 冻结载荷随模块不可分：每条 lane 一条规范折线，facility 带也有一条。
        let payload = module.admitted.geometry_payload().unwrap();
        assert_eq!(payload.frozen.lateral_curves.len(), 4);
    }

    /// §9.2 前端计数视图：reference line 加一段 cubic 后，声明/引用/关系、曲线段、
    /// 控制点、offset 分布与规范点数全部按冻结结果精确报告。
    #[test]
    fn finish_freezes_geometry_module_counts() {
        let document = FULL_DOCUMENT.replacen(
            "\"referenceLine\":{\"start\":[0,0,0],\"segments\":[{\"kind\":\"line\",\"end\":[10,0,0]}]}",
            "\"referenceLine\":{\"start\":[0,1.25,-50.75],\"segments\":[{\"kind\":\"line\",\"end\":[100.5,1.25,-50.75]},{\"kind\":\"cubicBezier\",\"control1\":[105.74931981,1.25,-50.75],\"control2\":[110.95815175,1.25,-49.83151381],\"end\":[115.89090645,1.25,-48.03616794]}]}",
            1,
        );
        let module =
            finish_document(document.as_bytes(), &CompileLimits::p100_initial_v1()).unwrap();
        let counts = module.counts();

        // 声明计数 = 27 个顶层声明 + 3 条嵌套编制车道 + 1 个嵌套信号相位；
        // 引用/关系计数固定为 golden（口径与 finish_resource_counts 公式测试一致）。
        assert_eq!(counts.declaration_count(), 31);
        assert_eq!(counts.reference_count(), 40);
        assert_eq!(counts.relation_occurrence_count(), 37);

        // 曲线段与控制点：reference line 的 line+cubic；本文档无 internal edge geometry。
        assert_eq!(counts.line_segment_count(), 1);
        assert_eq!(counts.cubic_segment_count(), 1);
        assert_eq!(counts.control_point_count(), 2);

        // 横向 offset 曲线四条，|中心偏移| 分别为 0、3.5、6.25、8.875 米。
        assert_eq!(counts.offset_curve_count(), 4);
        let buckets: Vec<(u64, u64)> = counts
            .absolute_offset_distribution()
            .iter()
            .map(|bucket| (bucket.absolute_offset_meters_bits(), bucket.curve_count()))
            .collect();
        assert_eq!(
            buckets,
            [
                (0.0_f64.to_bits(), 1),
                (3.5_f64.to_bits(), 1),
                (6.25_f64.to_bits(), 1),
                (8.875_f64.to_bits(), 1),
            ]
        );

        // 规范点数与冻结载荷逐曲线点数之和一致；offset 曲线数与横向曲线数一致。
        let payload = module.admitted.geometry_payload().unwrap();
        let lateral_points = payload
            .frozen
            .lateral_curves
            .iter()
            .fold(0_u64, |total, curve| {
                total + u64::try_from(curve.points.len()).unwrap()
            });
        let internal_points = payload
            .frozen
            .internal_edge_curves
            .iter()
            .fold(0_u64, |total, curve| {
                total + u64::try_from(curve.points.len()).unwrap()
            });
        assert_eq!(
            counts.canonical_point_count(),
            lateral_points + internal_points
        );
        assert_eq!(
            usize::try_from(counts.offset_curve_count()).unwrap(),
            payload.frozen.lateral_curves.len()
        );
    }

    #[test]
    fn finish_maps_direct_provenance_into_the_descriptor() {
        let source = valid_minimal_document();
        let module = GeometryModuleBuilder::new(
            GeometryDocumentInput::new(
                "source/main",
                source.as_bytes(),
                Some("authoring/main.json"),
            ),
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            &CompileLimits::p100_initial_v1(),
        )
        .unwrap()
        .finish()
        .unwrap();
        let descriptor = module.descriptor();

        assert_eq!(descriptor.authoring_namespace_id(), "city/main");
        assert_eq!(
            descriptor.source_language(),
            SourceLanguage::GeometryDocument
        );
        assert_eq!(descriptor.frontend_version(), GEOMETRY_FRONTEND_VERSION);
        assert_eq!(
            descriptor.generator_build_id(),
            "laneflow-geometry-direct-v1"
        );
        assert_eq!(
            descriptor.parameters_and_inputs_digest(),
            &direct_parameters_and_inputs_digest()
        );
        assert_eq!(descriptor.random_seed(), None);
        assert_eq!(descriptor.provenance(), "minimal");
        assert_eq!(
            descriptor.frontend_options_digest(),
            &frontend_options_digest(
                GeometryAccuracyProfile::Balanced5Cm,
                GeometryDirectionProfile::Balanced2Deg,
                &direct_source_frontend_options_digest(),
            )
        );
        assert_eq!(descriptor.imports().len(), 0);
        assert!(matches!(
            module.accuracy_profile(),
            GeometryAccuracyProfile::Balanced5Cm
        ));
        assert!(matches!(
            module.direction_profile(),
            GeometryDirectionProfile::Balanced2Deg
        ));

        let documents: Vec<_> = module.source_documents().collect();
        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].source_document_key(), "source/main");
        assert_eq!(
            documents[0].source_document_digest(),
            &<[u8; 32]>::from(Sha256::digest(source.as_bytes()))
        );
        assert_eq!(documents[0].source_record_byte_len() as usize, source.len());
        assert_eq!(
            documents[0].origin().display_source(),
            Some("authoring/main.json")
        );
    }

    #[test]
    fn finish_maps_generated_provenance_into_the_descriptor() {
        let source = valid_minimal_document().replace(
            "\"provenance\":{\"kind\":\"direct\",\"description\":\"minimal\"}",
            concat!(
                "\"provenance\":{\"kind\":\"generated\",\"generatorBuildId\":\"generator.v1\",",
                "\"parametersAndInputsDigest\":\"0000000000000000000000000000000000000000000000000000000000000000\",",
                "\"frontendOptionsDigest\":\"ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff\",",
                "\"randomSeed\":\"42\",\"description\":\"generated source\"}"
            ),
        );
        let module = finish_document(source.as_bytes(), &CompileLimits::p100_initial_v1()).unwrap();
        let descriptor = module.descriptor();

        assert_eq!(descriptor.generator_build_id(), "generator.v1");
        assert_eq!(descriptor.parameters_and_inputs_digest(), &[0x00; 32]);
        assert_eq!(descriptor.random_seed(), Some(42));
        assert_eq!(descriptor.provenance(), "generated source");
        assert_eq!(
            descriptor.frontend_options_digest(),
            &frontend_options_digest(
                GeometryAccuracyProfile::Balanced5Cm,
                GeometryDirectionProfile::Balanced2Deg,
                &[0xff; 32],
            )
        );
    }

    #[test]
    fn finish_hashes_the_source_exactly_once() {
        super::super::descriptor::SOURCE_DOCUMENT_DIGEST_CALL_COUNT.with(|count| count.set(0));
        let source = valid_minimal_document();
        finish_document(source.as_bytes(), &CompileLimits::p100_initial_v1()).unwrap();
        super::super::descriptor::SOURCE_DOCUMENT_DIGEST_CALL_COUNT
            .with(|count| assert_eq!(count.get(), 1));
    }

    const TWO_LANE_SPAN: &str = concat!(
        "{\"spanKey\":\"span.main\",\"corridorKey\":\"corridor.main\",\"startStationMeters\":0,",
        "\"endStationMeters\":\"end\",\"referenceSectionKey\":\"section.main\",\"referenceLaneKey\":\"lane.a\",",
        "\"elements\":[{\"kind\":\"roadSection\",\"sectionKey\":\"section.main\"}],",
        "\"roadSections\":[{\"sectionKey\":\"section.main\",\"kindId\":\"motorLane\",\"lanes\":[",
        "{\"laneKey\":\"lane.a\",\"laneEdgeKey\":\"edge.a\",\"direction\":\"forward\",\"widthMeters\":3.5,\"speedLimitMetersPerSecond\":10,\"successors\":[]},",
        "{\"laneKey\":\"lane.b\",\"laneEdgeKey\":\"edge.b\",\"direction\":\"forward\",\"widthMeters\":3.5,\"speedLimitMetersPerSecond\":10,\"successors\":[]}",
        "],\"laneGroups\":[]}],\"facilityBands\":[]}"
    );

    const INTERNAL_EDGE: &str = concat!(
        "{\"laneEdgeKey\":\"edge.internal\",\"speedLimitMetersPerSecond\":8,",
        "\"geometry\":{\"start\":[0,0,0],\"segments\":[{\"kind\":\"line\",\"end\":[5,0,5]}]}}"
    );

    fn two_lane_document(junctions: &str) -> String {
        road_document_with_junctions(
            &format!("[{}]", road_fragment("road.main", TWO_LANE_SPAN)),
            junctions,
        )
    }

    fn junction_fragment(internal_edges: &str, connections: &str) -> String {
        junction_fragment_with_key("junction.main", internal_edges, connections)
    }

    fn junction_fragment_with_key(
        junction_key: &str,
        internal_edges: &str,
        connections: &str,
    ) -> String {
        format!(
            concat!(
                "{{\"junctionKey\":\"{junction_key}\",\"approachEdges\":[\"edge.a\",\"edge.b\"],",
                "\"internalEdges\":{internal_edges},\"connections\":{connections}}}"
            ),
            junction_key = junction_key,
            internal_edges = internal_edges,
            connections = connections
        )
    }

    fn connection_fragment(movement_key: &str, path_key: &str, sequence: &str) -> String {
        format!(
            concat!(
                "{{\"movementKey\":\"{movement_key}\",\"directedEntryApproachKey\":\"approach.in\",",
                "\"directedExitApproachKey\":\"approach.out\",\"maneuverPathKey\":\"{path_key}\",",
                "\"entryEdge\":\"edge.a\",\"internalEdgeSequence\":{sequence},\"exitEdge\":\"edge.b\"}}"
            ),
            movement_key = movement_key,
            path_key = path_key,
            sequence = sequence
        )
    }

    fn reference_keys(edges: &[OwnedEntityReference<LaneEdgeKind>]) -> Vec<&str> {
        edges
            .iter()
            .map(|edge| edge.declaration_key.as_ref())
            .collect()
    }

    #[test]
    fn finish_lowers_shared_internal_edges_and_ordered_connection_sequences() {
        let source = two_lane_document(&format!(
            "[{}]",
            junction_fragment(
                &format!("[{INTERNAL_EDGE}]"),
                &format!(
                    "[{},{}]",
                    connection_fragment("movement.a", "path.a", "[\"edge.internal\"]"),
                    connection_fragment("movement.b", "path.b", "[\"edge.internal\"]")
                ),
            )
        ));
        let module = finish_document(source.as_bytes(), &CompileLimits::p100_initial_v1()).unwrap();
        let declarations = &module.admitted.declarations;
        let names: Vec<&str> = declarations.iter().map(declaration_name).collect();
        assert_eq!(
            names,
            [
                "CanonicalFrame",
                "GeometryReferenceLine",
                "GeometryCrossSectionSpan",
                "RoadCorridor",
                "RoadSection",
                "LaneEdge",
                "LaneEdge",
                "Junction",
                "LaneEdge",
                "GeometryInternalEdge",
                "Movement",
                "ManeuverPath",
                "GeometryConnection",
                "Movement",
                "ManeuverPath",
                "GeometryConnection",
            ]
        );

        // 每条 internal record 产出由所属 Junction 唯一拥有、带显式 speed、successors
        // 为空的 LaneEdge 共同声明；length 从冻结中心线按 §6.2 规范 f64 累计派生。
        let TypedAstDeclaration::LaneEdge(edge) = &declarations[8] else {
            panic!("checked above");
        };
        assert_eq!(edge.header.stable_key.as_ref(), "edge.internal");
        assert_eq!(edge.length.value(), 50.0_f64.sqrt());
        assert_eq!(edge.speed_limit.value(), 8.0);
        assert!(edge.successors.is_empty());

        let TypedAstDeclaration::GeometryInternalEdge(intent) = &declarations[9] else {
            panic!("checked above");
        };
        assert_eq!(intent.key.as_ref(), "edge.internal");
        assert_eq!(intent.junction.declaration_key.as_ref(), "junction.main");
        assert_eq!(intent.junction.module_namespace.as_ref(), "city/main");

        // 权威路径序列唯一等于 entry+internal+exit；同一 internal edge 可被该
        // Junction 的多条 connection path 共享引用。
        for index in [11, 14] {
            let TypedAstDeclaration::ManeuverPath(path) = &declarations[index] else {
                panic!("checked above");
            };
            assert_eq!(path.entry_edge.declaration_key.as_ref(), "edge.a");
            assert_eq!(reference_keys(&path.internal_edges), ["edge.internal"]);
            assert_eq!(path.exit_edge.declaration_key.as_ref(), "edge.b");
        }
        for index in [12, 15] {
            let TypedAstDeclaration::GeometryConnection(intent) = &declarations[index] else {
                panic!("checked above");
            };
            assert_eq!(intent.junction.declaration_key.as_ref(), "junction.main");
            assert_eq!(reference_keys(&intent.internal_edges), ["edge.internal"]);
        }

        // internal edge 显式曲线量化后与 lateral 曲线共享同一 GeometryPointCount。
        let payload = module.admitted.geometry_payload().unwrap();
        assert_eq!(payload.frozen.lateral_curves.len(), 2);
        assert_eq!(payload.frozen.internal_edge_curves.len(), 1);
        let curve = &payload.frozen.internal_edge_curves[0];
        assert_eq!(curve.junction_key.as_ref(), "junction.main");
        assert_eq!(curve.lane_edge_key.as_ref(), "edge.internal");
        let points: Vec<(f32, f32, f32)> = curve
            .points
            .iter()
            .map(|point| (point.x, point.y, point.z))
            .collect();
        assert_eq!(points, [(0.0, 0.0, 0.0), (5.0, 0.0, 5.0)]);
        assert_eq!(payload.frozen.geometry_point_count, 6);
    }

    #[test]
    fn geometry_payload_shares_the_exact_point_budget_with_internal_edges() {
        fn freeze_with_limit(limit: u32) -> Result<u64, DiagnosticBundle> {
            let limits = CompileLimits::p100_initial_v1()
                .with_test_admission_limit(CompileLimitDimension::GeometryPointCount, limit);
            let source = two_lane_document(&format!(
                "[{}]",
                junction_fragment(
                    &format!("[{INTERNAL_EDGE}]"),
                    &format!(
                        "[{}]",
                        connection_fragment("movement.a", "path.a", "[\"edge.internal\"]")
                    ),
                )
            ));
            let builder = GeometryModuleBuilder::new(
                GeometryDocumentInput::new("source/main", source.as_bytes(), None),
                GeometryAccuracyProfile::Balanced5Cm,
                GeometryDirectionProfile::Balanced2Deg,
                &limits,
            )?;
            Ok(builder.freeze_geometry_payload()?.geometry_point_count)
        }

        // lateral 4 点 + internal 2 点共用同一预算：边界通过，边界减一失败。
        assert_eq!(freeze_with_limit(6).unwrap(), 6);
        let error = freeze_with_limit(5).err().unwrap();
        assert!(matches!(
            error.diagnostics()[0].payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::GeometryPointCount,
                limit: 5,
                observed: 6,
            }
        ));
    }

    #[test]
    fn finish_rejects_internal_edges_not_referenced_by_any_connection() {
        let source = two_lane_document(&format!(
            "[{}]",
            junction_fragment(
                &format!("[{INTERNAL_EDGE}]"),
                &format!(
                    "[{}]",
                    connection_fragment("movement.main", "path.main", "[]")
                ),
            )
        ));
        let error = finish_document(source.as_bytes(), &CompileLimits::p100_initial_v1())
            .err()
            .unwrap();
        assert_eq!(error.diagnostics().len(), 1);
        assert!(matches!(
            error.diagnostics()[0].payload(),
            DiagnosticPayload::InvalidGeometryDocument {
                violation: GeometryDocumentViolation::FieldValue,
                field: Some(field),
                actual: Some(actual),
                ..
            } if field.as_ref() == "junctions[].internalEdges" && actual.as_ref() == "edge.internal"
        ));
    }

    #[test]
    fn finish_rejects_internal_edge_sequences_outside_the_owning_junction() {
        let expect_sequence_error = |source: String, expected_actual: &str| {
            let error = finish_document(source.as_bytes(), &CompileLimits::p100_initial_v1())
                .err()
                .unwrap();
            assert_eq!(error.diagnostics().len(), 1);
            assert!(
                matches!(
                    error.diagnostics()[0].payload(),
                    DiagnosticPayload::InvalidGeometryDocument {
                        violation: GeometryDocumentViolation::FieldValue,
                        field: Some(field),
                        actual: Some(actual),
                        ..
                    } if field.as_ref() == "junctions[].connections[].internalEdgeSequence"
                        && actual.as_ref() == expected_actual
                ),
                "unexpected payload: {:?}",
                error.diagnostics()[0].payload()
            );
        };

        // 引用未声明的键失败。
        expect_sequence_error(
            two_lane_document(&format!(
                "[{}]",
                junction_fragment(
                    "[]",
                    &format!(
                        "[{}]",
                        connection_fragment("movement.a", "path.a", "[\"edge.internal\"]")
                    ),
                )
            )),
            "edge.internal",
        );
        // road lane edge 不是本 Junction 的 internal edge，即使属于 approach 也失败。
        expect_sequence_error(
            two_lane_document(&format!(
                "[{}]",
                junction_fragment(
                    &format!("[{INTERNAL_EDGE}]"),
                    &format!(
                        "[{},{}]",
                        connection_fragment("movement.a", "path.a", "[\"edge.internal\"]"),
                        connection_fragment("movement.b", "path.b", "[\"edge.a\"]")
                    ),
                )
            )),
            "edge.a",
        );
        // 引用另一 Junction 所有的 internal edge 失败。
        expect_sequence_error(
            two_lane_document(&format!(
                "[{},{}]",
                junction_fragment(
                    &format!("[{INTERNAL_EDGE}]"),
                    &format!(
                        "[{}]",
                        connection_fragment("movement.a", "path.a", "[\"edge.internal\"]")
                    ),
                ),
                junction_fragment_with_key(
                    "junction.other",
                    "[]",
                    &format!(
                        "[{}]",
                        connection_fragment("movement.other", "path.other", "[\"edge.internal\"]")
                    ),
                )
            )),
            "edge.internal",
        );
    }

    #[test]
    fn finish_rejects_aliased_duplicate_internal_edge_references_within_one_connection() {
        // parser 只排除字面重复 token；显式 namespace 别名解析到同一 internal edge
        // 仍违反 §4.4 的连接内不重复约束。
        let source = two_lane_document(&format!(
            "[{}]",
            junction_fragment(
                &format!("[{INTERNAL_EDGE}]"),
                &format!(
                    "[{}]",
                    connection_fragment(
                        "movement.a",
                        "path.a",
                        "[\"edge.internal\",\"city/main::edge.internal\"]"
                    )
                ),
            )
        ));
        let error = finish_document(source.as_bytes(), &CompileLimits::p100_initial_v1())
            .err()
            .unwrap();
        assert_eq!(error.diagnostics().len(), 1);
        assert!(matches!(
            error.diagnostics()[0].payload(),
            DiagnosticPayload::InvalidGeometryDocument {
                violation: GeometryDocumentViolation::FieldValue,
                field: Some(field),
                actual: Some(actual),
                ..
            } if field.as_ref() == "junctions[].connections[].internalEdgeSequence"
                && actual.as_ref() == "city/main::edge.internal"
        ));
    }

    #[test]
    fn finish_rejects_duplicate_internal_edge_keys_in_the_lane_edge_group() {
        // 与 road lane 的 laneEdgeKey 同一 LaneEdge 键分组。
        let conflicting = INTERNAL_EDGE.replace("edge.internal", "edge.a");
        let source = two_lane_document(&format!(
            "[{}]",
            junction_fragment(
                &format!("[{conflicting}]"),
                &format!(
                    "[{}]",
                    connection_fragment("movement.a", "path.a", "[\"edge.a\"]")
                ),
            )
        ));
        let error = finish_document(source.as_bytes(), &CompileLimits::p100_initial_v1())
            .err()
            .unwrap();
        assert!(matches!(
            error.diagnostics()[0].payload(),
            DiagnosticPayload::DuplicateDeclaration {
                entity_kind: EntityKind::LaneEdge,
                stable_key,
            } if stable_key.as_ref() == "edge.a"
        ));

        // 两个 Junction 声明同名 internal edge 同样冲突。
        let source = two_lane_document(&format!(
            "[{},{}]",
            junction_fragment(
                &format!("[{INTERNAL_EDGE}]"),
                &format!(
                    "[{}]",
                    connection_fragment("movement.a", "path.a", "[\"edge.internal\"]")
                ),
            ),
            junction_fragment_with_key(
                "junction.other",
                &format!("[{INTERNAL_EDGE}]"),
                &format!(
                    "[{}]",
                    connection_fragment("movement.other", "path.other", "[\"edge.internal\"]")
                ),
            )
        ));
        let error = finish_document(source.as_bytes(), &CompileLimits::p100_initial_v1())
            .err()
            .unwrap();
        assert!(matches!(
            error.diagnostics()[0].payload(),
            DiagnosticPayload::DuplicateDeclaration {
                entity_kind: EntityKind::LaneEdge,
                stable_key,
            } if stable_key.as_ref() == "edge.internal"
        ));
    }

    #[test]
    fn finish_rejects_non_positive_or_non_finite_internal_edge_speed_limits() {
        // 严格正约束与 road lane 同一 SpeedLimit 校验路径。
        for token in ["0", "-1"] {
            let edge = INTERNAL_EDGE.replace(
                "\"speedLimitMetersPerSecond\":8",
                &format!("\"speedLimitMetersPerSecond\":{token}"),
            );
            let source = two_lane_document(&format!(
                "[{}]",
                junction_fragment(
                    &format!("[{edge}]"),
                    &format!(
                        "[{}]",
                        connection_fragment("movement.a", "path.a", "[\"edge.internal\"]")
                    ),
                )
            ));
            let error = finish_document(source.as_bytes(), &CompileLimits::p100_initial_v1())
                .err()
                .unwrap();
            assert_eq!(error.diagnostics().len(), 1);
            assert!(
                matches!(
                    error.diagnostics()[0].payload(),
                    DiagnosticPayload::InvalidLaneEdgeSpeedLimit {
                        stable_key,
                        violation: crate::ScalarViolation::NotGreaterThan { .. },
                        ..
                    } if stable_key.as_ref() == "edge.internal"
                ),
                "token {token}: unexpected payload: {:?}",
                error.diagnostics()[0].payload()
            );
        }

        // 非有限十进制 token 与 road lane 同一 parse_finite 路径。
        let edge = INTERNAL_EDGE.replace(
            "\"speedLimitMetersPerSecond\":8",
            "\"speedLimitMetersPerSecond\":1e400",
        );
        let source = two_lane_document(&format!(
            "[{}]",
            junction_fragment(
                &format!("[{edge}]"),
                &format!(
                    "[{}]",
                    connection_fragment("movement.a", "path.a", "[\"edge.internal\"]")
                ),
            )
        ));
        let error = finish_document(source.as_bytes(), &CompileLimits::p100_initial_v1())
            .err()
            .unwrap();
        assert!(matches!(
            error.diagnostics()[0].payload(),
            DiagnosticPayload::InvalidGeometryDocument {
                violation: GeometryDocumentViolation::FieldValue,
                field: Some(field),
                ..
            } if field.as_ref() == "junctions[].internalEdges[].speedLimitMetersPerSecond"
        ));
    }

    #[test]
    fn finish_rejects_connection_edges_outside_the_approach_set() {
        let cases = [
            ("edge.c", "edge.b", "connections[].entryEdge"),
            ("edge.a", "edge.c", "connections[].exitEdge"),
        ];
        for (entry, exit, expected_field) in cases {
            let source = road_document_with_junctions(
                "[]",
                &format!(
                    concat!(
                        "[{{\"junctionKey\":\"junction.main\",\"approachEdges\":[\"edge.a\",\"edge.b\"],",
                        "\"internalEdges\":[],\"connections\":[{{\"movementKey\":\"movement.main\",",
                        "\"directedEntryApproachKey\":\"approach.in\",\"directedExitApproachKey\":\"approach.out\",",
                        "\"maneuverPathKey\":\"path.main\",\"entryEdge\":\"{entry}\",",
                        "\"internalEdgeSequence\":[],\"exitEdge\":\"{exit}\"}}]}}]"
                    ),
                    entry = entry,
                    exit = exit
                ),
            );
            let error = finish_document(source.as_bytes(), &CompileLimits::p100_initial_v1())
                .err()
                .unwrap();
            assert_eq!(error.diagnostics().len(), 1);
            assert!(matches!(
                error.diagnostics()[0].payload(),
                DiagnosticPayload::InvalidGeometryDocument {
                    violation: GeometryDocumentViolation::FieldValue,
                    field: Some(field),
                    ..
                } if field.as_ref() == expected_field
            ));
        }
    }

    #[test]
    fn finish_rejects_duplicate_keys_by_group_and_allows_cross_group_spelling() {
        let road = road_fragment("road.main", MINIMAL_SPAN);
        let duplicate_road = road_document(&format!("[{road},{road}]"), "[]");
        let error = finish_document(duplicate_road.as_bytes(), &CompileLimits::p100_initial_v1())
            .err()
            .unwrap();
        assert!(matches!(
            error.diagnostics()[0].payload(),
            DiagnosticPayload::InvalidGeometryDocument {
                violation: GeometryDocumentViolation::FieldValue,
                field: Some(field),
                ..
            } if field.as_ref() == "roads[].roadKey"
        ));

        let first = road_fragment("road.a", MINIMAL_SPAN);
        let second = road_fragment("road.b", MINIMAL_SPAN);
        let duplicate_span = road_document(&format!("[{first},{second}]"), "[]");
        let error = finish_document(duplicate_span.as_bytes(), &CompileLimits::p100_initial_v1())
            .err()
            .unwrap();
        assert!(matches!(
            error.diagnostics()[0].payload(),
            DiagnosticPayload::InvalidGeometryDocument {
                violation: GeometryDocumentViolation::FieldValue,
                field: Some(field),
                ..
            } if field.as_ref() == "roads[].crossSectionSpans[].spanKey"
        ));

        let shared_edge_span = concat!(
            "{\"spanKey\":\"span.a\",\"corridorKey\":\"corridor.a\",\"startStationMeters\":0,",
            "\"endStationMeters\":\"end\",\"referenceSectionKey\":\"section.a\",\"referenceLaneKey\":\"lane.a\",",
            "\"elements\":[{\"kind\":\"roadSection\",\"sectionKey\":\"section.a\"},{\"kind\":\"roadSection\",\"sectionKey\":\"section.b\"}],",
            "\"roadSections\":[",
            "{\"sectionKey\":\"section.a\",\"kindId\":\"motorLane\",\"lanes\":[{\"laneKey\":\"lane.a\",\"laneEdgeKey\":\"edge.same\",\"direction\":\"forward\",\"widthMeters\":3.5,\"speedLimitMetersPerSecond\":10,\"successors\":[]}],\"laneGroups\":[]},",
            "{\"sectionKey\":\"section.b\",\"kindId\":\"motorLane\",\"lanes\":[{\"laneKey\":\"lane.b\",\"laneEdgeKey\":\"edge.same\",\"direction\":\"forward\",\"widthMeters\":3.5,\"speedLimitMetersPerSecond\":10,\"successors\":[]}],\"laneGroups\":[]}",
            "],\"facilityBands\":[]}"
        );
        let duplicate_edge = road_document(
            &format!("[{}]", road_fragment("road.main", shared_edge_span)),
            "[]",
        );
        let error = finish_document(duplicate_edge.as_bytes(), &CompileLimits::p100_initial_v1())
            .err()
            .unwrap();
        assert!(matches!(
            error.diagnostics()[0].payload(),
            DiagnosticPayload::DuplicateDeclaration {
                entity_kind: EntityKind::LaneEdge,
                stable_key,
            } if stable_key.as_ref() == "edge.same"
        ));

        // 键查重按 per-EntityKind 分组：全部记录拼写相同但分组不同，finish 照常完成。
        let shared_span = concat!(
            "{\"spanKey\":\"shared.key\",\"corridorKey\":\"shared.key\",\"startStationMeters\":0,",
            "\"endStationMeters\":\"end\",\"referenceSectionKey\":\"shared.key\",\"referenceLaneKey\":\"shared.key\",",
            "\"elements\":[{\"kind\":\"roadSection\",\"sectionKey\":\"shared.key\"}],",
            "\"roadSections\":[{\"sectionKey\":\"shared.key\",\"kindId\":\"motorLane\",\"lanes\":[{\"laneKey\":\"shared.key\",\"laneEdgeKey\":\"shared.key\",\"direction\":\"forward\",\"widthMeters\":3.5,\"speedLimitMetersPerSecond\":10,\"successors\":[]}],\"laneGroups\":[]}],",
            "\"facilityBands\":[]}"
        );
        let shared_road = road_fragment("shared.key", shared_span)
            .replace("\"frame\":\"frame.main\"", "\"frame\":\"shared.key\"");
        let source = road_document_with_frame("shared.key", &format!("[{shared_road}]"), "[]");
        finish_document(source.as_bytes(), &CompileLimits::p100_initial_v1()).unwrap();
    }

    #[test]
    fn finish_resolves_local_and_imported_references_and_rejects_unimported_modules() {
        let span = concat!(
            "{\"spanKey\":\"span.main\",\"corridorKey\":\"corridor.main\",\"startStationMeters\":0,",
            "\"endStationMeters\":\"end\",\"referenceSectionKey\":\"section.main\",\"referenceLaneKey\":\"lane.main\",",
            "\"elements\":[{\"kind\":\"roadSection\",\"sectionKey\":\"section.main\"}],",
            "\"roadSections\":[{\"sectionKey\":\"section.main\",\"kindId\":\"motorLane\",\"lanes\":[{\"laneKey\":\"lane.main\",\"laneEdgeKey\":\"edge.main\",\"direction\":\"forward\",\"widthMeters\":3.5,\"speedLimitMetersPerSecond\":10,",
            "\"successors\":[\"edge.local\",\"city/base::edge.external\"]}],\"laneGroups\":[]}],\"facilityBands\":[]}"
        );
        let source = road_document(
            &format!("[{}]", road_fragment("road.main", span)),
            "[\"city/base\"]",
        );
        let module = finish_document(source.as_bytes(), &CompileLimits::p100_initial_v1()).unwrap();
        let TypedAstDeclaration::LaneEdge(edge) = &module.admitted.declarations[5] else {
            panic!("minimal lowering order checked by the full document test");
        };
        // successors 按 (module namespace, declaration key) 规范化排序。
        let successors: Vec<(&str, &str)> = edge
            .successors
            .iter()
            .map(|successor| {
                (
                    successor.module_namespace.as_ref(),
                    successor.declaration_key.as_ref(),
                )
            })
            .collect();
        assert_eq!(
            successors,
            [("city/base", "edge.external"), ("city/main", "edge.local")]
        );
        assert_eq!(
            module.descriptor().imports().collect::<Vec<_>>(),
            ["city/base"]
        );

        let unimported = source.replace("city/base::edge.external", "city/other::edge.external");
        let error = finish_document(unimported.as_bytes(), &CompileLimits::p100_initial_v1())
            .err()
            .unwrap();
        assert_eq!(error.diagnostics().len(), 1);
        assert!(matches!(
            error.diagnostics()[0].payload(),
            DiagnosticPayload::UnimportedReferenceModule { namespace } if namespace.as_ref() == "city/other"
        ));
    }

    #[test]
    fn finish_rejects_unknown_and_mismatched_facility_kinds() {
        // MINIMAL_DOCUMENT 的 `road.vehicle` 不是共同设施类别种子；finish 失败关闭。
        let error = finish_document(
            super::schema::MINIMAL_DOCUMENT,
            &CompileLimits::p100_initial_v1(),
        )
        .err()
        .unwrap();
        assert_eq!(error.diagnostics().len(), 1);
        assert!(matches!(
            error.diagnostics()[0].payload(),
            DiagnosticPayload::InvalidFacilityKind {
                entity_kind: EntityKind::RoadSection,
                violation: FacilityKindViolation::Unknown,
                ..
            }
        ));

        let span = concat!(
            "{\"spanKey\":\"span.main\",\"corridorKey\":\"corridor.main\",\"startStationMeters\":0,",
            "\"endStationMeters\":\"end\",\"referenceSectionKey\":\"section.main\",\"referenceLaneKey\":\"lane.main\",",
            "\"elements\":[{\"kind\":\"roadSection\",\"sectionKey\":\"section.main\"},{\"kind\":\"facilityBand\",\"facilityBandKey\":\"band.walk\"}],",
            "\"roadSections\":[{\"sectionKey\":\"section.main\",\"kindId\":\"motorLane\",\"lanes\":[{\"laneKey\":\"lane.main\",\"laneEdgeKey\":\"edge.main\",\"direction\":\"forward\",\"widthMeters\":3.5,\"speedLimitMetersPerSecond\":10,\"successors\":[]}],\"laneGroups\":[]}],",
            "\"facilityBands\":[{\"facilityBandKey\":\"band.walk\",\"kindId\":\"motorLane\",\"widthMeters\":2}]}"
        );
        let source = road_document(&format!("[{}]", road_fragment("road.main", span)), "[]");
        let error = finish_document(source.as_bytes(), &CompileLimits::p100_initial_v1())
            .err()
            .unwrap();
        assert!(matches!(
            error.diagnostics()[0].payload(),
            DiagnosticPayload::InvalidFacilityKind {
                entity_kind: EntityKind::FacilityBand,
                expected_category: FacilityKindCategory::NonTraversable,
                violation: FacilityKindViolation::CategoryMismatch {
                    actual: FacilityKindCategory::LaneBearing,
                },
                ..
            }
        ));
    }

    #[test]
    fn finish_enforces_section_direction_elements_and_reference_section() {
        let mixed_direction = concat!(
            "{\"spanKey\":\"span.main\",\"corridorKey\":\"corridor.main\",\"startStationMeters\":0,",
            "\"endStationMeters\":\"end\",\"referenceSectionKey\":\"section.main\",\"referenceLaneKey\":\"lane.f\",",
            "\"elements\":[{\"kind\":\"roadSection\",\"sectionKey\":\"section.main\"}],",
            "\"roadSections\":[{\"sectionKey\":\"section.main\",\"kindId\":\"motorLane\",\"lanes\":[",
            "{\"laneKey\":\"lane.f\",\"laneEdgeKey\":\"edge.f\",\"direction\":\"forward\",\"widthMeters\":3.5,\"speedLimitMetersPerSecond\":10,\"successors\":[]},",
            "{\"laneKey\":\"lane.b\",\"laneEdgeKey\":\"edge.b\",\"direction\":\"backward\",\"widthMeters\":3.5,\"speedLimitMetersPerSecond\":10,\"successors\":[]}",
            "],\"laneGroups\":[]}],\"facilityBands\":[]}"
        );
        let duplicate_element = MINIMAL_SPAN.replace(
            "\"elements\":[{\"kind\":\"roadSection\",\"sectionKey\":\"section.main\"}]",
            "\"elements\":[{\"kind\":\"roadSection\",\"sectionKey\":\"section.main\"},{\"kind\":\"roadSection\",\"sectionKey\":\"section.main\"}]",
        );
        let missing_member = MINIMAL_SPAN.replace(
            "\"elements\":[{\"kind\":\"roadSection\",\"sectionKey\":\"section.main\"}]",
            "\"elements\":[{\"kind\":\"roadSection\",\"sectionKey\":\"section.missing\"}]",
        );
        let missing_reference = MINIMAL_SPAN.replace(
            "\"referenceSectionKey\":\"section.main\"",
            "\"referenceSectionKey\":\"section.missing\"",
        );
        let cases = [
            // 混合 direction 在 numeric freeze（②）先闭合；③ 的同向检查是兜底。
            (mixed_direction, "lanes.direction"),
            (duplicate_element.as_str(), "crossSectionSpans[].elements"),
            // elements 指向未声明成员同样由 freeze 的横断面布局解析先闭合。
            (missing_member.as_str(), "elements.sectionKey"),
            (missing_reference.as_str(), "referenceSectionKey"),
        ];
        for (span, expected_field) in cases {
            let source = road_document(&format!("[{}]", road_fragment("road.main", span)), "[]");
            let error = finish_document(source.as_bytes(), &CompileLimits::p100_initial_v1())
                .err()
                .unwrap();
            let payload = error.diagnostics()[0].payload();
            assert!(
                matches!(
                    payload,
                    DiagnosticPayload::InvalidGeometryDocument {
                        violation: GeometryDocumentViolation::FieldValue,
                        field: Some(field),
                        ..
                    } if field.as_ref() == expected_field
                ),
                "case {expected_field}: unexpected payload: {payload:?}"
            );
        }
    }

    #[test]
    fn finish_enforces_module_level_limits_with_module_context() {
        let source = valid_minimal_document();
        let cases = [
            (CompileLimitDimension::DeclarationCount, 3, 7),
            (CompileLimitDimension::TypedAstRecordCount, 6, 7),
        ];
        for (dimension, limit, observed) in cases {
            let limits =
                CompileLimits::p100_initial_v1().with_test_admission_limit(dimension, limit);
            let error = finish_document(source.as_bytes(), &limits).err().unwrap();
            assert_eq!(error.diagnostics().len(), 1);
            let diagnostic = &error.diagnostics()[0];
            assert!(matches!(
                diagnostic.payload(),
                DiagnosticPayload::CompileLimitExceeded {
                    dimension: actual,
                    limit: actual_limit,
                    observed: actual_observed,
                } if *actual == dimension && *actual_limit == u64::from(limit) && *actual_observed == observed
            ));
            assert!(diagnostic.primary_span().is_some());
        }
    }

    #[test]
    fn new_enforces_stage_scratch_bytes_at_parse_stage() {
        // §7.2 阶段 1 parser 资源错误：每层容器 512B，本文档最深 10 层
        // （roads→road→crossSectionSpans→span→roadSections→section→lanes→lane→successors），
        // 限 1024 时第三层 overlays.signalGroups 的候选 3×512=1536B 越限，new 即失败关闭。
        let source = valid_minimal_document();
        let limits = CompileLimits::p100_initial_v1()
            .with_test_admission_limit(CompileLimitDimension::StageScratchBytes, 1024);
        let error = GeometryModuleBuilder::new(
            GeometryDocumentInput::new("source/main", source.as_bytes(), None),
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            &limits,
        )
        .err()
        .unwrap();
        assert_eq!(error.diagnostics().len(), 1);
        let diagnostic = &error.diagnostics()[0];
        assert!(matches!(
            diagnostic.payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::StageScratchBytes,
                limit: 1024,
                observed: 1536,
            }
        ));
    }

    #[test]
    fn finish_enforces_stage_scratch_bytes_at_freeze_stage() {
        // 实测：文档最深 10 层容器 → parse 峰值 10×512 = 5120B；
        // 200 条 line segment 的 station freeze 峰值 ≈ 201×40 + 200×40 + 32 = 16072B。
        // 阈值 8192 让 parse（阶段 1）通过、freeze（阶段 2）失败关闭。
        let segments = (1..=200)
            .map(|index| format!(r#"{{"kind":"line","end":[{},0,0]}}"#, 10 * index))
            .collect::<Vec<_>>()
            .join(",");
        let source = valid_minimal_document().replace(
            r#""segments":[{"kind":"line","end":[10,0,0]}]"#,
            &format!("\"segments\":[{}]", segments),
        );
        assert!(source.contains("\"end\":[2000,0,0]"));
        let limits = CompileLimits::p100_initial_v1()
            .with_test_admission_limit(CompileLimitDimension::StageScratchBytes, 8192);
        let error = finish_document(source.as_bytes(), &limits).err().unwrap();
        assert_eq!(error.diagnostics().len(), 1);
        let diagnostic = &error.diagnostics()[0];
        assert!(matches!(
            diagnostic.payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::StageScratchBytes,
                limit: 8192,
                observed: 8193,
            }
        ));
    }

    #[test]
    fn finish_resource_counts_follow_the_wire_and_declaration_formulas() {
        let source = valid_minimal_document();
        let module = finish_document(source.as_bytes(), &CompileLimits::p100_initial_v1()).unwrap();
        let counts = &module.admitted.resource_counts;

        let names: Vec<&str> = module
            .admitted
            .declarations
            .iter()
            .map(declaration_name)
            .collect();
        assert_eq!(
            names,
            [
                "CanonicalFrame",
                "GeometryReferenceLine",
                "GeometryCrossSectionSpan",
                "RoadCorridor",
                "RoadSection",
                "LaneEdge",
            ]
        );

        assert_eq!(counts.source_bytes, u64::try_from(source.len()).unwrap());
        // 6 个顶层声明 + RoadSection 嵌套的 1 条编制车道。
        assert_eq!(counts.declaration_count, 7);
        // §7.1 wire 口径：模块头 1 + frame 1 + road 1 + 参考线段 1 + span 1
        // + section 1 + lane 1。
        assert_eq!(counts.typed_ast_record_count, 7);
        // 引用：frame←road 1 + frame←span 1 + corridor←span 1 + referenceSection/elements 2
        // + lane→edge 1；intent 只进 reference 维度。
        assert_eq!(counts.reference_count, 6);
        assert_eq!(counts.relation_occurrence_count, 4);
        assert_eq!(counts.identity_field_occurrence_count, 12);
        assert_eq!(counts.symbol_count, 5);
        assert_eq!(counts.string_item_count, 24);
        assert_eq!(counts.string_bytes, 298);
        assert_eq!(counts.maneuver_gate_count, 0);
        assert_eq!(counts.waiting_zone_count, 0);
        assert_eq!(counts.route_occurrence_count, 0);
        assert_eq!(counts.geometry_point_count, 2);

        let structural = size_bytes::<CanonicalFrameDeclaration>(1)
            + size_bytes::<GeometryReferenceLineIntent>(1)
            + size_bytes::<GeometryCrossSectionSpanIntent>(1)
            + size_bytes::<OwnedEntityReference<CanonicalFrameKind>>(1)
            + size_bytes::<GeometryOffsetIntent>(1)
            + size_bytes::<RoadCorridorDeclaration>(1)
            + size_bytes::<OwnedEntityReference<RoadSectionKind>>(1)
            + size_bytes::<OwnedCorridorElementReference>(1)
            + size_bytes::<RoadSectionDeclaration>(1)
            + size_bytes::<AuthoringLaneDeclaration>(1)
            + size_bytes::<OwnedEntityReference<LaneEdgeKind>>(1)
            + size_bytes::<OwnedEntityReference<LaneGroupKind>>(0)
            + size_bytes::<LaneEdgeDeclaration>(1);
        let payload_bytes =
            size_bytes::<FrozenLateralCurve>(1) + size_bytes::<FrozenCanonicalPoint>(2);
        // controlled 字符串：模块头 ns/doc/generator/provenance 54 + 各声明 155 = 209。
        let expected_live = 209
            + structural
            + size_bytes::<super::super::descriptor::SourceDocumentDescriptor>(1)
            + payload_bytes;
        assert_eq!(counts.controlled_live_bytes, expected_live);
    }

    #[test]
    fn finish_resource_counts_include_internal_edge_records_references_and_points() {
        let source = two_lane_document(&format!(
            "[{}]",
            junction_fragment(
                &format!("[{INTERNAL_EDGE}]"),
                &format!(
                    "[{}]",
                    connection_fragment("movement.a", "path.a", "[\"edge.internal\"]")
                ),
            )
        ));
        let module = finish_document(source.as_bytes(), &CompileLimits::p100_initial_v1()).unwrap();
        let counts = &module.admitted.resource_counts;

        let names: Vec<&str> = module
            .admitted
            .declarations
            .iter()
            .map(declaration_name)
            .collect();
        assert_eq!(
            names,
            [
                "CanonicalFrame",
                "GeometryReferenceLine",
                "GeometryCrossSectionSpan",
                "RoadCorridor",
                "RoadSection",
                "LaneEdge",
                "LaneEdge",
                "Junction",
                "LaneEdge",
                "GeometryInternalEdge",
                "Movement",
                "ManeuverPath",
                "GeometryConnection",
            ]
        );

        assert_eq!(counts.source_bytes, u64::try_from(source.len()).unwrap());
        // 13 个顶层声明 + RoadSection 嵌套的 2 条编制车道。
        assert_eq!(counts.declaration_count, 15);
        // §7.1 wire 口径：模块头 1 + frame 1 + road 1 + 参考线段 1 + span 1
        // + section 1 + lane 2 + junction 1 + connection 1 + internal edge 1。
        assert_eq!(counts.typed_ast_record_count, 11);
        // 引用：frame←road 1 + frame←span 1 + corridor←span 1 + referenceSection/elements 2
        // + lane→edge 2 + junction←intent 1 + junction←movement 1
        // + path 4（movement/entry/internal/exit）+ connection intent 5。
        assert_eq!(counts.reference_count, 18);
        // corridor 2 + section 4 + movement 1 + path 4；intent 不进 relation 维度。
        assert_eq!(counts.relation_occurrence_count, 11);
        assert_eq!(counts.identity_field_occurrence_count, 31);
        assert_eq!(counts.symbol_count, 11);
        assert_eq!(counts.string_item_count, 53);
        assert_eq!(counts.string_bytes, 676);
        assert_eq!(counts.maneuver_gate_count, 0);
        assert_eq!(counts.waiting_zone_count, 0);
        assert_eq!(counts.route_occurrence_count, 0);
        // lateral 2×2 点 + internal edge 2 点。
        assert_eq!(counts.geometry_point_count, 6);

        let structural = size_bytes::<CanonicalFrameDeclaration>(1)
            + size_bytes::<GeometryReferenceLineIntent>(1)
            + size_bytes::<GeometryCrossSectionSpanIntent>(1)
            + size_bytes::<OwnedEntityReference<CanonicalFrameKind>>(1)
            + size_bytes::<GeometryOffsetIntent>(2)
            + size_bytes::<RoadCorridorDeclaration>(1)
            + size_bytes::<OwnedEntityReference<RoadSectionKind>>(1)
            + size_bytes::<OwnedCorridorElementReference>(1)
            + size_bytes::<RoadSectionDeclaration>(1)
            + size_bytes::<AuthoringLaneDeclaration>(2)
            + size_bytes::<OwnedEntityReference<LaneEdgeKind>>(2)
            + size_bytes::<OwnedEntityReference<LaneGroupKind>>(0)
            + size_bytes::<LaneEdgeDeclaration>(3)
            + size_bytes::<JunctionDeclaration>(1)
            + size_bytes::<GeometryInternalEdgeIntent>(1)
            + size_bytes::<OwnedEntityReference<JunctionKind>>(1)
            + size_bytes::<MovementDeclaration>(1)
            + size_bytes::<OwnedEntityReference<JunctionKind>>(1)
            + size_bytes::<ManeuverPathDeclaration>(1)
            + size_bytes::<OwnedEntityReference<MovementKind>>(1)
            + size_bytes::<OwnedEntityReference<LaneEdgeKind>>(3)
            + size_bytes::<GeometryConnectionIntent>(1)
            + size_bytes::<OwnedEntityReference<LaneEdgeKind>>(1);
        let payload_bytes = size_bytes::<FrozenLateralCurve>(2)
            + size_bytes::<FrozenInternalEdgeCurve>(1)
            + size_bytes::<FrozenCanonicalPoint>(6);
        // controlled 字符串：模块头 ns/doc/generator/provenance 51 + 各声明 350 = 401。
        let expected_live = 401
            + structural
            + size_bytes::<super::super::descriptor::SourceDocumentDescriptor>(1)
            + payload_bytes;
        assert_eq!(counts.controlled_live_bytes, expected_live);
    }

    fn road_fragment(road_key: &str, span: &str) -> String {
        format!(
            concat!(
                "{{\"roadKey\":\"{road_key}\",\"frame\":\"frame.main\",",
                "\"referenceLine\":{{\"start\":[0,0,0],\"segments\":[{{\"kind\":\"line\",\"end\":[10,0,0]}}]}},",
                "\"crossSectionSpans\":[{span}]}}"
            ),
            road_key = road_key,
            span = span
        )
    }

    const MINIMAL_SPAN: &str = concat!(
        "{\"spanKey\":\"span.main\",\"corridorKey\":\"corridor.main\",\"startStationMeters\":0,",
        "\"endStationMeters\":\"end\",\"referenceSectionKey\":\"section.main\",\"referenceLaneKey\":\"lane.main\",",
        "\"elements\":[{\"kind\":\"roadSection\",\"sectionKey\":\"section.main\"}],",
        "\"roadSections\":[{\"sectionKey\":\"section.main\",\"kindId\":\"motorLane\",\"lanes\":[{\"laneKey\":\"lane.main\",\"laneEdgeKey\":\"edge.main\",\"direction\":\"forward\",\"widthMeters\":3.5,\"speedLimitMetersPerSecond\":10,\"successors\":[]}],\"laneGroups\":[]}],",
        "\"facilityBands\":[]}"
    );

    fn road_document(roads: &str, imports: &str) -> String {
        road_document_with_frame("frame.main", roads, imports)
    }

    fn road_document_with_frame(frame_key: &str, roads: &str, imports: &str) -> String {
        road_document_with(frame_key, roads, "[]", imports)
    }

    fn road_document_with_junctions(roads: &str, junctions: &str) -> String {
        road_document_with("frame.main", roads, junctions, "[]")
    }

    fn road_document_with(frame_key: &str, roads: &str, junctions: &str, imports: &str) -> String {
        let roads = if roads == "[]" {
            format!("[{}]", road_fragment("road.main", MINIMAL_SPAN))
        } else {
            roads.to_string()
        };
        format!(
            concat!(
                "{{\"geometryVersion\":\"1\",\"module\":{{\"namespace\":\"city/main\",\"documentKey\":\"source/main\",",
                "\"imports\":{imports},\"provenance\":{{\"kind\":\"direct\",\"description\":\"test\"}}}},",
                "\"units\":{{\"distance\":\"meter\",\"angle\":\"radian\",\"speed\":\"meter-per-second\",\"time\":\"second\"}},",
                "\"frames\":[{{\"frameKey\":\"{frame_key}\"}}],\"roads\":{roads},\"junctions\":{junctions},",
                "\"overlays\":{{\"signalGroups\":[],\"signalControllers\":[],\"parkingAreas\":[],\"parkingSpaces\":[],",
                "\"participantClasses\":[],\"vehicleProfiles\":[],\"accessRules\":[],\"staticRoutes\":[],",
                "\"stopLines\":[],\"maneuverGates\":[],\"waitingZones\":[]}}}}"
            ),
            frame_key = frame_key,
            roads = roads,
            junctions = junctions,
            imports = imports
        )
    }

    fn decode_digest(value: &str) -> [u8; 32] {
        assert_eq!(value.len(), 64);
        let mut output = [0_u8; 32];
        for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
            output[index] = (decode_nibble(chunk[0]) << 4) | decode_nibble(chunk[1]);
        }
        output
    }

    const fn decode_nibble(value: u8) -> u8 {
        match value {
            b'0'..=b'9' => value - b'0',
            b'a'..=b'f' => value - b'a' + 10,
            _ => panic!("test digest must use lowercase hexadecimal"),
        }
    }
}

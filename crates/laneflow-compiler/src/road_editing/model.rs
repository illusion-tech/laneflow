use std::cmp::Ordering;
use std::marker::PhantomData;

use laneflow_static_contract::{
    AccessEffect, AccessRuleKind, AuthoringLaneKind, CANONICAL_POINT_COMPONENT_MAX_METERS,
    CANONICAL_POINT_COMPONENT_MIN_METERS, CanonicalFrameKind, ConflictZoneKind, EntityKind,
    EntityKindMarker, FacilityBandKind, JunctionKind, LaneEdgeKind, LaneGroupKind,
    MAX_CONFLICT_ZONE_REGION_RING_POINTS, MAX_LANE_EDGE_LENGTH_MM, MAX_MIN_GAP_MM,
    MAX_PARKING_LATERAL_OFFSET_ABS_MM, MAX_SPEED_MM_S, MAX_VEHICLE_LENGTH_MM,
    MIN_PARKING_LATERAL_OFFSET_ABS_MM, MIN_SPEED_MM_S, MIN_VEHICLE_LENGTH_MM, ManeuverGateKind,
    ManeuverPathKind, MovementKind, PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM, ParkingFacilityKind,
    ParkingSpaceKind, ParticipantClassKind, ParticipantStreamKind, RoadCorridorKind,
    RoadSectionKind, SignalAspect, SignalControllerKind, SignalGroupKind, SignalPhaseKind,
    StopLineKind, VehicleProfileKind, WaitingZoneKind,
};

use super::rules::{
    accel_violation, heading_violation, input_error, millimetre_i32_abs_range_violation,
    millimetre_range_violation, require_non_empty, require_unique, time_headway_violation,
    validate_facility_kind, validate_inclusive_range, validate_non_empty_text,
    validate_non_negative, validate_positive, validate_token, validate_visible_ascii,
};
use crate::declaration::MAX_PORTABLE_SIGNAL_TIME_MS;
use crate::{DiagnosticBundle, FacilityKindCategory, RoadEditingInputViolation};

/// 直接编制来源沿袭使用的冻结生成器构建标识。
pub(super) const DIRECT_GENERATOR_BUILD_ID: &str = "laneflow-road-editing-direct-v1";
/// 直接编制来源沿袭的冻结参数与输入摘要。
pub(super) const DIRECT_INPUTS_DIGEST: [u8; 32] = [
    0x6b, 0x27, 0xd0, 0xf7, 0x66, 0x93, 0xbc, 0xd3, 0x86, 0xac, 0x13, 0xdf, 0x72, 0x4e, 0x30, 0xf5,
    0xfb, 0x5a, 0xd3, 0xb9, 0xa1, 0x52, 0xa5, 0xe1, 0xf8, 0x8d, 0xe1, 0xa6, 0x24, 0xce, 0xa8, 0xaa,
];
/// 直接编制来源沿袭的冻结前端选项摘要。
pub(super) const DIRECT_FRONTEND_OPTIONS_DIGEST: [u8; 32] = [
    0xb1, 0x62, 0x1e, 0x4a, 0x2d, 0xb8, 0xd7, 0x17, 0xb6, 0x50, 0x6b, 0x0a, 0xfb, 0x6f, 0xef, 0x5b,
    0xd4, 0xd5, 0x15, 0x6e, 0xcf, 0xe8, 0x87, 0xc5, 0xab, 0xf3, 0x6d, 0x08, 0x86, 0x9c, 0x78, 0x92,
];

/// 指向道路编辑来源中某个稳定实体的拥有型、有类型引用。
///
/// owner-scoped 种类必须携带从模块根 owner 到直接 parent 的完整 key 链；类型参数
/// `K` 来自封闭的 Identity v1 registry，构造器据此检查精确链深。
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RoadEditingReference<K: EntityKindMarker> {
    module_namespace: Option<Box<str>>,
    owner_keys: Box<[Box<str>]>,
    local_key: Box<str>,
    marker: PhantomData<fn() -> K>,
}

impl<K: EntityKindMarker> RoadEditingReference<K> {
    /// 构造指向当前模块 module-scoped 实体的引用。
    pub fn local(local_key: impl Into<String>) -> Result<Self, DiagnosticBundle> {
        Self::try_new(None, Vec::new(), local_key.into())
    }

    /// 构造指向当前模块 owner-scoped 实体的完整引用。
    pub fn owner_scoped(
        owner_keys: Vec<String>,
        local_key: impl Into<String>,
    ) -> Result<Self, DiagnosticBundle> {
        Self::try_new(None, owner_keys, local_key.into())
    }

    /// 构造指向显式导入模块实体的完整引用。
    pub fn imported(
        module_namespace: impl Into<String>,
        owner_keys: Vec<String>,
        local_key: impl Into<String>,
    ) -> Result<Self, DiagnosticBundle> {
        Self::try_new(Some(module_namespace.into()), owner_keys, local_key.into())
    }

    fn try_new(
        module_namespace: Option<String>,
        owner_keys: Vec<String>,
        local_key: String,
    ) -> Result<Self, DiagnosticBundle> {
        let field = format!("reference<{}>", K::KIND.slug());
        let expected = owner_depth(K::KIND);
        let actual = u8::try_from(owner_keys.len()).unwrap_or(u8::MAX);
        if actual != expected {
            return Err(input_error(
                &field,
                RoadEditingInputViolation::InvalidReferenceDepth { expected, actual },
            ));
        }
        if let Some(namespace) = module_namespace.as_deref() {
            validate_token(namespace, &format!("{field}.moduleNamespace"))?;
        }
        for (index, owner_key) in owner_keys.iter().enumerate() {
            validate_token(owner_key, &format!("{field}.ownerKeys[{index}]"))?;
        }
        validate_token(&local_key, &format!("{field}.localKey"))?;
        Ok(Self {
            module_namespace: module_namespace.map(Into::into),
            owner_keys: owner_keys.into_iter().map(String::into_boxed_str).collect(),
            local_key: local_key.into_boxed_str(),
            marker: PhantomData,
        })
    }

    /// 返回限定引用的目标 namespace；`None` 表示当前模块。
    #[must_use]
    pub fn module_namespace(&self) -> Option<&str> {
        self.module_namespace.as_deref()
    }

    /// 返回从模块根 owner 到直接 parent 的完整 key 链。
    #[must_use]
    pub fn owner_keys(&self) -> impl ExactSizeIterator<Item = &str> {
        self.owner_keys.iter().map(AsRef::as_ref)
    }

    /// 返回目标实体在直接 owner 下的 local key。
    #[must_use]
    pub fn local_key(&self) -> &str {
        &self.local_key
    }

    /// 依次返回 owner key 链各组成部分与 local key。
    pub(super) fn components(&self) -> impl Iterator<Item = &str> {
        self.owner_keys().chain(std::iter::once(self.local_key()))
    }

    /// 拼出引用在来源缓冲区 wire 编码中的完整字符串形式。
    pub(super) fn wire_spelling(&self) -> String {
        let mut spelling = String::with_capacity(self.wire_len());
        if let Some(namespace) = &self.module_namespace {
            spelling.push_str(namespace);
            spelling.push_str("::");
        }
        for (index, component) in self.components().enumerate() {
            if index > 0 {
                spelling.push('>');
            }
            spelling.push_str(component);
        }
        spelling
    }

    /// 返回 wire 拼写的字节长度。
    pub(super) fn wire_len(&self) -> usize {
        self.components()
            .map(str::len)
            .sum::<usize>()
            .saturating_add(self.owner_keys.len())
            .saturating_add(
                self.module_namespace
                    .as_ref()
                    .map_or(0, |namespace| namespace.len().saturating_add(2)),
            )
    }

    /// 按规范顺序比较两个引用目标；未限定 namespace 时按当前模块 namespace 比较。
    pub(super) fn canonical_target_cmp(&self, other: &Self, current_namespace: &str) -> Ordering {
        self.module_namespace()
            .unwrap_or(current_namespace)
            .as_bytes()
            .cmp(
                other
                    .module_namespace()
                    .unwrap_or(current_namespace)
                    .as_bytes(),
            )
            .then_with(|| self.owner_keys().cmp(other.owner_keys()))
            .then_with(|| {
                self.local_key()
                    .as_bytes()
                    .cmp(other.local_key().as_bytes())
            })
    }
}

const fn owner_depth(kind: EntityKind) -> u8 {
    match kind {
        EntityKind::RoadSection
        | EntityKind::Movement
        | EntityKind::ConflictZone
        | EntityKind::ParticipantStream
        | EntityKind::FacilityBand
        | EntityKind::SignalPhase => 1,
        EntityKind::AuthoringLane | EntityKind::ManeuverPath | EntityKind::LaneGroup => 2,
        EntityKind::ManeuverGate | EntityKind::WaitingZone => 3,
        EntityKind::RoadCorridor
        | EntityKind::LaneEdge
        | EntityKind::Junction
        | EntityKind::StopLine
        | EntityKind::SignalGroup
        | EntityKind::SignalController
        | EntityKind::ParkingFacility
        | EntityKind::ParkingSpace
        | EntityKind::ParticipantClass
        | EntityKind::AccessRule
        | EntityKind::VehicleProfile
        | EntityKind::RightOfWayPolicySet
        | EntityKind::CanonicalFrame => 0,
    }
}

/// 指向道路走廊声明的有类型引用。
pub type RoadCorridorReference = RoadEditingReference<RoadCorridorKind>;
/// 指向道路区段声明的有类型引用。
pub type RoadSectionReference = RoadEditingReference<RoadSectionKind>;
/// 指向编制车道声明的有类型引用。
pub type AuthoringLaneReference = RoadEditingReference<AuthoringLaneKind>;
/// 指向车道图边声明的有类型引用。
pub type LaneEdgeReference = RoadEditingReference<LaneEdgeKind>;
/// 指向路口声明的有类型引用。
pub type JunctionReference = RoadEditingReference<JunctionKind>;
/// 指向通行流向声明的有类型引用。
pub type MovementReference = RoadEditingReference<MovementKind>;
/// 指向机动路径声明的有类型引用。
pub type ManeuverPathReference = RoadEditingReference<ManeuverPathKind>;
/// 指向机动门声明的有类型引用。
pub type ManeuverGateReference = RoadEditingReference<ManeuverGateKind>;
/// 指向等待区声明的有类型引用。
pub type WaitingZoneReference = RoadEditingReference<WaitingZoneKind>;
/// 指向停止线声明的有类型引用。
pub type StopLineReference = RoadEditingReference<StopLineKind>;
/// 指向信号组声明的有类型引用。
pub type SignalGroupReference = RoadEditingReference<SignalGroupKind>;
/// 指向信号控制器声明的有类型引用。
pub type SignalControllerReference = RoadEditingReference<SignalControllerKind>;
/// 指向信号相位声明的有类型引用。
pub type SignalPhaseReference = RoadEditingReference<SignalPhaseKind>;
/// 指向停车设施声明的有类型引用。
pub type ParkingFacilityReference = RoadEditingReference<ParkingFacilityKind>;
/// 指向停车位声明的有类型引用。
pub type ParkingSpaceReference = RoadEditingReference<ParkingSpaceKind>;
/// 指向车道组声明的有类型引用。
pub type LaneGroupReference = RoadEditingReference<LaneGroupKind>;
/// 指向设施带声明的有类型引用。
pub type FacilityBandReference = RoadEditingReference<FacilityBandKind>;
/// 指向参与者类别声明的有类型引用。
pub type ParticipantClassReference = RoadEditingReference<ParticipantClassKind>;
/// 指向准入规则声明的有类型引用。
pub type AccessRuleReference = RoadEditingReference<AccessRuleKind>;
/// 指向车辆配置声明的有类型引用。
pub type VehicleProfileReference = RoadEditingReference<VehicleProfileKind>;
/// 指向规范坐标框架声明的有类型引用。
pub type CanonicalFrameReference = RoadEditingReference<CanonicalFrameKind>;
/// 指向冲突区声明的有类型引用。
pub type ConflictZoneReference = RoadEditingReference<ConflictZoneKind>;
/// 指向参与者流声明的有类型引用。
pub type ParticipantStreamReference = RoadEditingReference<ParticipantStreamKind>;

/// 当前模块内、不进入 Identity v1 的道路走向键引用。
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RoadAlignmentReference(Box<str>);

impl RoadAlignmentReference {
    /// 构造道路走向键引用并校验键合法性。
    pub fn try_new(key: impl Into<String>) -> Result<Self, DiagnosticBundle> {
        let key = key.into();
        validate_token(&key, "roadAlignmentReference")?;
        Ok(Self(key.into_boxed_str()))
    }

    /// 返回道路走向键。
    #[must_use]
    pub fn key(&self) -> &str {
        &self.0
    }
}

/// 模块来源沿袭类别。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RoadEditingProvenanceKind {
    Direct,
    Generated,
}

/// 第一方道路编辑来源沿袭。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoadEditingProvenance {
    kind: RoadEditingProvenanceKind,
    generator_build_id: Box<str>,
    parameters_and_inputs_digest: [u8; 32],
    frontend_options_digest: [u8; 32],
    random_seed: Option<u64>,
    description: Box<str>,
}

impl RoadEditingProvenance {
    /// 构造直接编制来源沿袭，使用冻结的构建标识与摘要。
    pub fn direct(description: impl Into<String>) -> Result<Self, DiagnosticBundle> {
        let description = description.into();
        validate_visible_ascii(&description, "provenance.description")?;
        Ok(Self {
            kind: RoadEditingProvenanceKind::Direct,
            generator_build_id: DIRECT_GENERATOR_BUILD_ID.into(),
            parameters_and_inputs_digest: DIRECT_INPUTS_DIGEST,
            frontend_options_digest: DIRECT_FRONTEND_OPTIONS_DIGEST,
            random_seed: None,
            description: description.into_boxed_str(),
        })
    }

    /// 构造程序化生成来源沿袭，记录生成器构建标识、摘要与可选随机种子。
    pub fn generated(
        generator_build_id: impl Into<String>,
        parameters_and_inputs_digest: [u8; 32],
        frontend_options_digest: [u8; 32],
        random_seed: Option<u64>,
        description: impl Into<String>,
    ) -> Result<Self, DiagnosticBundle> {
        let generator_build_id = generator_build_id.into();
        let description = description.into();
        validate_token(&generator_build_id, "provenance.generatorBuildId")?;
        validate_visible_ascii(&description, "provenance.description")?;
        Ok(Self {
            kind: RoadEditingProvenanceKind::Generated,
            generator_build_id: generator_build_id.into_boxed_str(),
            parameters_and_inputs_digest,
            frontend_options_digest,
            random_seed,
            description: description.into_boxed_str(),
        })
    }

    /// 返回来源沿袭类别。
    #[must_use]
    pub const fn kind(&self) -> RoadEditingProvenanceKind {
        self.kind
    }

    /// 返回生成器构建标识。
    #[must_use]
    pub fn generator_build_id(&self) -> &str {
        &self.generator_build_id
    }

    /// 返回生成参数与输入摘要。
    #[must_use]
    pub const fn parameters_and_inputs_digest(&self) -> &[u8; 32] {
        &self.parameters_and_inputs_digest
    }

    /// 返回前端选项摘要。
    #[must_use]
    pub const fn frontend_options_digest(&self) -> &[u8; 32] {
        &self.frontend_options_digest
    }

    /// 返回可选随机种子。
    #[must_use]
    pub const fn random_seed(&self) -> Option<u64> {
        self.random_seed
    }

    /// 返回来源描述文本。
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }
}

/// 道路编辑模块头。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoadEditingModuleHeader {
    authoring_namespace_id: Box<str>,
    source_document_key: Box<str>,
    imports: Box<[Box<str>]>,
    provenance: RoadEditingProvenance,
}

impl RoadEditingModuleHeader {
    /// 构造模块头并校验编制命名空间、来源文档键与导入列表。
    pub fn try_new(
        authoring_namespace_id: impl Into<String>,
        source_document_key: impl Into<String>,
        imports: Vec<String>,
        provenance: RoadEditingProvenance,
    ) -> Result<Self, DiagnosticBundle> {
        let authoring_namespace_id = authoring_namespace_id.into();
        let source_document_key = source_document_key.into();
        validate_token(&authoring_namespace_id, "moduleHeader.authoringNamespaceId")?;
        validate_token(&source_document_key, "moduleHeader.sourceDocumentKey")?;
        for (index, import) in imports.iter().enumerate() {
            validate_token(import, &format!("moduleHeader.imports[{index}]"))?;
            if import == &authoring_namespace_id {
                return Err(input_error(
                    "moduleHeader.imports",
                    RoadEditingInputViolation::InvalidCombination,
                ));
            }
        }
        require_unique(&imports, "moduleHeader.imports")?;
        Ok(Self {
            authoring_namespace_id: authoring_namespace_id.into_boxed_str(),
            source_document_key: source_document_key.into_boxed_str(),
            imports: imports.into_iter().map(String::into_boxed_str).collect(),
            provenance,
        })
    }

    /// 返回编制命名空间标识。
    #[must_use]
    pub fn authoring_namespace_id(&self) -> &str {
        &self.authoring_namespace_id
    }

    /// 返回来源文档键。
    #[must_use]
    pub fn source_document_key(&self) -> &str {
        &self.source_document_key
    }

    /// 按声明顺序返回显式导入的模块命名空间。
    #[must_use]
    pub fn imports(&self) -> impl ExactSizeIterator<Item = &str> {
        self.imports.iter().map(AsRef::as_ref)
    }

    /// 返回模块来源沿袭。
    #[must_use]
    pub const fn provenance(&self) -> &RoadEditingProvenance {
        &self.provenance
    }
}

/// 编制坐标中的有限 `f64` 三维点。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoadEditingPoint3 {
    x: f64,
    y: f64,
    z: f64,
}

impl RoadEditingPoint3 {
    /// 构造三维点并校验各分量均在允许范围内。
    pub fn try_new(x: f64, y: f64, z: f64) -> Result<Self, DiagnosticBundle> {
        let minimum = f64::from(CANONICAL_POINT_COMPONENT_MIN_METERS);
        let maximum = f64::from(CANONICAL_POINT_COMPONENT_MAX_METERS);
        Ok(Self {
            x: validate_inclusive_range(x, minimum, maximum, "point.x")?,
            y: validate_inclusive_range(y, minimum, maximum, "point.y")?,
            z: validate_inclusive_range(z, minimum, maximum, "point.z")?,
        })
    }

    /// 返回 x 分量。
    #[must_use]
    pub const fn x(self) -> f64 {
        self.x
    }

    /// 返回 y 分量。
    #[must_use]
    pub const fn y(self) -> f64 {
        self.y
    }

    /// 返回 z 分量。
    #[must_use]
    pub const fn z(self) -> f64 {
        self.z
    }
}

/// 规范坐标框架 XZ 平面中的有限 `f64` 点。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoadEditingPoint2 {
    x: f64,
    z: f64,
}

impl RoadEditingPoint2 {
    /// 构造 XZ 平面点并校验各分量均在允许范围内。
    pub fn try_new(x: f64, z: f64) -> Result<Self, DiagnosticBundle> {
        let minimum = f64::from(CANONICAL_POINT_COMPONENT_MIN_METERS);
        let maximum = f64::from(CANONICAL_POINT_COMPONENT_MAX_METERS);
        Ok(Self {
            x: validate_inclusive_range(x, minimum, maximum, "point.x")?,
            z: validate_inclusive_range(z, minimum, maximum, "point.z")?,
        })
    }

    /// 返回 x 分量。
    #[must_use]
    pub const fn x(self) -> f64 {
        self.x
    }

    /// 返回 z 分量。
    #[must_use]
    pub const fn z(self) -> f64 {
        self.z
    }
}

/// corridor station 区间内的线性非负宽度。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinearWidthProfile {
    start_width_meters: f64,
    end_width_meters: f64,
}

impl LinearWidthProfile {
    /// 构造线性宽度配置；起止宽度必须非负且不同时为零。
    pub fn try_new(
        start_width_meters: f64,
        end_width_meters: f64,
    ) -> Result<Self, DiagnosticBundle> {
        let start_width_meters =
            validate_non_negative(start_width_meters, "widthProfile.startWidthMeters")?;
        let end_width_meters =
            validate_non_negative(end_width_meters, "widthProfile.endWidthMeters")?;
        if start_width_meters == 0.0 && end_width_meters == 0.0 {
            return Err(input_error(
                "widthProfile",
                RoadEditingInputViolation::InvalidCombination,
            ));
        }
        Ok(Self {
            start_width_meters,
            end_width_meters,
        })
    }

    /// 返回区间起点宽度（米）。
    #[must_use]
    pub const fn start_width_meters(self) -> f64 {
        self.start_width_meters
    }

    /// 返回区间终点宽度（米）。
    #[must_use]
    pub const fn end_width_meters(self) -> f64 {
        self.end_width_meters
    }
}

/// 一条编制曲线段的闭合几何 variant。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RoadEditingCurveSegmentGeometry {
    Line {
        end: RoadEditingPoint3,
    },
    CubicBezier {
        control_1: RoadEditingPoint3,
        control_2: RoadEditingPoint3,
        end: RoadEditingPoint3,
    },
}

/// 道路走向或路口内部边中的 owner-local 曲线段。
#[derive(Clone, Debug, PartialEq)]
pub struct RoadEditingCurveSegment {
    geometry: RoadEditingCurveSegmentGeometry,
    canvas_selection: Option<Box<str>>,
}

impl RoadEditingCurveSegment {
    /// 构造直线段。
    #[must_use]
    pub const fn line(end: RoadEditingPoint3) -> Self {
        Self {
            geometry: RoadEditingCurveSegmentGeometry::Line { end },
            canvas_selection: None,
        }
    }

    /// 构造三次贝塞尔曲线段。
    #[must_use]
    pub const fn cubic_bezier(
        control_1: RoadEditingPoint3,
        control_2: RoadEditingPoint3,
        end: RoadEditingPoint3,
    ) -> Self {
        Self {
            geometry: RoadEditingCurveSegmentGeometry::CubicBezier {
                control_1,
                control_2,
                end,
            },
            canvas_selection: None,
        }
    }

    /// 设置画布选择键。
    pub fn with_canvas_selection(
        mut self,
        canvas_selection: impl Into<String>,
    ) -> Result<Self, DiagnosticBundle> {
        self.canvas_selection = Some(validated_canvas(canvas_selection.into())?);
        Ok(self)
    }

    /// 返回曲线段几何。
    #[must_use]
    pub const fn geometry(&self) -> RoadEditingCurveSegmentGeometry {
        self.geometry
    }

    /// 返回可选画布选择键。
    #[must_use]
    pub fn canvas_selection(&self) -> Option<&str> {
        self.canvas_selection.as_deref()
    }
}

/// 一条从显式起点开始、至少包含一个 segment 的编制曲线。
#[derive(Clone, Debug, PartialEq)]
pub struct RoadEditingCurveProgram {
    start: RoadEditingPoint3,
    segments: Box<[RoadEditingCurveSegment]>,
}

impl RoadEditingCurveProgram {
    /// 构造编制曲线；segments 必须非空。
    pub fn try_new(
        start: RoadEditingPoint3,
        segments: Vec<RoadEditingCurveSegment>,
    ) -> Result<Self, DiagnosticBundle> {
        require_non_empty(&segments, "curveProgram.segments")?;
        Ok(Self {
            start,
            segments: segments.into_boxed_slice(),
        })
    }

    /// 返回曲线起点。
    #[must_use]
    pub const fn start(&self) -> RoadEditingPoint3 {
        self.start
    }

    /// 返回有序曲线段序列。
    #[must_use]
    pub fn segments(&self) -> &[RoadEditingCurveSegment] {
        &self.segments
    }
}

/// 当前模块中不分配 StableId 的道路走向定义。
#[derive(Clone, Debug, PartialEq)]
pub struct RoadAlignmentInput {
    road_alignment_key: Box<str>,
    canonical_frame: CanonicalFrameReference,
    reference_line: RoadEditingCurveProgram,
    canvas_selection: Option<Box<str>>,
}

impl RoadAlignmentInput {
    /// 构造道路走向定义并校验走向键。
    pub fn try_new(
        road_alignment_key: impl Into<String>,
        canonical_frame: CanonicalFrameReference,
        reference_line: RoadEditingCurveProgram,
    ) -> Result<Self, DiagnosticBundle> {
        let road_alignment_key = road_alignment_key.into();
        validate_token(&road_alignment_key, "roadAlignment.roadAlignmentKey")?;
        Ok(Self {
            road_alignment_key: road_alignment_key.into_boxed_str(),
            canonical_frame,
            reference_line,
            canvas_selection: None,
        })
    }

    /// 设置画布选择键。
    pub fn with_canvas_selection(
        mut self,
        canvas_selection: impl Into<String>,
    ) -> Result<Self, DiagnosticBundle> {
        self.canvas_selection = Some(validated_canvas(canvas_selection.into())?);
        Ok(self)
    }

    /// 返回道路走向键。
    #[must_use]
    pub fn road_alignment_key(&self) -> &str {
        &self.road_alignment_key
    }

    /// 返回走向几何所在的规范坐标框架引用。
    #[must_use]
    pub const fn canonical_frame(&self) -> &CanonicalFrameReference {
        &self.canonical_frame
    }

    /// 返回参考线曲线。
    #[must_use]
    pub const fn reference_line(&self) -> &RoadEditingCurveProgram {
        &self.reference_line
    }

    /// 返回可选画布选择键。
    #[must_use]
    pub fn canvas_selection(&self) -> Option<&str> {
        self.canvas_selection.as_deref()
    }
}

fn validated_canvas(value: String) -> Result<Box<str>, DiagnosticBundle> {
    validate_token(&value, "canvasSelection")?;
    Ok(value.into_boxed_str())
}

macro_rules! impl_canvas {
    ($type:ident) => {
        impl $type {
            /// 设置画布选择键。
            pub fn with_canvas_selection(
                mut self,
                canvas_selection: impl Into<String>,
            ) -> Result<Self, DiagnosticBundle> {
                self.canvas_selection = Some(validated_canvas(canvas_selection.into())?);
                Ok(self)
            }

            /// 返回可选画布选择键。
            #[must_use]
            pub fn canvas_selection(&self) -> Option<&str> {
                self.canvas_selection.as_deref()
            }
        }
    };
}

/// 道路走廊 station 区间的闭合终点形式。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RoadEditingStationEnd {
    Finite(f64),
    AlignmentEnd,
}

/// 道路走廊横断面中的有序成员引用。
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum RoadEditingCorridorElement {
    RoadSection(RoadSectionReference),
    FacilityBand(FacilityBandReference),
}

/// 编制车道相对 alignment 参考方向的行驶方向。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RoadEditingLaneDirection {
    Forward,
    Backward,
}

/// 道路走廊声明。
#[derive(Clone, Debug, PartialEq)]
pub struct RoadCorridorInput {
    road_corridor_key: Box<str>,
    road_alignment: RoadAlignmentReference,
    start_station_meters: f64,
    end_station: RoadEditingStationEnd,
    reference_section: RoadSectionReference,
    reference_lane: AuthoringLaneReference,
    elements: Box<[RoadEditingCorridorElement]>,
    canvas_selection: Option<Box<str>>,
}

impl RoadCorridorInput {
    /// 构造道路走廊声明并校验 station 区间、参考成员与元素序列。
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        road_corridor_key: impl Into<String>,
        road_alignment: RoadAlignmentReference,
        start_station_meters: f64,
        end_station: RoadEditingStationEnd,
        reference_section: RoadSectionReference,
        reference_lane: AuthoringLaneReference,
        elements: Vec<RoadEditingCorridorElement>,
    ) -> Result<Self, DiagnosticBundle> {
        let road_corridor_key = road_corridor_key.into();
        validate_token(&road_corridor_key, "roadCorridor.roadCorridorKey")?;
        let start_station_meters =
            validate_non_negative(start_station_meters, "roadCorridor.startStationMeters")?;
        let end_station = match end_station {
            RoadEditingStationEnd::Finite(end) => {
                let end = validate_positive(end, "roadCorridor.endStationMeters")?;
                if end <= start_station_meters {
                    return Err(input_error(
                        "roadCorridor.endStationMeters",
                        RoadEditingInputViolation::InvalidCombination,
                    ));
                }
                RoadEditingStationEnd::Finite(end)
            }
            RoadEditingStationEnd::AlignmentEnd => RoadEditingStationEnd::AlignmentEnd,
        };
        require_corridor_owned_reference(
            &reference_section,
            &road_corridor_key,
            "roadCorridor.referenceSection",
        )?;
        require_corridor_owned_reference(
            &reference_lane,
            &road_corridor_key,
            "roadCorridor.referenceLane",
        )?;
        require_non_empty(&elements, "roadCorridor.elements")?;
        require_unique(&elements, "roadCorridor.elements")?;
        Ok(Self {
            road_corridor_key: road_corridor_key.into_boxed_str(),
            road_alignment,
            start_station_meters,
            end_station,
            reference_section,
            reference_lane,
            elements: elements.into_boxed_slice(),
            canvas_selection: None,
        })
    }

    /// 返回道路走廊稳定键。
    #[must_use]
    pub fn road_corridor_key(&self) -> &str {
        &self.road_corridor_key
    }

    /// 返回走廊所沿的道路走向引用。
    #[must_use]
    pub const fn road_alignment(&self) -> &RoadAlignmentReference {
        &self.road_alignment
    }

    /// 返回起点 station（米）。
    #[must_use]
    pub const fn start_station_meters(&self) -> f64 {
        self.start_station_meters
    }

    /// 返回终点 station 形式。
    #[must_use]
    pub const fn end_station(&self) -> RoadEditingStationEnd {
        self.end_station
    }

    /// 返回参考道路区段引用。
    #[must_use]
    pub const fn reference_section(&self) -> &RoadSectionReference {
        &self.reference_section
    }

    /// 返回参考编制车道引用。
    #[must_use]
    pub const fn reference_lane(&self) -> &AuthoringLaneReference {
        &self.reference_lane
    }

    /// 返回横断面有序成员序列。
    #[must_use]
    pub fn elements(&self) -> &[RoadEditingCorridorElement] {
        &self.elements
    }
}
impl_canvas!(RoadCorridorInput);

fn require_corridor_owned_reference<K: EntityKindMarker>(
    reference: &RoadEditingReference<K>,
    corridor_key: &str,
    field: &'static str,
) -> Result<(), DiagnosticBundle> {
    if reference.module_namespace().is_some() || reference.owner_keys().next() != Some(corridor_key)
    {
        return Err(input_error(
            field,
            RoadEditingInputViolation::InvalidCombination,
        ));
    }
    Ok(())
}

/// 道路区段声明。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoadSectionInput {
    road_section_key: Box<str>,
    kind_id: Box<str>,
    authoring_lanes: Box<[AuthoringLaneReference]>,
    road_corridor: RoadCorridorReference,
    canvas_selection: Option<Box<str>>,
}

impl RoadSectionInput {
    /// 构造道路区段声明并校验键、设施类别与编制车道成员。
    pub fn try_new(
        road_section_key: impl Into<String>,
        kind_id: impl Into<String>,
        authoring_lanes: Vec<AuthoringLaneReference>,
        road_corridor: RoadCorridorReference,
    ) -> Result<Self, DiagnosticBundle> {
        let road_section_key = road_section_key.into();
        let kind_id = kind_id.into();
        validate_token(&road_section_key, "roadSection.roadSectionKey")?;
        validate_token(&kind_id, "roadSection.kindId")?;
        validate_facility_kind(
            &kind_id,
            FacilityKindCategory::LaneBearing,
            "roadSection.kindId",
        )?;
        require_non_empty(&authoring_lanes, "roadSection.authoringLanes")?;
        require_unique(&authoring_lanes, "roadSection.authoringLanes")?;
        Ok(Self {
            road_section_key: road_section_key.into_boxed_str(),
            kind_id: kind_id.into_boxed_str(),
            authoring_lanes: authoring_lanes.into_boxed_slice(),
            road_corridor,
            canvas_selection: None,
        })
    }

    /// 返回道路区段稳定键。
    #[must_use]
    pub fn road_section_key(&self) -> &str {
        &self.road_section_key
    }

    /// 返回区段设施类别标识。
    #[must_use]
    pub fn kind_id(&self) -> &str {
        &self.kind_id
    }

    /// 返回区段的编制车道成员序列。
    #[must_use]
    pub fn authoring_lanes(&self) -> &[AuthoringLaneReference] {
        &self.authoring_lanes
    }

    /// 返回所属道路走廊引用。
    #[must_use]
    pub const fn road_corridor(&self) -> &RoadCorridorReference {
        &self.road_corridor
    }
}
impl_canvas!(RoadSectionInput);

/// 编制车道声明。
#[derive(Clone, Debug, PartialEq)]
pub struct AuthoringLaneInput {
    authoring_lane_key: Box<str>,
    lane_edge: LaneEdgeReference,
    direction: RoadEditingLaneDirection,
    width_profile: LinearWidthProfile,
    lane_group: Option<LaneGroupReference>,
    road_section: RoadSectionReference,
    canvas_selection: Option<Box<str>>,
}

impl AuthoringLaneInput {
    /// 构造编制车道声明并校验车道键。
    pub fn try_new(
        authoring_lane_key: impl Into<String>,
        lane_edge: LaneEdgeReference,
        direction: RoadEditingLaneDirection,
        width_profile: LinearWidthProfile,
        lane_group: Option<LaneGroupReference>,
        road_section: RoadSectionReference,
    ) -> Result<Self, DiagnosticBundle> {
        let authoring_lane_key = authoring_lane_key.into();
        validate_token(&authoring_lane_key, "authoringLane.authoringLaneKey")?;
        Ok(Self {
            authoring_lane_key: authoring_lane_key.into_boxed_str(),
            lane_edge,
            direction,
            width_profile,
            lane_group,
            road_section,
            canvas_selection: None,
        })
    }

    /// 返回编制车道稳定键。
    #[must_use]
    pub fn authoring_lane_key(&self) -> &str {
        &self.authoring_lane_key
    }

    /// 返回车道展开到的车道图边引用。
    #[must_use]
    pub const fn lane_edge(&self) -> &LaneEdgeReference {
        &self.lane_edge
    }

    /// 返回相对 alignment 参考方向的行驶方向。
    #[must_use]
    pub const fn direction(&self) -> RoadEditingLaneDirection {
        self.direction
    }

    /// 返回线性宽度配置。
    #[must_use]
    pub const fn width_profile(&self) -> LinearWidthProfile {
        self.width_profile
    }

    /// 返回可选车道组成员引用。
    #[must_use]
    pub const fn lane_group(&self) -> Option<&LaneGroupReference> {
        self.lane_group.as_ref()
    }

    /// 返回所属道路区段引用。
    #[must_use]
    pub const fn road_section(&self) -> &RoadSectionReference {
        &self.road_section
    }
}
impl_canvas!(AuthoringLaneInput);

/// 车道图边声明。
#[derive(Clone, Debug, PartialEq)]
pub struct LaneEdgeInput {
    lane_edge_key: Box<str>,
    speed_limit_meters_per_second: f64,
    successors: Box<[LaneEdgeReference]>,
    explicit_geometry: Option<RoadEditingCurveProgram>,
    canvas_selection: Option<Box<str>>,
}

impl LaneEdgeInput {
    /// 构造车道图边声明并校验限速与后继序列。
    pub fn try_new(
        lane_edge_key: impl Into<String>,
        speed_limit_meters_per_second: f64,
        successors: Vec<LaneEdgeReference>,
        explicit_geometry: Option<RoadEditingCurveProgram>,
    ) -> Result<Self, DiagnosticBundle> {
        let lane_edge_key = lane_edge_key.into();
        validate_token(&lane_edge_key, "laneEdge.laneEdgeKey")?;
        let speed_limit_meters_per_second = require_closed_mm(
            speed_limit_meters_per_second,
            MIN_SPEED_MM_S,
            MAX_SPEED_MM_S,
            "laneEdge.speedLimitMetersPerSecond",
        )?;
        require_unique(&successors, "laneEdge.successors")?;
        Ok(Self {
            lane_edge_key: lane_edge_key.into_boxed_str(),
            speed_limit_meters_per_second,
            successors: successors.into_boxed_slice(),
            explicit_geometry,
            canvas_selection: None,
        })
    }

    /// 返回车道图边稳定键。
    #[must_use]
    pub fn lane_edge_key(&self) -> &str {
        &self.lane_edge_key
    }

    /// 返回限速（米每秒）。
    #[must_use]
    pub const fn speed_limit_meters_per_second(&self) -> f64 {
        self.speed_limit_meters_per_second
    }

    /// 返回后继车道图边引用序列。
    #[must_use]
    pub fn successors(&self) -> &[LaneEdgeReference] {
        &self.successors
    }

    /// 返回可选显式几何曲线。
    #[must_use]
    pub const fn explicit_geometry(&self) -> Option<&RoadEditingCurveProgram> {
        self.explicit_geometry.as_ref()
    }
}
impl_canvas!(LaneEdgeInput);

/// 路口声明。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JunctionInput {
    junction_key: Box<str>,
    approach_edges: Box<[LaneEdgeReference]>,
    internal_edges: Box<[LaneEdgeReference]>,
    canvas_selection: Option<Box<str>>,
}

impl JunctionInput {
    /// 构造路口声明；接近边必须非空且与内部边集合不相交。
    pub fn try_new(
        junction_key: impl Into<String>,
        approach_edges: Vec<LaneEdgeReference>,
        internal_edges: Vec<LaneEdgeReference>,
    ) -> Result<Self, DiagnosticBundle> {
        let junction_key = junction_key.into();
        validate_token(&junction_key, "junction.junctionKey")?;
        require_non_empty(&approach_edges, "junction.approachEdges")?;
        require_unique(&approach_edges, "junction.approachEdges")?;
        require_unique(&internal_edges, "junction.internalEdges")?;
        if approach_edges
            .iter()
            .any(|edge| internal_edges.contains(edge))
        {
            return Err(input_error(
                "junction.edgeRoles",
                RoadEditingInputViolation::InvalidCombination,
            ));
        }
        Ok(Self {
            junction_key: junction_key.into_boxed_str(),
            approach_edges: approach_edges.into_boxed_slice(),
            internal_edges: internal_edges.into_boxed_slice(),
            canvas_selection: None,
        })
    }

    /// 返回路口稳定键。
    #[must_use]
    pub fn junction_key(&self) -> &str {
        &self.junction_key
    }

    /// 返回接近边引用序列。
    #[must_use]
    pub fn approach_edges(&self) -> &[LaneEdgeReference] {
        &self.approach_edges
    }

    /// 返回内部边引用序列。
    #[must_use]
    pub fn internal_edges(&self) -> &[LaneEdgeReference] {
        &self.internal_edges
    }
}
impl_canvas!(JunctionInput);

/// 路口内一个通行流向声明。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MovementInput {
    movement_key: Box<str>,
    junction: JunctionReference,
    directed_entry_approach_key: Box<str>,
    directed_exit_approach_key: Box<str>,
    turn_direction: Option<crate::ManeuverDirection>,
    canvas_selection: Option<Box<str>>,
}

impl MovementInput {
    /// 构造通行流向声明并校验键与有向接近臂键。
    pub fn try_new(
        movement_key: impl Into<String>,
        junction: JunctionReference,
        directed_entry_approach_key: impl Into<String>,
        directed_exit_approach_key: impl Into<String>,
    ) -> Result<Self, DiagnosticBundle> {
        let movement_key = movement_key.into();
        let directed_entry_approach_key = directed_entry_approach_key.into();
        let directed_exit_approach_key = directed_exit_approach_key.into();
        validate_token(&movement_key, "movement.movementKey")?;
        validate_token(
            &directed_entry_approach_key,
            "movement.directedEntryApproachKey",
        )?;
        validate_token(
            &directed_exit_approach_key,
            "movement.directedExitApproachKey",
        )?;
        Ok(Self {
            movement_key: movement_key.into_boxed_str(),
            junction,
            directed_entry_approach_key: directed_entry_approach_key.into_boxed_str(),
            directed_exit_approach_key: directed_exit_approach_key.into_boxed_str(),
            turn_direction: None,
            canvas_selection: None,
        })
    }

    /// 返回通行流向稳定键。
    #[must_use]
    pub fn movement_key(&self) -> &str {
        &self.movement_key
    }

    /// 返回所属路口引用。
    #[must_use]
    pub const fn junction(&self) -> &JunctionReference {
        &self.junction
    }

    /// 返回有向入口接近臂键。
    #[must_use]
    pub fn directed_entry_approach_key(&self) -> &str {
        &self.directed_entry_approach_key
    }

    /// 返回有向出口接近臂键。
    #[must_use]
    pub fn directed_exit_approach_key(&self) -> &str {
        &self.directed_exit_approach_key
    }
}
impl_canvas!(MovementInput);

impl MovementInput {
    /// 设置显式机动方向，不改变稳定身份。
    #[must_use]
    pub const fn with_turn_direction(mut self, direction: crate::ManeuverDirection) -> Self {
        self.turn_direction = Some(direction);
        self
    }

    /// 返回可选显式机动方向。
    #[must_use]
    pub const fn turn_direction(&self) -> Option<crate::ManeuverDirection> {
        self.turn_direction
    }
}

/// 路口内一条机动路径声明。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManeuverPathInput {
    maneuver_path_key: Box<str>,
    movement: MovementReference,
    entry_edge: LaneEdgeReference,
    internal_edges: Box<[LaneEdgeReference]>,
    exit_edge: LaneEdgeReference,
    canvas_selection: Option<Box<str>>,
}

impl ManeuverPathInput {
    /// 构造机动路径声明并校验键与内部边序列。
    pub fn try_new(
        maneuver_path_key: impl Into<String>,
        movement: MovementReference,
        entry_edge: LaneEdgeReference,
        internal_edges: Vec<LaneEdgeReference>,
        exit_edge: LaneEdgeReference,
    ) -> Result<Self, DiagnosticBundle> {
        let maneuver_path_key = maneuver_path_key.into();
        validate_token(&maneuver_path_key, "maneuverPath.maneuverPathKey")?;
        require_unique(&internal_edges, "maneuverPath.internalEdges")?;
        Ok(Self {
            maneuver_path_key: maneuver_path_key.into_boxed_str(),
            movement,
            entry_edge,
            internal_edges: internal_edges.into_boxed_slice(),
            exit_edge,
            canvas_selection: None,
        })
    }

    /// 返回机动路径稳定键。
    #[must_use]
    pub fn maneuver_path_key(&self) -> &str {
        &self.maneuver_path_key
    }

    /// 返回所属通行流向引用。
    #[must_use]
    pub const fn movement(&self) -> &MovementReference {
        &self.movement
    }

    /// 返回入口边引用。
    #[must_use]
    pub const fn entry_edge(&self) -> &LaneEdgeReference {
        &self.entry_edge
    }

    /// 返回内部边引用序列。
    #[must_use]
    pub fn internal_edges(&self) -> &[LaneEdgeReference] {
        &self.internal_edges
    }

    /// 返回出口边引用。
    #[must_use]
    pub const fn exit_edge(&self) -> &LaneEdgeReference {
        &self.exit_edge
    }
}
impl_canvas!(ManeuverPathInput);

/// 机动门的固定时制信号绑定。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RoadEditingSignalControl {
    None,
    SignalGroup(SignalGroupReference),
}

/// 机动路径转换上的控制门声明。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManeuverGateInput {
    maneuver_gate_key: Box<str>,
    maneuver_path: ManeuverPathReference,
    transition_index: u32,
    stop_line: StopLineReference,
    signal_control: RoadEditingSignalControl,
    canvas_selection: Option<Box<str>>,
}

impl ManeuverGateInput {
    /// 构造机动门声明并校验门键。
    pub fn try_new(
        maneuver_gate_key: impl Into<String>,
        maneuver_path: ManeuverPathReference,
        transition_index: u32,
        stop_line: StopLineReference,
        signal_control: RoadEditingSignalControl,
    ) -> Result<Self, DiagnosticBundle> {
        let maneuver_gate_key = maneuver_gate_key.into();
        validate_token(&maneuver_gate_key, "maneuverGate.maneuverGateKey")?;
        Ok(Self {
            maneuver_gate_key: maneuver_gate_key.into_boxed_str(),
            maneuver_path,
            transition_index,
            stop_line,
            signal_control,
            canvas_selection: None,
        })
    }

    /// 返回机动门稳定键。
    #[must_use]
    pub fn maneuver_gate_key(&self) -> &str {
        &self.maneuver_gate_key
    }

    /// 返回绑定的机动路径引用。
    #[must_use]
    pub const fn maneuver_path(&self) -> &ManeuverPathReference {
        &self.maneuver_path
    }

    /// 返回门所在的路径转换下标。
    #[must_use]
    pub const fn transition_index(&self) -> u32 {
        self.transition_index
    }

    /// 返回关联停止线引用。
    #[must_use]
    pub const fn stop_line(&self) -> &StopLineReference {
        &self.stop_line
    }

    /// 返回固定时制信号绑定。
    #[must_use]
    pub const fn signal_control(&self) -> &RoadEditingSignalControl {
        &self.signal_control
    }
}
impl_canvas!(ManeuverGateInput);

/// 一条机动路径上的等待区声明。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WaitingZoneInput {
    waiting_zone_key: Box<str>,
    maneuver_path: ManeuverPathReference,
    entry_gate: ManeuverGateReference,
    release_gate: ManeuverGateReference,
    max_occupancy: u32,
    canvas_selection: Option<Box<str>>,
}

impl WaitingZoneInput {
    /// 构造等待区声明；等待容量必须大于零。
    pub fn try_new(
        waiting_zone_key: impl Into<String>,
        maneuver_path: ManeuverPathReference,
        entry_gate: ManeuverGateReference,
        release_gate: ManeuverGateReference,
        max_occupancy: u32,
    ) -> Result<Self, DiagnosticBundle> {
        let waiting_zone_key = waiting_zone_key.into();
        validate_token(&waiting_zone_key, "waitingZone.waitingZoneKey")?;
        if max_occupancy == 0 {
            return Err(input_error(
                "waitingZone.maxOccupancy",
                RoadEditingInputViolation::InvalidCombination,
            ));
        }
        Ok(Self {
            waiting_zone_key: waiting_zone_key.into_boxed_str(),
            maneuver_path,
            entry_gate,
            release_gate,
            max_occupancy,
            canvas_selection: None,
        })
    }

    /// 返回等待区稳定键。
    #[must_use]
    pub fn waiting_zone_key(&self) -> &str {
        &self.waiting_zone_key
    }

    /// 返回绑定的机动路径引用。
    #[must_use]
    pub const fn maneuver_path(&self) -> &ManeuverPathReference {
        &self.maneuver_path
    }

    /// 返回入口机动门引用。
    #[must_use]
    pub const fn entry_gate(&self) -> &ManeuverGateReference {
        &self.entry_gate
    }

    /// 返回放行机动门引用。
    #[must_use]
    pub const fn release_gate(&self) -> &ManeuverGateReference {
        &self.release_gate
    }

    /// 返回最大同时等待容量。
    #[must_use]
    pub const fn max_occupancy(&self) -> u32 {
        self.max_occupancy
    }
}
impl_canvas!(WaitingZoneInput);

/// 车道图边末端的停止线声明。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StopLineInput {
    stop_line_key: Box<str>,
    lane_edge: LaneEdgeReference,
    canvas_selection: Option<Box<str>>,
}

impl StopLineInput {
    /// 构造停止线声明并校验键。
    pub fn try_new(
        stop_line_key: impl Into<String>,
        lane_edge: LaneEdgeReference,
    ) -> Result<Self, DiagnosticBundle> {
        let stop_line_key = stop_line_key.into();
        validate_token(&stop_line_key, "stopLine.stopLineKey")?;
        Ok(Self {
            stop_line_key: stop_line_key.into_boxed_str(),
            lane_edge,
            canvas_selection: None,
        })
    }

    /// 返回停止线稳定键。
    #[must_use]
    pub fn stop_line_key(&self) -> &str {
        &self.stop_line_key
    }

    /// 返回停止线所在车道图边引用。
    #[must_use]
    pub const fn lane_edge(&self) -> &LaneEdgeReference {
        &self.lane_edge
    }
}
impl_canvas!(StopLineInput);

/// 固定时制控制器可控制的信号组声明。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignalGroupInput {
    signal_group_key: Box<str>,
    canvas_selection: Option<Box<str>>,
}

impl SignalGroupInput {
    /// 构造信号组声明并校验组键。
    pub fn try_new(signal_group_key: impl Into<String>) -> Result<Self, DiagnosticBundle> {
        let signal_group_key = signal_group_key.into();
        validate_token(&signal_group_key, "signalGroup.signalGroupKey")?;
        Ok(Self {
            signal_group_key: signal_group_key.into_boxed_str(),
            canvas_selection: None,
        })
    }

    /// 返回信号组稳定键。
    #[must_use]
    pub fn signal_group_key(&self) -> &str {
        &self.signal_group_key
    }
}
impl_canvas!(SignalGroupInput);

/// 固定时制相位中一个信号组的完整状态。
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RoadEditingSignalPhaseState {
    signal_group: SignalGroupReference,
    aspect: SignalAspect,
}

impl RoadEditingSignalPhaseState {
    /// 构造相位状态；指示只允许红、黄、绿。
    pub fn try_new(
        signal_group: SignalGroupReference,
        aspect: SignalAspect,
    ) -> Result<Self, DiagnosticBundle> {
        match aspect {
            SignalAspect::Red | SignalAspect::Yellow | SignalAspect::Green => {}
            _ => {
                return Err(input_error(
                    "signalPhaseState.aspect",
                    RoadEditingInputViolation::InvalidCombination,
                ));
            }
        }
        Ok(Self {
            signal_group,
            aspect,
        })
    }

    /// 返回信号组引用。
    #[must_use]
    pub const fn signal_group(&self) -> &SignalGroupReference {
        &self.signal_group
    }

    /// 返回该相位中信号组的指示。
    #[must_use]
    pub const fn aspect(&self) -> SignalAspect {
        self.aspect
    }
}

/// 固定时制信号控制器声明。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignalControllerInput {
    signal_controller_key: Box<str>,
    offset_milliseconds: u64,
    signal_groups: Box<[SignalGroupReference]>,
    signal_phases: Box<[SignalPhaseReference]>,
    canvas_selection: Option<Box<str>>,
}

impl SignalControllerInput {
    /// 构造信号控制器声明并校验键、周期偏移与信号组、相位序列。
    pub fn try_new(
        signal_controller_key: impl Into<String>,
        offset_milliseconds: u64,
        signal_groups: Vec<SignalGroupReference>,
        signal_phases: Vec<SignalPhaseReference>,
    ) -> Result<Self, DiagnosticBundle> {
        let signal_controller_key = signal_controller_key.into();
        validate_token(
            &signal_controller_key,
            "signalController.signalControllerKey",
        )?;
        if offset_milliseconds > MAX_PORTABLE_SIGNAL_TIME_MS {
            return Err(input_error(
                "signalController.offsetMilliseconds",
                RoadEditingInputViolation::InvalidCombination,
            ));
        }
        require_non_empty(&signal_groups, "signalController.signalGroups")?;
        require_unique(&signal_groups, "signalController.signalGroups")?;
        require_non_empty(&signal_phases, "signalController.signalPhases")?;
        require_unique(&signal_phases, "signalController.signalPhases")?;
        Ok(Self {
            signal_controller_key: signal_controller_key.into_boxed_str(),
            offset_milliseconds,
            signal_groups: signal_groups.into_boxed_slice(),
            signal_phases: signal_phases.into_boxed_slice(),
            canvas_selection: None,
        })
    }

    /// 返回信号控制器稳定键。
    #[must_use]
    pub fn signal_controller_key(&self) -> &str {
        &self.signal_controller_key
    }

    /// 返回控制器周期偏移（毫秒）。
    #[must_use]
    pub const fn offset_milliseconds(&self) -> u64 {
        self.offset_milliseconds
    }

    /// 返回受控信号组序列。
    #[must_use]
    pub fn signal_groups(&self) -> &[SignalGroupReference] {
        &self.signal_groups
    }

    /// 返回有序信号相位序列。
    #[must_use]
    pub fn signal_phases(&self) -> &[SignalPhaseReference] {
        &self.signal_phases
    }
}
impl_canvas!(SignalControllerInput);

/// 控制器内 owner-scoped 的固定时制相位声明。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignalPhaseInput {
    signal_phase_key: Box<str>,
    duration_milliseconds: u64,
    states: Box<[RoadEditingSignalPhaseState]>,
    signal_controller: SignalControllerReference,
    canvas_selection: Option<Box<str>>,
}

impl SignalPhaseInput {
    /// 构造信号相位声明并校验键、时长与各信号组状态。
    pub fn try_new(
        signal_phase_key: impl Into<String>,
        duration_milliseconds: u64,
        states: Vec<RoadEditingSignalPhaseState>,
        signal_controller: SignalControllerReference,
    ) -> Result<Self, DiagnosticBundle> {
        let signal_phase_key = signal_phase_key.into();
        validate_token(&signal_phase_key, "signalPhase.signalPhaseKey")?;
        if duration_milliseconds == 0 || duration_milliseconds > MAX_PORTABLE_SIGNAL_TIME_MS {
            return Err(input_error(
                "signalPhase.durationMilliseconds",
                RoadEditingInputViolation::InvalidCombination,
            ));
        }
        require_non_empty(&states, "signalPhase.states")?;
        let groups = states
            .iter()
            .map(RoadEditingSignalPhaseState::signal_group)
            .collect::<Vec<_>>();
        require_unique(&groups, "signalPhase.states.signalGroup")?;
        Ok(Self {
            signal_phase_key: signal_phase_key.into_boxed_str(),
            duration_milliseconds,
            states: states.into_boxed_slice(),
            signal_controller,
            canvas_selection: None,
        })
    }

    /// 返回信号相位稳定键。
    #[must_use]
    pub fn signal_phase_key(&self) -> &str {
        &self.signal_phase_key
    }

    /// 返回相位时长（毫秒）。
    #[must_use]
    pub const fn duration_milliseconds(&self) -> u64 {
        self.duration_milliseconds
    }

    /// 返回相位内各信号组的状态序列。
    #[must_use]
    pub fn states(&self) -> &[RoadEditingSignalPhaseState] {
        &self.states
    }

    /// 返回所属信号控制器引用。
    #[must_use]
    pub const fn signal_controller(&self) -> &SignalControllerReference {
        &self.signal_controller
    }
}
impl_canvas!(SignalPhaseInput);

/// 同时组织显式停车位与可选虚拟容量的停车设施声明。
#[derive(Clone, Debug, PartialEq)]
pub struct ParkingFacilityInput {
    parking_facility_key: Box<str>,
    virtual_capacity: u32,
    virtual_entries: Vec<ParkingLaneAnchor>,
    virtual_exits: Vec<ParkingLaneAnchor>,
    canvas_selection: Option<Box<str>>,
}

impl ParkingFacilityInput {
    /// 构造停车设施声明并校验设施键。
    pub fn try_new(parking_facility_key: impl Into<String>) -> Result<Self, DiagnosticBundle> {
        let parking_facility_key = parking_facility_key.into();
        validate_token(&parking_facility_key, "parkingFacility.parkingFacilityKey")?;
        Ok(Self {
            parking_facility_key: parking_facility_key.into_boxed_str(),
            virtual_capacity: 0,
            virtual_entries: Vec::new(),
            virtual_exits: Vec::new(),
            canvas_selection: None,
        })
    }

    /// 配置不展开成停车位或内部路网的虚拟容量及其可达锚点。
    #[must_use]
    pub fn with_virtual_capacity(
        mut self,
        virtual_capacity: u32,
        virtual_entries: Vec<ParkingLaneAnchor>,
        virtual_exits: Vec<ParkingLaneAnchor>,
    ) -> Self {
        self.virtual_capacity = virtual_capacity;
        self.virtual_entries = virtual_entries;
        self.virtual_exits = virtual_exits;
        self
    }

    /// 返回停车设施稳定键。
    #[must_use]
    pub fn parking_facility_key(&self) -> &str {
        &self.parking_facility_key
    }

    /// 返回虚拟停车容量。
    #[must_use]
    pub const fn virtual_capacity(&self) -> u32 {
        self.virtual_capacity
    }

    /// 返回虚拟容量入口锚点序列。
    #[must_use]
    pub fn virtual_entries(&self) -> &[ParkingLaneAnchor] {
        &self.virtual_entries
    }

    /// 返回虚拟容量出口锚点序列。
    #[must_use]
    pub fn virtual_exits(&self) -> &[ParkingLaneAnchor] {
        &self.virtual_exits
    }
}
impl_canvas!(ParkingFacilityInput);

/// 停车位在车道图边上的入口或出口锚点。
#[derive(Clone, Debug, PartialEq)]
pub struct ParkingLaneAnchor {
    lane_edge: LaneEdgeReference,
    progress_meters: f64,
}

impl ParkingLaneAnchor {
    /// 构造停车锚点；此处只校验进度落在全局闭区间 `[1 mm, MAX_LANE_EDGE_LENGTH_MM - 1 mm]`。
    /// 相对所引车道图边实际长度的边内范围检查在 HIR 绑定阶段进行。
    pub fn try_new(
        lane_edge: LaneEdgeReference,
        progress_meters: f64,
    ) -> Result<Self, DiagnosticBundle> {
        let progress_meters = require_closed_mm(
            progress_meters,
            PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM,
            MAX_LANE_EDGE_LENGTH_MM.saturating_sub(PARKING_ANCHOR_ENDPOINT_CLEARANCE_MM),
            "parkingLaneAnchor.progressMeters",
        )?;
        Ok(Self {
            lane_edge,
            progress_meters,
        })
    }

    /// 返回锚点所在车道图边引用。
    #[must_use]
    pub const fn lane_edge(&self) -> &LaneEdgeReference {
        &self.lane_edge
    }

    /// 返回沿边进度（米）。
    #[must_use]
    pub const fn progress_meters(&self) -> f64 {
        self.progress_meters
    }
}

/// 停车位相对入口边切线的矩形几何。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParkingSpaceGeometry {
    lateral_offset_meters: f64,
    heading_offset_radians: f64,
    length_meters: f64,
    width_meters: f64,
}

impl ParkingSpaceGeometry {
    /// 构造停车位矩形几何并校验横向偏移、朝向与尺寸范围。
    pub fn try_new(
        lateral_offset_meters: f64,
        heading_offset_radians: f64,
        length_meters: f64,
        width_meters: f64,
    ) -> Result<Self, DiagnosticBundle> {
        let lateral_offset_meters = require_closed_mm_i32_abs(
            lateral_offset_meters,
            MIN_PARKING_LATERAL_OFFSET_ABS_MM,
            MAX_PARKING_LATERAL_OFFSET_ABS_MM,
            "parkingSpace.geometry.lateralOffsetMeters",
        )?;
        let heading_offset_radians = require_heading(
            heading_offset_radians,
            "parkingSpace.geometry.headingOffsetRadians",
        )?;
        let length_meters = require_closed_mm(
            length_meters,
            MIN_VEHICLE_LENGTH_MM,
            MAX_VEHICLE_LENGTH_MM,
            "parkingSpace.geometry.lengthMeters",
        )?;
        let width_meters = require_closed_mm(
            width_meters,
            MIN_VEHICLE_LENGTH_MM,
            MAX_VEHICLE_LENGTH_MM,
            "parkingSpace.geometry.widthMeters",
        )?;
        Ok(Self {
            lateral_offset_meters,
            heading_offset_radians,
            length_meters,
            width_meters,
        })
    }

    /// 返回相对入口边切线的横向偏移（米）。
    #[must_use]
    pub const fn lateral_offset_meters(self) -> f64 {
        self.lateral_offset_meters
    }

    /// 返回朝向偏移（弧度）。
    #[must_use]
    pub const fn heading_offset_radians(self) -> f64 {
        self.heading_offset_radians
    }

    /// 返回停车位长度（米）。
    #[must_use]
    pub const fn length_meters(self) -> f64 {
        self.length_meters
    }

    /// 返回停车位宽度（米）。
    #[must_use]
    pub const fn width_meters(self) -> f64 {
        self.width_meters
    }
}

/// 停车位声明。
#[derive(Clone, Debug, PartialEq)]
pub struct ParkingSpaceInput {
    parking_space_key: Box<str>,
    parking_facility: Option<ParkingFacilityReference>,
    entry: ParkingLaneAnchor,
    exit: ParkingLaneAnchor,
    geometry: ParkingSpaceGeometry,
    canvas_selection: Option<Box<str>>,
}

impl ParkingSpaceInput {
    /// 构造停车位声明并校验键。
    pub fn try_new(
        parking_space_key: impl Into<String>,
        entry: ParkingLaneAnchor,
        exit: ParkingLaneAnchor,
        geometry: ParkingSpaceGeometry,
    ) -> Result<Self, DiagnosticBundle> {
        let parking_space_key = parking_space_key.into();
        validate_token(&parking_space_key, "parkingSpace.parkingSpaceKey")?;
        Ok(Self {
            parking_space_key: parking_space_key.into_boxed_str(),
            parking_facility: None,
            entry,
            exit,
            geometry,
            canvas_selection: None,
        })
    }

    /// 设置所属停车设施引用。
    #[must_use]
    pub fn with_parking_facility(mut self, parking_facility: ParkingFacilityReference) -> Self {
        self.parking_facility = Some(parking_facility);
        self
    }

    /// 返回停车位稳定键。
    #[must_use]
    pub fn parking_space_key(&self) -> &str {
        &self.parking_space_key
    }

    /// 返回可选所属停车设施引用。
    #[must_use]
    pub const fn parking_facility(&self) -> Option<&ParkingFacilityReference> {
        self.parking_facility.as_ref()
    }

    /// 返回入口锚点。
    #[must_use]
    pub const fn entry(&self) -> &ParkingLaneAnchor {
        &self.entry
    }

    /// 返回出口锚点。
    #[must_use]
    pub const fn exit(&self) -> &ParkingLaneAnchor {
        &self.exit
    }

    /// 返回停车位几何。
    #[must_use]
    pub const fn geometry(&self) -> ParkingSpaceGeometry {
        self.geometry
    }
}
impl_canvas!(ParkingSpaceInput);

/// 道路区段拥有的车道组声明。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaneGroupInput {
    lane_group_key: Box<str>,
    road_section: RoadSectionReference,
    canvas_selection: Option<Box<str>>,
}

impl LaneGroupInput {
    /// 构造车道组声明并校验组键。
    pub fn try_new(
        lane_group_key: impl Into<String>,
        road_section: RoadSectionReference,
    ) -> Result<Self, DiagnosticBundle> {
        let lane_group_key = lane_group_key.into();
        validate_token(&lane_group_key, "laneGroup.laneGroupKey")?;
        Ok(Self {
            lane_group_key: lane_group_key.into_boxed_str(),
            road_section,
            canvas_selection: None,
        })
    }

    /// 返回车道组稳定键。
    #[must_use]
    pub fn lane_group_key(&self) -> &str {
        &self.lane_group_key
    }

    /// 返回所属道路区段引用。
    #[must_use]
    pub const fn road_section(&self) -> &RoadSectionReference {
        &self.road_section
    }
}
impl_canvas!(LaneGroupInput);

/// 道路走廊拥有的非通行设施带声明。
#[derive(Clone, Debug, PartialEq)]
pub struct FacilityBandInput {
    facility_band_key: Box<str>,
    kind_id: Box<str>,
    width_profile: LinearWidthProfile,
    road_corridor: RoadCorridorReference,
    canvas_selection: Option<Box<str>>,
}

impl FacilityBandInput {
    /// 构造设施带声明并校验键与设施类别。
    pub fn try_new(
        facility_band_key: impl Into<String>,
        kind_id: impl Into<String>,
        width_profile: LinearWidthProfile,
        road_corridor: RoadCorridorReference,
    ) -> Result<Self, DiagnosticBundle> {
        let facility_band_key = facility_band_key.into();
        let kind_id = kind_id.into();
        validate_token(&facility_band_key, "facilityBand.facilityBandKey")?;
        validate_token(&kind_id, "facilityBand.kindId")?;
        validate_facility_kind(
            &kind_id,
            FacilityKindCategory::NonTraversable,
            "facilityBand.kindId",
        )?;
        Ok(Self {
            facility_band_key: facility_band_key.into_boxed_str(),
            kind_id: kind_id.into_boxed_str(),
            width_profile,
            road_corridor,
            canvas_selection: None,
        })
    }

    /// 返回设施带稳定键。
    #[must_use]
    pub fn facility_band_key(&self) -> &str {
        &self.facility_band_key
    }

    /// 返回设施带类别标识。
    #[must_use]
    pub fn kind_id(&self) -> &str {
        &self.kind_id
    }

    /// 返回线性宽度配置。
    #[must_use]
    pub const fn width_profile(&self) -> LinearWidthProfile {
        self.width_profile
    }

    /// 返回所属道路走廊引用。
    #[must_use]
    pub const fn road_corridor(&self) -> &RoadCorridorReference {
        &self.road_corridor
    }
}
impl_canvas!(FacilityBandInput);

/// 静态准入分类法中的参与者类别声明。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParticipantClassInput {
    participant_class_key: Box<str>,
    extends: Option<ParticipantClassReference>,
    canvas_selection: Option<Box<str>>,
}

impl ParticipantClassInput {
    /// 构造参与者类别声明并校验类别键。
    pub fn try_new(participant_class_key: impl Into<String>) -> Result<Self, DiagnosticBundle> {
        let participant_class_key = participant_class_key.into();
        validate_token(
            &participant_class_key,
            "participantClass.participantClassKey",
        )?;
        Ok(Self {
            participant_class_key: participant_class_key.into_boxed_str(),
            extends: None,
            canvas_selection: None,
        })
    }

    /// 设置继承的父参与者类别。
    #[must_use]
    pub fn with_extends(mut self, extends: ParticipantClassReference) -> Self {
        self.extends = Some(extends);
        self
    }

    /// 返回参与者类别稳定键。
    #[must_use]
    pub fn participant_class_key(&self) -> &str {
        &self.participant_class_key
    }

    /// 返回可选继承的父参与者类别引用。
    #[must_use]
    pub const fn extends(&self) -> Option<&ParticipantClassReference> {
        self.extends.as_ref()
    }
}
impl_canvas!(ParticipantClassInput);

/// v1 静态准入规则允许的封闭目标集合。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RoadEditingAccessTarget {
    LaneEdge(LaneEdgeReference),
    LaneGroup(LaneGroupReference),
    RoadSection(RoadSectionReference),
    ManeuverPath(ManeuverPathReference),
}

pub use crate::RegulationIdentity;

/// 静态准入规则声明。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccessRuleInput {
    access_rule_key: Box<str>,
    target: RoadEditingAccessTarget,
    effect: AccessEffect,
    participant_classes: Box<[ParticipantClassReference]>,
    regulation: Option<RegulationIdentity>,
    priority: i32,
    canvas_selection: Option<Box<str>>,
}

impl AccessRuleInput {
    /// 构造准入规则声明；效果只允许允许或拒绝，参与者类别必须非空且不重复。
    pub fn try_new(
        access_rule_key: impl Into<String>,
        target: RoadEditingAccessTarget,
        effect: AccessEffect,
        participant_classes: Vec<ParticipantClassReference>,
        priority: i32,
    ) -> Result<Self, DiagnosticBundle> {
        let access_rule_key = access_rule_key.into();
        validate_token(&access_rule_key, "accessRule.accessRuleKey")?;
        match effect {
            AccessEffect::Allow | AccessEffect::Deny => {}
            _ => {
                return Err(input_error(
                    "accessRule.effect",
                    RoadEditingInputViolation::InvalidCombination,
                ));
            }
        }
        require_non_empty(&participant_classes, "accessRule.participantClasses")?;
        require_unique(&participant_classes, "accessRule.participantClasses")?;
        Ok(Self {
            access_rule_key: access_rule_key.into_boxed_str(),
            target,
            effect,
            participant_classes: participant_classes.into_boxed_slice(),
            regulation: None,
            priority,
            canvas_selection: None,
        })
    }

    /// 设置关联的法规标识。
    #[must_use]
    pub fn with_regulation(mut self, regulation: RegulationIdentity) -> Self {
        self.regulation = Some(regulation);
        self
    }

    /// 返回准入规则稳定键。
    #[must_use]
    pub fn access_rule_key(&self) -> &str {
        &self.access_rule_key
    }

    /// 返回规则作用目标。
    #[must_use]
    pub const fn target(&self) -> &RoadEditingAccessTarget {
        &self.target
    }

    /// 返回规则效果。
    #[must_use]
    pub const fn effect(&self) -> AccessEffect {
        self.effect
    }

    /// 返回规则适用的参与者类别序列。
    #[must_use]
    pub fn participant_classes(&self) -> &[ParticipantClassReference] {
        &self.participant_classes
    }

    /// 返回可选关联法规标识。
    #[must_use]
    pub const fn regulation(&self) -> Option<&RegulationIdentity> {
        self.regulation.as_ref()
    }

    /// 返回规则优先级。
    #[must_use]
    pub const fn priority(&self) -> i32 {
        self.priority
    }
}
impl_canvas!(AccessRuleInput);

/// 当前道路机动车执行域的 IIDM 静态参数。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IidmVehicleProfileInput {
    length_meters: f64,
    desired_speed_meters_per_second: f64,
    min_gap_meters: f64,
    time_headway_seconds: f64,
    max_acceleration_meters_per_second_squared: f64,
    comfortable_deceleration_meters_per_second_squared: f64,
    emergency_deceleration_meters_per_second_squared: f64,
}

impl IidmVehicleProfileInput {
    /// 构造 IIDM 车辆参数并校验各字段范围与减速次序。
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        length_meters: f64,
        desired_speed_meters_per_second: f64,
        min_gap_meters: f64,
        time_headway_seconds: f64,
        max_acceleration_meters_per_second_squared: f64,
        comfortable_deceleration_meters_per_second_squared: f64,
        emergency_deceleration_meters_per_second_squared: f64,
    ) -> Result<Self, DiagnosticBundle> {
        let length_meters = require_closed_mm(
            length_meters,
            MIN_VEHICLE_LENGTH_MM,
            MAX_VEHICLE_LENGTH_MM,
            "vehicleProfile.iidm.lengthMeters",
        )?;
        let desired_speed_meters_per_second = require_closed_mm(
            desired_speed_meters_per_second,
            MIN_SPEED_MM_S,
            MAX_SPEED_MM_S,
            "vehicleProfile.iidm.desiredSpeedMetersPerSecond",
        )?;
        let min_gap_meters = require_closed_mm(
            min_gap_meters,
            0,
            MAX_MIN_GAP_MM,
            "vehicleProfile.iidm.minGapMeters",
        )?;
        let time_headway_seconds = require_time_headway(
            time_headway_seconds,
            "vehicleProfile.iidm.timeHeadwaySeconds",
        )?;
        let max_acceleration_meters_per_second_squared = require_accel(
            max_acceleration_meters_per_second_squared,
            "vehicleProfile.iidm.maxAccelerationMetersPerSecondSquared",
        )?;
        let comfortable_deceleration_meters_per_second_squared = require_accel(
            comfortable_deceleration_meters_per_second_squared,
            "vehicleProfile.iidm.comfortableDecelerationMetersPerSecondSquared",
        )?;
        let emergency_deceleration_meters_per_second_squared = require_accel(
            emergency_deceleration_meters_per_second_squared,
            "vehicleProfile.iidm.emergencyDecelerationMetersPerSecondSquared",
        )?;
        if (emergency_deceleration_meters_per_second_squared as f32)
            < (comfortable_deceleration_meters_per_second_squared as f32)
        {
            return Err(input_error(
                "vehicleProfile.iidm.emergencyDecelerationMetersPerSecondSquared",
                RoadEditingInputViolation::InvalidCombination,
            ));
        }
        Ok(Self {
            length_meters,
            desired_speed_meters_per_second,
            min_gap_meters,
            time_headway_seconds,
            max_acceleration_meters_per_second_squared,
            comfortable_deceleration_meters_per_second_squared,
            emergency_deceleration_meters_per_second_squared,
        })
    }

    /// 返回车长（米）。
    #[must_use]
    pub const fn length_meters(self) -> f64 {
        self.length_meters
    }
    /// 返回期望速度（米每秒）。
    #[must_use]
    pub const fn desired_speed_meters_per_second(self) -> f64 {
        self.desired_speed_meters_per_second
    }
    /// 返回最小间距（米）。
    #[must_use]
    pub const fn min_gap_meters(self) -> f64 {
        self.min_gap_meters
    }
    /// 返回车头时距（秒）。
    #[must_use]
    pub const fn time_headway_seconds(self) -> f64 {
        self.time_headway_seconds
    }
    /// 返回最大加速度（米每二次方秒）。
    #[must_use]
    pub const fn max_acceleration_meters_per_second_squared(self) -> f64 {
        self.max_acceleration_meters_per_second_squared
    }
    /// 返回舒适减速度（米每二次方秒）。
    #[must_use]
    pub const fn comfortable_deceleration_meters_per_second_squared(self) -> f64 {
        self.comfortable_deceleration_meters_per_second_squared
    }
    /// 返回紧急减速度（米每二次方秒）。
    #[must_use]
    pub const fn emergency_deceleration_meters_per_second_squared(self) -> f64 {
        self.emergency_deceleration_meters_per_second_squared
    }
}

/// 当前道路机动车车辆配置声明。
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleProfileInput {
    vehicle_profile_key: Box<str>,
    participant_class: ParticipantClassReference,
    iidm: IidmVehicleProfileInput,
    canvas_selection: Option<Box<str>>,
}

impl VehicleProfileInput {
    /// 构造车辆配置声明并校验配置键。
    pub fn try_new(
        vehicle_profile_key: impl Into<String>,
        participant_class: ParticipantClassReference,
        iidm: IidmVehicleProfileInput,
    ) -> Result<Self, DiagnosticBundle> {
        let vehicle_profile_key = vehicle_profile_key.into();
        validate_token(&vehicle_profile_key, "vehicleProfile.vehicleProfileKey")?;
        Ok(Self {
            vehicle_profile_key: vehicle_profile_key.into_boxed_str(),
            participant_class,
            iidm,
            canvas_selection: None,
        })
    }

    /// 返回车辆配置稳定键。
    #[must_use]
    pub fn vehicle_profile_key(&self) -> &str {
        &self.vehicle_profile_key
    }
    /// 返回绑定的参与者类别引用。
    #[must_use]
    pub const fn participant_class(&self) -> &ParticipantClassReference {
        &self.participant_class
    }
    /// 返回 IIDM 静态参数。
    #[must_use]
    pub const fn iidm(&self) -> IidmVehicleProfileInput {
        self.iidm
    }
}
impl_canvas!(VehicleProfileInput);

/// 与路口绑定的稳定冲突区声明。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConflictZoneInput {
    conflict_zone_key: Box<str>,
    junction: JunctionReference,
    canvas_selection: Option<Box<str>>,
}

impl ConflictZoneInput {
    /// 构造冲突区声明并校验键。
    pub fn try_new(
        conflict_zone_key: impl Into<String>,
        junction: JunctionReference,
    ) -> Result<Self, DiagnosticBundle> {
        let conflict_zone_key = conflict_zone_key.into();
        validate_token(&conflict_zone_key, "conflictZone.conflictZoneKey")?;
        Ok(Self {
            conflict_zone_key: conflict_zone_key.into_boxed_str(),
            junction,
            canvas_selection: None,
        })
    }

    /// 返回冲突区稳定键。
    #[must_use]
    pub fn conflict_zone_key(&self) -> &str {
        &self.conflict_zone_key
    }

    /// 返回绑定的路口引用。
    #[must_use]
    pub const fn junction(&self) -> &JunctionReference {
        &self.junction
    }
}
impl_canvas!(ConflictZoneInput);

/// `ParticipantStream` 路径上的闭合位置 variant。
#[derive(Clone, Debug, PartialEq)]
pub enum PathAnchorInput {
    Gate(ManeuverGateReference),
    EdgeBoundary {
        boundary_index: u32,
    },
    Interior {
        path_edge_index: u32,
        progress_meters: f64,
    },
}

impl PathAnchorInput {
    /// 构造位于机动门的路径锚点。
    #[must_use]
    pub fn gate(gate: ManeuverGateReference) -> Self {
        Self::Gate(gate)
    }

    /// 构造位于路径边边界的路径锚点。
    #[must_use]
    pub const fn edge_boundary(boundary_index: u32) -> Self {
        Self::EdgeBoundary { boundary_index }
    }

    /// 构造位于路径边内部进度处的锚点；此处只校验进度落在全局闭区间
    /// `[1 mm, MAX_LANE_EDGE_LENGTH_MM - 1 mm]`，相对边实际长度的检查在 HIR 绑定阶段进行。
    pub fn interior(path_edge_index: u32, progress_meters: f64) -> Result<Self, DiagnosticBundle> {
        let progress_meters = require_closed_mm(
            progress_meters,
            1,
            MAX_LANE_EDGE_LENGTH_MM.saturating_sub(1),
            "pathAnchor.progressMeters",
        )?;
        Ok(Self::Interior {
            path_edge_index,
            progress_meters,
        })
    }
}

/// 一个参与者流穿越一个冲突区的 owner-local 行。
#[derive(Clone, Debug, PartialEq)]
pub struct ConflictPassageInput {
    conflict_zone: ConflictZoneReference,
    entry: PathAnchorInput,
    exit: PathAnchorInput,
}

impl ConflictPassageInput {
    /// 构造一条冲突区通行记录。
    #[must_use]
    pub const fn new(
        conflict_zone: ConflictZoneReference,
        entry: PathAnchorInput,
        exit: PathAnchorInput,
    ) -> Self {
        Self {
            conflict_zone,
            entry,
            exit,
        }
    }

    /// 返回穿越的冲突区引用。
    #[must_use]
    pub const fn conflict_zone(&self) -> &ConflictZoneReference {
        &self.conflict_zone
    }

    /// 返回进入锚点。
    #[must_use]
    pub const fn entry(&self) -> &PathAnchorInput {
        &self.entry
    }

    /// 返回离开锚点。
    #[must_use]
    pub const fn exit(&self) -> &PathAnchorInput {
        &self.exit
    }
}

/// 与路口和唯一机动路径绑定的稳定参与者流声明。
#[derive(Clone, Debug, PartialEq)]
pub struct ParticipantStreamInput {
    participant_stream_key: Box<str>,
    junction: JunctionReference,
    maneuver_path: ManeuverPathReference,
    passages: Box<[ConflictPassageInput]>,
    canvas_selection: Option<Box<str>>,
}

impl ParticipantStreamInput {
    /// 构造参与者流声明并校验键与通行序列。
    pub fn try_new(
        participant_stream_key: impl Into<String>,
        junction: JunctionReference,
        maneuver_path: ManeuverPathReference,
        passages: Vec<ConflictPassageInput>,
    ) -> Result<Self, DiagnosticBundle> {
        let participant_stream_key = participant_stream_key.into();
        validate_token(
            &participant_stream_key,
            "participantStream.participantStreamKey",
        )?;
        require_non_empty(&passages, "participantStream.passages")?;
        Ok(Self {
            participant_stream_key: participant_stream_key.into_boxed_str(),
            junction,
            maneuver_path,
            passages: passages.into_boxed_slice(),
            canvas_selection: None,
        })
    }

    /// 返回参与者流稳定键。
    #[must_use]
    pub fn participant_stream_key(&self) -> &str {
        &self.participant_stream_key
    }

    /// 返回绑定的路口引用。
    #[must_use]
    pub const fn junction(&self) -> &JunctionReference {
        &self.junction
    }

    /// 返回绑定的机动路径引用。
    #[must_use]
    pub const fn maneuver_path(&self) -> &ManeuverPathReference {
        &self.maneuver_path
    }

    /// 返回穿越各冲突区的通行序列。
    #[must_use]
    pub const fn passages(&self) -> &[ConflictPassageInput] {
        &self.passages
    }
}
impl_canvas!(ParticipantStreamInput);

/// 一个 ConflictZone 在规范坐标框架中的可选 owner-local 空间区域。
#[derive(Clone, Debug, PartialEq)]
pub struct ConflictZoneRegionInput {
    conflict_zone: ConflictZoneReference,
    canonical_frame: CanonicalFrameReference,
    min_y: f64,
    max_y: f64,
    ring_xz: Box<[RoadEditingPoint2]>,
    canvas_selection: Option<Box<str>>,
}

impl ConflictZoneRegionInput {
    /// 构造冲突区空间区域并校验高度范围与环点数量。
    pub fn try_new(
        conflict_zone: ConflictZoneReference,
        canonical_frame: CanonicalFrameReference,
        min_y: f64,
        max_y: f64,
        ring_xz: Vec<RoadEditingPoint2>,
    ) -> Result<Self, DiagnosticBundle> {
        let minimum = f64::from(CANONICAL_POINT_COMPONENT_MIN_METERS);
        let maximum = f64::from(CANONICAL_POINT_COMPONENT_MAX_METERS);
        let min_y = validate_inclusive_range(min_y, minimum, maximum, "conflictZoneRegion.minY")?;
        let max_y = validate_inclusive_range(max_y, minimum, maximum, "conflictZoneRegion.maxY")?;
        if min_y >= max_y || ring_xz.len() < 3 {
            return Err(input_error(
                "conflictZoneRegion",
                RoadEditingInputViolation::InvalidCombination,
            ));
        }
        let point_count = u64::try_from(ring_xz.len()).unwrap_or(u64::MAX);
        if point_count > u64::from(MAX_CONFLICT_ZONE_REGION_RING_POINTS) {
            return Err(input_error(
                "conflictZoneRegion.ringXZ",
                RoadEditingInputViolation::CollectionTooLarge {
                    maximum: u64::from(MAX_CONFLICT_ZONE_REGION_RING_POINTS),
                    actual: point_count,
                },
            ));
        }
        Ok(Self {
            conflict_zone,
            canonical_frame,
            min_y,
            max_y,
            ring_xz: ring_xz.into_boxed_slice(),
            canvas_selection: None,
        })
    }

    /// 返回所属冲突区引用。
    #[must_use]
    pub const fn conflict_zone(&self) -> &ConflictZoneReference {
        &self.conflict_zone
    }

    /// 返回区域所在的规范坐标框架引用。
    #[must_use]
    pub const fn canonical_frame(&self) -> &CanonicalFrameReference {
        &self.canonical_frame
    }

    /// 返回区域最小高度（米）。
    #[must_use]
    pub const fn min_y(&self) -> f64 {
        self.min_y
    }

    /// 返回区域最大高度（米）。
    #[must_use]
    pub const fn max_y(&self) -> f64 {
        self.max_y
    }

    /// 返回 XZ 平面环点序列。
    #[must_use]
    pub const fn ring_xz(&self) -> &[RoadEditingPoint2] {
        &self.ring_xz
    }
}
impl_canvas!(ConflictZoneRegionInput);

/// 固定单位、手性与范围的规范坐标框架声明。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalFrameInput {
    canonical_frame_key: Box<str>,
    canvas_selection: Option<Box<str>>,
}

impl CanonicalFrameInput {
    /// 构造规范坐标框架声明并校验框架键。
    pub fn try_new(canonical_frame_key: impl Into<String>) -> Result<Self, DiagnosticBundle> {
        let canonical_frame_key = canonical_frame_key.into();
        validate_token(&canonical_frame_key, "canonicalFrame.canonicalFrameKey")?;
        Ok(Self {
            canonical_frame_key: canonical_frame_key.into_boxed_str(),
            canvas_selection: None,
        })
    }

    /// 返回规范坐标框架稳定键。
    #[must_use]
    pub fn canonical_frame_key(&self) -> &str {
        &self.canonical_frame_key
    }
}
impl_canvas!(CanonicalFrameInput);

mod policy;
pub use policy::*;

/// Road Editing Source 的 24 个可构造声明种类。
#[derive(Clone, Debug, PartialEq)]
pub enum RoadEditingDeclaration {
    RightOfWayPolicySet(RightOfWayPolicySetInput),
    RoadCorridor(RoadCorridorInput),
    RoadSection(RoadSectionInput),
    AuthoringLane(AuthoringLaneInput),
    LaneEdge(LaneEdgeInput),
    Junction(JunctionInput),
    Movement(MovementInput),
    ManeuverPath(ManeuverPathInput),
    ManeuverGate(ManeuverGateInput),
    WaitingZone(WaitingZoneInput),
    StopLine(StopLineInput),
    SignalGroup(SignalGroupInput),
    SignalController(SignalControllerInput),
    SignalPhase(SignalPhaseInput),
    ParkingFacility(ParkingFacilityInput),
    ParkingSpace(ParkingSpaceInput),
    LaneGroup(LaneGroupInput),
    FacilityBand(FacilityBandInput),
    ParticipantClass(ParticipantClassInput),
    AccessRule(AccessRuleInput),
    VehicleProfile(VehicleProfileInput),
    CanonicalFrame(CanonicalFrameInput),
    ConflictZone(ConflictZoneInput),
    ParticipantStream(ParticipantStreamInput),
}

impl RoadEditingDeclaration {
    /// 返回与 Identity v1 registry 一致的声明类别。
    #[must_use]
    pub const fn entity_kind(&self) -> EntityKind {
        match self {
            Self::RightOfWayPolicySet(_) => EntityKind::RightOfWayPolicySet,
            Self::RoadCorridor(_) => EntityKind::RoadCorridor,
            Self::RoadSection(_) => EntityKind::RoadSection,
            Self::AuthoringLane(_) => EntityKind::AuthoringLane,
            Self::LaneEdge(_) => EntityKind::LaneEdge,
            Self::Junction(_) => EntityKind::Junction,
            Self::Movement(_) => EntityKind::Movement,
            Self::ManeuverPath(_) => EntityKind::ManeuverPath,
            Self::ManeuverGate(_) => EntityKind::ManeuverGate,
            Self::WaitingZone(_) => EntityKind::WaitingZone,
            Self::StopLine(_) => EntityKind::StopLine,
            Self::SignalGroup(_) => EntityKind::SignalGroup,
            Self::SignalController(_) => EntityKind::SignalController,
            Self::SignalPhase(_) => EntityKind::SignalPhase,
            Self::ParkingFacility(_) => EntityKind::ParkingFacility,
            Self::ParkingSpace(_) => EntityKind::ParkingSpace,
            Self::LaneGroup(_) => EntityKind::LaneGroup,
            Self::FacilityBand(_) => EntityKind::FacilityBand,
            Self::ParticipantClass(_) => EntityKind::ParticipantClass,
            Self::AccessRule(_) => EntityKind::AccessRule,
            Self::VehicleProfile(_) => EntityKind::VehicleProfile,
            Self::CanonicalFrame(_) => EntityKind::CanonicalFrame,
            Self::ConflictZone(_) => EntityKind::ConflictZone,
            Self::ParticipantStream(_) => EntityKind::ParticipantStream,
        }
    }

    /// 返回声明在其直接 owner 下的稳定 local key。
    #[must_use]
    pub fn local_key(&self) -> &str {
        match self {
            Self::RightOfWayPolicySet(value) => value.policy_set_key(),
            Self::RoadCorridor(value) => value.road_corridor_key(),
            Self::RoadSection(value) => value.road_section_key(),
            Self::AuthoringLane(value) => value.authoring_lane_key(),
            Self::LaneEdge(value) => value.lane_edge_key(),
            Self::Junction(value) => value.junction_key(),
            Self::Movement(value) => value.movement_key(),
            Self::ManeuverPath(value) => value.maneuver_path_key(),
            Self::ManeuverGate(value) => value.maneuver_gate_key(),
            Self::WaitingZone(value) => value.waiting_zone_key(),
            Self::StopLine(value) => value.stop_line_key(),
            Self::SignalGroup(value) => value.signal_group_key(),
            Self::SignalController(value) => value.signal_controller_key(),
            Self::SignalPhase(value) => value.signal_phase_key(),
            Self::ParkingFacility(value) => value.parking_facility_key(),
            Self::ParkingSpace(value) => value.parking_space_key(),
            Self::LaneGroup(value) => value.lane_group_key(),
            Self::FacilityBand(value) => value.facility_band_key(),
            Self::ParticipantClass(value) => value.participant_class_key(),
            Self::AccessRule(value) => value.access_rule_key(),
            Self::VehicleProfile(value) => value.vehicle_profile_key(),
            Self::CanonicalFrame(value) => value.canonical_frame_key(),
            Self::ConflictZone(value) => value.conflict_zone_key(),
            Self::ParticipantStream(value) => value.participant_stream_key(),
        }
    }

    /// 返回声明从模块根 owner 到直接 parent 的 key 链；module-scoped 声明返回空链。
    pub(super) fn owner_key_components(&self) -> Box<[&str]> {
        match self {
            Self::RoadSection(value) => value.road_corridor().components().collect(),
            Self::AuthoringLane(value) => value.road_section().components().collect(),
            Self::Movement(value) => value.junction().components().collect(),
            Self::ManeuverPath(value) => value.movement().components().collect(),
            Self::ManeuverGate(value) => value.maneuver_path().components().collect(),
            Self::WaitingZone(value) => value.maneuver_path().components().collect(),
            Self::SignalPhase(value) => value.signal_controller().components().collect(),
            Self::LaneGroup(value) => value.road_section().components().collect(),
            Self::FacilityBand(value) => value.road_corridor().components().collect(),
            Self::ConflictZone(value) => value.junction().components().collect(),
            Self::ParticipantStream(value) => value.junction().components().collect(),
            Self::RightOfWayPolicySet(_)
            | Self::RoadCorridor(_)
            | Self::LaneEdge(_)
            | Self::Junction(_)
            | Self::StopLine(_)
            | Self::SignalGroup(_)
            | Self::SignalController(_)
            | Self::ParkingFacility(_)
            | Self::ParkingSpace(_)
            | Self::ParticipantClass(_)
            | Self::AccessRule(_)
            | Self::VehicleProfile(_)
            | Self::CanonicalFrame(_) => Box::new([]),
        }
    }

    /// 按实体类别、owner key 链与 local key 的规范地址顺序比较两个声明。
    pub(super) fn canonical_address_cmp(&self, other: &Self) -> Ordering {
        self.entity_kind()
            .cmp(&other.entity_kind())
            .then_with(|| {
                (0..usize::from(owner_depth(self.entity_kind())))
                    .map(|index| self.owner_key_at(index).expect("closed owner depth"))
                    .cmp(
                        (0..usize::from(owner_depth(other.entity_kind())))
                            .map(|index| other.owner_key_at(index).expect("closed owner depth")),
                    )
            })
            .then_with(|| {
                self.local_key()
                    .as_bytes()
                    .cmp(other.local_key().as_bytes())
            })
    }

    fn owner_key_at(&self, index: usize) -> Option<&str> {
        match self {
            Self::RoadSection(value) => value.road_corridor().components().nth(index),
            Self::AuthoringLane(value) => value.road_section().components().nth(index),
            Self::Movement(value) => value.junction().components().nth(index),
            Self::ManeuverPath(value) => value.movement().components().nth(index),
            Self::ManeuverGate(value) => value.maneuver_path().components().nth(index),
            Self::WaitingZone(value) => value.maneuver_path().components().nth(index),
            Self::SignalPhase(value) => value.signal_controller().components().nth(index),
            Self::LaneGroup(value) => value.road_section().components().nth(index),
            Self::FacilityBand(value) => value.road_corridor().components().nth(index),
            Self::ConflictZone(value) => value.junction().components().nth(index),
            Self::ParticipantStream(value) => value.junction().components().nth(index),
            _ => None,
        }
    }
}

fn require_closed_mm(
    value: f64,
    min_mm: u32,
    max_mm: u32,
    field: &'static str,
) -> Result<f64, DiagnosticBundle> {
    if let Some(violation) = millimetre_range_violation(value, min_mm, max_mm) {
        return Err(input_error(field, violation));
    }
    Ok(value)
}

fn require_closed_mm_i32_abs(
    value: f64,
    min_abs_mm: u32,
    max_abs_mm: u32,
    field: &'static str,
) -> Result<f64, DiagnosticBundle> {
    if let Some(violation) = millimetre_i32_abs_range_violation(value, min_abs_mm, max_abs_mm) {
        return Err(input_error(field, violation));
    }
    Ok(value)
}

fn require_heading(value: f64, field: &'static str) -> Result<f64, DiagnosticBundle> {
    if let Some(violation) = heading_violation(value) {
        return Err(input_error(field, violation));
    }
    Ok(value)
}

fn require_time_headway(value: f64, field: &'static str) -> Result<f64, DiagnosticBundle> {
    if let Some(violation) = time_headway_violation(value) {
        return Err(input_error(field, violation));
    }
    Ok(value)
}

fn require_accel(value: f64, field: &'static str) -> Result<f64, DiagnosticBundle> {
    if let Some(violation) = accel_violation(value) {
        return Err(input_error(field, violation));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_scoped_reference_requires_exact_identity_depth() {
        assert!(RoadSectionReference::local("section-a").is_err());
        assert!(RoadSectionReference::owner_scoped(vec!["corridor-a".into()], "section-a").is_ok());
        assert!(ConflictZoneReference::local("zone-a").is_err());
        assert!(ConflictZoneReference::owner_scoped(vec!["junction-a".into()], "zone-a").is_ok());
        assert!(ParticipantStreamReference::local("stream-a").is_err());
        assert!(
            ParticipantStreamReference::owner_scoped(vec!["junction-a".into()], "stream-a",)
                .is_ok()
        );
        assert!(
            AuthoringLaneReference::owner_scoped(
                vec!["corridor-a".into(), "section-a".into()],
                "lane-a",
            )
            .is_ok()
        );
        assert!(
            ManeuverGateReference::owner_scoped(
                vec!["junction-a".into(), "movement-a".into(), "path-a".into()],
                "gate-a",
            )
            .is_ok()
        );
    }

    #[test]
    fn sibling_local_key_can_repeat_under_different_owners() {
        let first = SignalPhaseReference::owner_scoped(vec!["controller-a".into()], "green")
            .expect("first phase reference");
        let second = SignalPhaseReference::owner_scoped(vec!["controller-b".into()], "green")
            .expect("second phase reference");

        assert_eq!(first.local_key(), second.local_key());
        assert_ne!(first, second);
    }

    #[test]
    fn direct_provenance_uses_frozen_build_and_digest_values() {
        let provenance = RoadEditingProvenance::direct("editor save").expect("direct provenance");

        assert_eq!(provenance.kind(), RoadEditingProvenanceKind::Direct);
        assert_eq!(provenance.generator_build_id(), DIRECT_GENERATOR_BUILD_ID);
        assert_eq!(
            provenance.parameters_and_inputs_digest(),
            &DIRECT_INPUTS_DIGEST
        );
        assert_eq!(
            provenance.frontend_options_digest(),
            &DIRECT_FRONTEND_OPTIONS_DIGEST
        );
        assert_eq!(provenance.random_seed(), None);
    }

    #[test]
    fn scalar_constructors_reject_invalid_values_and_canonicalize_zero() {
        assert!(RoadEditingPoint3::try_new(f64::NAN, 0.0, 0.0).is_err());
        assert!(
            RoadEditingPoint3::try_new(
                f64::from(CANONICAL_POINT_COMPONENT_MAX_METERS) + 0.25,
                0.0,
                0.0,
            )
            .is_err()
        );
        assert!(LinearWidthProfile::try_new(0.0, 0.0).is_err());
        assert!(LaneEdgeInput::try_new("edge", 0.0, Vec::new(), None).is_err());

        let point = RoadEditingPoint3::try_new(-0.0, -0.0, -0.0).expect("canonical point");
        assert_eq!(point.x().to_bits(), 0.0_f64.to_bits());
        assert_eq!(point.y().to_bits(), 0.0_f64.to_bits());
        assert_eq!(point.z().to_bits(), 0.0_f64.to_bits());
    }

    #[test]
    fn cross_section_inputs_enforce_facility_kind_categories() {
        let corridor = RoadCorridorReference::local("corridor-a").expect("corridor");
        let lane = AuthoringLaneReference::owner_scoped(
            vec!["corridor-a".into(), "section-a".into()],
            "lane-a",
        )
        .expect("lane");
        let width = LinearWidthProfile::try_new(1.0, 1.0).expect("width");

        assert!(
            RoadSectionInput::try_new(
                "section-a",
                "motorLane",
                vec![lane.clone()],
                corridor.clone(),
            )
            .is_ok()
        );
        assert!(
            RoadSectionInput::try_new("section-b", "x-lane-tram", vec![lane], corridor.clone(),)
                .is_ok()
        );
        assert!(
            RoadSectionInput::try_new(
                "section-c",
                "sidewalk",
                vec![
                    AuthoringLaneReference::owner_scoped(
                        vec!["corridor-a".into(), "section-c".into()],
                        "lane-c",
                    )
                    .expect("lane c")
                ],
                corridor.clone(),
            )
            .is_err()
        );

        assert!(FacilityBandInput::try_new("band-a", "sidewalk", width, corridor.clone(),).is_ok());
        assert!(
            FacilityBandInput::try_new("band-b", "x-snow-storage", width, corridor.clone(),)
                .is_ok()
        );
        assert!(
            FacilityBandInput::try_new("band-c", "motorLane", width, corridor.clone()).is_err()
        );
        assert!(FacilityBandInput::try_new("band-d", "sidewak", width, corridor).is_err());
    }

    #[test]
    fn corridor_reference_section_and_lane_must_share_the_corridor_owner() {
        let section = RoadSectionReference::owner_scoped(vec!["corridor-a".into()], "section-a")
            .expect("section");
        let lane = AuthoringLaneReference::owner_scoped(
            vec!["corridor-a".into(), "section-a".into()],
            "lane-a",
        )
        .expect("lane");
        let alignment = || RoadAlignmentReference::try_new("alignment").expect("alignment");
        let elements = || vec![RoadEditingCorridorElement::RoadSection(section.clone())];

        RoadCorridorInput::try_new(
            "corridor-a",
            alignment(),
            0.0,
            RoadEditingStationEnd::AlignmentEnd,
            section.clone(),
            lane.clone(),
            elements(),
        )
        .expect("matching corridor owner");

        let other_section =
            RoadSectionReference::owner_scoped(vec!["corridor-b".into()], "section-a")
                .expect("other section");
        assert!(
            RoadCorridorInput::try_new(
                "corridor-a",
                alignment(),
                0.0,
                RoadEditingStationEnd::AlignmentEnd,
                other_section,
                lane.clone(),
                elements(),
            )
            .is_err()
        );

        let imported_lane = AuthoringLaneReference::imported(
            "base",
            vec!["corridor-a".into(), "section-a".into()],
            "lane-a",
        )
        .expect("imported lane");
        assert!(
            RoadCorridorInput::try_new(
                "corridor-a",
                alignment(),
                0.0,
                RoadEditingStationEnd::AlignmentEnd,
                section.clone(),
                imported_lane,
                elements(),
            )
            .is_err()
        );
    }

    #[test]
    fn collection_and_canvas_rules_fail_closed() {
        let group = SignalGroupReference::local("group-a").expect("group reference");
        assert!(
            SignalControllerInput::try_new(
                "controller-a",
                0,
                vec![group.clone(), group],
                vec![
                    SignalPhaseReference::owner_scoped(vec!["controller-a".into()], "phase-a",)
                        .expect("phase reference")
                ],
            )
            .is_err()
        );
        assert!(
            CanonicalFrameInput::try_new("frame-a")
                .expect("frame")
                .with_canvas_selection("canvas::reserved")
                .is_err()
        );
    }

    #[test]
    fn signal_time_fields_use_the_portable_integer_range() {
        let group = || SignalGroupReference::local("group-a").expect("group reference");
        let phase = || {
            SignalPhaseReference::owner_scoped(vec!["controller-a".into()], "phase-a")
                .expect("phase reference")
        };
        let state = || {
            RoadEditingSignalPhaseState::try_new(group(), SignalAspect::Green).expect("phase state")
        };
        let controller = |offset| {
            SignalControllerInput::try_new("controller-a", offset, vec![group()], vec![phase()])
        };
        let signal_phase = |duration| {
            SignalPhaseInput::try_new(
                "phase-a",
                duration,
                vec![state()],
                SignalControllerReference::local("controller-a").expect("controller reference"),
            )
        };

        assert!(controller(MAX_PORTABLE_SIGNAL_TIME_MS).is_ok());
        assert!(controller(MAX_PORTABLE_SIGNAL_TIME_MS + 1).is_err());
        assert!(signal_phase(MAX_PORTABLE_SIGNAL_TIME_MS).is_ok());
        assert!(signal_phase(MAX_PORTABLE_SIGNAL_TIME_MS + 1).is_err());
    }

    #[test]
    fn junction_allows_a_direct_path_without_internal_edges() {
        let approaches = vec![
            LaneEdgeReference::local("entry").expect("entry"),
            LaneEdgeReference::local("exit").expect("exit"),
        ];

        let junction = JunctionInput::try_new("junction", approaches, Vec::new())
            .expect("direct path junction");

        assert!(junction.internal_edges().is_empty());
    }

    #[test]
    fn declaration_address_retains_full_owner_tuple() {
        let section = RoadSectionInput::try_new(
            "section-a",
            "motorLane",
            vec![
                AuthoringLaneReference::owner_scoped(
                    vec!["corridor-a".into(), "section-a".into()],
                    "lane-a",
                )
                .expect("lane reference"),
            ],
            RoadCorridorReference::local("corridor-a").expect("corridor reference"),
        )
        .expect("section");
        let declaration = RoadEditingDeclaration::RoadSection(section);

        assert_eq!(declaration.entity_kind(), EntityKind::RoadSection);
        assert_eq!(declaration.local_key(), "section-a");
        assert_eq!(&*declaration.owner_key_components(), &["corridor-a"]);
    }

    #[test]
    fn access_regulation_provenance_accepts_bounded_unicode_text() {
        let regulation = RegulationIdentity::try_new("中国", "二〇二六")
            .expect("unicode regulation")
            .with_source("法规库")
            .expect("unicode source");

        assert_eq!(regulation.jurisdiction(), "中国");
        assert_eq!(regulation.version(), "二〇二六");
        assert_eq!(regulation.source(), Some("法规库"));
    }

    #[test]
    fn conflict_region_rejects_point_ceiling_plus_one_at_first_party_input() {
        let points = vec![
            RoadEditingPoint2::try_new(0.0, 0.0).unwrap();
            MAX_CONFLICT_ZONE_REGION_RING_POINTS as usize + 1
        ];
        let error = ConflictZoneRegionInput::try_new(
            ConflictZoneReference::owner_scoped(vec!["junction".into()], "zone").unwrap(),
            CanonicalFrameReference::local("frame").unwrap(),
            -1.0,
            1.0,
            points,
        )
        .unwrap_err();

        assert!(matches!(
            error.diagnostics()[0].payload(),
            crate::DiagnosticPayload::InvalidRoadEditingInput {
                field,
                violation: RoadEditingInputViolation::CollectionTooLarge {
                    maximum,
                    actual,
                },
            } if field.as_ref() == "conflictZoneRegion.ringXZ"
                && *maximum == u64::from(MAX_CONFLICT_ZONE_REGION_RING_POINTS)
                && *actual == u64::from(MAX_CONFLICT_ZONE_REGION_RING_POINTS) + 1
        ));
    }
}

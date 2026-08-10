use std::cmp::Ordering;
use std::marker::PhantomData;

use laneflow_static_contract::{
    AccessEffect, AccessRuleKind, AuthoringLaneKind, CanonicalFrameKind, EntityKind,
    EntityKindMarker, FacilityBandKind, JunctionKind, LaneEdgeKind, LaneGroupKind,
    MIN_PARKING_EXTENT_EXCLUSIVE_METERS, MIN_PARKING_LATERAL_OFFSET_ABS_EXCLUSIVE_METERS,
    MIN_VEHICLE_LENGTH_EXCLUSIVE_METERS, ManeuverGateKind, ManeuverPathKind, MovementKind,
    PARKING_ANCHOR_ENDPOINT_CLEARANCE_METERS, PARKING_HEADING_OFFSET_MAXIMUM_RADIANS,
    PARKING_HEADING_OFFSET_MINIMUM_RADIANS, ParkingAreaKind, ParkingSpaceKind,
    ParticipantClassKind, RoadCorridorKind, RoadSectionKind, SignalAspect, SignalControllerKind,
    SignalGroupKind, SignalPhaseKind, StaticRouteKind, StopLineKind, VehicleProfileKind,
    WaitingZoneKind,
};

use super::rules::{
    input_error, require_non_empty, require_unique, validate_finite, validate_non_negative,
    validate_positive, validate_token, validate_visible_ascii,
};
use crate::{DiagnosticBundle, RoadEditingInputViolation};

pub(super) const DIRECT_GENERATOR_BUILD_ID: &str = "laneflow-road-editing-direct-v1";
pub(super) const DIRECT_INPUTS_DIGEST: [u8; 32] = [
    0x6b, 0x27, 0xd0, 0xf7, 0x66, 0x93, 0xbc, 0xd3, 0x86, 0xac, 0x13, 0xdf, 0x72, 0x4e, 0x30, 0xf5,
    0xfb, 0x5a, 0xd3, 0xb9, 0xa1, 0x52, 0xa5, 0xe1, 0xf8, 0x8d, 0xe1, 0xa6, 0x24, 0xce, 0xa8, 0xaa,
];
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

    pub(super) fn components(&self) -> impl Iterator<Item = &str> {
        self.owner_keys().chain(std::iter::once(self.local_key()))
    }

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
        | EntityKind::ParkingArea
        | EntityKind::ParkingSpace
        | EntityKind::ParticipantClass
        | EntityKind::AccessRule
        | EntityKind::VehicleProfile
        | EntityKind::StaticRoute
        | EntityKind::CanonicalFrame => 0,
    }
}

pub type RoadCorridorReference = RoadEditingReference<RoadCorridorKind>;
pub type RoadSectionReference = RoadEditingReference<RoadSectionKind>;
pub type AuthoringLaneReference = RoadEditingReference<AuthoringLaneKind>;
pub type LaneEdgeReference = RoadEditingReference<LaneEdgeKind>;
pub type JunctionReference = RoadEditingReference<JunctionKind>;
pub type MovementReference = RoadEditingReference<MovementKind>;
pub type ManeuverPathReference = RoadEditingReference<ManeuverPathKind>;
pub type ManeuverGateReference = RoadEditingReference<ManeuverGateKind>;
pub type WaitingZoneReference = RoadEditingReference<WaitingZoneKind>;
pub type StopLineReference = RoadEditingReference<StopLineKind>;
pub type SignalGroupReference = RoadEditingReference<SignalGroupKind>;
pub type SignalControllerReference = RoadEditingReference<SignalControllerKind>;
pub type SignalPhaseReference = RoadEditingReference<SignalPhaseKind>;
pub type ParkingAreaReference = RoadEditingReference<ParkingAreaKind>;
pub type ParkingSpaceReference = RoadEditingReference<ParkingSpaceKind>;
pub type LaneGroupReference = RoadEditingReference<LaneGroupKind>;
pub type FacilityBandReference = RoadEditingReference<FacilityBandKind>;
pub type ParticipantClassReference = RoadEditingReference<ParticipantClassKind>;
pub type AccessRuleReference = RoadEditingReference<AccessRuleKind>;
pub type VehicleProfileReference = RoadEditingReference<VehicleProfileKind>;
pub type StaticRouteReference = RoadEditingReference<StaticRouteKind>;
pub type CanonicalFrameReference = RoadEditingReference<CanonicalFrameKind>;

/// 当前模块内、不进入 Identity v1 的道路走向键引用。
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RoadAlignmentReference(Box<str>);

impl RoadAlignmentReference {
    pub fn try_new(key: impl Into<String>) -> Result<Self, DiagnosticBundle> {
        let key = key.into();
        validate_token(&key, "roadAlignmentReference")?;
        Ok(Self(key.into_boxed_str()))
    }

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

    #[must_use]
    pub const fn kind(&self) -> RoadEditingProvenanceKind {
        self.kind
    }

    #[must_use]
    pub fn generator_build_id(&self) -> &str {
        &self.generator_build_id
    }

    #[must_use]
    pub const fn parameters_and_inputs_digest(&self) -> &[u8; 32] {
        &self.parameters_and_inputs_digest
    }

    #[must_use]
    pub const fn frontend_options_digest(&self) -> &[u8; 32] {
        &self.frontend_options_digest
    }

    #[must_use]
    pub const fn random_seed(&self) -> Option<u64> {
        self.random_seed
    }

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

    #[must_use]
    pub fn authoring_namespace_id(&self) -> &str {
        &self.authoring_namespace_id
    }

    #[must_use]
    pub fn source_document_key(&self) -> &str {
        &self.source_document_key
    }

    #[must_use]
    pub fn imports(&self) -> impl ExactSizeIterator<Item = &str> {
        self.imports.iter().map(AsRef::as_ref)
    }

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
    pub fn try_new(x: f64, y: f64, z: f64) -> Result<Self, DiagnosticBundle> {
        Ok(Self {
            x: validate_finite(x, "point.x")?,
            y: validate_finite(y, "point.y")?,
            z: validate_finite(z, "point.z")?,
        })
    }

    #[must_use]
    pub const fn x(self) -> f64 {
        self.x
    }

    #[must_use]
    pub const fn y(self) -> f64 {
        self.y
    }

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

    #[must_use]
    pub const fn start_width_meters(self) -> f64 {
        self.start_width_meters
    }

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
    #[must_use]
    pub const fn line(end: RoadEditingPoint3) -> Self {
        Self {
            geometry: RoadEditingCurveSegmentGeometry::Line { end },
            canvas_selection: None,
        }
    }

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

    pub fn with_canvas_selection(
        mut self,
        canvas_selection: impl Into<String>,
    ) -> Result<Self, DiagnosticBundle> {
        self.canvas_selection = Some(validated_canvas(canvas_selection.into())?);
        Ok(self)
    }

    #[must_use]
    pub const fn geometry(&self) -> RoadEditingCurveSegmentGeometry {
        self.geometry
    }

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

    #[must_use]
    pub const fn start(&self) -> RoadEditingPoint3 {
        self.start
    }

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

    pub fn with_canvas_selection(
        mut self,
        canvas_selection: impl Into<String>,
    ) -> Result<Self, DiagnosticBundle> {
        self.canvas_selection = Some(validated_canvas(canvas_selection.into())?);
        Ok(self)
    }

    #[must_use]
    pub fn road_alignment_key(&self) -> &str {
        &self.road_alignment_key
    }

    #[must_use]
    pub const fn canonical_frame(&self) -> &CanonicalFrameReference {
        &self.canonical_frame
    }

    #[must_use]
    pub const fn reference_line(&self) -> &RoadEditingCurveProgram {
        &self.reference_line
    }

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
            pub fn with_canvas_selection(
                mut self,
                canvas_selection: impl Into<String>,
            ) -> Result<Self, DiagnosticBundle> {
                self.canvas_selection = Some(validated_canvas(canvas_selection.into())?);
                Ok(self)
            }

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

    #[must_use]
    pub fn road_corridor_key(&self) -> &str {
        &self.road_corridor_key
    }

    #[must_use]
    pub const fn road_alignment(&self) -> &RoadAlignmentReference {
        &self.road_alignment
    }

    #[must_use]
    pub const fn start_station_meters(&self) -> f64 {
        self.start_station_meters
    }

    #[must_use]
    pub const fn end_station(&self) -> RoadEditingStationEnd {
        self.end_station
    }

    #[must_use]
    pub const fn reference_section(&self) -> &RoadSectionReference {
        &self.reference_section
    }

    #[must_use]
    pub const fn reference_lane(&self) -> &AuthoringLaneReference {
        &self.reference_lane
    }

    #[must_use]
    pub fn elements(&self) -> &[RoadEditingCorridorElement] {
        &self.elements
    }
}
impl_canvas!(RoadCorridorInput);

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

    #[must_use]
    pub fn road_section_key(&self) -> &str {
        &self.road_section_key
    }

    #[must_use]
    pub fn kind_id(&self) -> &str {
        &self.kind_id
    }

    #[must_use]
    pub fn authoring_lanes(&self) -> &[AuthoringLaneReference] {
        &self.authoring_lanes
    }

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

    #[must_use]
    pub fn authoring_lane_key(&self) -> &str {
        &self.authoring_lane_key
    }

    #[must_use]
    pub const fn lane_edge(&self) -> &LaneEdgeReference {
        &self.lane_edge
    }

    #[must_use]
    pub const fn direction(&self) -> RoadEditingLaneDirection {
        self.direction
    }

    #[must_use]
    pub const fn width_profile(&self) -> LinearWidthProfile {
        self.width_profile
    }

    #[must_use]
    pub const fn lane_group(&self) -> Option<&LaneGroupReference> {
        self.lane_group.as_ref()
    }

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
    pub fn try_new(
        lane_edge_key: impl Into<String>,
        speed_limit_meters_per_second: f64,
        successors: Vec<LaneEdgeReference>,
        explicit_geometry: Option<RoadEditingCurveProgram>,
    ) -> Result<Self, DiagnosticBundle> {
        let lane_edge_key = lane_edge_key.into();
        validate_token(&lane_edge_key, "laneEdge.laneEdgeKey")?;
        let speed_limit_meters_per_second = validate_positive(
            speed_limit_meters_per_second,
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

    #[must_use]
    pub fn lane_edge_key(&self) -> &str {
        &self.lane_edge_key
    }

    #[must_use]
    pub const fn speed_limit_meters_per_second(&self) -> f64 {
        self.speed_limit_meters_per_second
    }

    #[must_use]
    pub fn successors(&self) -> &[LaneEdgeReference] {
        &self.successors
    }

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
    pub fn try_new(
        junction_key: impl Into<String>,
        approach_edges: Vec<LaneEdgeReference>,
        internal_edges: Vec<LaneEdgeReference>,
    ) -> Result<Self, DiagnosticBundle> {
        let junction_key = junction_key.into();
        validate_token(&junction_key, "junction.junctionKey")?;
        require_non_empty(&approach_edges, "junction.approachEdges")?;
        require_non_empty(&internal_edges, "junction.internalEdges")?;
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

    #[must_use]
    pub fn junction_key(&self) -> &str {
        &self.junction_key
    }

    #[must_use]
    pub fn approach_edges(&self) -> &[LaneEdgeReference] {
        &self.approach_edges
    }

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
    canvas_selection: Option<Box<str>>,
}

impl MovementInput {
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
            canvas_selection: None,
        })
    }

    #[must_use]
    pub fn movement_key(&self) -> &str {
        &self.movement_key
    }

    #[must_use]
    pub const fn junction(&self) -> &JunctionReference {
        &self.junction
    }

    #[must_use]
    pub fn directed_entry_approach_key(&self) -> &str {
        &self.directed_entry_approach_key
    }

    #[must_use]
    pub fn directed_exit_approach_key(&self) -> &str {
        &self.directed_exit_approach_key
    }
}
impl_canvas!(MovementInput);

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

    #[must_use]
    pub fn maneuver_path_key(&self) -> &str {
        &self.maneuver_path_key
    }

    #[must_use]
    pub const fn movement(&self) -> &MovementReference {
        &self.movement
    }

    #[must_use]
    pub const fn entry_edge(&self) -> &LaneEdgeReference {
        &self.entry_edge
    }

    #[must_use]
    pub fn internal_edges(&self) -> &[LaneEdgeReference] {
        &self.internal_edges
    }

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

    #[must_use]
    pub fn maneuver_gate_key(&self) -> &str {
        &self.maneuver_gate_key
    }

    #[must_use]
    pub const fn maneuver_path(&self) -> &ManeuverPathReference {
        &self.maneuver_path
    }

    #[must_use]
    pub const fn transition_index(&self) -> u32 {
        self.transition_index
    }

    #[must_use]
    pub const fn stop_line(&self) -> &StopLineReference {
        &self.stop_line
    }

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

    #[must_use]
    pub fn waiting_zone_key(&self) -> &str {
        &self.waiting_zone_key
    }

    #[must_use]
    pub const fn maneuver_path(&self) -> &ManeuverPathReference {
        &self.maneuver_path
    }

    #[must_use]
    pub const fn entry_gate(&self) -> &ManeuverGateReference {
        &self.entry_gate
    }

    #[must_use]
    pub const fn release_gate(&self) -> &ManeuverGateReference {
        &self.release_gate
    }

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

    #[must_use]
    pub fn stop_line_key(&self) -> &str {
        &self.stop_line_key
    }

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
    pub fn try_new(signal_group_key: impl Into<String>) -> Result<Self, DiagnosticBundle> {
        let signal_group_key = signal_group_key.into();
        validate_token(&signal_group_key, "signalGroup.signalGroupKey")?;
        Ok(Self {
            signal_group_key: signal_group_key.into_boxed_str(),
            canvas_selection: None,
        })
    }

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

    #[must_use]
    pub const fn signal_group(&self) -> &SignalGroupReference {
        &self.signal_group
    }

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

    #[must_use]
    pub fn signal_controller_key(&self) -> &str {
        &self.signal_controller_key
    }

    #[must_use]
    pub const fn offset_milliseconds(&self) -> u64 {
        self.offset_milliseconds
    }

    #[must_use]
    pub fn signal_groups(&self) -> &[SignalGroupReference] {
        &self.signal_groups
    }

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
    pub fn try_new(
        signal_phase_key: impl Into<String>,
        duration_milliseconds: u64,
        states: Vec<RoadEditingSignalPhaseState>,
        signal_controller: SignalControllerReference,
    ) -> Result<Self, DiagnosticBundle> {
        let signal_phase_key = signal_phase_key.into();
        validate_token(&signal_phase_key, "signalPhase.signalPhaseKey")?;
        if duration_milliseconds == 0 {
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

    #[must_use]
    pub fn signal_phase_key(&self) -> &str {
        &self.signal_phase_key
    }

    #[must_use]
    pub const fn duration_milliseconds(&self) -> u64 {
        self.duration_milliseconds
    }

    #[must_use]
    pub fn states(&self) -> &[RoadEditingSignalPhaseState] {
        &self.states
    }

    #[must_use]
    pub const fn signal_controller(&self) -> &SignalControllerReference {
        &self.signal_controller
    }
}
impl_canvas!(SignalPhaseInput);

/// 可选组织停车位的停车区域声明。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParkingAreaInput {
    parking_area_key: Box<str>,
    canvas_selection: Option<Box<str>>,
}

impl ParkingAreaInput {
    pub fn try_new(parking_area_key: impl Into<String>) -> Result<Self, DiagnosticBundle> {
        let parking_area_key = parking_area_key.into();
        validate_token(&parking_area_key, "parkingArea.parkingAreaKey")?;
        Ok(Self {
            parking_area_key: parking_area_key.into_boxed_str(),
            canvas_selection: None,
        })
    }

    #[must_use]
    pub fn parking_area_key(&self) -> &str {
        &self.parking_area_key
    }
}
impl_canvas!(ParkingAreaInput);

/// 停车位在车道图边上的入口或出口锚点。
#[derive(Clone, Debug, PartialEq)]
pub struct ParkingLaneAnchor {
    lane_edge: LaneEdgeReference,
    progress_meters: f64,
}

impl ParkingLaneAnchor {
    pub fn try_new(
        lane_edge: LaneEdgeReference,
        progress_meters: f64,
    ) -> Result<Self, DiagnosticBundle> {
        let progress_meters =
            validate_positive(progress_meters, "parkingLaneAnchor.progressMeters")?;
        if progress_meters <= PARKING_ANCHOR_ENDPOINT_CLEARANCE_METERS {
            return Err(input_error(
                "parkingLaneAnchor.progressMeters",
                RoadEditingInputViolation::InvalidCombination,
            ));
        }
        Ok(Self {
            lane_edge,
            progress_meters,
        })
    }

    #[must_use]
    pub const fn lane_edge(&self) -> &LaneEdgeReference {
        &self.lane_edge
    }

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
    pub fn try_new(
        lateral_offset_meters: f64,
        heading_offset_radians: f64,
        length_meters: f64,
        width_meters: f64,
    ) -> Result<Self, DiagnosticBundle> {
        let lateral_offset_meters = validate_finite(
            lateral_offset_meters,
            "parkingSpace.geometry.lateralOffsetMeters",
        )?;
        if lateral_offset_meters.abs() <= MIN_PARKING_LATERAL_OFFSET_ABS_EXCLUSIVE_METERS {
            return Err(input_error(
                "parkingSpace.geometry.lateralOffsetMeters",
                RoadEditingInputViolation::InvalidCombination,
            ));
        }
        let heading_offset_radians = validate_finite(
            heading_offset_radians,
            "parkingSpace.geometry.headingOffsetRadians",
        )?;
        if !(PARKING_HEADING_OFFSET_MINIMUM_RADIANS..PARKING_HEADING_OFFSET_MAXIMUM_RADIANS)
            .contains(&heading_offset_radians)
        {
            return Err(input_error(
                "parkingSpace.geometry.headingOffsetRadians",
                RoadEditingInputViolation::InvalidCombination,
            ));
        }
        let length_meters = validate_positive(length_meters, "parkingSpace.geometry.lengthMeters")?;
        let width_meters = validate_positive(width_meters, "parkingSpace.geometry.widthMeters")?;
        if length_meters <= MIN_PARKING_EXTENT_EXCLUSIVE_METERS
            || width_meters <= MIN_PARKING_EXTENT_EXCLUSIVE_METERS
        {
            return Err(input_error(
                "parkingSpace.geometry.extent",
                RoadEditingInputViolation::InvalidCombination,
            ));
        }
        Ok(Self {
            lateral_offset_meters,
            heading_offset_radians,
            length_meters,
            width_meters,
        })
    }

    #[must_use]
    pub const fn lateral_offset_meters(self) -> f64 {
        self.lateral_offset_meters
    }

    #[must_use]
    pub const fn heading_offset_radians(self) -> f64 {
        self.heading_offset_radians
    }

    #[must_use]
    pub const fn length_meters(self) -> f64 {
        self.length_meters
    }

    #[must_use]
    pub const fn width_meters(self) -> f64 {
        self.width_meters
    }
}

/// 停车位声明。
#[derive(Clone, Debug, PartialEq)]
pub struct ParkingSpaceInput {
    parking_space_key: Box<str>,
    parking_area: Option<ParkingAreaReference>,
    entry: ParkingLaneAnchor,
    exit: ParkingLaneAnchor,
    geometry: ParkingSpaceGeometry,
    canvas_selection: Option<Box<str>>,
}

impl ParkingSpaceInput {
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
            parking_area: None,
            entry,
            exit,
            geometry,
            canvas_selection: None,
        })
    }

    #[must_use]
    pub fn with_parking_area(mut self, parking_area: ParkingAreaReference) -> Self {
        self.parking_area = Some(parking_area);
        self
    }

    #[must_use]
    pub fn parking_space_key(&self) -> &str {
        &self.parking_space_key
    }

    #[must_use]
    pub const fn parking_area(&self) -> Option<&ParkingAreaReference> {
        self.parking_area.as_ref()
    }

    #[must_use]
    pub const fn entry(&self) -> &ParkingLaneAnchor {
        &self.entry
    }

    #[must_use]
    pub const fn exit(&self) -> &ParkingLaneAnchor {
        &self.exit
    }

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

    #[must_use]
    pub fn lane_group_key(&self) -> &str {
        &self.lane_group_key
    }

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
        Ok(Self {
            facility_band_key: facility_band_key.into_boxed_str(),
            kind_id: kind_id.into_boxed_str(),
            width_profile,
            road_corridor,
            canvas_selection: None,
        })
    }

    #[must_use]
    pub fn facility_band_key(&self) -> &str {
        &self.facility_band_key
    }

    #[must_use]
    pub fn kind_id(&self) -> &str {
        &self.kind_id
    }

    #[must_use]
    pub const fn width_profile(&self) -> LinearWidthProfile {
        self.width_profile
    }

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

    #[must_use]
    pub fn with_extends(mut self, extends: ParticipantClassReference) -> Self {
        self.extends = Some(extends);
        self
    }

    #[must_use]
    pub fn participant_class_key(&self) -> &str {
        &self.participant_class_key
    }

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

/// 准入规则携带的可选法规来源。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccessRegulationInput {
    jurisdiction: Box<str>,
    version: Box<str>,
    source: Option<Box<str>>,
}

impl AccessRegulationInput {
    pub fn try_new(
        jurisdiction: impl Into<String>,
        version: impl Into<String>,
    ) -> Result<Self, DiagnosticBundle> {
        let jurisdiction = jurisdiction.into();
        let version = version.into();
        validate_visible_ascii(&jurisdiction, "accessRegulation.jurisdiction")?;
        validate_visible_ascii(&version, "accessRegulation.version")?;
        Ok(Self {
            jurisdiction: jurisdiction.into_boxed_str(),
            version: version.into_boxed_str(),
            source: None,
        })
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Result<Self, DiagnosticBundle> {
        let source = source.into();
        validate_visible_ascii(&source, "accessRegulation.source")?;
        self.source = Some(source.into_boxed_str());
        Ok(self)
    }

    #[must_use]
    pub fn jurisdiction(&self) -> &str {
        &self.jurisdiction
    }

    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    #[must_use]
    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }
}

/// 静态准入规则声明。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccessRuleInput {
    access_rule_key: Box<str>,
    target: RoadEditingAccessTarget,
    effect: AccessEffect,
    participant_classes: Box<[ParticipantClassReference]>,
    regulation: Option<AccessRegulationInput>,
    priority: i32,
    canvas_selection: Option<Box<str>>,
}

impl AccessRuleInput {
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

    #[must_use]
    pub fn with_regulation(mut self, regulation: AccessRegulationInput) -> Self {
        self.regulation = Some(regulation);
        self
    }

    #[must_use]
    pub fn access_rule_key(&self) -> &str {
        &self.access_rule_key
    }

    #[must_use]
    pub const fn target(&self) -> &RoadEditingAccessTarget {
        &self.target
    }

    #[must_use]
    pub const fn effect(&self) -> AccessEffect {
        self.effect
    }

    #[must_use]
    pub fn participant_classes(&self) -> &[ParticipantClassReference] {
        &self.participant_classes
    }

    #[must_use]
    pub const fn regulation(&self) -> Option<&AccessRegulationInput> {
        self.regulation.as_ref()
    }

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
        let length_meters = validate_positive(length_meters, "vehicleProfile.iidm.lengthMeters")?;
        if length_meters <= MIN_VEHICLE_LENGTH_EXCLUSIVE_METERS {
            return Err(input_error(
                "vehicleProfile.iidm.lengthMeters",
                RoadEditingInputViolation::InvalidCombination,
            ));
        }
        let desired_speed_meters_per_second = validate_positive(
            desired_speed_meters_per_second,
            "vehicleProfile.iidm.desiredSpeedMetersPerSecond",
        )?;
        let min_gap_meters =
            validate_non_negative(min_gap_meters, "vehicleProfile.iidm.minGapMeters")?;
        let time_headway_seconds = validate_positive(
            time_headway_seconds,
            "vehicleProfile.iidm.timeHeadwaySeconds",
        )?;
        let max_acceleration_meters_per_second_squared = validate_positive(
            max_acceleration_meters_per_second_squared,
            "vehicleProfile.iidm.maxAccelerationMetersPerSecondSquared",
        )?;
        let comfortable_deceleration_meters_per_second_squared = validate_positive(
            comfortable_deceleration_meters_per_second_squared,
            "vehicleProfile.iidm.comfortableDecelerationMetersPerSecondSquared",
        )?;
        let emergency_deceleration_meters_per_second_squared = validate_positive(
            emergency_deceleration_meters_per_second_squared,
            "vehicleProfile.iidm.emergencyDecelerationMetersPerSecondSquared",
        )?;
        if emergency_deceleration_meters_per_second_squared
            < comfortable_deceleration_meters_per_second_squared
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

    #[must_use]
    pub const fn length_meters(self) -> f64 {
        self.length_meters
    }
    #[must_use]
    pub const fn desired_speed_meters_per_second(self) -> f64 {
        self.desired_speed_meters_per_second
    }
    #[must_use]
    pub const fn min_gap_meters(self) -> f64 {
        self.min_gap_meters
    }
    #[must_use]
    pub const fn time_headway_seconds(self) -> f64 {
        self.time_headway_seconds
    }
    #[must_use]
    pub const fn max_acceleration_meters_per_second_squared(self) -> f64 {
        self.max_acceleration_meters_per_second_squared
    }
    #[must_use]
    pub const fn comfortable_deceleration_meters_per_second_squared(self) -> f64 {
        self.comfortable_deceleration_meters_per_second_squared
    }
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

    #[must_use]
    pub fn vehicle_profile_key(&self) -> &str {
        &self.vehicle_profile_key
    }
    #[must_use]
    pub const fn participant_class(&self) -> &ParticipantClassReference {
        &self.participant_class
    }
    #[must_use]
    pub const fn iidm(&self) -> IidmVehicleProfileInput {
        self.iidm
    }
}
impl_canvas!(VehicleProfileInput);

/// 编制期权威的有序静态路线。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StaticRouteInput {
    static_route_key: Box<str>,
    edge_sequence: Box<[LaneEdgeReference]>,
    canvas_selection: Option<Box<str>>,
}

impl StaticRouteInput {
    pub fn try_new(
        static_route_key: impl Into<String>,
        edge_sequence: Vec<LaneEdgeReference>,
    ) -> Result<Self, DiagnosticBundle> {
        let static_route_key = static_route_key.into();
        validate_token(&static_route_key, "staticRoute.staticRouteKey")?;
        require_non_empty(&edge_sequence, "staticRoute.edgeSequence")?;
        Ok(Self {
            static_route_key: static_route_key.into_boxed_str(),
            edge_sequence: edge_sequence.into_boxed_slice(),
            canvas_selection: None,
        })
    }

    #[must_use]
    pub fn static_route_key(&self) -> &str {
        &self.static_route_key
    }
    #[must_use]
    pub fn edge_sequence(&self) -> &[LaneEdgeReference] {
        &self.edge_sequence
    }
}
impl_canvas!(StaticRouteInput);

/// 固定单位、手性与范围的规范坐标框架声明。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalFrameInput {
    canonical_frame_key: Box<str>,
    canvas_selection: Option<Box<str>>,
}

impl CanonicalFrameInput {
    pub fn try_new(canonical_frame_key: impl Into<String>) -> Result<Self, DiagnosticBundle> {
        let canonical_frame_key = canonical_frame_key.into();
        validate_token(&canonical_frame_key, "canonicalFrame.canonicalFrameKey")?;
        Ok(Self {
            canonical_frame_key: canonical_frame_key.into_boxed_str(),
            canvas_selection: None,
        })
    }

    #[must_use]
    pub fn canonical_frame_key(&self) -> &str {
        &self.canonical_frame_key
    }
}
impl_canvas!(CanonicalFrameInput);

/// Road Editing Source v1 的 22 类稳定声明闭集。
#[derive(Clone, Debug, PartialEq)]
pub enum RoadEditingDeclaration {
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
    ParkingArea(ParkingAreaInput),
    ParkingSpace(ParkingSpaceInput),
    LaneGroup(LaneGroupInput),
    FacilityBand(FacilityBandInput),
    ParticipantClass(ParticipantClassInput),
    AccessRule(AccessRuleInput),
    VehicleProfile(VehicleProfileInput),
    StaticRoute(StaticRouteInput),
    CanonicalFrame(CanonicalFrameInput),
}

impl RoadEditingDeclaration {
    /// 返回与 Identity v1 registry 一致的声明类别。
    #[must_use]
    pub const fn entity_kind(&self) -> EntityKind {
        match self {
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
            Self::ParkingArea(_) => EntityKind::ParkingArea,
            Self::ParkingSpace(_) => EntityKind::ParkingSpace,
            Self::LaneGroup(_) => EntityKind::LaneGroup,
            Self::FacilityBand(_) => EntityKind::FacilityBand,
            Self::ParticipantClass(_) => EntityKind::ParticipantClass,
            Self::AccessRule(_) => EntityKind::AccessRule,
            Self::VehicleProfile(_) => EntityKind::VehicleProfile,
            Self::StaticRoute(_) => EntityKind::StaticRoute,
            Self::CanonicalFrame(_) => EntityKind::CanonicalFrame,
        }
    }

    /// 返回声明在其直接 owner 下的稳定 local key。
    #[must_use]
    pub fn local_key(&self) -> &str {
        match self {
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
            Self::ParkingArea(value) => value.parking_area_key(),
            Self::ParkingSpace(value) => value.parking_space_key(),
            Self::LaneGroup(value) => value.lane_group_key(),
            Self::FacilityBand(value) => value.facility_band_key(),
            Self::ParticipantClass(value) => value.participant_class_key(),
            Self::AccessRule(value) => value.access_rule_key(),
            Self::VehicleProfile(value) => value.vehicle_profile_key(),
            Self::StaticRoute(value) => value.static_route_key(),
            Self::CanonicalFrame(value) => value.canonical_frame_key(),
        }
    }

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
            Self::RoadCorridor(_)
            | Self::LaneEdge(_)
            | Self::Junction(_)
            | Self::StopLine(_)
            | Self::SignalGroup(_)
            | Self::SignalController(_)
            | Self::ParkingArea(_)
            | Self::ParkingSpace(_)
            | Self::ParticipantClass(_)
            | Self::AccessRule(_)
            | Self::VehicleProfile(_)
            | Self::StaticRoute(_)
            | Self::CanonicalFrame(_) => Box::new([]),
        }
    }

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
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_scoped_reference_requires_exact_identity_depth() {
        assert!(RoadSectionReference::local("section-a").is_err());
        assert!(RoadSectionReference::owner_scoped(vec!["corridor-a".into()], "section-a").is_ok());
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
        assert!(LinearWidthProfile::try_new(0.0, 0.0).is_err());
        assert!(LaneEdgeInput::try_new("edge", 0.0, Vec::new(), None).is_err());

        let point = RoadEditingPoint3::try_new(-0.0, -0.0, -0.0).expect("canonical point");
        assert_eq!(point.x().to_bits(), 0.0_f64.to_bits());
        assert_eq!(point.y().to_bits(), 0.0_f64.to_bits());
        assert_eq!(point.z().to_bits(), 0.0_f64.to_bits());
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
    fn declaration_address_retains_full_owner_tuple() {
        let section = RoadSectionInput::try_new(
            "section-a",
            "road",
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
}

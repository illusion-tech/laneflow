//! 闭合、与来源介质匹配的编译来源位置。
//!
//! 文本前端继续使用 [`SourceSpan`]；道路编辑 FlatBuffer 使用稳定文档身份、实体地址、
//! owner-local 关系与闭合属性路径。道路编辑位置中的 ordinal 只在一次编译内解析共享
//! context，不参与持久身份、摘要或规范排序。

use std::cmp::Ordering;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use laneflow_static_contract::EntityKind;

use crate::SourceSpan;

/// 编译器支持的闭合来源位置。
#[derive(Clone, Debug)]
pub enum SourceLocation {
    /// 具有真实一基行列范围的文本来源。
    Text(SourceSpan),
    /// 受检道路编辑 FlatBuffer 中的有类型语义位置。
    RoadEditing(RoadEditingSourceLocation),
}

impl SourceLocation {
    /// 返回位置所属的稳定来源文档键。
    #[must_use]
    pub fn source_document_key(&self) -> &str {
        match self {
            Self::Text(span) => span.source_document_key(),
            Self::RoadEditing(location) => location.document_identity.source_document_key(),
        }
    }

    /// 若位置来自文本，返回真实文本范围。
    #[must_use]
    pub const fn text_span(&self) -> Option<&SourceSpan> {
        match self {
            Self::Text(span) => Some(span),
            Self::RoadEditing(_) => None,
        }
    }

    /// 若位置来自道路编辑来源，返回其有类型位置。
    #[must_use]
    pub const fn road_editing(&self) -> Option<&RoadEditingSourceLocation> {
        match self {
            Self::Text(_) => None,
            Self::RoadEditing(location) => Some(location),
        }
    }
}

impl From<SourceSpan> for SourceLocation {
    fn from(value: SourceSpan) -> Self {
        Self::Text(value)
    }
}

impl PartialEq for SourceLocation {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for SourceLocation {}

impl PartialOrd for SourceLocation {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SourceLocation {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Text(left), Self::Text(right)) => left.cmp(right),
            (Self::Text(_), Self::RoadEditing(_)) => Ordering::Less,
            (Self::RoadEditing(_), Self::Text(_)) => Ordering::Greater,
            (Self::RoadEditing(left), Self::RoadEditing(right)) => left.cmp(right),
        }
    }
}

impl Hash for SourceLocation {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::Text(span) => {
                0_u8.hash(state);
                span.hash(state);
            }
            Self::RoadEditing(location) => {
                1_u8.hash(state);
                location.hash(state);
            }
        }
    }
}

/// verifier 前后不同可信度的道路编辑文档身份。
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RoadEditingDocumentIdentity {
    /// wire 尚未验证，只能使用调用方在输入外提供并已受检的预期文档键。
    Input(RoadEditingInputDocumentIdentity),
    /// wire 模块命名空间和文档键均已验证且与外部预期键逐字节相等。
    Verified(RoadEditingVerifiedDocumentIdentity),
}

impl RoadEditingDocumentIdentity {
    /// 返回稳定来源文档键。
    #[must_use]
    pub fn source_document_key(&self) -> &str {
        match self {
            Self::Input(identity) => &identity.expected_source_document_key,
            Self::Verified(identity) => &identity.source_document_key,
        }
    }

    /// verifier 成功后返回受检模块命名空间；输入级损坏诊断没有该值。
    #[must_use]
    pub fn module_namespace(&self) -> Option<&str> {
        match self {
            Self::Input(_) => None,
            Self::Verified(identity) => Some(&identity.module_namespace),
        }
    }

    #[cfg(test)]
    pub(crate) fn module_namespace_arc(&self) -> Option<Arc<str>> {
        match self {
            Self::Input(_) => None,
            Self::Verified(identity) => Some(Arc::clone(&identity.module_namespace)),
        }
    }

    pub(crate) fn input(expected_source_document_key: Arc<str>) -> Self {
        Self::Input(RoadEditingInputDocumentIdentity {
            expected_source_document_key,
        })
    }

    pub(crate) fn verified(module_namespace: Arc<str>, source_document_key: Arc<str>) -> Self {
        Self::Verified(RoadEditingVerifiedDocumentIdentity {
            module_namespace,
            source_document_key,
        })
    }
}

/// verifier 前可用的道路编辑输入文档身份。
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RoadEditingInputDocumentIdentity {
    expected_source_document_key: Arc<str>,
}

/// verifier 后可用的道路编辑模块和文档身份。
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RoadEditingVerifiedDocumentIdentity {
    module_namespace: Arc<str>,
    source_document_key: Arc<str>,
}

/// 道路编辑 schema 的根向量种类。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum RoadEditingRootVectorKind {
    RoadAlignment,
    RoadCorridor,
    RoadSection,
    AuthoringLane,
    LaneEdge,
    Junction,
    Movement,
    ManeuverPath,
    ManeuverGate,
    WaitingZone,
    StopLine,
    SignalGroup,
    SignalController,
    SignalPhase,
    ParkingFacility,
    ParkingSpace,
    LaneGroup,
    FacilityBand,
    ParticipantClass,
    AccessRule,
    VehicleProfile,
    CanonicalFrame,
    ConflictZone,
    ParticipantStream,
    ConflictZoneRegion,
}

/// 道路编辑来源地址中的有类型声明种类；道路走向不是 Identity v1 实体。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RoadEditingAddressKind {
    RoadAlignment,
    Declaration(EntityKind),
}

/// 道路编辑 schema 中可出现在来源路径或 wire fallback 的 table 种类。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum RoadEditingTableKind {
    RoadEditingSource,
    ModuleHeader,
    Provenance,
    LineSegment,
    CubicBezierSegment,
    CurveSegment,
    CurveProgram,
    RoadAlignment,
    CorridorElement,
    RoadCorridor,
    RoadSection,
    AuthoringLane,
    LaneEdge,
    Junction,
    Movement,
    ManeuverPath,
    ManeuverGate,
    WaitingZone,
    StopLine,
    SignalGroup,
    SignalController,
    SignalPhaseState,
    SignalPhase,
    ParkingFacility,
    ParkingLaneAnchor,
    ParkingSpaceGeometry,
    ParkingSpace,
    LaneGroup,
    FacilityBand,
    ParticipantClass,
    AccessRegulation,
    AccessRule,
    IidmVehicleProfile,
    VehicleProfile,
    ConflictZoneRegion,
    CanonicalFrame,
    ConflictZone,
    PathAnchor,
    ConflictPassage,
    ParticipantStream,
    RightOfWayPolicySet,
    PolicyEvidence,
    PolicyGapProfile,
    PolicyStreamRule,
    PolicyGateRule,
}

/// 道路编辑 schema 中可出现在来源路径的 inline struct 种类。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum RoadEditingStructKind {
    Digest256,
    OptionalU64,
    Vec3F64,
    LinearWidthProfile,
    Vec2F64,
}

/// 道路编辑 schema 中可出现在来源路径的 union 种类。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum RoadEditingUnionKind {
    CurveSegmentGeometry,
}

/// owner-local 关系的闭合集合。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum RoadEditingRelationKind {
    Import,
    CurveSegment,
    CorridorElement,
    RoadSectionAuthoringLane,
    LaneEdgeSuccessor,
    JunctionApproachEdge,
    JunctionInternalEdge,
    ManeuverPathInternalEdge,
    SignalControllerGroup,
    SignalControllerPhase,
    SignalPhaseState,
    AccessRuleParticipantClass,
    ParkingFacilityVirtualEntry,
    ParkingFacilityVirtualExit,
    ParticipantStreamPassage,
    ConflictZoneRegion,
    PolicyEvidence,
    PolicyGapProfile,
    PolicyStreamRule,
    PolicyGateRule,
}

/// 有序产品关系或规范集合关系中的稳定 occurrence。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RoadEditingRelationOccurrence {
    OrderedProductOrdinal(u32),
    CanonicalSetOrdinal(u32),
}

/// context 内字符串的编译期 ordinal；不得持久化或参与摘要。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RoadEditingStringOrdinal(u32);

/// context 内属性路径的编译期 ordinal；不得持久化或参与摘要。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RoadEditingPropertyPathOrdinal(u32);

/// context 内画布选择键的编译期 ordinal；不得持久化或参与摘要。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RoadEditingCanvasSelectionOrdinal(u32);

const MAX_ROAD_EDITING_OWNER_DEPTH: usize = 3;
const ROAD_EDITING_CONTEXT_HEADER_LOGICAL_BYTES: u64 = 4 + 4 + 4;
const ROAD_EDITING_CONTEXT_ITEM_LENGTH_LOGICAL_BYTES: u64 = 4;
const ROAD_EDITING_PROPERTY_STEP_LOGICAL_BYTES: u64 = 4 + 4;

/// 道路编辑来源中的稳定实体地址；它不是产品 `CanonicalIdentity`。
#[derive(Clone, Copy, Debug)]
pub struct RoadEditingSourceAddress {
    module_namespace: RoadEditingStringOrdinal,
    kind: RoadEditingAddressKind,
    owner_local_keys: [RoadEditingStringOrdinal; MAX_ROAD_EDITING_OWNER_DEPTH],
    owner_local_key_count: u8,
    local_key: RoadEditingStringOrdinal,
}

impl RoadEditingSourceAddress {
    /// 返回来源地址种类。
    #[must_use]
    pub const fn kind(&self) -> RoadEditingAddressKind {
        self.kind
    }

    /// 稳定实体声明返回其 Identity v1 种类；道路走向返回 `None`。
    #[must_use]
    pub const fn entity_kind(&self) -> Option<EntityKind> {
        match self.kind {
            RoadEditingAddressKind::RoadAlignment => None,
            RoadEditingAddressKind::Declaration(kind) => Some(kind),
        }
    }

    /// 解析模块命名空间。
    #[must_use]
    pub fn module_namespace<'a>(&self, context: &'a RoadEditingLocationContext) -> &'a str {
        context.resolve_string(self.module_namespace)
    }

    /// 按父先子后顺序解析完整 owner local-key tuple。
    pub fn owner_local_keys<'a>(
        &'a self,
        context: &'a RoadEditingLocationContext,
    ) -> impl ExactSizeIterator<Item = &'a str> + 'a {
        self.owner_local_keys[..usize::from(self.owner_local_key_count)]
            .iter()
            .copied()
            .map(|ordinal| context.resolve_string(ordinal))
    }

    /// 解析直接 owner 下的 sibling-local key。
    #[must_use]
    pub fn local_key<'a>(&self, context: &'a RoadEditingLocationContext) -> &'a str {
        context.resolve_string(self.local_key)
    }

    #[allow(
        dead_code,
        reason = "consumed by the staged road-editing location-context builder"
    )]
    pub(crate) fn new<I>(
        module_namespace: RoadEditingStringOrdinal,
        kind: RoadEditingAddressKind,
        owner_local_keys: I,
        local_key: RoadEditingStringOrdinal,
    ) -> Self
    where
        I: IntoIterator<Item = RoadEditingStringOrdinal>,
    {
        let mut inline_owner_local_keys = [module_namespace; MAX_ROAD_EDITING_OWNER_DEPTH];
        let mut owner_local_key_count = 0_usize;
        for ordinal in owner_local_keys {
            assert!(
                owner_local_key_count < MAX_ROAD_EDITING_OWNER_DEPTH,
                "road-editing source addresses are bounded to three owner components"
            );
            inline_owner_local_keys[owner_local_key_count] = ordinal;
            owner_local_key_count += 1;
        }
        Self {
            module_namespace,
            kind,
            owner_local_keys: inline_owner_local_keys,
            owner_local_key_count: u8::try_from(owner_local_key_count)
                .expect("owner depth is bounded to three"),
            local_key,
        }
    }
}

/// 道路编辑来源中最多四步的闭合叶属性路径。
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RoadEditingPropertyPath {
    steps: Box<[RoadEditingPropertyStep]>,
}

impl RoadEditingPropertyPath {
    /// 返回按外到内顺序排列的属性步骤。
    #[must_use]
    pub fn steps(&self) -> &[RoadEditingPropertyStep] {
        &self.steps
    }

    #[allow(
        dead_code,
        reason = "consumed by the staged road-editing location-context builder"
    )]
    pub(crate) fn new(steps: Box<[RoadEditingPropertyStep]>) -> Self {
        assert!(
            (1..=4).contains(&steps.len()),
            "property path depth must be 1..=4"
        );
        Self { steps }
    }
}

/// 一个已知 table field、struct member 或 union variant 步骤。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RoadEditingPropertyStep {
    TableField {
        table: RoadEditingTableKind,
        field_id: u16,
    },
    StructMember {
        structure: RoadEditingStructKind,
        member_id: u8,
    },
    UnionVariant {
        union: RoadEditingUnionKind,
        discriminant: u8,
    },
}

/// 已证明完全位于输入 buffer 内的结构损坏字节范围。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RoadEditingByteRange {
    start: u32,
    length: u32,
}

impl RoadEditingByteRange {
    /// 返回零基起始 byte offset。
    #[must_use]
    pub const fn start(self) -> u32 {
        self.start
    }

    /// 返回范围 byte 数。
    #[must_use]
    pub const fn length(self) -> u32 {
        self.length
    }

    #[allow(
        dead_code,
        reason = "consumed by the staged verifier trace diagnostic integration"
    )]
    pub(crate) fn checked(start: u32, length: u32, source_len: usize) -> Option<Self> {
        let end = start.checked_add(length)?;
        (u64::from(end) <= u64::try_from(source_len).ok()?).then_some(Self { start, length })
    }
}

/// 道路编辑 owner-local 关系的有类型 owner。
#[derive(Clone, Copy, Debug)]
pub enum RoadEditingOwner {
    ModuleHeader,
    Address(RoadEditingSourceAddress),
}

/// 道路编辑语义或结构位置的闭合 subject。
#[derive(Clone, Copy, Debug)]
pub enum RoadEditingSubject {
    ModuleHeader,
    RoadAlignment {
        address: RoadEditingSourceAddress,
    },
    Declaration {
        address: RoadEditingSourceAddress,
    },
    OwnerLocal {
        owner: RoadEditingOwner,
        relation: RoadEditingRelationKind,
        occurrence: RoadEditingRelationOccurrence,
    },
    Wire {
        root_vector: RoadEditingRootVectorKind,
        physical_index: u32,
        table: RoadEditingTableKind,
    },
}

/// 一个模块共享、冻结后不可变的道路编辑位置 context。
#[derive(Debug)]
pub struct RoadEditingLocationContext {
    strings: Box<[Arc<str>]>,
    property_paths: Box<[RoadEditingPropertyPath]>,
    canvas_selection_keys: Box<[Arc<str>]>,
}

impl RoadEditingLocationContext {
    fn resolve_string(&self, ordinal: RoadEditingStringOrdinal) -> &str {
        &self.strings[ordinal.0 as usize]
    }

    fn resolve_property_path(
        &self,
        ordinal: RoadEditingPropertyPathOrdinal,
    ) -> &RoadEditingPropertyPath {
        &self.property_paths[ordinal.0 as usize]
    }

    fn resolve_canvas_selection(&self, ordinal: RoadEditingCanvasSelectionOrdinal) -> &str {
        &self.canvas_selection_keys[ordinal.0 as usize]
    }

    /// 返回该共享 context 在来源映射闭合逻辑编码中的字节数。
    ///
    /// 编码按三个有序表保存字符串、属性路径与画布选择。字符串项为 `u32` 长度加
    /// UTF-8 bytes；路径项为 `u32` 步数加固定 8-byte 有类型步骤。该口径不使用
    /// Rust `Arc`、enum padding 或 allocator 元数据。
    pub(crate) fn source_map_logical_bytes(&self) -> u64 {
        let strings = self.strings.iter().fold(0_u64, |total, value| {
            total
                .saturating_add(ROAD_EDITING_CONTEXT_ITEM_LENGTH_LOGICAL_BYTES)
                .saturating_add(u64::try_from(value.len()).unwrap_or(u64::MAX))
        });
        let property_paths = self.property_paths.iter().fold(0_u64, |total, path| {
            total
                .saturating_add(ROAD_EDITING_CONTEXT_ITEM_LENGTH_LOGICAL_BYTES)
                .saturating_add(
                    u64::try_from(path.steps().len())
                        .unwrap_or(u64::MAX)
                        .saturating_mul(ROAD_EDITING_PROPERTY_STEP_LOGICAL_BYTES),
                )
        });
        let canvas_selections = self
            .canvas_selection_keys
            .iter()
            .fold(0_u64, |total, value| {
                total
                    .saturating_add(ROAD_EDITING_CONTEXT_ITEM_LENGTH_LOGICAL_BYTES)
                    .saturating_add(u64::try_from(value.len()).unwrap_or(u64::MAX))
            });
        ROAD_EDITING_CONTEXT_HEADER_LOGICAL_BYTES
            .saturating_add(strings)
            .saturating_add(property_paths)
            .saturating_add(canvas_selections)
    }

    /// 返回该共享 context 自身全部堆分配的保守请求字节数。
    ///
    /// 每条 `RoadEditingSourceLocation` 中的强引用 handle 已由所属 Typed AST 记录的
    /// 结构大小覆盖；这里仅计一次 Arc allocation、三个 boxed slice、唯一字符串载荷与
    /// 属性路径 step backing，避免按位置数量重复计算同一个 context。
    pub(crate) fn controlled_live_bytes(&self) -> u64 {
        let usize_bytes = u64::try_from(core::mem::size_of::<usize>()).unwrap_or(u64::MAX);
        let arc_header_bytes = usize_bytes.saturating_mul(2);
        let arc_string_bytes = |value: &Arc<str>| {
            arc_header_bytes.saturating_add(u64::try_from(value.len()).unwrap_or(u64::MAX))
        };
        let string_slots = u64::try_from(self.strings.len()).unwrap_or(u64::MAX);
        let canvas_slots = u64::try_from(self.canvas_selection_keys.len()).unwrap_or(u64::MAX);
        let path_slots = u64::try_from(self.property_paths.len()).unwrap_or(u64::MAX);
        let string_payload = self
            .strings
            .iter()
            .chain(self.canvas_selection_keys.iter())
            .fold(0_u64, |total, value| {
                total.saturating_add(arc_string_bytes(value))
            });
        let path_step_bytes = self.property_paths.iter().fold(0_u64, |total, path| {
            total.saturating_add(
                u64::try_from(path.steps.len())
                    .unwrap_or(u64::MAX)
                    .saturating_mul(
                        u64::try_from(core::mem::size_of::<RoadEditingPropertyStep>())
                            .unwrap_or(u64::MAX),
                    ),
            )
        });

        arc_header_bytes
            .saturating_add(u64::try_from(core::mem::size_of::<Self>()).unwrap_or(u64::MAX))
            .saturating_add(string_slots.saturating_mul(
                u64::try_from(core::mem::size_of::<Arc<str>>()).unwrap_or(u64::MAX),
            ))
            .saturating_add(canvas_slots.saturating_mul(
                u64::try_from(core::mem::size_of::<Arc<str>>()).unwrap_or(u64::MAX),
            ))
            .saturating_add(path_slots.saturating_mul(
                u64::try_from(core::mem::size_of::<RoadEditingPropertyPath>()).unwrap_or(u64::MAX),
            ))
            .saturating_add(string_payload)
            .saturating_add(path_step_bytes)
    }

    pub(crate) fn string_ordinal_for(&self, value: &str) -> RoadEditingStringOrdinal {
        let index = self
            .strings
            .binary_search_by(|candidate| candidate.as_bytes().cmp(value.as_bytes()))
            .expect("location factory interns every address component before freezing");
        self.string_ordinal(index)
    }

    pub(crate) fn property_path_ordinal_for(
        &self,
        value: &RoadEditingPropertyPath,
    ) -> RoadEditingPropertyPathOrdinal {
        let index = self
            .property_paths
            .binary_search(value)
            .expect("location factory freezes every closed property path");
        self.property_path_ordinal(index)
    }

    pub(crate) fn canvas_selection_ordinal_for(
        &self,
        value: &str,
    ) -> RoadEditingCanvasSelectionOrdinal {
        let index = self
            .canvas_selection_keys
            .binary_search_by(|candidate| candidate.as_bytes().cmp(value.as_bytes()))
            .expect("location factory interns every canvas selection before freezing");
        self.canvas_selection_ordinal(index)
    }

    pub(crate) fn new(
        strings: Box<[Arc<str>]>,
        property_paths: Box<[RoadEditingPropertyPath]>,
        canvas_selection_keys: Box<[Arc<str>]>,
    ) -> Self {
        Self {
            strings,
            property_paths,
            canvas_selection_keys,
        }
    }

    pub(crate) fn string_ordinal(&self, index: usize) -> RoadEditingStringOrdinal {
        assert!(
            index < self.strings.len(),
            "string ordinal must resolve in context"
        );
        RoadEditingStringOrdinal(u32::try_from(index).expect("compile limits bound ordinals"))
    }

    pub(crate) fn property_path_ordinal(&self, index: usize) -> RoadEditingPropertyPathOrdinal {
        assert!(
            index < self.property_paths.len(),
            "property path ordinal must resolve in context"
        );
        RoadEditingPropertyPathOrdinal(u32::try_from(index).expect("compile limits bound ordinals"))
    }

    pub(crate) fn canvas_selection_ordinal(
        &self,
        index: usize,
    ) -> RoadEditingCanvasSelectionOrdinal {
        assert!(
            index < self.canvas_selection_keys.len(),
            "canvas selection ordinal must resolve in context"
        );
        RoadEditingCanvasSelectionOrdinal(
            u32::try_from(index).expect("compile limits bound ordinals"),
        )
    }
}

/// 一条道路编辑来源位置；所有 ordinal 均由同一个冻结 context 解析。
#[derive(Clone, Debug)]
pub struct RoadEditingSourceLocation {
    context: Arc<RoadEditingLocationContext>,
    document_identity: RoadEditingDocumentIdentity,
    subject: RoadEditingSubject,
    property_path: Option<RoadEditingPropertyPathOrdinal>,
    canvas_selection: Option<RoadEditingCanvasSelectionOrdinal>,
    byte_range: Option<RoadEditingByteRange>,
}

impl RoadEditingSourceLocation {
    /// 返回文档身份。
    #[must_use]
    pub const fn document_identity(&self) -> &RoadEditingDocumentIdentity {
        &self.document_identity
    }

    /// 返回稳定语义或结构 fallback subject。
    #[must_use]
    pub const fn subject(&self) -> &RoadEditingSubject {
        &self.subject
    }

    /// 返回共享的只读位置 context。
    #[must_use]
    pub fn context(&self) -> &RoadEditingLocationContext {
        &self.context
    }

    /// 解析可选闭合属性路径。
    #[must_use]
    pub fn property_path(&self) -> Option<&RoadEditingPropertyPath> {
        self.property_path
            .map(|ordinal| self.context.resolve_property_path(ordinal))
    }

    /// 解析可选画布选择键。
    #[must_use]
    pub fn canvas_selection(&self) -> Option<&str> {
        self.canvas_selection
            .map(|ordinal| self.context.resolve_canvas_selection(ordinal))
    }

    /// 仅结构损坏位置可携带受检 byte range。
    #[must_use]
    pub const fn byte_range(&self) -> Option<RoadEditingByteRange> {
        self.byte_range
    }

    pub(crate) fn new(
        context: Arc<RoadEditingLocationContext>,
        document_identity: RoadEditingDocumentIdentity,
        subject: RoadEditingSubject,
        property_path: Option<RoadEditingPropertyPathOrdinal>,
        canvas_selection: Option<RoadEditingCanvasSelectionOrdinal>,
        byte_range: Option<RoadEditingByteRange>,
    ) -> Self {
        Self {
            context,
            document_identity,
            subject,
            property_path,
            canvas_selection,
            byte_range,
        }
    }
}

impl PartialEq for RoadEditingSourceLocation {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for RoadEditingSourceLocation {}

impl PartialOrd for RoadEditingSourceLocation {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RoadEditingSourceLocation {
    fn cmp(&self, other: &Self) -> Ordering {
        self.document_identity
            .cmp(&other.document_identity)
            .then_with(|| {
                compare_subject(&self.subject, &self.context, &other.subject, &other.context)
            })
            .then_with(|| self.property_path().cmp(&other.property_path()))
            .then_with(|| self.canvas_selection().cmp(&other.canvas_selection()))
            .then_with(|| self.byte_range.cmp(&other.byte_range))
    }
}

impl Hash for RoadEditingSourceLocation {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.document_identity.hash(state);
        hash_subject(&self.subject, &self.context, state);
        self.property_path().hash(state);
        self.canvas_selection().hash(state);
        self.byte_range.hash(state);
    }
}

fn subject_rank(subject: &RoadEditingSubject) -> u8 {
    match subject {
        RoadEditingSubject::ModuleHeader => 0,
        RoadEditingSubject::RoadAlignment { .. } => 1,
        RoadEditingSubject::Declaration { .. } => 2,
        RoadEditingSubject::OwnerLocal { .. } => 3,
        RoadEditingSubject::Wire { .. } => 4,
    }
}

fn compare_subject(
    left: &RoadEditingSubject,
    left_context: &RoadEditingLocationContext,
    right: &RoadEditingSubject,
    right_context: &RoadEditingLocationContext,
) -> Ordering {
    subject_rank(left)
        .cmp(&subject_rank(right))
        .then_with(|| match (left, right) {
            (RoadEditingSubject::ModuleHeader, RoadEditingSubject::ModuleHeader) => Ordering::Equal,
            (
                RoadEditingSubject::RoadAlignment { address: left },
                RoadEditingSubject::RoadAlignment { address: right },
            )
            | (
                RoadEditingSubject::Declaration { address: left },
                RoadEditingSubject::Declaration { address: right },
            ) => compare_address(left, left_context, right, right_context),
            (
                RoadEditingSubject::OwnerLocal {
                    owner: left_owner,
                    relation: left_relation,
                    occurrence: left_occurrence,
                },
                RoadEditingSubject::OwnerLocal {
                    owner: right_owner,
                    relation: right_relation,
                    occurrence: right_occurrence,
                },
            ) => compare_owner(left_owner, left_context, right_owner, right_context)
                .then_with(|| left_relation.cmp(right_relation))
                .then_with(|| left_occurrence.cmp(right_occurrence)),
            (
                RoadEditingSubject::Wire {
                    root_vector: left_root,
                    physical_index: left_index,
                    table: left_table,
                },
                RoadEditingSubject::Wire {
                    root_vector: right_root,
                    physical_index: right_index,
                    table: right_table,
                },
            ) => left_root
                .cmp(right_root)
                .then_with(|| left_index.cmp(right_index))
                .then_with(|| left_table.cmp(right_table)),
            _ => Ordering::Equal,
        })
}

fn compare_address(
    left: &RoadEditingSourceAddress,
    left_context: &RoadEditingLocationContext,
    right: &RoadEditingSourceAddress,
    right_context: &RoadEditingLocationContext,
) -> Ordering {
    left.kind
        .cmp(&right.kind)
        .then_with(|| {
            left.module_namespace(left_context)
                .as_bytes()
                .cmp(right.module_namespace(right_context).as_bytes())
        })
        .then_with(|| {
            left.owner_local_keys(left_context)
                .map(str::as_bytes)
                .cmp(right.owner_local_keys(right_context).map(str::as_bytes))
        })
        .then_with(|| {
            left.local_key(left_context)
                .as_bytes()
                .cmp(right.local_key(right_context).as_bytes())
        })
}

fn compare_owner(
    left: &RoadEditingOwner,
    left_context: &RoadEditingLocationContext,
    right: &RoadEditingOwner,
    right_context: &RoadEditingLocationContext,
) -> Ordering {
    match (left, right) {
        (RoadEditingOwner::ModuleHeader, RoadEditingOwner::ModuleHeader) => Ordering::Equal,
        (RoadEditingOwner::ModuleHeader, RoadEditingOwner::Address(_)) => Ordering::Less,
        (RoadEditingOwner::Address(_), RoadEditingOwner::ModuleHeader) => Ordering::Greater,
        (RoadEditingOwner::Address(left), RoadEditingOwner::Address(right)) => {
            compare_address(left, left_context, right, right_context)
        }
    }
}

fn hash_subject<H: Hasher>(
    subject: &RoadEditingSubject,
    context: &RoadEditingLocationContext,
    state: &mut H,
) {
    subject_rank(subject).hash(state);
    match subject {
        RoadEditingSubject::ModuleHeader => {}
        RoadEditingSubject::RoadAlignment { address }
        | RoadEditingSubject::Declaration { address } => hash_address(address, context, state),
        RoadEditingSubject::OwnerLocal {
            owner,
            relation,
            occurrence,
        } => {
            match owner {
                RoadEditingOwner::ModuleHeader => 0_u8.hash(state),
                RoadEditingOwner::Address(address) => {
                    1_u8.hash(state);
                    hash_address(address, context, state);
                }
            }
            relation.hash(state);
            occurrence.hash(state);
        }
        RoadEditingSubject::Wire {
            root_vector,
            physical_index,
            table,
        } => {
            root_vector.hash(state);
            physical_index.hash(state);
            table.hash(state);
        }
    }
}

fn hash_address<H: Hasher>(
    address: &RoadEditingSourceAddress,
    context: &RoadEditingLocationContext,
    state: &mut H,
) {
    address.kind.hash(state);
    address.module_namespace(context).hash(state);
    address.owner_local_key_count.hash(state);
    for owner in address.owner_local_keys(context) {
        owner.hash(state);
    }
    address.local_key(context).hash(state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::hash_map::DefaultHasher;

    fn hash_of(value: &SourceLocation) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn road_editing_location_resolves_shared_ordinals_without_text_span() {
        let context = Arc::new(RoadEditingLocationContext::new(
            Box::new([
                Arc::from("city/main"),
                Arc::from("corridor-main"),
                Arc::from("section-w2e"),
                Arc::from("lane-1"),
            ]),
            Box::new([RoadEditingPropertyPath::new(Box::new([
                RoadEditingPropertyStep::TableField {
                    table: RoadEditingTableKind::AuthoringLane,
                    field_id: 3,
                },
                RoadEditingPropertyStep::StructMember {
                    structure: RoadEditingStructKind::LinearWidthProfile,
                    member_id: 1,
                },
            ]))]),
            Box::new([Arc::from("canvas/lane-1")]),
        ));
        let address = RoadEditingSourceAddress::new(
            context.string_ordinal(0),
            RoadEditingAddressKind::Declaration(EntityKind::AuthoringLane),
            [context.string_ordinal(1), context.string_ordinal(2)],
            context.string_ordinal(3),
        );
        let location = SourceLocation::RoadEditing(RoadEditingSourceLocation::new(
            Arc::clone(&context),
            RoadEditingDocumentIdentity::verified(
                Arc::from("city/main"),
                Arc::from("city-main.lfre"),
            ),
            RoadEditingSubject::Declaration { address },
            Some(context.property_path_ordinal(0)),
            Some(context.canvas_selection_ordinal(0)),
            None,
        ));

        assert_eq!(location.source_document_key(), "city-main.lfre");
        assert!(location.text_span().is_none());
        let road = location.road_editing().expect("road-editing location");
        assert_eq!(road.canvas_selection(), Some("canvas/lane-1"));
        assert_eq!(
            road.property_path().expect("property path").steps().len(),
            2
        );
        let RoadEditingSubject::Declaration { address } = road.subject() else {
            panic!("declaration subject expected");
        };
        assert_eq!(address.module_namespace(road.context()), "city/main");
        assert_eq!(
            address.owner_local_keys(road.context()).collect::<Vec<_>>(),
            ["corridor-main", "section-w2e"]
        );
        assert_eq!(address.local_key(road.context()), "lane-1");
    }

    #[test]
    fn byte_range_must_be_fully_inside_source() {
        assert_eq!(
            RoadEditingByteRange::checked(4, 8, 12),
            Some(RoadEditingByteRange {
                start: 4,
                length: 8
            })
        );
        assert!(RoadEditingByteRange::checked(4, 9, 12).is_none());
        assert!(RoadEditingByteRange::checked(u32::MAX, 2, usize::MAX).is_none());
    }

    #[test]
    fn semantic_order_and_hash_ignore_context_interning_order() {
        fn location(strings: [&str; 4], indexes: [usize; 4]) -> SourceLocation {
            let context = Arc::new(RoadEditingLocationContext::new(
                strings.map(Arc::from).into(),
                Box::default(),
                Box::default(),
            ));
            let address = RoadEditingSourceAddress::new(
                context.string_ordinal(indexes[0]),
                RoadEditingAddressKind::Declaration(EntityKind::AuthoringLane),
                [
                    context.string_ordinal(indexes[1]),
                    context.string_ordinal(indexes[2]),
                ],
                context.string_ordinal(indexes[3]),
            );
            SourceLocation::RoadEditing(RoadEditingSourceLocation::new(
                context,
                RoadEditingDocumentIdentity::verified(
                    Arc::from("city/main"),
                    Arc::from("city-main.lfre"),
                ),
                RoadEditingSubject::Declaration { address },
                None,
                None,
                None,
            ))
        }

        let left = location(
            ["city/main", "corridor-main", "section-w2e", "lane-1"],
            [0, 1, 2, 3],
        );
        let right = location(
            ["lane-1", "section-w2e", "city/main", "corridor-main"],
            [2, 3, 1, 0],
        );

        assert_eq!(left, right);
        assert_eq!(left.cmp(&right), Ordering::Equal);
        assert_eq!(hash_of(&left), hash_of(&right));
    }

    #[test]
    fn source_address_is_copy_and_context_logical_size_is_closed() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<RoadEditingSourceAddress>();

        let context = RoadEditingLocationContext::new(
            Box::new([Arc::from("a")]),
            Box::new([RoadEditingPropertyPath::new(Box::new([
                RoadEditingPropertyStep::TableField {
                    table: RoadEditingTableKind::LaneEdge,
                    field_id: 0,
                },
            ]))]),
            Box::new([Arc::from("canvas")]),
        );
        assert_eq!(
            context.source_map_logical_bytes(),
            12 + (4 + 1) + (4 + 8) + (4 + 6)
        );
    }
}

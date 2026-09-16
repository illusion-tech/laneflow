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

    /// 测试辅助：返回受检模块命名空间的共享引用；输入级身份为 `None`。
    #[cfg(test)]
    pub(crate) fn module_namespace_arc(&self) -> Option<Arc<str>> {
        match self {
            Self::Input(_) => None,
            Self::Verified(identity) => Some(Arc::clone(&identity.module_namespace)),
        }
    }

    /// 构造输入级文档身份（仅携带调用方提供的预期文档键）。
    pub(crate) fn input(expected_source_document_key: Arc<str>) -> Self {
        Self::Input(RoadEditingInputDocumentIdentity {
            expected_source_document_key,
        })
    }

    /// 构造已验证文档身份（模块命名空间与文档键均已受检）。
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
    /// 根向量 `road_alignments`：道路走向定义记录。
    RoadAlignment,
    /// 根向量 `road_corridors`：道路走廊声明。
    RoadCorridor,
    /// 根向量 `road_sections`：道路区段声明。
    RoadSection,
    /// 根向量 `authoring_lanes`：编制车道声明。
    AuthoringLane,
    /// 根向量 `lane_edges`：车道图边声明。
    LaneEdge,
    /// 根向量 `junctions`：路口声明。
    Junction,
    /// 根向量 `movements`：通行流向声明。
    Movement,
    /// 根向量 `maneuver_paths`：机动路径声明。
    ManeuverPath,
    /// 根向量 `maneuver_gates`：机动门声明。
    ManeuverGate,
    /// 根向量 `waiting_zones`：等待区声明。
    WaitingZone,
    /// 根向量 `stop_lines`：停止线声明。
    StopLine,
    /// 根向量 `signal_groups`：信号组声明。
    SignalGroup,
    /// 根向量 `signal_controllers`：信号控制器声明。
    SignalController,
    /// 根向量 `signal_phases`：信号相位声明。
    SignalPhase,
    /// 根向量 `parking_facilities`：停车设施声明。
    ParkingFacility,
    /// 根向量 `parking_spaces`：停车位声明。
    ParkingSpace,
    /// 根向量 `lane_groups`：车道组声明。
    LaneGroup,
    /// 根向量 `facility_bands`：设施带声明。
    FacilityBand,
    /// 根向量 `participant_classes`：参与者类别声明。
    ParticipantClass,
    /// 根向量 `access_rules`：准入规则声明。
    AccessRule,
    /// 根向量 `vehicle_profiles`：车辆配置声明。
    VehicleProfile,
    /// 根向量 `canonical_frames`：规范坐标框架声明。
    CanonicalFrame,
    /// 根向量 `conflict_zones`：冲突区声明。
    ConflictZone,
    /// 根向量 `participant_streams`：参与者流声明。
    ParticipantStream,
    /// 根向量 `conflict_zone_regions`：冲突区空间区域记录。
    ConflictZoneRegion,
    /// 根向量 `right_of_way_policy_sets`：路权策略集声明。
    RightOfWayPolicySet,
}

/// 道路编辑来源地址中的有类型声明种类；道路走向不是 Identity v1 实体。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RoadEditingAddressKind {
    /// 地址指向道路走向定义；有稳定编辑键但不属于 Identity v1。
    RoadAlignment,
    /// 地址指向 Identity v1 稳定实体声明，并携带其种类。
    Declaration(EntityKind),
}

/// 道路编辑 schema 中可出现在来源路径或 wire fallback 的 table 种类。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum RoadEditingTableKind {
    /// 根表 `RoadEditingSource`：格式版本、模块头与全部声明向量的 v4 根。
    RoadEditingSource,
    /// 模块头 `ModuleHeader`：每个逻辑来源模块恰有一个的模块头。
    ModuleHeader,
    /// 溯源 `Provenance`：记录生成方式、构建标识与输入摘要的溯源表。
    Provenance,
    /// 直线段 `LineSegment`：以三维终点表达的编制坐标直线段。
    LineSegment,
    /// 三次 Bézier 段 `CubicBezierSegment`：两个控制点加终点的编制坐标段。
    CubicBezierSegment,
    /// 曲线段 `CurveSegment`：道路走向内 owner-local 的单段曲线几何。
    CurveSegment,
    /// 曲线程序 `CurveProgram`：起点加有序曲线段序列的完整曲线。
    CurveProgram,
    /// 道路走向 `RoadAlignment`：重建当前道路所需的编制描述。
    RoadAlignment,
    /// 走廊成员 `CorridorElement`：横断面中一个有序 owner-local 成员引用。
    CorridorElement,
    /// 道路走廊 `RoadCorridor`：组织方向性道路区段与设施带的横断面所有者。
    RoadCorridor,
    /// 道路区段 `RoadSection`：走廊内具有方向和横断面成员关系的区段。
    RoadSection,
    /// 编制车道 `AuthoringLane`：可展开为车道图边的车道声明。
    AuthoringLane,
    /// 车道图边 `LaneEdge`：显式稳定边键的基础遍历拓扑实体。
    LaneEdge,
    /// 路口 `Junction`：组织通行流向和机动路径的静态路口。
    Junction,
    /// 通行流向 `Movement`：从有向入口臂到有向出口臂的静态通行意图。
    Movement,
    /// 机动路径 `ManeuverPath`：连接入口边、内部边与出口边的可遍历路径。
    ManeuverPath,
    /// 机动门 `ManeuverGate`：绑定机动路径、用于准入与信号约束的静态门。
    ManeuverGate,
    /// 等待区 `WaitingZone`：表达等待容量、顺序或位置的静态区域。
    WaitingZone,
    /// 停止线 `StopLine`：车辆须在其前满足通行约束的静态线。
    StopLine,
    /// 信号组 `SignalGroup`：面向一组门或通行意图输出信号指示的静态组。
    SignalGroup,
    /// 信号控制器 `SignalController`：产生信号相位与指示时间序列的控制程序。
    SignalController,
    /// 信号相位状态 `SignalPhaseState`：相位中某信号组的灯态项。
    SignalPhaseState,
    /// 信号相位 `SignalPhase`：控制器内具有稳定键的阶段声明。
    SignalPhase,
    /// 停车设施 `ParkingFacility`：组织显式停车位与虚拟容量的设施实体。
    ParkingFacility,
    /// 车道停车锚点 `ParkingLaneAnchor`：车道边加里程位置的车道参照。
    ParkingLaneAnchor,
    /// 停车位几何 `ParkingSpaceGeometry`：横向偏移、朝向偏移与外形尺寸。
    ParkingSpaceGeometry,
    /// 停车位 `ParkingSpace`：有排他占用、静态几何和 parked pose 的具体位置。
    ParkingSpace,
    /// 车道组 `LaneGroup`：道路区段内组织车道成员的静态分组。
    LaneGroup,
    /// 设施带 `FacilityBand`：走廊内不承担机动车遍历拓扑的设施横带。
    FacilityBand,
    /// 参与者类别 `ParticipantClass`：可继承的准入分类。
    ParticipantClass,
    /// 准入法规身份 `AccessRegulation`：法域、版本与可选来源的共同值。
    AccessRegulation,
    /// 准入规则 `AccessRule`：对参与者与目标施加允许、拒绝或约束的规则。
    AccessRule,
    /// IIDM 车辆动力学参数 `IidmVehicleProfile`：长度、期望速度与加减速等。
    IidmVehicleProfile,
    /// 车辆配置 `VehicleProfile`：冻结车辆运动与安全参数的静态配置。
    VehicleProfile,
    /// 冲突区空间区域 `ConflictZoneRegion`：与冲突区配对的 XZ 多边形与高度范围。
    ConflictZoneRegion,
    /// 规范坐标框架 `CanonicalFrame`：空间几何和位姿共享的稳定局部坐标框架。
    CanonicalFrame,
    /// 冲突区 `ConflictZone`：多个参与者流可能冲突、需运行时裁决的区域。
    ConflictZone,
    /// 路径锚点 `PathAnchor`：以门、路径边界或边内位置标识机动路径位置。
    PathAnchor,
    /// 冲突通行段 `ConflictPassage`：某参与者流穿过冲突区的 entry/exit 区间。
    ConflictPassage,
    /// 参与者流 `ParticipantStream`：进入冲突裁决的有向参与者流。
    ParticipantStream,
    /// 路权策略集 `RightOfWayPolicySet`：规则、依据与间隙参数的稳定集合。
    RightOfWayPolicySet,
    /// 策略依据 `PolicyEvidence`：定位法规条款的策略证据条目。
    PolicyEvidence,
    /// 间隙参数档 `PolicyGapProfile`：最小领先/滞后间隙与清空缓冲参数。
    PolicyGapProfile,
    /// 通行流路权规则 `PolicyStreamRule`：为参与者流选择优先级与让行目标。
    PolicyStreamRule,
    /// 门合规规则 `PolicyGateRule`：把适用灯态与禁令解释为停止或候选。
    PolicyGateRule,
}

/// 道路编辑 schema 中可出现在来源路径的 inline struct 种类。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum RoadEditingStructKind {
    /// 32-byte SHA-256 摘要 inline struct。
    Digest256,
    /// 可选无符号 64-bit 值 inline struct；缺失表示 `None`。
    OptionalU64,
    /// 编制坐标中的 f64 三维点 inline struct。
    Vec3F64,
    /// 一个 station 区间两端线性宽度的 inline struct。
    LinearWidthProfile,
    /// 编制坐标中的 f64 二维点 inline struct；用于 XZ 平面。
    Vec2F64,
}

/// 道路编辑 schema 中可出现在来源路径的 union 种类。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum RoadEditingUnionKind {
    /// 曲线段几何 union：直线段或三次 Bézier 段。
    CurveSegmentGeometry,
}

/// owner-local 关系的闭合集合。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum RoadEditingRelationKind {
    /// 模块对被导入模块的 owner-local 关系。
    Import,
    /// 道路走向对其曲线段的 owner-local 关系。
    CurveSegment,
    /// 道路走廊对其横断面成员的 owner-local 关系。
    CorridorElement,
    /// 道路区段对其编制车道成员的 owner-local 关系。
    RoadSectionAuthoringLane,
    /// 车道图边对后继边的关系。
    LaneEdgeSuccessor,
    /// 路口对接近边的关系；集合含每条机动路径的入口边与出口边。
    JunctionApproachEdge,
    /// 路口对内部边的关系。
    JunctionInternalEdge,
    /// 机动路径对内部边的关系。
    ManeuverPathInternalEdge,
    /// 信号控制器对信号组的关系。
    SignalControllerGroup,
    /// 信号控制器对信号相位的关系。
    SignalControllerPhase,
    /// 信号相位对相位内灯态项的关系。
    SignalPhaseState,
    /// 准入规则对参与者类别的关系。
    AccessRuleParticipantClass,
    /// 停车设施对虚拟入口的关系。
    ParkingFacilityVirtualEntry,
    /// 停车设施对虚拟出口的关系。
    ParkingFacilityVirtualExit,
    /// 参与者流对冲突通行段的关系。
    ParticipantStreamPassage,
    /// 空间区域记录对所属模块的关系（冲突区仅为记录内引用；ordinal 按模块
    /// 规范根向量序分配）。
    ConflictZoneRegion,
    /// 路权策略集对依据条目的关系。
    PolicyEvidence,
    /// 路权策略集对间隙参数档的关系。
    PolicyGapProfile,
    /// 路权策略集对通行流路权规则的关系。
    PolicyStreamRule,
    /// 路权策略集对门合规规则的关系。
    PolicyGateRule,
}

/// 有序产品关系或规范集合关系中的稳定 occurrence。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RoadEditingRelationOccurrence {
    /// 有序产品关系中的 occurrence；成员在 owner 向量中的零基下标。
    OrderedProductOrdinal(u32),
    /// 规范集合关系中的 occurrence；成员在规范排序中的零基下标。
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

    /// 构造完整来源地址；owner 链按父先子后顺序内联保存。
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

    /// 构造闭合叶属性路径；深度必须为一至四步。
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
    /// 已知 table 的一个 field 步骤。
    TableField {
        /// 步骤所在的 table 种类。
        table: RoadEditingTableKind,
        /// table 内的 field id。
        field_id: u16,
    },
    /// 已知 inline struct 的一个 member 步骤。
    StructMember {
        /// 成员所在的 inline struct 种类。
        structure: RoadEditingStructKind,
        /// struct 内的 member id。
        member_id: u8,
    },
    /// 已知 union 的一个 variant 步骤。
    UnionVariant {
        /// 步骤所在的 union 种类。
        union: RoadEditingUnionKind,
        /// union 内的 variant 判别值。
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

    /// 构造已证明完全位于输入 buffer 内的字节范围；越界时返回 `None`。
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
    /// owner 是该模块唯一的模块头。
    ModuleHeader,
    /// owner 是一个稳定实体地址。
    Address(RoadEditingSourceAddress),
}

/// 道路编辑语义或结构位置的闭合 subject。
#[derive(Clone, Copy, Debug)]
pub enum RoadEditingSubject {
    /// subject 是该模块唯一的模块头。
    ModuleHeader,
    /// subject 是一条道路走向定义。
    RoadAlignment {
        /// 道路走向的稳定实体地址。
        address: RoadEditingSourceAddress,
    },
    /// subject 是一条 Identity v1 稳定实体声明。
    Declaration {
        /// 声明的稳定实体地址。
        address: RoadEditingSourceAddress,
    },
    /// subject 是一次 owner-local 关系出现。
    OwnerLocal {
        /// 拥有该关系的有类型 owner。
        owner: RoadEditingOwner,
        /// owner-local 关系种类。
        relation: RoadEditingRelationKind,
        /// 关系中的稳定 occurrence。
        occurrence: RoadEditingRelationOccurrence,
    },
    /// 结构损坏时按 wire 物理位置定位的 fallback subject。
    Wire {
        /// 物理下标所在的根向量种类。
        root_vector: RoadEditingRootVectorKind,
        /// 根向量内元素的零基物理下标。
        physical_index: u32,
        /// 该元素的 table 种类。
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

    /// 返回已驻留字符串的 ordinal；调用前必须由 location factory 完成驻留。
    pub(crate) fn string_ordinal_for(&self, value: &str) -> RoadEditingStringOrdinal {
        let index = self
            .strings
            .binary_search_by(|candidate| candidate.as_bytes().cmp(value.as_bytes()))
            .expect("location factory interns every address component before freezing");
        self.string_ordinal(index)
    }

    /// 返回已冻结属性路径的 ordinal；调用前必须完成冻结。
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

    /// 返回已驻留画布选择键的 ordinal；调用前必须完成驻留。
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

    /// 由已排序的字符串、属性路径与画布选择键表构造共享 context。
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

    /// 把字符串表下标转换为 ordinal；越界时 panic。
    pub(crate) fn string_ordinal(&self, index: usize) -> RoadEditingStringOrdinal {
        assert!(
            index < self.strings.len(),
            "string ordinal must resolve in context"
        );
        RoadEditingStringOrdinal(u32::try_from(index).expect("compile limits bound ordinals"))
    }

    /// 把属性路径表下标转换为 ordinal；越界时 panic。
    pub(crate) fn property_path_ordinal(&self, index: usize) -> RoadEditingPropertyPathOrdinal {
        assert!(
            index < self.property_paths.len(),
            "property path ordinal must resolve in context"
        );
        RoadEditingPropertyPathOrdinal(u32::try_from(index).expect("compile limits bound ordinals"))
    }

    /// 把画布选择键表下标转换为 ordinal；越界时 panic。
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

    /// 组装一条道路编辑来源位置；ordinal 必须由同一个冻结 context 解析。
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

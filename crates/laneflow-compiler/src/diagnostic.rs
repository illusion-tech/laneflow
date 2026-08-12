//! 与渲染文本解耦的结构化编译诊断。
//!
//! 规范判断依赖 [`DiagnosticCode`]、严重程度、有类型载荷和来源位置，而不是
//! [`Display`](core::fmt::Display) 生成的中文句子。诊断按完整结构值排序；收集器即使
//! 达到保留上限也继续检查安全候选，并最终保留全局规范顺序最小的前缀，避免遍历顺序
//! 改变对外可见结果。任一错误阶段只返回 [`DiagnosticBundle`]，不携带部分阶段输出。

use core::fmt;
use std::sync::Arc;

use laneflow_static_contract::{EntityKind, FieldTag, StableId128};

use crate::declaration::{FacilityKindCategory, FacilityKindViolation, ScalarViolation};
use crate::identity::CanonicalIdentityViolation;
use crate::{CompileLimitDimension, RoadEditingLocationContext, SourceLocation};

/// 来源文档内受检的一基行列位置。
///
/// 合成前端取 Rust 调用位置，后续文本前端可提供真实文本位置。零值不会由当前公共
/// 构造路径产生，但本值本身不负责重新验证范围。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourcePosition {
    line: u32,
    column: u32,
}

impl SourcePosition {
    /// 返回一基行号。
    #[must_use]
    pub const fn line(self) -> u32 {
        self.line
    }

    /// 返回一基列号。
    #[must_use]
    pub const fn column(self) -> u32 {
        self.column
    }
}

/// 与机器路径无关的来源范围。
///
/// `source_document_key` 是来源模块提供的稳定键，不是宿主文件系统路径；范围的起止
/// 位置都采用一基 `u32` 行列。位置服务诊断与源映射，不参与实体身份。
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourceSpan {
    source_document_key: Arc<str>,
    start: SourcePosition,
    end: SourcePosition,
}

impl SourceSpan {
    /// 为包内官方前端建立单点范围；调用者负责传入已验证的文档键与一基位置。
    pub(crate) fn point(source_document_key: Arc<str>, line: u32, column: u32) -> Self {
        let position = SourcePosition { line, column };
        Self {
            source_document_key,
            start: position,
            end: position,
        }
    }

    /// 把合成 DSL 的 Rust 调用点转换为与机器路径无关的来源单点。
    pub(crate) fn at_caller(
        source_document_key: Arc<str>,
        caller: &'static std::panic::Location<'static>,
    ) -> Self {
        Self::point(source_document_key, caller.line(), caller.column())
    }

    /// 返回来源模块内稳定的文档键。
    #[must_use]
    pub fn source_document_key(&self) -> &str {
        &self.source_document_key
    }

    /// 返回包含范围的起始位置。
    #[must_use]
    pub const fn start(&self) -> SourcePosition {
        self.start
    }

    /// 返回包含范围的结束位置；单点范围与 `start` 相同。
    #[must_use]
    pub const fn end(&self) -> SourcePosition {
        self.end
    }

    pub(crate) fn failure_identity_allocation(&self) -> (*const u8, u64) {
        (
            self.source_document_key.as_ptr(),
            crate::source_location::arc_str_requested_bytes(self.source_document_key.len()),
        )
    }
}

/// 稳定诊断代码。
///
/// 对外稳定标识是 [`DiagnosticCode::as_str`] 返回的字符串，不是 Rust 枚举判别值。
/// 枚举为 `non_exhaustive`，调用方必须允许后续版本新增代码。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum DiagnosticCode {
    /// 第一方道路编辑编制模型的字段值或闭合构造非法。
    InvalidRoadEditingInput,
    /// size-prefixed 道路编辑来源的 framing、wire、版本或外部身份绑定非法。
    InvalidRoadEditingSource,
    /// 来源模块头字段违反文本或资源约束。
    InvalidSourceHeaderField,
    /// 导入命名空间不是合法外部 token。
    InvalidImportNamespace,
    /// 同一来源模块重复声明相同导入。
    DuplicateImport,
    /// 编译单元包含两个相同 authoring namespace 的模块。
    DuplicateModuleNamespace,
    /// 编译单元内两个模块或同一模块内两份文档声明相同 `sourceDocumentKey`。
    DuplicateSourceDocumentKey,
    /// 来源位置引用的文档未登记在拥有该语义记录的逻辑模块中。
    SourceDocumentOwnershipMismatch,
    /// 显式导入在完整编译单元中没有目标模块。
    UnknownImport,
    /// 一个或多个显式导入边形成循环。
    ImportCycle,
    /// 声明稳定键不是合法外部 token。
    InvalidDeclarationKey,
    /// 同一模块、同一实体种类重复声明稳定键。
    DuplicateDeclaration,
    /// 引用中显式模块命名空间不是合法外部 token。
    InvalidReferenceNamespace,
    /// 引用的目标声明键不是合法外部 token。
    InvalidReferenceKey,
    /// 跨模块引用没有对应的显式导入。
    UnimportedReferenceModule,
    /// 导入闭合后仍找不到引用的目标声明。
    UnknownReferenceTarget,
    /// Identity v1 的显式 ASCII 领域字段违反来源 token 规则。
    InvalidIdentityAsciiField,
    /// 车道图边长度不是满足当前契约的有限 `f64` 米值。
    InvalidLaneEdgeLength,
    /// 基础道路限速不是严格为正的有限 `f64` 米每秒值。
    InvalidLaneEdgeSpeedLimit,
    /// 同一车道图边重复列出相同下游目标。
    DuplicateLaneEdgeSuccessor,
    /// 道路区段或设施带使用未知或类别不匹配的物理设施 token。
    InvalidFacilityKind,
    /// 道路区段没有声明任何编制车道。
    EmptyRoadSectionLanes,
    /// 编制车道没有声明任何车道图边覆盖。
    EmptyAuthoringLaneEdgeChain,
    /// 同一编制车道覆盖链重复引用同一车道图边。
    DuplicateAuthoringLaneEdge,
    /// 道路走廊没有声明任何横断面成员。
    EmptyRoadCorridorElements,
    /// 同一道路走廊的有序横断面重复引用同一成员。
    DuplicateRoadCorridorElement,
    /// 必须进入横断面所有者树的实体没有任何道路走廊父项。
    MissingCrossSectionOwner,
    /// 横断面实体被多个道路走廊拥有。
    MultipleCrossSectionOwners,
    /// 道路走廊的参考道路区段不属于自身有序成员。
    InvalidCorridorReferenceSection,
    /// 编制车道覆盖链中的相邻车道图边没有直接连接。
    DisconnectedAuthoringLaneEdgeChain,
    /// 同一车道图边被多个编制车道覆盖。
    MultipleAuthoringLaneOwners,
    /// 编制车道引用的车道组不属于同一道路区段。
    LaneGroupParentMismatch,
    /// 车道组没有任何编制车道成员。
    EmptyLaneGroup,
    /// 路口没有任何通行流向成员。
    EmptyJunction,
    /// 通行流向没有任何机动路径成员。
    EmptyMovement,
    /// 机动路径完整边序列中的相邻边没有直接连接。
    DisconnectedManeuverPath,
    /// 两条机动路径声明了相同的完整遍历序列。
    DuplicateManeuverPathSequence,
    /// 同一内部边被不同路口排他声明。
    InternalEdgeJunctionConflict,
    /// 同一边同时被声明为路口内部边和任一路口的边界边。
    InternalBoundaryRoleConflict,
    /// 路口显式 approach/internal 集与路径角色或 section-derived 边界不闭合。
    JunctionEdgeSetMismatch,
    /// 机动门引用的转换下标不在拥有路径的合法范围内。
    ManeuverGateTransitionOutOfRange,
    /// 同一机动路径转换重复声明机动门。
    DuplicateManeuverGatePathTransition,
    /// 机动门停止线不位于转换的起始边末端。
    ManeuverGateStopLineMismatch,
    /// 同一车道图边重复声明停止线。
    DuplicateStopLineEdge,
    /// 停止线位于无法形成路径转换的终止边。
    OrphanStopLine,
    /// 非终止边上的停止线未被任何机动门引用。
    UnreferencedStopLine,
    /// 启用入口门的停止线存在没有机动路径覆盖的下游转换。
    MissingManeuverPathCoverage,
    /// 启用入口门的停止线存在没有入口机动门覆盖的候选路径。
    MissingManeuverGateCoverage,
    /// 等待区容量为零。
    InvalidWaitingZoneCapacity,
    /// 等待区引用的入口门或释放门不属于其声明路径。
    WaitingZoneGatePathMismatch,
    /// 等待区入口门没有严格早于释放门。
    InvalidWaitingZoneGateOrder,
    /// 同一机动路径上的两个等待区内部重叠或嵌套。
    OverlappingWaitingZones,
    /// 静态路线没有任何车道图边出现项。
    EmptyStaticRoute,
    /// 静态路线中的一对相邻车道图边没有直接连接。
    DisconnectedStaticRouteEdge,
    /// 静态路线从路口内部边开始。
    StaticRouteStartsInsideJunction,
    /// 静态路线在路口内部边结束。
    StaticRouteEndsInsideJunction,
    /// 静态路线的最终边带有停止线，因而遗漏了受控后继转换。
    StaticRouteTerminatesAtStopLine,
    /// 静态路线进入了已知机动路径，但没有包含其完整边序列。
    StaticRouteManeuverNoFullMatch,
    /// 静态路线同一入口位置完整匹配多条机动路径。
    StaticRouteManeuverMultipleFullMatches,
    /// 静态路线中的两个机动路径出现项覆盖了同一内部边出现项。
    StaticRouteManeuverInternalOverlap,
    /// 静态路线中的路口内部边没有被任何完整机动路径出现项覆盖。
    StaticRouteInternalEdgeUncovered,
    /// 信号控制器没有任何信号组成员。
    EmptySignalControllerGroups,
    /// 信号控制器没有任何程序相位。
    EmptySignalControllerPhases,
    /// 信号控制器重复列出同一信号组。
    DuplicateSignalControllerGroup,
    /// 同一信号组被多个控制器拥有。
    SignalGroupMultipleControllers,
    /// 信号组没有控制器所有者。
    UnownedSignalGroup,
    /// 信号组没有被任何机动门使用。
    UnusedSignalGroup,
    /// 同一控制器内重复声明相位键。
    DuplicateSignalPhaseKey,
    /// 信号相位持续时间不在可移植正整数范围内。
    InvalidSignalPhaseDuration,
    /// 相位重复定义同一信号组状态。
    DuplicateSignalPhaseGroup,
    /// 相位状态引用不属于所属控制器的信号组。
    UnknownSignalPhaseGroup,
    /// 相位缺少所属控制器的信号组状态。
    MissingSignalPhaseGroup,
    /// 控制器相位周期累计值超过可移植范围。
    SignalCycleDurationOverflow,
    /// 控制器时间偏移不在可移植且小于周期的规范范围内。
    InvalidSignalControllerOffset,
    /// 停车位入口或出口锚点不位于车道图边的严格内部。
    InvalidParkingAnchorProgress,
    /// 停车位矩形几何字段违反有限性、范围或最小尺寸约束。
    InvalidParkingSpaceGeometry,
    /// 停车区域没有任何停车位成员。
    OrphanParkingArea,
    /// 参与者类别的单继承链形成循环。
    ParticipantClassInheritanceCycle,
    /// 车辆配置的 IIDM 数值违反 current Core 约束。
    InvalidVehicleProfileValue,
    /// 车辆配置的紧急减速度小于舒适减速度。
    InvalidVehicleProfileDecelerationOrder,
    /// 规范空间几何违反点、线段、长度绑定或覆盖完整性约束。
    InvalidSpatialGeometry,
    /// 不可遍历设施带的规范中心线违反点、长度或 frame 约束。
    InvalidFacilityBandGeometry,
    /// 准入规则没有声明任何参与者类别。
    EmptyAccessRuleParticipantClasses,
    /// 准入规则请求了首版尚未实现的能力。
    AccessCapabilityUnavailable,
    /// 准入规则的法规来源字段违反长度约束。
    InvalidAccessRegulationString,
    /// 同一编译单元中的法规来源法域或版本不一致。
    AccessRegulationMismatch,
    /// 规范裁决后仍存在效果相反且完全并列的准入规则。
    AccessRuleAmbiguity,
    /// 编译器构造的规范身份字段不满足 Identity v1 登记表。
    InvalidCanonicalIdentity,
    /// 同一完整规范身份在编译单元中出现多次。
    DuplicateCanonicalIdentity,
    /// 不同完整规范身份派生出相同 StableId128。
    IdentityDigestCollision,
    /// 选择的编译资源配置档不支持候选官方模块必需的维度。
    CompileProfileIncompatible,
    /// 候选输入或阶段工作集超过显式编译资源配置档。
    CompileLimitExceeded,
}

impl DiagnosticCode {
    /// 返回跨渲染语言稳定的代码字符串。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRoadEditingInput => "LF-COMP-ROAD-EDITING-INPUT",
            Self::InvalidRoadEditingSource => "LF-COMP-ROAD-EDITING-SOURCE",
            Self::InvalidSourceHeaderField => "LF-COMP-SOURCE-HEADER-FIELD",
            Self::InvalidImportNamespace => "LF-COMP-IMPORT-NAMESPACE",
            Self::DuplicateImport => "LF-COMP-DUPLICATE-IMPORT",
            Self::DuplicateModuleNamespace => "LF-COMP-DUPLICATE-MODULE-NAMESPACE",
            Self::DuplicateSourceDocumentKey => "LF-COMP-DUPLICATE-SOURCE-DOCUMENT-KEY",
            Self::SourceDocumentOwnershipMismatch => "LF-COMP-SOURCE-DOCUMENT-OWNERSHIP-MISMATCH",
            Self::UnknownImport => "LF-COMP-UNKNOWN-IMPORT",
            Self::ImportCycle => "LF-COMP-IMPORT-CYCLE",
            Self::InvalidDeclarationKey => "LF-COMP-DECLARATION-KEY",
            Self::DuplicateDeclaration => "LF-COMP-DUPLICATE-DECLARATION",
            Self::InvalidReferenceNamespace => "LF-COMP-REFERENCE-NAMESPACE",
            Self::InvalidReferenceKey => "LF-COMP-REFERENCE-KEY",
            Self::UnimportedReferenceModule => "LF-COMP-UNIMPORTED-REFERENCE-MODULE",
            Self::UnknownReferenceTarget => "LF-COMP-UNKNOWN-REFERENCE-TARGET",
            Self::InvalidIdentityAsciiField => "LF-COMP-IDENTITY-ASCII-FIELD",
            Self::InvalidLaneEdgeLength => "LF-COMP-LANE-EDGE-LENGTH",
            Self::InvalidLaneEdgeSpeedLimit => "LF-COMP-LANE-EDGE-SPEED-LIMIT",
            Self::DuplicateLaneEdgeSuccessor => "LF-COMP-DUPLICATE-LANE-EDGE-SUCCESSOR",
            Self::InvalidFacilityKind => "LF-COMP-FACILITY-KIND",
            Self::EmptyRoadSectionLanes => "LF-COMP-EMPTY-ROAD-SECTION-LANES",
            Self::EmptyAuthoringLaneEdgeChain => "LF-COMP-EMPTY-AUTHORING-LANE-EDGE-CHAIN",
            Self::DuplicateAuthoringLaneEdge => "LF-COMP-DUPLICATE-AUTHORING-LANE-EDGE",
            Self::EmptyRoadCorridorElements => "LF-COMP-EMPTY-ROAD-CORRIDOR-ELEMENTS",
            Self::DuplicateRoadCorridorElement => "LF-COMP-DUPLICATE-ROAD-CORRIDOR-ELEMENT",
            Self::MissingCrossSectionOwner => "LF-COMP-MISSING-CROSS-SECTION-OWNER",
            Self::MultipleCrossSectionOwners => "LF-COMP-MULTIPLE-CROSS-SECTION-OWNERS",
            Self::InvalidCorridorReferenceSection => "LF-COMP-CORRIDOR-REFERENCE-SECTION",
            Self::DisconnectedAuthoringLaneEdgeChain => {
                "LF-COMP-DISCONNECTED-AUTHORING-LANE-EDGE-CHAIN"
            }
            Self::MultipleAuthoringLaneOwners => "LF-COMP-MULTIPLE-AUTHORING-LANE-OWNERS",
            Self::LaneGroupParentMismatch => "LF-COMP-LANE-GROUP-PARENT-MISMATCH",
            Self::EmptyLaneGroup => "LF-COMP-EMPTY-LANE-GROUP",
            Self::EmptyJunction => "LF-COMP-EMPTY-JUNCTION",
            Self::EmptyMovement => "LF-COMP-EMPTY-MOVEMENT",
            Self::DisconnectedManeuverPath => "LF-COMP-DISCONNECTED-MANEUVER-PATH",
            Self::DuplicateManeuverPathSequence => "LF-COMP-DUPLICATE-MANEUVER-PATH-SEQUENCE",
            Self::InternalEdgeJunctionConflict => "LF-COMP-INTERNAL-EDGE-JUNCTION-CONFLICT",
            Self::InternalBoundaryRoleConflict => "LF-COMP-INTERNAL-BOUNDARY-ROLE-CONFLICT",
            Self::JunctionEdgeSetMismatch => "LF-COMP-JUNCTION-EDGE-SET-MISMATCH",
            Self::ManeuverGateTransitionOutOfRange => {
                "LF-COMP-MANEUVER-GATE-TRANSITION-OUT-OF-RANGE"
            }
            Self::DuplicateManeuverGatePathTransition => {
                "LF-COMP-DUPLICATE-MANEUVER-GATE-PATH-TRANSITION"
            }
            Self::ManeuverGateStopLineMismatch => "LF-COMP-MANEUVER-GATE-STOP-LINE-MISMATCH",
            Self::DuplicateStopLineEdge => "LF-COMP-DUPLICATE-STOP-LINE-EDGE",
            Self::OrphanStopLine => "LF-COMP-ORPHAN-STOP-LINE",
            Self::UnreferencedStopLine => "LF-COMP-UNREFERENCED-STOP-LINE",
            Self::MissingManeuverPathCoverage => "LF-COMP-MISSING-MANEUVER-PATH-COVERAGE",
            Self::MissingManeuverGateCoverage => "LF-COMP-MISSING-MANEUVER-GATE-COVERAGE",
            Self::InvalidWaitingZoneCapacity => "LF-COMP-WAITING-ZONE-CAPACITY",
            Self::WaitingZoneGatePathMismatch => "LF-COMP-WAITING-ZONE-GATE-PATH-MISMATCH",
            Self::InvalidWaitingZoneGateOrder => "LF-COMP-WAITING-ZONE-GATE-ORDER",
            Self::OverlappingWaitingZones => "LF-COMP-OVERLAPPING-WAITING-ZONES",
            Self::EmptyStaticRoute => "LF-COMP-EMPTY-STATIC-ROUTE",
            Self::DisconnectedStaticRouteEdge => "LF-COMP-DISCONNECTED-STATIC-ROUTE-EDGE",
            Self::StaticRouteStartsInsideJunction => "LF-COMP-STATIC-ROUTE-STARTS-INSIDE-JUNCTION",
            Self::StaticRouteEndsInsideJunction => "LF-COMP-STATIC-ROUTE-ENDS-INSIDE-JUNCTION",
            Self::StaticRouteTerminatesAtStopLine => "LF-COMP-STATIC-ROUTE-TERMINATES-AT-STOP-LINE",
            Self::StaticRouteManeuverNoFullMatch => "LF-COMP-STATIC-ROUTE-MANEUVER-NO-FULL-MATCH",
            Self::StaticRouteManeuverMultipleFullMatches => {
                "LF-COMP-STATIC-ROUTE-MANEUVER-MULTIPLE-FULL-MATCHES"
            }
            Self::StaticRouteManeuverInternalOverlap => {
                "LF-COMP-STATIC-ROUTE-MANEUVER-INTERNAL-OVERLAP"
            }
            Self::StaticRouteInternalEdgeUncovered => {
                "LF-COMP-STATIC-ROUTE-INTERNAL-EDGE-UNCOVERED"
            }
            Self::EmptySignalControllerGroups => "LF-COMP-EMPTY-SIGNAL-CONTROLLER-GROUPS",
            Self::EmptySignalControllerPhases => "LF-COMP-EMPTY-SIGNAL-CONTROLLER-PHASES",
            Self::DuplicateSignalControllerGroup => "LF-COMP-DUPLICATE-SIGNAL-CONTROLLER-GROUP",
            Self::SignalGroupMultipleControllers => "LF-COMP-SIGNAL-GROUP-MULTIPLE-CONTROLLERS",
            Self::UnownedSignalGroup => "LF-COMP-UNOWNED-SIGNAL-GROUP",
            Self::UnusedSignalGroup => "LF-COMP-UNUSED-SIGNAL-GROUP",
            Self::DuplicateSignalPhaseKey => "LF-COMP-DUPLICATE-SIGNAL-PHASE-KEY",
            Self::InvalidSignalPhaseDuration => "LF-COMP-SIGNAL-PHASE-DURATION",
            Self::DuplicateSignalPhaseGroup => "LF-COMP-DUPLICATE-SIGNAL-PHASE-GROUP",
            Self::UnknownSignalPhaseGroup => "LF-COMP-UNKNOWN-SIGNAL-PHASE-GROUP",
            Self::MissingSignalPhaseGroup => "LF-COMP-MISSING-SIGNAL-PHASE-GROUP",
            Self::SignalCycleDurationOverflow => "LF-COMP-SIGNAL-CYCLE-DURATION-OVERFLOW",
            Self::InvalidSignalControllerOffset => "LF-COMP-SIGNAL-CONTROLLER-OFFSET",
            Self::InvalidParkingAnchorProgress => "LF-COMP-PARKING-ANCHOR-PROGRESS",
            Self::InvalidParkingSpaceGeometry => "LF-COMP-PARKING-SPACE-GEOMETRY",
            Self::OrphanParkingArea => "LF-COMP-ORPHAN-PARKING-AREA",
            Self::ParticipantClassInheritanceCycle => "LF-COMP-PARTICIPANT-CLASS-CYCLE",
            Self::InvalidVehicleProfileValue => "LF-COMP-VEHICLE-PROFILE-VALUE",
            Self::InvalidVehicleProfileDecelerationOrder => {
                "LF-COMP-VEHICLE-PROFILE-DECELERATION-ORDER"
            }
            Self::InvalidSpatialGeometry => "LF-COMP-SPATIAL-GEOMETRY",
            Self::InvalidFacilityBandGeometry => "LF-COMP-FACILITY-BAND-GEOMETRY",
            Self::EmptyAccessRuleParticipantClasses => "LF-COMP-EMPTY-ACCESS-RULE-CLASSES",
            Self::AccessCapabilityUnavailable => "LF-COMP-ACCESS-CAPABILITY-UNAVAILABLE",
            Self::InvalidAccessRegulationString => "LF-COMP-ACCESS-REGULATION-STRING",
            Self::AccessRegulationMismatch => "LF-COMP-ACCESS-REGULATION-MISMATCH",
            Self::AccessRuleAmbiguity => "LF-COMP-ACCESS-RULE-AMBIGUITY",
            Self::InvalidCanonicalIdentity => "LF-COMP-INVALID-CANONICAL-IDENTITY",
            Self::DuplicateCanonicalIdentity => "LF-COMP-DUPLICATE-CANONICAL-IDENTITY",
            Self::IdentityDigestCollision => "LF-COMP-IDENTITY-DIGEST-COLLISION",
            Self::CompileProfileIncompatible => "LF-COMP-PROFILE-INCOMPATIBLE",
            Self::CompileLimitExceeded => "LF-COMP-RESOURCE-LIMIT",
        }
    }
}

/// 第一方道路编辑编制模型拒绝字段值的结构化原因。
///
/// 该诊断发生在来源 buffer 尚未建立时，因此不伪造 wire offset 或文本行列。字段路径由
/// [`DiagnosticPayload::InvalidRoadEditingInput`] 单独携带。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum RoadEditingInputViolation {
    /// token、显示键或来源文本违反统一文本规则。
    InvalidText(SourceTextViolation),
    /// owner-qualified 引用的 key component 数量与目标种类不一致。
    InvalidReferenceDepth { expected: u8, actual: u8 },
    /// 语义要求非空的集合没有成员。
    EmptyCollection,
    /// 唯一集合或所有者向量中出现重复值。
    DuplicateValue,
    /// 浮点字段是 NaN 或正负无穷。
    NonFinite { value_bits: u64 },
    /// 浮点字段没有严格大于零。
    NotGreaterThanZero { value_bits: u64 },
    /// 浮点字段小于零。
    LessThanZero { value_bits: u64 },
    /// 浮点字段落在闭合规范范围之外。
    OutsideInclusiveRange {
        value_bits: u64,
        minimum_bits: u64,
        maximum_bits: u64,
    },
    /// 多个字段的组合违反闭合 variant 规则。
    InvalidCombination,
}

/// 道路编辑 authoring 几何在确定性 numeric freeze 中失败的闭合原因。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum RoadEditingNumericViolation {
    /// 中间标量或向量结果不是有限值。
    NonFinite,
    /// 冻结运算图命中零除数。
    DivisionByZero,
    /// 平方根输入落在定义域外。
    SquareRootDomain,
    /// source curve 的水平导数在一个确定点为零。
    HorizontalDerivativeZero,
    /// 有界 regularity walk 无法证明整段水平导数非零。
    HorizontalDerivativeNotProvenNonZero,
    /// 量化前坐标超出 canonical `f32` 领域。
    CoordinateOutOfRange,
    /// 深度、方向或误差门槛内无法形成接受的规范折线。
    ApproximationNotConverged,
    /// corridor station 不在所选 alignment 区间内。
    StationOutOfRange,
    /// corridor、section、lane、edge 或 facility 的几何所有权不能闭合。
    GeometryTopologyMismatch,
    /// 同一 source curve 相邻 offset 段的端点间隙超过 canonical weld 门槛。
    SourceJoinGapExceeded,
    /// 规范折线包含退化段或不能形成正长度。
    DegenerateCanonicalSegment,
    /// 规范弦或跨边连接超过所选方向档。
    DirectionDiscontinuity,
}
/// size-prefixed 道路编辑来源在受检 reader 边界的闭合失败类别。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum RoadEditingSourceViolation {
    /// buffer 小于读取 size prefix、root offset 与 `LFRE` 所需的最小长度。
    TruncatedFraming,
    /// 四字节 size prefix 与实际尾部长度不完全相等。
    SizePrefixMismatch { declared: u64, actual: u64 },
    /// size-prefixed FlatBuffer 不带精确 `LFRE` file identifier。
    FileIdentifierMismatch,
    /// verifier 发现一般结构损坏、UTF-8、required field 或 union 不一致。
    MalformedWire,
    /// verifier 命中固定 schema 深度上限，或上限公式无法表示。
    VerifierDepthExceeded,
    /// verifier 命中固定 16 倍 apparent-size 上限，或上限公式无法表示。
    VerifierApparentSizeExceeded,
    /// wire table 数超过调用点剩余 Typed AST record 预算。
    VerifierTableBudgetExceeded,
    /// reader 只接受 exact v1 语义。
    UnsupportedFormatVersion { expected: u32, actual: u32 },
    /// verified wire 内的 source-document key 与 wire 外 expected key 不同。
    SourceDocumentKeyMismatch,
    /// verifier 后的字段值违反与第一方 authoring model 共用的闭合语义规则。
    InvalidSemanticValue(RoadEditingInputViolation),
    /// authoring 曲线与 offset 几何不能按冻结数值契约形成规范折线。
    NumericFreeze(RoadEditingNumericViolation),
}

/// 路口显式边集合与路径角色闭包不一致的原因。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum JunctionEdgeSetViolation {
    ApproachNotSectionDerived,
    InternalIsSectionDerived,
    BoundaryNotDeclaredApproach,
    InternalNotDeclared,
    DeclaredInternalUnused,
    ApproachClaimedInternal,
}

/// 诊断严重程度。数值顺序同时是规范排序顺序。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
#[non_exhaustive]
pub enum DiagnosticSeverity {
    /// 当前阶段不能提交输出。
    Error = 1,
    /// 输出仍可提交，但调用方应向作者展示的问题。
    Warning = 2,
    /// 不改变成功与否的补充说明。
    Note = 3,
}

/// 来源模块头中由调用方提供的字段。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum SourceHeaderField {
    /// 声明身份使用的 authoring namespace。
    AuthoringNamespaceId,
    /// 与机器路径无关的来源文档键。
    SourceDocumentKey,
    /// 生成器构建标识。
    GeneratorBuildId,
    /// 来源沿袭展示文本。
    Provenance,
}

impl SourceHeaderField {
    /// 返回诊断载荷使用的稳定 lowerCamelCase 字段名。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AuthoringNamespaceId => "authoringNamespaceId",
            Self::SourceDocumentKey => "sourceDocumentKey",
            Self::GeneratorBuildId => "generatorBuildId",
            Self::Provenance => "provenance",
        }
    }
}

/// 等待区诊断中发生约束失败的门角色。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum WaitingZoneGateRole {
    /// 界定等待区起点的入口门。
    Entry,
    /// 界定等待区终点的释放门。
    Release,
}

/// 停车锚点诊断中发生约束失败的连接角色。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum ParkingAnchorRole {
    /// 驶入并提交停车动作前到达的入口锚点。
    Entry,
    /// 离开停车位后重新接入车道图的出口锚点。
    Exit,
}

impl ParkingAnchorRole {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Entry => "entry",
            Self::Exit => "exit",
        }
    }
}

/// 停车位矩形几何诊断中的受检字段。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum ParkingGeometryField {
    /// 相对入口边中心线的横向偏移，单位为米。
    LateralOffsetMeters,
    /// 相对入口边正向切线的朝向偏移，单位为弧度。
    HeadingOffsetRadians,
    /// 沿停车朝向的泊位长度，单位为米。
    LengthMeters,
    /// 垂直停车朝向的泊位宽度，单位为米。
    WidthMeters,
}

impl ParkingGeometryField {
    const fn as_str(self) -> &'static str {
        match self {
            Self::LateralOffsetMeters => "lateralOffsetMeters",
            Self::HeadingOffsetRadians => "headingOffsetRadians",
            Self::LengthMeters => "lengthMeters",
            Self::WidthMeters => "widthMeters",
        }
    }
}

/// 停车位矩形几何字段的结构化失败原因。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum ParkingGeometryViolation {
    /// 输入为 NaN 或正负无穷。
    NotFinite,
    /// 输入的绝对值没有严格大于排他下限。
    AbsoluteNotGreaterThan {
        /// 排他下限的 IEEE 754 位模式。
        exclusive_minimum_bits: u64,
    },
    /// 输入没有严格大于排他下限。
    NotGreaterThan {
        /// 排他下限的 IEEE 754 位模式。
        exclusive_minimum_bits: u64,
    },
    /// 输入不在包含下界、排除上界的半开区间内。
    OutsideHalfOpenRange {
        /// 包含下界的 IEEE 754 位模式。
        minimum_inclusive_bits: u64,
        /// 排他上界的 IEEE 754 位模式。
        maximum_exclusive_bits: u64,
    },
}

/// 规范空间点诊断中使用的坐标轴。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum SpatialAxis {
    X,
    Y,
    Z,
}

impl SpatialAxis {
    const fn as_str(self) -> &'static str {
        match self {
            Self::X => "x",
            Self::Y => "y",
            Self::Z => "z",
        }
    }
}

/// 规范空间几何的结构化失败原因。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum SpatialGeometryViolation {
    InsufficientPoints {
        minimum: u32,
        actual: u32,
    },
    NonFiniteCoordinate {
        point_index: u32,
        axis: SpatialAxis,
        value_bits: u32,
    },
    CoordinateOutOfRange {
        point_index: u32,
        axis: SpatialAxis,
        value_bits: u32,
        minimum_bits: u32,
        maximum_bits: u32,
    },
    DuplicateEdgeBinding,
    MissingEdgeBinding,
    /// 已编译 authoring 几何没有携带产生其点表的配置档。
    MissingGeometryProfiles,
    /// 同一编译单元内两个已编译 authoring 模块使用了不同配置档。
    GeometryProfileMismatch {
        expected_accuracy_code: u8,
        expected_direction_code: u8,
        actual_accuracy_code: u8,
        actual_direction_code: u8,
    },
    /// 已编译点表既没有显式 frame，也不能从合法机动路径推导 frame。
    MissingCanonicalFrame,
    /// 同一机动路径的 entry 与 exit 没有解析到同一 frame。
    ManeuverPathFrameMismatch,
    /// 共享 internal edge 从不同机动路径推导出冲突 frame。
    InternalEdgeFrameConflict,
    DegenerateSegment {
        segment_index: u32,
        length_bits: u32,
        minimum_bits: u32,
    },
    DegenerateProjectedUp {
        segment_index: u32,
        projected_up_bits: u32,
        minimum_bits: u32,
    },
    ArcLengthAccumulationFailed {
        segment_index: u32,
        accumulated_bits: u32,
        segment_length_bits: u32,
    },
    LengthMismatch {
        expected_length_bits: u64,
        geometry_length_bits: u32,
        tolerance_bits: u64,
    },
    ConnectedEdgesUseDifferentFrames,
    DiscontinuousJoin {
        distance_bits: u64,
        tolerance_bits: u64,
    },
    /// 相连 edge 最终 `f32` 首尾弦超过所选方向档。
    DirectionDiscontinuity {
        dot_bits: u64,
        lhs_bits: u64,
        rhs_bits: u64,
    },
}

impl WaitingZoneGateRole {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Entry => "entryGate",
            Self::Release => "releaseGate",
        }
    }
}

/// 来源文本字段的有类型失败原因。
///
/// 所有位置与长度都按 UTF-8 原始字节计；ASCII 校验失败后不会把字符索引误报为字节
/// 索引。枚举保持结构化数据，显示文本不是机器契约。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum SourceTextViolation {
    /// 必填字段为空。
    Empty,
    /// UTF-8 字节数超过所选资源配置档的单字符串上限。
    TooLong { limit: u64, observed: u64 },
    /// 指定零基字节位置不是 ASCII。
    NonAscii { byte_index: u64 },
    /// token 首字节不是 ASCII 字母或数字。
    InvalidFirstByte { byte: u8 },
    /// token 在指定零基位置包含不在允许集合内的 ASCII 字节。
    InvalidTokenByte { byte_index: u64, byte: u8 },
    /// 可见文本包含控制字节；空格不属于此错误。
    ControlByte { byte_index: u64, byte: u8 },
    /// 来源键包含为限定引用保留的 `::` 分隔符。
    ReservedDelimiter { byte_index: u64 },
}

/// 首版静态准入编译明确拒绝的能力。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum AccessCapability {
    /// 以 `FacilityBand` 作为准入目标。
    FacilityBandTarget,
    /// 带时段窗口的动态准入规则。
    TimeWindows,
}

/// 准入规则组合裁决所在的独立平面。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum AccessPlane {
    /// 车道图边以及展开到边的车道组/道路区段规则。
    Edge,
    /// 保持机动路径身份、不展平为边的规则。
    ManeuverPath,
}

/// 法规来源中受长度约束的字段。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum AccessRegulationField {
    /// 法域。
    Jurisdiction,
    /// 法规版本。
    Version,
    /// 可选来源说明。
    Source,
}

/// 诊断的有类型载荷。
///
/// 载荷保留复现和机器判断所需的原始结构，例如计数维度、目标命名空间与浮点位模式；
/// 调用方不应解析 [`Diagnostic`] 的显示字符串来恢复这些信息。
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum DiagnosticPayload {
    /// 第一方道路编辑编制模型中的字段级失败。
    InvalidRoadEditingInput {
        field: Box<str>,
        violation: RoadEditingInputViolation,
    },
    /// 道路编辑来源 reader 的 framing、wire、版本或外部身份绑定失败。
    InvalidRoadEditingSource {
        violation: RoadEditingSourceViolation,
        field: Option<Box<str>>,
        expected_source_document_key: Box<str>,
        actual_source_document_key: Option<Box<str>>,
    },
    /// 模块头字段及其文本失败原因。
    InvalidSourceHeaderField {
        field: SourceHeaderField,
        violation: SourceTextViolation,
    },
    /// 超限维度、配置值与候选观测值。
    CompileLimitExceeded {
        dimension: CompileLimitDimension,
        limit: u64,
        observed: u64,
    },
    /// 缺少必需维度的配置档标识与维度。
    CompileProfileIncompatible {
        profile_id: Box<str>,
        required_dimension: CompileLimitDimension,
    },
    /// 导入命名空间的文本失败原因。
    InvalidImportNamespace {
        /// 命名空间违反的精确文本规则。
        violation: SourceTextViolation,
    },
    /// 重复导入的规范命名空间。
    DuplicateImport {
        /// 在同一来源模块内第二次出现的命名空间。
        namespace: Box<str>,
    },
    /// 重复来源模块的 authoring namespace。
    DuplicateModuleNamespace {
        /// 在编译单元内发生冲突的 authoring namespace。
        namespace: Box<str>,
    },
    /// 在编译单元内不能唯一定位来源位置的重复文档键。
    DuplicateSourceDocumentKey {
        /// 两个模块或同一模块内两份文档共同声明的 `sourceDocumentKey`。
        source_document_key: Box<str>,
    },
    /// 来源位置文档缺失或属于另一个逻辑模块。
    SourceDocumentOwnershipMismatch {
        source_document_key: Box<str>,
        expected_authoring_namespace_id: Box<str>,
        actual_authoring_namespace_id: Option<Box<str>>,
    },
    /// 在编译单元中没有目标模块的导入命名空间。
    UnknownImport {
        /// 没有对应来源模块的目标命名空间。
        namespace: Box<str>,
    },
    /// 一条规范选择的导入循环；顺序用于稳定展示见证，不代表全部可能回路。
    ImportCycle {
        /// 按规范选择的循环模块序列；首项不会在末尾重复。
        namespaces: Box<[Box<str>]>,
    },
    /// 非法声明稳定键所属实体种类及失败原因。
    InvalidDeclarationKey {
        entity_kind: EntityKind,
        violation: SourceTextViolation,
    },
    /// 模块内发生冲突的实体种类与稳定键。
    DuplicateDeclaration {
        entity_kind: EntityKind,
        stable_key: Box<str>,
    },
    /// 引用模块命名空间的文本失败原因。
    InvalidReferenceNamespace {
        /// 显式目标命名空间违反的精确文本规则。
        violation: SourceTextViolation,
    },
    /// 非法目标键所属实体种类及失败原因。
    InvalidReferenceKey {
        entity_kind: EntityKind,
        violation: SourceTextViolation,
    },
    /// 未经显式导入就被引用的模块命名空间。
    UnimportedReferenceModule {
        /// 被引用但未出现在当前模块导入集合中的命名空间。
        namespace: Box<str>,
    },
    /// 来源声明及无法解析的完整目标二元组。
    UnknownReferenceTarget {
        entity_kind: EntityKind,
        source_key: Box<str>,
        target_namespace: Box<str>,
        target_owner_local_keys: Box<[Box<str>]>,
        target_key: Box<str>,
    },
    /// 显式 Identity v1 ASCII 字段及其文本失败原因。
    InvalidIdentityAsciiField {
        entity_kind: EntityKind,
        stable_key: Box<str>,
        field_tag: FieldTag,
        violation: SourceTextViolation,
    },
    /// 车道图边稳定键、非法长度位模式及数值约束原因。
    InvalidLaneEdgeLength {
        stable_key: Box<str>,
        /// 非法 `f64` 的原始 IEEE 754 位模式，避免 NaN 与格式化差异破坏确定性。
        value_bits: u64,
        violation: ScalarViolation,
    },
    /// 车道图边稳定键、非法限速位模式及数值约束原因。
    InvalidLaneEdgeSpeedLimit {
        stable_key: Box<str>,
        /// 非法 `f64` 的原始 IEEE 754 位模式。
        value_bits: u64,
        violation: ScalarViolation,
    },
    /// 来源车道图边与重复的完整目标二元组。
    DuplicateLaneEdgeSuccessor {
        stable_key: Box<str>,
        target_namespace: Box<str>,
        target_key: Box<str>,
    },
    /// 未知或不能由指定横断面实体承载的物理设施类别。
    InvalidFacilityKind {
        entity_kind: EntityKind,
        stable_key: Box<str>,
        kind_id: Box<str>,
        expected_category: FacilityKindCategory,
        violation: FacilityKindViolation,
    },
    /// 没有编制车道的道路区段。
    EmptyRoadSectionLanes {
        stable_key: Box<str>,
    },
    /// 没有车道图边覆盖的编制车道。
    EmptyAuthoringLaneEdgeChain {
        stable_key: Box<str>,
    },
    /// 编制车道覆盖链中的重复车道图边引用。
    DuplicateAuthoringLaneEdge {
        stable_key: Box<str>,
        target_namespace: Box<str>,
        target_key: Box<str>,
    },
    /// 没有横断面成员的道路走廊。
    EmptyRoadCorridorElements {
        stable_key: Box<str>,
    },
    /// 道路走廊有序横断面中的重复成员引用。
    DuplicateRoadCorridorElement {
        stable_key: Box<str>,
        target_kind: EntityKind,
        target_namespace: Box<str>,
        target_key: Box<str>,
    },
    /// 需要父项才能派生身份、但没有被任何道路走廊拥有的横断面实体。
    MissingCrossSectionOwner {
        entity_kind: EntityKind,
        stable_key: Box<str>,
    },
    /// 同一横断面实体及发生冲突的两个道路走廊稳定键。
    MultipleCrossSectionOwners {
        entity_kind: EntityKind,
        stable_key: Box<str>,
        first_owner_key: Box<str>,
        second_owner_key: Box<str>,
    },
    /// 道路走廊与不在自身成员序列内的参考道路区段。
    InvalidCorridorReferenceSection {
        corridor_key: Box<str>,
        target_namespace: Box<str>,
        target_key: Box<str>,
    },
    /// 编制车道覆盖链中不相连的一对相邻车道图边。
    DisconnectedAuthoringLaneEdgeChain {
        lane_key: Box<str>,
        predecessor_key: Box<str>,
        successor_key: Box<str>,
    },
    /// 同一车道图边及发生冲突的两个编制车道稳定键。
    MultipleAuthoringLaneOwners {
        edge_key: Box<str>,
        first_lane_key: Box<str>,
        second_lane_key: Box<str>,
    },
    /// 编制车道、所引用车道组以及不一致的两个道路区段父项。
    LaneGroupParentMismatch {
        lane_key: Box<str>,
        lane_group_key: Box<str>,
        lane_section_key: Box<str>,
        group_section_key: Box<str>,
    },
    /// 没有任何编制车道成员的车道组。
    EmptyLaneGroup {
        stable_key: Box<str>,
    },
    /// 没有任何通行流向成员的路口。
    EmptyJunction {
        junction_key: Box<str>,
    },
    /// 没有任何机动路径成员的通行流向。
    EmptyMovement {
        movement_key: Box<str>,
    },
    /// 机动路径完整边序列中不相连的一对相邻边。
    DisconnectedManeuverPath {
        path_key: Box<str>,
        predecessor_key: Box<str>,
        successor_key: Box<str>,
    },
    /// 共享相同完整遍历序列的首个和重复机动路径及其路口。
    DuplicateManeuverPathSequence {
        first_path_key: Box<str>,
        duplicate_path_key: Box<str>,
        first_junction_key: Box<str>,
        duplicate_junction_key: Box<str>,
    },
    /// 同一内部边及发生排他所有者冲突的两个路口和路径。
    InternalEdgeJunctionConflict {
        edge_key: Box<str>,
        first_junction_key: Box<str>,
        duplicate_junction_key: Box<str>,
        first_path_key: Box<str>,
        duplicate_path_key: Box<str>,
    },
    /// 同一边及分别把它声明为内部/边界角色的两条路径。
    InternalBoundaryRoleConflict {
        edge_key: Box<str>,
        internal_path_key: Box<str>,
        boundary_path_key: Box<str>,
    },
    JunctionEdgeSetMismatch {
        junction_key: Box<str>,
        edge_key: Box<str>,
        path_key: Option<Box<str>>,
        violation: JunctionEdgeSetViolation,
    },
    /// 机动门、路径、越界转换下标及该路径可用转换数。
    ManeuverGateTransitionOutOfRange {
        maneuver_gate_key: Box<str>,
        maneuver_path_key: Box<str>,
        transition_index: u32,
        transition_count: u32,
    },
    /// 同一路径转换上的首个和重复机动门。
    DuplicateManeuverGatePathTransition {
        maneuver_path_key: Box<str>,
        transition_index: u32,
        first_maneuver_gate_key: Box<str>,
        duplicate_maneuver_gate_key: Box<str>,
    },
    /// 机动门引用停止线的边与路径转换起始边不一致。
    ManeuverGateStopLineMismatch {
        maneuver_gate_key: Box<str>,
        stop_line_key: Box<str>,
        path_from_edge_key: Box<str>,
        stop_line_edge_key: Box<str>,
    },
    /// 同一车道图边上的首个和重复停止线。
    DuplicateStopLineEdge {
        edge_key: Box<str>,
        first_stop_line_key: Box<str>,
        duplicate_stop_line_key: Box<str>,
    },
    /// 位于终止边、无法形成任何路径转换的停止线。
    OrphanStopLine {
        stop_line_key: Box<str>,
        edge_key: Box<str>,
    },
    /// 位于非终止边但未被任何机动门引用的停止线。
    UnreferencedStopLine {
        stop_line_key: Box<str>,
        edge_key: Box<str>,
    },
    /// 启用入口门的停止线及没有任何候选路径的下游转换。
    MissingManeuverPathCoverage {
        stop_line_key: Box<str>,
        from_edge_key: Box<str>,
        to_edge_key: Box<str>,
    },
    /// 启用入口门的停止线及缺少入口门的候选路径。
    MissingManeuverGateCoverage {
        stop_line_key: Box<str>,
        edge_key: Box<str>,
        maneuver_path_key: Box<str>,
    },
    /// 最大占用数为零的等待区。
    InvalidWaitingZoneCapacity {
        waiting_zone_key: Box<str>,
    },
    /// 等待区中不属于声明路径的入口门或释放门。
    WaitingZoneGatePathMismatch {
        waiting_zone_key: Box<str>,
        gate_role: WaitingZoneGateRole,
        gate_key: Box<str>,
        declared_path_key: Box<str>,
        gate_path_key: Box<str>,
    },
    /// 等待区的入口和释放转换没有形成严格正向区间。
    InvalidWaitingZoneGateOrder {
        waiting_zone_key: Box<str>,
        entry_transition_index: u32,
        release_transition_index: u32,
    },
    /// 同一路径上内部区间相交的两个等待区。
    OverlappingWaitingZones {
        maneuver_path_key: Box<str>,
        first_waiting_zone_key: Box<str>,
        second_waiting_zone_key: Box<str>,
    },
    /// 没有任何边出现项的静态路线。
    EmptyStaticRoute {
        static_route_key: Box<str>,
    },
    /// 静态路线中不相连的一对相邻边及后项在路线内的下标。
    DisconnectedStaticRouteEdge {
        static_route_key: Box<str>,
        predecessor_key: Box<str>,
        successor_key: Box<str>,
        successor_route_edge_index: u32,
    },
    /// 从路口内部边开始的静态路线。
    StaticRouteStartsInsideJunction {
        static_route_key: Box<str>,
        edge_key: Box<str>,
    },
    /// 在路口内部边结束的静态路线。
    StaticRouteEndsInsideJunction {
        static_route_key: Box<str>,
        edge_key: Box<str>,
    },
    /// 在带停止线的最终边结束的静态路线。
    StaticRouteTerminatesAtStopLine {
        static_route_key: Box<str>,
        edge_key: Box<str>,
        stop_line_key: Box<str>,
    },
    /// 从指定路线转换进入候选路径，但没有完整匹配。
    StaticRouteManeuverNoFullMatch {
        static_route_key: Box<str>,
        entry_route_edge_index: u32,
        entry_edge_key: Box<str>,
        next_edge_key: Box<str>,
    },
    /// 从指定路线转换完整匹配的首两条路径。
    StaticRouteManeuverMultipleFullMatches {
        static_route_key: Box<str>,
        entry_route_edge_index: u32,
        first_path_key: Box<str>,
        second_path_key: Box<str>,
    },
    /// 同一路线内部边出现项被两条完整机动路径重复覆盖。
    StaticRouteManeuverInternalOverlap {
        static_route_key: Box<str>,
        route_edge_index: u32,
        edge_key: Box<str>,
        first_path_key: Box<str>,
        second_path_key: Box<str>,
    },
    /// 没有落入完整机动路径出现项的路口内部边出现项。
    StaticRouteInternalEdgeUncovered {
        static_route_key: Box<str>,
        route_edge_index: u32,
        edge_key: Box<str>,
    },
    EmptySignalControllerGroups {
        signal_controller_key: Box<str>,
    },
    EmptySignalControllerPhases {
        signal_controller_key: Box<str>,
    },
    DuplicateSignalControllerGroup {
        signal_controller_key: Box<str>,
        signal_group_key: Box<str>,
    },
    SignalGroupMultipleControllers {
        signal_group_key: Box<str>,
        first_controller_key: Box<str>,
        duplicate_controller_key: Box<str>,
    },
    UnownedSignalGroup {
        signal_group_key: Box<str>,
    },
    UnusedSignalGroup {
        signal_group_key: Box<str>,
    },
    DuplicateSignalPhaseKey {
        signal_controller_key: Box<str>,
        signal_phase_key: Box<str>,
    },
    InvalidSignalPhaseDuration {
        signal_controller_key: Box<str>,
        signal_phase_key: Box<str>,
        duration_ms: u64,
        max_inclusive: u64,
    },
    DuplicateSignalPhaseGroup {
        signal_controller_key: Box<str>,
        signal_phase_key: Box<str>,
        signal_group_key: Box<str>,
    },
    UnknownSignalPhaseGroup {
        signal_controller_key: Box<str>,
        signal_phase_key: Box<str>,
        signal_group_key: Box<str>,
    },
    MissingSignalPhaseGroup {
        signal_controller_key: Box<str>,
        signal_phase_key: Box<str>,
        signal_group_key: Box<str>,
    },
    SignalCycleDurationOverflow {
        signal_controller_key: Box<str>,
        max_inclusive: u64,
    },
    InvalidSignalControllerOffset {
        signal_controller_key: Box<str>,
        offset_ms: u64,
        cycle_duration_ms: u64,
        max_inclusive: u64,
    },
    /// 非法停车锚点及边界比较所需的精确浮点位模式。
    InvalidParkingAnchorProgress {
        parking_space_key: Box<str>,
        role: ParkingAnchorRole,
        lane_edge_key: Box<str>,
        progress_bits: u64,
        edge_length_bits: u64,
        endpoint_clearance_bits: u64,
    },
    /// 非法停车几何字段、原始值和结构化失败原因。
    InvalidParkingSpaceGeometry {
        parking_space_key: Box<str>,
        field: ParkingGeometryField,
        value_bits: u64,
        violation: ParkingGeometryViolation,
    },
    /// 没有任何成员的停车区域。
    OrphanParkingArea {
        parking_area_key: Box<str>,
    },
    ParticipantClassInheritanceCycle {
        participant_class_key: Box<str>,
    },
    /// 非法车辆配置字段、原始值和结构化数值约束。
    InvalidVehicleProfileValue {
        vehicle_profile_key: Box<str>,
        field: Box<str>,
        value_bits: u64,
        violation: ScalarViolation,
    },
    /// 车辆配置两项减速度幅值没有形成合法顺序。
    InvalidVehicleProfileDecelerationOrder {
        vehicle_profile_key: Box<str>,
        comfortable_deceleration_bits: u64,
        emergency_deceleration_bits: u64,
    },
    /// 非法规范空间几何及可选的关联后继边。
    InvalidSpatialGeometry {
        canonical_frame_key: Option<Box<str>>,
        lane_edge_key: Box<str>,
        related_lane_edge_key: Option<Box<str>>,
        violation: SpatialGeometryViolation,
    },
    /// 非法 FacilityBand 规范中心线。
    InvalidFacilityBandGeometry {
        canonical_frame_key: Option<Box<str>>,
        facility_band_key: Box<str>,
        violation: SpatialGeometryViolation,
    },
    EmptyAccessRuleParticipantClasses {
        access_rule_key: Box<str>,
    },
    AccessCapabilityUnavailable {
        access_rule_key: Box<str>,
        capability: AccessCapability,
    },
    InvalidAccessRegulationString {
        access_rule_key: Box<str>,
        field: AccessRegulationField,
        character_count: u32,
    },
    AccessRegulationMismatch {
        first_rule_key: Box<str>,
        first_jurisdiction: Box<str>,
        first_version: Box<str>,
        second_rule_key: Box<str>,
        second_jurisdiction: Box<str>,
        second_version: Box<str>,
    },
    AccessRuleAmbiguity {
        plane: AccessPlane,
        target_kind: EntityKind,
        target_key: Box<str>,
        participant_class_key: Box<str>,
        first_rule_key: Box<str>,
        second_rule_key: Box<str>,
    },
    /// 实体种类、来源稳定键及不能形成 Identity v1 前像的精确原因。
    InvalidCanonicalIdentity {
        entity_kind: EntityKind,
        stable_key: Box<str>,
        violation: CanonicalIdentityViolation,
    },
    /// 重复完整身份的实体种类和已派生摘要。
    DuplicateCanonicalIdentity {
        entity_kind: EntityKind,
        stable_id: StableId128,
    },
    /// 发生 BLAKE3-128 摘要碰撞的实体种类和冲突摘要。
    IdentityDigestCollision {
        entity_kind: EntityKind,
        stable_id: StableId128,
    },
}

/// 一条不可变结构化诊断。
///
/// 排序同时考虑规范模块顺序、来源位置、代码、严重程度、载荷、稳定键和关联位置。
/// `primary_location` 指向主要失败位置，`related_locations` 用于重复声明、跨模块引用等需要同时
/// 展示上下文的情况。
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Diagnostic {
    canonical_module_order: u32,
    primary_span: Option<SourceLocation>,
    code: DiagnosticCode,
    severity: DiagnosticSeverity,
    payload: DiagnosticPayload,
    stable_key: Option<Box<str>>,
    related_spans: Box<[SourceLocation]>,
}

pub(crate) trait IntoSourceLocationOption {
    fn into_source_location_option(self) -> Option<SourceLocation>;
}

impl<T: Into<SourceLocation>> IntoSourceLocationOption for Option<T> {
    fn into_source_location_option(self) -> Option<SourceLocation> {
        self.map(Into::into)
    }
}

impl Diagnostic {
    pub(crate) const fn failure_owned_bytes_upper_bound() -> u64 {
        // 一个 failure 只保留一条诊断。闭合字段名最多 256 bytes，此外最多三份受
        // SingleStringBytes 限制的 payload/stable token 与一条 related location backing。
        // context/identity allocation 由调用点另计。
        256 + 53 * 3 + core::mem::size_of::<SourceLocation>() as u64
    }

    pub(crate) fn invalid_road_editing_input(
        field: &str,
        violation: RoadEditingInputViolation,
    ) -> Self {
        Self {
            canonical_module_order: 0,
            primary_span: None,
            code: DiagnosticCode::InvalidRoadEditingInput,
            severity: DiagnosticSeverity::Error,
            payload: DiagnosticPayload::InvalidRoadEditingInput {
                field: field.into(),
                violation,
            },
            stable_key: None,
            related_spans: Box::default(),
        }
    }
    #[allow(
        dead_code,
        reason = "called by the staged road-editing reader before public admission lands"
    )]
    pub(crate) fn invalid_road_editing_source(
        violation: RoadEditingSourceViolation,
        field: Option<&str>,
        expected_source_document_key: &str,
        actual_source_document_key: Option<&str>,
    ) -> Self {
        Self::invalid_road_editing_source_at(
            violation,
            field,
            expected_source_document_key,
            actual_source_document_key,
            None,
        )
    }

    pub(crate) fn invalid_road_editing_source_at(
        violation: RoadEditingSourceViolation,
        field: Option<&str>,
        expected_source_document_key: &str,
        actual_source_document_key: Option<&str>,
        primary_location: Option<SourceLocation>,
    ) -> Self {
        Self {
            canonical_module_order: 0,
            primary_span: primary_location,
            code: DiagnosticCode::InvalidRoadEditingSource,
            severity: DiagnosticSeverity::Error,
            payload: DiagnosticPayload::InvalidRoadEditingSource {
                violation,
                field: field.map(Into::into),
                expected_source_document_key: expected_source_document_key.into(),
                actual_source_document_key: actual_source_document_key.map(Into::into),
            },
            stable_key: Some(expected_source_document_key.into()),
            related_spans: Box::default(),
        }
    }

    pub(crate) fn invalid_source_header_field(
        field: SourceHeaderField,
        violation: SourceTextViolation,
    ) -> Self {
        Self {
            canonical_module_order: 0,
            primary_span: None,
            code: DiagnosticCode::InvalidSourceHeaderField,
            severity: DiagnosticSeverity::Error,
            payload: DiagnosticPayload::InvalidSourceHeaderField { field, violation },
            stable_key: None,
            related_spans: Box::default(),
        }
    }

    pub(crate) fn compile_limit_exceeded(
        dimension: CompileLimitDimension,
        limit: u64,
        observed: u64,
    ) -> Self {
        Self {
            canonical_module_order: 0,
            primary_span: None,
            code: DiagnosticCode::CompileLimitExceeded,
            severity: DiagnosticSeverity::Error,
            payload: DiagnosticPayload::CompileLimitExceeded {
                dimension,
                limit,
                observed,
            },
            stable_key: None,
            related_spans: Box::default(),
        }
    }

    pub(crate) fn compile_profile_incompatible(
        profile_id: &str,
        required_dimension: CompileLimitDimension,
        primary_span: impl Into<SourceLocation>,
        stable_key: &str,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::CompileProfileIncompatible,
            DiagnosticPayload::CompileProfileIncompatible {
                profile_id: profile_id.into(),
                required_dimension,
            },
            Some(primary_span),
            Box::default(),
            Some(stable_key.into()),
        )
    }

    pub(crate) fn invalid_import_namespace(
        violation: SourceTextViolation,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::InvalidImportNamespace,
            DiagnosticPayload::InvalidImportNamespace { violation },
            Some(primary_span),
            Box::default(),
            None,
        )
    }

    pub(crate) fn duplicate_import(
        namespace: &str,
        primary_span: impl Into<SourceLocation>,
        related_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::DuplicateImport,
            DiagnosticPayload::DuplicateImport {
                namespace: namespace.into(),
            },
            Some(primary_span),
            Box::new([related_span.into()]),
            Some(namespace.into()),
        )
    }

    pub(crate) fn duplicate_module_namespace(
        namespace: &str,
        primary_span: impl Into<SourceLocation>,
        related_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::DuplicateModuleNamespace,
            DiagnosticPayload::DuplicateModuleNamespace {
                namespace: namespace.into(),
            },
            Some(primary_span),
            Box::new([related_span.into()]),
            Some(namespace.into()),
        )
    }

    pub(crate) fn duplicate_source_document_key(
        source_document_key: &str,
        primary_span: impl Into<SourceLocation>,
        related_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::DuplicateSourceDocumentKey,
            DiagnosticPayload::DuplicateSourceDocumentKey {
                source_document_key: source_document_key.into(),
            },
            Some(primary_span),
            Box::new([related_span.into()]),
            Some(source_document_key.into()),
        )
    }

    pub(crate) fn source_document_ownership_mismatch(
        source_document_key: &str,
        expected_authoring_namespace_id: &str,
        actual_authoring_namespace_id: Option<&str>,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::SourceDocumentOwnershipMismatch,
            DiagnosticPayload::SourceDocumentOwnershipMismatch {
                source_document_key: source_document_key.into(),
                expected_authoring_namespace_id: expected_authoring_namespace_id.into(),
                actual_authoring_namespace_id: actual_authoring_namespace_id.map(Into::into),
            },
            Some(primary_span),
            Box::default(),
            Some(source_document_key.into()),
        )
    }

    pub(crate) fn unknown_import(namespace: &str, primary_span: impl Into<SourceLocation>) -> Self {
        Self::error_with_context(
            DiagnosticCode::UnknownImport,
            DiagnosticPayload::UnknownImport {
                namespace: namespace.into(),
            },
            Some(primary_span),
            Box::default(),
            Some(namespace.into()),
        )
    }

    pub(crate) fn import_cycle<T: Into<SourceLocation>>(
        namespaces: &[&str],
        spans: Box<[T]>,
    ) -> Self {
        let spans: Box<[SourceLocation]> = spans.into_vec().into_iter().map(Into::into).collect();
        let stable_key = namespaces.first().copied().map(Into::into);
        let mut spans = spans.into_vec();
        let primary_span = if spans.is_empty() {
            None
        } else {
            Some(spans.remove(0))
        };
        Self::error_with_context(
            DiagnosticCode::ImportCycle,
            DiagnosticPayload::ImportCycle {
                namespaces: namespaces
                    .iter()
                    .map(|namespace| (*namespace).into())
                    .collect(),
            },
            primary_span,
            spans.into_boxed_slice(),
            stable_key,
        )
    }

    pub(crate) fn invalid_declaration_key(
        entity_kind: EntityKind,
        violation: SourceTextViolation,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::InvalidDeclarationKey,
            DiagnosticPayload::InvalidDeclarationKey {
                entity_kind,
                violation,
            },
            Some(primary_span),
            Box::default(),
            None,
        )
    }

    pub(crate) fn duplicate_declaration(
        entity_kind: EntityKind,
        stable_key: &str,
        primary_span: impl Into<SourceLocation>,
        related_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::DuplicateDeclaration,
            DiagnosticPayload::DuplicateDeclaration {
                entity_kind,
                stable_key: stable_key.into(),
            },
            Some(primary_span),
            Box::new([related_span.into()]),
            Some(stable_key.into()),
        )
    }

    pub(crate) fn invalid_reference_key(
        entity_kind: EntityKind,
        violation: SourceTextViolation,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::InvalidReferenceKey,
            DiagnosticPayload::InvalidReferenceKey {
                entity_kind,
                violation,
            },
            Some(primary_span),
            Box::default(),
            None,
        )
    }

    pub(crate) fn invalid_reference_namespace(
        violation: SourceTextViolation,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::InvalidReferenceNamespace,
            DiagnosticPayload::InvalidReferenceNamespace { violation },
            Some(primary_span),
            Box::default(),
            None,
        )
    }

    pub(crate) fn unimported_reference_module(
        namespace: &str,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::UnimportedReferenceModule,
            DiagnosticPayload::UnimportedReferenceModule {
                namespace: namespace.into(),
            },
            Some(primary_span),
            Box::default(),
            Some(namespace.into()),
        )
    }

    pub(crate) fn unknown_reference_target(
        entity_kind: EntityKind,
        source_key: &str,
        target_namespace: &str,
        target_key: &str,
        primary_span: impl Into<SourceLocation>,
        source_declaration_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::unknown_owner_qualified_reference_target(
            entity_kind,
            source_key,
            target_namespace,
            &[],
            target_key,
            primary_span,
            source_declaration_span,
        )
    }

    pub(crate) fn unknown_owner_qualified_reference_target(
        entity_kind: EntityKind,
        source_key: &str,
        target_namespace: &str,
        target_owner_local_keys: &[Arc<str>],
        target_key: &str,
        primary_span: impl Into<SourceLocation>,
        source_declaration_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::UnknownReferenceTarget,
            DiagnosticPayload::UnknownReferenceTarget {
                entity_kind,
                source_key: source_key.into(),
                target_namespace: target_namespace.into(),
                target_owner_local_keys: target_owner_local_keys
                    .iter()
                    .map(|key| Box::<str>::from(key.as_ref()))
                    .collect(),
                target_key: target_key.into(),
            },
            Some(primary_span),
            Box::new([source_declaration_span.into()]),
            Some(source_key.into()),
        )
    }

    pub(crate) fn invalid_identity_ascii_field(
        entity_kind: EntityKind,
        stable_key: &str,
        field_tag: FieldTag,
        violation: SourceTextViolation,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::InvalidIdentityAsciiField,
            DiagnosticPayload::InvalidIdentityAsciiField {
                entity_kind,
                stable_key: stable_key.into(),
                field_tag,
                violation,
            },
            Some(primary_span),
            Box::default(),
            Some(stable_key.into()),
        )
    }

    pub(crate) fn invalid_lane_edge_length(
        stable_key: &str,
        value: f64,
        violation: ScalarViolation,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::InvalidLaneEdgeLength,
            DiagnosticPayload::InvalidLaneEdgeLength {
                stable_key: stable_key.into(),
                value_bits: value.to_bits(),
                violation,
            },
            Some(primary_span),
            Box::default(),
            Some(stable_key.into()),
        )
    }

    pub(crate) fn invalid_lane_edge_speed_limit(
        stable_key: &str,
        value: f64,
        violation: ScalarViolation,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::InvalidLaneEdgeSpeedLimit,
            DiagnosticPayload::InvalidLaneEdgeSpeedLimit {
                stable_key: stable_key.into(),
                value_bits: value.to_bits(),
                violation,
            },
            Some(primary_span),
            Box::default(),
            Some(stable_key.into()),
        )
    }

    pub(crate) fn invalid_vehicle_profile_value(
        vehicle_profile_key: &str,
        field: &'static str,
        value: f64,
        violation: ScalarViolation,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::InvalidVehicleProfileValue,
            DiagnosticPayload::InvalidVehicleProfileValue {
                vehicle_profile_key: vehicle_profile_key.into(),
                field: field.into(),
                value_bits: value.to_bits(),
                violation,
            },
            Some(primary_span),
            Box::default(),
            Some(vehicle_profile_key.into()),
        )
    }

    pub(crate) fn invalid_vehicle_profile_deceleration_order(
        vehicle_profile_key: &str,
        comfortable_deceleration: f64,
        emergency_deceleration: f64,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::InvalidVehicleProfileDecelerationOrder,
            DiagnosticPayload::InvalidVehicleProfileDecelerationOrder {
                vehicle_profile_key: vehicle_profile_key.into(),
                comfortable_deceleration_bits: comfortable_deceleration.to_bits(),
                emergency_deceleration_bits: emergency_deceleration.to_bits(),
            },
            Some(primary_span),
            Box::default(),
            Some(vehicle_profile_key.into()),
        )
    }

    pub(crate) fn invalid_spatial_geometry(
        canonical_frame_key: Option<&str>,
        lane_edge_key: &str,
        related_lane_edge_key: Option<&str>,
        violation: SpatialGeometryViolation,
        primary_span: impl Into<SourceLocation>,
        related_span: Option<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::InvalidSpatialGeometry,
            DiagnosticPayload::InvalidSpatialGeometry {
                canonical_frame_key: canonical_frame_key.map(Into::into),
                lane_edge_key: lane_edge_key.into(),
                related_lane_edge_key: related_lane_edge_key.map(Into::into),
                violation,
            },
            Some(primary_span),
            related_span.map_or_else(
                || Vec::new().into_boxed_slice(),
                |span| vec![span].into_boxed_slice(),
            ),
            Some(lane_edge_key.into()),
        )
    }

    pub(crate) fn invalid_facility_band_geometry(
        canonical_frame_key: Option<&str>,
        facility_band_key: &str,
        violation: SpatialGeometryViolation,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::InvalidFacilityBandGeometry,
            DiagnosticPayload::InvalidFacilityBandGeometry {
                canonical_frame_key: canonical_frame_key.map(Into::into),
                facility_band_key: facility_band_key.into(),
                violation,
            },
            Some(primary_span),
            Box::default(),
            Some(facility_band_key.into()),
        )
    }

    pub(crate) fn duplicate_lane_edge_successor(
        stable_key: &str,
        target_namespace: &str,
        target_key: &str,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::DuplicateLaneEdgeSuccessor,
            DiagnosticPayload::DuplicateLaneEdgeSuccessor {
                stable_key: stable_key.into(),
                target_namespace: target_namespace.into(),
                target_key: target_key.into(),
            },
            Some(primary_span),
            Box::default(),
            Some(stable_key.into()),
        )
    }

    pub(crate) fn invalid_facility_kind(
        entity_kind: EntityKind,
        stable_key: &str,
        kind_id: &str,
        expected_category: FacilityKindCategory,
        violation: FacilityKindViolation,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::InvalidFacilityKind,
            DiagnosticPayload::InvalidFacilityKind {
                entity_kind,
                stable_key: stable_key.into(),
                kind_id: kind_id.into(),
                expected_category,
                violation,
            },
            Some(primary_span),
            Box::default(),
            Some(stable_key.into()),
        )
    }

    pub(crate) fn empty_road_section_lanes(
        stable_key: &str,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::EmptyRoadSectionLanes,
            DiagnosticPayload::EmptyRoadSectionLanes {
                stable_key: stable_key.into(),
            },
            Some(primary_span),
            Box::default(),
            Some(stable_key.into()),
        )
    }

    pub(crate) fn empty_authoring_lane_edge_chain(
        stable_key: &str,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::EmptyAuthoringLaneEdgeChain,
            DiagnosticPayload::EmptyAuthoringLaneEdgeChain {
                stable_key: stable_key.into(),
            },
            Some(primary_span),
            Box::default(),
            Some(stable_key.into()),
        )
    }

    pub(crate) fn duplicate_authoring_lane_edge(
        stable_key: &str,
        target_namespace: &str,
        target_key: &str,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::DuplicateAuthoringLaneEdge,
            DiagnosticPayload::DuplicateAuthoringLaneEdge {
                stable_key: stable_key.into(),
                target_namespace: target_namespace.into(),
                target_key: target_key.into(),
            },
            Some(primary_span),
            Box::default(),
            Some(stable_key.into()),
        )
    }

    pub(crate) fn empty_road_corridor_elements(
        stable_key: &str,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::EmptyRoadCorridorElements,
            DiagnosticPayload::EmptyRoadCorridorElements {
                stable_key: stable_key.into(),
            },
            Some(primary_span),
            Box::default(),
            Some(stable_key.into()),
        )
    }

    pub(crate) fn duplicate_road_corridor_element(
        stable_key: &str,
        target_kind: EntityKind,
        target_namespace: &str,
        target_key: &str,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::DuplicateRoadCorridorElement,
            DiagnosticPayload::DuplicateRoadCorridorElement {
                stable_key: stable_key.into(),
                target_kind,
                target_namespace: target_namespace.into(),
                target_key: target_key.into(),
            },
            Some(primary_span),
            Box::default(),
            Some(stable_key.into()),
        )
    }

    pub(crate) fn missing_cross_section_owner(
        entity_kind: EntityKind,
        stable_key: &str,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::MissingCrossSectionOwner,
            DiagnosticPayload::MissingCrossSectionOwner {
                entity_kind,
                stable_key: stable_key.into(),
            },
            Some(primary_span),
            Box::default(),
            Some(stable_key.into()),
        )
    }

    pub(crate) fn multiple_cross_section_owners(
        entity_kind: EntityKind,
        stable_key: &str,
        first_owner_key: &str,
        second_owner_key: &str,
        primary_span: impl Into<SourceLocation>,
        first_owner_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::MultipleCrossSectionOwners,
            DiagnosticPayload::MultipleCrossSectionOwners {
                entity_kind,
                stable_key: stable_key.into(),
                first_owner_key: first_owner_key.into(),
                second_owner_key: second_owner_key.into(),
            },
            Some(primary_span),
            Box::new([first_owner_span.into()]),
            Some(stable_key.into()),
        )
    }

    pub(crate) fn invalid_corridor_reference_section(
        corridor_key: &str,
        target_namespace: &str,
        target_key: &str,
        primary_span: impl Into<SourceLocation>,
        corridor_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::InvalidCorridorReferenceSection,
            DiagnosticPayload::InvalidCorridorReferenceSection {
                corridor_key: corridor_key.into(),
                target_namespace: target_namespace.into(),
                target_key: target_key.into(),
            },
            Some(primary_span),
            Box::new([corridor_span.into()]),
            Some(corridor_key.into()),
        )
    }

    pub(crate) fn disconnected_authoring_lane_edge_chain(
        lane_key: &str,
        predecessor_key: &str,
        successor_key: &str,
        primary_span: impl Into<SourceLocation>,
        predecessor_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::DisconnectedAuthoringLaneEdgeChain,
            DiagnosticPayload::DisconnectedAuthoringLaneEdgeChain {
                lane_key: lane_key.into(),
                predecessor_key: predecessor_key.into(),
                successor_key: successor_key.into(),
            },
            Some(primary_span),
            Box::new([predecessor_span.into()]),
            Some(lane_key.into()),
        )
    }

    pub(crate) fn multiple_authoring_lane_owners(
        edge_key: &str,
        first_lane_key: &str,
        second_lane_key: &str,
        primary_span: impl Into<SourceLocation>,
        first_lane_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::MultipleAuthoringLaneOwners,
            DiagnosticPayload::MultipleAuthoringLaneOwners {
                edge_key: edge_key.into(),
                first_lane_key: first_lane_key.into(),
                second_lane_key: second_lane_key.into(),
            },
            Some(primary_span),
            Box::new([first_lane_span.into()]),
            Some(edge_key.into()),
        )
    }

    pub(crate) fn lane_group_parent_mismatch(
        lane_key: &str,
        lane_group_key: &str,
        lane_section_key: &str,
        group_section_key: &str,
        primary_span: impl Into<SourceLocation>,
        group_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::LaneGroupParentMismatch,
            DiagnosticPayload::LaneGroupParentMismatch {
                lane_key: lane_key.into(),
                lane_group_key: lane_group_key.into(),
                lane_section_key: lane_section_key.into(),
                group_section_key: group_section_key.into(),
            },
            Some(primary_span),
            Box::new([group_span.into()]),
            Some(lane_key.into()),
        )
    }

    pub(crate) fn empty_lane_group(
        stable_key: &str,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::EmptyLaneGroup,
            DiagnosticPayload::EmptyLaneGroup {
                stable_key: stable_key.into(),
            },
            Some(primary_span),
            Box::default(),
            Some(stable_key.into()),
        )
    }

    pub(crate) fn empty_junction(
        junction_key: &str,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::EmptyJunction,
            DiagnosticPayload::EmptyJunction {
                junction_key: junction_key.into(),
            },
            Some(primary_span),
            Box::default(),
            Some(junction_key.into()),
        )
    }

    pub(crate) fn empty_movement(
        movement_key: &str,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::EmptyMovement,
            DiagnosticPayload::EmptyMovement {
                movement_key: movement_key.into(),
            },
            Some(primary_span),
            Box::default(),
            Some(movement_key.into()),
        )
    }

    pub(crate) fn disconnected_maneuver_path(
        path_key: &str,
        predecessor_key: &str,
        successor_key: &str,
        primary_span: impl Into<SourceLocation>,
        predecessor_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::DisconnectedManeuverPath,
            DiagnosticPayload::DisconnectedManeuverPath {
                path_key: path_key.into(),
                predecessor_key: predecessor_key.into(),
                successor_key: successor_key.into(),
            },
            Some(primary_span),
            Box::new([predecessor_span.into()]),
            Some(path_key.into()),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn duplicate_maneuver_path_sequence(
        first_path_key: &str,
        duplicate_path_key: &str,
        first_junction_key: &str,
        duplicate_junction_key: &str,
        primary_span: impl Into<SourceLocation>,
        first_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::DuplicateManeuverPathSequence,
            DiagnosticPayload::DuplicateManeuverPathSequence {
                first_path_key: first_path_key.into(),
                duplicate_path_key: duplicate_path_key.into(),
                first_junction_key: first_junction_key.into(),
                duplicate_junction_key: duplicate_junction_key.into(),
            },
            Some(primary_span),
            Box::new([first_span.into()]),
            Some(duplicate_path_key.into()),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn internal_edge_junction_conflict(
        edge_key: &str,
        first_junction_key: &str,
        duplicate_junction_key: &str,
        first_path_key: &str,
        duplicate_path_key: &str,
        primary_span: impl Into<SourceLocation>,
        first_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::InternalEdgeJunctionConflict,
            DiagnosticPayload::InternalEdgeJunctionConflict {
                edge_key: edge_key.into(),
                first_junction_key: first_junction_key.into(),
                duplicate_junction_key: duplicate_junction_key.into(),
                first_path_key: first_path_key.into(),
                duplicate_path_key: duplicate_path_key.into(),
            },
            Some(primary_span),
            Box::new([first_span.into()]),
            Some(edge_key.into()),
        )
    }

    pub(crate) fn internal_boundary_role_conflict(
        edge_key: &str,
        internal_path_key: &str,
        boundary_path_key: &str,
        primary_span: impl Into<SourceLocation>,
        internal_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::InternalBoundaryRoleConflict,
            DiagnosticPayload::InternalBoundaryRoleConflict {
                edge_key: edge_key.into(),
                internal_path_key: internal_path_key.into(),
                boundary_path_key: boundary_path_key.into(),
            },
            Some(primary_span),
            Box::new([internal_span.into()]),
            Some(edge_key.into()),
        )
    }

    pub(crate) fn junction_edge_set_mismatch(
        junction_key: &str,
        edge_key: &str,
        path_key: Option<&str>,
        violation: JunctionEdgeSetViolation,
        primary_span: impl Into<SourceLocation>,
        related_span: Option<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::JunctionEdgeSetMismatch,
            DiagnosticPayload::JunctionEdgeSetMismatch {
                junction_key: junction_key.into(),
                edge_key: edge_key.into(),
                path_key: path_key.map(Into::into),
                violation,
            },
            Some(primary_span),
            related_span.into_iter().collect(),
            Some(edge_key.into()),
        )
    }

    pub(crate) fn maneuver_gate_transition_out_of_range(
        maneuver_gate_key: &str,
        maneuver_path_key: &str,
        transition_index: u32,
        transition_count: u32,
        primary_span: impl Into<SourceLocation>,
        path_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::ManeuverGateTransitionOutOfRange,
            DiagnosticPayload::ManeuverGateTransitionOutOfRange {
                maneuver_gate_key: maneuver_gate_key.into(),
                maneuver_path_key: maneuver_path_key.into(),
                transition_index,
                transition_count,
            },
            Some(primary_span),
            Box::new([path_span.into()]),
            Some(maneuver_gate_key.into()),
        )
    }

    pub(crate) fn duplicate_maneuver_gate_path_transition(
        maneuver_path_key: &str,
        transition_index: u32,
        first_maneuver_gate_key: &str,
        duplicate_maneuver_gate_key: &str,
        primary_span: impl Into<SourceLocation>,
        first_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::DuplicateManeuverGatePathTransition,
            DiagnosticPayload::DuplicateManeuverGatePathTransition {
                maneuver_path_key: maneuver_path_key.into(),
                transition_index,
                first_maneuver_gate_key: first_maneuver_gate_key.into(),
                duplicate_maneuver_gate_key: duplicate_maneuver_gate_key.into(),
            },
            Some(primary_span),
            Box::new([first_span.into()]),
            Some(duplicate_maneuver_gate_key.into()),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn maneuver_gate_stop_line_mismatch(
        maneuver_gate_key: &str,
        stop_line_key: &str,
        path_from_edge_key: &str,
        stop_line_edge_key: &str,
        primary_span: impl Into<SourceLocation>,
        stop_line_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::ManeuverGateStopLineMismatch,
            DiagnosticPayload::ManeuverGateStopLineMismatch {
                maneuver_gate_key: maneuver_gate_key.into(),
                stop_line_key: stop_line_key.into(),
                path_from_edge_key: path_from_edge_key.into(),
                stop_line_edge_key: stop_line_edge_key.into(),
            },
            Some(primary_span),
            Box::new([stop_line_span.into()]),
            Some(maneuver_gate_key.into()),
        )
    }

    pub(crate) fn duplicate_stop_line_edge(
        edge_key: &str,
        first_stop_line_key: &str,
        duplicate_stop_line_key: &str,
        primary_span: impl Into<SourceLocation>,
        first_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::DuplicateStopLineEdge,
            DiagnosticPayload::DuplicateStopLineEdge {
                edge_key: edge_key.into(),
                first_stop_line_key: first_stop_line_key.into(),
                duplicate_stop_line_key: duplicate_stop_line_key.into(),
            },
            Some(primary_span),
            Box::new([first_span.into()]),
            Some(duplicate_stop_line_key.into()),
        )
    }

    pub(crate) fn orphan_stop_line(
        stop_line_key: &str,
        edge_key: &str,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::OrphanStopLine,
            DiagnosticPayload::OrphanStopLine {
                stop_line_key: stop_line_key.into(),
                edge_key: edge_key.into(),
            },
            Some(primary_span),
            Box::default(),
            Some(stop_line_key.into()),
        )
    }

    pub(crate) fn unreferenced_stop_line(
        stop_line_key: &str,
        edge_key: &str,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::UnreferencedStopLine,
            DiagnosticPayload::UnreferencedStopLine {
                stop_line_key: stop_line_key.into(),
                edge_key: edge_key.into(),
            },
            Some(primary_span),
            Box::default(),
            Some(stop_line_key.into()),
        )
    }

    pub(crate) fn missing_maneuver_path_coverage(
        stop_line_key: &str,
        from_edge_key: &str,
        to_edge_key: &str,
        primary_span: impl Into<SourceLocation>,
        to_edge_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::MissingManeuverPathCoverage,
            DiagnosticPayload::MissingManeuverPathCoverage {
                stop_line_key: stop_line_key.into(),
                from_edge_key: from_edge_key.into(),
                to_edge_key: to_edge_key.into(),
            },
            Some(primary_span),
            Box::new([to_edge_span.into()]),
            Some(stop_line_key.into()),
        )
    }

    pub(crate) fn missing_maneuver_gate_coverage(
        stop_line_key: &str,
        edge_key: &str,
        maneuver_path_key: &str,
        primary_span: impl Into<SourceLocation>,
        path_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::MissingManeuverGateCoverage,
            DiagnosticPayload::MissingManeuverGateCoverage {
                stop_line_key: stop_line_key.into(),
                edge_key: edge_key.into(),
                maneuver_path_key: maneuver_path_key.into(),
            },
            Some(primary_span),
            Box::new([path_span.into()]),
            Some(stop_line_key.into()),
        )
    }

    pub(crate) fn invalid_waiting_zone_capacity(
        waiting_zone_key: &str,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::InvalidWaitingZoneCapacity,
            DiagnosticPayload::InvalidWaitingZoneCapacity {
                waiting_zone_key: waiting_zone_key.into(),
            },
            Some(primary_span),
            Box::default(),
            Some(waiting_zone_key.into()),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn waiting_zone_gate_path_mismatch(
        waiting_zone_key: &str,
        gate_role: WaitingZoneGateRole,
        gate_key: &str,
        declared_path_key: &str,
        gate_path_key: &str,
        primary_span: impl Into<SourceLocation>,
        gate_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::WaitingZoneGatePathMismatch,
            DiagnosticPayload::WaitingZoneGatePathMismatch {
                waiting_zone_key: waiting_zone_key.into(),
                gate_role,
                gate_key: gate_key.into(),
                declared_path_key: declared_path_key.into(),
                gate_path_key: gate_path_key.into(),
            },
            Some(primary_span),
            Box::new([gate_span.into()]),
            Some(waiting_zone_key.into()),
        )
    }

    pub(crate) fn invalid_waiting_zone_gate_order(
        waiting_zone_key: &str,
        entry_transition_index: u32,
        release_transition_index: u32,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::InvalidWaitingZoneGateOrder,
            DiagnosticPayload::InvalidWaitingZoneGateOrder {
                waiting_zone_key: waiting_zone_key.into(),
                entry_transition_index,
                release_transition_index,
            },
            Some(primary_span),
            Box::default(),
            Some(waiting_zone_key.into()),
        )
    }

    pub(crate) fn overlapping_waiting_zones(
        maneuver_path_key: &str,
        first_waiting_zone_key: &str,
        second_waiting_zone_key: &str,
        primary_span: impl Into<SourceLocation>,
        first_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::OverlappingWaitingZones,
            DiagnosticPayload::OverlappingWaitingZones {
                maneuver_path_key: maneuver_path_key.into(),
                first_waiting_zone_key: first_waiting_zone_key.into(),
                second_waiting_zone_key: second_waiting_zone_key.into(),
            },
            Some(primary_span),
            Box::new([first_span.into()]),
            Some(second_waiting_zone_key.into()),
        )
    }

    pub(crate) fn empty_static_route(
        static_route_key: &str,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::EmptyStaticRoute,
            DiagnosticPayload::EmptyStaticRoute {
                static_route_key: static_route_key.into(),
            },
            Some(primary_span),
            Box::default(),
            Some(static_route_key.into()),
        )
    }

    pub(crate) fn disconnected_static_route_edge(
        static_route_key: &str,
        predecessor_key: &str,
        successor_key: &str,
        successor_route_edge_index: u32,
        primary_span: impl Into<SourceLocation>,
        predecessor_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::DisconnectedStaticRouteEdge,
            DiagnosticPayload::DisconnectedStaticRouteEdge {
                static_route_key: static_route_key.into(),
                predecessor_key: predecessor_key.into(),
                successor_key: successor_key.into(),
                successor_route_edge_index,
            },
            Some(primary_span),
            Box::new([predecessor_span.into()]),
            Some(static_route_key.into()),
        )
    }

    pub(crate) fn static_route_starts_inside_junction(
        static_route_key: &str,
        edge_key: &str,
        primary_span: impl Into<SourceLocation>,
        path_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::StaticRouteStartsInsideJunction,
            DiagnosticPayload::StaticRouteStartsInsideJunction {
                static_route_key: static_route_key.into(),
                edge_key: edge_key.into(),
            },
            Some(primary_span),
            Box::new([path_span.into()]),
            Some(static_route_key.into()),
        )
    }

    pub(crate) fn static_route_ends_inside_junction(
        static_route_key: &str,
        edge_key: &str,
        primary_span: impl Into<SourceLocation>,
        path_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::StaticRouteEndsInsideJunction,
            DiagnosticPayload::StaticRouteEndsInsideJunction {
                static_route_key: static_route_key.into(),
                edge_key: edge_key.into(),
            },
            Some(primary_span),
            Box::new([path_span.into()]),
            Some(static_route_key.into()),
        )
    }

    pub(crate) fn static_route_terminates_at_stop_line(
        static_route_key: &str,
        edge_key: &str,
        stop_line_key: &str,
        primary_span: impl Into<SourceLocation>,
        stop_line_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::StaticRouteTerminatesAtStopLine,
            DiagnosticPayload::StaticRouteTerminatesAtStopLine {
                static_route_key: static_route_key.into(),
                edge_key: edge_key.into(),
                stop_line_key: stop_line_key.into(),
            },
            Some(primary_span),
            Box::new([stop_line_span.into()]),
            Some(static_route_key.into()),
        )
    }

    pub(crate) fn static_route_maneuver_no_full_match(
        static_route_key: &str,
        entry_route_edge_index: u32,
        entry_edge_key: &str,
        next_edge_key: &str,
        primary_span: impl Into<SourceLocation>,
        candidate_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::StaticRouteManeuverNoFullMatch,
            DiagnosticPayload::StaticRouteManeuverNoFullMatch {
                static_route_key: static_route_key.into(),
                entry_route_edge_index,
                entry_edge_key: entry_edge_key.into(),
                next_edge_key: next_edge_key.into(),
            },
            Some(primary_span),
            Box::new([candidate_span.into()]),
            Some(static_route_key.into()),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn static_route_maneuver_multiple_full_matches(
        static_route_key: &str,
        entry_route_edge_index: u32,
        first_path_key: &str,
        second_path_key: &str,
        primary_span: impl Into<SourceLocation>,
        first_path_span: impl Into<SourceLocation>,
        second_path_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::StaticRouteManeuverMultipleFullMatches,
            DiagnosticPayload::StaticRouteManeuverMultipleFullMatches {
                static_route_key: static_route_key.into(),
                entry_route_edge_index,
                first_path_key: first_path_key.into(),
                second_path_key: second_path_key.into(),
            },
            Some(primary_span),
            Box::new([first_path_span.into(), second_path_span.into()]),
            Some(static_route_key.into()),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn static_route_maneuver_internal_overlap(
        static_route_key: &str,
        route_edge_index: u32,
        edge_key: &str,
        first_path_key: &str,
        second_path_key: &str,
        primary_span: impl Into<SourceLocation>,
        first_path_span: impl Into<SourceLocation>,
        second_path_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::StaticRouteManeuverInternalOverlap,
            DiagnosticPayload::StaticRouteManeuverInternalOverlap {
                static_route_key: static_route_key.into(),
                route_edge_index,
                edge_key: edge_key.into(),
                first_path_key: first_path_key.into(),
                second_path_key: second_path_key.into(),
            },
            Some(primary_span),
            Box::new([first_path_span.into(), second_path_span.into()]),
            Some(static_route_key.into()),
        )
    }

    pub(crate) fn static_route_internal_edge_uncovered(
        static_route_key: &str,
        route_edge_index: u32,
        edge_key: &str,
        primary_span: impl Into<SourceLocation>,
        path_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::StaticRouteInternalEdgeUncovered,
            DiagnosticPayload::StaticRouteInternalEdgeUncovered {
                static_route_key: static_route_key.into(),
                route_edge_index,
                edge_key: edge_key.into(),
            },
            Some(primary_span),
            Box::new([path_span.into()]),
            Some(static_route_key.into()),
        )
    }

    pub(crate) fn empty_signal_controller_groups(
        controller_key: &str,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::EmptySignalControllerGroups,
            DiagnosticPayload::EmptySignalControllerGroups {
                signal_controller_key: controller_key.into(),
            },
            Some(primary_span),
            Box::default(),
            Some(controller_key.into()),
        )
    }

    pub(crate) fn empty_signal_controller_phases(
        controller_key: &str,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::EmptySignalControllerPhases,
            DiagnosticPayload::EmptySignalControllerPhases {
                signal_controller_key: controller_key.into(),
            },
            Some(primary_span),
            Box::default(),
            Some(controller_key.into()),
        )
    }

    pub(crate) fn duplicate_signal_controller_group(
        controller_key: &str,
        group_key: &str,
        primary_span: impl Into<SourceLocation>,
        first_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::DuplicateSignalControllerGroup,
            DiagnosticPayload::DuplicateSignalControllerGroup {
                signal_controller_key: controller_key.into(),
                signal_group_key: group_key.into(),
            },
            Some(primary_span),
            Box::new([first_span.into()]),
            Some(controller_key.into()),
        )
    }

    pub(crate) fn signal_group_multiple_controllers(
        group_key: &str,
        first_controller_key: &str,
        duplicate_controller_key: &str,
        primary_span: impl Into<SourceLocation>,
        first_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::SignalGroupMultipleControllers,
            DiagnosticPayload::SignalGroupMultipleControllers {
                signal_group_key: group_key.into(),
                first_controller_key: first_controller_key.into(),
                duplicate_controller_key: duplicate_controller_key.into(),
            },
            Some(primary_span),
            Box::new([first_span.into()]),
            Some(group_key.into()),
        )
    }

    pub(crate) fn unowned_signal_group(
        group_key: &str,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::UnownedSignalGroup,
            DiagnosticPayload::UnownedSignalGroup {
                signal_group_key: group_key.into(),
            },
            Some(primary_span),
            Box::default(),
            Some(group_key.into()),
        )
    }

    pub(crate) fn unused_signal_group(
        group_key: &str,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::UnusedSignalGroup,
            DiagnosticPayload::UnusedSignalGroup {
                signal_group_key: group_key.into(),
            },
            Some(primary_span),
            Box::default(),
            Some(group_key.into()),
        )
    }

    pub(crate) fn duplicate_signal_phase_key(
        controller_key: &str,
        phase_key: &str,
        primary_span: impl Into<SourceLocation>,
        first_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::DuplicateSignalPhaseKey,
            DiagnosticPayload::DuplicateSignalPhaseKey {
                signal_controller_key: controller_key.into(),
                signal_phase_key: phase_key.into(),
            },
            Some(primary_span),
            Box::new([first_span.into()]),
            Some(controller_key.into()),
        )
    }

    pub(crate) fn invalid_signal_phase_duration(
        controller_key: &str,
        phase_key: &str,
        duration_ms: u64,
        max_inclusive: u64,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::InvalidSignalPhaseDuration,
            DiagnosticPayload::InvalidSignalPhaseDuration {
                signal_controller_key: controller_key.into(),
                signal_phase_key: phase_key.into(),
                duration_ms,
                max_inclusive,
            },
            Some(primary_span),
            Box::default(),
            Some(controller_key.into()),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn duplicate_signal_phase_group(
        controller_key: &str,
        phase_key: &str,
        group_key: &str,
        primary_span: impl Into<SourceLocation>,
        first_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::DuplicateSignalPhaseGroup,
            DiagnosticPayload::DuplicateSignalPhaseGroup {
                signal_controller_key: controller_key.into(),
                signal_phase_key: phase_key.into(),
                signal_group_key: group_key.into(),
            },
            Some(primary_span),
            Box::new([first_span.into()]),
            Some(controller_key.into()),
        )
    }

    pub(crate) fn unknown_signal_phase_group(
        controller_key: &str,
        phase_key: &str,
        group_key: &str,
        primary_span: impl Into<SourceLocation>,
        controller_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::UnknownSignalPhaseGroup,
            DiagnosticPayload::UnknownSignalPhaseGroup {
                signal_controller_key: controller_key.into(),
                signal_phase_key: phase_key.into(),
                signal_group_key: group_key.into(),
            },
            Some(primary_span),
            Box::new([controller_span.into()]),
            Some(controller_key.into()),
        )
    }

    pub(crate) fn missing_signal_phase_group(
        controller_key: &str,
        phase_key: &str,
        group_key: &str,
        primary_span: impl Into<SourceLocation>,
        group_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::MissingSignalPhaseGroup,
            DiagnosticPayload::MissingSignalPhaseGroup {
                signal_controller_key: controller_key.into(),
                signal_phase_key: phase_key.into(),
                signal_group_key: group_key.into(),
            },
            Some(primary_span),
            Box::new([group_span.into()]),
            Some(controller_key.into()),
        )
    }

    pub(crate) fn signal_cycle_duration_overflow(
        controller_key: &str,
        max_inclusive: u64,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::SignalCycleDurationOverflow,
            DiagnosticPayload::SignalCycleDurationOverflow {
                signal_controller_key: controller_key.into(),
                max_inclusive,
            },
            Some(primary_span),
            Box::default(),
            Some(controller_key.into()),
        )
    }

    pub(crate) fn invalid_signal_controller_offset(
        controller_key: &str,
        offset_ms: u64,
        cycle_duration_ms: u64,
        max_inclusive: u64,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::InvalidSignalControllerOffset,
            DiagnosticPayload::InvalidSignalControllerOffset {
                signal_controller_key: controller_key.into(),
                offset_ms,
                cycle_duration_ms,
                max_inclusive,
            },
            Some(primary_span),
            Box::default(),
            Some(controller_key.into()),
        )
    }

    pub(crate) fn invalid_parking_anchor_progress(
        parking_space_key: &str,
        role: ParkingAnchorRole,
        lane_edge_key: &str,
        progress_meters: f64,
        edge_length_meters: f64,
        endpoint_clearance_meters: f64,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::InvalidParkingAnchorProgress,
            DiagnosticPayload::InvalidParkingAnchorProgress {
                parking_space_key: parking_space_key.into(),
                role,
                lane_edge_key: lane_edge_key.into(),
                progress_bits: progress_meters.to_bits(),
                edge_length_bits: edge_length_meters.to_bits(),
                endpoint_clearance_bits: endpoint_clearance_meters.to_bits(),
            },
            Some(primary_span),
            Box::default(),
            Some(parking_space_key.into()),
        )
    }

    pub(crate) fn invalid_parking_space_geometry(
        parking_space_key: &str,
        field: ParkingGeometryField,
        value: f64,
        violation: ParkingGeometryViolation,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::InvalidParkingSpaceGeometry,
            DiagnosticPayload::InvalidParkingSpaceGeometry {
                parking_space_key: parking_space_key.into(),
                field,
                value_bits: value.to_bits(),
                violation,
            },
            Some(primary_span),
            Box::default(),
            Some(parking_space_key.into()),
        )
    }

    pub(crate) fn orphan_parking_area(
        parking_area_key: &str,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::OrphanParkingArea,
            DiagnosticPayload::OrphanParkingArea {
                parking_area_key: parking_area_key.into(),
            },
            Some(primary_span),
            Box::default(),
            Some(parking_area_key.into()),
        )
    }

    pub(crate) fn participant_class_inheritance_cycle(
        participant_class_key: &str,
        primary_span: impl Into<SourceLocation>,
        related_spans: Box<[impl Into<SourceLocation>]>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::ParticipantClassInheritanceCycle,
            DiagnosticPayload::ParticipantClassInheritanceCycle {
                participant_class_key: participant_class_key.into(),
            },
            Some(primary_span),
            related_spans
                .into_vec()
                .into_iter()
                .map(Into::into)
                .collect(),
            Some(participant_class_key.into()),
        )
    }

    pub(crate) fn empty_access_rule_participant_classes(
        access_rule_key: &str,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::EmptyAccessRuleParticipantClasses,
            DiagnosticPayload::EmptyAccessRuleParticipantClasses {
                access_rule_key: access_rule_key.into(),
            },
            Some(primary_span),
            Box::default(),
            Some(access_rule_key.into()),
        )
    }

    pub(crate) fn access_capability_unavailable(
        access_rule_key: &str,
        capability: AccessCapability,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::AccessCapabilityUnavailable,
            DiagnosticPayload::AccessCapabilityUnavailable {
                access_rule_key: access_rule_key.into(),
                capability,
            },
            Some(primary_span),
            Box::default(),
            Some(access_rule_key.into()),
        )
    }

    pub(crate) fn invalid_access_regulation_string(
        access_rule_key: &str,
        field: AccessRegulationField,
        character_count: u32,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::InvalidAccessRegulationString,
            DiagnosticPayload::InvalidAccessRegulationString {
                access_rule_key: access_rule_key.into(),
                field,
                character_count,
            },
            Some(primary_span),
            Box::default(),
            Some(access_rule_key.into()),
        )
    }

    #[expect(clippy::too_many_arguments, reason = "诊断必须保留两份完整法规来源")]
    pub(crate) fn access_regulation_mismatch(
        first_rule_key: &str,
        first_jurisdiction: &str,
        first_version: &str,
        second_rule_key: &str,
        second_jurisdiction: &str,
        second_version: &str,
        primary_span: impl Into<SourceLocation>,
        related_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::AccessRegulationMismatch,
            DiagnosticPayload::AccessRegulationMismatch {
                first_rule_key: first_rule_key.into(),
                first_jurisdiction: first_jurisdiction.into(),
                first_version: first_version.into(),
                second_rule_key: second_rule_key.into(),
                second_jurisdiction: second_jurisdiction.into(),
                second_version: second_version.into(),
            },
            Some(primary_span),
            vec![related_span.into()].into_boxed_slice(),
            Some(second_rule_key.into()),
        )
    }

    #[expect(clippy::too_many_arguments, reason = "诊断必须保留完整裁决冲突键")]
    pub(crate) fn access_rule_ambiguity(
        plane: AccessPlane,
        target_kind: EntityKind,
        target_key: &str,
        participant_class_key: &str,
        first_rule_key: &str,
        second_rule_key: &str,
        primary_span: impl Into<SourceLocation>,
        related_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::AccessRuleAmbiguity,
            DiagnosticPayload::AccessRuleAmbiguity {
                plane,
                target_kind,
                target_key: target_key.into(),
                participant_class_key: participant_class_key.into(),
                first_rule_key: first_rule_key.into(),
                second_rule_key: second_rule_key.into(),
            },
            Some(primary_span),
            vec![related_span.into()].into_boxed_slice(),
            Some(second_rule_key.into()),
        )
    }

    pub(crate) fn invalid_canonical_identity(
        entity_kind: EntityKind,
        stable_key: &str,
        violation: CanonicalIdentityViolation,
        primary_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::InvalidCanonicalIdentity,
            DiagnosticPayload::InvalidCanonicalIdentity {
                entity_kind,
                stable_key: stable_key.into(),
                violation,
            },
            Some(primary_span),
            Box::default(),
            Some(stable_key.into()),
        )
    }

    pub(crate) fn duplicate_canonical_identity(
        entity_kind: EntityKind,
        stable_key: &str,
        stable_id: StableId128,
        primary_span: impl Into<SourceLocation>,
        existing_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::DuplicateCanonicalIdentity,
            DiagnosticPayload::DuplicateCanonicalIdentity {
                entity_kind,
                stable_id,
            },
            Some(primary_span),
            Box::new([existing_span.into()]),
            Some(stable_key.into()),
        )
    }

    pub(crate) fn identity_digest_collision(
        entity_kind: EntityKind,
        stable_key: &str,
        stable_id: StableId128,
        primary_span: impl Into<SourceLocation>,
        existing_span: impl Into<SourceLocation>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::IdentityDigestCollision,
            DiagnosticPayload::IdentityDigestCollision {
                entity_kind,
                stable_id,
            },
            Some(primary_span),
            Box::new([existing_span.into()]),
            Some(stable_key.into()),
        )
    }

    pub(crate) const fn set_canonical_module_order(&mut self, order: u32) {
        self.canonical_module_order = order;
    }

    pub(crate) fn compile_limit_exceeded_at(
        dimension: CompileLimitDimension,
        limit: u64,
        observed: u64,
        primary_span: impl IntoSourceLocationOption,
        stable_key: Option<Box<str>>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::CompileLimitExceeded,
            DiagnosticPayload::CompileLimitExceeded {
                dimension,
                limit,
                observed,
            },
            primary_span,
            Box::default(),
            stable_key,
        )
    }

    fn error_with_context(
        code: DiagnosticCode,
        payload: DiagnosticPayload,
        primary_span: impl IntoSourceLocationOption,
        related_spans: Box<[SourceLocation]>,
        stable_key: Option<Box<str>>,
    ) -> Self {
        Self::error_with_location_context(
            code,
            payload,
            primary_span.into_source_location_option(),
            related_spans,
            stable_key,
        )
    }

    fn error_with_location_context(
        code: DiagnosticCode,
        payload: DiagnosticPayload,
        primary_location: Option<SourceLocation>,
        related_locations: Box<[SourceLocation]>,
        stable_key: Option<Box<str>>,
    ) -> Self {
        Self {
            canonical_module_order: 0,
            primary_span: primary_location,
            code,
            severity: DiagnosticSeverity::Error,
            payload,
            stable_key,
            related_spans: related_locations,
        }
    }

    /// 返回跨语言渲染稳定的诊断代码。
    #[must_use]
    pub const fn code(&self) -> DiagnosticCode {
        self.code
    }

    /// 返回该诊断对阶段提交的影响级别。
    #[must_use]
    pub const fn severity(&self) -> DiagnosticSeverity {
        self.severity
    }

    /// 返回机器可消费的有类型载荷。
    #[must_use]
    pub const fn payload(&self) -> &DiagnosticPayload {
        &self.payload
    }

    /// 返回主要闭合来源位置；资源或头级错误可能没有具体位置。
    #[must_use]
    pub const fn primary_location(&self) -> Option<&SourceLocation> {
        self.primary_span.as_ref()
    }

    /// 若主要位置来自文本，返回其真实文本范围。
    #[must_use]
    pub const fn primary_span(&self) -> Option<&SourceSpan> {
        match self.primary_span.as_ref() {
            Some(SourceLocation::Text(span)) => Some(span),
            Some(SourceLocation::RoadEditing(_)) | None => None,
        }
    }

    /// 返回与主要错误相关的其他闭合来源位置。
    #[must_use]
    pub fn related_locations(&self) -> &[SourceLocation] {
        &self.related_spans
    }

    /// 遍历关联位置中的文本范围；道路编辑位置不会被伪装成文本行列。
    pub fn related_spans(&self) -> impl Iterator<Item = &SourceSpan> {
        self.related_spans
            .iter()
            .filter_map(SourceLocation::text_span)
    }

    /// 返回用于规范排序和快速定位的稳定键（若该诊断与键相关）。
    #[must_use]
    pub fn stable_key(&self) -> Option<&str> {
        self.stable_key.as_deref()
    }

    /// 返回构造该单诊断时新增且由返回对象继续拥有的保守请求字节数。
    pub(crate) fn failure_owned_bytes(&self) -> u64 {
        let box_str = |value: &str| u64::try_from(value.len()).unwrap_or(u64::MAX);
        let payload_bytes = match &self.payload {
            DiagnosticPayload::InvalidRoadEditingInput { field, .. } => box_str(field),
            DiagnosticPayload::InvalidRoadEditingSource {
                field,
                expected_source_document_key,
                actual_source_document_key,
                ..
            } => field
                .as_deref()
                .map_or(0, box_str)
                .saturating_add(box_str(expected_source_document_key))
                .saturating_add(actual_source_document_key.as_deref().map_or(0, box_str)),
            DiagnosticPayload::CompileProfileIncompatible { profile_id, .. } => box_str(profile_id),
            DiagnosticPayload::DuplicateModuleNamespace { namespace } => box_str(namespace),
            DiagnosticPayload::DuplicateSourceDocumentKey {
                source_document_key,
            } => box_str(source_document_key),
            _ => 0,
        };
        let stable_key_bytes = self.stable_key.as_deref().map_or(0, box_str);
        let related_backing = u64::try_from(self.related_spans.len())
            .unwrap_or(u64::MAX)
            .saturating_mul(
                u64::try_from(core::mem::size_of::<SourceLocation>()).unwrap_or(u64::MAX),
            );
        payload_bytes
            .saturating_add(stable_key_bytes)
            .saturating_add(related_backing)
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: ", self.code.as_str())?;
        match &self.payload {
            DiagnosticPayload::InvalidRoadEditingInput { field, violation } => {
                write!(formatter, "道路编辑编制字段 {field} 非法：{violation:?}")
            }
            DiagnosticPayload::InvalidRoadEditingSource {
                violation,
                field,
                expected_source_document_key,
                actual_source_document_key,
            } => {
                write!(
                    formatter,
                    "道路编辑来源 {expected_source_document_key} 非法：{violation:?}"
                )?;
                if let Some(field) = field {
                    write!(formatter, "，字段 {field}")?;
                }
                if let Some(actual) = actual_source_document_key {
                    write!(formatter, "，wire 文档键 {actual:?}")?;
                }
                Ok(())
            }
            DiagnosticPayload::InvalidSourceHeaderField { field, violation } => {
                write!(
                    formatter,
                    "来源模块头字段 {} 非法：{}",
                    field.as_str(),
                    SourceTextViolationDisplay(*violation)
                )
            }
            DiagnosticPayload::CompileLimitExceeded {
                dimension,
                limit,
                observed,
            } => write!(
                formatter,
                "编译资源维度 {} 超过上限：允许 {limit}，实际 {observed}",
                dimension.as_str()
            ),
            DiagnosticPayload::CompileProfileIncompatible {
                profile_id,
                required_dimension,
            } => write!(
                formatter,
                "编译资源配置档 {profile_id} 不支持必需维度 {}",
                required_dimension.as_str()
            ),
            DiagnosticPayload::InvalidImportNamespace { violation } => write!(
                formatter,
                "导入模块命名空间非法：{}",
                SourceTextViolationDisplay(*violation)
            ),
            DiagnosticPayload::DuplicateImport { namespace } => {
                write!(formatter, "来源模块重复导入 {namespace}")
            }
            DiagnosticPayload::DuplicateModuleNamespace { namespace } => {
                write!(formatter, "编译单元包含重复模块命名空间 {namespace}")
            }
            DiagnosticPayload::DuplicateSourceDocumentKey {
                source_document_key,
            } => write!(
                formatter,
                "编译单元包含重复来源文档键 {source_document_key}"
            ),
            DiagnosticPayload::SourceDocumentOwnershipMismatch {
                source_document_key,
                expected_authoring_namespace_id,
                actual_authoring_namespace_id,
            } => match actual_authoring_namespace_id {
                Some(actual) => write!(
                    formatter,
                    "来源文档 {source_document_key} 属于逻辑模块 {actual}，不能用于 {expected_authoring_namespace_id} 的来源位置"
                ),
                None => write!(
                    formatter,
                    "来源文档 {source_document_key} 未登记在逻辑模块 {expected_authoring_namespace_id} 的文档集中"
                ),
            },
            DiagnosticPayload::UnknownImport { namespace } => {
                write!(formatter, "导入目标模块 {namespace} 不存在")
            }
            DiagnosticPayload::ImportCycle { namespaces } => write!(
                formatter,
                "来源模块导入形成循环：{}",
                namespaces
                    .iter()
                    .map(AsRef::as_ref)
                    .collect::<Vec<&str>>()
                    .join(" -> ")
            ),
            DiagnosticPayload::InvalidDeclarationKey {
                entity_kind,
                violation,
            } => write!(
                formatter,
                "{} 声明的稳定键非法：{}",
                entity_kind.slug(),
                SourceTextViolationDisplay(*violation)
            ),
            DiagnosticPayload::DuplicateDeclaration {
                entity_kind,
                stable_key,
            } => write!(
                formatter,
                "来源模块重复声明 {} 稳定键 {stable_key}",
                entity_kind.slug()
            ),
            DiagnosticPayload::InvalidIdentityAsciiField {
                entity_kind,
                stable_key,
                field_tag,
                violation,
            } => write!(
                formatter,
                "{} 声明 {stable_key} 的 Identity v1 字段 {} 非法：{}",
                entity_kind.slug(),
                field_tag.name(),
                SourceTextViolationDisplay(*violation)
            ),
            DiagnosticPayload::InvalidReferenceNamespace { violation } => write!(
                formatter,
                "引用目标模块命名空间非法：{}",
                SourceTextViolationDisplay(*violation)
            ),
            DiagnosticPayload::InvalidReferenceKey {
                entity_kind,
                violation,
            } => write!(
                formatter,
                "指向 {} 声明的引用键非法：{}",
                entity_kind.slug(),
                SourceTextViolationDisplay(*violation)
            ),
            DiagnosticPayload::UnimportedReferenceModule { namespace } => {
                write!(
                    formatter,
                    "引用目标模块 {namespace} 未被当前来源模块显式导入"
                )
            }
            DiagnosticPayload::UnknownReferenceTarget {
                entity_kind,
                source_key,
                target_namespace,
                target_owner_local_keys,
                target_key,
            } => {
                write!(
                    formatter,
                    "{} 声明 {source_key} 引用了不存在的目标 {target_namespace}::",
                    entity_kind.slug()
                )?;
                for owner_key in target_owner_local_keys {
                    write!(formatter, "{owner_key}>")?;
                }
                formatter.write_str(target_key)
            }
            DiagnosticPayload::InvalidLaneEdgeLength {
                stable_key,
                value_bits,
                violation,
            } => write!(
                formatter,
                "车道图边 {stable_key} 的长度 {} 非法：{}",
                f64::from_bits(*value_bits),
                ScalarViolationDisplay(*violation)
            ),
            DiagnosticPayload::InvalidLaneEdgeSpeedLimit {
                stable_key,
                value_bits,
                violation,
            } => write!(
                formatter,
                "车道图边 {stable_key} 的基础道路限速 {} 非法：{}",
                f64::from_bits(*value_bits),
                ScalarViolationDisplay(*violation)
            ),
            DiagnosticPayload::DuplicateLaneEdgeSuccessor {
                stable_key,
                target_namespace,
                target_key,
            } => write!(
                formatter,
                "车道图边 {stable_key} 重复声明下游连接 {target_namespace}:{target_key}"
            ),
            DiagnosticPayload::InvalidFacilityKind {
                entity_kind,
                stable_key,
                kind_id,
                expected_category,
                violation,
            } => write!(
                formatter,
                "{} 声明 {stable_key} 的物理设施类别 {kind_id} 不能用于 {}：{}",
                entity_kind.slug(),
                match expected_category {
                    FacilityKindCategory::LaneBearing => "承载车道的道路区段",
                    FacilityKindCategory::NonTraversable => "非遍历设施带",
                },
                FacilityKindViolationDisplay(*violation)
            ),
            DiagnosticPayload::EmptyRoadSectionLanes { stable_key } => {
                write!(formatter, "道路区段 {stable_key} 必须至少声明一条编制车道")
            }
            DiagnosticPayload::EmptyAuthoringLaneEdgeChain { stable_key } => {
                write!(formatter, "编制车道 {stable_key} 必须至少覆盖一条车道图边")
            }
            DiagnosticPayload::DuplicateAuthoringLaneEdge {
                stable_key,
                target_namespace,
                target_key,
            } => write!(
                formatter,
                "编制车道 {stable_key} 重复覆盖车道图边 {target_namespace}:{target_key}"
            ),
            DiagnosticPayload::EmptyRoadCorridorElements { stable_key } => {
                write!(
                    formatter,
                    "道路走廊 {stable_key} 必须至少声明一个横断面成员"
                )
            }
            DiagnosticPayload::DuplicateRoadCorridorElement {
                stable_key,
                target_kind,
                target_namespace,
                target_key,
            } => write!(
                formatter,
                "道路走廊 {stable_key} 重复引用 {} 成员 {target_namespace}:{target_key}",
                target_kind.slug()
            ),
            DiagnosticPayload::MissingCrossSectionOwner {
                entity_kind,
                stable_key,
            } => write!(
                formatter,
                "{} 声明 {stable_key} 没有道路走廊所有者，无法派生父项锚定身份",
                entity_kind.slug()
            ),
            DiagnosticPayload::MultipleCrossSectionOwners {
                entity_kind,
                stable_key,
                first_owner_key,
                second_owner_key,
            } => write!(
                formatter,
                "{} 声明 {stable_key} 同时被道路走廊 {first_owner_key} 与 {second_owner_key} 拥有",
                entity_kind.slug()
            ),
            DiagnosticPayload::InvalidCorridorReferenceSection {
                corridor_key,
                target_namespace,
                target_key,
            } => write!(
                formatter,
                "道路走廊 {corridor_key} 的参考道路区段 {target_namespace}:{target_key} 不在自身横断面成员中"
            ),
            DiagnosticPayload::DisconnectedAuthoringLaneEdgeChain {
                lane_key,
                predecessor_key,
                successor_key,
            } => write!(
                formatter,
                "编制车道 {lane_key} 的相邻覆盖边 {predecessor_key} 与 {successor_key} 没有直接连接"
            ),
            DiagnosticPayload::MultipleAuthoringLaneOwners {
                edge_key,
                first_lane_key,
                second_lane_key,
            } => write!(
                formatter,
                "车道图边 {edge_key} 同时被编制车道 {first_lane_key} 与 {second_lane_key} 覆盖"
            ),
            DiagnosticPayload::LaneGroupParentMismatch {
                lane_key,
                lane_group_key,
                lane_section_key,
                group_section_key,
            } => write!(
                formatter,
                "编制车道 {lane_key} 属于道路区段 {lane_section_key}，但引用的车道组 {lane_group_key} 属于 {group_section_key}"
            ),
            DiagnosticPayload::EmptyLaneGroup { stable_key } => {
                write!(formatter, "车道组 {stable_key} 必须至少包含一条编制车道")
            }
            DiagnosticPayload::EmptyJunction { junction_key } => {
                write!(formatter, "路口 {junction_key} 必须至少包含一个通行流向")
            }
            DiagnosticPayload::EmptyMovement { movement_key } => {
                write!(
                    formatter,
                    "通行流向 {movement_key} 必须至少包含一条机动路径"
                )
            }
            DiagnosticPayload::DisconnectedManeuverPath {
                path_key,
                predecessor_key,
                successor_key,
            } => write!(
                formatter,
                "机动路径 {path_key} 的相邻车道图边 {predecessor_key} -> {successor_key} 未直接连通"
            ),
            DiagnosticPayload::DuplicateManeuverPathSequence {
                first_path_key,
                duplicate_path_key,
                first_junction_key,
                duplicate_junction_key,
            } => write!(
                formatter,
                "机动路径 {duplicate_path_key}（路口 {duplicate_junction_key}）与 {first_path_key}（路口 {first_junction_key}）声明了相同完整遍历序列"
            ),
            DiagnosticPayload::InternalEdgeJunctionConflict {
                edge_key,
                first_junction_key,
                duplicate_junction_key,
                first_path_key,
                duplicate_path_key,
            } => write!(
                formatter,
                "车道图边 {edge_key} 被路口 {first_junction_key} 的路径 {first_path_key} 与路口 {duplicate_junction_key} 的路径 {duplicate_path_key} 同时声明为内部边"
            ),
            DiagnosticPayload::InternalBoundaryRoleConflict {
                edge_key,
                internal_path_key,
                boundary_path_key,
            } => write!(
                formatter,
                "车道图边 {edge_key} 同时被路径 {internal_path_key} 声明为内部边、被路径 {boundary_path_key} 声明为边界边"
            ),
            DiagnosticPayload::JunctionEdgeSetMismatch {
                junction_key,
                edge_key,
                path_key,
                violation,
            } => {
                write!(formatter, "路口 {junction_key} 的车道图边 {edge_key}")?;
                if let Some(path_key) = path_key {
                    write!(formatter, "（机动路径 {path_key}）")?;
                }
                formatter.write_str(match violation {
                    JunctionEdgeSetViolation::ApproachNotSectionDerived => {
                        "不是道路区段派生边，不能声明为 approach edge"
                    }
                    JunctionEdgeSetViolation::InternalIsSectionDerived => {
                        "是道路区段派生边，不能声明为 junction-internal edge"
                    }
                    JunctionEdgeSetViolation::BoundaryNotDeclaredApproach => {
                        "被路径用作边界，但不在显式 approachEdges 集中"
                    }
                    JunctionEdgeSetViolation::InternalNotDeclared => {
                        "被路径用作内部边，但不在显式 internalEdges 集中"
                    }
                    JunctionEdgeSetViolation::DeclaredInternalUnused => {
                        "出现在显式 internalEdges 集中，但不属于该路口任何路径"
                    }
                    JunctionEdgeSetViolation::ApproachClaimedInternal => {
                        "同时属于全局 approach 与 junction-internal 角色"
                    }
                })
            }
            DiagnosticPayload::ManeuverGateTransitionOutOfRange {
                maneuver_gate_key,
                maneuver_path_key,
                transition_index,
                transition_count,
            } => write!(
                formatter,
                "机动门 {maneuver_gate_key} 的 transitionIndex={transition_index} 越界：机动路径 {maneuver_path_key} 只有 {transition_count} 个转换"
            ),
            DiagnosticPayload::DuplicateManeuverGatePathTransition {
                maneuver_path_key,
                transition_index,
                first_maneuver_gate_key,
                duplicate_maneuver_gate_key,
            } => write!(
                formatter,
                "机动路径 {maneuver_path_key} 的转换 {transition_index} 重复声明机动门：首个为 {first_maneuver_gate_key}，重复项为 {duplicate_maneuver_gate_key}"
            ),
            DiagnosticPayload::ManeuverGateStopLineMismatch {
                maneuver_gate_key,
                stop_line_key,
                path_from_edge_key,
                stop_line_edge_key,
            } => write!(
                formatter,
                "机动门 {maneuver_gate_key} 的转换起始边 {path_from_edge_key} 与停止线 {stop_line_key} 所属边 {stop_line_edge_key} 不一致"
            ),
            DiagnosticPayload::DuplicateStopLineEdge {
                edge_key,
                first_stop_line_key,
                duplicate_stop_line_key,
            } => write!(
                formatter,
                "车道图边 {edge_key} 重复声明停止线：首个为 {first_stop_line_key}，重复项为 {duplicate_stop_line_key}"
            ),
            DiagnosticPayload::OrphanStopLine {
                stop_line_key,
                edge_key,
            } => write!(
                formatter,
                "停止线 {stop_line_key} 位于终止边 {edge_key}，无法形成机动路径转换"
            ),
            DiagnosticPayload::UnreferencedStopLine {
                stop_line_key,
                edge_key,
            } => write!(
                formatter,
                "停止线 {stop_line_key} 位于非终止边 {edge_key}，但未被任何机动门引用"
            ),
            DiagnosticPayload::MissingManeuverPathCoverage {
                stop_line_key,
                from_edge_key,
                to_edge_key,
            } => write!(
                formatter,
                "停止线 {stop_line_key} 启用了入口机动门，但车道图转换 {from_edge_key} -> {to_edge_key} 没有候选机动路径"
            ),
            DiagnosticPayload::MissingManeuverGateCoverage {
                stop_line_key,
                edge_key,
                maneuver_path_key,
            } => write!(
                formatter,
                "停止线 {stop_line_key} 所在边 {edge_key} 的候选机动路径 {maneuver_path_key} 缺少 transitionIndex=0 入口机动门"
            ),
            DiagnosticPayload::InvalidWaitingZoneCapacity { waiting_zone_key } => write!(
                formatter,
                "等待区 {waiting_zone_key} 的 maxOccupancy 必须大于零"
            ),
            DiagnosticPayload::WaitingZoneGatePathMismatch {
                waiting_zone_key,
                gate_role,
                gate_key,
                declared_path_key,
                gate_path_key,
            } => write!(
                formatter,
                "等待区 {waiting_zone_key} 的 {} {gate_key} 属于机动路径 {gate_path_key}，与声明路径 {declared_path_key} 不一致",
                gate_role.as_str()
            ),
            DiagnosticPayload::InvalidWaitingZoneGateOrder {
                waiting_zone_key,
                entry_transition_index,
                release_transition_index,
            } => write!(
                formatter,
                "等待区 {waiting_zone_key} 的入口转换 {entry_transition_index} 必须严格早于释放转换 {release_transition_index}"
            ),
            DiagnosticPayload::OverlappingWaitingZones {
                maneuver_path_key,
                first_waiting_zone_key,
                second_waiting_zone_key,
            } => write!(
                formatter,
                "机动路径 {maneuver_path_key} 上的等待区 {first_waiting_zone_key} 与 {second_waiting_zone_key} 内部重叠或嵌套"
            ),
            DiagnosticPayload::EmptyStaticRoute { static_route_key } => {
                write!(
                    formatter,
                    "静态路线 {static_route_key} 必须至少包含一条车道图边"
                )
            }
            DiagnosticPayload::DisconnectedStaticRouteEdge {
                static_route_key,
                predecessor_key,
                successor_key,
                successor_route_edge_index,
            } => write!(
                formatter,
                "静态路线 {static_route_key} 的边出现项 {successor_route_edge_index} 不连通：{predecessor_key} -> {successor_key}"
            ),
            DiagnosticPayload::StaticRouteStartsInsideJunction {
                static_route_key,
                edge_key,
            } => write!(
                formatter,
                "静态路线 {static_route_key} 从路口内部边 {edge_key} 开始，缺少进入路口的边界转换"
            ),
            DiagnosticPayload::StaticRouteEndsInsideJunction {
                static_route_key,
                edge_key,
            } => write!(
                formatter,
                "静态路线 {static_route_key} 在路口内部边 {edge_key} 结束，缺少离开路口的边界转换"
            ),
            DiagnosticPayload::StaticRouteTerminatesAtStopLine {
                static_route_key,
                edge_key,
                stop_line_key,
            } => write!(
                formatter,
                "静态路线 {static_route_key} 在带停止线 {stop_line_key} 的边 {edge_key} 结束，缺少受控后继转换"
            ),
            DiagnosticPayload::StaticRouteManeuverNoFullMatch {
                static_route_key,
                entry_route_edge_index,
                entry_edge_key,
                next_edge_key,
            } => write!(
                formatter,
                "静态路线 {static_route_key} 从出现项 {entry_route_edge_index} 的转换 {entry_edge_key} -> {next_edge_key} 进入机动路径，但没有完整匹配任何候选路径"
            ),
            DiagnosticPayload::StaticRouteManeuverMultipleFullMatches {
                static_route_key,
                entry_route_edge_index,
                first_path_key,
                second_path_key,
            } => write!(
                formatter,
                "静态路线 {static_route_key} 从出现项 {entry_route_edge_index} 同时完整匹配机动路径 {first_path_key} 与 {second_path_key}"
            ),
            DiagnosticPayload::StaticRouteManeuverInternalOverlap {
                static_route_key,
                route_edge_index,
                edge_key,
                first_path_key,
                second_path_key,
            } => write!(
                formatter,
                "静态路线 {static_route_key} 的内部边出现项 {route_edge_index}（{edge_key}）同时被机动路径 {first_path_key} 与 {second_path_key} 覆盖"
            ),
            DiagnosticPayload::StaticRouteInternalEdgeUncovered {
                static_route_key,
                route_edge_index,
                edge_key,
            } => write!(
                formatter,
                "静态路线 {static_route_key} 的路口内部边出现项 {route_edge_index}（{edge_key}）未被完整机动路径覆盖"
            ),
            DiagnosticPayload::EmptySignalControllerGroups {
                signal_controller_key,
            } => write!(
                formatter,
                "信号控制器 {signal_controller_key} 必须至少拥有一个信号组"
            ),
            DiagnosticPayload::EmptySignalControllerPhases {
                signal_controller_key,
            } => write!(
                formatter,
                "信号控制器 {signal_controller_key} 必须至少声明一个相位"
            ),
            DiagnosticPayload::DuplicateSignalControllerGroup {
                signal_controller_key,
                signal_group_key,
            } => write!(
                formatter,
                "信号控制器 {signal_controller_key} 重复拥有信号组 {signal_group_key}"
            ),
            DiagnosticPayload::SignalGroupMultipleControllers {
                signal_group_key,
                first_controller_key,
                duplicate_controller_key,
            } => write!(
                formatter,
                "信号组 {signal_group_key} 同时被控制器 {first_controller_key} 与 {duplicate_controller_key} 拥有"
            ),
            DiagnosticPayload::UnownedSignalGroup { signal_group_key } => write!(
                formatter,
                "信号组 {signal_group_key} 没有唯一的信号控制器所有者"
            ),
            DiagnosticPayload::UnusedSignalGroup { signal_group_key } => {
                write!(formatter, "信号组 {signal_group_key} 未被任何机动门使用")
            }
            DiagnosticPayload::DuplicateSignalPhaseKey {
                signal_controller_key,
                signal_phase_key,
            } => write!(
                formatter,
                "信号控制器 {signal_controller_key} 重复声明相位键 {signal_phase_key}"
            ),
            DiagnosticPayload::InvalidSignalPhaseDuration {
                signal_controller_key,
                signal_phase_key,
                duration_ms,
                max_inclusive,
            } => write!(
                formatter,
                "信号控制器 {signal_controller_key} 的相位 {signal_phase_key} 持续时间 {duration_ms} ms 非法：必须位于 1..={max_inclusive}"
            ),
            DiagnosticPayload::DuplicateSignalPhaseGroup {
                signal_controller_key,
                signal_phase_key,
                signal_group_key,
            } => write!(
                formatter,
                "信号控制器 {signal_controller_key} 的相位 {signal_phase_key} 重复声明信号组 {signal_group_key}"
            ),
            DiagnosticPayload::UnknownSignalPhaseGroup {
                signal_controller_key,
                signal_phase_key,
                signal_group_key,
            } => write!(
                formatter,
                "信号控制器 {signal_controller_key} 的相位 {signal_phase_key} 引用了不属于该控制器的信号组 {signal_group_key}"
            ),
            DiagnosticPayload::MissingSignalPhaseGroup {
                signal_controller_key,
                signal_phase_key,
                signal_group_key,
            } => write!(
                formatter,
                "信号控制器 {signal_controller_key} 的相位 {signal_phase_key} 缺少信号组 {signal_group_key} 的状态"
            ),
            DiagnosticPayload::SignalCycleDurationOverflow {
                signal_controller_key,
                max_inclusive,
            } => write!(
                formatter,
                "信号控制器 {signal_controller_key} 的循环持续时间超过可移植上限 {max_inclusive} ms"
            ),
            DiagnosticPayload::InvalidSignalControllerOffset {
                signal_controller_key,
                offset_ms,
                cycle_duration_ms,
                max_inclusive,
            } => write!(
                formatter,
                "信号控制器 {signal_controller_key} 的偏移 {offset_ms} ms 非法：必须不超过 {max_inclusive} 且严格小于循环时长 {cycle_duration_ms} ms"
            ),
            DiagnosticPayload::InvalidParkingAnchorProgress {
                parking_space_key,
                role,
                lane_edge_key,
                progress_bits,
                edge_length_bits,
                endpoint_clearance_bits,
            } => write!(
                formatter,
                "停车位 {parking_space_key} 的 {} 锚点在车道图边 {lane_edge_key} 上进度为 {} m；必须有限且严格位于 ({}, {}) m",
                role.as_str(),
                f64::from_bits(*progress_bits),
                f64::from_bits(*endpoint_clearance_bits),
                f64::from_bits(*edge_length_bits) - f64::from_bits(*endpoint_clearance_bits),
            ),
            DiagnosticPayload::InvalidParkingSpaceGeometry {
                parking_space_key,
                field,
                value_bits,
                violation,
            } => write!(
                formatter,
                "停车位 {parking_space_key} 的 {}={} 非法：{}",
                field.as_str(),
                f64::from_bits(*value_bits),
                ParkingGeometryViolationDisplay(*violation),
            ),
            DiagnosticPayload::OrphanParkingArea { parking_area_key } => {
                write!(formatter, "停车区域 {parking_area_key} 没有任何停车位成员")
            }
            DiagnosticPayload::ParticipantClassInheritanceCycle {
                participant_class_key,
            } => write!(
                formatter,
                "参与者类别 {participant_class_key} 所在的单继承链形成循环"
            ),
            DiagnosticPayload::InvalidVehicleProfileValue {
                vehicle_profile_key,
                field,
                value_bits,
                violation,
            } => write!(
                formatter,
                "车辆配置 {vehicle_profile_key} 的 {field}={} 非法：{}",
                f64::from_bits(*value_bits),
                ScalarViolationDisplay(*violation),
            ),
            DiagnosticPayload::InvalidVehicleProfileDecelerationOrder {
                vehicle_profile_key,
                comfortable_deceleration_bits,
                emergency_deceleration_bits,
            } => write!(
                formatter,
                "车辆配置 {vehicle_profile_key} 的 emergencyDeceleration={} 必须不小于 comfortableDeceleration={}",
                f64::from_bits(*emergency_deceleration_bits),
                f64::from_bits(*comfortable_deceleration_bits),
            ),
            DiagnosticPayload::InvalidSpatialGeometry {
                canonical_frame_key,
                lane_edge_key,
                related_lane_edge_key,
                violation,
            } => {
                write!(formatter, "车道图边 {lane_edge_key} 的规范空间几何")?;
                if let Some(frame_key) = canonical_frame_key {
                    write!(formatter, "（规范坐标框架 {frame_key}）")?;
                }
                if let Some(related_key) = related_lane_edge_key {
                    write!(formatter, "（关联边 {related_key}）")?;
                }
                write!(
                    formatter,
                    "非法：{}",
                    SpatialGeometryViolationDisplay(*violation)
                )
            }
            DiagnosticPayload::InvalidFacilityBandGeometry {
                canonical_frame_key,
                facility_band_key,
                violation,
            } => {
                write!(formatter, "设施带 {facility_band_key} 的规范空间几何")?;
                if let Some(frame_key) = canonical_frame_key {
                    write!(formatter, "（规范坐标框架 {frame_key}）")?;
                }
                write!(
                    formatter,
                    "非法：{}",
                    SpatialGeometryViolationDisplay(*violation)
                )
            }
            DiagnosticPayload::EmptyAccessRuleParticipantClasses { access_rule_key } => write!(
                formatter,
                "准入规则 {access_rule_key} 必须至少引用一个参与者类别"
            ),
            DiagnosticPayload::AccessCapabilityUnavailable {
                access_rule_key,
                capability,
            } => write!(
                formatter,
                "准入规则 {access_rule_key} 请求的能力 {} 在首版静态编译中不可用",
                capability.as_str()
            ),
            DiagnosticPayload::InvalidAccessRegulationString {
                access_rule_key,
                field,
                character_count,
            } => write!(
                formatter,
                "准入规则 {access_rule_key} 的法规来源字段 {} 含 {character_count} 个字符；必须位于 1 到 128 个字符",
                field.as_str()
            ),
            DiagnosticPayload::AccessRegulationMismatch {
                first_rule_key,
                first_jurisdiction,
                first_version,
                second_rule_key,
                second_jurisdiction,
                second_version,
            } => write!(
                formatter,
                "准入规则 {second_rule_key} 的法规来源 ({second_jurisdiction}, {second_version}) 与规则 {first_rule_key} 的 ({first_jurisdiction}, {first_version}) 不一致"
            ),
            DiagnosticPayload::AccessRuleAmbiguity {
                plane,
                target_kind,
                target_key,
                participant_class_key,
                first_rule_key,
                second_rule_key,
            } => write!(
                formatter,
                "准入{}平面中，{} {target_key} 对参与者类别 {participant_class_key} 的规则 {first_rule_key} 与 {second_rule_key} 完全并列但效果相反",
                plane.as_str(),
                target_kind.slug()
            ),
            DiagnosticPayload::InvalidCanonicalIdentity {
                entity_kind,
                stable_key,
                violation,
            } => write!(
                formatter,
                "{} 声明 {stable_key} 的规范身份非法：{}",
                entity_kind.slug(),
                CanonicalIdentityViolationDisplay(*violation)
            ),
            DiagnosticPayload::DuplicateCanonicalIdentity {
                entity_kind,
                stable_id,
            } => write!(
                formatter,
                "{} 完整规范身份重复，StableId128 为 {stable_id:x}",
                entity_kind.slug()
            ),
            DiagnosticPayload::IdentityDigestCollision {
                entity_kind,
                stable_id,
            } => write!(
                formatter,
                "{} 的不同规范身份产生相同 StableId128 {stable_id:x}",
                entity_kind.slug()
            ),
        }
    }
}

impl AccessCapability {
    const fn as_str(self) -> &'static str {
        match self {
            Self::FacilityBandTarget => "facilityBandTarget",
            Self::TimeWindows => "timeWindows",
        }
    }
}

impl AccessPlane {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Edge => "车道图边",
            Self::ManeuverPath => "机动路径",
        }
    }
}

impl AccessRegulationField {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Jurisdiction => "jurisdiction",
            Self::Version => "version",
            Self::Source => "source",
        }
    }
}

struct CanonicalIdentityViolationDisplay(CanonicalIdentityViolation);

impl fmt::Display for CanonicalIdentityViolationDisplay {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            CanonicalIdentityViolation::FieldCountMismatch { expected, actual } => {
                write!(formatter, "字段数不匹配，要求 {expected}，实际 {actual}")
            }
            CanonicalIdentityViolation::UnknownFieldTag { position, tag } => {
                write!(formatter, "字段位置 {position} 使用未知标签 {tag}")
            }
            CanonicalIdentityViolation::UnexpectedFieldTag {
                position,
                expected,
                actual,
            } => write!(
                formatter,
                "字段位置 {position} 要求标签 {expected}，实际为 {actual}"
            ),
            CanonicalIdentityViolation::InvalidAsciiField { tag, violation } => write!(
                formatter,
                "标签 {tag} 的 ASCII 值非法：{}",
                SourceTextViolationDisplay(violation)
            ),
            CanonicalIdentityViolation::InvalidStableIdLength { tag, actual } => write!(
                formatter,
                "标签 {tag} 的 StableId128 必须为 16 字节，实际为 {actual}"
            ),
            CanonicalIdentityViolation::FieldByteLengthOverflow { tag, actual } => write!(
                formatter,
                "标签 {tag} 的字段字节数不能写入 u32，实际为 {actual}"
            ),
            CanonicalIdentityViolation::CanonicalByteLengthOverflow { actual } => write!(
                formatter,
                "规范身份总字节数不能由当前平台表示，实际为 {actual}"
            ),
        }
    }
}

struct ScalarViolationDisplay(ScalarViolation);

impl fmt::Display for ScalarViolationDisplay {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            ScalarViolation::NotFinite => formatter.write_str("必须是有限数"),
            ScalarViolation::NotGreaterThan {
                exclusive_minimum_bits,
            } => write!(
                formatter,
                "必须严格大于 {}",
                f64::from_bits(exclusive_minimum_bits)
            ),
            ScalarViolation::NotLessThan {
                inclusive_minimum_bits,
            } => write!(
                formatter,
                "必须大于或等于 {}",
                f64::from_bits(inclusive_minimum_bits)
            ),
        }
    }
}

struct ParkingGeometryViolationDisplay(ParkingGeometryViolation);

impl fmt::Display for ParkingGeometryViolationDisplay {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            ParkingGeometryViolation::NotFinite => formatter.write_str("必须是有限数"),
            ParkingGeometryViolation::AbsoluteNotGreaterThan {
                exclusive_minimum_bits,
            } => write!(
                formatter,
                "绝对值必须严格大于 {}",
                f64::from_bits(exclusive_minimum_bits)
            ),
            ParkingGeometryViolation::NotGreaterThan {
                exclusive_minimum_bits,
            } => write!(
                formatter,
                "必须严格大于 {}",
                f64::from_bits(exclusive_minimum_bits)
            ),
            ParkingGeometryViolation::OutsideHalfOpenRange {
                minimum_inclusive_bits,
                maximum_exclusive_bits,
            } => write!(
                formatter,
                "必须位于 [{}, {})",
                f64::from_bits(minimum_inclusive_bits),
                f64::from_bits(maximum_exclusive_bits)
            ),
        }
    }
}

struct SpatialGeometryViolationDisplay(SpatialGeometryViolation);

impl fmt::Display for SpatialGeometryViolationDisplay {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            SpatialGeometryViolation::InsufficientPoints { minimum, actual } => {
                write!(formatter, "中心线至少需要 {minimum} 个点，实际为 {actual}")
            }
            SpatialGeometryViolation::NonFiniteCoordinate {
                point_index,
                axis,
                value_bits,
            } => {
                write!(
                    formatter,
                    "第 {point_index} 点的 {}={} 不是有限数",
                    axis.as_str(),
                    f32::from_bits(value_bits)
                )
            }
            SpatialGeometryViolation::CoordinateOutOfRange {
                point_index,
                axis,
                value_bits,
                minimum_bits,
                maximum_bits,
            } => write!(
                formatter,
                "第 {point_index} 点的 {}={} 不在 [{}, {}]",
                axis.as_str(),
                f32::from_bits(value_bits),
                f32::from_bits(minimum_bits),
                f32::from_bits(maximum_bits)
            ),
            SpatialGeometryViolation::DuplicateEdgeBinding => {
                formatter.write_str("同一车道图边被重复绑定")
            }
            SpatialGeometryViolation::MissingEdgeBinding => {
                formatter.write_str("启用空间几何后该车道图边缺少中心线")
            }
            SpatialGeometryViolation::MissingGeometryProfiles => {
                formatter.write_str("已编译 authoring 几何缺少位置/方向配置档")
            }
            SpatialGeometryViolation::GeometryProfileMismatch {
                expected_accuracy_code,
                expected_direction_code,
                actual_accuracy_code,
                actual_direction_code,
            } => write!(
                formatter,
                "同一编译单元混用了几何配置档：期望 ({expected_accuracy_code}, {expected_direction_code})，实际 ({actual_accuracy_code}, {actual_direction_code})"
            ),
            SpatialGeometryViolation::MissingCanonicalFrame => {
                formatter.write_str("中心线没有可解析或可从机动路径唯一推导的 canonical frame")
            }
            SpatialGeometryViolation::ManeuverPathFrameMismatch => {
                formatter.write_str("同一机动路径的 entry 与 exit 属于不同 canonical frame")
            }
            SpatialGeometryViolation::InternalEdgeFrameConflict => {
                formatter.write_str("共享 internal edge 从不同机动路径推导出冲突 canonical frame")
            }
            SpatialGeometryViolation::DegenerateSegment {
                segment_index,
                length_bits,
                minimum_bits,
            } => write!(
                formatter,
                "第 {segment_index} 线段长度 {} 必须严格大于 {} 米",
                f32::from_bits(length_bits),
                f32::from_bits(minimum_bits)
            ),
            SpatialGeometryViolation::DegenerateProjectedUp {
                segment_index,
                projected_up_bits,
                minimum_bits,
            } => write!(
                formatter,
                "第 {segment_index} 线段的水平投影长度 {} 必须不小于 {}",
                f32::from_bits(projected_up_bits),
                f32::from_bits(minimum_bits)
            ),
            SpatialGeometryViolation::ArcLengthAccumulationFailed {
                segment_index,
                accumulated_bits,
                segment_length_bits,
            } => write!(
                formatter,
                "第 {segment_index} 线段无法把长度 {} 米累加到 {} 米并保持有限且严格递增",
                f32::from_bits(segment_length_bits),
                f32::from_bits(accumulated_bits)
            ),
            SpatialGeometryViolation::LengthMismatch {
                expected_length_bits,
                geometry_length_bits,
                tolerance_bits,
            } => write!(
                formatter,
                "中心线长度 {} 与声明长度 {} 米的差超过容差 {} 米",
                f32::from_bits(geometry_length_bits),
                f64::from_bits(expected_length_bits),
                f64::from_bits(tolerance_bits)
            ),
            SpatialGeometryViolation::ConnectedEdgesUseDifferentFrames => {
                formatter.write_str("直接连接的两条边属于不同 canonical frame")
            }
            SpatialGeometryViolation::DiscontinuousJoin {
                distance_bits,
                tolerance_bits,
            } => write!(
                formatter,
                "连接端点距离 {} 米超过容差 {} 米",
                f64::from_bits(distance_bits),
                f64::from_bits(tolerance_bits)
            ),
            SpatialGeometryViolation::DirectionDiscontinuity {
                dot_bits,
                lhs_bits,
                rhs_bits,
            } => write!(
                formatter,
                "相连 edge 方向不连续：dot={}，dot²={}，最低加权范数={}",
                f64::from_bits(dot_bits),
                f64::from_bits(lhs_bits),
                f64::from_bits(rhs_bits)
            ),
        }
    }
}

struct FacilityKindViolationDisplay(FacilityKindViolation);

impl fmt::Display for FacilityKindViolationDisplay {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            FacilityKindViolation::InvalidToken(violation) => {
                write!(
                    formatter,
                    "token 非法：{}",
                    SourceTextViolationDisplay(violation)
                )
            }
            FacilityKindViolation::Unknown => formatter.write_str("未登记的物理设施类别"),
            FacilityKindViolation::CategoryMismatch { actual } => write!(
                formatter,
                "实际类别为 {}",
                match actual {
                    FacilityKindCategory::LaneBearing => "lane-bearing",
                    FacilityKindCategory::NonTraversable => "non-traversable",
                }
            ),
        }
    }
}

struct SourceTextViolationDisplay(SourceTextViolation);

impl fmt::Display for SourceTextViolationDisplay {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            SourceTextViolation::Empty => formatter.write_str("不得为空"),
            SourceTextViolation::TooLong { limit, observed } => {
                write!(formatter, "字节数超过上限，允许 {limit}，实际 {observed}")
            }
            SourceTextViolation::NonAscii { byte_index } => {
                write!(formatter, "字节位置 {byte_index} 不是 ASCII")
            }
            SourceTextViolation::InvalidFirstByte { byte } => {
                write!(formatter, "首字节 0x{byte:02x} 不是 ASCII 字母或数字")
            }
            SourceTextViolation::InvalidTokenByte { byte_index, byte } => write!(
                formatter,
                "字节位置 {byte_index} 包含非法 ASCII 令牌字节 0x{byte:02x}"
            ),
            SourceTextViolation::ControlByte { byte_index, byte } => {
                write!(formatter, "字节位置 {byte_index} 包含控制字节 0x{byte:02x}")
            }
            SourceTextViolation::ReservedDelimiter { byte_index } => write!(
                formatter,
                "字节位置 {byte_index} 包含为限定引用保留的 :: 分隔符"
            ),
        }
    }
}

/// 一次失败原子返回的规范有序诊断集合。
///
/// `diagnostics` 始终按规范顺序排列。当安全候选数超过配置档上限时只保留该顺序最小
/// 的前缀，并令 [`DiagnosticBundle::diagnostics_truncated`] 返回 `true`；这不表示编译
/// 可以继续，也不表示未保留候选未被检查。
#[derive(Clone, Debug)]
pub struct DiagnosticBundle {
    diagnostics: DiagnosticStorage,
    diagnostics_truncated: bool,
    failure_resource_observation: Option<FailureResourceObservation>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FailureResourceObservation {
    pub(crate) compiler_controlled_peak_bytes: u64,
    pub(crate) caller_retained_bytes: u64,
}

/// 只在已经确定失败后使用的 live-byte 物化门。
///
/// 成功路径不预留失败空间；rich report 无法在剩余 headroom 内构造时，返回内联、零堆
/// allocation 的 live-limit 诊断。`existing_context_bytes` 表示 rich bundle 将引用、但已
/// 包含在 `current_live_bytes` 中的共享 context/identity；其中仅本次 candidate 转移给调用方
/// 的部分由 `caller_transferred_context_bytes` 重新归入 caller-retained 观测。
#[derive(Clone, Copy, Debug)]
pub(crate) struct FailureLiveBudget {
    pub(crate) limit: u64,
    pub(crate) historical_peak_bytes: u64,
    pub(crate) current_live_bytes: u64,
    pub(crate) existing_context_bytes: u64,
    pub(crate) caller_transferred_context_bytes: u64,
}

impl FailureLiveBudget {
    #[cfg(test)]
    pub(crate) const fn unlimited() -> Self {
        Self {
            limit: u64::MAX,
            historical_peak_bytes: 0,
            current_live_bytes: 0,
            existing_context_bytes: 0,
            caller_transferred_context_bytes: 0,
        }
    }

    pub(crate) fn materialize(
        self,
        rich_increment_upper_bytes: u64,
        rich: impl FnOnce() -> DiagnosticBundle,
    ) -> DiagnosticBundle {
        let upper_observed = self
            .current_live_bytes
            .saturating_add(rich_increment_upper_bytes);
        let rich_peak_upper = self.historical_peak_bytes.max(upper_observed);
        if rich_peak_upper > self.limit {
            return DiagnosticBundle::allocation_free_live_limit(
                self.limit,
                rich_peak_upper,
                self.historical_peak_bytes.max(self.current_live_bytes),
            );
        }

        let bundle = rich();
        let retained = bundle.failure_retained_bytes();
        let actual_increment = retained.saturating_sub(self.existing_context_bytes);
        debug_assert!(
            actual_increment <= rich_increment_upper_bytes,
            "failure report exceeded its precharged rich materialization upper bound"
        );
        let compiler_peak = self
            .historical_peak_bytes
            .max(self.current_live_bytes.saturating_add(actual_increment));
        bundle.with_failure_resource_observation(FailureResourceObservation {
            compiler_controlled_peak_bytes: compiler_peak,
            caller_retained_bytes: actual_increment
                .saturating_add(self.caller_transferred_context_bytes),
        })
    }

    pub(crate) fn with_transient(
        self,
        retained_bytes: u64,
        peak_bytes: u64,
    ) -> Result<Self, DiagnosticBundle> {
        let peak_observed = self.current_live_bytes.saturating_add(peak_bytes);
        let operation_peak = self.historical_peak_bytes.max(peak_observed);
        if operation_peak > self.limit {
            return Err(DiagnosticBundle::allocation_free_live_limit(
                self.limit,
                operation_peak,
                self.historical_peak_bytes.max(self.current_live_bytes),
            ));
        }
        Ok(Self {
            historical_peak_bytes: self.historical_peak_bytes.max(peak_observed),
            current_live_bytes: self.current_live_bytes.saturating_add(retained_bytes),
            ..self
        })
    }
}

/// 单诊断失败是编译器最常见的原子拒绝路径。内联保存该项，使 live-byte 前门本身也能在
/// 已没有 heap headroom 时返回不带位置/字符串的最小资源诊断；多诊断路径继续使用受界
/// `Box<[Diagnostic]>`。
#[derive(Clone, Debug, Eq, PartialEq)]
enum DiagnosticStorage {
    Single(Diagnostic),
    Multiple(Box<[Diagnostic]>),
}

impl PartialEq for DiagnosticBundle {
    fn eq(&self, other: &Self) -> bool {
        self.diagnostics == other.diagnostics
            && self.diagnostics_truncated == other.diagnostics_truncated
    }
}

impl Eq for DiagnosticBundle {}

impl DiagnosticBundle {
    pub(crate) fn single(diagnostic: Diagnostic) -> Self {
        Self {
            diagnostics: DiagnosticStorage::Single(diagnostic),
            diagnostics_truncated: false,
            failure_resource_observation: None,
        }
    }

    pub(crate) fn with_failure_resource_observation(
        mut self,
        observation: FailureResourceObservation,
    ) -> Self {
        self.failure_resource_observation = Some(observation);
        self
    }

    #[cfg(feature = "road-editing-g3-evidence")]
    #[must_use]
    pub fn failure_compiler_controlled_peak_bytes(&self) -> Option<u64> {
        self.failure_resource_observation
            .map(|observation| observation.compiler_controlled_peak_bytes)
    }

    #[cfg(feature = "road-editing-g3-evidence")]
    #[must_use]
    pub fn caller_retained_bytes(&self) -> Option<u64> {
        self.failure_resource_observation
            .map(|observation| observation.caller_retained_bytes)
    }

    pub(crate) fn single_failure_owned_bytes(&self) -> u64 {
        self.diagnostics()
            .first()
            .map_or(0, Diagnostic::failure_owned_bytes)
    }

    pub(crate) fn failure_retained_bytes(&self) -> u64 {
        self.single_failure_owned_bytes()
            .saturating_add(failure_locations_retained_bytes(
                self.diagnostics().iter().flat_map(|diagnostic| {
                    diagnostic
                        .primary_location()
                        .into_iter()
                        .chain(diagnostic.related_locations())
                }),
            ))
    }

    pub(crate) fn allocation_free_live_limit(
        limit: u64,
        observed: u64,
        compiler_controlled_peak_bytes: u64,
    ) -> Self {
        Self::single(Diagnostic::compile_limit_exceeded(
            CompileLimitDimension::CompilerControlledLiveBytes,
            limit,
            observed,
        ))
        .with_failure_resource_observation(FailureResourceObservation {
            compiler_controlled_peak_bytes,
            caller_retained_bytes: 0,
        })
    }

    fn diagnostics_mut(&mut self) -> &mut [Diagnostic] {
        match &mut self.diagnostics {
            DiagnosticStorage::Single(diagnostic) => core::slice::from_mut(diagnostic),
            DiagnosticStorage::Multiple(diagnostics) => diagnostics,
        }
    }

    pub(crate) fn with_fallback_primary_location(mut self, location: SourceLocation) -> Self {
        for diagnostic in self.diagnostics_mut() {
            if diagnostic.primary_span.is_none() {
                diagnostic.primary_span = Some(location.clone());
            }
        }
        self
    }

    /// 返回按规范顺序保留的诊断切片。
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        match &self.diagnostics {
            DiagnosticStorage::Single(diagnostic) => core::slice::from_ref(diagnostic),
            DiagnosticStorage::Multiple(diagnostics) => diagnostics,
        }
    }

    /// 指示至少还有一个已发现诊断未被保留。
    #[must_use]
    pub const fn diagnostics_truncated(&self) -> bool {
        self.diagnostics_truncated
    }

    /// 判断保留的诊断中是否包含阻止阶段提交的错误。
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
    }
}

pub(crate) fn failure_locations_retained_bytes<'a>(
    locations: impl IntoIterator<Item = &'a SourceLocation>,
) -> u64 {
    let mut contexts: [Option<*const RoadEditingLocationContext>; 3] = [None, None, None];
    let mut context_count = 0_usize;
    let mut identities: [Option<*const u8>; 6] = [None; 6];
    let mut identity_count = 0_usize;
    let mut retained = 0_u64;
    for location in locations {
        if let SourceLocation::RoadEditing(road) = location {
            let pointer = core::ptr::from_ref(road.context());
            if !contexts[..context_count].contains(&Some(pointer)) {
                if context_count < contexts.len() {
                    contexts[context_count] = Some(pointer);
                    context_count += 1;
                }
                retained = retained.saturating_add(location.failure_context_bytes());
            }
        }
        for (pointer, bytes) in location
            .failure_identity_allocations()
            .into_iter()
            .flatten()
        {
            if identities[..identity_count].contains(&Some(pointer)) {
                continue;
            }
            if identity_count < identities.len() {
                identities[identity_count] = Some(pointer);
                identity_count += 1;
            }
            retained = retained.saturating_add(bytes);
        }
    }
    retained
}

pub(crate) fn failure_location_retained_bytes_excluding(
    candidate: &SourceLocation,
    excluded: Option<&SourceLocation>,
) -> u64 {
    let excluded_bytes = failure_locations_retained_bytes(excluded);
    failure_locations_retained_bytes(core::iter::once(candidate).chain(excluded))
        .saturating_sub(excluded_bytes)
}

impl fmt::Display for DiagnosticBundle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.diagnostics().first() {
            Some(first) if self.diagnostics().len() == 1 && !self.diagnostics_truncated => {
                first.fmt(formatter)
            }
            Some(first) => write!(
                formatter,
                "{}（共保留 {} 项诊断{}）",
                first,
                self.diagnostics().len(),
                if self.diagnostics_truncated {
                    "，其余已按规范顺序截断"
                } else {
                    ""
                }
            ),
            None => formatter.write_str("诊断集合为空"),
        }
    }
}

impl std::error::Error for DiagnosticBundle {}

/// 有界保留、但不提前终止候选检查的诊断收集器。
pub(crate) struct DiagnosticCollector {
    retained: Vec<Diagnostic>,
    limit: usize,
    diagnostics_truncated: bool,
}

impl DiagnosticCollector {
    pub(crate) fn new(limit: u64) -> Self {
        let limit = usize::try_from(limit).unwrap_or(0);
        Self {
            retained: Vec::with_capacity(limit),
            limit,
            diagnostics_truncated: false,
        }
    }

    pub(crate) fn push(&mut self, diagnostic: Diagnostic) {
        if self.retained.len() < self.limit {
            self.retained.push(diagnostic);
            return;
        }

        self.diagnostics_truncated = true;
        // 不能在容量满时简单丢弃后续项：候选发现顺序不是规范顺序。用新候选替换当前
        // 最大项，才能在扫描结束后得到全体候选的规范最小前缀。
        if let Some((max_index, current_max)) = self
            .retained
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| left.cmp(right))
            && diagnostic < *current_max
        {
            self.retained[max_index] = diagnostic;
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.retained.is_empty() && !self.diagnostics_truncated
    }

    pub(crate) fn finish(mut self) -> DiagnosticBundle {
        self.retained.sort_unstable();
        DiagnosticBundle {
            diagnostics: match self.retained.len() {
                1 => DiagnosticStorage::Single(
                    self.retained
                        .pop()
                        .expect("one retained diagnostic was observed"),
                ),
                _ => DiagnosticStorage::Multiple(self.retained.into_boxed_slice()),
            },
            diagnostics_truncated: self.diagnostics_truncated,
            failure_resource_observation: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RoadEditingDocumentIdentity, RoadEditingSourceLocation, RoadEditingSubject};

    #[test]
    fn bounded_collector_retains_global_canonical_prefix() {
        let mut collector = DiagnosticCollector::new(16);
        let dimensions = [
            CompileLimitDimension::RetainedCapacityBytes,
            CompileLimitDimension::CompilerControlledLiveBytes,
            CompileLimitDimension::OutputBytes,
            CompileLimitDimension::StageScratchBytes,
            CompileLimitDimension::DiagnosticCount,
            CompileLimitDimension::TotalStringBytes,
            CompileLimitDimension::SingleStringBytes,
            CompileLimitDimension::StringItemCount,
            CompileLimitDimension::SymbolCount,
            CompileLimitDimension::GeometryPointCount,
            CompileLimitDimension::WaitingZoneCount,
            CompileLimitDimension::ManeuverGateCount,
            CompileLimitDimension::RouteOccurrenceCount,
            CompileLimitDimension::IdentityFieldOccurrenceCount,
            CompileLimitDimension::RelationOccurrenceCount,
            CompileLimitDimension::ReferenceCount,
            CompileLimitDimension::LirRecordCount,
            CompileLimitDimension::MirRecordCount,
            CompileLimitDimension::HirRecordCount,
            CompileLimitDimension::TypedAstRecordCount,
        ];

        for dimension in dimensions {
            collector.push(Diagnostic::compile_limit_exceeded(dimension, 1, 2));
        }

        let bundle = collector.finish();
        assert!(bundle.diagnostics_truncated());
        assert_eq!(bundle.diagnostics().len(), 16);
        assert!(
            bundle
                .diagnostics()
                .windows(2)
                .all(|pair| pair[0] < pair[1])
        );
        assert!(bundle.diagnostics().iter().all(|diagnostic| {
            !matches!(
                diagnostic.payload(),
                DiagnosticPayload::CompileLimitExceeded {
                    dimension: CompileLimitDimension::RetainedCapacityBytes
                        | CompileLimitDimension::CompilerControlledLiveBytes
                        | CompileLimitDimension::OutputBytes
                        | CompileLimitDimension::StageScratchBytes,
                    ..
                }
            )
        }));
    }

    #[test]
    fn chinese_rendering_keeps_code_and_typed_values() {
        let diagnostic =
            Diagnostic::compile_limit_exceeded(CompileLimitDimension::ModuleCount, 522, 523);
        assert_eq!(diagnostic.code().as_str(), "LF-COMP-RESOURCE-LIMIT");
        assert_eq!(
            diagnostic.to_string(),
            "LF-COMP-RESOURCE-LIMIT: 编译资源维度 max_module_count 超过上限：允许 522，实际 523"
        );
    }

    #[test]
    fn source_span_value_uses_one_based_u32_positions() {
        let span = SourceSpan {
            source_document_key: Arc::from("generator.main"),
            start: SourcePosition { line: 7, column: 3 },
            end: SourcePosition {
                line: 7,
                column: 11,
            },
        };

        assert_eq!(span.source_document_key(), "generator.main");
        assert_eq!(span.start().line(), 7);
        assert_eq!(span.start().column(), 3);
        assert_eq!(span.end().line(), 7);
        assert_eq!(span.end().column(), 11);
    }

    #[test]
    fn unverified_wire_document_key_is_escaped_when_rendered() {
        let diagnostic = Diagnostic::invalid_road_editing_source(
            RoadEditingSourceViolation::SourceDocumentKeyMismatch,
            None,
            "roads/expected",
            Some("roads/actual\n\u{1b}[31m"),
        );

        let rendered = diagnostic.to_string();
        assert!(!rendered.contains('\n'));
        assert!(!rendered.contains('\u{1b}'));
        assert!(rendered.contains(r#"wire 文档键 "roads/actual\n\u{1b}[31m""#));
    }

    #[test]
    fn failure_observation_does_not_change_diagnostic_semantic_equality() {
        let plain = DiagnosticBundle::single(Diagnostic::compile_limit_exceeded(
            CompileLimitDimension::CompilerControlledLiveBytes,
            10,
            11,
        ));
        let observed =
            plain
                .clone()
                .with_failure_resource_observation(FailureResourceObservation {
                    compiler_controlled_peak_bytes: 11,
                    caller_retained_bytes: 0,
                });

        assert_eq!(plain, observed);
    }

    #[test]
    fn failure_live_gate_falls_back_without_bundle_or_location_backing() {
        let budget = FailureLiveBudget {
            limit: 10,
            historical_peak_bytes: 9,
            current_live_bytes: 10,
            existing_context_bytes: 0,
            caller_transferred_context_bytes: 0,
        };
        let bundle = budget.materialize(1, || {
            panic!("rich diagnostic must not materialize past the live limit")
        });

        assert!(matches!(
            bundle.diagnostics()[0].payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::CompilerControlledLiveBytes,
                limit: 10,
                observed: 11,
            }
        ));
        assert_eq!(bundle.failure_retained_bytes(), 0);
    }

    #[test]
    fn failure_location_transfer_deduplicates_shared_candidate_context_and_identity() {
        let context = Arc::new(RoadEditingLocationContext::new(
            Box::default(),
            Box::default(),
            Box::default(),
        ));
        let namespace: Arc<str> = Arc::from("city/shared");
        let document: Arc<str> = Arc::from("source/shared");
        let location = || {
            SourceLocation::RoadEditing(RoadEditingSourceLocation::new(
                Arc::clone(&context),
                RoadEditingDocumentIdentity::verified(
                    Arc::clone(&namespace),
                    Arc::clone(&document),
                ),
                RoadEditingSubject::ModuleHeader,
                None,
                None,
                None,
            ))
        };
        let existing = location();
        let candidate = location();
        let shared_location_bytes = failure_locations_retained_bytes([&existing, &candidate]);
        assert_eq!(
            shared_location_bytes,
            existing.failure_existing_bytes(),
            "the same context and identity allocations are retained only once"
        );
        assert_eq!(
            failure_location_retained_bytes_excluding(&candidate, Some(&existing)),
            0,
            "a candidate that shares every backing allocation transfers no new location bytes"
        );

        let rich = DiagnosticBundle::single(Diagnostic::duplicate_module_namespace(
            "city/shared",
            candidate,
            existing,
        ));
        let rich_owned_bytes = rich.single_failure_owned_bytes();
        let bundle = FailureLiveBudget {
            limit: u64::MAX,
            historical_peak_bytes: shared_location_bytes,
            current_live_bytes: shared_location_bytes,
            existing_context_bytes: shared_location_bytes,
            caller_transferred_context_bytes: 0,
        }
        .materialize(Diagnostic::failure_owned_bytes_upper_bound(), || rich);
        let observation = bundle
            .failure_resource_observation
            .expect("materialized failure must retain its resource observation");
        assert_eq!(observation.caller_retained_bytes, rich_owned_bytes);
        assert_eq!(
            bundle.failure_retained_bytes(),
            shared_location_bytes.saturating_add(rich_owned_bytes)
        );
    }
}

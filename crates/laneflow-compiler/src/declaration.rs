//! 合成领域专用语言当前支持的受检声明值。
//!
//! 公共输入仍保留调用方借用的文本；`SyntheticModuleBuilder` 校验标识、数值、导入与
//! 资源上限后，才把它们复制为本模块的拥有型 Typed AST 记录。这里的引用只描述
//! “目标模块命名空间 + 模块内稳定键”，真正的符号解析留给 HIR 阶段完成。

use std::marker::PhantomData;
use std::sync::Arc;

use laneflow_static_contract::{
    EntityKind, EntityKindMarker, FacilityBandKind, JunctionKind, LaneEdgeKind, LaneGroupKind,
    ManeuverGateKind, ManeuverPathKind, MovementKind, RoadSectionKind, StopLineKind,
};

use crate::SourceSpan;

/// 指向同一编译单元内某类来源声明的有类型未解析引用。
///
/// 类型参数 `K` 防止把不同实体种类的引用混用。构造引用不会查询目标；加入声明时仅
/// 校验拼写和显式导入边界，目标存在性在完整模块图建立后的 HIR 符号解析中验证。
#[derive(Debug, Eq, Hash, PartialEq)]
pub struct EntityReference<'a, K: EntityKindMarker> {
    module_namespace: Option<&'a str>,
    declaration_key: &'a str,
    marker: PhantomData<fn() -> K>,
}

impl<K: EntityKindMarker> Clone for EntityReference<'_, K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K: EntityKindMarker> Copy for EntityReference<'_, K> {}

impl<'a, K: EntityKindMarker> EntityReference<'a, K> {
    /// 建立指向当前来源模块声明的引用。
    ///
    /// 目标可以在当前声明之后加入；构造顺序不决定最终解析顺序。
    #[must_use]
    pub const fn local(declaration_key: &'a str) -> Self {
        Self {
            module_namespace: None,
            declaration_key,
            marker: PhantomData,
        }
    }

    /// 建立指向显式导入模块声明的引用。
    ///
    /// `module_namespace` 必须是当前模块自身或已通过
    /// `SyntheticModuleBuilder::add_import` 声明的命名空间，否则加入声明会失败。
    #[must_use]
    pub const fn imported(module_namespace: &'a str, declaration_key: &'a str) -> Self {
        Self {
            module_namespace: Some(module_namespace),
            declaration_key,
            marker: PhantomData,
        }
    }

    /// 返回显式目标模块；`None` 表示当前来源模块。
    #[must_use]
    pub const fn module_namespace(self) -> Option<&'a str> {
        self.module_namespace
    }

    /// 返回目标声明在其来源模块内的稳定键。
    #[must_use]
    pub const fn declaration_key(self) -> &'a str {
        self.declaration_key
    }
}

/// 指向车道图边声明的有类型未解析引用。
pub type LaneEdgeReference<'a> = EntityReference<'a, LaneEdgeKind>;
/// 指向道路区段声明的有类型未解析引用。
pub type RoadSectionReference<'a> = EntityReference<'a, RoadSectionKind>;
/// 指向车道组声明的有类型未解析引用。
pub type LaneGroupReference<'a> = EntityReference<'a, LaneGroupKind>;
/// 指向设施带声明的有类型未解析引用。
pub type FacilityBandReference<'a> = EntityReference<'a, FacilityBandKind>;
/// 指向路口声明的有类型未解析引用。
pub type JunctionReference<'a> = EntityReference<'a, JunctionKind>;
/// 指向通行流向声明的有类型未解析引用。
pub type MovementReference<'a> = EntityReference<'a, MovementKind>;
/// 指向机动路径声明的有类型未解析引用。
pub type ManeuverPathReference<'a> = EntityReference<'a, ManeuverPathKind>;
/// 指向停止线声明的有类型未解析引用。
pub type StopLineReference<'a> = EntityReference<'a, StopLineKind>;
/// 指向机动门声明的有类型未解析引用。
pub type ManeuverGateReference<'a> = EntityReference<'a, ManeuverGateKind>;
/// 横断面物理设施类别可承载的结构形态。
///
/// 该分类只约束 `FacilityKind` token 可以用于道路区段还是设施带，不授予任何交通
/// 参与者行为或准入能力。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum FacilityKindCategory {
    /// 可承载编制车道和车道图边覆盖的道路区段类别。
    LaneBearing,
    /// 不进入遍历图的设施带类别。
    NonTraversable,
}

/// 物理设施类别 token 不能用于声明时的结构化原因。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum FacilityKindViolation {
    /// token 本身违反来源文本约束。
    InvalidToken(crate::SourceTextViolation),
    /// token 既不是 SSOT seed，也没有合法的 `x-` 扩展前缀。
    Unknown,
    /// token 已知，但其结构类别不能由当前实体承载。
    CategoryMismatch {
        /// token 实际声明的结构类别。
        actual: FacilityKindCategory,
    },
}

/// 道路走廊有序横断面中的一种有类型成员引用。
///
/// 枚举值的切片顺序就是走廊参考方向从左到右的规范顺序，不能排序或去重后再解释。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum CorridorElementReference<'a> {
    /// 引用一个有方向、承载编制车道的道路区段。
    RoadSection(RoadSectionReference<'a>),
    /// 引用一个非方向、不可遍历的设施带。
    FacilityBand(FacilityBandReference<'a>),
}

impl<'a> CorridorElementReference<'a> {
    /// 建立道路区段成员引用。
    #[must_use]
    pub const fn road_section(reference: RoadSectionReference<'a>) -> Self {
        Self::RoadSection(reference)
    }

    /// 建立设施带成员引用。
    #[must_use]
    pub const fn facility_band(reference: FacilityBandReference<'a>) -> Self {
        Self::FacilityBand(reference)
    }
}

/// 合成领域专用语言的道路走廊声明输入。
///
/// `elements` 是有序异构成员集合；每个道路区段和设施带必须最终恰好被一个走廊拥有。
/// `reference_section` 必须指向 `elements` 中的道路区段成员，用于冻结横断面参考方向。
#[derive(Clone, Copy, Debug)]
pub struct RoadCorridorInput<'a> {
    /// 来源模块内显式持久化且唯一的走廊稳定键。
    pub road_corridor_key: &'a str,
    /// 声明横断面参考方向的成员道路区段。
    pub reference_section: RoadSectionReference<'a>,
    /// 按参考方向从左到右排列的非空道路区段/设施带成员序列。
    pub elements: &'a [CorridorElementReference<'a>],
}

/// 道路区段中一条编制车道的输入。
///
/// 编制车道是 Identity v1 的稳定实体，不得以本切片下标代替 `authoring_lane_key`。
/// `edge_chain` 顺序沿行驶方向，且相邻车道图边必须直接连通。
#[derive(Clone, Copy, Debug)]
pub struct AuthoringLaneInput<'a> {
    /// 来源模块内显式持久化且唯一的编制车道稳定键。
    pub authoring_lane_key: &'a str,
    /// 非空、有序且不得重复的车道图边覆盖链。
    pub edge_chain: &'a [LaneEdgeReference<'a>],
    /// 可选的车道组；目标必须与本车道属于同一道路区段。
    pub lane_group: Option<LaneGroupReference<'a>>,
}

/// 合成领域专用语言的道路区段声明输入。
#[derive(Clone, Copy, Debug)]
pub struct RoadSectionInput<'a> {
    /// 来源模块内显式持久化且唯一的道路区段稳定键。
    pub road_section_key: &'a str,
    /// 物理设施类别 token；必须属于 lane-bearing 类别。
    pub kind_id: &'a str,
    /// 按所属走廊参考方向从左到右排列的非空编制车道集合。
    pub lanes: &'a [AuthoringLaneInput<'a>],
}

/// 合成领域专用语言的车道组声明输入。
#[derive(Clone, Copy, Debug)]
pub struct LaneGroupInput<'a> {
    /// 来源模块内显式持久化且唯一的车道组稳定键。
    pub lane_group_key: &'a str,
    /// 唯一拥有该组的道路区段。
    pub road_section: RoadSectionReference<'a>,
}

/// 合成领域专用语言的设施带声明输入。
#[derive(Clone, Copy, Debug)]
pub struct FacilityBandInput<'a> {
    /// 来源模块内显式持久化且唯一的设施带稳定键。
    pub facility_band_key: &'a str,
    /// 物理设施类别 token；必须属于 non-traversable 类别。
    pub kind_id: &'a str,
}

/// 合成领域专用语言的路口声明输入。
///
/// 路口不持久化第二份 Movement 成员数组；成员关系由 `MovementInput::junction`
/// 反向形成，并在 HIR 中校验非空。
#[derive(Clone, Copy, Debug)]
pub struct JunctionInput<'a> {
    /// 来源模块内显式持久化且唯一的路口稳定键。
    pub junction_key: &'a str,
}

/// 合成领域专用语言的通行流向声明输入。
///
/// 两个有向引道键是 Identity v1 的权威 ASCII 字段，不从入口/出口边名称、几何或
/// 转向分类推断；调用方必须在编制来源中显式维护其稳定性。
#[derive(Clone, Copy, Debug)]
pub struct MovementInput<'a> {
    /// 来源模块内显式持久化且唯一的通行流向稳定键。
    pub movement_key: &'a str,
    /// 唯一拥有该通行流向的路口。
    pub junction: JunctionReference<'a>,
    /// 进入路口的有向引道稳定键。
    pub directed_entry_approach_key: &'a str,
    /// 离开路口的有向引道稳定键。
    pub directed_exit_approach_key: &'a str,
}

/// 合成领域专用语言的机动路径声明输入。
///
/// 权威遍历序列为 `entry_edge + internal_edges + exit_edge`。内部边可以为空；所有
/// 相邻边必须直接连通。内部边角色由拥有 Movement 的 Junction 排他声明，但同一
/// Junction 内的多条路径可以共享内部边。
#[derive(Clone, Copy, Debug)]
pub struct ManeuverPathInput<'a> {
    /// 来源模块内显式持久化且唯一的机动路径稳定键，对应 Identity v1 `pathKey`。
    pub maneuver_path_key: &'a str,
    /// 唯一拥有该路径的通行流向。
    pub movement: MovementReference<'a>,
    /// 进入路口前的边界边。
    pub entry_edge: LaneEdgeReference<'a>,
    /// 按遍历顺序排列的零到多条路口内部边。
    pub internal_edges: &'a [LaneEdgeReference<'a>],
    /// 离开路口后的第一条边界边。
    pub exit_edge: LaneEdgeReference<'a>,
}

/// 合成领域专用语言的停止线声明输入。
///
/// 停止线的位置固定为 `lane_edge` 的末端。它只有被同一转换起始边上的
/// `ManeuverGateInput` 引用后才形成有效控制边界；孤立停止线会在 HIR 闭包时被拒绝。
#[derive(Clone, Copy, Debug)]
pub struct StopLineInput<'a> {
    /// 来源模块内显式持久化且唯一的停止线稳定键，对应 Identity v1 `stopLineKey`。
    pub stop_line_key: &'a str,
    /// 停止线所在的车道图边；位置语义为该边末端。
    pub lane_edge: LaneEdgeReference<'a>,
}

/// 合成领域专用语言的机动门声明输入。
///
/// `transition_index` 指向 `maneuver_path` 边序列中从下标 `i` 到 `i + 1` 的转换，
/// 因而必须存在后继边。停止线必须位于该转换的起始边末端；同一路径同一转换最多声明
/// 一个机动门。
#[derive(Clone, Copy, Debug)]
pub struct ManeuverGateInput<'a> {
    /// 路径所有者局部唯一且稳定的机动门键，对应 Identity v1 `gateKey`。
    pub maneuver_gate_key: &'a str,
    /// 唯一拥有该机动门的机动路径。
    pub maneuver_path: ManeuverPathReference<'a>,
    /// 路径边序列中受控转换的起始边下标。
    pub transition_index: u32,
    /// 标记该转换起始边末端的停止线。
    pub stop_line: StopLineReference<'a>,
}

/// 合成领域专用语言的等待区声明输入。
///
/// 等待区由同一路径上的入口门和释放门界定。入口转换必须严格早于释放转换；同一路径
/// 的等待区内部不得重叠或嵌套，但相邻等待区可以共享边界门。
#[derive(Clone, Copy, Debug)]
pub struct WaitingZoneInput<'a> {
    /// 路径所有者局部唯一且稳定的等待区键，对应 Identity v1 `waitingZoneKey`。
    pub waiting_zone_key: &'a str,
    /// 唯一拥有该等待区的机动路径。
    pub maneuver_path: ManeuverPathReference<'a>,
    /// 进入等待区的机动门。
    pub entry_gate: ManeuverGateReference<'a>,
    /// 释放等待区占用者的机动门。
    pub release_gate: ManeuverGateReference<'a>,
    /// 等待区可同时容纳的最大交通参与单元数；必须大于零。
    pub max_occupancy: u32,
}

/// 合成领域专用语言的静态路线声明输入。
///
/// `edge_sequence` 是编制期权威有序序列；相同 `LaneEdge` 可以在路线中多次出现，
/// 每次出现由路线内下标区分。HIR 会据此预编译机动路径、机动门与等待区出现项，
/// 运行时不再扫描全局控制表重新匹配。
#[derive(Clone, Copy, Debug)]
pub struct StaticRouteInput<'a> {
    /// 来源模块内显式持久化且唯一的路线稳定键，对应 Identity v1 `routeKey`。
    pub static_route_key: &'a str,
    /// 非空、有序的车道图边序列；相邻边必须直接连通。
    pub edge_sequence: &'a [LaneEdgeReference<'a>],
}

/// 合成领域专用语言的车道图边声明输入。
///
/// 车道图边身份由 `lane_edge_key` 与所属 authoring namespace 决定，不依赖
/// RoadSection/Junction 等可选角色。终止边、孤立边和自环在本声明层均为合法拓扑；
/// `successors` 的传入顺序不具有语义，构建器会按目标命名空间和稳定键规范化。
#[derive(Clone, Copy, Debug)]
pub struct LaneEdgeInput<'a> {
    /// 来源模块内显式持久化且唯一的稳定边键；不得以声明序号替代。
    pub lane_edge_key: &'a str,
    /// 交通权威长度，单位为米。
    pub length_meters: f64,
    /// 基础道路限速，单位为米每秒。
    pub speed_limit_meters_per_second: f64,
    /// 无序、不得重复的显式下游连接集合；空集合表示终止边。
    pub successors: &'a [LaneEdgeReference<'a>],
}

#[derive(Clone)]
/// 通过前端校验后由 Typed AST 拥有的有类型引用及其诊断位置。
pub(crate) struct OwnedEntityReference<K: EntityKindMarker> {
    /// 已展开为本模块或显式导入模块的规范命名空间。
    pub(crate) module_namespace: Arc<str>,
    /// 目标模块内的稳定声明键；此阶段尚未解析为 HIR 键。
    pub(crate) declaration_key: Arc<str>,
    /// 引用出现的位置，用于解析失败时定位调用方来源。
    pub(crate) span: SourceSpan,
    marker: PhantomData<fn() -> K>,
}

impl<K: EntityKindMarker> OwnedEntityReference<K> {
    pub(crate) fn new(
        module_namespace: Arc<str>,
        declaration_key: Arc<str>,
        span: SourceSpan,
    ) -> Self {
        Self {
            module_namespace,
            declaration_key,
            span,
            marker: PhantomData,
        }
    }
}

#[derive(Clone, Copy)]
/// 已验证的交通权威车道图边长度，内部继续保留 `f64` 精度。
pub(crate) struct EdgeLength(f64);

impl EdgeLength {
    /// 当前契约的排他最小长度，单位为米。
    pub(crate) const EXCLUSIVE_MINIMUM_METERS: f64 = 1.0e-9;

    pub(crate) fn try_new(value: f64) -> Result<Self, ScalarViolation> {
        validate_greater_than(value, Self::EXCLUSIVE_MINIMUM_METERS).map(Self)
    }

    pub(crate) const fn value(self) -> f64 {
        self.0
    }
}

#[derive(Clone, Copy)]
/// 已验证的基础道路限速，单位为米每秒并保留 `f64` 精度。
pub(crate) struct SpeedLimit(f64);

impl SpeedLimit {
    pub(crate) fn try_new(value: f64) -> Result<Self, ScalarViolation> {
        validate_greater_than(value, 0.0).map(Self)
    }

    pub(crate) const fn value(self) -> f64 {
        self.0
    }
}

fn validate_greater_than(value: f64, exclusive_minimum: f64) -> Result<f64, ScalarViolation> {
    if !value.is_finite() {
        return Err(ScalarViolation::NotFinite);
    }
    if value <= exclusive_minimum {
        return Err(ScalarViolation::NotGreaterThan {
            exclusive_minimum_bits: exclusive_minimum.to_bits(),
        });
    }
    Ok(value)
}

/// 受检领域数值不能建立时的结构化原因。
///
/// 错误保留 IEEE 754 位模式而不是格式化字符串，使诊断排序和后续渲染不受浮点文本
/// 格式变化影响。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum ScalarViolation {
    /// 输入为 NaN 或正负无穷。
    NotFinite,
    /// 输入没有严格大于给定的排他下限。
    NotGreaterThan {
        /// 排他下限的 IEEE 754 位模式。
        exclusive_minimum_bits: u64,
    },
}

/// 所有受检 Typed AST 声明共享的身份与诊断上下文。
pub(crate) struct DeclarationHeader {
    /// 声明实体种类；与外层 `SyntheticDeclaration` 变体保持一致。
    pub(crate) entity_kind: EntityKind,
    /// 所属来源模块内唯一且显式持久化的稳定键。
    pub(crate) stable_key: Arc<str>,
    /// 声明出现的位置，不参与实体身份。
    pub(crate) span: SourceSpan,
}

/// 已通过字段级与模块内约束检查的车道图边 Typed AST 记录。
pub(crate) struct LaneEdgeDeclaration {
    pub(crate) header: DeclarationHeader,
    pub(crate) length: EdgeLength,
    pub(crate) speed_limit: SpeedLimit,
    pub(crate) successors: Box<[OwnedEntityReference<LaneEdgeKind>]>,
}

/// 通过前端校验后的走廊横断面成员引用。
pub(crate) enum OwnedCorridorElementReference {
    RoadSection(OwnedEntityReference<RoadSectionKind>),
    FacilityBand(OwnedEntityReference<FacilityBandKind>),
}

/// 已通过字段级检查、等待全编译单元解析成员的道路走廊 Typed AST 记录。
pub(crate) struct RoadCorridorDeclaration {
    pub(crate) header: DeclarationHeader,
    pub(crate) reference_section: OwnedEntityReference<RoadSectionKind>,
    pub(crate) elements: Box<[OwnedCorridorElementReference]>,
}

/// 已通过字段级检查的编制车道 Typed AST 记录。
pub(crate) struct AuthoringLaneDeclaration {
    pub(crate) header: DeclarationHeader,
    pub(crate) edge_chain: Box<[OwnedEntityReference<LaneEdgeKind>]>,
    pub(crate) lane_group: Option<OwnedEntityReference<LaneGroupKind>>,
}

/// 已通过字段级检查、拥有有序编制车道的道路区段 Typed AST 记录。
pub(crate) struct RoadSectionDeclaration {
    pub(crate) header: DeclarationHeader,
    pub(crate) kind_id: Arc<str>,
    pub(crate) lanes: Box<[AuthoringLaneDeclaration]>,
}

/// 已通过字段级检查、等待解析唯一道路区段父项的车道组 Typed AST 记录。
pub(crate) struct LaneGroupDeclaration {
    pub(crate) header: DeclarationHeader,
    pub(crate) road_section: OwnedEntityReference<RoadSectionKind>,
}

/// 已通过字段级检查、等待走廊唯一所有者闭包的设施带 Typed AST 记录。
pub(crate) struct FacilityBandDeclaration {
    pub(crate) header: DeclarationHeader,
    pub(crate) kind_id: Arc<str>,
}

/// 已通过字段级检查、等待反向形成非空 Movement 成员集的路口 Typed AST 记录。
pub(crate) struct JunctionDeclaration {
    pub(crate) header: DeclarationHeader,
}

/// 已通过字段级检查、等待解析唯一 Junction 父项的通行流向 Typed AST 记录。
pub(crate) struct MovementDeclaration {
    pub(crate) header: DeclarationHeader,
    pub(crate) junction: OwnedEntityReference<JunctionKind>,
    pub(crate) directed_entry_approach_key: Arc<str>,
    pub(crate) directed_exit_approach_key: Arc<str>,
}

/// 已通过字段级检查、等待解析父项和完整边序列的机动路径 Typed AST 记录。
pub(crate) struct ManeuverPathDeclaration {
    pub(crate) header: DeclarationHeader,
    pub(crate) movement: OwnedEntityReference<MovementKind>,
    pub(crate) entry_edge: OwnedEntityReference<LaneEdgeKind>,
    pub(crate) internal_edges: Box<[OwnedEntityReference<LaneEdgeKind>]>,
    pub(crate) exit_edge: OwnedEntityReference<LaneEdgeKind>,
}

/// 已通过字段级检查、等待停止线使用闭包校验的停止线 Typed AST 记录。
pub(crate) struct StopLineDeclaration {
    pub(crate) header: DeclarationHeader,
    pub(crate) lane_edge: OwnedEntityReference<LaneEdgeKind>,
}

/// 已通过字段级检查、等待路径转换与停止线位置闭包校验的机动门 Typed AST 记录。
pub(crate) struct ManeuverGateDeclaration {
    pub(crate) header: DeclarationHeader,
    pub(crate) maneuver_path: OwnedEntityReference<ManeuverPathKind>,
    pub(crate) transition_index: u32,
    pub(crate) stop_line: OwnedEntityReference<StopLineKind>,
}

/// 已通过字段级检查、等待门顺序和区间重叠闭包校验的等待区 Typed AST 记录。
pub(crate) struct WaitingZoneDeclaration {
    pub(crate) header: DeclarationHeader,
    pub(crate) maneuver_path: OwnedEntityReference<ManeuverPathKind>,
    pub(crate) entry_gate: OwnedEntityReference<ManeuverGateKind>,
    pub(crate) release_gate: OwnedEntityReference<ManeuverGateKind>,
    pub(crate) max_occupancy: u32,
}

/// 已通过字段级检查、等待解析边序列并预编译控制出现项的静态路线 Typed AST 记录。
pub(crate) struct StaticRouteDeclaration {
    pub(crate) header: DeclarationHeader,
    pub(crate) edge_sequence: Box<[OwnedEntityReference<LaneEdgeKind>]>,
}

/// 官方合成前端当前支持的封闭声明集合。
pub(crate) enum SyntheticDeclaration {
    LaneEdge(LaneEdgeDeclaration),
    RoadCorridor(RoadCorridorDeclaration),
    RoadSection(RoadSectionDeclaration),
    LaneGroup(LaneGroupDeclaration),
    FacilityBand(FacilityBandDeclaration),
    Junction(JunctionDeclaration),
    Movement(MovementDeclaration),
    ManeuverPath(ManeuverPathDeclaration),
    StopLine(StopLineDeclaration),
    ManeuverGate(ManeuverGateDeclaration),
    WaitingZone(WaitingZoneDeclaration),
    StaticRoute(StaticRouteDeclaration),
}

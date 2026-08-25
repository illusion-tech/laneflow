//! 合成领域专用语言当前支持的受检声明值。
//!
//! 公共输入仍保留调用方借用的文本；`SyntheticModuleBuilder` 校验标识、数值、导入与
//! 资源上限后，才把它们复制为本模块的拥有型 Typed AST 记录。这里的引用只描述
//! “目标模块命名空间 + 有类型来源地址”，真正的符号解析留给 HIR 阶段完成。

use std::marker::PhantomData;
use std::sync::Arc;

use laneflow_static_contract::{
    AccessEffect, AuthoringLaneKind, CanonicalFrameKind, EntityKind, EntityKindMarker,
    FacilityBandKind, JunctionKind, LaneEdgeKind, LaneGroupKind, MAX_LANE_EDGE_LENGTH_MM,
    MAX_SPEED_MM_S, MIN_LANE_EDGE_LENGTH_MM, MIN_SPEED_MM_S, ManeuverGateKind, ManeuverPathKind,
    MovementKind, ParkingAreaKind, ParticipantClassKind, RoadSectionKind, SignalAspect,
    SignalGroupKind, StopLineKind, VehicleProfileKind, millimetres_from_si,
};

use crate::SourceLocation;

/// JavaScript/JSON 等常见编制前端可以无损表达的最大整数毫秒值。
pub(crate) const MAX_PORTABLE_SIGNAL_TIME_MS: u64 = 9_007_199_254_740_991;

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
/// 指向信号组声明的有类型未解析引用。
pub type SignalGroupReference<'a> = EntityReference<'a, SignalGroupKind>;
/// 指向停车区域声明的有类型未解析引用。
pub type ParkingAreaReference<'a> = EntityReference<'a, ParkingAreaKind>;
/// 指向参与者类别声明的有类型未解析引用。
pub type ParticipantClassReference<'a> = EntityReference<'a, ParticipantClassKind>;
/// 指向车辆配置声明的有类型未解析引用。
pub type VehicleProfileReference<'a> = EntityReference<'a, VehicleProfileKind>;
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

pub(crate) fn facility_kind_category(kind_id: &str) -> Option<FacilityKindCategory> {
    let seed_category = match kind_id {
        "motorLane" | "nonMotorLane" => Some(FacilityKindCategory::LaneBearing),
        "sidewalk" | "median" | "plantingStrip" | "facilityStrip" | "shoulder" => {
            Some(FacilityKindCategory::NonTraversable)
        }
        _ => None,
    };
    if seed_category.is_some() {
        return seed_category;
    }
    // `x-lane-` 是 `x-` 的特化前缀，必须先失败关闭；空 lane 后缀不能回退成普通 band。
    if let Some(suffix) = kind_id.strip_prefix("x-lane-") {
        return (!suffix.is_empty()).then_some(FacilityKindCategory::LaneBearing);
    }
    kind_id
        .strip_prefix("x-")
        .filter(|suffix| !suffix.is_empty())
        .map(|_| FacilityKindCategory::NonTraversable)
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
    /// 信号层控制绑定；`None` 只表示信号层不施加约束。
    pub signal_control: SignalControlInput<'a>,
}

/// 机动门的编制期信号层控制绑定。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum SignalControlInput<'a> {
    /// 由指定信号组控制。
    Group(SignalGroupReference<'a>),
    /// 不受固定时制（fixed-time）信号层控制。
    None,
}

/// 合成领域专用语言的信号组声明输入。
#[derive(Clone, Copy, Debug)]
pub struct SignalGroupInput<'a> {
    /// 来源模块内显式持久化且唯一的信号组稳定键。
    pub signal_group_key: &'a str,
}

/// 一个固定时制相位中某信号组的完整状态记录。
#[derive(Clone, Copy, Debug)]
pub struct SignalGroupStateInput<'a> {
    /// 必须属于同一控制器的信号组。
    pub signal_group: SignalGroupReference<'a>,
    /// 本相位内该组的灯色指示。
    pub aspect: SignalAspect,
}

/// 信号控制器程序中的所有者局部（owner-local）有序相位输入。
#[derive(Clone, Copy, Debug)]
pub struct SignalPhaseInput<'a> {
    /// 在所属控制器内唯一且稳定的相位键。
    pub signal_phase_key: &'a str,
    /// 相位持续时间，单位为毫秒；必须位于可移植安全整数正区间。
    pub duration_ms: u64,
    /// 对控制器全部信号组恰好各出现一次的状态集合。
    pub states: &'a [SignalGroupStateInput<'a>],
}

/// 合成领域专用语言的固定时制信号控制器输入。
#[derive(Clone, Copy, Debug)]
pub struct SignalControllerInput<'a> {
    /// 来源模块内显式持久化且唯一的控制器稳定键。
    pub signal_controller_key: &'a str,
    /// 相对世界时间零点的规范相位偏移，单位为毫秒。
    pub offset_ms: u64,
    /// 非空、无重复且由本控制器唯一拥有的信号组集合。
    pub signal_groups: &'a [SignalGroupReference<'a>],
    /// 非空且顺序定义循环程序的相位序列。
    pub phases: &'a [SignalPhaseInput<'a>],
}

/// 停车位连接车道图边的入口或出口锚点输入。
#[derive(Clone, Copy, Debug)]
pub struct ParkingLaneAnchorInput<'a> {
    /// 锚点所在的车道图边。
    pub lane_edge: LaneEdgeReference<'a>,
    /// 从边起点量取的纵向进度，单位为米。
    pub progress_meters: f64,
}

/// 停车位相对入口边正向切线的矩形几何输入。
#[derive(Clone, Copy, Debug)]
pub struct ParkingSpaceGeometryInput {
    /// 相对入口边中心线的横向偏移，单位为米；正值位于行驶方向左侧。
    pub lateral_offset_meters: f64,
    /// 相对入口边正向切线的逆时针朝向偏移，单位为弧度。
    pub heading_offset_radians: f64,
    /// 沿停车朝向的泊位长度，单位为米。
    pub length_meters: f64,
    /// 垂直停车朝向的泊位宽度，单位为米。
    pub width_meters: f64,
}

/// 合成领域专用语言的停车区域声明输入。
#[derive(Clone, Copy, Debug)]
pub struct ParkingAreaInput<'a> {
    /// 来源模块内显式持久化且唯一的停车区域稳定键。
    pub parking_area_key: &'a str,
}

/// 合成领域专用语言的停车位声明输入。
///
/// `parking_area` 只建立可选组织关系，不参与停车位 Identity v1；改变区域归属不能
/// 造成停车位身份漂移。入口和出口锚点均必须解析到既有车道图边。
#[derive(Clone, Copy, Debug)]
pub struct ParkingSpaceInput<'a> {
    /// 来源模块内显式持久化且唯一的停车位稳定键。
    pub parking_space_key: &'a str,
    /// 可选停车区域；`None` 表示合法的独立停车位。
    pub parking_area: Option<ParkingAreaReference<'a>>,
    /// 停车提交前交通参与单元必须到达的入口锚点。
    pub entry: ParkingLaneAnchorInput<'a>,
    /// 离开停车位后重新进入车道图的出口锚点。
    pub exit: ParkingLaneAnchorInput<'a>,
    /// 停车表现使用的不可变矩形几何。
    pub geometry: ParkingSpaceGeometryInput,
}

/// 合成领域专用语言的参与者类别声明输入。
///
/// 类别只建立准入分类法，不声明交通执行域、运动模型或生命周期能力。
#[derive(Clone, Copy, Debug)]
pub struct ParticipantClassInput<'a> {
    /// 来源模块内显式持久化且唯一的类别稳定键。
    pub participant_class_key: &'a str,
    /// 可选单继承父类；完整层级必须无环。
    pub extends: Option<ParticipantClassReference<'a>>,
}

/// 当前道路机动车执行域采用的 IIDM 静态参数。
///
/// 这些字段逐项沿用 current Core `IidmProfileSpec` 的 `f64` 数值语义；该类型不是
/// 其他交通执行域的通用运行参数基类。
#[derive(Clone, Copy, Debug)]
pub struct IidmVehicleProfileInput {
    /// 车辆长度，单位为米；量化后必须落在 `100..=128_000` mm。
    pub length_meters: f64,
    /// 自由流期望速度，单位为米每秒；量化后必须落在 `1..=100_000` mm/s。
    pub desired_speed_meters_per_second: f64,
    /// 行为最小间距，单位为米；量化后必须落在 `0..=128_000` mm。
    pub min_gap_meters: f64,
    /// 期望时间间隔，单位为秒；量化到 `f32` 后必须满足 `(0, 60]`。
    pub time_headway_seconds: f64,
    /// 最大舒适加速度，单位为米每二次方秒；量化到 `f32` 后必须落在 `0.5..=50`。
    pub max_acceleration_meters_per_second_squared: f64,
    /// 舒适减速度幅值，单位为米每二次方秒；量化到 `f32` 后必须落在 `0.5..=50`。
    pub comfortable_deceleration_meters_per_second_squared: f64,
    /// 紧急减速度幅值，单位为米每二次方秒；量化到 `f32` 后必须落在 `0.5..=50` 且不小于舒适减速度。
    pub emergency_deceleration_meters_per_second_squared: f64,
}

/// 合成领域专用语言的当前道路机动车车辆配置声明输入。
///
/// `participant_class` 只决定静态准入分类，不改变 IIDM 模型或交通执行域。
#[derive(Clone, Copy, Debug)]
pub struct VehicleProfileInput<'a> {
    /// 来源模块内显式持久化且唯一的车辆配置稳定键。
    pub vehicle_profile_key: &'a str,
    /// 恰好一个参与者类别；目标必须存在于同一编译单元。
    pub participant_class: ParticipantClassReference<'a>,
    /// 当前 Core 已接受的 IIDM 静态参数。
    pub iidm: IidmVehicleProfileInput,
}

/// 已量化到规范 `f32` 空间的一点，单位为米。
///
/// 构建器会拒绝非有限值和超出 canonical frame 范围的分量，并把带符号零规范化；
/// 成功后 LIR 只保存 `+0.0`。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalPoint3F32Input {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// 一条车道图边在某个规范坐标框架中的显式中心线。
#[derive(Clone, Copy, Debug)]
pub struct LaneEdgeGeometryInput<'a> {
    /// 被完整覆盖的既有车道图边。
    pub lane_edge: LaneEdgeReference<'a>,
    /// 按行驶方向排列的量化后规范点；至少包含两点。
    pub centerline_points: &'a [CanonicalPoint3F32Input],
}

/// 合成领域专用语言的规范坐标框架声明输入。
///
/// `canonical_frame_key` 对应 SpatialPackage v0.1 的 `frameId`。坐标单位、手性、
/// 上方向和有界范围由全局 canonical frame 契约固定，不是每条声明可变的属性。
#[derive(Clone, Copy, Debug)]
pub struct CanonicalFrameInput<'a> {
    /// 来源模块内显式持久化且唯一的规范坐标框架稳定键。
    pub canonical_frame_key: &'a str,
    /// 由该 frame 拥有的车道图边中心线集合；集合顺序不参与语义。
    pub lane_edge_geometries: &'a [LaneEdgeGeometryInput<'a>],
}

/// 静态准入规则可以引用的目标。
///
/// 四个可遍历目标在本切片编译为静态准入表；`FacilityBand` 保留为可诊断输入，HIR
/// 会在确认引用存在后以 capability-unavailable 拒绝，不能静默忽略。
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum AccessRuleTargetInput<'a> {
    /// 单条车道图边。
    LaneEdge(LaneEdgeReference<'a>),
    /// 通过成员车道覆盖到车道图边的车道组。
    LaneGroup(LaneGroupReference<'a>),
    /// 通过编制车道覆盖到车道图边的道路区段。
    RoadSection(RoadSectionReference<'a>),
    /// 不展平为边的机动路径。
    ManeuverPath(ManeuverPathReference<'a>),
    /// 首版不具备运行时行为的设施带目标。
    FacilityBand(FacilityBandReference<'a>),
}

/// 准入规则携带的法规来源信息；该信息用于审计，不参与规则优先级计算。
#[derive(Clone, Copy, Debug)]
pub struct AccessRegulationInput<'a> {
    /// 法域；字符数必须位于 1 到 128。
    pub jurisdiction: &'a str,
    /// 法规版本；字符数必须位于 1 到 128。
    pub version: &'a str,
    /// 可选来源说明；存在时字符数必须位于 1 到 128。
    pub source: Option<&'a str>,
}

/// 合成领域专用语言的静态准入规则声明输入。
///
/// `participant_classes` 按集合解释：输入顺序不影响规范结果，重复引用会被规范化去重。
/// 本切片只接受永远适用的静态规则；时变窗口由后继运行时 G1 处理。
#[derive(Clone, Copy, Debug)]
pub struct AccessRuleInput<'a> {
    /// 来源模块内显式持久化且唯一的规则稳定键。
    pub access_rule_key: &'a str,
    /// 恰好一个准入目标。
    pub target: AccessRuleTargetInput<'a>,
    /// 平面内准入效果。
    pub effect: AccessEffect,
    /// 非空参与者类别集合；类别的传递后代也匹配本规则。
    pub participant_classes: &'a [ParticipantClassReference<'a>],
    /// 可选法规来源；同一编译单元内所有已声明来源必须共享法域和版本。
    pub regulation: Option<AccessRegulationInput<'a>>,
    /// 在参与者和目标 specificity 相同后使用的显式优先级。
    pub priority: i32,
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
    /// 目标模块内的完整来源地址；此阶段尚未解析为 HIR 键。
    pub(crate) target_address: TypedAstEntityAddress,
    /// 引用出现的位置，用于解析失败时定位调用方来源。
    pub(crate) span: SourceLocation,
    marker: PhantomData<fn() -> K>,
}

/// Typed AST 中与产品身份分层的来源实体地址。
///
/// module-scoped 声明的 owner tuple 为空；owner-scoped 声明按父先子后顺序保存原始
/// sibling-local key。地址用于符号查找，`local_key` 仍只是 Identity v1 前像中的一个
/// 字段，不能单独充当模块内全局键。
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct TypedAstEntityAddress {
    // 模块级地址使用 `None`，避免为最常见的空 owner tuple 在每个声明和引用上分别
    // 分配一份 `Arc` header。
    owner_local_keys: Option<Arc<[Arc<str>]>>,
    local_key: Arc<str>,
}

impl TypedAstEntityAddress {
    pub(crate) fn module_scoped(local_key: Arc<str>) -> Self {
        Self {
            owner_local_keys: None,
            local_key,
        }
    }

    pub(crate) fn owner_scoped(owner_local_keys: Arc<[Arc<str>]>, local_key: Arc<str>) -> Self {
        debug_assert!(!owner_local_keys.is_empty());
        Self {
            owner_local_keys: Some(owner_local_keys),
            local_key,
        }
    }

    pub(crate) fn owner_local_keys(&self) -> &[Arc<str>] {
        self.owner_local_keys.as_deref().unwrap_or(&[])
    }

    pub(crate) fn local_key(&self) -> &Arc<str> {
        &self.local_key
    }
}

impl<K: EntityKindMarker> OwnedEntityReference<K> {
    pub(crate) fn new(
        module_namespace: Arc<str>,
        declaration_key: Arc<str>,
        span: impl Into<SourceLocation>,
    ) -> Self {
        Self {
            module_namespace,
            target_address: TypedAstEntityAddress::module_scoped(declaration_key),
            span: span.into(),
            marker: PhantomData,
        }
    }

    pub(crate) fn with_target_address(
        module_namespace: Arc<str>,
        target_address: TypedAstEntityAddress,
        span: impl Into<SourceLocation>,
    ) -> Self {
        Self {
            module_namespace,
            target_address,
            span: span.into(),
            marker: PhantomData,
        }
    }

    pub(crate) fn declaration_key(&self) -> &Arc<str> {
        self.target_address.local_key()
    }
}

#[derive(Clone, Copy)]
/// 已验证的交通权威车道图边长度；准入按毫米闭包，编制值仍保留 `f64`。
pub(crate) struct EdgeLength(f64);

impl EdgeLength {
    pub(crate) fn try_new(value: f64) -> Result<Self, ScalarViolation> {
        closed_millimetres(value, MIN_LANE_EDGE_LENGTH_MM, MAX_LANE_EDGE_LENGTH_MM)
            .map(|_| Self(value))
    }

    /// 编制曲线冻结长度：最短仍是 `100` mm，但不套交通边 `10 km` 上界。
    pub(crate) fn try_from_authoring_metres(value: f64) -> Result<Self, ScalarViolation> {
        closed_millimetres(value, MIN_LANE_EDGE_LENGTH_MM, u32::MAX).map(|_| Self(value))
    }

    pub(crate) const fn value(self) -> f64 {
        self.0
    }
}

#[derive(Clone, Copy)]
/// 已验证的基础道路限速；准入按毫米每秒闭包，编制值仍保留 `f64`。
pub(crate) struct SpeedLimit(f64);

impl SpeedLimit {
    pub(crate) fn try_new(value: f64) -> Result<Self, ScalarViolation> {
        closed_millimetres(value, MIN_SPEED_MM_S, MAX_SPEED_MM_S).map(|_| Self(value))
    }

    pub(crate) const fn value(self) -> f64 {
        self.0
    }
}

pub(crate) fn closed_millimetres(
    value: f64,
    min_mm: u32,
    max_mm: u32,
) -> Result<u32, ScalarViolation> {
    let Some(actual_mm) = millimetres_from_si(value) else {
        return Err(if value.is_finite() {
            ScalarViolation::QuantizeFailed
        } else {
            ScalarViolation::NotFinite
        });
    };
    if actual_mm < min_mm || actual_mm > max_mm {
        return Err(ScalarViolation::OutsideClosedMillimetreRange {
            min_mm,
            max_mm,
            actual_mm,
        });
    }
    Ok(actual_mm)
}

/// 受检领域数值不能建立时的结构化原因。
///
/// 毫米闭包保留量化后的整数；其余错误保留 IEEE 754 位模式，使诊断排序和后续渲染
/// 不受浮点文本格式变化影响。
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
    /// 输入小于给定的包含下限。
    NotLessThan {
        /// 包含下限的 IEEE 754 位模式。
        inclusive_minimum_bits: u64,
    },
    /// 输入大于给定的包含上限。
    NotAtMost {
        /// 包含上限的 IEEE 754 位模式。
        inclusive_maximum_bits: u64,
    },
    /// 量化后的毫米值落在闭包之外。
    OutsideClosedMillimetreRange {
        /// 包含下限，单位为毫米。
        min_mm: u32,
        /// 包含上限，单位为毫米。
        max_mm: u32,
        /// 量化后的实际值，单位为毫米。
        actual_mm: u32,
    },
    /// 量化后的 `f32` 值落在闭包之外。
    OutsideClosedF32Range {
        /// 包含下限的 binary32 位型。
        min_bits: u32,
        /// 包含上限的 binary32 位型。
        max_bits: u32,
    },
    /// 有限输入无法量化到目标整数单位。
    QuantizeFailed,
}

/// 所有受检 Typed AST 声明共享的身份与诊断上下文。
pub(crate) struct DeclarationHeader {
    /// 声明实体种类；与外层 `TypedAstDeclaration` 变体保持一致。
    pub(crate) entity_kind: EntityKind,
    /// 只用于来源符号查找的完整 owner-qualified 地址。
    pub(crate) source_address: TypedAstEntityAddress,
    /// Identity v1 前像中独立的 sibling-local key。
    pub(crate) stable_key: Arc<str>,
    /// 声明出现的位置，不参与实体身份。
    pub(crate) span: SourceLocation,
}

impl DeclarationHeader {
    pub(crate) fn module_scoped(
        entity_kind: EntityKind,
        stable_key: Arc<str>,
        span: SourceLocation,
    ) -> Self {
        Self {
            entity_kind,
            source_address: TypedAstEntityAddress::module_scoped(Arc::clone(&stable_key)),
            stable_key,
            span,
        }
    }

    pub(crate) fn with_source_address(
        entity_kind: EntityKind,
        source_address: TypedAstEntityAddress,
        identity_local_key: Arc<str>,
        span: SourceLocation,
    ) -> Self {
        debug_assert_eq!(
            source_address.local_key().as_ref(),
            identity_local_key.as_ref()
        );
        Self {
            entity_kind,
            source_address,
            stable_key: identity_local_key,
            span,
        }
    }
}

/// 共同 Typed AST 中由编制曲线保留的有限 `f64` 点。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AuthoringPoint3F64 {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) z: f64,
}

/// 一条编制曲线段的闭合几何载荷。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum AuthoringCurveSegmentGeometry {
    Line {
        end: AuthoringPoint3F64,
    },
    CubicBezier {
        control_1: AuthoringPoint3F64,
        control_2: AuthoringPoint3F64,
        end: AuthoringPoint3F64,
    },
}

/// 共同 Typed AST 中一条带来源位置的 owner-local 曲线段。
#[allow(
    dead_code,
    reason = "consumed by the following topology/geometry compiler slice"
)]
pub(crate) struct AuthoringCurveSegmentDeclaration {
    pub(crate) geometry: AuthoringCurveSegmentGeometry,
    pub(crate) span: SourceLocation,
}

/// 共同 Typed AST 中一条从显式起点开始的非空编制曲线。
#[allow(
    dead_code,
    reason = "consumed by the following topology/geometry compiler slice"
)]
pub(crate) struct AuthoringCurveProgramDeclaration {
    pub(crate) start: AuthoringPoint3F64,
    pub(crate) start_span: SourceLocation,
    pub(crate) segments: Box<[AuthoringCurveSegmentDeclaration]>,
}

/// 一个 canonical frame 内不分配 StableId 的道路走向。
pub(crate) struct RoadAlignmentDeclaration {
    #[allow(
        dead_code,
        reason = "consumed by the following topology/geometry compiler slice"
    )]
    pub(crate) road_alignment_key: Arc<str>,
    pub(crate) canonical_frame: OwnedEntityReference<CanonicalFrameKind>,
    pub(crate) reference_line: AuthoringCurveProgramDeclaration,
    pub(crate) span: SourceLocation,
}

impl RoadAlignmentDeclaration {
    pub(crate) fn try_visit_source_locations<E>(
        &self,
        mut visit: impl FnMut(&SourceLocation) -> Result<(), E>,
    ) -> Result<(), E> {
        visit(&self.span)?;
        try_visit_reference(&self.canonical_frame, &mut visit)?;
        try_visit_authoring_curve(&self.reference_line, &mut visit)
    }
}

/// authoring station 区间的闭合终点。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum AuthoringStationEnd {
    Finite(f64),
    AlignmentEnd,
}

/// corridor station 区间内的线性宽度。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AuthoringWidthProfile {
    pub(crate) start_width_meters: f64,
    pub(crate) end_width_meters: f64,
}

/// authoring lane 相对 alignment reference 参数方向的行驶方向。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AuthoringLaneDirection {
    Forward,
    Backward,
}

/// 需要由 topology/geometry lowering 派生的 corridor 几何语义。
#[allow(
    dead_code,
    reason = "consumed by the following topology/geometry compiler slice"
)]
pub(crate) struct RoadCorridorAuthoringGeometry {
    pub(crate) road_alignment_key: Arc<str>,
    pub(crate) start_station_meters: f64,
    pub(crate) end_station: AuthoringStationEnd,
    pub(crate) reference_lane: OwnedEntityReference<AuthoringLaneKind>,
}

/// 需要由 topology/geometry lowering 派生的一条编制车道几何语义。
#[allow(
    dead_code,
    reason = "consumed by the following topology/geometry compiler slice"
)]
pub(crate) struct AuthoringLaneGeometry {
    pub(crate) direction: AuthoringLaneDirection,
    pub(crate) width_profile: AuthoringWidthProfile,
}

/// 规范点表中连续范围到 authoring source segment 的压缩来源映射。
#[allow(
    dead_code,
    reason = "consumed by the following spatial HIR/MIR/LIR source-map slice"
)]
pub(crate) struct CompiledGeometrySourceRange {
    pub(crate) point_start: u32,
    pub(crate) point_end_exclusive: u32,
    pub(crate) source_segment_ordinal: u32,
    pub(crate) source: SourceLocation,
}

/// RoadEditingSource authoring 几何已冻结为共同规范点表后的 LaneEdge 权威。
pub(crate) struct CompiledLaneEdgeGeometry {
    pub(crate) length: EdgeLength,
    /// section-derived edge 继承 alignment frame；junction-internal edge 在 HIR 从 path 推导。
    pub(crate) canonical_frame: Option<OwnedEntityReference<CanonicalFrameKind>>,
    pub(crate) centerline_points: Box<[CanonicalPoint3F32Input]>,
    #[allow(
        dead_code,
        reason = "consumed by the following spatial HIR/MIR/LIR source-map slice"
    )]
    pub(crate) source_ranges: Box<[CompiledGeometrySourceRange]>,
}

/// 不可遍历 FacilityBand 的规范中心线；后续 LIR 以独立稀疏范围表保存。
pub(crate) struct CompiledFacilityBandGeometry {
    pub(crate) length: EdgeLength,
    pub(crate) canonical_frame: OwnedEntityReference<CanonicalFrameKind>,
    pub(crate) centerline_points: Box<[CanonicalPoint3F32Input]>,
    pub(crate) source_ranges: Box<[CompiledGeometrySourceRange]>,
}

/// LaneEdge 的唯一几何权威；它表达语义形状，不表达来源编码。
pub(crate) enum LaneEdgeGeometryAuthority {
    /// 现有 Synthetic 前端已经给出 Traffic length；显式点列由 canonical frame 声明提供。
    DirectLength(EdgeLength),
    /// RoadEditingSource 由横断面派生，或者以路口内部显式曲线编译最终长度和点列。
    Authoring {
        explicit_curve: Option<AuthoringCurveProgramDeclaration>,
    },
    Compiled(CompiledLaneEdgeGeometry),
}

impl LaneEdgeGeometryAuthority {
    pub(crate) const fn direct_length(&self) -> Option<EdgeLength> {
        match self {
            Self::DirectLength(length) => Some(*length),
            Self::Compiled(geometry) => Some(geometry.length),
            Self::Authoring { .. } => None,
        }
    }
}

/// 已通过字段级与模块内约束检查的车道图边 Typed AST 记录。
pub(crate) struct LaneEdgeDeclaration {
    pub(crate) header: DeclarationHeader,
    pub(crate) geometry_authority: LaneEdgeGeometryAuthority,
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
    pub(crate) authoring_geometry: Option<RoadCorridorAuthoringGeometry>,
}

/// 已通过字段级检查的编制车道 Typed AST 记录。
pub(crate) struct AuthoringLaneDeclaration {
    pub(crate) header: DeclarationHeader,
    pub(crate) section_relation_span: SourceLocation,
    pub(crate) edge_chain: Box<[OwnedEntityReference<LaneEdgeKind>]>,
    pub(crate) lane_group: Option<OwnedEntityReference<LaneGroupKind>>,
    #[allow(
        dead_code,
        reason = "consumed by the following topology/geometry compiler slice"
    )]
    pub(crate) authoring_geometry: Option<AuthoringLaneGeometry>,
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
    #[allow(
        dead_code,
        reason = "consumed by the following topology/geometry compiler slice"
    )]
    pub(crate) authoring_width_profile: Option<AuthoringWidthProfile>,
    pub(crate) compiled_geometry: Option<CompiledFacilityBandGeometry>,
}

/// 已通过字段级检查、等待解析显式边界并反向形成非空 Movement 成员集的路口 Typed AST 记录。
pub(crate) struct JunctionDeclaration {
    pub(crate) header: DeclarationHeader,
    /// 来源显式声明的全部 approach edge；不产生新的稳定实体或 LIR 表行。
    ///
    /// Synthetic 前端没有这一独立来源集合，因此保持为空；RoadEditingSource 用它在 HIR
    /// 阶段验证引用存在性以及 junction boundary/internal role 闭包。
    pub(crate) approach_edges: Box<[OwnedEntityReference<LaneEdgeKind>]>,
    /// 来源显式声明的全部 junction-internal edge；HIR 要求它与所属路径内部边的并集
    /// 精确相等，并且与所有路口 approach 集全局不交叠。
    pub(crate) internal_edges: Box<[OwnedEntityReference<LaneEdgeKind>]>,
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
    pub(crate) signal_control: OwnedSignalControl,
}

/// Typed AST 中已拥有的机动门信号控制绑定。
pub(crate) enum OwnedSignalControl {
    Group(OwnedEntityReference<SignalGroupKind>),
    None,
}

pub(crate) struct SignalGroupDeclaration {
    pub(crate) header: DeclarationHeader,
}

pub(crate) struct SignalGroupStateDeclaration {
    pub(crate) signal_group: OwnedEntityReference<SignalGroupKind>,
    pub(crate) aspect: SignalAspect,
}

pub(crate) struct SignalPhaseDeclaration {
    pub(crate) header: DeclarationHeader,
    /// 该相位在所属控制器有序 `signal_phases` 关系中的来源位置。
    pub(crate) controller_relation_span: SourceLocation,
    pub(crate) duration_ms: u64,
    pub(crate) states: Box<[SignalGroupStateDeclaration]>,
}

pub(crate) struct SignalControllerDeclaration {
    pub(crate) header: DeclarationHeader,
    pub(crate) offset_ms: u64,
    pub(crate) signal_groups: Box<[OwnedEntityReference<SignalGroupKind>]>,
    pub(crate) phases: Box<[SignalPhaseDeclaration]>,
}

/// 已通过字段级检查、等待反向成员闭包的停车区域 Typed AST 记录。
pub(crate) struct ParkingAreaDeclaration {
    pub(crate) header: DeclarationHeader,
}

/// Typed AST 中拥有的停车锚点。
pub(crate) struct ParkingLaneAnchorDeclaration {
    pub(crate) lane_edge: OwnedEntityReference<LaneEdgeKind>,
    pub(crate) progress_meters: f64,
}

/// 已通过字段级检查、等待解析区域、锚点和几何的停车位 Typed AST 记录。
pub(crate) struct ParkingSpaceDeclaration {
    pub(crate) header: DeclarationHeader,
    pub(crate) parking_area: Option<OwnedEntityReference<ParkingAreaKind>>,
    pub(crate) entry: ParkingLaneAnchorDeclaration,
    pub(crate) exit: ParkingLaneAnchorDeclaration,
    pub(crate) geometry: ParkingSpaceGeometryInput,
}

/// 已通过字段级检查、等待解析父类并编译层级区间的参与者类别 Typed AST 记录。
pub(crate) struct ParticipantClassDeclaration {
    pub(crate) header: DeclarationHeader,
    pub(crate) extends: Option<OwnedEntityReference<ParticipantClassKind>>,
}

/// 已通过字段级检查、等待解析唯一参与者类别的车辆配置 Typed AST 记录。
pub(crate) struct VehicleProfileDeclaration {
    pub(crate) header: DeclarationHeader,
    pub(crate) participant_class: OwnedEntityReference<ParticipantClassKind>,
    pub(crate) iidm: IidmVehicleProfileInput,
}

/// 已通过字段级检查、等待冻结稳定身份的规范坐标框架 Typed AST 记录。
pub(crate) struct CanonicalFrameDeclaration {
    pub(crate) header: DeclarationHeader,
    pub(crate) lane_edge_geometries: Box<[LaneEdgeGeometryDeclaration]>,
}

/// Typed AST 中已拥有的一条车道图边规范中心线。
pub(crate) struct LaneEdgeGeometryDeclaration {
    pub(crate) lane_edge: OwnedEntityReference<LaneEdgeKind>,
    pub(crate) centerline_points: Box<[CanonicalPoint3F32Input]>,
}

/// Typed AST 中已拥有的准入目标引用。
pub(crate) enum OwnedAccessRuleTarget {
    LaneEdge(OwnedEntityReference<LaneEdgeKind>),
    LaneGroup(OwnedEntityReference<LaneGroupKind>),
    RoadSection(OwnedEntityReference<RoadSectionKind>),
    ManeuverPath(OwnedEntityReference<ManeuverPathKind>),
    FacilityBand(OwnedEntityReference<FacilityBandKind>),
}

/// Typed AST 中已拥有的法规来源信息。
pub(crate) struct OwnedAccessRegulation {
    pub(crate) jurisdiction: Arc<str>,
    pub(crate) version: Arc<str>,
    pub(crate) source: Option<Arc<str>>,
}

/// 已通过字段级检查、等待目标/类别解析和组合裁决的静态准入规则记录。
pub(crate) struct AccessRuleDeclaration {
    pub(crate) header: DeclarationHeader,
    pub(crate) target: OwnedAccessRuleTarget,
    pub(crate) effect: AccessEffect,
    pub(crate) participant_classes: Box<[OwnedEntityReference<ParticipantClassKind>]>,
    pub(crate) regulation: Option<OwnedAccessRegulation>,
    pub(crate) priority: i32,
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
pub(crate) enum TypedAstDeclaration {
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
    SignalGroup(SignalGroupDeclaration),
    SignalController(SignalControllerDeclaration),
    ParkingArea(ParkingAreaDeclaration),
    ParkingSpace(ParkingSpaceDeclaration),
    ParticipantClass(ParticipantClassDeclaration),
    VehicleProfile(VehicleProfileDeclaration),
    CanonicalFrame(CanonicalFrameDeclaration),
    AccessRule(AccessRuleDeclaration),
}

impl TypedAstDeclaration {
    /// 以声明内的规范结构顺序访问全部来源位置。
    ///
    /// 该遍历显式覆盖每个声明变体及其嵌套声明、引用和关系位置，使 HIR 能在产生任何
    /// 语义诊断前统一核对来源文档所有权。它不分配、不改变声明顺序，也不把来源位置
    /// 纳入稳定身份。
    pub(crate) fn try_visit_source_locations<E>(
        &self,
        mut visit: impl FnMut(&SourceLocation) -> Result<(), E>,
    ) -> Result<(), E> {
        match self {
            Self::LaneEdge(LaneEdgeDeclaration {
                header,
                geometry_authority,
                speed_limit: _,
                successors,
            }) => {
                try_visit_declaration_header(header, &mut visit)?;
                try_visit_references(successors, &mut visit)?;
                if let LaneEdgeGeometryAuthority::Authoring {
                    explicit_curve: Some(curve),
                } = geometry_authority
                {
                    try_visit_authoring_curve(curve, &mut visit)?;
                }
                if let LaneEdgeGeometryAuthority::Compiled(geometry) = geometry_authority {
                    if let Some(frame) = &geometry.canonical_frame {
                        try_visit_reference(frame, &mut visit)?;
                    }
                    for range in &geometry.source_ranges {
                        visit(&range.source)?;
                    }
                }
            }
            Self::RoadCorridor(RoadCorridorDeclaration {
                header,
                reference_section,
                elements,
                authoring_geometry,
            }) => {
                try_visit_declaration_header(header, &mut visit)?;
                try_visit_reference(reference_section, &mut visit)?;
                if let Some(authoring_geometry) = authoring_geometry {
                    try_visit_reference(&authoring_geometry.reference_lane, &mut visit)?;
                }
                for element in elements {
                    match element {
                        OwnedCorridorElementReference::RoadSection(reference) => {
                            try_visit_reference(reference, &mut visit)?;
                        }
                        OwnedCorridorElementReference::FacilityBand(reference) => {
                            try_visit_reference(reference, &mut visit)?;
                        }
                    }
                }
            }
            Self::RoadSection(RoadSectionDeclaration {
                header,
                kind_id: _,
                lanes,
            }) => {
                try_visit_declaration_header(header, &mut visit)?;
                for lane in lanes {
                    let AuthoringLaneDeclaration {
                        header,
                        section_relation_span,
                        edge_chain,
                        lane_group,
                        authoring_geometry: _,
                    } = lane;
                    try_visit_declaration_header(header, &mut visit)?;
                    visit(section_relation_span)?;
                    try_visit_references(edge_chain, &mut visit)?;
                    if let Some(lane_group) = lane_group {
                        try_visit_reference(lane_group, &mut visit)?;
                    }
                }
            }
            Self::LaneGroup(LaneGroupDeclaration {
                header,
                road_section,
            }) => {
                try_visit_declaration_header(header, &mut visit)?;
                try_visit_reference(road_section, &mut visit)?;
            }
            Self::FacilityBand(FacilityBandDeclaration {
                header,
                kind_id: _,
                authoring_width_profile: _,
                compiled_geometry,
            }) => {
                try_visit_declaration_header(header, &mut visit)?;
                if let Some(geometry) = compiled_geometry {
                    try_visit_reference(&geometry.canonical_frame, &mut visit)?;
                    for range in &geometry.source_ranges {
                        visit(&range.source)?;
                    }
                }
            }
            Self::SignalGroup(SignalGroupDeclaration { header })
            | Self::ParkingArea(ParkingAreaDeclaration { header }) => {
                try_visit_declaration_header(header, &mut visit)?;
            }
            Self::Junction(JunctionDeclaration {
                header,
                approach_edges,
                internal_edges,
            }) => {
                try_visit_declaration_header(header, &mut visit)?;
                try_visit_references(approach_edges, &mut visit)?;
                try_visit_references(internal_edges, &mut visit)?;
            }
            Self::Movement(MovementDeclaration {
                header,
                junction,
                directed_entry_approach_key: _,
                directed_exit_approach_key: _,
            }) => {
                try_visit_declaration_header(header, &mut visit)?;
                try_visit_reference(junction, &mut visit)?;
            }
            Self::ManeuverPath(ManeuverPathDeclaration {
                header,
                movement,
                entry_edge,
                internal_edges,
                exit_edge,
            }) => {
                try_visit_declaration_header(header, &mut visit)?;
                try_visit_reference(movement, &mut visit)?;
                try_visit_reference(entry_edge, &mut visit)?;
                try_visit_references(internal_edges, &mut visit)?;
                try_visit_reference(exit_edge, &mut visit)?;
            }
            Self::StopLine(StopLineDeclaration { header, lane_edge }) => {
                try_visit_declaration_header(header, &mut visit)?;
                try_visit_reference(lane_edge, &mut visit)?;
            }
            Self::ManeuverGate(ManeuverGateDeclaration {
                header,
                maneuver_path,
                transition_index: _,
                stop_line,
                signal_control,
            }) => {
                try_visit_declaration_header(header, &mut visit)?;
                try_visit_reference(maneuver_path, &mut visit)?;
                try_visit_reference(stop_line, &mut visit)?;
                match signal_control {
                    OwnedSignalControl::Group(group) => {
                        try_visit_reference(group, &mut visit)?;
                    }
                    OwnedSignalControl::None => {}
                }
            }
            Self::WaitingZone(WaitingZoneDeclaration {
                header,
                maneuver_path,
                entry_gate,
                release_gate,
                max_occupancy: _,
            }) => {
                try_visit_declaration_header(header, &mut visit)?;
                try_visit_reference(maneuver_path, &mut visit)?;
                try_visit_reference(entry_gate, &mut visit)?;
                try_visit_reference(release_gate, &mut visit)?;
            }
            Self::StaticRoute(StaticRouteDeclaration {
                header,
                edge_sequence,
            }) => {
                try_visit_declaration_header(header, &mut visit)?;
                try_visit_references(edge_sequence, &mut visit)?;
            }
            Self::SignalController(SignalControllerDeclaration {
                header,
                offset_ms: _,
                signal_groups,
                phases,
            }) => {
                try_visit_declaration_header(header, &mut visit)?;
                try_visit_references(signal_groups, &mut visit)?;
                for phase in phases {
                    let SignalPhaseDeclaration {
                        header,
                        controller_relation_span,
                        duration_ms: _,
                        states,
                    } = phase;
                    try_visit_declaration_header(header, &mut visit)?;
                    visit(controller_relation_span)?;
                    for state in states {
                        let SignalGroupStateDeclaration {
                            signal_group,
                            aspect: _,
                        } = state;
                        try_visit_reference(signal_group, &mut visit)?;
                    }
                }
            }
            Self::ParkingSpace(ParkingSpaceDeclaration {
                header,
                parking_area,
                entry,
                exit,
                geometry: _,
            }) => {
                try_visit_declaration_header(header, &mut visit)?;
                if let Some(parking_area) = parking_area {
                    try_visit_reference(parking_area, &mut visit)?;
                }
                for anchor in [entry, exit] {
                    let ParkingLaneAnchorDeclaration {
                        lane_edge,
                        progress_meters: _,
                    } = anchor;
                    try_visit_reference(lane_edge, &mut visit)?;
                }
            }
            Self::ParticipantClass(ParticipantClassDeclaration { header, extends }) => {
                try_visit_declaration_header(header, &mut visit)?;
                if let Some(extends) = extends {
                    try_visit_reference(extends, &mut visit)?;
                }
            }
            Self::VehicleProfile(VehicleProfileDeclaration {
                header,
                participant_class,
                iidm: _,
            }) => {
                try_visit_declaration_header(header, &mut visit)?;
                try_visit_reference(participant_class, &mut visit)?;
            }
            Self::CanonicalFrame(CanonicalFrameDeclaration {
                header,
                lane_edge_geometries,
            }) => {
                try_visit_declaration_header(header, &mut visit)?;
                for geometry in lane_edge_geometries {
                    let LaneEdgeGeometryDeclaration {
                        lane_edge,
                        centerline_points: _,
                    } = geometry;
                    try_visit_reference(lane_edge, &mut visit)?;
                }
            }
            Self::AccessRule(AccessRuleDeclaration {
                header,
                target,
                effect: _,
                participant_classes,
                regulation: _,
                priority: _,
            }) => {
                try_visit_declaration_header(header, &mut visit)?;
                match target {
                    OwnedAccessRuleTarget::LaneEdge(reference) => {
                        try_visit_reference(reference, &mut visit)?;
                    }
                    OwnedAccessRuleTarget::LaneGroup(reference) => {
                        try_visit_reference(reference, &mut visit)?;
                    }
                    OwnedAccessRuleTarget::RoadSection(reference) => {
                        try_visit_reference(reference, &mut visit)?;
                    }
                    OwnedAccessRuleTarget::ManeuverPath(reference) => {
                        try_visit_reference(reference, &mut visit)?;
                    }
                    OwnedAccessRuleTarget::FacilityBand(reference) => {
                        try_visit_reference(reference, &mut visit)?;
                    }
                }
                try_visit_references(participant_classes, &mut visit)?;
            }
        }
        Ok(())
    }
}

fn try_visit_declaration_header<E>(
    header: &DeclarationHeader,
    visit: &mut impl FnMut(&SourceLocation) -> Result<(), E>,
) -> Result<(), E> {
    let DeclarationHeader {
        entity_kind: _,
        source_address: _,
        stable_key: _,
        span,
    } = header;
    visit(span)
}

fn try_visit_authoring_curve<E>(
    curve: &AuthoringCurveProgramDeclaration,
    visit: &mut impl FnMut(&SourceLocation) -> Result<(), E>,
) -> Result<(), E> {
    visit(&curve.start_span)?;
    for segment in &curve.segments {
        visit(&segment.span)?;
    }
    Ok(())
}

fn try_visit_reference<K: EntityKindMarker, E>(
    reference: &OwnedEntityReference<K>,
    visit: &mut impl FnMut(&SourceLocation) -> Result<(), E>,
) -> Result<(), E> {
    let OwnedEntityReference {
        module_namespace: _,
        target_address: _,
        span,
        marker: _,
    } = reference;
    visit(span)
}

fn try_visit_references<K: EntityKindMarker, E>(
    references: &[OwnedEntityReference<K>],
    visit: &mut impl FnMut(&SourceLocation) -> Result<(), E>,
) -> Result<(), E> {
    for reference in references {
        try_visit_reference(reference, visit)?;
    }
    Ok(())
}

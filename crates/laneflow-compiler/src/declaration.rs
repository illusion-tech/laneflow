//! 合成领域专用语言当前支持的受检声明值。
//!
//! 公共输入仍保留调用方借用的文本；`SyntheticModuleBuilder` 校验标识、数值、导入与
//! 资源上限后，才把它们复制为本模块的拥有型 Typed AST 记录。这里的引用只描述
//! “目标模块命名空间 + 模块内稳定键”，真正的符号解析留给 HIR 阶段完成。

use std::marker::PhantomData;
use std::sync::Arc;

use laneflow_static_contract::{
    EntityKind, EntityKindMarker, FacilityBandKind, LaneEdgeKind, LaneGroupKind, RoadSectionKind,
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

/// 官方合成前端当前支持的封闭声明集合。
pub(crate) enum SyntheticDeclaration {
    LaneEdge(LaneEdgeDeclaration),
    RoadCorridor(RoadCorridorDeclaration),
    RoadSection(RoadSectionDeclaration),
    LaneGroup(LaneGroupDeclaration),
    FacilityBand(FacilityBandDeclaration),
}

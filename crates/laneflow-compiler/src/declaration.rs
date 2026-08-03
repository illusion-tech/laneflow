//! 合成领域专用语言当前支持的受检声明值。
//!
//! 公共输入仍保留调用方借用的文本；`SyntheticModuleBuilder` 校验标识、数值、导入与
//! 资源上限后，才把它们复制为本模块的拥有型 Typed AST 记录。这里的引用只描述
//! “目标模块命名空间 + 模块内稳定键”，真正的符号解析留给 HIR 阶段完成。

use std::marker::PhantomData;
use std::sync::Arc;

use laneflow_static_contract::{EntityKind, EntityKindMarker, LaneEdgeKind};

use crate::SourceSpan;

/// 指向同一编译单元内某类来源声明的有类型未解析引用。
///
/// 类型参数 `K` 防止把不同实体种类的引用混用。构造引用不会查询目标；加入声明时仅
/// 校验拼写和显式导入边界，目标存在性在完整模块图建立后的 HIR 符号解析中验证。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct EntityReference<'a, K: EntityKindMarker> {
    module_namespace: Option<&'a str>,
    declaration_key: &'a str,
    marker: PhantomData<fn() -> K>,
}

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

/// 官方合成前端当前支持的封闭声明集合。
pub(crate) enum SyntheticDeclaration {
    LaneEdge(LaneEdgeDeclaration),
}

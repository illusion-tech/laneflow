use std::marker::PhantomData;
use std::sync::Arc;

use laneflow_static_contract::{EntityKind, EntityKindMarker, LaneEdgeKind};

use crate::SourceSpan;

/// 指向同一编译单元内某类来源声明的有类型未解析引用。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct EntityReference<'a, K: EntityKindMarker> {
    module_namespace: Option<&'a str>,
    declaration_key: &'a str,
    marker: PhantomData<fn() -> K>,
}

impl<'a, K: EntityKindMarker> EntityReference<'a, K> {
    /// 建立指向当前来源模块声明的引用。
    #[must_use]
    pub const fn local(declaration_key: &'a str) -> Self {
        Self {
            module_namespace: None,
            declaration_key,
            marker: PhantomData,
        }
    }

    /// 建立指向显式导入模块声明的引用。
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
#[derive(Clone, Copy, Debug)]
pub struct LaneEdgeInput<'a> {
    /// 来源模块内显式持久化且唯一的稳定边键。
    pub lane_edge_key: &'a str,
    /// 交通权威长度，单位为米。
    pub length_meters: f64,
    /// 基础道路限速，单位为米每秒。
    pub speed_limit_meters_per_second: f64,
    /// 无序、不得重复的显式下游连接集合。
    pub successors: &'a [LaneEdgeReference<'a>],
}

#[derive(Clone)]
pub(crate) struct OwnedEntityReference<K: EntityKindMarker> {
    pub(crate) module_namespace: Arc<str>,
    pub(crate) declaration_key: Arc<str>,
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
pub(crate) struct EdgeLength(f64);

impl EdgeLength {
    pub(crate) const EXCLUSIVE_MINIMUM_METERS: f64 = 1.0e-9;

    pub(crate) fn try_new(value: f64) -> Result<Self, ScalarViolation> {
        validate_greater_than(value, Self::EXCLUSIVE_MINIMUM_METERS).map(Self)
    }

    pub(crate) const fn value(self) -> f64 {
        self.0
    }
}

#[derive(Clone, Copy)]
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

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
/// 受检领域数值不能建立时的结构化原因。
pub enum ScalarViolation {
    /// 输入不是有限数。
    NotFinite,
    /// 输入没有严格大于给定的排他下限。
    NotGreaterThan {
        /// 排他下限的 IEEE 754 位模式。
        exclusive_minimum_bits: u64,
    },
}

pub(crate) struct DeclarationHeader {
    pub(crate) entity_kind: EntityKind,
    pub(crate) stable_key: Arc<str>,
    pub(crate) span: SourceSpan,
}

pub(crate) struct LaneEdgeDeclaration {
    pub(crate) header: DeclarationHeader,
    pub(crate) length: EdgeLength,
    pub(crate) speed_limit: SpeedLimit,
    pub(crate) successors: Box<[OwnedEntityReference<LaneEdgeKind>]>,
}

pub(crate) enum SyntheticDeclaration {
    LaneEdge(LaneEdgeDeclaration),
}

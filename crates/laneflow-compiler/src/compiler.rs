//! 官方来源编译到原子已验证输出的公共入口。
//!
//! [`Compiler::compile`] 是唯一能够构造 [`ValidatedCanonicalLir`]、
//! [`ValidatedSourceMapInput`] 和 [`CompilationOutput`] 的路径。当前实现是干净单工作线程
//! 确定性预言机：每个阶段成功后才提交下一阶段，任一错误只返回
//! [`DiagnosticBundle`]；来源伴随数据在 AST/HIR/MIR 释放前冻结。

use laneflow_static_contract::{
    AuthoringLaneId, AuthoringLaneOrdinal, FacilityBandId, FacilityBandOrdinal, FieldTag,
    JunctionId, JunctionOrdinal, LaneEdgeId, LaneEdgeOrdinal, LaneGroupId, LaneGroupOrdinal,
    ManeuverPathId, ManeuverPathOrdinal, MovementId, MovementOrdinal, RoadCorridorId,
    RoadCorridorOrdinal, RoadSectionId, RoadSectionOrdinal,
};

use crate::hir::build_hir;
use crate::lir::{
    LirAuthoringLane, LirCorridorElement, LirFacilityBand, LirIdentityField, LirJunction,
    LirJunctionInternalEdge, LirLaneEdge, LirLaneGroup, LirManeuverPath, LirMovement,
    LirRoadCorridor, LirRoadSection, LirUnit, freeze_lir,
};
use crate::mir::lower_to_mir;
use crate::source_map::{ValidatedSourceMapInput, freeze_source_map};
use crate::{CompilationUnit, Diagnostic, DiagnosticBundle};

/// 已完成 #292 当前支持子集全部静态语义验证的 Canonical LIR。
///
/// 字段保持私有，调用方只能按规范稳定顺序读取有类型表、身份字段和关系区间。不存在从
/// 裸表、未验证 MIR 或自报状态构造本类型的入口。
pub struct ValidatedCanonicalLir {
    inner: LirUnit,
}

impl ValidatedCanonicalLir {
    /// 按完整 Identity v1 前像规范顺序遍历全部车道图边。
    pub fn lane_edges(&self) -> impl ExactSizeIterator<Item = CanonicalLaneEdgeView<'_>> {
        self.inner
            .lane_edges
            .iter()
            .map(|edge| CanonicalLaneEdgeView {
                lir: &self.inner,
                edge,
            })
    }

    /// 通过当前 LIR 实例的有类型序号读取车道图边。
    ///
    /// 序号来自其他编译结果时可能命中错误实体；跨编译关联必须先使用 `LaneEdgeId`。
    #[must_use]
    pub fn lane_edge(&self, ordinal: LaneEdgeOrdinal) -> Option<CanonicalLaneEdgeView<'_>> {
        self.inner
            .lane_edges
            .get(ordinal.index())
            .map(|edge| CanonicalLaneEdgeView {
                lir: &self.inner,
                edge,
            })
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部道路走廊。
    pub fn road_corridors(&self) -> impl ExactSizeIterator<Item = CanonicalRoadCorridorView<'_>> {
        self.inner
            .road_corridors
            .iter()
            .map(|record| CanonicalRoadCorridorView {
                lir: &self.inner,
                record,
            })
    }

    /// 通过当前 LIR 实例的有类型序号读取道路走廊。
    #[must_use]
    pub fn road_corridor(
        &self,
        ordinal: RoadCorridorOrdinal,
    ) -> Option<CanonicalRoadCorridorView<'_>> {
        self.inner
            .road_corridors
            .get(ordinal.index())
            .map(|record| CanonicalRoadCorridorView {
                lir: &self.inner,
                record,
            })
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部道路区段。
    pub fn road_sections(&self) -> impl ExactSizeIterator<Item = CanonicalRoadSectionView<'_>> {
        self.inner
            .road_sections
            .iter()
            .map(|record| CanonicalRoadSectionView {
                lir: &self.inner,
                record,
            })
    }

    /// 通过当前 LIR 实例的有类型序号读取道路区段。
    #[must_use]
    pub fn road_section(
        &self,
        ordinal: RoadSectionOrdinal,
    ) -> Option<CanonicalRoadSectionView<'_>> {
        self.inner
            .road_sections
            .get(ordinal.index())
            .map(|record| CanonicalRoadSectionView {
                lir: &self.inner,
                record,
            })
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部编制车道。
    pub fn authoring_lanes(&self) -> impl ExactSizeIterator<Item = CanonicalAuthoringLaneView<'_>> {
        self.inner
            .authoring_lanes
            .iter()
            .map(|record| CanonicalAuthoringLaneView {
                lir: &self.inner,
                record,
            })
    }

    /// 通过当前 LIR 实例的有类型序号读取编制车道。
    #[must_use]
    pub fn authoring_lane(
        &self,
        ordinal: AuthoringLaneOrdinal,
    ) -> Option<CanonicalAuthoringLaneView<'_>> {
        self.inner
            .authoring_lanes
            .get(ordinal.index())
            .map(|record| CanonicalAuthoringLaneView {
                lir: &self.inner,
                record,
            })
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部车道组。
    pub fn lane_groups(&self) -> impl ExactSizeIterator<Item = CanonicalLaneGroupView<'_>> {
        self.inner
            .lane_groups
            .iter()
            .map(|record| CanonicalLaneGroupView {
                lir: &self.inner,
                record,
            })
    }

    /// 通过当前 LIR 实例的有类型序号读取车道组。
    #[must_use]
    pub fn lane_group(&self, ordinal: LaneGroupOrdinal) -> Option<CanonicalLaneGroupView<'_>> {
        self.inner
            .lane_groups
            .get(ordinal.index())
            .map(|record| CanonicalLaneGroupView {
                lir: &self.inner,
                record,
            })
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部设施带。
    pub fn facility_bands(&self) -> impl ExactSizeIterator<Item = CanonicalFacilityBandView<'_>> {
        self.inner
            .facility_bands
            .iter()
            .map(|record| CanonicalFacilityBandView {
                lir: &self.inner,
                record,
            })
    }

    /// 通过当前 LIR 实例的有类型序号读取设施带。
    #[must_use]
    pub fn facility_band(
        &self,
        ordinal: FacilityBandOrdinal,
    ) -> Option<CanonicalFacilityBandView<'_>> {
        self.inner
            .facility_bands
            .get(ordinal.index())
            .map(|record| CanonicalFacilityBandView {
                lir: &self.inner,
                record,
            })
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部路口。
    pub fn junctions(&self) -> impl ExactSizeIterator<Item = CanonicalJunctionView<'_>> {
        self.inner
            .junctions
            .iter()
            .map(|record| CanonicalJunctionView {
                lir: &self.inner,
                record,
            })
    }

    /// 通过当前 LIR 实例的有类型序号读取路口。
    #[must_use]
    pub fn junction(&self, ordinal: JunctionOrdinal) -> Option<CanonicalJunctionView<'_>> {
        self.inner
            .junctions
            .get(ordinal.index())
            .map(|record| CanonicalJunctionView {
                lir: &self.inner,
                record,
            })
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部转向动作。
    pub fn movements(&self) -> impl ExactSizeIterator<Item = CanonicalMovementView<'_>> {
        self.inner
            .movements
            .iter()
            .map(|record| CanonicalMovementView {
                lir: &self.inner,
                record,
            })
    }

    /// 通过当前 LIR 实例的有类型序号读取转向动作。
    #[must_use]
    pub fn movement(&self, ordinal: MovementOrdinal) -> Option<CanonicalMovementView<'_>> {
        self.inner
            .movements
            .get(ordinal.index())
            .map(|record| CanonicalMovementView {
                lir: &self.inner,
                record,
            })
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部机动路径。
    pub fn maneuver_paths(&self) -> impl ExactSizeIterator<Item = CanonicalManeuverPathView<'_>> {
        self.inner
            .maneuver_paths
            .iter()
            .map(|record| CanonicalManeuverPathView {
                lir: &self.inner,
                record,
            })
    }

    /// 通过当前 LIR 实例的有类型序号读取机动路径。
    #[must_use]
    pub fn maneuver_path(
        &self,
        ordinal: ManeuverPathOrdinal,
    ) -> Option<CanonicalManeuverPathView<'_>> {
        self.inner
            .maneuver_paths
            .get(ordinal.index())
            .map(|record| CanonicalManeuverPathView {
                lir: &self.inner,
                record,
            })
    }

    /// 按 `LaneEdgeOrdinal` 遍历全部派生的路口内部边所有权。
    ///
    /// 验证阶段已证明每条内部边最多属于一个路口，并且不会同时承担任一路径的入口或出口
    /// 边角色；同一路口的多条路径仍可合法共享同一内部边。
    pub fn junction_internal_edges(
        &self,
    ) -> impl ExactSizeIterator<Item = CanonicalJunctionInternalEdgeView<'_>> {
        self.inner
            .junction_internal_edges
            .iter()
            .map(|record| CanonicalJunctionInternalEdgeView { record })
    }

    /// 返回一条车道图边的派生路口内部所有者；边不承担内部角色时返回 `None`。
    #[must_use]
    pub fn junction_internal_owner(&self, edge: LaneEdgeOrdinal) -> Option<JunctionOrdinal> {
        self.inner
            .junction_internal_edges
            .binary_search_by_key(&edge, |relation| relation.edge)
            .ok()
            .map(|index| self.inner.junction_internal_edges[index].junction)
    }
}

/// Canonical LIR 中一条 `LaneEdge` 记录的借用视图。
#[derive(Clone, Copy)]
pub struct CanonicalLaneEdgeView<'a> {
    lir: &'a LirUnit,
    edge: &'a LirLaneEdge,
}

impl CanonicalLaneEdgeView<'_> {
    /// 返回当前表中的有类型逻辑序号。
    #[must_use]
    pub const fn ordinal(&self) -> LaneEdgeOrdinal {
        self.edge.ordinal
    }

    /// 返回由完整 Identity v1 前像派生的稳定标识。
    #[must_use]
    pub const fn stable_id(&self) -> LaneEdgeId {
        self.edge.stable_id
    }

    /// 按 Identity v1 登记顺序遍历完整规范身份字段。
    pub fn identity_fields(&self) -> impl ExactSizeIterator<Item = CanonicalIdentityFieldView<'_>> {
        self.lir.identity_fields[self.edge.identity_fields.as_usize_range()]
            .iter()
            .map(|field| CanonicalIdentityFieldView {
                identity_field_bytes: &self.lir.identity_field_bytes,
                field,
            })
    }

    /// 返回交通权威长度，单位为米。
    #[must_use]
    pub const fn length_meters(&self) -> f64 {
        self.edge.length_meters
    }

    /// 返回基础道路限速，单位为米每秒。
    #[must_use]
    pub const fn speed_limit_meters_per_second(&self) -> f64 {
        self.edge.speed_limit_meters_per_second
    }

    /// 返回按领域顺序冻结的下游边有类型序号。
    #[must_use]
    pub fn successors(&self) -> &[LaneEdgeOrdinal] {
        &self.lir.lane_edge_successors[self.edge.successors.as_usize_range()]
    }
}

macro_rules! impl_stable_entity_view {
    ($view:ident, $record:ty, $ordinal:ty, $id:ty) => {
        /// Canonical LIR 中一个已验证稳定实体的借用视图。
        #[derive(Clone, Copy)]
        pub struct $view<'a> {
            lir: &'a LirUnit,
            record: &'a $record,
        }

        impl $view<'_> {
            /// 返回当前实体表中的有类型逻辑序号。
            #[must_use]
            pub const fn ordinal(&self) -> $ordinal {
                self.record.ordinal
            }

            /// 返回由完整 Identity v1 前像派生的有类型稳定标识。
            #[must_use]
            pub const fn stable_id(&self) -> $id {
                self.record.stable_id
            }

            /// 按 Identity v1 登记顺序遍历完整规范身份字段。
            pub fn identity_fields(
                &self,
            ) -> impl ExactSizeIterator<Item = CanonicalIdentityFieldView<'_>> {
                self.lir.identity_fields[self.record.identity_fields.as_usize_range()]
                    .iter()
                    .map(|field| CanonicalIdentityFieldView {
                        identity_field_bytes: &self.lir.identity_field_bytes,
                        field,
                    })
            }
        }
    };
}

impl_stable_entity_view!(
    CanonicalRoadCorridorView,
    LirRoadCorridor,
    RoadCorridorOrdinal,
    RoadCorridorId
);
impl_stable_entity_view!(
    CanonicalRoadSectionView,
    LirRoadSection,
    RoadSectionOrdinal,
    RoadSectionId
);
impl_stable_entity_view!(
    CanonicalAuthoringLaneView,
    LirAuthoringLane,
    AuthoringLaneOrdinal,
    AuthoringLaneId
);
impl_stable_entity_view!(
    CanonicalLaneGroupView,
    LirLaneGroup,
    LaneGroupOrdinal,
    LaneGroupId
);
impl_stable_entity_view!(
    CanonicalFacilityBandView,
    LirFacilityBand,
    FacilityBandOrdinal,
    FacilityBandId
);
impl_stable_entity_view!(
    CanonicalJunctionView,
    LirJunction,
    JunctionOrdinal,
    JunctionId
);
impl_stable_entity_view!(
    CanonicalMovementView,
    LirMovement,
    MovementOrdinal,
    MovementId
);
impl_stable_entity_view!(
    CanonicalManeuverPathView,
    LirManeuverPath,
    ManeuverPathOrdinal,
    ManeuverPathId
);

/// 道路走廊有序横断面中的一项有类型成员。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CanonicalCorridorElement {
    /// 一个承载编制车道的有方向道路区段。
    RoadSection(RoadSectionOrdinal),
    /// 一个不进入遍历图的非方向设施带。
    FacilityBand(FacilityBandOrdinal),
}

impl CanonicalRoadCorridorView<'_> {
    /// 返回定义横断面参考方向、且已证明属于本走廊的道路区段。
    #[must_use]
    pub const fn reference_section(&self) -> RoadSectionOrdinal {
        self.record.reference_section
    }

    /// 按走廊参考方向从左到右遍历横断面成员；该顺序具有领域语义。
    pub fn elements(&self) -> impl ExactSizeIterator<Item = CanonicalCorridorElement> + '_ {
        self.lir.corridor_elements[self.record.elements.as_usize_range()]
            .iter()
            .map(|element| match element {
                LirCorridorElement::RoadSection(ordinal) => {
                    CanonicalCorridorElement::RoadSection(*ordinal)
                }
                LirCorridorElement::FacilityBand(ordinal) => {
                    CanonicalCorridorElement::FacilityBand(*ordinal)
                }
            })
    }
}

impl CanonicalRoadSectionView<'_> {
    /// 返回唯一拥有本区段的道路走廊。
    #[must_use]
    pub const fn road_corridor(&self) -> RoadCorridorOrdinal {
        self.record.road_corridor
    }

    /// 返回已验证为 lane-bearing 类别的物理设施 token。
    #[must_use]
    pub fn kind_id(&self) -> &str {
        &self.record.kind_id
    }

    /// 返回按走廊参考方向从左到右排列的编制车道序号。
    #[must_use]
    pub fn lanes(&self) -> &[AuthoringLaneOrdinal] {
        &self.lir.road_section_lanes[self.record.lanes.as_usize_range()]
    }
}

impl CanonicalAuthoringLaneView<'_> {
    /// 返回唯一拥有本编制车道的道路区段。
    #[must_use]
    pub const fn road_section(&self) -> RoadSectionOrdinal {
        self.record.road_section
    }

    /// 返回沿行驶方向排列、已证明直接连通的车道图边覆盖链。
    #[must_use]
    pub fn edge_chain(&self) -> &[LaneEdgeOrdinal] {
        &self.lir.authoring_lane_edges[self.record.edge_chain.as_usize_range()]
    }

    /// 返回可选车道组；存在时已证明与本车道属于同一道路区段。
    #[must_use]
    pub const fn lane_group(&self) -> Option<LaneGroupOrdinal> {
        self.record.lane_group
    }
}

impl CanonicalLaneGroupView<'_> {
    /// 返回唯一拥有本组的道路区段。
    #[must_use]
    pub const fn road_section(&self) -> RoadSectionOrdinal {
        self.record.road_section
    }

    /// 返回非空且全部属于同一父区段的编制车道成员。
    #[must_use]
    pub fn members(&self) -> &[AuthoringLaneOrdinal] {
        &self.lir.lane_group_members[self.record.members.as_usize_range()]
    }
}

impl CanonicalFacilityBandView<'_> {
    /// 返回唯一拥有本设施带的道路走廊。
    #[must_use]
    pub const fn road_corridor(&self) -> RoadCorridorOrdinal {
        self.record.road_corridor
    }

    /// 返回已验证为 non-traversable 类别的物理设施 token。
    #[must_use]
    pub fn kind_id(&self) -> &str {
        &self.record.kind_id
    }
}

impl CanonicalJunctionView<'_> {
    /// 返回本路口拥有的非空转向动作集合。
    #[must_use]
    pub fn movements(&self) -> &[MovementOrdinal] {
        &self.lir.junction_movements[self.record.movements.as_usize_range()]
    }
}

impl CanonicalMovementView<'_> {
    /// 返回唯一拥有本转向动作的路口。
    #[must_use]
    pub const fn junction(&self) -> JunctionOrdinal {
        self.record.junction
    }

    /// 返回参与 Identity v1 的有向入口接近键；该键由编制端显式提供，编译器不从几何推断。
    #[must_use]
    pub fn directed_entry_approach_key(&self) -> &str {
        &self.record.directed_entry_approach_key
    }

    /// 返回参与 Identity v1 的有向出口接近键；该键由编制端显式提供，编译器不从几何推断。
    #[must_use]
    pub fn directed_exit_approach_key(&self) -> &str {
        &self.record.directed_exit_approach_key
    }

    /// 返回本转向动作拥有的非空机动路径集合。
    #[must_use]
    pub fn maneuver_paths(&self) -> &[ManeuverPathOrdinal] {
        &self.lir.movement_maneuver_paths[self.record.maneuver_paths.as_usize_range()]
    }
}

impl CanonicalManeuverPathView<'_> {
    /// 返回唯一拥有本机动路径的转向动作。
    #[must_use]
    pub const fn movement(&self) -> MovementOrdinal {
        self.record.movement
    }

    /// 返回完整且已验证直接连通的 `entry + internal + exit` 车道图边序列。
    #[must_use]
    pub fn edges(&self) -> &[LaneEdgeOrdinal] {
        &self.lir.maneuver_path_edges[self.record.edges.as_usize_range()]
    }

    /// 返回完整路径序列的入口边。
    #[must_use]
    pub fn entry_edge(&self) -> LaneEdgeOrdinal {
        self.edges()[0]
    }

    /// 返回完整路径序列中可为空的内部边切片。
    #[must_use]
    pub fn internal_edges(&self) -> &[LaneEdgeOrdinal] {
        let edges = self.edges();
        &edges[1..edges.len() - 1]
    }

    /// 返回完整路径序列的出口边。
    #[must_use]
    pub fn exit_edge(&self) -> LaneEdgeOrdinal {
        let edges = self.edges();
        edges[edges.len() - 1]
    }
}

/// Canonical LIR 中一条派生路口内部边所有权的借用视图。
#[derive(Clone, Copy)]
pub struct CanonicalJunctionInternalEdgeView<'a> {
    record: &'a LirJunctionInternalEdge,
}

impl CanonicalJunctionInternalEdgeView<'_> {
    /// 返回承担路口内部角色的车道图边。
    #[must_use]
    pub const fn edge(&self) -> LaneEdgeOrdinal {
        self.record.edge
    }

    /// 返回该内部边的唯一所有者路口。
    #[must_use]
    pub const fn junction(&self) -> JunctionOrdinal {
        self.record.junction
    }
}

/// Canonical LIR 共享身份字段池中的一项借用视图。
#[derive(Clone, Copy)]
pub struct CanonicalIdentityFieldView<'a> {
    identity_field_bytes: &'a [u8],
    field: &'a LirIdentityField,
}

impl CanonicalIdentityFieldView<'_> {
    /// 返回 Identity v1 登记字段标签。
    #[must_use]
    pub const fn tag(&self) -> FieldTag {
        self.field.tag
    }

    /// 返回字段的完整规范值字节，不包含标签和长度前缀。
    #[must_use]
    pub fn value_bytes(&self) -> &[u8] {
        &self.identity_field_bytes[self.field.value_bytes.as_usize_range()]
    }
}

/// 一次成功编译原子拥有的已验证结果。
///
/// LIR 与来源伴随数据不能分别构造或重新配对；后继源映射后端必须从同一个实例同时借用
/// 二者。当前支持子集不产生 warning/note，因此 `diagnostics` 为空，但该成功契约保留
/// 非错误级诊断通道。
pub struct CompilationOutput {
    lir: ValidatedCanonicalLir,
    source_map_input: ValidatedSourceMapInput,
    diagnostics: Box<[Diagnostic]>,
}

impl CompilationOutput {
    /// 借用所有静态语义后端的唯一输入。
    #[must_use]
    pub const fn lir(&self) -> &ValidatedCanonicalLir {
        &self.lir
    }

    /// 借用仅供源映射/诊断后端使用的来源伴随数据。
    #[must_use]
    pub const fn source_map_input(&self) -> &ValidatedSourceMapInput {
        &self.source_map_input
    }

    /// 返回成功路径保留的非错误级诊断。
    #[must_use]
    pub const fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }
}

/// 可复用的 LaneFlow 静态路网编译器。
///
/// 当前干净单线程预言机没有跨编译语义状态；因此失败后无需清理缓存，也不可能让上次
/// 编译污染下一次结果。后继若加入容量复用，仍必须维持这一可观察契约。
pub struct Compiler {
    _private: (),
}

// #292 G1 只冻结显式 `Compiler::new()`，没有授权额外的公共 `Default` 构造契约。
#[allow(clippy::new_without_default)]
impl Compiler {
    /// 建立一个没有隐式输入、线程配置或无限资源模式的编译器实例。
    #[must_use]
    pub const fn new() -> Self {
        Self { _private: () }
    }

    /// 消费一个受检编译单元并运行当前已实现的 #292 领域子集编译管线。
    ///
    /// # Errors
    ///
    /// 任一阶段出现错误级语义诊断或超过 `CompilationUnit` 携带的资源配置档时，返回
    /// 规范排序的 [`DiagnosticBundle`]。错误路径不会返回部分 LIR 或部分源映射输入；
    /// 同一个 `Compiler` 可以立即用于下一次编译。
    pub fn compile(
        &mut self,
        unit: CompilationUnit,
    ) -> Result<CompilationOutput, DiagnosticBundle> {
        let hir = build_hir(&unit)?;
        let mir = lower_to_mir(&unit, &hir)?;
        // MIR 已拥有后继阶段所需的完整语义与来源位置；尽早释放 HIR，避免把阶段共存
        // 时间延长到 LIR/source-map 冻结并破坏资源峰值模型。
        drop(hir);
        let frozen_lir = freeze_lir(&unit, &mir)?;
        let source_map_input = freeze_source_map(unit, &mir, &frozen_lir)?;
        drop(mir);
        let crate::lir::LirFreezeOutput { lir, .. } = frozen_lir;
        Ok(CompilationOutput {
            lir: ValidatedCanonicalLir { inner: lir },
            source_map_input,
            diagnostics: Box::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AuthoringLaneInput, CompilationUnitBuilder, CompileLimitDimension, CompileLimits,
        CorridorElementReference, DiagnosticCode, DiagnosticPayload, FacilityBandInput,
        FacilityBandReference, JunctionInput, JunctionReference, LaneEdgeInput, LaneEdgeReference,
        LaneGroupInput, LaneGroupReference, ManeuverPathInput, MovementInput, MovementReference,
        RoadCorridorInput, RoadSectionInput, RoadSectionReference, SourceModuleDescriptor,
        SourceModuleHeader, SourceModuleHeaderInput, SourceRelationRole, SyntheticModule,
        SyntheticModuleBuilder,
    };

    fn module(
        namespace: &str,
        document: &str,
        imports: &[&str],
        edges: &[(&str, f64, &[LaneEdgeReference<'_>])],
    ) -> SyntheticModule {
        let limits = CompileLimits::p100_initial_v1();
        let header = SourceModuleHeader::new(
            SourceModuleHeaderInput {
                authoring_namespace_id: namespace,
                source_document_key: document,
                generator_build_id: "git:0123456789abcdef",
                parameters_and_inputs_digest: [0x11; 32],
                frontend_options_digest: [0x22; 32],
                random_seed: Some(42),
                provenance: "repository:laneflow",
            },
            &limits,
        )
        .unwrap();
        let mut builder = SyntheticModuleBuilder::new(header, &limits).unwrap();
        for import in imports {
            builder.add_import(import).unwrap();
        }
        for (key, length_meters, successors) in edges {
            builder
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: key,
                    length_meters: *length_meters,
                    speed_limit_meters_per_second: 13.75,
                    successors,
                })
                .unwrap();
        }
        builder.finish().unwrap()
    }

    fn unit(modules: impl IntoIterator<Item = SyntheticModule>) -> CompilationUnit {
        let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
        for module in modules {
            builder.add_synthetic_module(module).unwrap();
        }
        builder.build().unwrap()
    }

    fn cross_section_module(permuted: bool) -> SyntheticModule {
        let limits = CompileLimits::p100_initial_v1();
        let header = SourceModuleHeader::new(
            SourceModuleHeaderInput {
                authoring_namespace_id: "city/cross-section",
                source_document_key: "cross-section.document",
                generator_build_id: "git:0123456789abcdef",
                parameters_and_inputs_digest: [0x11; 32],
                frontend_options_digest: [0x22; 32],
                random_seed: Some(42),
                provenance: "repository:laneflow",
            },
            &limits,
        )
        .unwrap();
        let mut builder = SyntheticModuleBuilder::new(header, &limits).unwrap();
        let add_edges = |builder: &mut SyntheticModuleBuilder| {
            builder
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "edge-a",
                    length_meters: 10.0,
                    speed_limit_meters_per_second: 12.0,
                    successors: &[LaneEdgeReference::local("edge-b")],
                })
                .unwrap()
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "edge-b",
                    length_meters: 12.0,
                    speed_limit_meters_per_second: 12.0,
                    successors: &[],
                })
                .unwrap();
        };
        let add_band = |builder: &mut SyntheticModuleBuilder| {
            builder
                .add_facility_band(FacilityBandInput {
                    facility_band_key: "sidewalk-left",
                    kind_id: "sidewalk",
                })
                .unwrap();
        };
        let add_group = |builder: &mut SyntheticModuleBuilder| {
            builder
                .add_lane_group(LaneGroupInput {
                    lane_group_key: "through",
                    road_section: RoadSectionReference::local("carriageway"),
                })
                .unwrap();
        };
        let add_section = |builder: &mut SyntheticModuleBuilder| {
            builder
                .add_road_section(RoadSectionInput {
                    road_section_key: "carriageway",
                    kind_id: "motorLane",
                    lanes: &[AuthoringLaneInput {
                        authoring_lane_key: "lane-main",
                        edge_chain: &[
                            LaneEdgeReference::local("edge-a"),
                            LaneEdgeReference::local("edge-b"),
                        ],
                        lane_group: Some(LaneGroupReference::local("through")),
                    }],
                })
                .unwrap();
        };
        let add_corridor = |builder: &mut SyntheticModuleBuilder| {
            builder
                .add_road_corridor(RoadCorridorInput {
                    road_corridor_key: "main-road",
                    reference_section: RoadSectionReference::local("carriageway"),
                    elements: &[
                        CorridorElementReference::facility_band(FacilityBandReference::local(
                            "sidewalk-left",
                        )),
                        CorridorElementReference::road_section(RoadSectionReference::local(
                            "carriageway",
                        )),
                    ],
                })
                .unwrap();
        };

        if permuted {
            add_corridor(&mut builder);
            add_section(&mut builder);
            add_group(&mut builder);
            add_band(&mut builder);
            add_edges(&mut builder);
        } else {
            add_edges(&mut builder);
            add_band(&mut builder);
            add_group(&mut builder);
            add_section(&mut builder);
            add_corridor(&mut builder);
        }
        builder.finish().unwrap()
    }

    fn junction_builder(document: &str) -> SyntheticModuleBuilder {
        let limits = CompileLimits::p100_initial_v1();
        let header = SourceModuleHeader::new(
            SourceModuleHeaderInput {
                authoring_namespace_id: "city/junction",
                source_document_key: document,
                generator_build_id: "git:0123456789abcdef",
                parameters_and_inputs_digest: [0x11; 32],
                frontend_options_digest: [0x22; 32],
                random_seed: Some(42),
                provenance: "repository:laneflow",
            },
            &limits,
        )
        .unwrap();
        SyntheticModuleBuilder::new(header, &limits).unwrap()
    }

    fn junction_module(permuted: bool, selected_internal: &'static str) -> SyntheticModule {
        let mut builder = junction_builder(if permuted {
            "junction-permuted.document"
        } else {
            "junction.document"
        });
        let add_edges = |builder: &mut SyntheticModuleBuilder| {
            let internal_successors = [LaneEdgeReference::local("exit")];
            let entry_successors = [
                LaneEdgeReference::local("internal-a"),
                LaneEdgeReference::local("internal-b"),
            ];
            builder
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "entry-a",
                    length_meters: 10.0,
                    speed_limit_meters_per_second: 10.0,
                    successors: &entry_successors,
                })
                .unwrap()
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "entry-b",
                    length_meters: 10.0,
                    speed_limit_meters_per_second: 10.0,
                    successors: &entry_successors,
                })
                .unwrap()
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "internal-a",
                    length_meters: 8.0,
                    speed_limit_meters_per_second: 8.0,
                    successors: &internal_successors,
                })
                .unwrap()
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "internal-b",
                    length_meters: 8.0,
                    speed_limit_meters_per_second: 8.0,
                    successors: &internal_successors,
                })
                .unwrap()
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "exit",
                    length_meters: 12.0,
                    speed_limit_meters_per_second: 10.0,
                    successors: &[],
                })
                .unwrap();
        };
        let add_junction = |builder: &mut SyntheticModuleBuilder| {
            builder
                .add_junction(JunctionInput {
                    junction_key: "junction-main",
                })
                .unwrap();
        };
        let add_movement = |builder: &mut SyntheticModuleBuilder| {
            builder
                .add_movement(MovementInput {
                    movement_key: "movement-through",
                    junction: JunctionReference::local("junction-main"),
                    directed_entry_approach_key: "approach-westbound",
                    directed_exit_approach_key: "approach-eastbound",
                })
                .unwrap();
        };
        let add_path = |builder: &mut SyntheticModuleBuilder, key: &str, entry: &str| {
            let internal = [LaneEdgeReference::local(selected_internal)];
            builder
                .add_maneuver_path(ManeuverPathInput {
                    maneuver_path_key: key,
                    movement: MovementReference::local("movement-through"),
                    entry_edge: LaneEdgeReference::local(entry),
                    internal_edges: &internal,
                    exit_edge: LaneEdgeReference::local("exit"),
                })
                .unwrap();
        };

        if permuted {
            add_path(&mut builder, "path-b", "entry-b");
            add_path(&mut builder, "path-a", "entry-a");
            add_movement(&mut builder);
            add_junction(&mut builder);
            add_edges(&mut builder);
        } else {
            add_edges(&mut builder);
            add_junction(&mut builder);
            add_movement(&mut builder);
            add_path(&mut builder, "path-a", "entry-a");
            add_path(&mut builder, "path-b", "entry-b");
        }
        builder.finish().unwrap()
    }

    fn stable_key<'a>(
        mut fields: impl Iterator<Item = CanonicalIdentityFieldView<'a>>,
        tag: FieldTag,
    ) -> String {
        fields
            .find(|field| field.tag() == tag)
            .map(|field| String::from_utf8(field.value_bytes().to_vec()).unwrap())
            .unwrap()
    }

    fn compile_diagnostic_codes(builder: SyntheticModuleBuilder) -> Vec<DiagnosticCode> {
        match Compiler::new().compile(unit([builder.finish().unwrap()])) {
            Ok(_) => panic!("expected junction topology validation failure"),
            Err(diagnostics) => diagnostics
                .diagnostics()
                .iter()
                .map(Diagnostic::code)
                .collect(),
        }
    }

    fn edge_key(edge: CanonicalLaneEdgeView<'_>) -> String {
        edge.identity_fields()
            .find(|field| field.tag() == FieldTag::LaneEdgeKey)
            .map(|field| String::from_utf8(field.value_bytes().to_vec()).unwrap())
            .unwrap()
    }

    #[test]
    fn compiler_atomically_returns_lir_source_map_and_success_diagnostics() {
        let successors = [LaneEdgeReference::imported("city/base", "edge-b")];
        let input = unit([
            module(
                "city/app",
                "app.document",
                &["city/base"],
                &[("edge-a", 10.0, &successors)],
            ),
            module("city/base", "base.document", &[], &[("edge-b", 20.0, &[])]),
        ]);
        let output = Compiler::new().compile(input).unwrap();

        assert!(output.diagnostics().is_empty());
        let edges = output.lir().lane_edges().collect::<Vec<_>>();
        assert_eq!(edges.len(), 2);
        assert_eq!(edge_key(edges[0]), "edge-a");
        assert_eq!(edges[0].ordinal().raw(), 0);
        assert_eq!(edges[0].successors(), [LaneEdgeOrdinal::from_raw(1)]);
        assert_eq!(edges[0].length_meters(), 10.0);
        assert_eq!(edges[0].speed_limit_meters_per_second(), 13.75);
        assert_eq!(
            output
                .lir()
                .lane_edge(edges[1].ordinal())
                .unwrap()
                .stable_id(),
            edges[1].stable_id()
        );

        let modules = output
            .source_map_input()
            .source_modules()
            .map(SourceModuleDescriptor::authoring_namespace_id)
            .collect::<Vec<_>>();
        assert_eq!(modules, ["city/base", "city/app"]);
        let documents = output
            .source_map_input()
            .source_documents()
            .map(|document| document.source_document_key())
            .collect::<Vec<_>>();
        assert_eq!(documents, ["base.document", "app.document"]);

        let entity_sources = output
            .source_map_input()
            .lane_edge_sources()
            .collect::<Vec<_>>();
        assert_eq!(entity_sources.len(), 2);
        for (edge, source) in edges.iter().zip(entity_sources) {
            assert_eq!(source.ordinal(), edge.ordinal());
            assert_eq!(source.stable_id(), edge.stable_id());
            assert!(source.contributing_sources().next().is_none());
        }
        assert_eq!(
            output
                .source_map_input()
                .lane_edge_successor_sources()
                .map(|source| (
                    source.owner_ordinal().raw(),
                    source.role(),
                    source.local_index(),
                    source.primary_source().source_document_key().to_owned(),
                ))
                .collect::<Vec<_>>(),
            [(
                0,
                SourceRelationRole::LaneEdgeSuccessor,
                0,
                "app.document".to_owned(),
            )]
        );
    }

    #[test]
    fn compiler_freezes_complete_cross_section_owner_tree_and_source_relations() {
        let output = Compiler::new()
            .compile(unit([cross_section_module(false)]))
            .unwrap();
        let lir = output.lir();
        let corridor = lir.road_corridors().next().unwrap();
        let section = lir.road_sections().next().unwrap();
        let lane = lir.authoring_lanes().next().unwrap();
        let group = lir.lane_groups().next().unwrap();
        let band = lir.facility_bands().next().unwrap();

        assert_eq!(corridor.reference_section(), section.ordinal());
        assert_eq!(
            corridor.elements().collect::<Vec<_>>(),
            [
                CanonicalCorridorElement::FacilityBand(band.ordinal()),
                CanonicalCorridorElement::RoadSection(section.ordinal()),
            ]
        );
        assert_eq!(section.road_corridor(), corridor.ordinal());
        assert_eq!(section.kind_id(), "motorLane");
        assert_eq!(section.lanes(), [lane.ordinal()]);
        assert_eq!(lane.road_section(), section.ordinal());
        assert_eq!(lane.edge_chain().len(), 2);
        assert_eq!(lane.lane_group(), Some(group.ordinal()));
        assert_eq!(group.road_section(), section.ordinal());
        assert_eq!(group.members(), [lane.ordinal()]);
        assert_eq!(band.road_corridor(), corridor.ordinal());
        assert_eq!(band.kind_id(), "sidewalk");

        let section_fields = section
            .identity_fields()
            .map(|field| (field.tag(), field.value_bytes().to_vec()))
            .collect::<Vec<_>>();
        assert_eq!(
            section_fields
                .iter()
                .map(|field| field.0)
                .collect::<Vec<_>>(),
            [
                FieldTag::AuthoringNamespaceId,
                FieldTag::SectionKey,
                FieldTag::RoadCorridorStableId,
            ]
        );
        assert_eq!(
            section_fields[2].1,
            corridor.stable_id().as_untyped().as_bytes()
        );

        let source_map = output.source_map_input();
        assert_eq!(source_map.road_corridor_sources().len(), 1);
        assert_eq!(source_map.road_section_sources().len(), 1);
        assert_eq!(source_map.authoring_lane_sources().len(), 1);
        assert_eq!(source_map.lane_group_sources().len(), 1);
        assert_eq!(source_map.facility_band_sources().len(), 1);
        assert_eq!(
            source_map
                .cross_section_relation_sources()
                .map(|source| {
                    (
                        source.owner().entity_kind(),
                        source.role(),
                        source.local_index(),
                    )
                })
                .collect::<Vec<_>>(),
            [
                (
                    laneflow_static_contract::EntityKind::RoadCorridor,
                    SourceRelationRole::RoadCorridorElement,
                    0,
                ),
                (
                    laneflow_static_contract::EntityKind::RoadCorridor,
                    SourceRelationRole::RoadCorridorElement,
                    1,
                ),
                (
                    laneflow_static_contract::EntityKind::RoadSection,
                    SourceRelationRole::RoadSectionLane,
                    0,
                ),
                (
                    laneflow_static_contract::EntityKind::AuthoringLane,
                    SourceRelationRole::AuthoringLaneEdge,
                    0,
                ),
                (
                    laneflow_static_contract::EntityKind::AuthoringLane,
                    SourceRelationRole::AuthoringLaneEdge,
                    1,
                ),
                (
                    laneflow_static_contract::EntityKind::LaneGroup,
                    SourceRelationRole::LaneGroupMember,
                    0,
                ),
            ]
        );
    }

    #[test]
    fn cross_section_lir_semantics_ignore_top_level_declaration_order() {
        let baseline = Compiler::new()
            .compile(unit([cross_section_module(false)]))
            .unwrap();
        let permuted = Compiler::new()
            .compile(unit([cross_section_module(true)]))
            .unwrap();

        assert_eq!(
            baseline.lir.inner.semantic_digest,
            permuted.lir.inner.semantic_digest
        );
        assert_eq!(
            baseline
                .lir()
                .road_corridors()
                .map(|corridor| corridor.stable_id())
                .collect::<Vec<_>>(),
            permuted
                .lir()
                .road_corridors()
                .map(|corridor| corridor.stable_id())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            baseline
                .lir()
                .authoring_lanes()
                .map(|lane| lane.stable_id())
                .collect::<Vec<_>>(),
            permuted
                .lir()
                .authoring_lanes()
                .map(|lane| lane.stable_id())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn compiler_freezes_complete_junction_topology_and_source_relations() {
        let output = Compiler::new()
            .compile(unit([junction_module(false, "internal-a")]))
            .unwrap();
        let lir = output.lir();
        let junction = lir.junctions().next().unwrap();
        let movement = lir.movements().next().unwrap();
        let paths = lir.maneuver_paths().collect::<Vec<_>>();
        let edges = lir
            .lane_edges()
            .map(|edge| (edge_key(edge), edge.ordinal()))
            .collect::<std::collections::HashMap<_, _>>();

        assert_eq!(junction.movements(), [movement.ordinal()]);
        assert_eq!(movement.junction(), junction.ordinal());
        assert_eq!(movement.directed_entry_approach_key(), "approach-westbound");
        assert_eq!(movement.directed_exit_approach_key(), "approach-eastbound");
        assert_eq!(movement.maneuver_paths().len(), 2);
        assert_eq!(paths.len(), 2);
        assert_eq!(
            stable_key(paths[0].identity_fields(), FieldTag::PathKey),
            "path-a"
        );
        assert_eq!(
            paths[0].edges(),
            [edges["entry-a"], edges["internal-a"], edges["exit"]]
        );
        assert_eq!(paths[0].entry_edge(), edges["entry-a"]);
        assert_eq!(paths[0].internal_edges(), [edges["internal-a"]]);
        assert_eq!(paths[0].exit_edge(), edges["exit"]);
        assert_eq!(
            lir.junction_internal_owner(edges["internal-a"]),
            Some(junction.ordinal())
        );
        assert_eq!(lir.junction_internal_owner(edges["entry-a"]), None);
        assert_eq!(
            lir.junction_internal_edges()
                .map(|relation| (relation.edge(), relation.junction()))
                .collect::<Vec<_>>(),
            [(edges["internal-a"], junction.ordinal())]
        );
        assert_eq!(
            movement
                .identity_fields()
                .map(|field| field.tag())
                .collect::<Vec<_>>(),
            [
                FieldTag::AuthoringNamespaceId,
                FieldTag::MovementKey,
                FieldTag::DirectedEntryApproachKey,
                FieldTag::DirectedExitApproachKey,
                FieldTag::JunctionStableId,
            ]
        );
        assert_eq!(
            paths[0]
                .identity_fields()
                .map(|field| field.tag())
                .collect::<Vec<_>>(),
            [
                FieldTag::AuthoringNamespaceId,
                FieldTag::PathKey,
                FieldTag::MovementStableId,
                FieldTag::EntryEdgeStableId,
                FieldTag::ExitEdgeStableId,
            ]
        );

        let source_map = output.source_map_input();
        assert_eq!(source_map.junction_sources().len(), 1);
        assert_eq!(source_map.movement_sources().len(), 1);
        assert_eq!(source_map.maneuver_path_sources().len(), 2);
        assert_eq!(
            source_map
                .junction_relation_sources()
                .map(|source| (
                    source.owner().entity_kind(),
                    source.role(),
                    source.local_index()
                ))
                .collect::<Vec<_>>(),
            [
                (
                    laneflow_static_contract::EntityKind::Junction,
                    SourceRelationRole::JunctionMovement,
                    0,
                ),
                (
                    laneflow_static_contract::EntityKind::Movement,
                    SourceRelationRole::MovementManeuverPath,
                    0,
                ),
                (
                    laneflow_static_contract::EntityKind::Movement,
                    SourceRelationRole::MovementManeuverPath,
                    1,
                ),
                (
                    laneflow_static_contract::EntityKind::ManeuverPath,
                    SourceRelationRole::ManeuverPathEdge,
                    0,
                ),
                (
                    laneflow_static_contract::EntityKind::ManeuverPath,
                    SourceRelationRole::ManeuverPathEdge,
                    1,
                ),
                (
                    laneflow_static_contract::EntityKind::ManeuverPath,
                    SourceRelationRole::ManeuverPathEdge,
                    2,
                ),
                (
                    laneflow_static_contract::EntityKind::ManeuverPath,
                    SourceRelationRole::ManeuverPathEdge,
                    0,
                ),
                (
                    laneflow_static_contract::EntityKind::ManeuverPath,
                    SourceRelationRole::ManeuverPathEdge,
                    1,
                ),
                (
                    laneflow_static_contract::EntityKind::ManeuverPath,
                    SourceRelationRole::ManeuverPathEdge,
                    2,
                ),
                (
                    laneflow_static_contract::EntityKind::Junction,
                    SourceRelationRole::JunctionInternalEdge,
                    0,
                ),
            ]
        );
    }

    #[test]
    fn compiler_accepts_a_direct_maneuver_path_without_internal_edges() {
        let mut builder = junction_builder("direct-path.document");
        builder
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "entry",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[LaneEdgeReference::local("exit")],
            })
            .unwrap()
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "exit",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .unwrap()
            .add_junction(JunctionInput {
                junction_key: "junction-main",
            })
            .unwrap()
            .add_movement(MovementInput {
                movement_key: "movement-main",
                junction: JunctionReference::local("junction-main"),
                directed_entry_approach_key: "approach-entry",
                directed_exit_approach_key: "approach-exit",
            })
            .unwrap()
            .add_maneuver_path(ManeuverPathInput {
                maneuver_path_key: "path-direct",
                movement: MovementReference::local("movement-main"),
                entry_edge: LaneEdgeReference::local("entry"),
                internal_edges: &[],
                exit_edge: LaneEdgeReference::local("exit"),
            })
            .unwrap();

        let output = Compiler::new()
            .compile(unit([builder.finish().unwrap()]))
            .unwrap();
        let path = output.lir().maneuver_paths().next().unwrap();
        assert_eq!(path.edges().len(), 2);
        assert!(path.internal_edges().is_empty());
        assert_eq!(output.lir().junction_internal_edges().len(), 0);
    }

    #[test]
    fn junction_lir_is_deterministic_and_path_identity_excludes_internal_edges() {
        let baseline = Compiler::new()
            .compile(unit([junction_module(false, "internal-a")]))
            .unwrap();
        let permuted = Compiler::new()
            .compile(unit([junction_module(true, "internal-a")]))
            .unwrap();
        let different_internal = Compiler::new()
            .compile(unit([junction_module(false, "internal-b")]))
            .unwrap();

        assert_eq!(
            baseline.lir.inner.semantic_digest,
            permuted.lir.inner.semantic_digest
        );
        assert_eq!(
            baseline
                .lir()
                .maneuver_paths()
                .map(|path| path.stable_id())
                .collect::<Vec<_>>(),
            permuted
                .lir()
                .maneuver_paths()
                .map(|path| path.stable_id())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            baseline
                .lir()
                .maneuver_paths()
                .map(|path| path.stable_id())
                .collect::<Vec<_>>(),
            different_internal
                .lir()
                .maneuver_paths()
                .map(|path| path.stable_id())
                .collect::<Vec<_>>()
        );
        assert_ne!(
            baseline.lir.inner.semantic_digest,
            different_internal.lir.inner.semantic_digest
        );
    }

    #[test]
    fn compiler_rejects_junction_topology_semantic_failures_before_lir() {
        let add_junction = |builder: &mut SyntheticModuleBuilder, key: &'static str| {
            builder
                .add_junction(JunctionInput { junction_key: key })
                .unwrap();
        };
        let add_movement =
            |builder: &mut SyntheticModuleBuilder, key: &'static str, junction: &'static str| {
                builder
                    .add_movement(MovementInput {
                        movement_key: key,
                        junction: JunctionReference::local(junction),
                        directed_entry_approach_key: "approach-entry",
                        directed_exit_approach_key: "approach-exit",
                    })
                    .unwrap();
            };
        let add_edge = |builder: &mut SyntheticModuleBuilder,
                        key: &'static str,
                        successors: &[LaneEdgeReference<'static>]| {
            builder
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: key,
                    length_meters: 10.0,
                    speed_limit_meters_per_second: 10.0,
                    successors,
                })
                .unwrap();
        };

        let mut empty_junction = junction_builder("empty-junction.document");
        add_junction(&mut empty_junction, "junction-empty");
        assert!(compile_diagnostic_codes(empty_junction).contains(&DiagnosticCode::EmptyJunction));

        let mut empty_movement = junction_builder("empty-movement.document");
        add_junction(&mut empty_movement, "junction-main");
        add_movement(&mut empty_movement, "movement-empty", "junction-main");
        assert!(compile_diagnostic_codes(empty_movement).contains(&DiagnosticCode::EmptyMovement));

        let mut disconnected = junction_builder("disconnected-path.document");
        add_edge(&mut disconnected, "entry", &[]);
        add_edge(&mut disconnected, "exit", &[]);
        add_junction(&mut disconnected, "junction-main");
        add_movement(&mut disconnected, "movement-main", "junction-main");
        disconnected
            .add_maneuver_path(ManeuverPathInput {
                maneuver_path_key: "path-main",
                movement: MovementReference::local("movement-main"),
                entry_edge: LaneEdgeReference::local("entry"),
                internal_edges: &[],
                exit_edge: LaneEdgeReference::local("exit"),
            })
            .unwrap();
        assert!(
            compile_diagnostic_codes(disconnected)
                .contains(&DiagnosticCode::DisconnectedManeuverPath)
        );

        let mut duplicate = junction_builder("duplicate-path.document");
        add_edge(&mut duplicate, "entry", &[LaneEdgeReference::local("exit")]);
        add_edge(&mut duplicate, "exit", &[]);
        add_junction(&mut duplicate, "junction-main");
        add_movement(&mut duplicate, "movement-main", "junction-main");
        for path_key in ["path-a", "path-b"] {
            duplicate
                .add_maneuver_path(ManeuverPathInput {
                    maneuver_path_key: path_key,
                    movement: MovementReference::local("movement-main"),
                    entry_edge: LaneEdgeReference::local("entry"),
                    internal_edges: &[],
                    exit_edge: LaneEdgeReference::local("exit"),
                })
                .unwrap();
        }
        assert!(
            compile_diagnostic_codes(duplicate)
                .contains(&DiagnosticCode::DuplicateManeuverPathSequence)
        );

        let mut cross_junction = junction_builder("cross-junction-internal.document");
        add_edge(
            &mut cross_junction,
            "entry-a",
            &[LaneEdgeReference::local("internal")],
        );
        add_edge(
            &mut cross_junction,
            "entry-b",
            &[LaneEdgeReference::local("internal")],
        );
        add_edge(
            &mut cross_junction,
            "internal",
            &[
                LaneEdgeReference::local("exit-a"),
                LaneEdgeReference::local("exit-b"),
            ],
        );
        add_edge(&mut cross_junction, "exit-a", &[]);
        add_edge(&mut cross_junction, "exit-b", &[]);
        for suffix in ["a", "b"] {
            let junction_key = if suffix == "a" {
                "junction-a"
            } else {
                "junction-b"
            };
            let movement_key = if suffix == "a" {
                "movement-a"
            } else {
                "movement-b"
            };
            add_junction(&mut cross_junction, junction_key);
            add_movement(&mut cross_junction, movement_key, junction_key);
            let internal = [LaneEdgeReference::local("internal")];
            cross_junction
                .add_maneuver_path(ManeuverPathInput {
                    maneuver_path_key: if suffix == "a" { "path-a" } else { "path-b" },
                    movement: MovementReference::local(movement_key),
                    entry_edge: LaneEdgeReference::local(if suffix == "a" {
                        "entry-a"
                    } else {
                        "entry-b"
                    }),
                    internal_edges: &internal,
                    exit_edge: LaneEdgeReference::local(if suffix == "a" {
                        "exit-a"
                    } else {
                        "exit-b"
                    }),
                })
                .unwrap();
        }
        assert!(
            compile_diagnostic_codes(cross_junction)
                .contains(&DiagnosticCode::InternalEdgeJunctionConflict)
        );

        let mut boundary_conflict = junction_builder("internal-boundary-conflict.document");
        add_edge(
            &mut boundary_conflict,
            "entry",
            &[LaneEdgeReference::local("internal")],
        );
        add_edge(
            &mut boundary_conflict,
            "internal",
            &[
                LaneEdgeReference::local("exit-a"),
                LaneEdgeReference::local("exit-b"),
            ],
        );
        add_edge(&mut boundary_conflict, "exit-a", &[]);
        add_edge(&mut boundary_conflict, "exit-b", &[]);
        add_junction(&mut boundary_conflict, "junction-main");
        add_movement(&mut boundary_conflict, "movement-main", "junction-main");
        let internal = [LaneEdgeReference::local("internal")];
        boundary_conflict
            .add_maneuver_path(ManeuverPathInput {
                maneuver_path_key: "path-with-internal",
                movement: MovementReference::local("movement-main"),
                entry_edge: LaneEdgeReference::local("entry"),
                internal_edges: &internal,
                exit_edge: LaneEdgeReference::local("exit-a"),
            })
            .unwrap()
            .add_maneuver_path(ManeuverPathInput {
                maneuver_path_key: "path-with-boundary",
                movement: MovementReference::local("movement-main"),
                entry_edge: LaneEdgeReference::local("internal"),
                internal_edges: &[],
                exit_edge: LaneEdgeReference::local("exit-b"),
            })
            .unwrap();
        assert!(
            compile_diagnostic_codes(boundary_conflict)
                .contains(&DiagnosticCode::InternalBoundaryRoleConflict)
        );
    }

    #[test]
    fn movement_approach_identity_fields_reject_non_ascii_input_atomically() {
        let mut builder = junction_builder("invalid-approach.document");
        builder
            .add_junction(JunctionInput {
                junction_key: "junction-main",
            })
            .unwrap();
        let diagnostic = match builder.add_movement(MovementInput {
            movement_key: "movement-main",
            junction: JunctionReference::local("junction-main"),
            directed_entry_approach_key: "入口",
            directed_exit_approach_key: "approach-exit",
        }) {
            Ok(_) => panic!("non-ASCII identity field must reject the declaration"),
            Err(diagnostic) => diagnostic,
        };
        assert_eq!(
            diagnostic.diagnostics()[0].code(),
            DiagnosticCode::InvalidIdentityAsciiField
        );
        // 同一个稳定键仍可被合法声明，证明失败路径没有预占符号或部分提交资源计数。
        builder
            .add_movement(MovementInput {
                movement_key: "movement-main",
                junction: JunctionReference::local("junction-main"),
                directed_entry_approach_key: "approach-entry",
                directed_exit_approach_key: "approach-exit",
            })
            .unwrap();
    }

    #[test]
    fn compiler_rejects_cross_section_semantic_failures_before_lir() {
        let limits = CompileLimits::p100_initial_v1();
        let make_builder = || {
            let header = SourceModuleHeader::new(
                SourceModuleHeaderInput {
                    authoring_namespace_id: "city/failure",
                    source_document_key: "failure.document",
                    generator_build_id: "git:0123456789abcdef",
                    parameters_and_inputs_digest: [0x11; 32],
                    frontend_options_digest: [0x22; 32],
                    random_seed: None,
                    provenance: "repository:laneflow",
                },
                &limits,
            )
            .unwrap();
            SyntheticModuleBuilder::new(header, &limits).unwrap()
        };

        let mut missing_owner = make_builder();
        missing_owner
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge-a",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .unwrap()
            .add_road_section(RoadSectionInput {
                road_section_key: "section-a",
                kind_id: "motorLane",
                lanes: &[AuthoringLaneInput {
                    authoring_lane_key: "lane-a",
                    edge_chain: &[LaneEdgeReference::local("edge-a")],
                    lane_group: None,
                }],
            })
            .unwrap();
        let diagnostics = match Compiler::new().compile(unit([missing_owner.finish().unwrap()])) {
            Ok(_) => panic!("missing owner must reject compilation"),
            Err(diagnostics) => diagnostics,
        };
        assert_eq!(
            diagnostics.diagnostics()[0].code(),
            DiagnosticCode::MissingCrossSectionOwner
        );

        let mut disconnected = make_builder();
        disconnected
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge-a",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .unwrap()
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge-b",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .unwrap()
            .add_road_section(RoadSectionInput {
                road_section_key: "section-a",
                kind_id: "motorLane",
                lanes: &[AuthoringLaneInput {
                    authoring_lane_key: "lane-a",
                    edge_chain: &[
                        LaneEdgeReference::local("edge-a"),
                        LaneEdgeReference::local("edge-b"),
                    ],
                    lane_group: None,
                }],
            })
            .unwrap()
            .add_road_corridor(RoadCorridorInput {
                road_corridor_key: "corridor-a",
                reference_section: RoadSectionReference::local("section-a"),
                elements: &[CorridorElementReference::road_section(
                    RoadSectionReference::local("section-a"),
                )],
            })
            .unwrap();
        let diagnostics = match Compiler::new().compile(unit([disconnected.finish().unwrap()])) {
            Ok(_) => panic!("disconnected lane chain must reject compilation"),
            Err(diagnostics) => diagnostics,
        };
        assert!(diagnostics.diagnostics().iter().any(|diagnostic| {
            diagnostic.code() == DiagnosticCode::DisconnectedAuthoringLaneEdgeChain
        }));

        let mut unknown_middle = make_builder();
        unknown_middle
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge-a",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .unwrap()
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge-c",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .unwrap()
            .add_road_section(RoadSectionInput {
                road_section_key: "section-a",
                kind_id: "motorLane",
                lanes: &[AuthoringLaneInput {
                    authoring_lane_key: "lane-a",
                    edge_chain: &[
                        LaneEdgeReference::local("edge-a"),
                        LaneEdgeReference::local("missing"),
                        LaneEdgeReference::local("edge-c"),
                    ],
                    lane_group: None,
                }],
            })
            .unwrap()
            .add_road_corridor(RoadCorridorInput {
                road_corridor_key: "corridor-a",
                reference_section: RoadSectionReference::local("section-a"),
                elements: &[CorridorElementReference::road_section(
                    RoadSectionReference::local("section-a"),
                )],
            })
            .unwrap();
        let diagnostics = match Compiler::new().compile(unit([unknown_middle.finish().unwrap()])) {
            Ok(_) => panic!("unknown lane edge must reject compilation"),
            Err(diagnostics) => diagnostics,
        };
        assert!(
            diagnostics
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code() == DiagnosticCode::UnknownReferenceTarget)
        );
        assert!(diagnostics.diagnostics().iter().all(|diagnostic| {
            diagnostic.code() != DiagnosticCode::DisconnectedAuthoringLaneEdgeChain
        }));

        let mut multiple_owner = make_builder();
        multiple_owner
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge-a",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .unwrap()
            .add_road_section(RoadSectionInput {
                road_section_key: "section-a",
                kind_id: "motorLane",
                lanes: &[AuthoringLaneInput {
                    authoring_lane_key: "lane-a",
                    edge_chain: &[LaneEdgeReference::local("edge-a")],
                    lane_group: None,
                }],
            })
            .unwrap()
            .add_road_corridor(RoadCorridorInput {
                road_corridor_key: "corridor-a",
                reference_section: RoadSectionReference::local("section-a"),
                elements: &[CorridorElementReference::road_section(
                    RoadSectionReference::local("section-a"),
                )],
            })
            .unwrap()
            .add_road_corridor(RoadCorridorInput {
                road_corridor_key: "corridor-b",
                reference_section: RoadSectionReference::local("section-a"),
                elements: &[CorridorElementReference::road_section(
                    RoadSectionReference::local("section-a"),
                )],
            })
            .unwrap();
        let diagnostics = match Compiler::new().compile(unit([multiple_owner.finish().unwrap()])) {
            Ok(_) => panic!("multiple cross-section owners must reject compilation"),
            Err(diagnostics) => diagnostics,
        };
        assert!(
            diagnostics.diagnostics().iter().any(|diagnostic| {
                diagnostic.code() == DiagnosticCode::MultipleCrossSectionOwners
            })
        );

        let mut group_parent_mismatch = make_builder();
        group_parent_mismatch
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge-a",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .unwrap()
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge-b",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .unwrap()
            .add_lane_group(LaneGroupInput {
                lane_group_key: "group-a",
                road_section: RoadSectionReference::local("section-a"),
            })
            .unwrap()
            .add_road_section(RoadSectionInput {
                road_section_key: "section-a",
                kind_id: "motorLane",
                lanes: &[AuthoringLaneInput {
                    authoring_lane_key: "lane-a",
                    edge_chain: &[LaneEdgeReference::local("edge-a")],
                    lane_group: None,
                }],
            })
            .unwrap()
            .add_road_section(RoadSectionInput {
                road_section_key: "section-b",
                kind_id: "motorLane",
                lanes: &[AuthoringLaneInput {
                    authoring_lane_key: "lane-b",
                    edge_chain: &[LaneEdgeReference::local("edge-b")],
                    lane_group: Some(LaneGroupReference::local("group-a")),
                }],
            })
            .unwrap()
            .add_road_corridor(RoadCorridorInput {
                road_corridor_key: "corridor-a",
                reference_section: RoadSectionReference::local("section-a"),
                elements: &[
                    CorridorElementReference::road_section(RoadSectionReference::local(
                        "section-a",
                    )),
                    CorridorElementReference::road_section(RoadSectionReference::local(
                        "section-b",
                    )),
                ],
            })
            .unwrap();
        let diagnostics =
            match Compiler::new().compile(unit([group_parent_mismatch.finish().unwrap()])) {
                Ok(_) => panic!("lane-group parent mismatch must reject compilation"),
                Err(diagnostics) => diagnostics,
            };
        assert!(
            diagnostics
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code() == DiagnosticCode::LaneGroupParentMismatch)
        );
    }

    #[test]
    fn source_changes_do_not_change_lir_semantic_digest() {
        let left = unit([module(
            "city/a",
            "left.document",
            &[],
            &[("edge-a", 10.0, &[])],
        )]);
        let right = unit([module(
            "city/a",
            "right.document",
            &[],
            &[("edge-a", 10.0, &[])],
        )]);
        let mut compiler = Compiler::new();
        let left = compiler.compile(left).unwrap();
        let right = compiler.compile(right).unwrap();

        assert_eq!(
            left.lir.inner.semantic_digest,
            right.lir.inner.semantic_digest
        );
        assert_ne!(
            left.source_map_input()
                .lane_edge_sources()
                .next()
                .unwrap()
                .primary_source()
                .source_document_key(),
            right
                .source_map_input()
                .lane_edge_sources()
                .next()
                .unwrap()
                .primary_source()
                .source_document_key()
        );
    }

    #[test]
    fn thirty_two_failures_do_not_pollute_reused_compiler() {
        let missing = [LaneEdgeReference::local("missing")];
        let mut compiler = Compiler::new();
        for index in 0..32 {
            let failed = unit([module(
                &format!("failed/{index}"),
                &format!("failed-{index}.document"),
                &[],
                &[("edge-a", 10.0, &missing)],
            )]);
            let diagnostics = match compiler.compile(failed) {
                Ok(_) => panic!("expected failed compilation"),
                Err(diagnostics) => diagnostics,
            };
            assert!(matches!(
                diagnostics.diagnostics()[0].payload(),
                DiagnosticPayload::UnknownReferenceTarget { .. }
            ));
        }

        let recovered = unit([module(
            "city/a",
            "city-a.document",
            &[],
            &[("edge-a", 10.0, &[])],
        )]);
        let fresh = unit([module(
            "city/a",
            "city-a.document",
            &[],
            &[("edge-a", 10.0, &[])],
        )]);
        assert_eq!(
            compiler
                .compile(recovered)
                .unwrap()
                .lir
                .inner
                .semantic_digest,
            Compiler::new()
                .compile(fresh)
                .unwrap()
                .lir
                .inner
                .semantic_digest
        );
    }

    #[test]
    fn source_map_output_limit_fails_after_lir_without_exposing_partial_output() {
        let probe = unit([module(
            "city/a",
            "city-a.document",
            &[],
            &[("edge-a", 10.0, &[])],
        )]);
        let hir = build_hir(&probe).unwrap();
        let mir = lower_to_mir(&probe, &hir).unwrap();
        let lir_output_bytes = freeze_lir(&probe, &mir).unwrap().lir.output_bytes;

        let mut constrained = unit([module(
            "city/a",
            "city-a.document",
            &[],
            &[("edge-a", 10.0, &[])],
        )]);
        constrained.limits = CompileLimits::p100_initial_v1().with_test_lir_limits(
            u32::MAX,
            u32::MAX,
            u32::try_from(lir_output_bytes).unwrap(),
            u32::MAX,
        );
        let mut compiler = Compiler::new();
        let diagnostics = match compiler.compile(constrained) {
            Ok(_) => panic!("expected source-map output limit failure"),
            Err(diagnostics) => diagnostics,
        };
        assert!(diagnostics.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic.payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::OutputBytes,
                limit,
                observed,
            } if *limit == lir_output_bytes && observed > limit
        )));

        let recovered = unit([module(
            "city/recovered",
            "recovered.document",
            &[],
            &[("edge-a", 10.0, &[])],
        )]);
        assert!(compiler.compile(recovered).is_ok());
    }
}

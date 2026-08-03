//! 与 Canonical LIR 同次成功编译冻结的来源伴随数据。
//!
//! 本模块保存来源模块描述符、唯一来源文档登记，以及 LIR 稳定实体和 owner-local 关系
//! 到来源位置的关联。它只描述“这项已冻结语义来自哪里”，无权补充 LIR 中不存在的
//! 默认值、身份字段、所有者或连接。后继 #298 的源映射发射器必须同时借用本类型和同一
//! [`crate::CompilationOutput`] 中的 [`crate::ValidatedCanonicalLir`]。

use laneflow_static_contract::{
    AuthoringLaneId, AuthoringLaneOrdinal, EntityKind, FacilityBandId, FacilityBandOrdinal,
    LaneEdgeId, LaneEdgeOrdinal, LaneGroupId, LaneGroupOrdinal, RoadCorridorId,
    RoadCorridorOrdinal, RoadSectionId, RoadSectionOrdinal,
};

use crate::diagnostic::DiagnosticCollector;
use crate::lir::LirFreezeOutput;
use crate::mir::MirUnit;
use crate::module::SourceDocumentOrdinal;
use crate::{
    CompilationUnit, CompileLimitDimension, Diagnostic, DiagnosticBundle, SourceModuleDescriptor,
    SourcePosition, SourceSpan,
};

const LANE_EDGE_SOURCE_LOGICAL_BYTES: u64 = 4 + 16 + 4 + 16 + 4;
const LANE_EDGE_SUCCESSOR_SOURCE_LOGICAL_BYTES: u64 = 16 + 4 + 2 + 4 + 4 + 16 + 4;
const STABLE_ENTITY_SOURCE_LOGICAL_BYTES: u64 = LANE_EDGE_SOURCE_LOGICAL_BYTES;
const CROSS_SECTION_RELATION_SOURCE_LOGICAL_BYTES: u64 = 2 + 4 + 16 + 2 + 4 + 4 + 16 + 4;

/// owner-local 来源记录中登记的有类型语义角色。
///
/// 数值只在当前编译结果内区分来源记录类别，不是后继源映射线格式代码。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u16)]
#[non_exhaustive]
pub enum SourceRelationRole {
    /// `LaneEdge` 声明中的一项下游连接。
    LaneEdgeSuccessor = 1,
    /// 道路走廊有序横断面中的一项成员。
    RoadCorridorElement = 2,
    /// 道路区段有序车道集合中的一项成员。
    RoadSectionLane = 3,
    /// 编制车道有序覆盖链中的一项车道图边。
    AuthoringLaneEdge = 4,
    /// 车道组中的一项编制车道成员。
    LaneGroupMember = 5,
}

#[derive(Clone, Copy)]
struct SourceLocationRecord {
    source_document_ordinal: SourceDocumentOrdinal,
    start: SourcePosition,
    end: SourcePosition,
}

struct LaneEdgeSourceRecord {
    ordinal: LaneEdgeOrdinal,
    stable_id: LaneEdgeId,
    primary: SourceLocationRecord,
}

struct StableEntitySourceRecord<O, I> {
    ordinal: O,
    stable_id: I,
    primary: SourceLocationRecord,
}

struct CrossSectionRelationSourceRecord {
    owner: CrossSectionRelationOwnerRecord,
    role: SourceRelationRole,
    local_index: u32,
    primary: SourceLocationRecord,
}

#[derive(Clone, Copy)]
enum CrossSectionRelationOwnerRecord {
    RoadCorridor(RoadCorridorOrdinal, RoadCorridorId),
    RoadSection(RoadSectionOrdinal, RoadSectionId),
    AuthoringLane(AuthoringLaneOrdinal, AuthoringLaneId),
    LaneGroup(LaneGroupOrdinal, LaneGroupId),
}

struct LaneEdgeSuccessorSourceRecord {
    owner_ordinal: LaneEdgeOrdinal,
    owner_stable_id: LaneEdgeId,
    role: SourceRelationRole,
    local_index: u32,
    primary: SourceLocationRecord,
}

/// 与一个 Canonical LIR 原子配对的已验证源映射输入。
///
/// 本类型不能由调用方构造。来源文档与来源模块在当前官方前端中一一对应，并按编译单元
/// 的依赖优先规范顺序保存；`sourceDocumentKey` 已在编译单元构造时证明全局唯一。
pub struct ValidatedSourceMapInput {
    source_modules: Box<[SourceModuleDescriptor]>,
    lane_edge_sources: Box<[LaneEdgeSourceRecord]>,
    lane_edge_successor_sources: Box<[LaneEdgeSuccessorSourceRecord]>,
    road_corridor_sources: Box<[StableEntitySourceRecord<RoadCorridorOrdinal, RoadCorridorId>]>,
    road_section_sources: Box<[StableEntitySourceRecord<RoadSectionOrdinal, RoadSectionId>]>,
    authoring_lane_sources: Box<[StableEntitySourceRecord<AuthoringLaneOrdinal, AuthoringLaneId>]>,
    lane_group_sources: Box<[StableEntitySourceRecord<LaneGroupOrdinal, LaneGroupId>]>,
    facility_band_sources: Box<[StableEntitySourceRecord<FacilityBandOrdinal, FacilityBandId>]>,
    cross_section_relation_sources: Box<[CrossSectionRelationSourceRecord]>,
}

impl ValidatedSourceMapInput {
    /// 按依赖优先规范顺序遍历来源模块描述符。
    pub fn source_modules(&self) -> impl ExactSizeIterator<Item = &SourceModuleDescriptor> {
        self.source_modules.iter()
    }

    /// 遍历唯一来源文档登记；当前每个官方来源模块恰好登记一个文档。
    pub fn source_documents(&self) -> impl ExactSizeIterator<Item = SourceDocumentView<'_>> {
        self.source_modules
            .iter()
            .map(|descriptor| SourceDocumentView { descriptor })
    }

    /// 按 `LaneEdgeOrdinal` 递增顺序遍历稳定实体来源记录。
    pub fn lane_edge_sources(&self) -> impl ExactSizeIterator<Item = LaneEdgeSourceView<'_>> {
        self.lane_edge_sources
            .iter()
            .map(|record| LaneEdgeSourceView {
                source_map: self,
                record,
            })
    }

    /// 按 owner ordinal、角色和 local index 遍历下游连接来源记录。
    pub fn lane_edge_successor_sources(
        &self,
    ) -> impl ExactSizeIterator<Item = LaneEdgeSuccessorSourceView<'_>> {
        self.lane_edge_successor_sources
            .iter()
            .map(|record| LaneEdgeSuccessorSourceView {
                source_map: self,
                record,
            })
    }

    /// 按 `RoadCorridorOrdinal` 递增顺序遍历道路走廊来源记录。
    pub fn road_corridor_sources(
        &self,
    ) -> impl ExactSizeIterator<Item = RoadCorridorSourceView<'_>> {
        self.road_corridor_sources
            .iter()
            .map(|record| RoadCorridorSourceView {
                source_map: self,
                record,
            })
    }

    /// 按 `RoadSectionOrdinal` 递增顺序遍历道路区段来源记录。
    pub fn road_section_sources(&self) -> impl ExactSizeIterator<Item = RoadSectionSourceView<'_>> {
        self.road_section_sources
            .iter()
            .map(|record| RoadSectionSourceView {
                source_map: self,
                record,
            })
    }

    /// 按 `AuthoringLaneOrdinal` 递增顺序遍历编制车道来源记录。
    pub fn authoring_lane_sources(
        &self,
    ) -> impl ExactSizeIterator<Item = AuthoringLaneSourceView<'_>> {
        self.authoring_lane_sources
            .iter()
            .map(|record| AuthoringLaneSourceView {
                source_map: self,
                record,
            })
    }

    /// 按 `LaneGroupOrdinal` 递增顺序遍历车道组来源记录。
    pub fn lane_group_sources(&self) -> impl ExactSizeIterator<Item = LaneGroupSourceView<'_>> {
        self.lane_group_sources
            .iter()
            .map(|record| LaneGroupSourceView {
                source_map: self,
                record,
            })
    }

    /// 按 `FacilityBandOrdinal` 递增顺序遍历设施带来源记录。
    pub fn facility_band_sources(
        &self,
    ) -> impl ExactSizeIterator<Item = FacilityBandSourceView<'_>> {
        self.facility_band_sources
            .iter()
            .map(|record| FacilityBandSourceView {
                source_map: self,
                record,
            })
    }

    /// 按 owner kind、owner ordinal、角色和 local index 遍历横断面关系来源。
    pub fn cross_section_relation_sources(
        &self,
    ) -> impl ExactSizeIterator<Item = CrossSectionRelationSourceView<'_>> {
        self.cross_section_relation_sources
            .iter()
            .map(|record| CrossSectionRelationSourceView {
                source_map: self,
                record,
            })
    }

    fn location(&self, record: SourceLocationRecord) -> SourceLocationView<'_> {
        let descriptor = &self.source_modules[record.source_document_ordinal.index()];
        SourceLocationView {
            source_document_key: descriptor.source_document_key(),
            start: record.start,
            end: record.end,
        }
    }
}

/// 来源文档登记的一项只读视图。
#[derive(Clone, Copy)]
pub struct SourceDocumentView<'a> {
    descriptor: &'a SourceModuleDescriptor,
}

impl<'a> SourceDocumentView<'a> {
    /// 返回与机器路径无关、在本编译单元内唯一的文档键。
    #[must_use]
    pub fn source_document_key(&self) -> &'a str {
        self.descriptor.source_document_key()
    }

    /// 返回拥有该文档的来源模块 authoring namespace。
    #[must_use]
    pub fn authoring_namespace_id(&self) -> &'a str {
        self.descriptor.authoring_namespace_id()
    }
}

/// 已解析到来源文档登记的一项只读来源位置。
#[derive(Clone, Copy)]
pub struct SourceLocationView<'a> {
    source_document_key: &'a str,
    start: SourcePosition,
    end: SourcePosition,
}

impl<'a> SourceLocationView<'a> {
    /// 返回稳定来源文档键，而不是宿主文件系统路径。
    #[must_use]
    pub const fn source_document_key(&self) -> &'a str {
        self.source_document_key
    }

    /// 返回一基起始行列。
    #[must_use]
    pub const fn start(&self) -> SourcePosition {
        self.start
    }

    /// 返回一基结束行列。
    #[must_use]
    pub const fn end(&self) -> SourcePosition {
        self.end
    }
}

/// 一条 `LaneEdge` 稳定实体来源记录的只读视图。
#[derive(Clone, Copy)]
pub struct LaneEdgeSourceView<'a> {
    source_map: &'a ValidatedSourceMapInput,
    record: &'a LaneEdgeSourceRecord,
}

impl LaneEdgeSourceView<'_> {
    /// 返回本次 LIR 中定位实体的有类型序号。
    #[must_use]
    pub const fn ordinal(&self) -> LaneEdgeOrdinal {
        self.record.ordinal
    }

    /// 返回跨编译定位实体的稳定标识。
    #[must_use]
    pub const fn stable_id(&self) -> LaneEdgeId {
        self.record.stable_id
    }

    /// 返回拥有该声明的主要来源位置。
    #[must_use]
    pub fn primary_source(&self) -> SourceLocationView<'_> {
        self.source_map.location(self.record.primary)
    }

    /// 返回额外贡献来源位置；当前显式 `LaneEdge` 声明没有额外贡献项。
    pub fn contributing_sources(&self) -> impl ExactSizeIterator<Item = SourceLocationView<'_>> {
        core::iter::empty()
    }
}

macro_rules! stable_source_view {
    ($view:ident, $ordinal:ty, $id:ty) => {
        /// 一个已冻结稳定实体的主要来源位置借用视图。
        #[derive(Clone, Copy)]
        pub struct $view<'a> {
            source_map: &'a ValidatedSourceMapInput,
            record: &'a StableEntitySourceRecord<$ordinal, $id>,
        }

        impl $view<'_> {
            /// 返回本次 LIR 中定位实体的有类型序号。
            #[must_use]
            pub const fn ordinal(&self) -> $ordinal {
                self.record.ordinal
            }

            /// 返回跨编译定位实体的有类型稳定标识。
            #[must_use]
            pub const fn stable_id(&self) -> $id {
                self.record.stable_id
            }

            /// 返回拥有该声明的主要来源位置。
            #[must_use]
            pub fn primary_source(&self) -> SourceLocationView<'_> {
                self.source_map.location(self.record.primary)
            }

            /// 当前显式声明没有额外贡献来源。
            pub fn contributing_sources(
                &self,
            ) -> impl ExactSizeIterator<Item = SourceLocationView<'_>> {
                core::iter::empty()
            }
        }
    };
}

stable_source_view!(RoadCorridorSourceView, RoadCorridorOrdinal, RoadCorridorId);
stable_source_view!(RoadSectionSourceView, RoadSectionOrdinal, RoadSectionId);
stable_source_view!(
    AuthoringLaneSourceView,
    AuthoringLaneOrdinal,
    AuthoringLaneId
);
stable_source_view!(LaneGroupSourceView, LaneGroupOrdinal, LaneGroupId);
stable_source_view!(FacilityBandSourceView, FacilityBandOrdinal, FacilityBandId);

/// 一条横断面 owner-local 关系来源记录的只读视图。
#[derive(Clone, Copy)]
pub struct CrossSectionRelationSourceView<'a> {
    source_map: &'a ValidatedSourceMapInput,
    record: &'a CrossSectionRelationSourceRecord,
}

/// 横断面关系 owner 的有类型 LIR 序号与稳定标识。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CrossSectionRelationOwner {
    /// 道路走廊及其有类型身份。
    RoadCorridor(RoadCorridorOrdinal, RoadCorridorId),
    /// 道路区段及其有类型身份。
    RoadSection(RoadSectionOrdinal, RoadSectionId),
    /// 编制车道及其有类型身份。
    AuthoringLane(AuthoringLaneOrdinal, AuthoringLaneId),
    /// 车道组及其有类型身份。
    LaneGroup(LaneGroupOrdinal, LaneGroupId),
}

impl CrossSectionRelationOwner {
    /// 返回 owner 对应的 Identity v1 实体种类。
    #[must_use]
    pub const fn entity_kind(self) -> EntityKind {
        match self {
            Self::RoadCorridor(_, _) => EntityKind::RoadCorridor,
            Self::RoadSection(_, _) => EntityKind::RoadSection,
            Self::AuthoringLane(_, _) => EntityKind::AuthoringLane,
            Self::LaneGroup(_, _) => EntityKind::LaneGroup,
        }
    }
}

impl CrossSectionRelationSourceView<'_> {
    /// 返回关系 owner 的有类型 LIR 序号与稳定标识。
    #[must_use]
    pub const fn owner(&self) -> CrossSectionRelationOwner {
        match self.record.owner {
            CrossSectionRelationOwnerRecord::RoadCorridor(ordinal, stable_id) => {
                CrossSectionRelationOwner::RoadCorridor(ordinal, stable_id)
            }
            CrossSectionRelationOwnerRecord::RoadSection(ordinal, stable_id) => {
                CrossSectionRelationOwner::RoadSection(ordinal, stable_id)
            }
            CrossSectionRelationOwnerRecord::AuthoringLane(ordinal, stable_id) => {
                CrossSectionRelationOwner::AuthoringLane(ordinal, stable_id)
            }
            CrossSectionRelationOwnerRecord::LaneGroup(ordinal, stable_id) => {
                CrossSectionRelationOwner::LaneGroup(ordinal, stable_id)
            }
        }
    }

    /// 返回 owner-local 关系的有类型角色。
    #[must_use]
    pub const fn role(&self) -> SourceRelationRole {
        self.record.role
    }

    /// 返回同一 owner 与角色内的零基序号。
    #[must_use]
    pub const fn local_index(&self) -> u32 {
        self.record.local_index
    }

    /// 返回关系声明的主要来源位置。
    #[must_use]
    pub fn primary_source(&self) -> SourceLocationView<'_> {
        self.source_map.location(self.record.primary)
    }

    /// 当前显式关系没有额外贡献来源。
    pub fn contributing_sources(&self) -> impl ExactSizeIterator<Item = SourceLocationView<'_>> {
        core::iter::empty()
    }
}

/// 一条 owner-local 下游连接来源记录的只读视图。
#[derive(Clone, Copy)]
pub struct LaneEdgeSuccessorSourceView<'a> {
    source_map: &'a ValidatedSourceMapInput,
    record: &'a LaneEdgeSuccessorSourceRecord,
}

impl LaneEdgeSuccessorSourceView<'_> {
    /// 返回拥有该关系的稳定实体序号。
    #[must_use]
    pub const fn owner_ordinal(&self) -> LaneEdgeOrdinal {
        self.record.owner_ordinal
    }

    /// 返回拥有该关系的稳定实体标识。
    #[must_use]
    pub const fn owner_stable_id(&self) -> LaneEdgeId {
        self.record.owner_stable_id
    }

    /// 返回 owner-local 关系的有类型角色。
    #[must_use]
    pub const fn role(&self) -> SourceRelationRole {
        self.record.role
    }

    /// 返回本次编译中、同一 owner 与角色内的零基序号。
    #[must_use]
    pub const fn local_index(&self) -> u32 {
        self.record.local_index
    }

    /// 返回显式关系声明的主要来源位置。
    #[must_use]
    pub fn primary_source(&self) -> SourceLocationView<'_> {
        self.source_map.location(self.record.primary)
    }

    /// 返回生成关系的贡献来源；当前显式 successor 关系没有推导链。
    pub fn contributing_sources(&self) -> impl ExactSizeIterator<Item = SourceLocationView<'_>> {
        core::iter::empty()
    }
}

/// 在 AST/HIR/MIR 释放前冻结来源描述符及全部当前 LIR 键关联。
pub(crate) fn freeze_source_map(
    unit: CompilationUnit,
    mir: &MirUnit,
    frozen_lir: &LirFreezeOutput,
) -> Result<ValidatedSourceMapInput, DiagnosticBundle> {
    let module_count = u64::try_from(unit.modules.len()).unwrap_or(u64::MAX);
    let lane_edge_count = u64::try_from(mir.lane_edges.len()).unwrap_or(u64::MAX);
    let successor_count = u64::try_from(mir.lane_edge_connections.len()).unwrap_or(u64::MAX);
    let cross_entity_count = [
        mir.road_corridors.len(),
        mir.road_sections.len(),
        mir.authoring_lanes.len(),
        mir.lane_groups.len(),
        mir.facility_bands.len(),
    ]
    .into_iter()
    .fold(0_u64, |total, count| {
        total.saturating_add(u64::try_from(count).unwrap_or(u64::MAX))
    });
    let cross_relation_count = [
        mir.corridor_elements.len(),
        mir.authoring_lanes.len(),
        mir.authoring_lane_edges.len(),
        mir.lane_group_members.len(),
    ]
    .into_iter()
    .fold(0_u64, |total, count| {
        total.saturating_add(u64::try_from(count).unwrap_or(u64::MAX))
    });
    let source_map_logical_bytes = unit
        .modules
        .iter()
        .fold(0_u64, |total, module| {
            total.saturating_add(module.descriptor().source_map_logical_bytes())
        })
        .saturating_add(lane_edge_count.saturating_mul(LANE_EDGE_SOURCE_LOGICAL_BYTES))
        .saturating_add(successor_count.saturating_mul(LANE_EDGE_SUCCESSOR_SOURCE_LOGICAL_BYTES))
        .saturating_add(cross_entity_count.saturating_mul(STABLE_ENTITY_SOURCE_LOGICAL_BYTES))
        .saturating_add(
            cross_relation_count.saturating_mul(CROSS_SECTION_RELATION_SOURCE_LOGICAL_BYTES),
        );
    let output_bytes = frozen_lir
        .lir
        .output_bytes
        .saturating_add(source_map_logical_bytes);
    // 描述符字段与 import backing 已由 CompilationUnit 持有；冻结时只新增描述符、
    // 各稳定实体来源表及 owner-local 关系来源表的连续存储。峰值仍保留完整 unit，
    // 直到全部伴随表构造成功。
    let source_map_new_owned_bytes = requested_bytes::<SourceModuleDescriptor>(module_count)
        .saturating_add(requested_bytes::<LaneEdgeSourceRecord>(lane_edge_count))
        .saturating_add(requested_bytes::<LaneEdgeSuccessorSourceRecord>(
            successor_count,
        ))
        .saturating_add(requested_bytes::<
            StableEntitySourceRecord<RoadCorridorOrdinal, RoadCorridorId>,
        >(
            mir.road_corridors.len().try_into().unwrap_or(u64::MAX)
        ))
        .saturating_add(requested_bytes::<
            StableEntitySourceRecord<RoadSectionOrdinal, RoadSectionId>,
        >(
            mir.road_sections.len().try_into().unwrap_or(u64::MAX)
        ))
        .saturating_add(requested_bytes::<
            StableEntitySourceRecord<AuthoringLaneOrdinal, AuthoringLaneId>,
        >(
            mir.authoring_lanes.len().try_into().unwrap_or(u64::MAX)
        ))
        .saturating_add(requested_bytes::<
            StableEntitySourceRecord<LaneGroupOrdinal, LaneGroupId>,
        >(
            mir.lane_groups.len().try_into().unwrap_or(u64::MAX)
        ))
        .saturating_add(requested_bytes::<
            StableEntitySourceRecord<FacilityBandOrdinal, FacilityBandId>,
        >(
            mir.facility_bands.len().try_into().unwrap_or(u64::MAX)
        ))
        .saturating_add(requested_bytes::<CrossSectionRelationSourceRecord>(
            cross_relation_count,
        ));
    let controlled_live_bytes = unit
        .controlled_live_bytes
        .saturating_add(mir.controlled_live_bytes)
        .saturating_add(frozen_lir.lir.controlled_live_bytes)
        .saturating_add(frozen_lir.mapping_bytes())
        .saturating_add(source_map_new_owned_bytes);
    let primary_span = unit
        .modules
        .first()
        .map(|module| module.descriptor().declaration_span().clone());
    let stable_key = unit
        .modules
        .first()
        .map(|module| module.descriptor().authoring_namespace_id().into());
    let mut diagnostics =
        DiagnosticCollector::new(unit.limits.value(CompileLimitDimension::DiagnosticCount));
    for (dimension, observed) in [
        (CompileLimitDimension::OutputBytes, output_bytes),
        (
            CompileLimitDimension::CompilerControlledLiveBytes,
            controlled_live_bytes,
        ),
    ] {
        if observed > unit.limits.value(dimension) {
            diagnostics.push(Diagnostic::compile_limit_exceeded_at(
                dimension,
                unit.limits.value(dimension),
                observed,
                primary_span.clone(),
                stable_key.clone(),
            ));
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    let edge_capacity = usize::try_from(lane_edge_count)
        .map_err(|_| output_overflow(&unit, primary_span.clone()))?;
    let successor_capacity = usize::try_from(successor_count)
        .map_err(|_| output_overflow(&unit, primary_span.clone()))?;
    let mut lane_edge_sources = Vec::with_capacity(edge_capacity);
    let mut lane_edge_successor_sources = Vec::with_capacity(successor_capacity);
    for mir_key in frozen_lir.canonical_mir_edge_order.iter().copied() {
        let edge = &mir.lane_edges[mir_key.index()];
        let ordinal = frozen_lir.mir_to_lir[mir_key.index()];
        let source_document_ordinal = mir.modules[edge.module.index()].source_document_ordinal;
        debug_assert_eq!(
            edge.source_span.source_document_key(),
            unit.modules[edge.module.index()]
                .descriptor()
                .source_document_key()
        );
        lane_edge_sources.push(LaneEdgeSourceRecord {
            ordinal,
            stable_id: edge.stable_id,
            primary: location(source_document_ordinal, &edge.source_span),
        });
        for (local_index, connection) in mir.lane_edge_connections
            [edge.connections.as_usize_range()]
        .iter()
        .enumerate()
        {
            debug_assert_eq!(
                connection.source_span.source_document_key(),
                edge.source_span.source_document_key()
            );
            lane_edge_successor_sources.push(LaneEdgeSuccessorSourceRecord {
                owner_ordinal: ordinal,
                owner_stable_id: edge.stable_id,
                role: SourceRelationRole::LaneEdgeSuccessor,
                local_index: u32::try_from(local_index)
                    .expect("LIR relation range precheck proved local index fits u32"),
                primary: location(source_document_ordinal, &connection.source_span),
            });
        }
    }

    let mut road_corridor_sources = Vec::with_capacity(mir.road_corridors.len());
    let mut road_section_sources = Vec::with_capacity(mir.road_sections.len());
    let mut authoring_lane_sources = Vec::with_capacity(mir.authoring_lanes.len());
    let mut lane_group_sources = Vec::with_capacity(mir.lane_groups.len());
    let mut facility_band_sources = Vec::with_capacity(mir.facility_bands.len());
    let mut cross_section_relation_sources = Vec::with_capacity(
        usize::try_from(cross_relation_count)
            .map_err(|_| output_overflow(&unit, primary_span.clone()))?,
    );

    for mir_key in frozen_lir.canonical_mir_corridor_order.iter().copied() {
        let corridor = &mir.road_corridors[mir_key.index()];
        let source_document_ordinal = mir.modules[corridor.module.index()].source_document_ordinal;
        let ordinal = frozen_lir.mir_corridor_to_lir[mir_key.index()];
        road_corridor_sources.push(StableEntitySourceRecord {
            ordinal,
            stable_id: corridor.stable_id,
            primary: location(source_document_ordinal, &corridor.source_span),
        });
        for local_index in 0..corridor.elements.len() {
            cross_section_relation_sources.push(CrossSectionRelationSourceRecord {
                owner: CrossSectionRelationOwnerRecord::RoadCorridor(ordinal, corridor.stable_id),
                role: SourceRelationRole::RoadCorridorElement,
                local_index,
                primary: location(source_document_ordinal, &corridor.source_span),
            });
        }
    }
    for mir_key in frozen_lir.canonical_mir_section_order.iter().copied() {
        let section = &mir.road_sections[mir_key.index()];
        let source_document_ordinal = mir.modules[section.module.index()].source_document_ordinal;
        let ordinal = frozen_lir.mir_section_to_lir[mir_key.index()];
        road_section_sources.push(StableEntitySourceRecord {
            ordinal,
            stable_id: section.stable_id,
            primary: location(source_document_ordinal, &section.source_span),
        });
        for (local_index, lane_index) in section.lanes.as_usize_range().enumerate() {
            let lane = &mir.authoring_lanes[lane_index];
            cross_section_relation_sources.push(CrossSectionRelationSourceRecord {
                owner: CrossSectionRelationOwnerRecord::RoadSection(ordinal, section.stable_id),
                role: SourceRelationRole::RoadSectionLane,
                local_index: u32::try_from(local_index)
                    .expect("MIR range precheck proved local index fits u32"),
                primary: location(
                    mir.modules[lane.module.index()].source_document_ordinal,
                    &lane.source_span,
                ),
            });
        }
    }
    for mir_key in frozen_lir.canonical_mir_lane_order.iter().copied() {
        let lane = &mir.authoring_lanes[mir_key.index()];
        let source_document_ordinal = mir.modules[lane.module.index()].source_document_ordinal;
        let ordinal = frozen_lir.mir_lane_to_lir[mir_key.index()];
        authoring_lane_sources.push(StableEntitySourceRecord {
            ordinal,
            stable_id: lane.stable_id,
            primary: location(source_document_ordinal, &lane.source_span),
        });
        for (local_index, edge) in mir.authoring_lane_edges[lane.edge_chain.as_usize_range()]
            .iter()
            .enumerate()
        {
            cross_section_relation_sources.push(CrossSectionRelationSourceRecord {
                owner: CrossSectionRelationOwnerRecord::AuthoringLane(ordinal, lane.stable_id),
                role: SourceRelationRole::AuthoringLaneEdge,
                local_index: u32::try_from(local_index)
                    .expect("MIR range precheck proved local index fits u32"),
                primary: location(source_document_ordinal, &edge.source_span),
            });
        }
    }
    for mir_key in frozen_lir.canonical_mir_group_order.iter().copied() {
        let group = &mir.lane_groups[mir_key.index()];
        let source_document_ordinal = mir.modules[group.module.index()].source_document_ordinal;
        let ordinal = frozen_lir.mir_group_to_lir[mir_key.index()];
        lane_group_sources.push(StableEntitySourceRecord {
            ordinal,
            stable_id: group.stable_id,
            primary: location(source_document_ordinal, &group.source_span),
        });
        for (local_index, member) in mir.lane_group_members[group.members.as_usize_range()]
            .iter()
            .enumerate()
        {
            let lane = &mir.authoring_lanes[member.lane.index()];
            cross_section_relation_sources.push(CrossSectionRelationSourceRecord {
                owner: CrossSectionRelationOwnerRecord::LaneGroup(ordinal, group.stable_id),
                role: SourceRelationRole::LaneGroupMember,
                local_index: u32::try_from(local_index)
                    .expect("MIR range precheck proved local index fits u32"),
                primary: location(
                    mir.modules[lane.module.index()].source_document_ordinal,
                    &lane.source_span,
                ),
            });
        }
    }
    for mir_key in frozen_lir.canonical_mir_band_order.iter().copied() {
        let band = &mir.facility_bands[mir_key.index()];
        facility_band_sources.push(StableEntitySourceRecord {
            ordinal: frozen_lir.mir_band_to_lir[mir_key.index()],
            stable_id: band.stable_id,
            primary: location(
                mir.modules[band.module.index()].source_document_ordinal,
                &band.source_span,
            ),
        });
    }

    debug_assert_eq!(lane_edge_sources.len(), edge_capacity);
    debug_assert_eq!(lane_edge_successor_sources.len(), successor_capacity);
    let source_modules = unit.into_source_module_descriptors();
    Ok(ValidatedSourceMapInput {
        source_modules,
        lane_edge_sources: lane_edge_sources.into_boxed_slice(),
        lane_edge_successor_sources: lane_edge_successor_sources.into_boxed_slice(),
        road_corridor_sources: road_corridor_sources.into_boxed_slice(),
        road_section_sources: road_section_sources.into_boxed_slice(),
        authoring_lane_sources: authoring_lane_sources.into_boxed_slice(),
        lane_group_sources: lane_group_sources.into_boxed_slice(),
        facility_band_sources: facility_band_sources.into_boxed_slice(),
        cross_section_relation_sources: cross_section_relation_sources.into_boxed_slice(),
    })
}

fn location(
    source_document_ordinal: SourceDocumentOrdinal,
    span: &SourceSpan,
) -> SourceLocationRecord {
    SourceLocationRecord {
        source_document_ordinal,
        start: span.start(),
        end: span.end(),
    }
}

fn requested_bytes<T>(count: u64) -> u64 {
    count.saturating_mul(u64::try_from(size_of::<T>()).unwrap_or(u64::MAX))
}

fn output_overflow(unit: &CompilationUnit, primary_span: Option<SourceSpan>) -> DiagnosticBundle {
    DiagnosticBundle::single(Diagnostic::compile_limit_exceeded_at(
        CompileLimitDimension::OutputBytes,
        unit.limits.value(CompileLimitDimension::OutputBytes),
        u64::MAX,
        primary_span,
        None,
    ))
}

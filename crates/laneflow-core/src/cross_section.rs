//! 横断面设施物理身份（FacilityKind）与 cross-section registry（SSOT §3/§4/§7）。

use std::ops::Range;

use indexmap::{IndexMap, IndexSet};

use crate::{
    error::CoreError,
    graph::LaneGraph,
    handle::{
        EdgeHandle, FacilityBandHandle, LaneGroupHandle, RoadCorridorHandle, RoadSectionHandle,
    },
    id::validate_external_id,
    junction::validate_capacity,
};

/// 设施类别：lane-bearing（可作 RoadSection kindId）或 non-traversable（FacilityBand）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FacilityKindCategory {
    /// 承载车道的可遍历设施类别。
    LaneBearing,
    /// 非遍历设施带类别。
    NonTraversable,
}

/// 设施物理身份的开放 token 词汇：SSOT seed 值 + `x-` 前缀自定义扩展。
///
/// `x-lane-` 前缀声明自定义 lane-bearing kind，其余 `x-` 前缀声明自定义
/// non-traversable band kind；Core 不赋予自定义 kind 任何行为语义。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FacilityKind {
    /// seed：机动车道。
    MotorLane,
    /// seed：非机动车道。
    NonMotorLane,
    /// seed：人行道。
    Sidewalk,
    /// seed：中央分隔带。
    Median,
    /// seed：绿化带。
    PlantingStrip,
    /// seed：设施带。
    FacilityStrip,
    /// seed：路肩。
    Shoulder,
    /// `x-lane-` 前缀的自定义 lane-bearing kind，保留完整 token。
    CustomLaneBearing(String),
    /// 其余 `x-` 前缀的自定义 non-traversable band kind，保留完整 token。
    CustomBand(String),
}

impl FacilityKind {
    /// 解析 kind token。未在 seed 表且无合法 `x-` 前缀的 token 返回
    /// `CoreError::UnknownFacilityKind`；纯 `x-`/`x-lane-`（前缀后无剩余部分）同样拒绝。
    pub fn parse(token: &str) -> Result<Self, CoreError> {
        match token {
            "motorLane" => return Ok(Self::MotorLane),
            "nonMotorLane" => return Ok(Self::NonMotorLane),
            "sidewalk" => return Ok(Self::Sidewalk),
            "median" => return Ok(Self::Median),
            "plantingStrip" => return Ok(Self::PlantingStrip),
            "facilityStrip" => return Ok(Self::FacilityStrip),
            "shoulder" => return Ok(Self::Shoulder),
            _ => {}
        }

        if let Some(rest) = token.strip_prefix("x-lane-") {
            if !rest.is_empty() {
                return Ok(Self::CustomLaneBearing(token.to_owned()));
            }
        } else if let Some(rest) = token.strip_prefix("x-") {
            if !rest.is_empty() {
                return Ok(Self::CustomBand(token.to_owned()));
            }
        }

        Err(CoreError::UnknownFacilityKind {
            kind: token.to_owned(),
        })
    }

    /// 返回 kind 的设施类别。
    pub const fn category(&self) -> FacilityKindCategory {
        match self {
            Self::MotorLane | Self::NonMotorLane | Self::CustomLaneBearing(_) => {
                FacilityKindCategory::LaneBearing
            }
            Self::Sidewalk
            | Self::Median
            | Self::PlantingStrip
            | Self::FacilityStrip
            | Self::Shoulder
            | Self::CustomBand(_) => FacilityKindCategory::NonTraversable,
        }
    }
}

/// FacilityBand 输入定义。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FacilityBand {
    id: String,
    kind_id: String,
}

impl FacilityBand {
    /// 创建 FacilityBand。ID 语法、唯一性与 kind 类别由
    /// `CrossSectionRegistry::try_new` 校验。
    pub fn new(id: impl Into<String>, kind_id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind_id: kind_id.into(),
        }
    }

    /// 返回 FacilityBand external ID。
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 返回 FacilityKind token。
    pub fn kind_id(&self) -> &str {
        &self.kind_id
    }
}

/// RoadSection 内单条 lane 的输入定义（ordered edge 链 + 可选 LaneGroup）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SectionLane {
    edge_ids: Vec<String>,
    lane_group_id: Option<String>,
}

impl SectionLane {
    /// 创建 SectionLane。edge 引用、链连通性与 group 归属由
    /// `CrossSectionRegistry::try_new` 校验。
    pub fn new<I, S>(edge_ids: I, lane_group_id: Option<&str>) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            edge_ids: edge_ids.into_iter().map(Into::into).collect(),
            lane_group_id: lane_group_id.map(str::to_owned),
        }
    }

    /// 返回 ordered lane edge external IDs。
    pub fn edge_ids(&self) -> &[String] {
        &self.edge_ids
    }

    /// 返回可选 LaneGroup external ID。
    pub fn lane_group_id(&self) -> Option<&str> {
        self.lane_group_id.as_deref()
    }
}

/// RoadSection 输入定义。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoadSection {
    id: String,
    kind_id: String,
    lanes: Vec<SectionLane>,
}

impl RoadSection {
    /// 创建 RoadSection。ID 语法、唯一性、kind 类别与 lane body 由
    /// `CrossSectionRegistry::try_new` 校验。
    pub fn new<I>(id: impl Into<String>, kind_id: impl Into<String>, lanes: I) -> Self
    where
        I: IntoIterator<Item = SectionLane>,
    {
        Self {
            id: id.into(),
            kind_id: kind_id.into(),
            lanes: lanes.into_iter().collect(),
        }
    }

    /// 返回 RoadSection external ID。
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 返回 FacilityKind token。
    pub fn kind_id(&self) -> &str {
        &self.kind_id
    }

    /// 返回 ordered lanes（lane index 按 corridor reference 方向从左到右）。
    pub fn lanes(&self) -> &[SectionLane] {
        &self.lanes
    }
}

/// LaneGroup 输入定义。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneGroup {
    id: String,
    road_section_id: String,
}

impl LaneGroup {
    /// 创建 LaneGroup。ID 语法、唯一性、parent 引用与成员关系由
    /// `CrossSectionRegistry::try_new` 校验。
    pub fn new(id: impl Into<String>, road_section_id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            road_section_id: road_section_id.into(),
        }
    }

    /// 返回 LaneGroup external ID。
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 返回 parent RoadSection external ID。
    pub fn road_section_id(&self) -> &str {
        &self.road_section_id
    }
}

/// RoadCorridor cross-section 元素引用（RoadSection 或 FacilityBand 二选一）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CorridorElementId {
    /// 引用 RoadSection。
    Section(String),
    /// 引用 FacilityBand。
    Band(String),
}

impl CorridorElementId {
    /// 创建 RoadSection 元素引用。
    pub fn section(id: impl Into<String>) -> Self {
        Self::Section(id.into())
    }

    /// 创建 FacilityBand 元素引用。
    pub fn band(id: impl Into<String>) -> Self {
        Self::Band(id.into())
    }

    /// 返回元素 external ID。
    pub fn id(&self) -> &str {
        match self {
            Self::Section(id) | Self::Band(id) => id,
        }
    }
}

/// RoadCorridor 输入定义。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoadCorridor {
    id: String,
    reference_section_id: String,
    elements: Vec<CorridorElementId>,
}

impl RoadCorridor {
    /// 创建 RoadCorridor。ID 语法、唯一性、elements 与 reference 成员性由
    /// `CrossSectionRegistry::try_new` 校验。
    pub fn new<I>(
        id: impl Into<String>,
        reference_section_id: impl Into<String>,
        elements: I,
    ) -> Self
    where
        I: IntoIterator<Item = CorridorElementId>,
    {
        Self {
            id: id.into(),
            reference_section_id: reference_section_id.into(),
            elements: elements.into_iter().collect(),
        }
    }

    /// 返回 RoadCorridor external ID。
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 返回声明 corridor 参考方向的成员 RoadSection external ID。
    pub fn reference_section_id(&self) -> &str {
        &self.reference_section_id
    }

    /// 返回 ordered cross-section 元素（按参考方向从左到右）。
    pub fn elements(&self) -> &[CorridorElementId] {
        &self.elements
    }
}

/// RoadCorridor cross-section 元素的 resolved 形态。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CorridorElement {
    /// RoadSection 元素。
    Section(RoadSectionHandle),
    /// FacilityBand 元素。
    Band(FacilityBandHandle),
}

impl CorridorElement {
    fn kind(self) -> &'static str {
        match self {
            Self::Section(_) => "section",
            Self::Band(_) => "band",
        }
    }
}

/// corridor 接缝一侧的横向邻居（SSOT §3.2.1，纯顺序派生，无方向数据）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeamNeighbor {
    /// section 元素贡献的 index 序最外侧 lane（左侧元素贡献最大 index lane，
    /// 右侧元素贡献 index 0 lane；lane index 按 corridor reference 方向从左到右）。
    OutermostLane {
        /// 所属 RoadSection。
        section: RoadSectionHandle,
        /// section 内 lane index。
        lane_index: usize,
    },
    /// FacilityBand 元素作为非遍历侧参与接缝。
    Band(FacilityBandHandle),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolvedFacilityBand {
    definition: FacilityBand,
    kind: FacilityKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolvedRoadSection {
    definition: RoadSection,
    kind: FacilityKind,
    lanes: Range<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolvedLaneGroup {
    definition: LaneGroup,
    section: RoadSectionHandle,
    lanes: Range<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolvedRoadCorridor {
    definition: RoadCorridor,
    reference_section: RoadSectionHandle,
    elements: Range<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SectionLaneEntry {
    edges: Range<usize>,
    group: Option<LaneGroupHandle>,
}

/// `RoadCorridor -> (RoadSection | FacilityBand)` 与 `RoadSection -> lane -> LaneEdge`
/// 的 immutable normalized cross-section aggregate（SSOT §3/§7）。
///
/// storage 沿用 road-junction-model §5 的 flat 形状：dense definitions +
/// flat corridor elements（per-corridor range）+ flat lanes（per-section range）+
/// flat lane edges（per-lane range）+ per-group lane member range + `edge_to_lane` 反查。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrossSectionRegistry {
    bands: Vec<ResolvedFacilityBand>,
    sections: Vec<ResolvedRoadSection>,
    groups: Vec<ResolvedLaneGroup>,
    corridors: Vec<ResolvedRoadCorridor>,
    band_handles: IndexMap<String, FacilityBandHandle>,
    section_handles: IndexMap<String, RoadSectionHandle>,
    group_handles: IndexMap<String, LaneGroupHandle>,
    corridor_handles: IndexMap<String, RoadCorridorHandle>,
    corridor_elements: Vec<CorridorElement>,
    section_lanes: Vec<SectionLaneEntry>,
    lane_edges: Vec<EdgeHandle>,
    group_lanes: Vec<usize>,
    edge_to_lane: Vec<Option<(RoadSectionHandle, usize)>>,
}

impl CrossSectionRegistry {
    /// 创建并校验 cross-section static topology。
    ///
    /// 校验按 SSOT §10 phase 3-8 顺序进行，identity phase 一律先于引用解析
    /// phase，同 phase 内按 input order 返回首错，任一错误不发布部分 registry：
    /// 3. FacilityBand：ID syntax/duplicate、unknown kindId、kind 类别错误；
    /// 4. RoadSection identity：ID syntax/duplicate；
    /// 5. LaneGroup identity：ID syntax/duplicate、unknown roadSectionId；
    /// 6. RoadSection body：kind 类别、empty lanes、empty lane chain、unknown edge、
    ///    链内 disconnected transition、链内 edge 重复、edge 多 lane/多 section
    ///    占用、unknown laneGroupId、group 与 lane 所属 section 不一致；
    /// 7. LaneGroup membership：empty group；
    /// 8. RoadCorridor：ID syntax/duplicate、empty elements、unknown element、
    ///    elements 内重复、element 多 corridor、零归属、reference 非成员。
    pub fn try_new<B, S, G, C>(
        lane_graph: &LaneGraph,
        bands: B,
        sections: S,
        groups: G,
        corridors: C,
    ) -> Result<Self, CoreError>
    where
        B: IntoIterator<Item = FacilityBand>,
        S: IntoIterator<Item = RoadSection>,
        G: IntoIterator<Item = LaneGroup>,
        C: IntoIterator<Item = RoadCorridor>,
    {
        let band_definitions: Vec<FacilityBand> = bands.into_iter().collect();
        let section_definitions: Vec<RoadSection> = sections.into_iter().collect();
        let group_definitions: Vec<LaneGroup> = groups.into_iter().collect();
        let corridor_definitions: Vec<RoadCorridor> = corridors.into_iter().collect();

        validate_capacity("facilityBands", band_definitions.len())?;
        validate_capacity("roadSections", section_definitions.len())?;
        validate_capacity("laneGroups", group_definitions.len())?;
        validate_capacity("roadCorridors", corridor_definitions.len())?;

        let mut lane_count = 0_usize;
        let mut lane_edge_ref_count = 0_usize;
        for section in &section_definitions {
            lane_count = lane_count.checked_add(section.lanes().len()).ok_or(
                CoreError::StaticDomainCapacityExceeded {
                    domain: "sectionLanes",
                    count: usize::MAX,
                    max_inclusive: u32::MAX,
                },
            )?;
            for lane in section.lanes() {
                lane_edge_ref_count = lane_edge_ref_count
                    .checked_add(lane.edge_ids().len())
                    .ok_or(CoreError::StaticDomainCapacityExceeded {
                        domain: "sectionLaneEdgeRefs",
                        count: usize::MAX,
                        max_inclusive: u32::MAX,
                    })?;
            }
        }
        validate_capacity("sectionLanes", lane_count)?;
        validate_capacity("sectionLaneEdgeRefs", lane_edge_ref_count)?;
        let corridor_element_count =
            corridor_definitions
                .iter()
                .try_fold(0_usize, |total, corridor| {
                    total.checked_add(corridor.elements().len()).ok_or(
                        CoreError::StaticDomainCapacityExceeded {
                            domain: "corridorElements",
                            count: usize::MAX,
                            max_inclusive: u32::MAX,
                        },
                    )
                })?;
        validate_capacity("corridorElements", corridor_element_count)?;

        // phase 3a：FacilityBand identity（ID syntax/duplicate）。
        let mut band_handles = IndexMap::new();
        for (index, band) in band_definitions.iter().enumerate() {
            validate_external_id("facilityBands[].id", band.id())?;
            if band_handles.contains_key(band.id()) {
                return Err(CoreError::DuplicateFacilityBandId {
                    band_id: band.id().to_owned(),
                });
            }
            band_handles.insert(band.id().to_owned(), FacilityBandHandle::new(index));
        }

        // phase 3b：FacilityBand kind（unknown kindId、类别必须 non-traversable）。
        let mut band_kinds = Vec::with_capacity(band_definitions.len());
        for band in &band_definitions {
            let kind = FacilityKind::parse(band.kind_id())?;
            if kind.category() != FacilityKindCategory::NonTraversable {
                return Err(CoreError::FacilityBandKindNotNonTraversable {
                    band_id: band.id().to_owned(),
                    kind_id: band.kind_id().to_owned(),
                });
            }
            band_kinds.push(kind);
        }

        // phase 4：RoadSection identity（先于 LaneGroup parent 解析）。
        let mut section_handles = IndexMap::new();
        for (index, section) in section_definitions.iter().enumerate() {
            validate_external_id("roadSections[].id", section.id())?;
            if section_handles.contains_key(section.id()) {
                return Err(CoreError::DuplicateRoadSectionId {
                    section_id: section.id().to_owned(),
                });
            }
            section_handles.insert(section.id().to_owned(), RoadSectionHandle::new(index));
        }

        // phase 5：LaneGroup identity + unknown roadSectionId（先于 RoadSection body）。
        let mut group_handles = IndexMap::new();
        let mut group_sections = Vec::with_capacity(group_definitions.len());
        for (index, group) in group_definitions.iter().enumerate() {
            validate_external_id("laneGroups[].id", group.id())?;
            if group_handles.contains_key(group.id()) {
                return Err(CoreError::DuplicateLaneGroupId {
                    group_id: group.id().to_owned(),
                });
            }
            validate_external_id("laneGroups[].roadSectionId", group.road_section_id())?;
            let section = section_handles
                .get(group.road_section_id())
                .copied()
                .ok_or_else(|| CoreError::UnknownLaneGroupRoadSection {
                    group_id: group.id().to_owned(),
                    section_id: group.road_section_id().to_owned(),
                })?;
            group_handles.insert(group.id().to_owned(), LaneGroupHandle::new(index));
            group_sections.push(section);
        }

        // phase 6a：RoadSection kind（unknown kindId、类别必须 lane-bearing）。
        let mut section_kinds = Vec::with_capacity(section_definitions.len());
        for section in &section_definitions {
            let kind = FacilityKind::parse(section.kind_id())?;
            if kind.category() != FacilityKindCategory::LaneBearing {
                return Err(CoreError::RoadSectionKindNotLaneBearing {
                    section_id: section.id().to_owned(),
                    kind_id: section.kind_id().to_owned(),
                });
            }
            section_kinds.push(kind);
        }

        // phase 6b：empty lanes。
        for section in &section_definitions {
            if section.lanes().is_empty() {
                return Err(CoreError::EmptyRoadSectionLanes {
                    section_id: section.id().to_owned(),
                });
            }
        }

        // phase 6c：empty lane chain。
        for section in &section_definitions {
            for (lane_index, lane) in section.lanes().iter().enumerate() {
                if lane.edge_ids().is_empty() {
                    return Err(CoreError::EmptySectionLaneChain {
                        section_id: section.id().to_owned(),
                        lane_index,
                    });
                }
            }
        }

        // phase 6d：edge 解析（unknown edge）。
        let mut lane_edge_scratch: Vec<Vec<Vec<EdgeHandle>>> =
            Vec::with_capacity(section_definitions.len());
        for section in &section_definitions {
            let mut section_scratch = Vec::with_capacity(section.lanes().len());
            for (lane_index, lane) in section.lanes().iter().enumerate() {
                let mut edges = Vec::with_capacity(lane.edge_ids().len());
                for edge_id in lane.edge_ids() {
                    validate_external_id("roadSections[].lanes[].edgeIds[]", edge_id)?;
                    edges.push(lane_graph.edge_handle(edge_id).ok_or_else(|| {
                        CoreError::UnknownSectionLaneEdge {
                            section_id: section.id().to_owned(),
                            lane_index,
                            edge_id: edge_id.to_owned(),
                        }
                    })?);
                }
                section_scratch.push(edges);
            }
            lane_edge_scratch.push(section_scratch);
        }

        // phase 6e：链内 disconnected transition。
        for (section, section_scratch) in section_definitions.iter().zip(&lane_edge_scratch) {
            for (lane_index, edges) in section_scratch.iter().enumerate() {
                for (transition_index, pair) in edges.windows(2).enumerate() {
                    if !lane_graph.can_traverse(pair[0], pair[1]) {
                        return Err(CoreError::DisconnectedSectionLane {
                            section_id: section.id().to_owned(),
                            lane_index,
                            transition_index,
                            from_edge_id: lane_graph
                                .edge_external_id(pair[0])
                                .expect("resolved lane edge must belong to lane graph")
                                .to_owned(),
                            to_edge_id: lane_graph
                                .edge_external_id(pair[1])
                                .expect("resolved lane edge must belong to lane graph")
                                .to_owned(),
                        });
                    }
                }
            }
        }

        // phase 6f：lane 链内 edge 重复。
        for (section, section_scratch) in section_definitions.iter().zip(&lane_edge_scratch) {
            for (lane_index, edges) in section_scratch.iter().enumerate() {
                let mut seen = IndexSet::new();
                for edge in edges {
                    if !seen.insert(*edge) {
                        return Err(CoreError::DuplicateSectionLaneEdge {
                            section_id: section.id().to_owned(),
                            lane_index,
                            edge_id: lane_graph
                                .edge_external_id(*edge)
                                .expect("resolved lane edge must belong to lane graph")
                                .to_owned(),
                        });
                    }
                }
            }
        }

        // phase 6g：一条 LaneEdge 至多一条 lane、至多一个 section。
        let mut edge_to_lane = vec![None::<(RoadSectionHandle, usize)>; lane_graph.edges().len()];
        for (section_index, (section, section_scratch)) in section_definitions
            .iter()
            .zip(&lane_edge_scratch)
            .enumerate()
        {
            for (lane_index, edges) in section_scratch.iter().enumerate() {
                for edge in edges {
                    if let Some((first_section, first_lane_index)) = edge_to_lane[edge.index()] {
                        return Err(CoreError::SectionLaneEdgeClaimConflict {
                            edge_id: lane_graph
                                .edge_external_id(*edge)
                                .expect("resolved lane edge must belong to lane graph")
                                .to_owned(),
                            first_section_id: section_definitions[first_section.index()]
                                .id()
                                .to_owned(),
                            first_lane_index,
                            duplicate_section_id: section.id().to_owned(),
                            duplicate_lane_index: lane_index,
                        });
                    }
                    edge_to_lane[edge.index()] =
                        Some((RoadSectionHandle::new(section_index), lane_index));
                }
            }
        }

        // phase 6h：lane 的 laneGroupId 解析与 section 一致性。
        let mut lane_group_scratch: Vec<Vec<Option<LaneGroupHandle>>> =
            Vec::with_capacity(section_definitions.len());
        for (section_index, section) in section_definitions.iter().enumerate() {
            let mut section_groups = Vec::with_capacity(section.lanes().len());
            for (lane_index, lane) in section.lanes().iter().enumerate() {
                let group = match lane.lane_group_id() {
                    Some(group_id) => {
                        validate_external_id("roadSections[].lanes[].laneGroupId", group_id)?;
                        let group = group_handles.get(group_id).copied().ok_or_else(|| {
                            CoreError::UnknownSectionLaneGroup {
                                section_id: section.id().to_owned(),
                                lane_index,
                                group_id: group_id.to_owned(),
                            }
                        })?;
                        if group_sections[group.index()] != RoadSectionHandle::new(section_index) {
                            return Err(CoreError::SectionLaneGroupSectionMismatch {
                                section_id: section.id().to_owned(),
                                lane_index,
                                group_id: group_id.to_owned(),
                                group_section_id: section_definitions
                                    [group_sections[group.index()].index()]
                                .id()
                                .to_owned(),
                            });
                        }
                        Some(group)
                    }
                    None => None,
                };
                section_groups.push(group);
            }
            lane_group_scratch.push(section_groups);
        }

        // phase 7：LaneGroup membership（empty group）。
        let mut group_member_counts = vec![0_usize; group_definitions.len()];
        for section_groups in &lane_group_scratch {
            for group in section_groups.iter().flatten() {
                group_member_counts[group.index()] += 1;
            }
        }
        for (index, group) in group_definitions.iter().enumerate() {
            if group_member_counts[index] == 0 {
                return Err(CoreError::EmptyLaneGroup {
                    group_id: group.id().to_owned(),
                });
            }
        }

        // phase 8a：RoadCorridor identity。
        let mut corridor_handles = IndexMap::new();
        for (index, corridor) in corridor_definitions.iter().enumerate() {
            validate_external_id("roadCorridors[].id", corridor.id())?;
            if corridor_handles.contains_key(corridor.id()) {
                return Err(CoreError::DuplicateRoadCorridorId {
                    corridor_id: corridor.id().to_owned(),
                });
            }
            corridor_handles.insert(corridor.id().to_owned(), RoadCorridorHandle::new(index));
        }

        // phase 8b：empty elements 先于一切 element 依赖检查；reference ID syntax 同阶段。
        for corridor in &corridor_definitions {
            if corridor.elements().is_empty() {
                return Err(CoreError::EmptyRoadCorridorElements {
                    corridor_id: corridor.id().to_owned(),
                });
            }
            validate_external_id(
                "roadCorridors[].referenceSectionId",
                corridor.reference_section_id(),
            )?;
        }

        // phase 8c：element 引用解析（unknown element）。
        let mut corridor_element_scratch: Vec<Vec<CorridorElement>> =
            Vec::with_capacity(corridor_definitions.len());
        for corridor in &corridor_definitions {
            let mut elements = Vec::with_capacity(corridor.elements().len());
            for element in corridor.elements() {
                let resolved = match element {
                    CorridorElementId::Section(id) => {
                        validate_external_id("roadCorridors[].elements[].sectionId", id)?;
                        CorridorElement::Section(section_handles.get(id).copied().ok_or_else(
                            || CoreError::UnknownCorridorElement {
                                corridor_id: corridor.id().to_owned(),
                                element_kind: "section",
                                element_id: id.clone(),
                            },
                        )?)
                    }
                    CorridorElementId::Band(id) => {
                        validate_external_id("roadCorridors[].elements[].bandId", id)?;
                        CorridorElement::Band(band_handles.get(id).copied().ok_or_else(|| {
                            CoreError::UnknownCorridorElement {
                                corridor_id: corridor.id().to_owned(),
                                element_kind: "band",
                                element_id: id.clone(),
                            }
                        })?)
                    }
                };
                elements.push(resolved);
            }
            corridor_element_scratch.push(elements);
        }

        // phase 8d：elements 内重复引用同一 section/band。
        for (corridor, elements) in corridor_definitions.iter().zip(&corridor_element_scratch) {
            let mut seen = IndexSet::new();
            for element in elements {
                if !seen.insert(*element) {
                    return Err(CoreError::DuplicateCorridorElement {
                        corridor_id: corridor.id().to_owned(),
                        element_kind: element.kind(),
                        element_id: corridor_element_external_id(
                            &section_definitions,
                            &band_definitions,
                            *element,
                        ),
                    });
                }
            }
        }

        // phase 8e：完备 owner 树（多 corridor 占用 + 零归属）。
        let mut section_owners = vec![None::<usize>; section_definitions.len()];
        let mut band_owners = vec![None::<usize>; band_definitions.len()];
        for (corridor_index, (corridor, elements)) in corridor_definitions
            .iter()
            .zip(&corridor_element_scratch)
            .enumerate()
        {
            for element in elements {
                let (owners, owner_index) = match element {
                    CorridorElement::Section(section) => (&mut section_owners, section.index()),
                    CorridorElement::Band(band) => (&mut band_owners, band.index()),
                };
                if let Some(first_corridor_index) = owners[owner_index] {
                    return Err(CoreError::CorridorElementMultipleOwners {
                        element_kind: element.kind(),
                        element_id: corridor_element_external_id(
                            &section_definitions,
                            &band_definitions,
                            *element,
                        ),
                        first_corridor_id: corridor_definitions[first_corridor_index]
                            .id()
                            .to_owned(),
                        duplicate_corridor_id: corridor.id().to_owned(),
                    });
                }
                owners[owner_index] = Some(corridor_index);
            }
        }
        // 零归属按声明顺序检查：bands（phase 3）先于 sections（phase 4）。
        for (index, band) in band_definitions.iter().enumerate() {
            if band_owners[index].is_none() {
                return Err(CoreError::UnownedCorridorElement {
                    element_kind: "band",
                    element_id: band.id().to_owned(),
                });
            }
        }
        for (index, section) in section_definitions.iter().enumerate() {
            if section_owners[index].is_none() {
                return Err(CoreError::UnownedCorridorElement {
                    element_kind: "section",
                    element_id: section.id().to_owned(),
                });
            }
        }

        // phase 8f：referenceSectionId 必须是成员 section。
        let mut corridor_references = Vec::with_capacity(corridor_definitions.len());
        for (corridor, elements) in corridor_definitions.iter().zip(&corridor_element_scratch) {
            let reference = section_handles
                .get(corridor.reference_section_id())
                .copied()
                .filter(|reference| {
                    elements
                        .iter()
                        .any(|element| matches!(element, CorridorElement::Section(section) if section == reference))
                })
                .ok_or_else(|| CoreError::CorridorReferenceSectionNotMember {
                    corridor_id: corridor.id().to_owned(),
                    reference_section_id: corridor.reference_section_id().to_owned(),
                })?;
            corridor_references.push(reference);
        }

        // phase 10（横断面部分）：构造 dense storage 与 flat member ranges。
        let bands = band_definitions
            .into_iter()
            .zip(band_kinds)
            .map(|(definition, kind)| ResolvedFacilityBand { definition, kind })
            .collect();

        let mut section_lanes = Vec::with_capacity(lane_count);
        let mut lane_edges = Vec::with_capacity(lane_edge_ref_count);
        let mut sections = Vec::with_capacity(section_definitions.len());
        for (section_index, definition) in section_definitions.into_iter().enumerate() {
            let lanes_start = section_lanes.len();
            for (lane_index, edges) in lane_edge_scratch[section_index].iter().enumerate() {
                let edges_start = lane_edges.len();
                lane_edges.extend_from_slice(edges);
                section_lanes.push(SectionLaneEntry {
                    edges: edges_start..lane_edges.len(),
                    group: lane_group_scratch[section_index][lane_index],
                });
            }
            sections.push(ResolvedRoadSection {
                definition,
                kind: section_kinds[section_index].clone(),
                lanes: lanes_start..section_lanes.len(),
            });
        }

        // per-group lane member range：成员按 section lane index 顺序排列。
        let mut group_lanes = Vec::new();
        let mut groups = Vec::with_capacity(group_definitions.len());
        for (group_index, definition) in group_definitions.into_iter().enumerate() {
            let section = group_sections[group_index];
            let section_lanes_range = sections[section.index()].lanes.clone();
            let lanes_start = group_lanes.len();
            for (lane_index, entry) in section_lanes[section_lanes_range].iter().enumerate() {
                if entry.group == Some(LaneGroupHandle::new(group_index)) {
                    group_lanes.push(lane_index);
                }
            }
            groups.push(ResolvedLaneGroup {
                definition,
                section,
                lanes: lanes_start..group_lanes.len(),
            });
        }

        let mut corridor_elements = Vec::with_capacity(corridor_element_count);
        let mut corridors = Vec::with_capacity(corridor_definitions.len());
        for (corridor_index, definition) in corridor_definitions.into_iter().enumerate() {
            let elements_start = corridor_elements.len();
            corridor_elements.extend_from_slice(&corridor_element_scratch[corridor_index]);
            corridors.push(ResolvedRoadCorridor {
                definition,
                reference_section: corridor_references[corridor_index],
                elements: elements_start..corridor_elements.len(),
            });
        }

        Ok(Self {
            bands,
            sections,
            groups,
            corridors,
            band_handles,
            section_handles,
            group_handles,
            corridor_handles,
            corridor_elements,
            section_lanes,
            lane_edges,
            group_lanes,
            edge_to_lane,
        })
    }

    /// 创建不含任何 cross-section definition 的空 registry。
    pub fn empty() -> Self {
        Self {
            bands: Vec::new(),
            sections: Vec::new(),
            groups: Vec::new(),
            corridors: Vec::new(),
            band_handles: IndexMap::new(),
            section_handles: IndexMap::new(),
            group_handles: IndexMap::new(),
            corridor_handles: IndexMap::new(),
            corridor_elements: Vec::new(),
            section_lanes: Vec::new(),
            lane_edges: Vec::new(),
            group_lanes: Vec::new(),
            edge_to_lane: Vec::new(),
        }
    }

    /// 按 retained external definitions 对目标 LaneGraph 重新 normalization。
    pub fn rebind_to_lane_graph(&self, lane_graph: &LaneGraph) -> Result<Self, CoreError> {
        Self::try_new(
            lane_graph,
            self.bands.iter().map(|band| band.definition.clone()),
            self.sections
                .iter()
                .map(|section| section.definition.clone()),
            self.groups.iter().map(|group| group.definition.clone()),
            self.corridors
                .iter()
                .map(|corridor| corridor.definition.clone()),
        )
    }

    /// 返回 registry 是否为空。
    pub fn is_empty(&self) -> bool {
        self.bands.is_empty()
            && self.sections.is_empty()
            && self.groups.is_empty()
            && self.corridors.is_empty()
    }

    /// 返回 FacilityBand external ID 对应的 handle。
    pub fn band_handle(&self, external_id: &str) -> Option<FacilityBandHandle> {
        self.band_handles.get(external_id).copied()
    }

    /// 返回 FacilityBand handle 对应的 external ID。
    pub fn band_external_id(&self, handle: FacilityBandHandle) -> Option<&str> {
        self.band(handle).map(FacilityBand::id)
    }

    /// 返回指定 FacilityBand definition。
    pub fn band(&self, handle: FacilityBandHandle) -> Option<&FacilityBand> {
        self.bands
            .get(handle.index())
            .map(|resolved| &resolved.definition)
    }

    /// 返回指定 FacilityBand 的 resolved kind。
    pub fn band_kind(&self, handle: FacilityBandHandle) -> Option<&FacilityKind> {
        self.bands
            .get(handle.index())
            .map(|resolved| &resolved.kind)
    }

    /// 按 normalization order 遍历 FacilityBand handles。
    pub fn bands(&self) -> impl ExactSizeIterator<Item = FacilityBandHandle> + '_ {
        (0..self.bands.len()).map(FacilityBandHandle::new)
    }

    /// 返回 RoadSection external ID 对应的 handle。
    pub fn section_handle(&self, external_id: &str) -> Option<RoadSectionHandle> {
        self.section_handles.get(external_id).copied()
    }

    /// 返回 RoadSection handle 对应的 external ID。
    pub fn section_external_id(&self, handle: RoadSectionHandle) -> Option<&str> {
        self.section(handle).map(RoadSection::id)
    }

    /// 返回指定 RoadSection definition。
    pub fn section(&self, handle: RoadSectionHandle) -> Option<&RoadSection> {
        self.sections
            .get(handle.index())
            .map(|resolved| &resolved.definition)
    }

    /// 返回指定 RoadSection 的 resolved kind。
    pub fn section_kind(&self, handle: RoadSectionHandle) -> Option<&FacilityKind> {
        self.sections
            .get(handle.index())
            .map(|resolved| &resolved.kind)
    }

    /// 按 normalization order 遍历 RoadSection handles。
    pub fn sections(&self) -> impl ExactSizeIterator<Item = RoadSectionHandle> + '_ {
        (0..self.sections.len()).map(RoadSectionHandle::new)
    }

    /// 返回 LaneGroup external ID 对应的 handle。
    pub fn group_handle(&self, external_id: &str) -> Option<LaneGroupHandle> {
        self.group_handles.get(external_id).copied()
    }

    /// 返回 LaneGroup handle 对应的 external ID。
    pub fn group_external_id(&self, handle: LaneGroupHandle) -> Option<&str> {
        self.group(handle).map(LaneGroup::id)
    }

    /// 返回指定 LaneGroup definition。
    pub fn group(&self, handle: LaneGroupHandle) -> Option<&LaneGroup> {
        self.groups
            .get(handle.index())
            .map(|resolved| &resolved.definition)
    }

    /// 按 normalization order 遍历 LaneGroup handles。
    pub fn groups(&self) -> impl ExactSizeIterator<Item = LaneGroupHandle> + '_ {
        (0..self.groups.len()).map(LaneGroupHandle::new)
    }

    /// 返回 LaneGroup 的 parent RoadSection。
    pub fn lane_group_section(&self, handle: LaneGroupHandle) -> Option<RoadSectionHandle> {
        self.groups.get(handle.index()).map(|group| group.section)
    }

    /// 返回 RoadCorridor external ID 对应的 handle。
    pub fn corridor_handle(&self, external_id: &str) -> Option<RoadCorridorHandle> {
        self.corridor_handles.get(external_id).copied()
    }

    /// 返回 RoadCorridor handle 对应的 external ID。
    pub fn corridor_external_id(&self, handle: RoadCorridorHandle) -> Option<&str> {
        self.corridor(handle).map(RoadCorridor::id)
    }

    /// 返回指定 RoadCorridor definition。
    pub fn corridor(&self, handle: RoadCorridorHandle) -> Option<&RoadCorridor> {
        self.corridors
            .get(handle.index())
            .map(|resolved| &resolved.definition)
    }

    /// 按 normalization order 遍历 RoadCorridor handles。
    pub fn corridors(&self) -> impl ExactSizeIterator<Item = RoadCorridorHandle> + '_ {
        (0..self.corridors.len()).map(RoadCorridorHandle::new)
    }

    /// 返回 RoadCorridor 声明参考方向的成员 RoadSection。
    pub fn corridor_reference_section(
        &self,
        handle: RoadCorridorHandle,
    ) -> Option<RoadSectionHandle> {
        self.corridors
            .get(handle.index())
            .map(|corridor| corridor.reference_section)
    }

    /// 返回 RoadCorridor 的 ordered cross-section 元素。
    pub fn corridor_elements(&self, handle: RoadCorridorHandle) -> Option<&[CorridorElement]> {
        let range = self.corridors.get(handle.index())?.elements.clone();
        Some(&self.corridor_elements[range])
    }

    /// 返回 RoadSection 的 ordered lanes：`(lane_index, edge handles)`，
    /// lane index 按 corridor reference 方向从左到右（SSOT §3.2）。
    pub fn section_lanes(
        &self,
        handle: RoadSectionHandle,
    ) -> Option<impl ExactSizeIterator<Item = (usize, &[EdgeHandle])> + '_> {
        let range = self.sections.get(handle.index())?.lanes.clone();
        Some(
            self.section_lanes[range]
                .iter()
                .enumerate()
                .map(|(lane_index, entry)| (lane_index, &self.lane_edges[entry.edges.clone()])),
        )
    }

    /// 返回 LaneGroup 的成员 lane indices（所属 section 的 index 序，升序）。
    pub fn group_lanes(
        &self,
        handle: LaneGroupHandle,
    ) -> Option<impl ExactSizeIterator<Item = usize> + '_> {
        let range = self.groups.get(handle.index())?.lanes.clone();
        Some(self.group_lanes[range].iter().copied())
    }

    /// 返回 edge 的 lane 归属反查（SSOT §3.5）：`(section, lane_index)`；
    /// 未被任何 lane 覆盖的 edge 返回 `None`。
    pub fn edge_lane_membership(&self, edge: EdgeHandle) -> Option<(RoadSectionHandle, usize)> {
        self.edge_to_lane.get(edge.index()).copied().flatten()
    }

    /// 返回 corridor 接缝 `j` 两侧的横向邻居（SSOT §3.2.1）。
    ///
    /// 边界位于 `elements[j-1]` 与 `elements[j]` 之间，`j ∈ 1..elements.len()`；
    /// 越界返回 `None`。派生纯基于元素与 lane index 顺序，不依赖任何方向数据。
    pub fn corridor_seam_neighbors(
        &self,
        handle: RoadCorridorHandle,
        j: usize,
    ) -> Option<(SeamNeighbor, SeamNeighbor)> {
        let range = self.corridors.get(handle.index())?.elements.clone();
        if j == 0 || j >= range.len() {
            return None;
        }
        let elements = &self.corridor_elements[range];
        let left = self.seam_neighbor(elements[j - 1], SeamSide::Left)?;
        let right = self.seam_neighbor(elements[j], SeamSide::Right)?;
        Some((left, right))
    }

    fn seam_neighbor(&self, element: CorridorElement, side: SeamSide) -> Option<SeamNeighbor> {
        match element {
            CorridorElement::Band(band) => Some(SeamNeighbor::Band(band)),
            CorridorElement::Section(section) => {
                let lane_count = self.sections.get(section.index())?.lanes.len();
                let lane_index = match side {
                    // 左侧元素的接缝侧是 reference 方向最右（最大 index lane）。
                    SeamSide::Left => lane_count - 1,
                    // 右侧元素的接缝侧是 reference 方向最左（index 0 lane）。
                    SeamSide::Right => 0,
                };
                Some(SeamNeighbor::OutermostLane {
                    section,
                    lane_index,
                })
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn retained_bytes(&self) -> usize {
        fn string_key_map_bytes<V>(map: &IndexMap<String, V>) -> usize {
            map.capacity() * std::mem::size_of::<(String, V)>()
                + map.keys().map(String::capacity).sum::<usize>()
        }

        let Self {
            bands,
            sections,
            groups,
            corridors,
            band_handles,
            section_handles,
            group_handles,
            corridor_handles,
            corridor_elements,
            section_lanes,
            lane_edges,
            group_lanes,
            edge_to_lane,
        } = self;

        let band_bytes = bands.capacity() * std::mem::size_of::<ResolvedFacilityBand>()
            + bands
                .iter()
                .map(|band| {
                    band.definition.id.capacity()
                        + band.definition.kind_id.capacity()
                        + match &band.kind {
                            FacilityKind::CustomLaneBearing(token)
                            | FacilityKind::CustomBand(token) => token.capacity(),
                            _ => 0,
                        }
                })
                .sum::<usize>();
        let section_bytes = sections.capacity() * std::mem::size_of::<ResolvedRoadSection>()
            + sections
                .iter()
                .map(|section| {
                    section.definition.id.capacity()
                        + section.definition.kind_id.capacity()
                        + match &section.kind {
                            FacilityKind::CustomLaneBearing(token)
                            | FacilityKind::CustomBand(token) => token.capacity(),
                            _ => 0,
                        }
                        + section.definition.lanes.capacity() * std::mem::size_of::<SectionLane>()
                        + section
                            .definition
                            .lanes
                            .iter()
                            .map(|lane| {
                                lane.edge_ids.capacity() * std::mem::size_of::<String>()
                                    + lane.edge_ids.iter().map(String::capacity).sum::<usize>()
                                    + lane.lane_group_id.as_ref().map_or(0, String::capacity)
                            })
                            .sum::<usize>()
                })
                .sum::<usize>();
        let group_bytes = groups.capacity() * std::mem::size_of::<ResolvedLaneGroup>()
            + groups
                .iter()
                .map(|group| {
                    group.definition.id.capacity() + group.definition.road_section_id.capacity()
                })
                .sum::<usize>();
        let corridor_bytes = corridors.capacity() * std::mem::size_of::<ResolvedRoadCorridor>()
            + corridors
                .iter()
                .map(|corridor| {
                    corridor.definition.id.capacity()
                        + corridor.definition.reference_section_id.capacity()
                        + corridor.definition.elements.capacity()
                            * std::mem::size_of::<CorridorElementId>()
                        + corridor
                            .definition
                            .elements
                            .iter()
                            .map(|element| match element {
                                CorridorElementId::Section(id) | CorridorElementId::Band(id) => {
                                    id.capacity()
                                }
                            })
                            .sum::<usize>()
                })
                .sum::<usize>();
        let resolver_bytes = string_key_map_bytes(band_handles)
            + string_key_map_bytes(section_handles)
            + string_key_map_bytes(group_handles)
            + string_key_map_bytes(corridor_handles);
        let flat_index_bytes = corridor_elements.capacity()
            * std::mem::size_of::<CorridorElement>()
            + section_lanes.capacity() * std::mem::size_of::<SectionLaneEntry>()
            + lane_edges.capacity() * std::mem::size_of::<EdgeHandle>()
            + group_lanes.capacity() * std::mem::size_of::<usize>()
            + edge_to_lane.capacity() * std::mem::size_of::<Option<(RoadSectionHandle, usize)>>();

        band_bytes
            + section_bytes
            + group_bytes
            + corridor_bytes
            + resolver_bytes
            + flat_index_bytes
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SeamSide {
    Left,
    Right,
}

fn corridor_element_external_id(
    sections: &[RoadSection],
    bands: &[FacilityBand],
    element: CorridorElement,
) -> String {
    match element {
        CorridorElement::Section(section) => sections[section.index()].id().to_owned(),
        CorridorElement::Band(band) => bands[band.index()].id().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{EdgeLength, LaneEdge, SpeedLimit};

    fn test_graph() -> LaneGraph {
        LaneGraph::try_new([LaneEdge::new(
            "edge-1",
            EdgeLength::try_new(10.0).expect("test edge length"),
            SpeedLimit::try_new(10.0).expect("test speed limit"),
            Vec::<String>::new(),
        )])
        .expect("test graph")
    }

    #[test]
    fn retained_bytes_tracks_declared_cross_section() {
        // 零基线：空 graph + 空声明，所有派生表（含 edge 反查）均为空。
        let empty = CrossSectionRegistry::try_new(
            &LaneGraph::empty(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .expect("empty cross-section is valid");
        let graph = test_graph();
        let registry = CrossSectionRegistry::try_new(
            &graph,
            [FacilityBand::new("band-1", "median")],
            [RoadSection::new(
                "section-1",
                "motorLane",
                [SectionLane::new(["edge-1"], None)],
            )],
            Vec::new(),
            [RoadCorridor::new(
                "corridor-1",
                "section-1",
                [
                    CorridorElementId::band("band-1"),
                    CorridorElementId::section("section-1"),
                ],
            )],
        )
        .expect("valid cross-section registry");

        assert_eq!(empty.retained_bytes(), 0);
        assert!(registry.retained_bytes() > 0);
    }
}

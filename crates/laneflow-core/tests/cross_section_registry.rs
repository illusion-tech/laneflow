use laneflow_core::{
    CoreError, CorridorElement, CorridorElementId, CrossSectionRegistry, EdgeLength, FacilityBand,
    LaneEdge, LaneGraph, LaneGroup, RoadCorridor, RoadSection, SeamNeighbor, SectionLane,
    SpeedLimit,
};

fn edge(id: &str, next: &[&str]) -> LaneEdge {
    LaneEdge::new(
        id,
        EdgeLength::try_new(10.0).expect("test edge length"),
        SpeedLimit::try_new(10.0).expect("test speed limit"),
        next.iter().copied(),
    )
}

/// 标准 fixture 图：section-a 两条双 edge lane、section-b 一条单 edge lane、
/// `e-x` 为不被任何 lane 覆盖的合法 edge。
fn graph() -> LaneGraph {
    LaneGraph::try_new([
        edge("e-a1", &["e-a2"]),
        edge("e-a2", &[]),
        edge("e-b1", &["e-b2"]),
        edge("e-b2", &[]),
        edge("e-c1", &[]),
        edge("e-x", &[]),
    ])
    .expect("test graph")
}

fn bands() -> Vec<FacilityBand> {
    vec![FacilityBand::new("band-m", "median")]
}

fn sections() -> Vec<RoadSection> {
    vec![
        RoadSection::new(
            "section-a",
            "motorLane",
            [
                SectionLane::new(["e-a1", "e-a2"], Some("group-a")),
                SectionLane::new(["e-b1", "e-b2"], None),
            ],
        ),
        RoadSection::new(
            "section-b",
            "nonMotorLane",
            [SectionLane::new(["e-c1"], None)],
        ),
    ]
}

fn groups() -> Vec<LaneGroup> {
    vec![LaneGroup::new("group-a", "section-a")]
}

fn corridors() -> Vec<RoadCorridor> {
    vec![RoadCorridor::new(
        "corridor-1",
        "section-a",
        [
            CorridorElementId::band("band-m"),
            CorridorElementId::section("section-a"),
            CorridorElementId::section("section-b"),
        ],
    )]
}

fn registry() -> CrossSectionRegistry {
    CrossSectionRegistry::try_new(&graph(), bands(), sections(), groups(), corridors())
        .expect("valid cross-section registry")
}

#[test]
fn registry_resolves_handles_and_external_ids_in_normalization_order() {
    let registry = registry();
    assert!(!registry.is_empty());

    let band_ids = registry
        .bands()
        .map(|handle| registry.band_external_id(handle).expect("band external ID"))
        .collect::<Vec<_>>();
    assert_eq!(band_ids, ["band-m"]);
    let section_ids = registry
        .sections()
        .map(|handle| {
            registry
                .section_external_id(handle)
                .expect("section external ID")
        })
        .collect::<Vec<_>>();
    assert_eq!(section_ids, ["section-a", "section-b"]);
    let group_ids = registry
        .groups()
        .map(|handle| {
            registry
                .group_external_id(handle)
                .expect("group external ID")
        })
        .collect::<Vec<_>>();
    assert_eq!(group_ids, ["group-a"]);
    let corridor_ids = registry
        .corridors()
        .map(|handle| {
            registry
                .corridor_external_id(handle)
                .expect("corridor external ID")
        })
        .collect::<Vec<_>>();
    assert_eq!(corridor_ids, ["corridor-1"]);

    let section_a = registry.section_handle("section-a").expect("section-a");
    assert_eq!(
        registry.section(section_a).map(RoadSection::kind_id),
        Some("motorLane")
    );
    assert!(registry.band_handle("missing").is_none());
    assert!(registry.section_handle("missing").is_none());
    assert!(registry.group_handle("missing").is_none());
    assert!(registry.corridor_handle("missing").is_none());
}

#[test]
fn empty_cross_section_is_valid() {
    let graph = graph();
    let registry =
        CrossSectionRegistry::try_new(&graph, Vec::new(), Vec::new(), Vec::new(), Vec::new())
            .expect("empty cross-section is valid");
    assert!(registry.is_empty());
    assert_eq!(registry.bands().count(), 0);
    assert_eq!(registry.sections().count(), 0);
}

#[test]
fn flat_structure_queries_expose_lanes_groups_elements_and_membership() {
    let registry = registry();
    let graph = graph();

    let section_a = registry.section_handle("section-a").expect("section-a");
    let lanes = registry
        .section_lanes(section_a)
        .expect("section-a lanes")
        .map(|(lane_index, edges)| {
            let edge_ids = edges
                .iter()
                .map(|edge| graph.edge_external_id(*edge).expect("edge external ID"))
                .collect::<Vec<_>>();
            (lane_index, edge_ids)
        })
        .collect::<Vec<_>>();
    assert_eq!(lanes.len(), 2);
    assert_eq!(lanes[0], (0, vec!["e-a1", "e-a2"]));
    assert_eq!(lanes[1], (1, vec!["e-b1", "e-b2"]));

    let group_a = registry.group_handle("group-a").expect("group-a");
    assert_eq!(registry.lane_group_section(group_a), Some(section_a));
    let member_lanes = registry
        .group_lanes(group_a)
        .expect("group-a member lanes")
        .collect::<Vec<_>>();
    assert_eq!(member_lanes, [0]);

    let corridor = registry.corridor_handle("corridor-1").expect("corridor-1");
    assert_eq!(
        registry.corridor_reference_section(corridor),
        Some(section_a)
    );
    let elements = registry
        .corridor_elements(corridor)
        .expect("corridor elements");
    let band_m = registry.band_handle("band-m").expect("band-m");
    let section_b = registry.section_handle("section-b").expect("section-b");
    assert_eq!(
        elements,
        [
            CorridorElement::Band(band_m),
            CorridorElement::Section(section_a),
            CorridorElement::Section(section_b),
        ]
    );

    // edge 反查：覆盖与未覆盖。
    let e_a2 = graph.edge_handle("e-a2").expect("e-a2");
    assert_eq!(registry.edge_lane_membership(e_a2), Some((section_a, 0)));
    let e_b1 = graph.edge_handle("e-b1").expect("e-b1");
    assert_eq!(registry.edge_lane_membership(e_b1), Some((section_a, 1)));
    let e_c1 = graph.edge_handle("e-c1").expect("e-c1");
    assert_eq!(registry.edge_lane_membership(e_c1), Some((section_b, 0)));
    let e_x = graph.edge_handle("e-x").expect("e-x");
    assert_eq!(registry.edge_lane_membership(e_x), None);

    // 越界 handle 返回 None（不 panic）。
    let bigger_graph = LaneGraph::try_new(
        (0..10)
            .map(|i| edge(&format!("big-{i}"), &[]))
            .collect::<Vec<_>>(),
    )
    .expect("bigger graph");
    let out_of_range = bigger_graph.edge_handle("big-9").expect("big-9");
    assert_eq!(registry.edge_lane_membership(out_of_range), None);
}

// ---------- phase 3：FacilityBand ----------

#[test]
fn phase3_band_identity_and_kind_errors() {
    let graph = graph();

    // ID syntax。
    let error = CrossSectionRegistry::try_new(
        &graph,
        [FacilityBand::new("bad id", "median")],
        Vec::new(),
        Vec::new(),
        Vec::new(),
    )
    .expect_err("malformed band id must fail");
    std::assert_matches!(
        error,
        CoreError::InvalidExternalId { field, external_id, .. }
            if field == "facilityBands[].id" && external_id == "bad id"
    );

    // duplicate。
    let error = CrossSectionRegistry::try_new(
        &graph,
        [
            FacilityBand::new("band-m", "median"),
            FacilityBand::new("band-m", "sidewalk"),
        ],
        Vec::new(),
        Vec::new(),
        Vec::new(),
    )
    .expect_err("duplicate band id must fail");
    std::assert_matches!(
        error,
        CoreError::DuplicateFacilityBandId { band_id } if band_id == "band-m"
    );

    // unknown kindId。
    let error = CrossSectionRegistry::try_new(
        &graph,
        [FacilityBand::new("band-m", "busLane")],
        Vec::new(),
        Vec::new(),
        Vec::new(),
    )
    .expect_err("unknown band kind must fail");
    std::assert_matches!(
        error,
        CoreError::UnknownFacilityKind { kind } if kind == "busLane"
    );

    // 类别错误：band 必须 non-traversable。
    let error = CrossSectionRegistry::try_new(
        &graph,
        [FacilityBand::new("band-m", "motorLane")],
        Vec::new(),
        Vec::new(),
        Vec::new(),
    )
    .expect_err("lane-bearing band kind must fail");
    std::assert_matches!(
        error,
        CoreError::FacilityBandKindNotNonTraversable { band_id, kind_id }
            if band_id == "band-m" && kind_id == "motorLane"
    );

    // 类顺序：identity（syntax/duplicate）先于 kind 解析。
    let error = CrossSectionRegistry::try_new(
        &graph,
        [
            FacilityBand::new("band-a", "busLane"),
            FacilityBand::new("band-a", "median"),
        ],
        Vec::new(),
        Vec::new(),
        Vec::new(),
    )
    .expect_err("duplicate id must precede kind errors");
    std::assert_matches!(error, CoreError::DuplicateFacilityBandId { .. });
}

// ---------- phase 4：RoadSection identity ----------

#[test]
fn phase4_section_identity_precedes_group_parent_resolution() {
    let graph = graph();

    let error = CrossSectionRegistry::try_new(
        &graph,
        Vec::new(),
        [RoadSection::new(
            "bad id",
            "motorLane",
            [SectionLane::new(["e-c1"], None)],
        )],
        Vec::new(),
        Vec::new(),
    )
    .expect_err("malformed section id must fail");
    std::assert_matches!(
        error,
        CoreError::InvalidExternalId { field, external_id, .. }
            if field == "roadSections[].id" && external_id == "bad id"
    );

    let error = CrossSectionRegistry::try_new(
        &graph,
        Vec::new(),
        [
            RoadSection::new("section-a", "motorLane", [SectionLane::new(["e-c1"], None)]),
            RoadSection::new("section-a", "motorLane", [SectionLane::new(["e-b1"], None)]),
        ],
        Vec::new(),
        Vec::new(),
    )
    .expect_err("duplicate section id must fail");
    std::assert_matches!(
        error,
        CoreError::DuplicateRoadSectionId { section_id } if section_id == "section-a"
    );

    // section identity 错误先于 LaneGroup 的 unknown parent。
    let error = CrossSectionRegistry::try_new(
        &graph,
        Vec::new(),
        [RoadSection::new(
            "bad id",
            "motorLane",
            [SectionLane::new(["e-c1"], None)],
        )],
        [LaneGroup::new("group-a", "missing")],
        Vec::new(),
    )
    .expect_err("section identity must fail first");
    std::assert_matches!(error, CoreError::InvalidExternalId { .. });
}

// ---------- phase 5：LaneGroup identity ----------

#[test]
fn phase5_group_identity_and_parent_resolution() {
    let graph = graph();

    let error = CrossSectionRegistry::try_new(
        &graph,
        Vec::new(),
        Vec::new(),
        [LaneGroup::new("bad id", "section-a")],
        Vec::new(),
    )
    .expect_err("malformed group id must fail");
    std::assert_matches!(
        error,
        CoreError::InvalidExternalId { field, external_id, .. }
            if field == "laneGroups[].id" && external_id == "bad id"
    );

    let error = CrossSectionRegistry::try_new(
        &graph,
        Vec::new(),
        [
            RoadSection::new("section-a", "motorLane", [SectionLane::new(["e-c1"], None)]),
            RoadSection::new("section-b", "motorLane", [SectionLane::new(["e-b1"], None)]),
        ],
        [
            LaneGroup::new("group-a", "section-a"),
            LaneGroup::new("group-a", "section-b"),
        ],
        Vec::new(),
    )
    .expect_err("duplicate group id must fail");
    std::assert_matches!(
        error,
        CoreError::DuplicateLaneGroupId { group_id } if group_id == "group-a"
    );

    let error = CrossSectionRegistry::try_new(
        &graph,
        Vec::new(),
        [RoadSection::new(
            "section-a",
            "motorLane",
            [SectionLane::new(["e-c1"], None)],
        )],
        [LaneGroup::new("group-a", "missing")],
        Vec::new(),
    )
    .expect_err("unknown group parent must fail");
    std::assert_matches!(
        error,
        CoreError::UnknownLaneGroupRoadSection { group_id, section_id }
            if group_id == "group-a" && section_id == "missing"
    );

    // phase 5 先于 phase 6 body：group unknown parent 先于 section 的 unknown edge。
    let error = CrossSectionRegistry::try_new(
        &graph,
        Vec::new(),
        [RoadSection::new(
            "section-a",
            "motorLane",
            [SectionLane::new(["missing"], None)],
        )],
        [LaneGroup::new("group-a", "missing")],
        Vec::new(),
    )
    .expect_err("group parent error must precede section body errors");
    std::assert_matches!(error, CoreError::UnknownLaneGroupRoadSection { .. });
}

// ---------- phase 6：RoadSection body ----------

#[test]
fn phase6_section_kind_and_lane_shape_errors() {
    let graph = graph();

    // unknown kindId。
    let error = CrossSectionRegistry::try_new(
        &graph,
        Vec::new(),
        [RoadSection::new(
            "section-a",
            "busLane",
            [SectionLane::new(["e-c1"], None)],
        )],
        Vec::new(),
        Vec::new(),
    )
    .expect_err("unknown section kind must fail");
    std::assert_matches!(
        error,
        CoreError::UnknownFacilityKind { kind } if kind == "busLane"
    );

    // 类别错误：section 必须 lane-bearing。
    let error = CrossSectionRegistry::try_new(
        &graph,
        Vec::new(),
        [RoadSection::new(
            "section-a",
            "sidewalk",
            [SectionLane::new(["e-c1"], None)],
        )],
        Vec::new(),
        Vec::new(),
    )
    .expect_err("non-lane-bearing section kind must fail");
    std::assert_matches!(
        error,
        CoreError::RoadSectionKindNotLaneBearing { section_id, kind_id }
            if section_id == "section-a" && kind_id == "sidewalk"
    );

    // empty lanes。
    let error = CrossSectionRegistry::try_new(
        &graph,
        Vec::new(),
        [RoadSection::new("section-a", "motorLane", Vec::new())],
        Vec::new(),
        Vec::new(),
    )
    .expect_err("empty section lanes must fail");
    std::assert_matches!(
        error,
        CoreError::EmptyRoadSectionLanes { section_id } if section_id == "section-a"
    );

    // 类顺序：kind 错误先于 empty lanes（即使在 input order 更靠后的 section 上）。
    let error = CrossSectionRegistry::try_new(
        &graph,
        Vec::new(),
        [
            RoadSection::new("section-empty", "motorLane", Vec::new()),
            RoadSection::new(
                "section-bad-kind",
                "sidewalk",
                [SectionLane::new(["e-c1"], None)],
            ),
        ],
        Vec::new(),
        Vec::new(),
    )
    .expect_err("kind error must precede empty lanes");
    std::assert_matches!(
        error,
        CoreError::RoadSectionKindNotLaneBearing { section_id, .. }
            if section_id == "section-bad-kind"
    );

    // empty lane chain。
    let error = CrossSectionRegistry::try_new(
        &graph,
        Vec::new(),
        [RoadSection::new(
            "section-a",
            "motorLane",
            [
                SectionLane::new(["e-c1"], None),
                SectionLane::new(Vec::<String>::new(), None),
            ],
        )],
        Vec::new(),
        Vec::new(),
    )
    .expect_err("empty lane chain must fail");
    std::assert_matches!(
        error,
        CoreError::EmptySectionLaneChain { section_id, lane_index }
            if section_id == "section-a" && lane_index == 1
    );
}

#[test]
fn phase6_lane_edge_resolution_connectivity_and_uniqueness() {
    let graph = graph();

    // unknown edge。
    let error = CrossSectionRegistry::try_new(
        &graph,
        Vec::new(),
        [RoadSection::new(
            "section-a",
            "motorLane",
            [SectionLane::new(["e-c1", "missing"], None)],
        )],
        Vec::new(),
        Vec::new(),
    )
    .expect_err("unknown lane edge must fail");
    std::assert_matches!(
        error,
        CoreError::UnknownSectionLaneEdge { section_id, lane_index, edge_id }
            if section_id == "section-a" && lane_index == 0 && edge_id == "missing"
    );

    // disconnected transition。
    let error = CrossSectionRegistry::try_new(
        &graph,
        Vec::new(),
        [RoadSection::new(
            "section-a",
            "motorLane",
            [SectionLane::new(["e-a1", "e-b2"], None)],
        )],
        Vec::new(),
        Vec::new(),
    )
    .expect_err("disconnected lane chain must fail");
    std::assert_matches!(
        error,
        CoreError::DisconnectedSectionLane {
            section_id,
            lane_index,
            transition_index,
            from_edge_id,
            to_edge_id,
        } if section_id == "section-a" && lane_index == 0 && transition_index == 0
            && from_edge_id == "e-a1" && to_edge_id == "e-b2"
    );

    // 链内 edge 重复（链连通但自重复）。
    let loop_graph = LaneGraph::try_new([edge("loop-1", &["loop-2"]), edge("loop-2", &["loop-1"])])
        .expect("loop graph");
    let error = CrossSectionRegistry::try_new(
        &loop_graph,
        Vec::new(),
        [RoadSection::new(
            "section-a",
            "motorLane",
            [SectionLane::new(["loop-1", "loop-2", "loop-1"], None)],
        )],
        Vec::new(),
        Vec::new(),
    )
    .expect_err("duplicate edge within lane chain must fail");
    std::assert_matches!(
        error,
        CoreError::DuplicateSectionLaneEdge { section_id, lane_index, edge_id }
            if section_id == "section-a" && lane_index == 0 && edge_id == "loop-1"
    );

    // 同一 edge 出现在同 section 的多条 lane。
    let error = CrossSectionRegistry::try_new(
        &graph,
        Vec::new(),
        [RoadSection::new(
            "section-a",
            "motorLane",
            [
                SectionLane::new(["e-a1"], None),
                SectionLane::new(["e-a1"], None),
            ],
        )],
        Vec::new(),
        Vec::new(),
    )
    .expect_err("edge claimed by multiple lanes must fail");
    std::assert_matches!(
        error,
        CoreError::SectionLaneEdgeClaimConflict {
            edge_id,
            first_section_id,
            first_lane_index,
            duplicate_section_id,
            duplicate_lane_index,
        } if edge_id == "e-a1" && first_section_id == "section-a" && first_lane_index == 0
            && duplicate_section_id == "section-a" && duplicate_lane_index == 1
    );

    // 同一 edge 出现在多个 section。
    let error = CrossSectionRegistry::try_new(
        &graph,
        Vec::new(),
        [
            RoadSection::new("section-a", "motorLane", [SectionLane::new(["e-a1"], None)]),
            RoadSection::new("section-b", "motorLane", [SectionLane::new(["e-a1"], None)]),
        ],
        Vec::new(),
        Vec::new(),
    )
    .expect_err("edge claimed by multiple sections must fail");
    std::assert_matches!(
        error,
        CoreError::SectionLaneEdgeClaimConflict {
            first_section_id,
            duplicate_section_id,
            ..
        } if first_section_id == "section-a" && duplicate_section_id == "section-b"
    );
}

#[test]
fn phase6_lane_group_reference_errors() {
    let graph = graph();

    // unknown laneGroupId。
    let error = CrossSectionRegistry::try_new(
        &graph,
        Vec::new(),
        [RoadSection::new(
            "section-a",
            "motorLane",
            [SectionLane::new(["e-c1"], Some("missing"))],
        )],
        Vec::new(),
        Vec::new(),
    )
    .expect_err("unknown lane group must fail");
    std::assert_matches!(
        error,
        CoreError::UnknownSectionLaneGroup { section_id, lane_index, group_id }
            if section_id == "section-a" && lane_index == 0 && group_id == "missing"
    );

    // lane 引用的 group 属于另一个 section。
    let error = CrossSectionRegistry::try_new(
        &graph,
        Vec::new(),
        [
            RoadSection::new(
                "section-a",
                "motorLane",
                [SectionLane::new(["e-a1"], Some("group-a"))],
            ),
            RoadSection::new(
                "section-b",
                "motorLane",
                [SectionLane::new(["e-c1"], Some("group-a"))],
            ),
        ],
        [LaneGroup::new("group-a", "section-a")],
        Vec::new(),
    )
    .expect_err("cross-section group reference must fail");
    std::assert_matches!(
        error,
        CoreError::SectionLaneGroupSectionMismatch {
            section_id,
            lane_index,
            group_id,
            group_section_id,
        } if section_id == "section-b" && lane_index == 0
            && group_id == "group-a" && group_section_id == "section-a"
    );
}

// ---------- phase 7：LaneGroup membership ----------

#[test]
fn phase7_empty_group_is_rejected() {
    let graph = graph();
    let error = CrossSectionRegistry::try_new(
        &graph,
        Vec::new(),
        [RoadSection::new(
            "section-a",
            "motorLane",
            [SectionLane::new(["e-c1"], None)],
        )],
        [LaneGroup::new("group-a", "section-a")],
        Vec::new(),
    )
    .expect_err("empty group must fail");
    std::assert_matches!(
        error,
        CoreError::EmptyLaneGroup { group_id } if group_id == "group-a"
    );
}

// ---------- phase 8：RoadCorridor ----------

#[test]
fn phase8_corridor_identity_and_empty_elements() {
    let graph = graph();

    let error = CrossSectionRegistry::try_new(
        &graph,
        bands(),
        sections(),
        groups(),
        [RoadCorridor::new(
            "bad id",
            "section-a",
            [CorridorElementId::band("band-m")],
        )],
    )
    .expect_err("malformed corridor id must fail");
    std::assert_matches!(
        error,
        CoreError::InvalidExternalId { field, external_id, .. }
            if field == "roadCorridors[].id" && external_id == "bad id"
    );

    let error = CrossSectionRegistry::try_new(
        &graph,
        bands(),
        sections(),
        groups(),
        [
            RoadCorridor::new(
                "corridor-1",
                "section-a",
                [CorridorElementId::band("band-m")],
            ),
            RoadCorridor::new(
                "corridor-1",
                "section-a",
                [CorridorElementId::section("section-a")],
            ),
        ],
    )
    .expect_err("duplicate corridor id must fail");
    std::assert_matches!(
        error,
        CoreError::DuplicateRoadCorridorId { corridor_id } if corridor_id == "corridor-1"
    );

    // empty elements 先于一切 element 依赖检查（unknown element 在更前的 corridor 上也不例外）。
    let error = CrossSectionRegistry::try_new(
        &graph,
        bands(),
        sections(),
        groups(),
        [
            RoadCorridor::new(
                "corridor-1",
                "section-a",
                [CorridorElementId::section("missing")],
            ),
            RoadCorridor::new("corridor-2", "section-a", Vec::new()),
        ],
    )
    .expect_err("empty elements must fail first");
    std::assert_matches!(
        error,
        CoreError::EmptyRoadCorridorElements { corridor_id } if corridor_id == "corridor-2"
    );

    // empty elements 先于 reference 成员性。
    let error = CrossSectionRegistry::try_new(
        &graph,
        bands(),
        sections(),
        groups(),
        [RoadCorridor::new("corridor-1", "missing", Vec::new())],
    )
    .expect_err("empty elements must precede reference checks");
    std::assert_matches!(error, CoreError::EmptyRoadCorridorElements { .. });
}

#[test]
fn phase8_corridor_element_reference_errors() {
    let graph = graph();

    // unknown element（section 与 band 两类归因）。
    let error = CrossSectionRegistry::try_new(
        &graph,
        bands(),
        sections(),
        groups(),
        [RoadCorridor::new(
            "corridor-1",
            "section-a",
            [
                CorridorElementId::band("band-m"),
                CorridorElementId::section("missing"),
            ],
        )],
    )
    .expect_err("unknown section element must fail");
    std::assert_matches!(
        error,
        CoreError::UnknownCorridorElement { corridor_id, element_kind, element_id }
            if corridor_id == "corridor-1" && element_kind == "section" && element_id == "missing"
    );

    let error = CrossSectionRegistry::try_new(
        &graph,
        bands(),
        sections(),
        groups(),
        [RoadCorridor::new(
            "corridor-1",
            "section-a",
            [
                CorridorElementId::section("section-a"),
                CorridorElementId::band("missing"),
            ],
        )],
    )
    .expect_err("unknown band element must fail");
    std::assert_matches!(
        error,
        CoreError::UnknownCorridorElement { element_kind, element_id, .. }
            if element_kind == "band" && element_id == "missing"
    );

    // elements 内重复引用同一 section/band。
    let error = CrossSectionRegistry::try_new(
        &graph,
        bands(),
        sections(),
        groups(),
        [RoadCorridor::new(
            "corridor-1",
            "section-a",
            [
                CorridorElementId::section("section-a"),
                CorridorElementId::section("section-a"),
            ],
        )],
    )
    .expect_err("duplicate element within corridor must fail");
    std::assert_matches!(
        error,
        CoreError::DuplicateCorridorElement { corridor_id, element_kind, element_id }
            if corridor_id == "corridor-1" && element_kind == "section" && element_id == "section-a"
    );

    // 同一 section 出现在多个 corridor。
    let error = CrossSectionRegistry::try_new(
        &graph,
        bands(),
        sections(),
        groups(),
        [
            RoadCorridor::new(
                "corridor-1",
                "section-a",
                [CorridorElementId::section("section-a")],
            ),
            RoadCorridor::new(
                "corridor-2",
                "section-a",
                [CorridorElementId::section("section-a")],
            ),
        ],
    )
    .expect_err("section claimed by multiple corridors must fail");
    std::assert_matches!(
        error,
        CoreError::CorridorElementMultipleOwners {
            element_kind,
            element_id,
            first_corridor_id,
            duplicate_corridor_id,
        } if element_kind == "section" && element_id == "section-a"
            && first_corridor_id == "corridor-1" && duplicate_corridor_id == "corridor-2"
    );
}

#[test]
fn phase8_complete_owner_tree_and_reference_membership() {
    let graph = graph();

    // 零归属：section/band 必须恰好属于一个 corridor。band 先于 section 归因。
    let error = CrossSectionRegistry::try_new(
        &graph,
        bands(),
        sections(),
        groups(),
        [RoadCorridor::new(
            "corridor-1",
            "section-a",
            [CorridorElementId::section("section-a")],
        )],
    )
    .expect_err("unowned elements must fail");
    std::assert_matches!(
        error,
        CoreError::UnownedCorridorElement { element_kind, element_id }
            if element_kind == "band" && element_id == "band-m"
    );

    let error = CrossSectionRegistry::try_new(
        &graph,
        bands(),
        sections(),
        groups(),
        [RoadCorridor::new(
            "corridor-1",
            "section-a",
            [
                CorridorElementId::band("band-m"),
                CorridorElementId::section("section-a"),
            ],
        )],
    )
    .expect_err("unowned section must fail");
    std::assert_matches!(
        error,
        CoreError::UnownedCorridorElement { element_kind, element_id }
            if element_kind == "section" && element_id == "section-b"
    );

    // referenceSectionId 不是成员 section（零归属先于 reference 检查，
    // 因此用第二个 corridor 收纳 section-b）。
    let error = CrossSectionRegistry::try_new(
        &graph,
        bands(),
        sections(),
        groups(),
        [
            RoadCorridor::new(
                "corridor-1",
                "section-b",
                [
                    CorridorElementId::band("band-m"),
                    CorridorElementId::section("section-a"),
                ],
            ),
            RoadCorridor::new(
                "corridor-2",
                "section-b",
                [CorridorElementId::section("section-b")],
            ),
        ],
    )
    .expect_err("non-member reference must fail");
    std::assert_matches!(
        error,
        CoreError::CorridorReferenceSectionNotMember { corridor_id, reference_section_id }
            if corridor_id == "corridor-1" && reference_section_id == "section-b"
    );

    // referenceSectionId 未声明同样按“非成员”归因。
    let error = CrossSectionRegistry::try_new(
        &graph,
        bands(),
        sections(),
        groups(),
        [RoadCorridor::new(
            "corridor-1",
            "missing",
            [
                CorridorElementId::band("band-m"),
                CorridorElementId::section("section-a"),
                CorridorElementId::section("section-b"),
            ],
        )],
    )
    .expect_err("undeclared reference must fail as non-member");
    std::assert_matches!(
        error,
        CoreError::CorridorReferenceSectionNotMember { reference_section_id, .. }
            if reference_section_id == "missing"
    );
}

// ---------- 接缝派生（SSOT §3.2.1） ----------

#[test]
fn seam_neighbors_derive_from_element_and_lane_index_order() {
    let registry = registry();
    let corridor = registry.corridor_handle("corridor-1").expect("corridor-1");
    let band_m = registry.band_handle("band-m").expect("band-m");
    let section_a = registry.section_handle("section-a").expect("section-a");
    let section_b = registry.section_handle("section-b").expect("section-b");

    // corridor-1 = [band-m, section-a, section-b]。
    // band-section 接缝：band 作为非遍历侧，右侧 section 贡献 index 0 lane。
    let (left, right) = registry
        .corridor_seam_neighbors(corridor, 1)
        .expect("seam 1 must exist");
    assert_eq!(left, SeamNeighbor::Band(band_m));
    assert_eq!(
        right,
        SeamNeighbor::OutermostLane {
            section: section_a,
            lane_index: 0,
        }
    );

    // section-section 接缝：左侧贡献最大 index lane，右侧贡献 index 0 lane。
    let (left, right) = registry
        .corridor_seam_neighbors(corridor, 2)
        .expect("seam 2 must exist");
    assert_eq!(
        left,
        SeamNeighbor::OutermostLane {
            section: section_a,
            lane_index: 1,
        }
    );
    assert_eq!(
        right,
        SeamNeighbor::OutermostLane {
            section: section_b,
            lane_index: 0,
        }
    );

    // 越界：j = 0 与 j = elements.len() 无接缝。
    assert_eq!(registry.corridor_seam_neighbors(corridor, 0), None);
    assert_eq!(registry.corridor_seam_neighbors(corridor, 3), None);
}

#[test]
fn seam_neighbors_are_pure_index_semantics_regardless_of_travel_direction() {
    // 反向 section：lane index 按 corridor reference 方向声明，与其自身行驶
    // 方向无关；派生只读 index 顺序，不需要任何方向数据（SSOT §3.2/§3.2.1）。
    // corridor-2 = [section-b, section-a, band-m]（reference = section-b）。
    let registry = CrossSectionRegistry::try_new(
        &graph(),
        bands(),
        sections(),
        groups(),
        [RoadCorridor::new(
            "corridor-2",
            "section-b",
            [
                CorridorElementId::section("section-b"),
                CorridorElementId::section("section-a"),
                CorridorElementId::band("band-m"),
            ],
        )],
    )
    .expect("valid reversed corridor");

    let corridor = registry.corridor_handle("corridor-2").expect("corridor-2");
    let band_m = registry.band_handle("band-m").expect("band-m");
    let section_a = registry.section_handle("section-a").expect("section-a");
    let section_b = registry.section_handle("section-b").expect("section-b");

    let (left, right) = registry
        .corridor_seam_neighbors(corridor, 1)
        .expect("seam 1 must exist");
    assert_eq!(
        left,
        SeamNeighbor::OutermostLane {
            section: section_b,
            lane_index: 0,
        }
    );
    assert_eq!(
        right,
        SeamNeighbor::OutermostLane {
            section: section_a,
            lane_index: 0,
        }
    );

    // section-band 接缝：左侧 section 贡献最大 index lane。
    let (left, right) = registry
        .corridor_seam_neighbors(corridor, 2)
        .expect("seam 2 must exist");
    assert_eq!(
        left,
        SeamNeighbor::OutermostLane {
            section: section_a,
            lane_index: 1,
        }
    );
    assert_eq!(right, SeamNeighbor::Band(band_m));
}

// ---------- permutation / rebind ----------

/// 以 external ID 对齐提取 registry 语义快照，用于 permutation/rebind 等价比较。
fn semantics_snapshot(
    registry: &CrossSectionRegistry,
    graph: &LaneGraph,
) -> (
    Vec<(String, Vec<Vec<String>>)>,
    Vec<(String, Option<(String, usize)>)>,
    Vec<(String, Vec<String>)>,
) {
    let sections = registry
        .sections()
        .map(|section| {
            let id = registry
                .section_external_id(section)
                .expect("section external ID")
                .to_owned();
            let lanes = registry
                .section_lanes(section)
                .expect("section lanes")
                .map(|(_, edges)| {
                    edges
                        .iter()
                        .map(|edge| {
                            graph
                                .edge_external_id(*edge)
                                .expect("edge external ID")
                                .to_owned()
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            (id, lanes)
        })
        .collect::<Vec<_>>();

    let memberships = graph
        .edges()
        .map(|edge| {
            let edge_id = edge.id().to_owned();
            let handle = graph.edge_handle(edge.id()).expect("edge handle");
            let membership = registry
                .edge_lane_membership(handle)
                .map(|(section, lane_index)| {
                    (
                        registry
                            .section_external_id(section)
                            .expect("section external ID")
                            .to_owned(),
                        lane_index,
                    )
                });
            (edge_id, membership)
        })
        .collect::<Vec<_>>();

    let corridors = registry
        .corridors()
        .map(|corridor| {
            let id = registry
                .corridor_external_id(corridor)
                .expect("corridor external ID")
                .to_owned();
            let elements = registry
                .corridor_elements(corridor)
                .expect("corridor elements")
                .iter()
                .map(|element| match element {
                    CorridorElement::Section(section) => registry
                        .section_external_id(*section)
                        .expect("section external ID")
                        .to_owned(),
                    CorridorElement::Band(band) => registry
                        .band_external_id(*band)
                        .expect("band external ID")
                        .to_owned(),
                })
                .collect::<Vec<_>>();
            (id, elements)
        })
        .collect::<Vec<_>>();

    (sections, memberships, corridors)
}

#[test]
fn input_permutation_preserves_external_id_aligned_semantics() {
    let canonical = registry();

    let mut permuted_bands = bands();
    let mut permuted_sections = sections();
    let mut permuted_groups = groups();
    let mut permuted_corridors = corridors();
    permuted_bands.reverse();
    permuted_sections.reverse();
    permuted_groups.reverse();
    permuted_corridors.reverse();
    let permuted = CrossSectionRegistry::try_new(
        &graph(),
        permuted_bands,
        permuted_sections,
        permuted_groups,
        permuted_corridors,
    )
    .expect("permuted registry must be valid");

    // normalization order 本身随 input permutation 改变；按 external ID 排序后比较。
    let (mut canonical_sections, canonical_memberships, mut canonical_corridors) =
        semantics_snapshot(&canonical, &graph());
    let (mut permuted_sections, permuted_memberships, mut permuted_corridors) =
        semantics_snapshot(&permuted, &graph());
    canonical_sections.sort();
    permuted_sections.sort();
    canonical_corridors.sort();
    permuted_corridors.sort();
    assert_eq!(canonical_sections, permuted_sections);
    assert_eq!(canonical_memberships, permuted_memberships);
    assert_eq!(canonical_corridors, permuted_corridors);
}

#[test]
fn rebind_to_lane_graph_reproduces_semantics_and_validates_new_graph() {
    let registry = registry();

    // 同 edge 集合、不同声明顺序的新 graph：handle 数值可变，语义等价。
    let reordered_graph = LaneGraph::try_new([
        edge("e-x", &[]),
        edge("e-c1", &[]),
        edge("e-b2", &[]),
        edge("e-b1", &["e-b2"]),
        edge("e-a2", &[]),
        edge("e-a1", &["e-a2"]),
    ])
    .expect("reordered graph");
    let rebound = registry
        .rebind_to_lane_graph(&reordered_graph)
        .expect("rebind to reordered graph must succeed");

    let canonical_snapshot = semantics_snapshot(&registry, &graph());
    let rebound_snapshot = semantics_snapshot(&rebound, &reordered_graph);
    // membership 按 graph 声明顺序排列，排序后比较。
    let (canonical_sections, mut canonical_memberships, canonical_corridors) = canonical_snapshot;
    let (rebound_sections, mut rebound_memberships, rebound_corridors) = rebound_snapshot;
    canonical_memberships.sort();
    rebound_memberships.sort();
    assert_eq!(canonical_sections, rebound_sections);
    assert_eq!(canonical_memberships, rebound_memberships);
    assert_eq!(canonical_corridors, rebound_corridors);

    // 缺失 lane edge 的 graph：rebind 必须按新 graph 重新校验。
    let reduced_graph = LaneGraph::try_new([
        edge("e-a1", &["e-a2"]),
        edge("e-a2", &[]),
        edge("e-b1", &[]),
        edge("e-c1", &[]),
        edge("e-x", &[]),
    ])
    .expect("reduced graph");
    let error = registry
        .rebind_to_lane_graph(&reduced_graph)
        .expect_err("rebind against graph missing lane edge must fail");
    std::assert_matches!(
        error,
        CoreError::UnknownSectionLaneEdge { edge_id, .. } if edge_id == "e-b2"
    );
}

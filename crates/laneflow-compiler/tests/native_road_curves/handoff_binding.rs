use super::*;
use laneflow_static_contract::{
    LaneEdgeOrdinal, NetworkRevisionId, RoadCorridorOrdinal, Sha256Digest,
};
use laneflow_static_network::vehicle::{
    Failure, HandoffSourceEvidence, HandoffSourceModule, HandoffWindow,
};
use laneflow_static_network::{BoundaryOwner, CorridorElement, StationQuery};

use std::ops::Bound::{Excluded, Included};

fn network(
    direction: RoadEditingLaneDirection,
    corridor_cuts: &[f64],
    lane_cuts: [&[f64]; 2],
    coverage: CrossSectionCoverage,
) -> std::sync::Arc<SharedNetworkRevision> {
    let road = Program::try_new(p(0., 0., 0.), vec![Segment::line(p(0., 0., 50.))]).unwrap();
    let source = save_lane_declarations(
        road,
        corridor_cuts,
        (
            GeometryAccuracyProfile::Fine2Cm,
            GeometryDirectionProfile::Smooth1Deg,
        ),
        [direction; 2],
        coverage,
        0,
        SectionFixture {
            lane_cuts_m: lane_cuts,
            ..SectionFixture::default()
        },
    );
    load_network(
        &publish(&compile_saved(&source).unwrap(), "bind-handoff"),
        SpatialBuildOption::Omit,
    )
}

#[test]
fn handoff_window_binds_the_two_occurrence_edges_at_independent_directional_cuts() {
    for (direction, indices, common, excluded) in [
        (
            RoadEditingLaneDirection::Forward,
            [1, 0],
            [Included(22_000), Excluded(28_000)],
            28_000,
        ),
        (
            RoadEditingLaneDirection::Backward,
            [0, 1],
            [Excluded(22_000), Included(28_000)],
            22_000,
        ),
    ] {
        let n = network(
            direction,
            &[],
            [&[22.], &[28.]],
            CrossSectionCoverage::Complete,
        );
        let revision = n.network_revision();
        let corridor = RoadCorridorOrdinal::from_raw(0);
        let relations = n.traffic().relations();
        let CorridorElement::RoadSection(section) =
            relations.corridor_elements(corridor).unwrap()[0]
        else {
            unreachable!()
        };
        let lanes = relations.section_lanes(section).unwrap();
        let edges = [0, 1].map(|i| relations.authoring_edge_chain(lanes[i]).unwrap()[indices[i]]);
        let window = HandoffWindow {
            edges,
            owner: BoundaryOwner::Section(section),
            left_index: 0,
            stations_mm: [0, 50_000],
        };
        for ordered_edges in [edges, [edges[1], edges[0]]] {
            let window = HandoffWindow {
                edges: ordered_edges,
                ..window
            };
            let binding = n.bind_handoff_window(revision, window).unwrap();
            assert_eq!(binding.revision(), revision);
            assert_eq!(binding.window(), window);
            assert_eq!(binding.corridor(), corridor);
            assert_eq!(
                binding.frame(),
                n.station_mapping().surface_spans(corridor)[0].frame
            );
            assert_eq!(
                binding.increasing(),
                direction == RoadEditingLaneDirection::Forward
            );
            assert_eq!(binding.common_stations_mm(), common);
        }
        let owned_mm = if direction == RoadEditingLaneDirection::Forward {
            [22_000, 27_999]
        } else {
            [22_001, 28_000]
        };
        for station_mm in owned_mm {
            let cut = HandoffWindow {
                stations_mm: [station_mm; 2],
                ..window
            };
            assert_eq!(
                n.bind_handoff_window(revision, cut)
                    .unwrap()
                    .common_stations_mm(),
                [Included(station_mm); 2]
            );
        }
        // 连续求证仍覆盖物理边拥有的最后一段：上游紧邻内切点，切点本身属于下游。
        for station in owned_mm {
            for (&lane, edge) in lanes.iter().zip(edges) {
                let StationQuery::Complete(found) =
                    n.station_mapping().at_station(corridor, station, lane)
                else {
                    panic!("missing address at {station}")
                };
                assert!(found.addresses().map(|a| a.edge).eq([edge]));
            }
        }
        let invalid = |window| n.bind_handoff_window(revision, window).unwrap_err();
        assert_eq!(
            invalid(HandoffWindow {
                stations_mm: [excluded; 2],
                ..window
            }),
            Failure::Binding
        );
        for (station, index) in [
            (
                0,
                usize::from(direction == RoadEditingLaneDirection::Backward),
            ),
            (
                50_000,
                usize::from(direction == RoadEditingLaneDirection::Forward),
            ),
        ] {
            let edges = [0, 1].map(|i| relations.authoring_edge_chain(lanes[i]).unwrap()[index]);
            let terminal = HandoffWindow {
                edges,
                stations_mm: [station; 2],
                ..window
            };
            assert_eq!(
                n.bind_handoff_window(revision, terminal)
                    .unwrap()
                    .common_stations_mm(),
                [Included(station); 2]
            );
        }
        if direction == RoadEditingLaneDirection::Backward {
            // 两个相邻区段最靠近边界的车道方向相反，不能绑定为同向换道。
            let CorridorElement::RoadSection(reference) =
                relations.corridor_elements(corridor).unwrap()[1]
            else {
                unreachable!()
            };
            let opposite = relations.section_lanes(reference).unwrap()[0];
            assert_eq!(
                invalid(HandoffWindow {
                    owner: BoundaryOwner::Corridor(corridor),
                    edges: [
                        edges[1],
                        relations.authoring_edge_chain(opposite).unwrap()[0]
                    ],
                    ..window
                }),
                Failure::Binding
            );
        }
    }
}

#[test]
fn handoff_window_rejects_other_corridor_with_matching_local_stations() {
    let n = network(
        RoadEditingLaneDirection::Forward,
        &[25.],
        [&[]; 2],
        CrossSectionCoverage::Complete,
    );
    let relations = n.traffic().relations();
    let sections = [0, 1].map(|i| {
        let CorridorElement::RoadSection(s) = relations
            .corridor_elements(RoadCorridorOrdinal::from_raw(i))
            .unwrap()[0]
        else {
            unreachable!()
        };
        s
    });
    let edges = [0, 1].map(|i| {
        let lane = relations.section_lanes(sections[i]).unwrap()[i];
        relations.authoring_edge_chain(lane).unwrap()[0]
    });
    let window = HandoffWindow {
        edges,
        stations_mm: [5_000, 10_000],
        owner: BoundaryOwner::Section(sections[0]),
        left_index: 0,
    };
    assert_eq!(
        n.bind_handoff_window(n.network_revision(), window),
        Err(Failure::Binding)
    );
}

#[test]
fn handoff_window_requires_known_revision_boundary_surface_and_ordered_stations() {
    let corridor = RoadCorridorOrdinal::from_raw(0);
    for coverage in [
        CrossSectionCoverage::StationMapping,
        CrossSectionCoverage::Complete,
    ] {
        let n = network(RoadEditingLaneDirection::Forward, &[], [&[]; 2], coverage);
        let r = n.traffic().relations();
        let CorridorElement::RoadSection(section) = r.corridor_elements(corridor).unwrap()[0]
        else {
            unreachable!()
        };
        let lanes = r.section_lanes(section).unwrap();
        let edges = [0, 1].map(|i| r.authoring_edge_chain(lanes[i]).unwrap()[0]);
        let window = HandoffWindow {
            edges,
            owner: BoundaryOwner::Section(section),
            left_index: 0,
            stations_mm: [0, 50_000],
        };
        let bind = |w| n.bind_handoff_window(n.network_revision(), w);
        if coverage == CrossSectionCoverage::StationMapping {
            assert_eq!(bind(window), Err(Failure::Unavailable));
            continue;
        }
        for stations_mm in [[5, 4], [0, 50_001], [0, u64::MAX]] {
            assert_eq!(
                bind(HandoffWindow {
                    stations_mm,
                    ..window
                }),
                Err(Failure::InvalidInput)
            );
        }
        for invalid in [
            HandoffWindow {
                left_index: u32::MAX,
                ..window
            },
            HandoffWindow {
                edges: [edges[0]; 2],
                ..window
            },
            HandoffWindow {
                owner: BoundaryOwner::Corridor(corridor),
                ..window
            },
        ] {
            assert_eq!(bind(invalid), Err(Failure::Binding));
        }
        let unknown = LaneEdgeOrdinal::from_raw(u32::MAX);
        assert_eq!(
            bind(HandoffWindow {
                edges: [edges[0], unknown],
                ..window
            }),
            Err(Failure::Unavailable)
        );
        let wrong_revision = NetworkRevisionId::from_digest(Sha256Digest::from_bytes([0; 32]));
        assert_eq!(
            n.bind_handoff_window(wrong_revision, window),
            Err(Failure::Binding)
        );
    }
}

fn remote_source(frame: &str) -> OwnedRoadEditingSourceBuffer {
    let limits = CompileProfile::single_network_10k_stable_entities();
    let header = RoadEditingModuleHeader::try_new(
        "remote",
        "other",
        vec![],
        RoadEditingProvenance::direct("remote road").unwrap(),
    )
    .unwrap();
    let mut module = RoadEditingSourceModuleBuilder::new(
        header,
        GeometryAccuracyProfile::Fine2Cm,
        GeometryDirectionProfile::Smooth1Deg,
        &limits,
    )
    .unwrap();
    module
        .add_declaration(RoadEditingDeclaration::CanonicalFrame(
            CanonicalFrameInput::try_new(frame).unwrap(),
        ))
        .unwrap();
    RoadEditingSourceWriter::new(&limits)
        .write(module.finish().unwrap())
        .unwrap()
}

fn editable_roads(
    road: &OwnedRoadEditingSourceBuffer,
    remote: &OwnedRoadEditingSourceBuffer,
) -> PortablePublicationCandidate {
    use laneflow_compiler::{SourceModuleHeader, SourceModuleHeaderInput, SyntheticModuleBuilder};
    let limits = CompileProfile::single_network_10k_stable_entities();
    let mut unit = CompilationUnitBuilder::new(limits.clone());
    for (key, source) in [("other", remote), ("native", road)] {
        unit.add_road_editing_module(
            RoadEditingModuleInput::try_new(key, source.as_bytes(), None).unwrap(),
        )
        .unwrap();
    }
    // 混合来源中的无关 Synthetic 模块只需 LFSM 来源清单，宿主不持有其原始文档。
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: "synthetic",
            source_document_key: "synthetic-frame",
            generator_build_id: "handoff-test",
            parameters_and_inputs_digest: [1; 32],
            frontend_options_digest: [2; 32],
            random_seed: None,
            provenance: "test",
        },
        &limits,
    )
    .unwrap();
    let mut generated = SyntheticModuleBuilder::new(header, &limits).unwrap();
    generated
        .add_canonical_frame(laneflow_compiler::CanonicalFrameInput {
            canonical_frame_key: "secondary-frame",
            lane_edge_geometries: &[],
        })
        .unwrap();
    unit.add_synthetic_module(generated.finish().unwrap())
        .unwrap();
    let output = Compiler::new().compile(unit.build().unwrap()).unwrap();
    assert!(
        output
            .source_map_input()
            .source_modules()
            .map(|m| m.authoring_namespace_id())
            .eq(["city", "remote", "synthetic"])
    );
    publish(&output, "handoff-rebind")
}

fn checked_bundle(
    candidate: &PortablePublicationCandidate,
) -> laneflow_format::PostEmissionCheckedBundle<&[u8], &[u8], &[u8]> {
    laneflow_format::check_post_emission_bundle(
        candidate.canonical_artifact().bytes(),
        candidate.source_map().bytes(),
        candidate.semantic_diff().bytes(),
        candidate.expected_semantic_diff_base(),
        FormatLimits::HARD,
    )
    .unwrap()
}

#[test]
fn handoff_window_rebind_checks_exact_sources_members_and_station_reference() {
    let road_source = |origin_x: f64, direction, rule_effect| {
        let road = Program::try_new(
            p(origin_x, 0., 0.),
            vec![Segment::line(p(origin_x, 0., 50.))],
        )
        .unwrap();
        save_lane_declarations(
            road,
            &[],
            (
                GeometryAccuracyProfile::Fine2Cm,
                GeometryDirectionProfile::Smooth1Deg,
            ),
            [direction; 2],
            CrossSectionCoverage::Complete,
            0,
            SectionFixture {
                lane_cuts_m: [&[22.], &[28.]],
                extents: vec![super::vehicle::extent(
                    "allow",
                    BoundaryStation::CorridorEndpoint,
                    BoundaryStation::CorridorEndpoint,
                    BoundaryPayloadInput::CrossingRule {
                        directions: 3,
                        effect: rule_effect,
                    },
                )],
                ..SectionFixture::default()
            },
        )
    };
    let base_road = road_source(0., RoadEditingLaneDirection::Forward, 1);
    let relocated = road_source(1., RoadEditingLaneDirection::Forward, 1);
    let reversed = road_source(0., RoadEditingLaneDirection::Backward, 1);
    let original_far = remote_source("far");
    let changed_far = remote_source("farther");
    let artifact = editable_roads(&base_road, &original_far);
    let base = load_network(&artifact, SpatialBuildOption::Omit);
    let checked = checked_bundle(&artifact);
    fn modules<'a>(
        road: &'a OwnedRoadEditingSourceBuffer,
        remote: &'a OwnedRoadEditingSourceBuffer,
    ) -> [HandoffSourceModule<'a>; 2] {
        [
            HandoffSourceModule {
                namespace: "city",
                document_key: "native",
                source_bytes: road.as_bytes(),
            },
            HandoffSourceModule {
                namespace: "remote",
                document_key: "other",
                source_bytes: remote.as_bytes(),
            },
        ]
    }
    let base_modules = modules(&base_road, &original_far);
    let evidence = HandoffSourceEvidence::check(&base, &checked, &base_modules).unwrap();
    // 替换结构完好且 chunk 摘要有效的 LFSM 模块/文档节，保持自报的 LFCA 绑定不变。
    let other_artifact = editable_roads(&relocated, &original_far);
    let other_bundle = checked_bundle(&other_artifact);
    let other_section = other_bundle
        .source_map_view()
        .registry_view()
        .section(1)
        .unwrap();
    let section = checked
        .source_map_view()
        .registry_view()
        .section(1)
        .unwrap();
    let mut spoofed = artifact.source_map().bytes().to_vec();
    let at = section.byte_offset() as usize;
    spoofed[at..at + section.byte_length() as usize].copy_from_slice(other_section.bytes());
    let spoofed = laneflow_format::check_post_emission_bundle(
        artifact.canonical_artifact().bytes(),
        spoofed.as_slice(),
        artifact.semantic_diff().bytes(),
        artifact.expected_semantic_diff_base(),
        FormatLimits::HARD,
    )
    .unwrap();
    let spoofed_sources = modules(&relocated, &original_far);
    assert!(matches!(
        HandoffSourceEvidence::check(&base, &spoofed, &spoofed_sources),
        Err(Failure::Binding)
    ));
    let duplicated_module = [base_modules[0], base_modules[0]];
    assert!(matches!(
        HandoffSourceEvidence::check(&base, &checked, &duplicated_module),
        Err(Failure::Binding)
    ));
    let invalid_modules = modules(&base_road, &changed_far);
    assert!(matches!(
        HandoffSourceEvidence::check(&base, &checked, &invalid_modules),
        Err(Failure::Binding)
    ));
    let corridor = RoadCorridorOrdinal::from_raw(0);
    let r = base.traffic().relations();
    let CorridorElement::RoadSection(section) = r.corridor_elements(corridor).unwrap()[0] else {
        unreachable!()
    };
    let lanes = r.section_lanes(section).unwrap();
    let window = HandoffWindow {
        owner: BoundaryOwner::Section(section),
        left_index: 0,
        edges: [
            r.authoring_edge_chain(lanes[0]).unwrap()[1],
            r.authoring_edge_chain(lanes[1]).unwrap()[0],
        ],
        stations_mm: [24_000, 26_000],
    };
    let old = base
        .bind_handoff_window(base.network_revision(), window)
        .unwrap();
    for (road, expected) in [
        (&base_road, None),
        (&relocated, Some(Failure::Unavailable)),
        (&reversed, Some(Failure::Binding)),
    ] {
        let candidate = editable_roads(road, &changed_far);
        let target = load_network(&candidate, SpatialBuildOption::RetainAvailable);
        assert_ne!(base.network_revision(), target.network_revision());
        let target_lfca = checked_bundle(&candidate);
        let target_modules = modules(road, &changed_far);
        let target_evidence =
            HandoffSourceEvidence::check(&target, &target_lfca, &target_modules[..1]).unwrap();
        assert!(matches!(
            HandoffSourceEvidence::check(&target, &checked, &base_modules),
            Err(Failure::Binding)
        ));
        let result = target.rebind_handoff_window(&base, old, &evidence, &target_evidence);
        if let Some(error) = expected {
            assert_eq!(result, Err(error));
        } else {
            let new = result.unwrap();
            assert_eq!(new.revision(), target.network_revision());
            assert_eq!(new.common_stations_mm(), old.common_stations_mm());
            assert!(matches!(
                target.rebind_handoff_window(&base, old, &target_evidence, &evidence),
                Err(Failure::Binding)
            ));
        }
    }
    // 初次注册只证结构；即使两端道路来源完全相同，禁止跨越的窗口也不能接管。
    let prohibited = road_source(0., RoadEditingLaneDirection::Forward, 2);
    let artifact_a = editable_roads(&prohibited, &original_far);
    let artifact_b = editable_roads(&prohibited, &changed_far);
    let a = load_network(&artifact_a, SpatialBuildOption::Omit);
    let b = load_network(&artifact_b, SpatialBuildOption::Omit);
    let old_bundle = checked_bundle(&artifact_a);
    let new_bundle = checked_bundle(&artifact_b);
    let only_road = [HandoffSourceModule {
        namespace: "city",
        document_key: "native",
        source_bytes: prohibited.as_bytes(),
    }];
    let a_source = HandoffSourceEvidence::check(&a, &old_bundle, &only_road).unwrap();
    let b_source = HandoffSourceEvidence::check(&b, &new_bundle, &only_road).unwrap();
    let old_window = a.bind_handoff_window(a.network_revision(), window).unwrap();
    assert_eq!(
        b.rebind_handoff_window(&a, old_window, &a_source, &b_source),
        Err(Failure::Binding)
    );
}

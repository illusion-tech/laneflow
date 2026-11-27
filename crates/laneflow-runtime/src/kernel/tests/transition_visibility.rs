//! #1220 不同过渡段共享横向区域时的路线注册与准入回归。
//!
//! 坐标约定沿用样本：`x` 为纵向，`z` 为横向，`y` 为高程。

use std::sync::Arc;

use laneflow_compiler::road_editing as editing;
use laneflow_compiler::*;
use laneflow_format::{FormatLimits, check_post_emission_bundle};
use laneflow_static_contract::LaneEdgeOrdinal;
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};

const SYNTH: &str = "ltv-synth";
const FRAME: &str = "ltv-frame";
const ROAD: &str = "ltv-road";
const CAR_LENGTH_MM: u64 = 4_500;
const SPEED_MM_S: u32 = 8_000;
const MAX_TICKS: usize = 300;

struct EdgeSpec {
    key: &'static str,
    from: (f32, f32),
    to: (f32, f32),
    successors: &'static [&'static str],
}

struct JunctionSpec {
    key: &'static str,
    entry: &'static str,
    second_entry: &'static str,
    internal: &'static str,
    exit: &'static str,
}

/// 统一模型验证：过渡段声明为连接区内部边，每条过渡段一条真实机动路径。
struct Topology {
    edges: Vec<EdgeSpec>,
    lanes: Vec<(&'static str, Vec<&'static str>)>,
    junctions: Vec<JunctionSpec>,
}

const fn edge(
    key: &'static str,
    from: (f32, f32),
    to: (f32, f32),
    successors: &'static [&'static str],
) -> EdgeSpec {
    EdgeSpec {
        key,
        from,
        to,
        successors,
    }
}

fn scoped(prefix: &str, key: &str) -> String {
    format!("{prefix}-{key}")
}

fn build(topology: &Topology) -> Arc<SharedNetworkRevision> {
    let limits = CompileProfile::single_network_1m_stable_entities();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: SYNTH,
            source_document_key: "ltv-synth.document",
            generator_build_id: "ltv-probe",
            parameters_and_inputs_digest: [0x11; 32],
            frontend_options_digest: [0x22; 32],
            random_seed: Some(0),
            provenance: "repository:laneflow-test",
        },
        &limits,
    )
    .unwrap();
    let mut synth = SyntheticModuleBuilder::new(header, &limits).unwrap();
    for spec in &topology.edges {
        let refs: Vec<_> = spec
            .successors
            .iter()
            .map(|target| LaneEdgeReference::local(target))
            .collect();
        let dx = f64::from(spec.to.0 - spec.from.0);
        let dz = f64::from(spec.to.1 - spec.from.1);
        synth
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: spec.key,
                length_meters: (dx * dx + dz * dz).sqrt(),
                speed_limit_meters_per_second: 13.9,
                successors: &refs,
            })
            .unwrap();
    }
    let chain_refs: Vec<Vec<LaneEdgeReference>> = topology
        .lanes
        .iter()
        .map(|(_, chain)| {
            chain
                .iter()
                .map(|key| LaneEdgeReference::local(key))
                .collect()
        })
        .collect();
    let lanes: Vec<AuthoringLaneInput> = topology
        .lanes
        .iter()
        .zip(chain_refs.iter())
        .map(|((key, _), refs)| AuthoringLaneInput {
            authoring_lane_key: key,
            edge_chain: refs,
            lane_group: None,
        })
        .collect();
    synth
        .add_road_section(RoadSectionInput {
            road_section_key: "ltv-section",
            kind_id: "motorLane",
            lanes: &lanes,
        })
        .unwrap();
    synth
        .add_road_corridor(RoadCorridorInput {
            road_corridor_key: "ltv-corridor",
            reference_section: RoadSectionReference::local("ltv-section"),
            elements: &[CorridorElementReference::road_section(
                RoadSectionReference::local("ltv-section"),
            )],
        })
        .unwrap();
    // 每个 maneuver 出口独立成走廊：它不属于发生换道的横断面。
    let exit_section_keys: Vec<String> = topology
        .junctions
        .iter()
        .map(|junction| scoped("exit-section", junction.key))
        .collect();
    let exit_lane_keys: Vec<String> = topology
        .junctions
        .iter()
        .map(|junction| scoped("exit-lane", junction.key))
        .collect();
    let exit_corridor_keys: Vec<String> = topology
        .junctions
        .iter()
        .map(|junction| scoped("exit-corridor", junction.key))
        .collect();
    for (index, junction) in topology.junctions.iter().enumerate() {
        let chain = [LaneEdgeReference::local(junction.exit)];
        synth
            .add_road_section(RoadSectionInput {
                road_section_key: &exit_section_keys[index],
                kind_id: "motorLane",
                lanes: &[AuthoringLaneInput {
                    authoring_lane_key: &exit_lane_keys[index],
                    edge_chain: &chain,
                    lane_group: None,
                }],
            })
            .unwrap();
        synth
            .add_road_corridor(RoadCorridorInput {
                road_corridor_key: &exit_corridor_keys[index],
                reference_section: RoadSectionReference::local(&exit_section_keys[index]),
                elements: &[CorridorElementReference::road_section(
                    RoadSectionReference::local(&exit_section_keys[index]),
                )],
            })
            .unwrap();
    }
    let point_pairs: Vec<[CanonicalPoint3F32Input; 2]> = topology
        .edges
        .iter()
        .map(|spec| {
            [
                CanonicalPoint3F32Input {
                    x: spec.from.0,
                    y: 0.0,
                    z: spec.from.1,
                },
                CanonicalPoint3F32Input {
                    x: spec.to.0,
                    y: 0.0,
                    z: spec.to.1,
                },
            ]
        })
        .collect();
    let geometries: Vec<_> = topology
        .edges
        .iter()
        .zip(point_pairs.iter())
        .map(|(spec, centerline_points)| LaneEdgeGeometryInput {
            lane_edge: LaneEdgeReference::local(spec.key),
            centerline_points,
        })
        .collect();
    synth
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: FRAME,
            lane_edge_geometries: &geometries,
        })
        .unwrap();

    let header = editing::RoadEditingModuleHeader::try_new(
        ROAD,
        "ltv-road.document",
        vec![SYNTH.to_owned()],
        editing::RoadEditingProvenance::direct("#1220 transition visibility regression").unwrap(),
    )
    .unwrap();
    let mut builder = editing::RoadEditingSourceModuleBuilder::new(
        header,
        GeometryAccuracyProfile::Balanced5Cm,
        GeometryDirectionProfile::Balanced2Deg,
        &limits,
    )
    .unwrap();
    let imported =
        |key: &str| editing::LaneEdgeReference::imported(SYNTH, Vec::new(), key).unwrap();
    let mut stream_rules = Vec::new();
    let mut gate_rules = Vec::new();
    let mut regions = Vec::new();
    for (index, junction) in topology.junctions.iter().enumerate() {
        let owner = vec![junction.key.to_owned()];
        let junction_ref = editing::JunctionReference::local(junction.key).unwrap();
        let zone = editing::ConflictZoneReference::owner_scoped(owner.clone(), "zone").unwrap();
        builder
            .add_declaration(editing::RoadEditingDeclaration::Junction(
                editing::JunctionInput::try_new(
                    junction.key,
                    vec![
                        imported(junction.entry),
                        imported(junction.second_entry),
                        imported(junction.exit),
                    ],
                    vec![imported(junction.internal)],
                )
                .unwrap(),
            ))
            .unwrap()
            .add_declaration(editing::RoadEditingDeclaration::ConflictZone(
                editing::ConflictZoneInput::try_new("zone", junction_ref.clone()).unwrap(),
            ))
            .unwrap();
        // 冲突区要求不少于两条参与流：第二入口只为满足该声明约束，探针车辆不走它。
        for (stream_index, entry) in [junction.entry, junction.second_entry]
            .into_iter()
            .enumerate()
        {
            let movement_key = format!("movement-{stream_index}");
            let stream_key = format!("stream-{stream_index}");
            let movement_ref =
                editing::MovementReference::owner_scoped(owner.clone(), movement_key.as_str())
                    .unwrap();
            let path_ref = editing::ManeuverPathReference::owner_scoped(
                vec![junction.key.to_owned(), movement_key.clone()],
                "path",
            )
            .unwrap();
            let gate_ref = editing::ManeuverGateReference::owner_scoped(
                vec![
                    junction.key.to_owned(),
                    movement_key.clone(),
                    "path".to_owned(),
                ],
                "admission",
            )
            .unwrap();
            let stop_key = format!("stop-{}-{stream_index}", junction.key);
            builder
                .add_declaration(editing::RoadEditingDeclaration::Movement(
                    editing::MovementInput::try_new(
                        movement_key.as_str(),
                        junction_ref.clone(),
                        entry.to_owned(),
                        junction.exit.to_owned(),
                    )
                    .unwrap(),
                ))
                .unwrap()
                .add_declaration(editing::RoadEditingDeclaration::ManeuverPath(
                    editing::ManeuverPathInput::try_new(
                        "path",
                        movement_ref,
                        imported(entry),
                        vec![imported(junction.internal)],
                        imported(junction.exit),
                    )
                    .unwrap(),
                ))
                .unwrap()
                .add_declaration(editing::RoadEditingDeclaration::StopLine(
                    editing::StopLineInput::try_new(stop_key.as_str(), imported(entry)).unwrap(),
                ))
                .unwrap()
                .add_declaration(editing::RoadEditingDeclaration::ManeuverGate(
                    editing::ManeuverGateInput::try_new(
                        "admission",
                        path_ref.clone(),
                        0,
                        editing::StopLineReference::local(stop_key.as_str()).unwrap(),
                        editing::RoadEditingSignalControl::None,
                    )
                    .unwrap(),
                ))
                .unwrap()
                .add_declaration(editing::RoadEditingDeclaration::ParticipantStream(
                    editing::ParticipantStreamInput::try_new(
                        stream_key.as_str(),
                        junction_ref.clone(),
                        path_ref,
                        vec![editing::ConflictPassageInput::new(
                            zone.clone(),
                            editing::PathAnchorInput::gate(gate_ref.clone()),
                            editing::PathAnchorInput::edge_boundary(2),
                        )],
                    )
                    .unwrap(),
                ))
                .unwrap();
            stream_rules.push(
                editing::PolicyStreamRuleInput::try_new(
                    format!("stream-rule-{}-{stream_index}", junction.key),
                    editing::ParticipantStreamReference::owner_scoped(
                        owner.clone(),
                        stream_key.as_str(),
                    )
                    .unwrap(),
                    None,
                    0,
                    vec![],
                    None,
                    vec![],
                )
                .unwrap(),
            );
            gate_rules.push(
                editing::PolicyGateRuleInput::try_new(
                    format!("gate-rule-{}-{stream_index}", junction.key),
                    gate_ref,
                    None,
                    GateInterpretation::Uncontrolled,
                    GateProhibition::None,
                    vec![],
                )
                .unwrap(),
            );
        }
        let base = 200.0 + 20.0 * index as f64;
        regions.push(
            editing::ConflictZoneRegionInput::try_new(
                zone,
                editing::CanonicalFrameReference::imported(SYNTH, Vec::new(), FRAME).unwrap(),
                -1.0,
                1.0,
                vec![
                    editing::RoadEditingPoint2::try_new(base, -2.0).unwrap(),
                    editing::RoadEditingPoint2::try_new(base + 10.0, -2.0).unwrap(),
                    editing::RoadEditingPoint2::try_new(base + 10.0, 2.0).unwrap(),
                    editing::RoadEditingPoint2::try_new(base, 2.0).unwrap(),
                ],
            )
            .unwrap(),
        );
    }
    builder
        .add_declaration(editing::RoadEditingDeclaration::ParticipantClass(
            editing::ParticipantClassInput::try_new("car-class").unwrap(),
        ))
        .unwrap()
        .add_declaration(editing::RoadEditingDeclaration::VehicleProfile(
            editing::VehicleProfileInput::try_new(
                "car",
                editing::ParticipantClassReference::local("car-class").unwrap(),
                editing::IidmVehicleProfileInput::try_new(4.5, 15.0, 2.5, 1.5, 1.5, 4.5, 9.0)
                    .unwrap(),
                laneflow_synth_generator::PASSENGER_CAR_GEOMETRY,
            )
            .unwrap(),
        ))
        .unwrap()
        .add_declaration(editing::RoadEditingDeclaration::RightOfWayPolicySet(
            editing::RightOfWayPolicySetInput::try_new(
                "policy",
                RegulationIdentity::try_new("engineering", "fixture-1")
                    .unwrap()
                    .with_source("repository:laneflow-test")
                    .unwrap(),
                vec![],
                vec![
                    editing::PolicyGapProfileInput::try_new(
                        "calibration-gap",
                        "fixture-1",
                        500,
                        500,
                        100,
                    )
                    .unwrap(),
                ],
                stream_rules,
                gate_rules,
            )
            .unwrap(),
        ))
        .unwrap();
    for region in regions {
        builder.add_conflict_zone_region(region).unwrap();
    }
    let source = editing::RoadEditingSourceWriter::new(&limits)
        .write(builder.finish().unwrap())
        .unwrap();
    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_synthetic_module(synth.finish().unwrap()).unwrap();
    unit.add_road_editing_module(
        editing::RoadEditingModuleInput::try_new("ltv-road.document", source.as_bytes(), None)
            .unwrap(),
    )
    .unwrap();
    let output = Compiler::new().compile(unit.build().unwrap()).unwrap();
    let candidate = emit_portable_candidate(
        &output,
        &PortableEmissionProvenance::try_new("ltv").unwrap(),
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .unwrap();
    let checked = check_post_emission_bundle(
        candidate.canonical_artifact().bytes(),
        candidate.source_map().bytes(),
        candidate.semantic_diff().bytes(),
        candidate.expected_semantic_diff_base(),
        FormatLimits::HARD,
    )
    .unwrap();
    build_shared_network_revision(
        checked.canonical_network_input(),
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::Omit,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .unwrap()
}

fn install(revision: Arc<SharedNetworkRevision>, workers: u32) -> crate::TrafficWorld {
    let origin = *revision.canonical_origin();
    let policy = laneflow_static_contract::RightOfWayPolicySetId::from_untyped(
        derive_canonical_stable_id_v1(
            laneflow_static_contract::EntityKind::RightOfWayPolicySet,
            ROAD,
            "policy",
        )
        .unwrap(),
    );
    crate::TrafficWorld::install(
        revision,
        crate::WorldConfig::new(4, 4, 64, 32, 100),
        crate::ExecutionConfig::new(std::num::NonZeroU32::new(workers).expect("workers")),
        crate::CommittedNetworkSource::Published {
            reference: crate::PublishedLfcaReference::new(
                "fixture://ltv",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .unwrap(),
        },
        1_220,
        crate::WorldPolicySelection::Pinned(crate::PolicyPin { policy }),
    )
    .unwrap()
}

/// 边按“编译后长度 + 前驱”解析：非过渡边长度唯一，过渡段由唯一前驱区分。
fn by_length(revision: &SharedNetworkRevision, length_mm: u32) -> LaneEdgeOrdinal {
    let traffic = revision.traffic();
    let lengths = traffic.lane_lengths_millimetres();
    let found: Vec<_> = (0..traffic.lane_edge_count())
        .map(LaneEdgeOrdinal::from_raw)
        .filter(|edge| lengths[edge.index()] == length_mm)
        .collect();
    assert_eq!(found.len(), 1, "length {length_mm} must be unique");
    found[0]
}

fn transition_after(revision: &SharedNetworkRevision, pred: LaneEdgeOrdinal) -> LaneEdgeOrdinal {
    let traffic = revision.traffic();
    traffic
        .successors(pred)
        .unwrap()
        .iter()
        .copied()
        .find(|edge| traffic.is_lateral_transition_segment(*edge))
        .expect("predecessor has a lateral transition successor")
}

/// 按稳定 id 解析边序号：用于长度/前驱都不唯一的边（如 #1321 的独立下游链）。
fn named(revision: &SharedNetworkRevision, key: &str) -> LaneEdgeOrdinal {
    let stable_id =
        derive_canonical_stable_id_v1(laneflow_static_contract::EntityKind::LaneEdge, SYNTH, key)
            .unwrap();
    revision
        .identity()
        .ordinal(laneflow_static_contract::LaneEdgeId::from_untyped(
            stable_id,
        ))
        .unwrap()
}

fn cross_topology() -> Topology {
    Topology {
        edges: vec![
            edge("e0a", (0.0, 0.0), (26.0, 0.0), &["s0", "t1"]),
            edge("s0", (26.0, 0.0), (30.0, 0.0), &["e0b"]),
            edge("e0b", (30.0, 0.0), (60.0, 0.0), &[]),
            edge("e2a", (-1.0, 3.0), (26.0, 3.0), &["s2", "t2"]),
            edge("s2", (26.0, 3.0), (30.0, 3.0), &["e2b"]),
            edge("e2b", (30.0, 3.0), (61.0, 3.0), &[]),
            // 3-4-5 三角：两条过渡段在 (28, 1.5) 真实交叉。
            edge("t1", (26.0, 0.0), (30.0, 3.0), &["e2b"]),
            edge("t2", (26.0, 3.0), (30.0, 0.0), &["e0b"]),
            edge("int1", (61.0, 3.0), (69.0, 3.0), &[]),
            edge("exit1", (69.0, 3.0), (79.0, 3.0), &[]),
            edge("int2", (60.0, 0.0), (69.0, 0.0), &[]),
            edge("exit2", (69.0, 0.0), (80.0, 0.0), &[]),
            edge("f1", (29.0, 3.0), (61.0, 3.0), &[]),
            edge("f2", (27.0, 0.0), (60.0, 0.0), &[]),
        ],
        lanes: vec![
            ("lane-0", vec!["e0a", "s0", "e0b"]),
            ("lane-1", vec!["e2a", "s2", "e2b"]),
            ("lane-f1", vec!["f1"]),
            ("lane-f2", vec!["f2"]),
        ],
        junctions: vec![
            JunctionSpec {
                key: "j1",
                entry: "e2b",
                second_entry: "f1",
                internal: "int1",
                exit: "exit1",
            },
            JunctionSpec {
                key: "j2",
                entry: "e0b",
                second_entry: "f2",
                internal: "int2",
                exit: "exit2",
            },
        ],
    }
}

fn merge_topology() -> Topology {
    Topology {
        edges: vec![
            edge("e0a", (0.0, 0.0), (26.0, 0.0), &["s0", "t1"]),
            edge("s0", (26.0, 0.0), (30.0, 0.0), &["e0b"]),
            edge("e0b", (30.0, 0.0), (61.0, 0.0), &[]),
            edge("e1a", (-1.0, 3.0), (26.0, 3.0), &["s1"]),
            edge("s1", (26.0, 3.0), (30.0, 3.0), &["e1b"]),
            edge("e1b", (30.0, 3.0), (60.0, 3.0), &[]),
            edge("e2a", (-2.0, 6.0), (26.0, 6.0), &["s2", "t2"]),
            edge("s2", (26.0, 6.0), (30.0, 6.0), &["e2b"]),
            edge("e2b", (30.0, 6.0), (62.0, 6.0), &[]),
            edge("t1", (26.0, 0.0), (30.0, 3.0), &["e1b"]),
            edge("t2", (26.0, 6.0), (30.0, 3.0), &["e1b"]),
            edge("int", (60.0, 3.0), (68.0, 3.0), &[]),
            edge("exit", (68.0, 3.0), (78.0, 3.0), &[]),
            edge("f1", (27.0, 3.0), (60.0, 3.0), &[]),
        ],
        lanes: vec![
            ("lane-0", vec!["e0a", "s0", "e0b"]),
            ("lane-1", vec!["e1a", "s1", "e1b"]),
            ("lane-2", vec!["e2a", "s2", "e2b"]),
            ("lane-f1", vec!["f1"]),
        ],
        junctions: vec![JunctionSpec {
            key: "j1",
            entry: "e1b",
            second_entry: "f1",
            internal: "int",
            exit: "exit",
        }],
    }
}

/// C1：共享横向投影区、下游路线不相交（X 形）。
fn staggered_topology(shared_middle: bool) -> Topology {
    let mut edges = vec![
        edge("e0a", (0.0, 0.0), (26.0, 0.0), &["s0", "t1"]),
        edge("s0", (26.0, 0.0), (30.0, 0.0), &["e0m"]),
        edge("e2a", (-1.0, 3.0), (26.0, 3.0), &["s2"]),
        edge("s2", (26.0, 3.0), (30.0, 3.0), &["e2m"]),
        edge("t1", (26.0, 0.0), (30.0, 3.0), &["e2m"]),
        edge("s2p", (90.0, 3.0), (94.0, 3.0), &["e2b"]),
        edge("s0p", (90.0, 0.0), (94.0, 0.0), &["e0b"]),
        edge("e2b", (94.0, 3.0), (125.0, 3.0), &[]),
        edge("e0b", (94.0, 0.0), (124.0, 0.0), &[]),
        edge("t2", (90.0, 3.0), (94.0, 0.0), &["e0b"]),
        edge("int1", (125.0, 3.0), (133.0, 3.0), &[]),
        edge("exit1", (133.0, 3.0), (143.0, 3.0), &[]),
        edge("int2", (124.0, 0.0), (133.0, 0.0), &[]),
        edge("exit2", (133.0, 0.0), (144.0, 0.0), &[]),
        edge("f1", (93.0, 3.0), (125.0, 3.0), &[]),
        edge("f2", (91.0, 0.0), (124.0, 0.0), &[]),
    ];
    let lanes = if shared_middle {
        edges.extend([
            edge("e2m", (30.0, 3.0), (90.0, 3.0), &["s2p", "t2"]),
            edge("e0m", (30.0, 0.0), (90.0, 0.0), &["s0p"]),
        ]);
        vec![
            ("lane-0", vec!["e0a", "s0", "e0m", "s0p", "e0b"]),
            ("lane-1", vec!["e2a", "s2", "e2m", "s2p", "e2b"]),
            ("lane-f1", vec!["f1"]),
            ("lane-f2", vec!["f2"]),
        ]
    } else {
        edges.extend([
            edge("e2m", (30.0, 3.0), (60.0, 3.0), &["e2n"]),
            edge("e2n", (60.0, 3.0), (90.0, 3.0), &["s2p", "t2"]),
            edge("e0m", (30.0, 0.0), (60.0, 0.0), &["e0n"]),
            edge("e0n", (60.0, 0.0), (90.0, 0.0), &["s0p"]),
        ]);
        vec![
            ("lane-0", vec!["e0a", "s0", "e0m", "e0n", "s0p", "e0b"]),
            ("lane-1", vec!["e2a", "s2", "e2m", "e2n", "s2p", "e2b"]),
            ("lane-f1", vec!["f1"]),
            ("lane-f2", vec!["f2"]),
        ]
    };
    Topology {
        edges,
        lanes,
        junctions: vec![
            JunctionSpec {
                key: "j1",
                entry: "e2b",
                second_entry: "f1",
                internal: "int1",
                exit: "exit1",
            },
            JunctionSpec {
                key: "j2",
                entry: "e0b",
                second_entry: "f2",
                internal: "int2",
                exit: "exit2",
            },
        ],
    }
}

fn adjacent_topology() -> Topology {
    Topology {
        edges: vec![
            edge("e0a", (0.0, 0.0), (26.0, 0.0), &["s0", "t1"]),
            edge("s0", (26.0, 0.0), (30.0, 0.0), &["e0b"]),
            edge("e0b", (30.0, 0.0), (66.0, 0.0), &[]),
            edge("e1a", (-1.0, 3.0), (26.0, 3.0), &["s1", "t2"]),
            edge("s1", (26.0, 3.0), (30.0, 3.0), &["e1b"]),
            edge("e1b", (30.0, 3.0), (60.0, 3.0), &[]),
            edge("e2a", (-2.0, 6.0), (26.0, 6.0), &["s2"]),
            edge("s2", (26.0, 6.0), (30.0, 6.0), &["e2b"]),
            edge("e2b", (30.0, 6.0), (61.0, 6.0), &[]),
            edge("t1", (26.0, 0.0), (30.0, 3.0), &["e1b"]),
            edge("t2", (26.0, 3.0), (30.0, 6.0), &["e2b"]),
            edge("int1", (60.0, 3.0), (68.0, 3.0), &[]),
            edge("exit1", (68.0, 3.0), (78.0, 3.0), &[]),
            edge("int2", (61.0, 6.0), (70.0, 6.0), &[]),
            edge("exit2", (70.0, 6.0), (81.0, 6.0), &[]),
            edge("f1", (28.0, 3.0), (60.0, 3.0), &[]),
            edge("f2", (28.0, 6.0), (61.0, 6.0), &[]),
        ],
        lanes: vec![
            ("lane-0", vec!["e0a", "s0", "e0b"]),
            ("lane-1", vec!["e1a", "s1", "e1b"]),
            ("lane-2", vec!["e2a", "s2", "e2b"]),
            ("lane-f1", vec!["f1"]),
            ("lane-f2", vec!["f2"]),
        ],
        junctions: vec![
            JunctionSpec {
                key: "j1",
                entry: "e1b",
                second_entry: "f1",
                internal: "int1",
                exit: "exit1",
            },
            JunctionSpec {
                key: "j2",
                entry: "e2b",
                second_entry: "f2",
                internal: "int2",
                exit: "exit2",
            },
        ],
    }
}

/// C5 拓扑：目标车道在过渡段汇入点之前分岔，直行车走平行边后驶向分岔支路。
fn diverging_straight_topology() -> Topology {
    let mut topology = cross_topology();
    for spec in &mut topology.edges {
        if spec.key == "e2a" {
            spec.successors = &["s2"];
        }
        if spec.key == "s2" {
            spec.successors = &["e2b", "e2x"];
        }
    }
    topology.edges.retain(|spec| spec.key != "t2");
    topology
        .edges
        .push(edge("e2x", (30.0, 3.0), (64.0, 3.0), &[]));
    topology.lanes.push(("lane-x", vec!["e2x"]));
    topology
}

/// 两车逐拍的 `(A 路线边下标, A 进度, B 路线边下标, B 进度)`。
type Trace = Vec<(u32, u32, u32, u32)>;

struct Drive {
    trace: Trace,
    a_entered: Option<usize>,
    b_entered: Option<usize>,
    bodies_together_in_transitions: bool,
}

fn drive(
    revision: &Arc<SharedNetworkRevision>,
    workers: u32,
    route_a: &[LaneEdgeOrdinal],
    start_a_mm: u32,
    route_b: &[LaneEdgeOrdinal],
    start_b_mm: u32,
) -> Drive {
    let lengths: Vec<u32> = revision.traffic().lane_lengths_millimetres().to_vec();
    let mut world = install(revision.clone(), workers);
    let mut spawn = |route: &[LaneEdgeOrdinal], start: u32| {
        let handle = world
            .register_route(crate::RouteRegisterInput::new(route.to_vec()))
            .expect("route registers");
        world
            .spawn_vehicle(
                crate::VehicleSpawnInput::new(
                    laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                    handle,
                    0,
                    start,
                    SPEED_MM_S,
                )
                .with_open_entrance(),
            )
            .expect("spawn")
    };
    let a = spawn(route_a, start_a_mm);
    let b = spawn(route_b, start_b_mm);
    world.state.rebuild_occupancy_index().unwrap();
    let span = |route: &[LaneEdgeOrdinal]| {
        let start = u64::from(lengths[route[0].index()]);
        (start, start + u64::from(lengths[route[1].index()]))
    };
    let (a_t0, a_t1) = span(route_a);
    let (b_t0, b_t1) = span(route_b);
    let mut result = Drive {
        trace: Vec::new(),
        a_entered: None,
        b_entered: None,
        bodies_together_in_transitions: false,
    };
    for tick in 0..MAX_TICKS {
        world.state.prepare_waiting_step(0.1).unwrap();
        world.step(crate::TickInput::new(100)).unwrap();
        let live = world.live_vehicles();
        if !live.contains(&a) || !live.contains(&b) {
            break;
        }
        let sa = world.state.committed.vehicles.state(a).unwrap();
        let sb = world.state.committed.vehicles.state(b).unwrap();
        result.trace.push((
            sa.route_edge_index,
            sa.progress_mm,
            sb.route_edge_index,
            sb.progress_mm,
        ));
        if sa.route_edge_index >= 1 {
            result.a_entered.get_or_insert(tick);
        }
        if sb.route_edge_index >= 1 {
            result.b_entered.get_or_insert(tick);
        }
        let pa = absolute_position(&lengths, route_a, sa.route_edge_index, sa.progress_mm);
        let pb = absolute_position(&lengths, route_b, sb.route_edge_index, sb.progress_mm);
        if body_in_span(pa, a_t0, a_t1) && body_in_span(pb, b_t0, b_t1) {
            result.bodies_together_in_transitions = true;
        }
        if sa.route_edge_index >= 2 && sb.route_edge_index >= 2 {
            break;
        }
    }
    result
}

fn body_in_span(position_mm: u64, start_mm: u64, end_mm: u64) -> bool {
    position_mm > start_mm && position_mm.saturating_sub(CAR_LENGTH_MM) < end_mm
}

fn absolute_position(lengths: &[u32], route: &[LaneEdgeOrdinal], index: u32, progress: u32) -> u64 {
    route[..index as usize]
        .iter()
        .map(|edge| u64::from(lengths[edge.index()]))
        .sum::<u64>()
        + u64::from(progress)
}

/// #1321 拒绝面的公共断言：三种 worker 配置下均在注册阶段得到
/// `TransitionUnsupported`；失败原子直接核对注册表状态——三类计数为零、
/// 路线槽位与空闲槽位仍空、命令游标未推进（世界路线容量只有 4，
/// 「再注册一条直行线」不足以证明失败没有泄漏槽位）；最后验证拒绝不波及
/// 同一路网上不经过这些过渡段的路线。
fn assert_rejected(
    revision: &Arc<SharedNetworkRevision>,
    routes: &[&[LaneEdgeOrdinal]],
    still_allowed: &[LaneEdgeOrdinal],
) {
    for workers in [1, 2, 4] {
        let mut world = install(revision.clone(), workers);
        for (position, route) in routes.iter().enumerate() {
            assert_eq!(
                world
                    .register_route(crate::RouteRegisterInput::new(route.to_vec()))
                    .map(|_| ()),
                Err(crate::RouteError::TransitionUnsupported),
                "workers = {workers}, route #{position}"
            );
            let committed = &world.state.committed;
            assert_eq!(committed.live_route_count, 0, "workers = {workers}");
            assert_eq!(
                committed.live_route_edge_occurrence_count, 0,
                "workers = {workers}"
            );
            assert_eq!(
                committed.live_route_conflict_occurrence_count, 0,
                "workers = {workers}"
            );
            assert!(
                committed.routes.is_empty() && committed.free_routes.is_empty(),
                "注册失败不得占用路线槽位：workers = {workers}"
            );
            assert_eq!(world.command_cursor(), 0, "workers = {workers}");
        }
        // 拒绝不波及同一路网上不经过这些过渡段的路线。
        let route = world
            .register_route(crate::RouteRegisterInput::new(still_allowed.to_vec()))
            .expect("route without transitions still registers");
        world
            .spawn_vehicle(
                crate::VehicleSpawnInput::new(
                    laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                    route,
                    0,
                    15_000,
                    SPEED_MM_S,
                )
                .with_open_entrance(),
            )
            .expect("spawn on allowed route");
        world.state.rebuild_occupancy_index().unwrap();
        world.state.prepare_waiting_step(0.1).unwrap();
        world
            .step(crate::TickInput::new(100))
            .expect("world continues");
    }
}

fn straight_after(revision: &SharedNetworkRevision, pred: LaneEdgeOrdinal) -> LaneEdgeOrdinal {
    let traffic = revision.traffic();
    traffic
        .successors(pred)
        .unwrap()
        .iter()
        .copied()
        .find(|edge| !traffic.is_lateral_transition_segment(*edge))
        .expect("straight successor")
}

/// C1：两条过渡段交叉、下游路线不相交。修复前两车同拍进入且互不减速。
#[test]
fn crossing_transitions_with_disjoint_downstream_are_rejected() {
    let revision = build(&cross_topology());
    let get = |length| by_length(&revision, length);
    let (e0a, e2a) = (get(26_000), get(27_000));
    let route_a = [
        e0a,
        transition_after(&revision, e0a),
        get(31_000),
        get(8_000),
        get(10_000),
    ];
    let route_b = [
        e2a,
        transition_after(&revision, e2a),
        get(30_000),
        get(9_000),
        get(11_000),
    ];
    let straight = [
        e0a,
        straight_after(&revision, e0a),
        get(30_000),
        get(9_000),
        get(11_000),
    ];
    assert_rejected(&revision, &[&route_a, &route_b], &straight);
}

/// C2：两条过渡段汇入同一后继边，P4 能发现对方，串行通过且与 worker 数无关。
#[test]
fn merging_transitions_with_shared_downstream_serialize() {
    let revision = build(&merge_topology());
    let get = |length| by_length(&revision, length);
    let (e0a, e2a) = (get(26_000), get(28_000));
    let (e1b, int_edge, exit) = (get(30_000), get(8_000), get(10_000));
    let route_a = [e0a, transition_after(&revision, e0a), e1b, int_edge, exit];
    let route_b = [e2a, transition_after(&revision, e2a), e1b, int_edge, exit];
    let baseline = drive(&revision, 1, &route_a, 15_000, &route_b, 17_000);
    assert!(!baseline.bodies_together_in_transitions);
    let (a_tick, b_tick) = (baseline.a_entered.unwrap(), baseline.b_entered.unwrap());
    assert!(a_tick < b_tick, "live order first: {a_tick} then {b_tick}");
    for workers in [2, 4] {
        let run = drive(&revision, workers, &route_a, 15_000, &route_b, 17_000);
        assert_eq!(run.trace, baseline.trace, "workers = {workers}");
    }
}

/// C3：后一条过渡段的前驱正是前一条的后继，P4 能发现对方，路线可注册且都能通过。
#[test]
fn staggered_transitions_sharing_a_middle_edge_are_allowed() {
    let revision = build(&staggered_topology(true));
    let traffic = revision.traffic();
    let get = |length| by_length(&revision, length);
    let e0a = get(26_000);
    let t1 = transition_after(&revision, e0a);
    let e2m = traffic.successors(t1).unwrap()[0];
    let t2 = transition_after(&revision, e2m);
    let s2p = straight_after(&revision, e2m);
    let route_a = [e0a, t1, e2m, s2p, get(31_000), get(8_000), get(10_000)];
    let route_b = [e2m, t2, get(30_000), get(9_000), get(11_000)];
    let run = drive(&revision, 1, &route_a, 15_000, &route_b, 40_000);
    assert!(run.a_entered.is_some() && run.b_entered.is_some());
}

/// C3′：纵向相隔多条边的两条过渡段物理上不冲突，属于有意的保守拒绝。
#[test]
fn separated_transitions_are_conservatively_rejected() {
    let revision = build(&staggered_topology(false));
    let traffic = revision.traffic();
    let lengths = traffic.lane_lengths_millimetres();
    let get = |length| by_length(&revision, length);
    let e0a = get(26_000);
    let t1 = transition_after(&revision, e0a);
    let e2m = traffic.successors(t1).unwrap()[0];
    let e2n = traffic.successors(e2m).unwrap()[0];
    let s2p = straight_after(&revision, e2n);
    let e2b = traffic.successors(s2p).unwrap()[0];
    let route_a = [e0a, t1, e2m, e2n, s2p, e2b, get(8_000), get(10_000)];
    let t2 = transition_after(&revision, e2n);
    let e0b = traffic.successors(t2).unwrap()[0];
    let route_b = [e2m, e2n, t2, e0b, get(9_000), get(11_000)];
    assert_eq!(lengths[t1.index()], lengths[t2.index()]);
    let s0 = straight_after(&revision, e0a);
    let e0m = traffic.successors(s0).unwrap()[0];
    let e0n = traffic.successors(e0m).unwrap()[0];
    let s0p = traffic.successors(e0n).unwrap()[0];
    let straight = [e0a, s0, e0m, e0n, s0p, e0b, get(9_000), get(11_000)];
    assert_rejected(&revision, &[&route_a, &route_b], &straight);
}

/// C4：同向相邻的过渡段横向跨度相交、邻接边不相交，属于有意的保守拒绝。
#[test]
fn adjacent_same_direction_transitions_are_conservatively_rejected() {
    let revision = build(&adjacent_topology());
    let get = |length| by_length(&revision, length);
    let (e0a, e1a) = (get(26_000), get(27_000));
    let route_a = [
        e0a,
        transition_after(&revision, e0a),
        get(30_000),
        get(8_000),
        get(10_000),
    ];
    let route_b = [
        e1a,
        transition_after(&revision, e1a),
        get(31_000),
        get(9_000),
        get(11_000),
    ];
    let straight = [e0a, straight_after(&revision, e0a), get(36_000)];
    assert_rejected(&revision, &[&route_a, &route_b], &straight);
}

/// C5（能力限制，合同 §10 已登记）：直行车走平行边并驶向分岔支路，路线不含过渡段的
/// 前驱、后继，P4 不覆盖它。本测试记录现状；修复后应按合同改写断言。
#[test]
fn straight_vehicle_on_parallel_edge_is_a_registered_gap() {
    let revision = build(&diverging_straight_topology());
    let get = |length| by_length(&revision, length);
    let (e0a, e2a) = (get(26_000), get(27_000));
    let t1 = transition_after(&revision, e0a);
    let route_a = [e0a, t1, get(31_000), get(8_000), get(10_000)];
    let route_b = [e2a, straight_after(&revision, e2a), get(34_000)];
    let run = drive(&revision, 1, &route_a, 15_000, &route_b, 16_000);
    // F4 后恢复：conflict 屏障窗口回退本拍，A 不再为窗外过渡资源提前
    // 减速；两车回到近似同时到达（P4 不覆盖 B 的缺口本身未变）。
    assert_eq!(run.a_entered, run.b_entered);
    assert!(run.bodies_together_in_transitions);
}

/// #1322：循环路线先经过过渡段 T 的直接后继 D、绕行后才回到 T 时，P4 不得让远处的
/// 未来 T 出现记录遮蔽更近的 D 边界——P4 证明边界必须与 P5 的预约停车边界一致
///（合同 §5.3）。换道车首次进入过渡段的那一拍，直行车到最近边界（D 起点）的距离
/// 不得小于其生产停止距离；对照路线截去未来 T 后，共享前缀期间沿用同一近端判定。
#[test]
fn loop_route_future_transition_does_not_hide_nearer_downstream_boundary() {
    let mut topology = cross_topology();
    for spec in &mut topology.edges {
        if spec.key == "e2a" {
            spec.successors = &["s2"];
        }
        if spec.key == "exit1" {
            spec.successors = &["loop-return"];
        }
    }
    topology.edges.retain(|spec| spec.key != "t2");
    topology
        .edges
        .push(edge("loop-return", (79.0, 3.0), (0.0, 0.0), &["e0a"]));
    topology.lanes.push(("lane-return", vec!["loop-return"]));
    let revision = build(&topology);
    let traffic = revision.traffic();
    let lengths = traffic.lane_lengths_millimetres().to_vec();
    let get = |length| by_length(&revision, length);
    let (e0a, e2a) = (get(26_000), get(27_000));
    let t = transition_after(&revision, e0a);
    let s2 = straight_after(&revision, e2a);
    let (e2b, int1, exit1) = (get(31_000), get(8_000), get(10_000));
    let return_edge = traffic.successors(exit1).unwrap()[0];
    let changer_edges = vec![e0a, t, e2b, int1, exit1];
    let straight_loop = vec![
        e2a,
        s2,
        e2b,
        int1,
        exit1,
        return_edge,
        e0a,
        t,
        e2b,
        int1,
        exit1,
    ];
    let mut outcomes = Vec::new();
    let mut baselines: Vec<(bool, Trace)> = Vec::new();
    for workers in [1, 2, 4] {
        for looping in [false, true] {
            let mut world = install(revision.clone(), workers);
            let mut spawn = |edges: &[LaneEdgeOrdinal], progress_mm: u32| {
                let handle = world
                    .register_route(crate::RouteRegisterInput::new(edges.to_vec()))
                    .expect("route registers");
                world
                    .spawn_vehicle(
                        crate::VehicleSpawnInput::new(
                            laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                            handle,
                            0,
                            progress_mm,
                            SPEED_MM_S,
                        )
                        .with_open_entrance(),
                    )
                    .expect("spawn")
            };
            let changer = spawn(&changer_edges, 15_000);
            let straight_edges = if looping {
                straight_loop.clone()
            } else {
                straight_loop[..5].to_vec()
            };
            let straight = spawn(&straight_edges, 16_000);
            world.state.rebuild_occupancy_index().unwrap();
            let mut trace = Trace::new();
            let mut entered = None;
            for tick in 0..100 {
                let before = {
                    let state = world.state.committed.vehicles.state(straight).unwrap();
                    (
                        state.route_edge_index,
                        state.progress_mm,
                        state.speed_mm_s,
                        state.carry_um,
                    )
                };
                world.state.prepare_waiting_step(0.1).unwrap();
                world.step(crate::TickInput::new(100)).unwrap();
                let (changed, continued) = {
                    let changer = world.state.committed.vehicles.state(changer).unwrap();
                    let straight = world.state.committed.vehicles.state(straight).unwrap();
                    (
                        (changer.route_edge_index, changer.progress_mm),
                        (straight.route_edge_index, straight.progress_mm),
                    )
                };
                trace.push((changed.0, changed.1, continued.0, continued.1));
                if changed.0 > 0 {
                    // 换道车首次进入 T：核对直行车拍初到最近边界（D 起点，即 P5 的
                    // 预约停车边界）的距离与生产停止距离。直行路线前两条边是
                    // e2a、s2，D（e2b）起点 = e2a 剩余 + s2 全长；下标 ≥3 表示
                    // 已越过 D，之后最近有效目标只剩远处的未来 T。
                    let near_boundary_mm = match before.0 {
                        0 => u64::from(lengths[e2a.index()] - before.1 + lengths[s2.index()]),
                        1 => u64::from(lengths[s2.index()] - before.1),
                        _ => 0,
                    };
                    // 具名参数取进入者档案（profile 0：服务减速度 4500 mm/s²、
                    // 最小间距 2500 mm），与 P4 逐进入者证明同一口径。
                    let stop_um = crate::kernel::lane_change::service_stop_distance_um(
                        before.2, 100, 4.5, 2_500,
                    )
                    .unwrap();
                    let safe =
                        before.0 >= 3 || near_boundary_mm * 1_000 >= stop_um + u64::from(before.3);
                    outcomes.push((workers, looping, tick, before.0, near_boundary_mm, stop_um));
                    assert!(
                        safe,
                        "P4 不得在近端下游边界不可安全让行时授予换道: {outcomes:?}"
                    );
                    entered = Some(tick);
                    break;
                }
            }
            assert!(
                entered.is_some(),
                "changer must eventually enter: workers={workers}, looping={looping}"
            );
            match baselines.iter_mut().find(|(known, _)| *known == looping) {
                Some((_, baseline)) => {
                    assert_eq!(
                        &trace, baseline,
                        "workers={workers}, looping={looping} 与单 worker 基线不一致"
                    );
                }
                None => baselines.push((looping, trace)),
            }
        }
    }
    // 验收第 2 项：截去未来 T 的对照与循环路线在共享前缀期间使用相同近端证明
    // 边界——两份基线都在换道车首次进入处截断，授予时点一致则换道车逐拍全等。
    // #1272：对照路线更短，直行车接近路终时按新语义提前制动，末端位置可以
    // 不同；近端证明边界的等价性由换道车轨迹承载。
    let changer_trace = |trace: &[_]| {
        trace
            .iter()
            .map(|(edge, progress, _, _)| (*edge, *progress))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        changer_trace(&baselines[0].1),
        changer_trace(&baselines[1].1),
        "对照与循环路线必须沿用同一近端证明边界"
    );
}

/// #1321 复现族：merge_topology 之上为 T1/T2 追加独立后继分支。`t1_extra`/
/// `t2_extra` 取 ""（不加）、"c"（挂 c 链）或 "d"（挂 d 链）。两条独立下游
/// 链总是声明——悬空边不进入未被选为后继一侧的邻接（f1/f2 悬空先例）。
fn branched_merge_topology(t1_extra: &str, t2_extra: &str) -> Topology {
    let branch = |key: &str, extra: &str| -> &'static [&'static str] {
        match (key, extra) {
            (_, "") => &["e1b"],
            (_, "c") => &["e1b", "review-c"],
            (_, "d") => &["e1b", "review-d"],
            _ => unreachable!("未知分支组合"),
        }
    };
    let mut topology = merge_topology();
    for spec in &mut topology.edges {
        if spec.key == "t1" {
            spec.successors = branch("t1", t1_extra);
        }
        if spec.key == "t2" {
            spec.successors = branch("t2", t2_extra);
        }
    }
    topology.edges.extend([
        edge("review-c", (30.0, 3.0), (63.0, 0.0), &[]),
        edge("review-d", (30.0, 3.0), (65.0, 6.0), &[]),
        edge("review-ic", (63.0, 0.0), (71.0, 0.0), &[]),
        edge("review-oc", (71.0, 0.0), (81.0, 0.0), &[]),
        edge("review-id", (65.0, 6.0), (74.0, 6.0), &[]),
        edge("review-od", (74.0, 6.0), (85.0, 6.0), &[]),
        edge("review-fc", (29.0, 0.0), (63.0, 0.0), &[]),
        edge("review-fd", (28.0, 6.0), (65.0, 6.0), &[]),
    ]);
    topology.lanes.extend([
        ("review-lane-c", vec!["review-c"]),
        ("review-lane-d", vec!["review-d"]),
        ("review-lane-fc", vec!["review-fc"]),
        ("review-lane-fd", vec!["review-fd"]),
    ]);
    topology.junctions.extend([
        JunctionSpec {
            key: "review-jc",
            entry: "review-c",
            second_entry: "review-fc",
            internal: "review-ic",
            exit: "review-oc",
        },
        JunctionSpec {
            key: "review-jd",
            entry: "review-d",
            second_entry: "review-fd",
            internal: "review-id",
            exit: "review-od",
        },
    ]);
    topology
}

/// #1321 ①：T1、T2 共享可选后继 e1b，两条路线都改走各自独立后继——图上
/// 允许的分支不受 P4/P5 覆盖，两条过渡路线均在注册阶段被结构化拒绝
/// （未修复基线上两车同拍进入，见 issue 实测 tick 12、workers 1/2/4）。
#[test]
fn unused_common_successor_with_independent_branches_is_rejected() {
    let revision = build(&branched_merge_topology("c", "d"));
    let route_a = [
        named(&revision, "e0a"),
        named(&revision, "t1"),
        named(&revision, "review-c"),
        named(&revision, "review-ic"),
        named(&revision, "review-oc"),
    ];
    let route_b = [
        named(&revision, "e2a"),
        named(&revision, "t2"),
        named(&revision, "review-d"),
        named(&revision, "review-id"),
        named(&revision, "review-od"),
    ];
    let straight = [
        named(&revision, "e0a"),
        named(&revision, "s0"),
        named(&revision, "e0b"),
    ];
    assert_rejected(&revision, &[&route_a, &route_b], &straight);
}

/// #1321 ②：T1、T2 出口集合相等但含两个出口 {e1b, review-c}，实际路线各选
/// 其一——集合相等不等于共同唯一出口，各自分流会让两份过渡预约的下游空间
/// 不相交（循环互等），必须拒绝。
#[test]
fn equal_multi_exit_sets_are_rejected_without_unique_common_exit() {
    let revision = build(&branched_merge_topology("c", "c"));
    let via_common = [
        named(&revision, "e0a"),
        named(&revision, "t1"),
        named(&revision, "e1b"),
        named(&revision, "int"),
        named(&revision, "exit"),
    ];
    let via_branch = [
        named(&revision, "e2a"),
        named(&revision, "t2"),
        named(&revision, "review-c"),
        named(&revision, "review-ic"),
        named(&revision, "review-oc"),
    ];
    let straight = [
        named(&revision, "e0a"),
        named(&revision, "s0"),
        named(&revision, "e0b"),
    ];
    assert_rejected(&revision, &[&via_common, &via_branch], &straight);
}

/// #1321 ③：只有一侧追加独立出口同样拒绝——放行证明要求双向出口覆盖，
/// 单侧未覆盖即不成立；两个方向分别钉住。
#[test]
fn one_sided_independent_successor_is_rejected_in_both_directions() {
    for (t1_extra, t2_extra) in [("c", ""), ("", "d")] {
        let revision = build(&branched_merge_topology(t1_extra, t2_extra));
        let route_a = [
            named(&revision, "e0a"),
            named(&revision, "t1"),
            named(&revision, "e1b"),
            named(&revision, "int"),
            named(&revision, "exit"),
        ];
        let route_b = [
            named(&revision, "e2a"),
            named(&revision, "t2"),
            named(&revision, "e1b"),
            named(&revision, "int"),
            named(&revision, "exit"),
        ];
        let straight = [
            named(&revision, "e0a"),
            named(&revision, "s0"),
            named(&revision, "e0b"),
        ];
        assert_rejected(&revision, &[&route_a, &route_b], &straight);
    }
}

/// #1321 ④：#1321 图上即使两条路线实际都走公共后继 e1b，注册仍然拒绝——
/// 覆盖证明是图级的（图允许未覆盖分支即无法证明），注册拒绝是控制点。
#[test]
fn shared_branch_graph_rejects_even_routes_using_it() {
    let revision = build(&branched_merge_topology("c", "d"));
    let route_a = [
        named(&revision, "e0a"),
        named(&revision, "t1"),
        named(&revision, "e1b"),
        named(&revision, "int"),
        named(&revision, "exit"),
    ];
    let route_b = [
        named(&revision, "e2a"),
        named(&revision, "t2"),
        named(&revision, "e1b"),
        named(&revision, "int"),
        named(&revision, "exit"),
    ];
    let straight = [
        named(&revision, "e0a"),
        named(&revision, "s0"),
        named(&revision, "e0b"),
    ];
    assert_rejected(&revision, &[&route_a, &route_b], &straight);
}

/// #1321 ⑤⑥：T1、T2 共享前驱 e0a（分岔型）。`shared_exit` 为真时两者还共享
/// 唯一出口 e1b（出口全同）——分岔拒绝必须优先于汇合放行成立；为假时 T2 改走
/// 独立出口 e2b。现行 P4/P5 对已进入对方过渡段、位于共享入口附近的车没有证明
/// 手段，两种形态都拒绝。补 j2 让 e2b 下游与 e1b 一样有 junction 机动流
/// （§8 流绑定：纯过渡路线会被 ManeuverMismatch 先拒，测不到能力判定）。
fn shared_predecessor_topology(shared_exit: bool) -> Topology {
    let mut topology = merge_topology();
    for spec in &mut topology.edges {
        if spec.key == "e0a" {
            spec.successors = &["s0", "t1", "t2"];
        }
        if spec.key == "e2a" {
            spec.successors = &["s2"];
        }
        if spec.key == "t2" {
            // 从共享前驱 e0a 的终点 (26,0) 出发（后继接续要求几何连续）；
            // 共享出口时终于 e1b 起点 (30,3)（与 t1 中心线重合，两端被
            // 接续关系钉死），独立出口时终于 e2b 起点 (30,6)。
            *spec = if shared_exit {
                edge("t2", (26.0, 0.0), (30.0, 3.0), &["e1b"])
            } else {
                edge("t2", (26.0, 0.0), (30.0, 6.0), &["e2b"])
            };
        }
    }
    topology.edges.extend([
        edge("f2", (32.0, 6.0), (62.0, 6.0), &[]),
        edge("int2", (62.0, 6.0), (70.0, 6.0), &[]),
        edge("exit2", (70.0, 6.0), (80.0, 6.0), &[]),
    ]);
    topology.lanes.push(("lane-f2", vec!["f2"]));
    topology.junctions.push(JunctionSpec {
        key: "j2",
        entry: "e2b",
        second_entry: "f2",
        internal: "int2",
        exit: "exit2",
    });
    topology
}

/// #1321 ⑤：共享前驱、独立后继——分岔分支专测。
#[test]
fn shared_predecessor_with_independent_exits_is_rejected() {
    let revision = build(&shared_predecessor_topology(false));
    let route_a = [
        named(&revision, "e0a"),
        named(&revision, "t1"),
        named(&revision, "e1b"),
        named(&revision, "int"),
        named(&revision, "exit"),
    ];
    let route_b = [
        named(&revision, "e0a"),
        named(&revision, "t2"),
        named(&revision, "e2b"),
        named(&revision, "int2"),
        named(&revision, "exit2"),
    ];
    let straight = [
        named(&revision, "e0a"),
        named(&revision, "s0"),
        named(&revision, "e0b"),
    ];
    assert_rejected(&revision, &[&route_a, &route_b], &straight);
}

/// #1321 ⑥：共享前驱且出口全同（S3 并存对同构）——即使出口覆盖条件满足，
/// 分岔拒绝优先成立，不得被汇合放行遮蔽。
#[test]
fn shared_predecessor_wins_over_full_exit_coverage() {
    let revision = build(&shared_predecessor_topology(true));
    let route_a = [
        named(&revision, "e0a"),
        named(&revision, "t1"),
        named(&revision, "e1b"),
        named(&revision, "int"),
        named(&revision, "exit"),
    ];
    let route_b = [
        named(&revision, "e0a"),
        named(&revision, "t2"),
        named(&revision, "e1b"),
        named(&revision, "int"),
        named(&revision, "exit"),
    ];
    let straight = [
        named(&revision, "e0a"),
        named(&revision, "s0"),
        named(&revision, "e0b"),
    ];
    assert_rejected(&revision, &[&route_a, &route_b], &straight);
}

/// #1321 ⑦：exits(F1)={m}、entries(F2)={m} 看似单锚接力，但 m 同时是 F2 的
/// 出口（t2 的后继回环到 m，exits(F2)={m,b}）——汇合分支必须终决，出口集合
/// 不相等即拒绝，不得回落接力放行。b 下游补 junction 机动流满足 §8 流绑定。
fn looped_relay_topology() -> Topology {
    Topology {
        edges: vec![
            edge("e0a", (0.0, 0.0), (26.0, 0.0), &["s0", "t1"]),
            edge("s0", (26.0, 0.0), (30.0, 0.0), &["e0b"]),
            edge("e0b", (30.0, 0.0), (60.0, 0.0), &[]),
            edge("t1", (26.0, 0.0), (30.0, 3.0), &["m"]),
            edge("m", (30.0, 3.0), (40.0, 0.0), &["t2"]),
            edge("t2", (40.0, 0.0), (30.0, 3.0), &["m", "b"]),
            edge("b", (30.0, 3.0), (38.0, 3.0), &[]),
            edge("bf", (32.0, 3.0), (38.0, 3.0), &[]),
            edge("intb", (38.0, 3.0), (46.0, 3.0), &[]),
            edge("exitb", (46.0, 3.0), (56.0, 3.0), &[]),
        ],
        lanes: vec![
            ("lane-0", vec!["e0a", "s0", "e0b"]),
            ("lane-m", vec!["m"]),
            ("lane-b", vec!["b"]),
            ("lane-bf", vec!["bf"]),
        ],
        junctions: vec![JunctionSpec {
            key: "jb",
            entry: "b",
            second_entry: "bf",
            internal: "intb",
            exit: "exitb",
        }],
    }
}

#[test]
fn failed_merge_coverage_does_not_fall_back_to_relay() {
    let revision = build(&looped_relay_topology());
    let through = [
        named(&revision, "e0a"),
        named(&revision, "t1"),
        named(&revision, "m"),
        named(&revision, "t2"),
        named(&revision, "b"),
        named(&revision, "intb"),
        named(&revision, "exitb"),
    ];
    let straight = [
        named(&revision, "e0a"),
        named(&revision, "s0"),
        named(&revision, "e0b"),
    ];
    assert_rejected(&revision, &[&through], &straight);
}

/// #1321 ⑧：两条多段过渡链 {a1,a2}、{b1,b2} 首段即交叉（(2.5,3) 附近）、
/// 尾部共享唯一出口 x——运行时保护单位是成员边，分量级出口全同不足以证明
/// 首段交叉受覆盖，多成员配对保守拒绝。x 下游补 junction 机动流满足 §8
/// 流绑定。
fn crossing_chains_topology() -> Topology {
    Topology {
        edges: vec![
            edge("pa", (-20.0, 0.0), (0.0, 0.0), &["a1", "sa"]),
            edge("sa", (0.0, 0.0), (10.0, 0.0), &["sb"]),
            edge("sb", (10.0, 0.0), (20.0, 0.0), &[]),
            edge("a1", (0.0, 0.0), (5.0, 6.0), &["a2"]),
            edge("a2", (5.0, 6.0), (10.0, 3.0), &["x"]),
            edge("x", (10.0, 3.0), (20.0, 3.0), &[]),
            edge("pb", (-5.0, 6.0), (0.0, 6.0), &["b1"]),
            edge("b1", (0.0, 6.0), (5.0, 0.0), &["b2"]),
            edge("b2", (5.0, 0.0), (10.0, 3.0), &["x"]),
            edge("xf", (12.0, 3.0), (20.0, 3.0), &[]),
            edge("intx", (20.0, 3.0), (28.0, 3.0), &[]),
            edge("exitx", (28.0, 3.0), (38.0, 3.0), &[]),
        ],
        lanes: vec![
            ("lane-0", vec!["pa", "sa", "sb"]),
            ("lane-1", vec!["pb"]),
            ("lane-x", vec!["x"]),
            ("lane-xf", vec!["xf"]),
        ],
        junctions: vec![JunctionSpec {
            key: "jx",
            entry: "x",
            second_entry: "xf",
            internal: "intx",
            exit: "exitx",
        }],
    }
}

#[test]
fn multi_member_components_sharing_unique_exit_are_rejected() {
    let revision = build(&crossing_chains_topology());
    let route_a = [
        named(&revision, "pa"),
        named(&revision, "a1"),
        named(&revision, "a2"),
        named(&revision, "x"),
        named(&revision, "intx"),
        named(&revision, "exitx"),
    ];
    let route_b = [
        named(&revision, "pb"),
        named(&revision, "b1"),
        named(&revision, "b2"),
        named(&revision, "x"),
        named(&revision, "intx"),
        named(&revision, "exitx"),
    ];
    let straight = [
        named(&revision, "pa"),
        named(&revision, "sa"),
        named(&revision, "sb"),
    ];
    assert_rejected(&revision, &[&route_a, &route_b], &straight);
}

/// #1321 ⑨：exits(F1)={m,c}、entries(F2)={m,d}——图上存在接力关系 m，但
/// F1 可走 c 绕过 m、F2 可从 d 进入绕过 m，单锚必经不成立，拒绝。b 下游补
/// junction 机动流满足 §8 流绑定。
fn bypassable_relay_topology() -> Topology {
    Topology {
        edges: vec![
            edge("e0a", (0.0, 0.0), (26.0, 0.0), &["s0", "t1"]),
            edge("s0", (26.0, 0.0), (30.0, 0.0), &["e0b"]),
            edge("e0b", (30.0, 0.0), (60.0, 0.0), &[]),
            edge("t1", (26.0, 0.0), (30.0, 3.0), &["m", "c"]),
            edge("m", (30.0, 3.0), (40.0, 3.0), &["t2"]),
            edge("c", (30.0, 3.0), (40.0, 1.0), &["cb"]),
            edge("cb", (40.0, 1.0), (50.0, 1.0), &[]),
            edge("d", (34.0, 6.0), (40.0, 3.0), &["t2"]),
            edge("t2", (40.0, 3.0), (44.0, 6.0), &["b"]),
            edge("b", (44.0, 6.0), (54.0, 6.0), &[]),
            edge("bf", (46.0, 6.0), (54.0, 6.0), &[]),
            edge("intb", (54.0, 6.0), (62.0, 6.0), &[]),
            edge("exitb", (62.0, 6.0), (72.0, 6.0), &[]),
        ],
        lanes: vec![
            ("lane-0", vec!["e0a", "s0", "e0b"]),
            ("lane-m", vec!["m"]),
            ("lane-c", vec!["c", "cb"]),
            ("lane-d", vec!["d"]),
            ("lane-b", vec!["b"]),
            ("lane-bf", vec!["bf"]),
        ],
        junctions: vec![JunctionSpec {
            key: "jb",
            entry: "b",
            second_entry: "bf",
            internal: "intb",
            exit: "exitb",
        }],
    }
}

#[test]
fn relay_shared_edge_with_bypass_branches_is_rejected() {
    let revision = build(&bypassable_relay_topology());
    let through = [
        named(&revision, "e0a"),
        named(&revision, "t1"),
        named(&revision, "m"),
        named(&revision, "t2"),
        named(&revision, "b"),
        named(&revision, "intb"),
        named(&revision, "exitb"),
    ];
    let straight = [
        named(&revision, "e0a"),
        named(&revision, "s0"),
        named(&revision, "e0b"),
    ];
    assert_rejected(&revision, &[&through], &straight);
}

/// #1506 评审（整数几何 §5.1）：常规候选已零速恰好到达过渡预约边界、过渡
/// 实际限定 `hard_room`，舒适耗尽速度仍为正时，到位零速必须优先。档案
/// `b_c = 4.5`、`b_e = 9.0`；`V = 900 mm/s`、`D = 45 mm`、`Δt = 100 ms`：
/// `Φ_e(V) = 45000 µm = 1000·D`，紧急档拍内恰好停稳；舒适耗尽速度为 450。
#[test]
fn transition_bound_arrival_commits_zero_speed() {
    let mut topology = cross_topology();
    for spec in &mut topology.edges {
        if spec.key == "e2a" {
            spec.successors = &["s2"];
        }
    }
    topology.edges.retain(|spec| spec.key != "t2");
    let revision = build(&topology);
    let lengths = revision.traffic().lane_lengths_millimetres().to_vec();
    let get = |length| by_length(&revision, length);
    let (e0a, e2a) = (get(26_000), get(27_000));
    let t = transition_after(&revision, e0a);
    let s2 = straight_after(&revision, e2a);
    let (e2b, int1, exit1) = (get(31_000), get(8_000), get(10_000));
    for carry in [0_u16, 400] {
        let mut world = install(revision.clone(), 1);
        let mut spawn = |edges: Vec<LaneEdgeOrdinal>, progress_mm: u32| {
            let handle = world
                .register_route(crate::RouteRegisterInput::new(edges))
                .expect("route registers");
            world
                .spawn_vehicle(
                    crate::VehicleSpawnInput::new(
                        laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                        handle,
                        0,
                        progress_mm,
                        SPEED_MM_S,
                    )
                    .with_open_entrance(),
                )
                .expect("spawn")
        };
        let changer = spawn(vec![e0a, t, e2b, int1, exit1], 15_000);
        let straight = spawn(vec![e2a, s2, e2b, int1, exit1], 16_000);
        world.state.rebuild_occupancy_index().unwrap();
        let mut entered = false;
        for _ in 0..100 {
            world.state.prepare_waiting_step(0.1).unwrap();
            world.step(crate::TickInput::new(100)).unwrap();
            let changer = world.state.committed.vehicles.state(changer).unwrap();
            if changer.route_edge_index == 1 {
                entered = true;
                break;
            }
        }
        assert!(entered, "换道车进入过渡段并持有过渡资源");
        {
            let mut slot = world
                .state
                .committed
                .vehicles
                .slot_mut(straight.index() as usize);
            let state = slot.state.as_mut().unwrap();
            state.route_edge_index = 1;
            state.progress_mm = lengths[s2.index()] - 45;
            state.speed_mm_s = 900;
            state.carry_um = carry;
        }
        world.state.rebuild_occupancy_index().unwrap();
        world.state.prepare_waiting_step(0.1).unwrap();
        world.step(crate::TickInput::new(100)).unwrap();
        let state = world.state.committed.vehicles.state(straight).unwrap();
        assert_eq!(
            (
                state.route_edge_index,
                state.progress_mm,
                state.speed_mm_s,
                state.carry_um
            ),
            (1, lengths[s2.index()], 0, 0),
            "C {carry}：恰好到达过渡边界并零速"
        );
    }
}

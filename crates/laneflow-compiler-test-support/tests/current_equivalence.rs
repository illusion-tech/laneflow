//! `LF-COMP-CURRENT-EQUIV-v1` 的两个冻结迁移样例。
//!
//! 当前 JSON loader 和 Synthetic DSL 前端分别消费同一固定制品；只有后者进入
//! `Compiler`。投影函数仍只看 `ValidatedCanonicalLir`，不会从当前对象图补语义。

use std::{collections::BTreeMap, hint::black_box, time::Instant};

use laneflow_compiler::{
    AccessEffect, AccessRegulationInput, AccessRuleInput, AccessRuleTargetInput,
    AuthoringLaneInput, CanonicalFrameInput, CanonicalPoint3F32Input, CompilationOutput,
    CompilationUnitBuilder, CompileLimits, Compiler, CorridorElementReference, FacilityBandInput,
    FacilityBandReference, IidmVehicleProfileInput, JunctionInput, JunctionReference,
    LaneEdgeGeometryInput, LaneEdgeInput, LaneEdgeReference, LaneGroupInput, LaneGroupReference,
    ManeuverGateInput, ManeuverGateReference, ManeuverPathInput, ManeuverPathReference,
    MovementInput, MovementReference, ParkingAreaInput, ParkingAreaReference,
    ParkingLaneAnchorInput, ParkingSpaceGeometryInput, ParkingSpaceInput, ParticipantClassInput,
    ParticipantClassReference, RoadCorridorInput, RoadSectionInput, RoadSectionReference,
    SignalAspect, SignalControlInput, SignalControllerInput, SignalGroupInput,
    SignalGroupReference, SignalGroupStateInput, SignalPhaseInput, SourceModuleHeader,
    SourceModuleHeaderInput, StaticRouteInput, StopLineInput, StopLineReference,
    SyntheticModuleBuilder, VehicleProfileInput, WaitingZoneInput,
};
use laneflow_compiler_test_support::project;
use laneflow_core::{
    AccessTargetId, CoreEvent, CoreWorld, CorridorElementId, EdgeProgress, InitialTrafficData,
    SignalControlInput as CoreSignalControlInput, Speed, TickInput, VehicleSpawnInput,
};
use laneflow_data::{NamedArtifact, from_json_slice, from_scenario_json_slice};
use laneflow_spatial::{SpatialEdgeInput, SpatialRegistry};
use laneflow_static_contract::{EntityKind, FieldTag};
use serde_json::{Value, json};

const TRAFFIC_NAMESPACE: &str = "fixture/traffic";
const SPATIAL_NAMESPACE: &str = "fixture/spatial";
const SIGNALIZED_TRAFFIC_REF: &str = "v0.10-signalized-corridor.laneflow.json";
const SIGNALIZED_SPATIAL_REF: &str = "v0.1-signalized-corridor.spatial.json";
const SIGNALIZED_TRAFFIC: &[u8] =
    include_bytes!("../../../examples/data/v0.10-signalized-corridor.laneflow.json");
const SIGNALIZED_SPATIAL: &[u8] =
    include_bytes!("../../../examples/data/v0.1-signalized-corridor.spatial.json");
const SIGNALIZED_MANIFEST: &[u8] =
    include_bytes!("../../../examples/data/v0.1-signalized-corridor.scenario.json");
const MULTI_GATE_TRAFFIC: &[u8] =
    include_bytes!("../../../examples/data/v0.10-multi-gate-waiting-zone.laneflow.json");
const PRODUCTION_BASELINE_ID: &str = "LF-COMP-P100-PRODUCTION-R0-v1";
const PRODUCTION_BASELINE_WORKLOAD_ID: &str = "LF-COMP-PRODUCTION-CORRIDOR-v1";
const PRODUCTION_BASELINE_SCALES: [usize; 5] = [1, 2, 3, 4, 5];
const PRODUCTION_BASELINE_SAMPLE_COUNT: usize = 7;

#[test]
fn signalized_corridor_projects_the_complete_current_static_contract() {
    let current = from_scenario_json_slice(
        SIGNALIZED_MANIFEST,
        &[
            NamedArtifact::new(SIGNALIZED_TRAFFIC_REF, SIGNALIZED_TRAFFIC),
            NamedArtifact::new(SIGNALIZED_SPATIAL_REF, SIGNALIZED_SPATIAL),
        ],
    )
    .unwrap();
    let output = compile_current_fixture(SIGNALIZED_TRAFFIC, Some(SIGNALIZED_SPATIAL));
    let projection = project(output.lir()).unwrap();
    let aliases = stable_id_aliases(output.lir());

    assert_eq!(
        traffic_snapshot(current.traffic().initial_traffic_data(), &BTreeMap::new()),
        traffic_snapshot(projection.traffic(), &aliases)
    );
    let original_graph = current.traffic().initial_traffic_data().lane_graph();
    let original_spatial = SpatialRegistry::try_new(
        original_graph,
        current.spatial().frame_id().clone(),
        current
            .spatial()
            .edges()
            .iter()
            .map(|edge| SpatialEdgeInput::new(edge.edge(), edge.points())),
    )
    .unwrap();
    let projected_spatial = projection.spatial().unwrap();
    assert_eq!(projected_spatial.len(), 66);
    for edge in original_graph.edges() {
        let original_handle = original_graph.edge_handle(edge.id()).unwrap();
        let projected_id = aliases
            .iter()
            .find_map(|(stable_id, original_id)| (original_id == edge.id()).then_some(stable_id))
            .unwrap();
        let projected_handle = projection
            .traffic()
            .lane_graph()
            .edge_handle(projected_id)
            .unwrap();
        for progress in [0.0, edge.length().value() / 2.0, edge.length().value()] {
            let progress = EdgeProgress::try_new(progress).unwrap();
            assert_eq!(
                original_spatial.sample(original_handle, progress).unwrap(),
                projected_spatial
                    .sample(projected_handle, progress)
                    .unwrap()
            );
        }
    }
    assert_eq!(
        projection
            .mappings()
            .entries_for(EntityKind::CanonicalFrame)
            .count(),
        1
    );
    assert_entity_counts(
        output.lir(),
        [
            66, 7, 14, 34, 6, 7, 2, 24, 32, 20, 32, 0, 8, 2, 24, 0, 0, 3, 2, 18, 28, 1,
        ],
    );
    assert_runtime_equivalence(
        current.traffic().initial_traffic_data().clone(),
        projection.traffic().clone(),
        &aliases,
    );

    // 新建 Compiler 会取得新的标准库 HashMap 随机种子；完整重编译仍必须产生同一规范
    // 静态语义与稳定映射，不能让进程内散列表顺序泄漏到迁移结果。
    let repeated_output = compile_current_fixture(SIGNALIZED_TRAFFIC, Some(SIGNALIZED_SPATIAL));
    let repeated_projection = project(repeated_output.lir()).unwrap();
    let repeated_aliases = stable_id_aliases(repeated_output.lir());
    assert_eq!(aliases, repeated_aliases);
    assert_eq!(
        traffic_snapshot(projection.traffic(), &aliases),
        traffic_snapshot(repeated_projection.traffic(), &repeated_aliases)
    );
}

#[test]
fn multi_gate_waiting_zone_preserves_gate_and_waiting_occurrences() {
    let current = from_json_slice(MULTI_GATE_TRAFFIC).unwrap();
    let output = compile_current_fixture(MULTI_GATE_TRAFFIC, None);
    let projection = project(output.lir()).unwrap();
    let aliases = stable_id_aliases(output.lir());

    assert_eq!(
        traffic_snapshot(current.initial_traffic_data(), &BTreeMap::new()),
        traffic_snapshot(projection.traffic(), &aliases)
    );
    assert!(projection.spatial().is_none());
    let route = output.lir().static_routes().next().unwrap();
    assert_eq!(route.gate_occurrences().len(), 3);
    assert_eq!(route.waiting_zone_occurrences().len(), 2);
    assert_entity_counts(
        output.lir(),
        [
            4, 0, 0, 0, 0, 0, 1, 1, 1, 3, 3, 2, 0, 0, 0, 0, 0, 1, 1, 0, 1, 0,
        ],
    );
}

/// 在 P100 推荐参考机型上生成 #292 首轮生产编译紧凑基线。
///
/// 该测试默认忽略，避免普通回归把墙钟测量误当成功能门禁。规模输入与
/// `CompilationUnit` 均在计时区外建立；唯一计时区只覆盖 `Compiler::compile`。每级
/// 只保留 min/median/max，不保存逐样本 raw，也不启动隔离子进程。
#[test]
#[ignore = "只在 #292 生产性能基线重测时以 release 单线程显式运行"]
fn p100_production_compiler_baseline() {
    let traffic: Value = serde_json::from_slice(SIGNALIZED_TRAFFIC).unwrap();
    let spatial: Value = serde_json::from_slice(SIGNALIZED_SPATIAL).unwrap();
    let mut compiler = Compiler::new();
    let mut levels = Vec::with_capacity(PRODUCTION_BASELINE_SCALES.len());

    for (level_index, copies) in PRODUCTION_BASELINE_SCALES.into_iter().enumerate() {
        eprintln!(
            "生产编译基线进度：第 {}/{} 级，{} 份完整信号化走廊",
            level_index + 1,
            PRODUCTION_BASELINE_SCALES.len(),
            copies
        );

        // 预热只消除首次执行固定成本；它不进入七个正式样本，也不改变 Compiler 的
        // retained capacity（当前实现恒为零）。
        let warmup = repeated_signalized_corridor_unit(&traffic, &spatial, copies);
        black_box(compiler.compile(warmup).unwrap());

        let mut elapsed_ns = Vec::with_capacity(PRODUCTION_BASELINE_SAMPLE_COUNT);
        let mut expected_metrics = None;
        let mut lane_edge_count = 0_usize;
        for _ in 0..PRODUCTION_BASELINE_SAMPLE_COUNT {
            // 前端 JSON 解析、DSL 构造和规模复制均明确留在计时区外。
            let unit = repeated_signalized_corridor_unit(&traffic, &spatial, copies);
            let started = Instant::now();
            let output = compiler.compile(black_box(unit)).unwrap();
            let duration_ns = u64::try_from(started.elapsed().as_nanos()).unwrap();

            // 指标读取与确定性判定发生在停表后，不能污染 compile wall clock。
            let metrics = output.metrics();
            if let Some(expected) = expected_metrics {
                assert_eq!(metrics, expected, "同一级重复编译必须产生相同观测值");
            } else {
                expected_metrics = Some(metrics);
            }
            assert_eq!(compiler.retained_capacity_bytes(), 0);
            lane_edge_count = output.lir().lane_edges().len();
            elapsed_ns.push(duration_ns);
            black_box(output);
        }

        elapsed_ns.sort_unstable();
        let metrics = expected_metrics.unwrap();
        assert_eq!(lane_edge_count, 66 * copies);
        levels.push(json!({
            "corridorCopies": copies,
            "sourceModuleCount": copies * 2,
            "laneEdgeCount": lane_edge_count,
            "formalSampleCount": PRODUCTION_BASELINE_SAMPLE_COUNT,
            "wallClockNs": {
                "min": elapsed_ns[0],
                "median": elapsed_ns[PRODUCTION_BASELINE_SAMPLE_COUNT / 2],
                "max": elapsed_ns[PRODUCTION_BASELINE_SAMPLE_COUNT - 1]
            },
            "lirRecordCount": metrics.lir_record_count(),
            "lirOutputLogicalBytes": metrics.output_logical_bytes(),
            "compilerControlledPeakBytes": metrics.compiler_controlled_peak_bytes(),
            "compilerRetainedCapacityBytes": compiler.retained_capacity_bytes(),
            "semanticFingerprint": encode_hex(&metrics.semantic_fingerprint())
        }));
    }

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schemaVersion": 1,
            "baselineId": PRODUCTION_BASELINE_ID,
            "workloadId": PRODUCTION_BASELINE_WORKLOAD_ID,
            "compileLimitsProfile": CompileLimits::p100_initial_v1().profile_id(),
            "timingBoundary": "Compiler::compile only",
            "samplePolicy": "one warmup plus seven formal samples per level; single process; one worker",
            "levels": levels
        }))
        .unwrap()
    );
}

fn repeated_signalized_corridor_unit(
    traffic: &Value,
    spatial: &Value,
    copies: usize,
) -> laneflow_compiler::CompilationUnit {
    let limits = CompileLimits::p100_initial_v1();
    let mut unit = CompilationUnitBuilder::new(limits.clone());
    for index in 0..copies {
        let traffic_namespace = format!("baseline/traffic/{index:02}");
        let spatial_namespace = format!("baseline/spatial/{index:02}");
        let traffic_document = format!("baseline/current-traffic-{index:02}.lfsynthetic");
        let spatial_document = format!("baseline/current-spatial-{index:02}.lfsynthetic");
        unit.add_synthetic_module(build_traffic_module_for(
            traffic,
            &limits,
            &traffic_namespace,
            &traffic_document,
        ))
        .unwrap();
        unit.add_synthetic_module(build_spatial_module_for(
            spatial,
            &limits,
            &spatial_namespace,
            &traffic_namespace,
            &spatial_document,
        ))
        .unwrap();
    }
    unit.build().unwrap()
}

fn encode_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut output, "{byte:02x}").unwrap();
    }
    output
}

fn compile_current_fixture(
    traffic_bytes: &[u8],
    spatial_bytes: Option<&[u8]>,
) -> CompilationOutput {
    let traffic: Value = serde_json::from_slice(traffic_bytes).unwrap();
    let limits = CompileLimits::p100_initial_v1();
    let traffic_module = build_traffic_module(&traffic, &limits);
    let mut unit = CompilationUnitBuilder::new(limits.clone());
    unit.add_synthetic_module(traffic_module).unwrap();
    if let Some(spatial_bytes) = spatial_bytes {
        let spatial: Value = serde_json::from_slice(spatial_bytes).unwrap();
        unit.add_synthetic_module(build_spatial_module(&spatial, &limits))
            .unwrap();
    }
    Compiler::new().compile(unit.build().unwrap()).unwrap()
}

fn build_traffic_module(
    traffic: &Value,
    limits: &CompileLimits,
) -> laneflow_compiler::SyntheticModule {
    build_traffic_module_for(
        traffic,
        limits,
        TRAFFIC_NAMESPACE,
        "fixture/current-traffic.lfsynthetic",
    )
}

fn build_traffic_module_for(
    traffic: &Value,
    limits: &CompileLimits,
    traffic_namespace: &str,
    source_document_key: &str,
) -> laneflow_compiler::SyntheticModule {
    let mut builder = SyntheticModuleBuilder::new(
        header(traffic_namespace, source_document_key, limits),
        limits,
    )
    .unwrap();

    for edge in array(&traffic["laneGraph"], "edges") {
        let successors = array(edge, "connections")
            .iter()
            .map(|connection| LaneEdgeReference::local(text(connection, "toEdgeId")))
            .collect::<Vec<_>>();
        builder
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: text(edge, "id"),
                length_meters: number(edge, "length"),
                speed_limit_meters_per_second: number(edge, "speedLimit"),
                successors: &successors,
            })
            .unwrap();
    }

    for band in array(traffic, "facilityBands") {
        builder
            .add_facility_band(FacilityBandInput {
                facility_band_key: text(band, "id"),
                kind_id: text(band, "kindId"),
            })
            .unwrap();
    }
    for group in array(traffic, "laneGroups") {
        builder
            .add_lane_group(LaneGroupInput {
                lane_group_key: text(group, "id"),
                road_section: RoadSectionReference::local(text(group, "roadSectionId")),
            })
            .unwrap();
    }
    for section in array(traffic, "roadSections") {
        let lanes = array(section, "lanes");
        let lane_keys = lanes
            .iter()
            .enumerate()
            .map(|(index, _)| format!("{}/lane/{index}", text(section, "id")))
            .collect::<Vec<_>>();
        let edge_chains = lanes
            .iter()
            .map(|lane| {
                string_array(lane, "edgeIds")
                    .map(LaneEdgeReference::local)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let lane_inputs = lanes
            .iter()
            .enumerate()
            .map(|(index, lane)| AuthoringLaneInput {
                authoring_lane_key: &lane_keys[index],
                edge_chain: &edge_chains[index],
                lane_group: optional_text(lane, "laneGroupId").map(LaneGroupReference::local),
            })
            .collect::<Vec<_>>();
        builder
            .add_road_section(RoadSectionInput {
                road_section_key: text(section, "id"),
                kind_id: text(section, "kindId"),
                lanes: &lane_inputs,
            })
            .unwrap();
    }
    for corridor in array(traffic, "roadCorridors") {
        let elements = array(corridor, "elements")
            .iter()
            .map(|element| {
                if let Some(id) = optional_text(element, "sectionId") {
                    CorridorElementReference::road_section(RoadSectionReference::local(id))
                } else {
                    CorridorElementReference::facility_band(FacilityBandReference::local(text(
                        element, "bandId",
                    )))
                }
            })
            .collect::<Vec<_>>();
        builder
            .add_road_corridor(RoadCorridorInput {
                road_corridor_key: text(corridor, "id"),
                reference_section: RoadSectionReference::local(text(
                    corridor,
                    "referenceSectionId",
                )),
                elements: &elements,
            })
            .unwrap();
    }

    for junction in array(traffic, "junctions") {
        builder
            .add_junction(JunctionInput {
                junction_key: text(junction, "id"),
            })
            .unwrap();
    }
    for movement in array(traffic, "movements") {
        let movement_id = text(movement, "id");
        let entry = format!("{movement_id}/entry");
        let exit = format!("{movement_id}/exit");
        builder
            .add_movement(MovementInput {
                movement_key: movement_id,
                junction: JunctionReference::local(text(movement, "junctionId")),
                directed_entry_approach_key: &entry,
                directed_exit_approach_key: &exit,
            })
            .unwrap();
    }
    for path in array(traffic, "maneuverPaths") {
        let internal_edges = string_array(path, "internalEdgeIds")
            .map(LaneEdgeReference::local)
            .collect::<Vec<_>>();
        builder
            .add_maneuver_path(ManeuverPathInput {
                maneuver_path_key: text(path, "id"),
                movement: MovementReference::local(text(path, "movementId")),
                entry_edge: LaneEdgeReference::local(text(path, "entryEdgeId")),
                internal_edges: &internal_edges,
                exit_edge: LaneEdgeReference::local(text(path, "exitEdgeId")),
            })
            .unwrap();
    }

    let signals = &traffic["signals"];
    for stop_line in array(signals, "stopLines") {
        builder
            .add_stop_line(StopLineInput {
                stop_line_key: text(stop_line, "id"),
                lane_edge: LaneEdgeReference::local(text(stop_line, "edgeId")),
            })
            .unwrap();
    }
    for group in array(signals, "groups") {
        builder
            .add_signal_group(SignalGroupInput {
                signal_group_key: text(group, "id"),
            })
            .unwrap();
    }
    for controller in array(signals, "controllers") {
        let groups = string_array(controller, "groupIds")
            .map(SignalGroupReference::local)
            .collect::<Vec<_>>();
        let phases = array(controller, "phases");
        let states = phases
            .iter()
            .map(|phase| {
                array(phase, "states")
                    .iter()
                    .map(|state| SignalGroupStateInput {
                        signal_group: SignalGroupReference::local(text(state, "groupId")),
                        aspect: signal_aspect(text(state, "aspect")),
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let phase_inputs = phases
            .iter()
            .enumerate()
            .map(|(index, phase)| SignalPhaseInput {
                signal_phase_key: text(phase, "id"),
                duration_ms: integer(phase, "durationMs"),
                states: &states[index],
            })
            .collect::<Vec<_>>();
        builder
            .add_signal_controller(SignalControllerInput {
                signal_controller_key: text(controller, "id"),
                offset_ms: integer(controller, "offsetMs"),
                signal_groups: &groups,
                phases: &phase_inputs,
            })
            .unwrap();
    }
    for gate in array(signals, "maneuverGates") {
        let signal_control = &gate["signalControl"];
        let control = match text(signal_control, "kind") {
            "group" => SignalControlInput::Group(SignalGroupReference::local(text(
                signal_control,
                "groupId",
            ))),
            "none" => SignalControlInput::None,
            other => panic!("unsupported fixture signal control {other}"),
        };
        builder
            .add_maneuver_gate(ManeuverGateInput {
                maneuver_gate_key: text(gate, "id"),
                maneuver_path: ManeuverPathReference::local(text(gate, "maneuverPathId")),
                transition_index: u32::try_from(integer(gate, "transitionIndex")).unwrap(),
                stop_line: StopLineReference::local(text(gate, "stopLineId")),
                signal_control: control,
            })
            .unwrap();
    }
    for zone in array(traffic, "waitingZones") {
        builder
            .add_waiting_zone(WaitingZoneInput {
                waiting_zone_key: text(zone, "id"),
                maneuver_path: ManeuverPathReference::local(text(zone, "maneuverPathId")),
                entry_gate: ManeuverGateReference::local(text(zone, "entryGateId")),
                release_gate: ManeuverGateReference::local(text(zone, "releaseGateId")),
                max_occupancy: u32::try_from(integer(zone, "maxOccupancy")).unwrap(),
            })
            .unwrap();
    }

    let parking = &traffic["parking"];
    for area in array(parking, "areas") {
        builder
            .add_parking_area(ParkingAreaInput {
                parking_area_key: text(area, "id"),
            })
            .unwrap();
    }
    for space in array(parking, "spaces") {
        builder
            .add_parking_space(ParkingSpaceInput {
                parking_space_key: text(space, "id"),
                parking_area: optional_text(space, "areaId").map(ParkingAreaReference::local),
                entry: parking_anchor(&space["entry"]),
                exit: parking_anchor(&space["exit"]),
                geometry: parking_geometry(&space["geometry"]),
            })
            .unwrap();
    }

    for class in array(traffic, "participantClasses") {
        builder
            .add_participant_class(ParticipantClassInput {
                participant_class_key: text(class, "id"),
                extends: optional_text(class, "extendsId").map(ParticipantClassReference::local),
            })
            .unwrap();
    }
    for profile in array(traffic, "vehicleProfiles") {
        builder
            .add_vehicle_profile(VehicleProfileInput {
                vehicle_profile_key: text(profile, "id"),
                participant_class: ParticipantClassReference::local(text(
                    profile,
                    "participantClassId",
                )),
                iidm: IidmVehicleProfileInput {
                    length_meters: number(profile, "length"),
                    desired_speed_meters_per_second: number(profile, "desiredSpeed"),
                    min_gap_meters: number(profile, "minGap"),
                    time_headway_seconds: number(profile, "timeHeadway"),
                    max_acceleration_meters_per_second_squared: number(profile, "maxAcceleration"),
                    comfortable_deceleration_meters_per_second_squared: number(
                        profile,
                        "comfortableDeceleration",
                    ),
                    emergency_deceleration_meters_per_second_squared: number(
                        profile,
                        "emergencyDeceleration",
                    ),
                },
            })
            .unwrap();
    }
    for rule in array(traffic, "accessRules") {
        let target = &rule["target"];
        let target = match text(target, "kind") {
            "laneEdge" => {
                AccessRuleTargetInput::LaneEdge(LaneEdgeReference::local(text(target, "id")))
            }
            "laneGroup" => {
                AccessRuleTargetInput::LaneGroup(LaneGroupReference::local(text(target, "id")))
            }
            "roadSection" => {
                AccessRuleTargetInput::RoadSection(RoadSectionReference::local(text(target, "id")))
            }
            "maneuverPath" => AccessRuleTargetInput::ManeuverPath(ManeuverPathReference::local(
                text(target, "id"),
            )),
            "facilityBand" => AccessRuleTargetInput::FacilityBand(FacilityBandReference::local(
                text(target, "id"),
            )),
            other => panic!("unsupported fixture access target {other}"),
        };
        let classes = string_array(rule, "participantClassIds")
            .map(ParticipantClassReference::local)
            .collect::<Vec<_>>();
        let regulation = rule
            .get("regulation")
            .map(|regulation| AccessRegulationInput {
                jurisdiction: text(regulation, "jurisdiction"),
                version: text(regulation, "version"),
                source: optional_text(regulation, "source"),
            });
        builder
            .add_access_rule(AccessRuleInput {
                access_rule_key: text(rule, "id"),
                target,
                effect: match text(rule, "effect") {
                    "allow" => AccessEffect::Allow,
                    "deny" => AccessEffect::Deny,
                    other => panic!("unsupported fixture access effect {other}"),
                },
                participant_classes: &classes,
                regulation,
                priority: rule
                    .get("priority")
                    .and_then(Value::as_i64)
                    .map_or(0, |value| i32::try_from(value).unwrap()),
            })
            .unwrap();
    }
    for route in array(traffic, "routes") {
        let edges = string_array(route, "edgeIds")
            .map(LaneEdgeReference::local)
            .collect::<Vec<_>>();
        builder
            .add_static_route(StaticRouteInput {
                static_route_key: text(route, "id"),
                edge_sequence: &edges,
            })
            .unwrap();
    }
    builder.finish().unwrap()
}

fn build_spatial_module(
    spatial: &Value,
    limits: &CompileLimits,
) -> laneflow_compiler::SyntheticModule {
    build_spatial_module_for(
        spatial,
        limits,
        SPATIAL_NAMESPACE,
        TRAFFIC_NAMESPACE,
        "fixture/current-spatial.lfsynthetic",
    )
}

fn build_spatial_module_for(
    spatial: &Value,
    limits: &CompileLimits,
    spatial_namespace: &str,
    traffic_namespace: &str,
    source_document_key: &str,
) -> laneflow_compiler::SyntheticModule {
    let mut builder = SyntheticModuleBuilder::new(
        header(spatial_namespace, source_document_key, limits),
        limits,
    )
    .unwrap();
    builder.add_import(traffic_namespace).unwrap();
    let points = array(spatial, "edges")
        .iter()
        .map(|edge| {
            edge["centerline"]["points"]
                .as_array()
                .unwrap()
                .iter()
                .map(|point| {
                    let point = point.as_array().unwrap();
                    CanonicalPoint3F32Input {
                        x: point[0].as_f64().unwrap() as f32,
                        y: point[1].as_f64().unwrap() as f32,
                        z: point[2].as_f64().unwrap() as f32,
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let geometries = array(spatial, "edges")
        .iter()
        .enumerate()
        .map(|(index, edge)| LaneEdgeGeometryInput {
            lane_edge: LaneEdgeReference::imported(traffic_namespace, text(edge, "trafficEdgeId")),
            centerline_points: &points[index],
        })
        .collect::<Vec<_>>();
    builder
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: text(spatial, "frameId"),
            lane_edge_geometries: &geometries,
        })
        .unwrap();
    builder.finish().unwrap()
}

fn header(namespace: &str, document: &str, limits: &CompileLimits) -> SourceModuleHeader {
    SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: namespace,
            source_document_key: document,
            generator_build_id: "git:lf-comp-current-equiv-v1",
            parameters_and_inputs_digest: [0x31; 32],
            frontend_options_digest: [0x32; 32],
            random_seed: None,
            provenance: "repository:laneflow/current-equivalence",
        },
        limits,
    )
    .unwrap()
}

fn parking_anchor(value: &Value) -> ParkingLaneAnchorInput<'_> {
    ParkingLaneAnchorInput {
        lane_edge: LaneEdgeReference::local(text(value, "edgeId")),
        progress_meters: number(value, "progress"),
    }
}

fn parking_geometry(value: &Value) -> ParkingSpaceGeometryInput {
    ParkingSpaceGeometryInput {
        lateral_offset_meters: number(value, "lateralOffset"),
        heading_offset_radians: number(value, "headingOffsetRadians"),
        length_meters: number(value, "length"),
        width_meters: number(value, "width"),
    }
}

fn signal_aspect(value: &str) -> SignalAspect {
    match value {
        "red" => SignalAspect::Red,
        "yellow" => SignalAspect::Yellow,
        "green" => SignalAspect::Green,
        other => panic!("unsupported fixture signal aspect {other}"),
    }
}

fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value[key].as_array().unwrap()
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap()
}

fn optional_text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn number(value: &Value, key: &str) -> f64 {
    value[key].as_f64().unwrap()
}

fn integer(value: &Value, key: &str) -> u64 {
    value[key].as_u64().unwrap()
}

fn string_array<'a>(value: &'a Value, key: &str) -> impl Iterator<Item = &'a str> {
    array(value, key).iter().map(|item| item.as_str().unwrap())
}

fn stable_id_aliases(lir: &laneflow_compiler::ValidatedCanonicalLir) -> BTreeMap<String, String> {
    let mut aliases = BTreeMap::new();
    macro_rules! add {
        ($iter:expr, $tag:expr) => {
            for view in $iter {
                let key = view
                    .identity_fields()
                    .find(|field| field.tag() == $tag)
                    .map(|field| String::from_utf8(field.value_bytes().to_vec()).unwrap())
                    .unwrap();
                aliases.insert(view.stable_id().to_string(), key);
            }
        };
    }
    add!(lir.lane_edges(), FieldTag::LaneEdgeKey);
    add!(lir.road_corridors(), FieldTag::CorridorKey);
    add!(lir.road_sections(), FieldTag::SectionKey);
    add!(lir.lane_groups(), FieldTag::LaneGroupKey);
    add!(lir.facility_bands(), FieldTag::FacilityBandKey);
    add!(lir.junctions(), FieldTag::JunctionKey);
    add!(lir.movements(), FieldTag::MovementKey);
    add!(lir.maneuver_paths(), FieldTag::PathKey);
    add!(lir.stop_lines(), FieldTag::StopLineKey);
    add!(lir.maneuver_gates(), FieldTag::GateKey);
    add!(lir.waiting_zones(), FieldTag::WaitingZoneKey);
    add!(lir.signal_groups(), FieldTag::SignalGroupKey);
    add!(lir.signal_controllers(), FieldTag::SignalControllerKey);
    add!(lir.signal_phases(), FieldTag::PhaseKey);
    add!(lir.parking_areas(), FieldTag::ParkingAreaKey);
    add!(lir.parking_spaces(), FieldTag::ParkingSpaceKey);
    add!(lir.participant_classes(), FieldTag::ParticipantClassKey);
    add!(lir.vehicle_profiles(), FieldTag::VehicleProfileKey);
    add!(lir.access_rules(), FieldTag::AccessRuleKey);
    add!(lir.static_routes(), FieldTag::RouteKey);
    add!(lir.canonical_frames(), FieldTag::CanonicalFrameKey);
    aliases
}

fn assert_runtime_equivalence(
    current_traffic: InitialTrafficData,
    projected_traffic: InitialTrafficData,
    aliases: &BTreeMap<String, String>,
) {
    const PROFILE_ID: &str = "passenger-car";
    const ROUTE_ID: &str = "route-main-west-near-left";

    let projected_profile_id = stable_id_for(PROFILE_ID, aliases);
    let projected_route_id = stable_id_for(ROUTE_ID, aliases);
    let current_profile = current_traffic
        .vehicle_profiles()
        .profile_handle(PROFILE_ID)
        .unwrap();
    let projected_profile = projected_traffic
        .vehicle_profiles()
        .profile_handle(projected_profile_id)
        .unwrap();
    let speed = Speed::try_new(8.0).unwrap();
    let mut current = CoreWorld::with_traffic_data(
        20,
        current_traffic,
        vec![VehicleSpawnInput::active(
            "equivalence-vehicle",
            current_profile,
            ROUTE_ID,
            0,
            EdgeProgress::ZERO,
            speed,
        )],
    )
    .unwrap();
    let mut projected = CoreWorld::with_traffic_data(
        20,
        projected_traffic,
        vec![VehicleSpawnInput::active(
            "equivalence-vehicle",
            projected_profile,
            projected_route_id,
            0,
            EdgeProgress::ZERO,
            speed,
        )],
    )
    .unwrap();

    // 一千个固定步进足以覆盖入口行驶、受控机动、跨边和多个信号周期；逐 tick 对照可防止
    // 最终状态偶然汇合掩盖中间行为、事件顺序或数值演进差异。
    for _ in 0..1_000 {
        let current_step = current.step(TickInput::new(20)).unwrap();
        let projected_step = projected.step(TickInput::new(20)).unwrap();
        assert_eq!(current_step.tick_index, projected_step.tick_index);
        assert_eq!(current_step.time_ms, projected_step.time_ms);
        assert_eq!(
            normalize_events(&current, &current_step.events, &BTreeMap::new()),
            normalize_events(&projected, &projected_step.events, aliases)
        );
        assert_eq!(
            normalize_vehicle_state(&current, &BTreeMap::new()),
            normalize_vehicle_state(&projected, aliases)
        );
    }
}

fn stable_id_for<'a>(original_id: &str, aliases: &'a BTreeMap<String, String>) -> &'a str {
    aliases
        .iter()
        .find_map(|(stable_id, original)| (original == original_id).then_some(stable_id.as_str()))
        .unwrap()
}

fn original_id<'a>(id: &'a str, aliases: &'a BTreeMap<String, String>) -> &'a str {
    aliases.get(id).map_or(id, String::as_str)
}

fn normalize_vehicle_state(world: &CoreWorld, aliases: &BTreeMap<String, String>) -> Vec<String> {
    world
        .vehicles()
        .map(|state| {
            format!(
                "{}|{}|{}|{}|{:016x}|{:016x}|{:016x}|{:?}",
                world.vehicle_external_id(state.handle).unwrap(),
                original_id(
                    world.vehicle_profile_external_id(state.profile).unwrap(),
                    aliases,
                ),
                original_id(world.route_external_id(state.route).unwrap(), aliases),
                state.route_edge_index,
                state.edge_progress.value().to_bits(),
                state.current_speed.value().to_bits(),
                state.applied_acceleration.value().to_bits(),
                state.status,
            )
        })
        .collect()
}

fn normalize_events(
    world: &CoreWorld,
    events: &[CoreEvent],
    aliases: &BTreeMap<String, String>,
) -> Vec<String> {
    let id = |value: &str| original_id(value, aliases).to_owned();
    events
        .iter()
        .map(|event| match event {
            CoreEvent::VehicleSpeedLimitProjectionApplied(event) => format!(
                "speed-limit|{}|{}|{}|{}|{}|{}|{}",
                event.tick_index,
                world.vehicle_external_id(event.vehicle).unwrap(),
                id(world.route_external_id(event.route).unwrap()),
                event.from_route_edge_index,
                event.to_route_edge_index,
                id(world.edge_external_id(event.from_edge).unwrap()),
                id(world.edge_external_id(event.to_edge).unwrap()),
            ),
            CoreEvent::VehicleSignalStopProjectionApplied(event) => format!(
                "signal-stop|{}|{}|{}|{}|{}|{}|{}|{}|{:?}",
                event.tick_index,
                world.vehicle_external_id(event.vehicle).unwrap(),
                id(world.route_external_id(event.route).unwrap()),
                event.from_route_edge_index,
                event.to_route_edge_index,
                id(world
                    .signals()
                    .maneuver_gate_external_id(event.gate)
                    .unwrap()),
                id(world
                    .signals()
                    .stop_line_external_id(event.stop_line)
                    .unwrap()),
                id(world.signals().group_external_id(event.group).unwrap()),
                event.aspect,
            ),
            CoreEvent::VehicleParkingStopProjectionApplied(event) => format!(
                "parking-stop|{}|{}|{}|{}|{}",
                event.tick_index,
                world.vehicle_external_id(event.vehicle).unwrap(),
                id(world.parking().space_external_id(event.space).unwrap()),
                id(world.route_external_id(event.route).unwrap()),
                event.route_edge_index,
            ),
            CoreEvent::VehicleFollowingSafetyProjectionApplied(event) => format!(
                "following|{}|{}|{}",
                event.tick_index,
                world.vehicle_external_id(event.vehicle).unwrap(),
                world.vehicle_external_id(event.leader).unwrap(),
            ),
            CoreEvent::VehicleChangedEdge(event) => format!(
                "changed-edge|{}|{}|{}|{}|{}|{}|{}",
                event.tick_index,
                world.vehicle_external_id(event.vehicle).unwrap(),
                id(world.route_external_id(event.route).unwrap()),
                id(world.edge_external_id(event.from_edge).unwrap()),
                id(world.edge_external_id(event.to_edge).unwrap()),
                event.from_route_edge_index,
                event.to_route_edge_index,
            ),
            CoreEvent::VehicleParkingArrivalReached(event) => format!(
                "parking-arrival|{}|{}|{}|{}|{}",
                event.tick_index,
                world.vehicle_external_id(event.vehicle).unwrap(),
                id(world.parking().space_external_id(event.space).unwrap()),
                id(world.route_external_id(event.route).unwrap()),
                event.route_edge_index,
            ),
            CoreEvent::ParkingReservationReleased(event) => format!(
                "parking-release|{}|{}|{}|{:?}",
                event.tick_index,
                world.vehicle_external_id(event.vehicle).unwrap(),
                id(world.parking().space_external_id(event.space).unwrap()),
                event.reason,
            ),
            CoreEvent::VehicleCompletedRoute(event) => format!(
                "completed|{}|{}|{}|{}|{}",
                event.tick_index,
                world.vehicle_external_id(event.vehicle).unwrap(),
                id(world.route_external_id(event.route).unwrap()),
                id(world.edge_external_id(event.edge).unwrap()),
                event.route_edge_index,
            ),
            CoreEvent::SignalPhaseChanged(event) => format!(
                "phase|{}|{}|{}|{}",
                event.tick_index,
                id(world
                    .signals()
                    .controller_external_id(event.controller)
                    .unwrap()),
                id(world.signals().phase_external_id(event.from_phase).unwrap()),
                id(world.signals().phase_external_id(event.to_phase).unwrap()),
            ),
            CoreEvent::SignalGroupAspectChanged(event) => format!(
                "aspect|{}|{}|{:?}|{:?}",
                event.tick_index,
                id(world.signals().group_external_id(event.group).unwrap()),
                event.from_aspect,
                event.to_aspect,
            ),
            _ => panic!("LF-COMP-CURRENT-EQUIV-v1 尚未登记新的 CoreEvent 变体"),
        })
        .collect()
}

fn traffic_snapshot(
    traffic: &InitialTrafficData,
    aliases: &BTreeMap<String, String>,
) -> Vec<String> {
    let alias = |id: &str| aliases.get(id).cloned().unwrap_or_else(|| id.to_owned());
    let mut rows = Vec::new();
    for edge in traffic.lane_graph().edges() {
        let mut successors = edge
            .next_edge_ids()
            .iter()
            .map(|id| alias(id))
            .collect::<Vec<_>>();
        successors.sort();
        rows.push(format!(
            "edge|{}|{:016x}|{:016x}|{}",
            alias(edge.id()),
            edge.length().value().to_bits(),
            edge.speed_limit().value().to_bits(),
            successors.join(",")
        ));
    }
    for route in traffic.routes() {
        rows.push(format!(
            "route|{}|{}",
            alias(route.id()),
            route
                .edge_ids()
                .iter()
                .map(|id| alias(id))
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    let junctions = traffic.junctions();
    for handle in junctions.junctions() {
        let value = junctions.junction(handle).unwrap();
        rows.push(format!("junction|{}", alias(value.id())));
    }
    for handle in junctions.movements() {
        let value = junctions.movement(handle).unwrap();
        rows.push(format!(
            "movement|{}|{}",
            alias(value.id()),
            alias(value.junction_id())
        ));
    }
    for handle in junctions.maneuver_paths() {
        let value = junctions.maneuver_path(handle).unwrap();
        rows.push(format!(
            "path|{}|{}|{}|{}|{}",
            alias(value.id()),
            alias(value.movement_id()),
            alias(value.entry_edge_id()),
            value
                .internal_edge_ids()
                .iter()
                .map(|id| alias(id))
                .collect::<Vec<_>>()
                .join(","),
            alias(value.exit_edge_id())
        ));
    }
    let signals = traffic.signals();
    for value in signals.stop_lines() {
        rows.push(format!(
            "stop|{}|{}|{:?}",
            alias(value.id()),
            alias(value.edge_id()),
            value.location()
        ));
    }
    for value in signals.groups() {
        rows.push(format!("signal-group|{}", alias(value.id())));
    }
    for value in signals.controllers() {
        let phases = value
            .phases()
            .iter()
            .map(|phase| {
                let mut states = phase
                    .states()
                    .iter()
                    .map(|state| format!("{}={:?}", alias(state.group_id()), state.aspect()))
                    .collect::<Vec<_>>();
                states.sort();
                format!(
                    "{}:{}:{}",
                    alias(phase.id()),
                    phase.duration_ms(),
                    states.join(",")
                )
            })
            .collect::<Vec<_>>();
        let mut groups = value
            .group_ids()
            .iter()
            .map(|id| alias(id))
            .collect::<Vec<_>>();
        groups.sort();
        rows.push(format!(
            "controller|{}|{}|{}|{}",
            alias(value.id()),
            value.offset_ms(),
            groups.join(","),
            phases.join(";")
        ));
    }
    for handle in signals.maneuver_gates() {
        let value = signals.maneuver_gate(handle).unwrap();
        let control = match value.signal_control() {
            CoreSignalControlInput::Group(id) => format!("group:{}", alias(id)),
            CoreSignalControlInput::None => "none".to_owned(),
        };
        rows.push(format!(
            "gate|{}|{}|{}|{}|{}",
            alias(value.id()),
            alias(value.maneuver_path_id()),
            value.transition_index(),
            alias(value.stop_line_id()),
            control
        ));
    }
    for handle in traffic.waiting().waiting_zones() {
        let value = traffic.waiting().waiting_zone(handle).unwrap();
        rows.push(format!(
            "waiting|{}|{}|{}|{}|{}",
            alias(value.id()),
            alias(value.maneuver_path_id()),
            alias(value.entry_gate_id()),
            alias(value.release_gate_id()),
            value.max_occupancy()
        ));
    }
    let cross = traffic.cross_section();
    for handle in cross.bands() {
        let value = cross.band(handle).unwrap();
        rows.push(format!("band|{}|{}", alias(value.id()), value.kind_id()));
    }
    for handle in cross.sections() {
        let value = cross.section(handle).unwrap();
        let lanes = value
            .lanes()
            .iter()
            .map(|lane| {
                format!(
                    "{}@{}",
                    lane.edge_ids()
                        .iter()
                        .map(|id| alias(id))
                        .collect::<Vec<_>>()
                        .join(","),
                    lane.lane_group_id()
                        .map(&alias)
                        .unwrap_or_else(|| "-".to_owned())
                )
            })
            .collect::<Vec<_>>();
        rows.push(format!(
            "section|{}|{}|{}",
            alias(value.id()),
            value.kind_id(),
            lanes.join(";")
        ));
    }
    for handle in cross.groups() {
        let value = cross.group(handle).unwrap();
        rows.push(format!(
            "lane-group|{}|{}",
            alias(value.id()),
            alias(value.road_section_id())
        ));
    }
    for handle in cross.corridors() {
        let value = cross.corridor(handle).unwrap();
        let elements = value
            .elements()
            .iter()
            .map(|element| match element {
                CorridorElementId::Section(id) => format!("section:{}", alias(id)),
                CorridorElementId::Band(id) => format!("band:{}", alias(id)),
            })
            .collect::<Vec<_>>();
        rows.push(format!(
            "corridor|{}|{}|{}",
            alias(value.id()),
            alias(value.reference_section_id()),
            elements.join(",")
        ));
    }
    for value in traffic.parking().areas() {
        rows.push(format!("parking-area|{}", alias(value.id())));
    }
    for value in traffic.parking().spaces() {
        let geometry = value.geometry();
        rows.push(format!(
            "parking-space|{}|{}|{}:{:016x}|{}:{:016x}|{:016x}:{:016x}:{:016x}:{:016x}",
            alias(value.id()),
            value
                .area_id()
                .map(&alias)
                .unwrap_or_else(|| "-".to_owned()),
            alias(value.entry_edge_id()),
            value.entry_progress().to_bits(),
            alias(value.exit_edge_id()),
            value.exit_progress().to_bits(),
            geometry.lateral_offset().to_bits(),
            geometry.heading_offset_radians().to_bits(),
            geometry.length().to_bits(),
            geometry.width().to_bits()
        ));
    }
    let classes = traffic.participant_classes();
    for handle in classes.classes() {
        let value = classes.class(handle).unwrap();
        rows.push(format!(
            "class|{}|{}",
            alias(value.id()),
            value
                .extends_id()
                .map(&alias)
                .unwrap_or_else(|| "-".to_owned())
        ));
    }
    for value in traffic.vehicle_profiles().profiles() {
        let class = classes
            .class_external_id(value.participant_class())
            .unwrap();
        let iidm = value.iidm();
        rows.push(format!(
            "profile|{}|{}|{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}",
            alias(value.external_id()),
            alias(class),
            iidm.length.to_bits(),
            iidm.desired_speed.to_bits(),
            iidm.min_gap.to_bits(),
            iidm.time_headway.to_bits(),
            iidm.max_acceleration.to_bits(),
            iidm.comfortable_deceleration.to_bits(),
            iidm.emergency_deceleration.to_bits()
        ));
    }
    for handle in traffic.access().rules() {
        let value = traffic.access().rule(handle).unwrap();
        let target = match value.target() {
            AccessTargetId::LaneEdge(id) => format!("edge:{}", alias(id)),
            AccessTargetId::LaneGroup(id) => format!("group:{}", alias(id)),
            AccessTargetId::RoadSection(id) => format!("section:{}", alias(id)),
            AccessTargetId::ManeuverPath(id) => format!("path:{}", alias(id)),
            AccessTargetId::FacilityBand(id) => format!("band:{}", alias(id)),
        };
        let mut rule_classes = value
            .participant_class_ids()
            .iter()
            .map(|id| alias(id))
            .collect::<Vec<_>>();
        rule_classes.sort();
        let regulation = value.regulation().map_or_else(
            || "-".to_owned(),
            |regulation| {
                format!(
                    "{}:{}:{}",
                    regulation.jurisdiction(),
                    regulation.version(),
                    regulation.source().unwrap_or("-")
                )
            },
        );
        rows.push(format!(
            "access|{}|{}|{:?}|{}|{}|{}",
            alias(value.id()),
            target,
            value.effect(),
            rule_classes.join(","),
            regulation,
            value.priority()
        ));
    }
    rows.sort();
    rows
}

fn assert_entity_counts(lir: &laneflow_compiler::ValidatedCanonicalLir, expected: [usize; 22]) {
    let actual = [
        lir.lane_edges().len(),
        lir.road_corridors().len(),
        lir.road_sections().len(),
        lir.authoring_lanes().len(),
        lir.lane_groups().len(),
        lir.facility_bands().len(),
        lir.junctions().len(),
        lir.movements().len(),
        lir.maneuver_paths().len(),
        lir.stop_lines().len(),
        lir.maneuver_gates().len(),
        lir.waiting_zones().len(),
        lir.signal_groups().len(),
        lir.signal_controllers().len(),
        lir.signal_phases().len(),
        lir.parking_areas().len(),
        lir.parking_spaces().len(),
        lir.participant_classes().len(),
        lir.vehicle_profiles().len(),
        lir.access_rules().len(),
        lir.static_routes().len(),
        lir.canonical_frames().len(),
    ];
    assert_eq!(actual, expected);
}

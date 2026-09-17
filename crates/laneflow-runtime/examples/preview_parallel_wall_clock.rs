//! #705 非插桩整步墙钟对照：与 cfg(test) 测量探针同场景的
//! 正常 release 构建（无诊断插桩、无统计计数），用于把「融合 vs 多 worker」
//! 的整步墙钟登记为生产形态基线。预览局部收益不等于城市性能通过（#707）。
//!
//! 单独运行：
//! `cargo run --release -p laneflow-runtime --example preview_parallel_wall_clock`

use std::sync::Arc;
use std::time::Instant;

use laneflow_compiler::{
    CompilationUnitBuilder, CompileLimits, Compiler, IidmVehicleProfileInput, JunctionInput,
    JunctionReference, LaneEdgeInput, LaneEdgeReference, ManeuverGateInput, ManeuverGateReference,
    ManeuverPathInput, ManeuverPathReference, MovementInput, MovementReference,
    ParticipantClassInput, ParticipantClassReference, PolicyGateRuleInput, PolicyInputSource,
    PortableDiffBase, PortableEmissionProvenance, RegulationIdentity, RightOfWayPolicySetInput,
    SourceModuleHeader, SourceModuleHeaderInput, StopLineInput, StopLineReference,
    SyntheticModuleBuilder, VehicleProfileInput, WaitingZoneInput, derive_canonical_stable_id_v1,
    emit_portable_candidate,
};
use laneflow_format::{FormatLimits, check_post_emission_bundle};
use laneflow_runtime::{
    CommittedNetworkSource, ExecutionConfig, PolicyPin, PublishedLfcaReference, RouteHandle,
    RouteRegisterInput, TickInput, TrafficWorld, VehicleSpawnInput, VehicleStatus, WorldConfig,
    WorldPolicySelection,
};
use laneflow_static_contract::{
    EntityKind, ManeuverPathOrdinal, RightOfWayPolicySetId, VehicleProfileOrdinal,
};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};

const VEHICLES: usize = 1_024;
const WARMUP_TICKS: usize = 24;
const MEASURED_TICKS: usize = 128;
const ROUNDS: usize = 3;
const DELTA_MS: u64 = 100;

fn build_multi_gate_revision(count: usize) -> Arc<SharedNetworkRevision> {
    const NS: &str = "city/waiting-scale";
    let limits = CompileLimits::single_network_1m_v2();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: NS,
            source_document_key: "waiting-scale.document",
            generator_build_id: "git:0123456789abcdef",
            parameters_and_inputs_digest: [0x31; 32],
            frontend_options_digest: [0x42; 32],
            random_seed: Some(282),
            provenance: "repository:laneflow",
        },
        &limits,
    )
    .expect("source header");
    let mut module = SyntheticModuleBuilder::new(header, &limits).expect("module");
    module
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "road-user",
            extends: None,
        })
        .expect("class")
        .add_vehicle_profile(VehicleProfileInput {
            vehicle_profile_key: "car",
            participant_class: ParticipantClassReference::local("road-user"),
            iidm: IidmVehicleProfileInput {
                length_meters: 4.5,
                desired_speed_meters_per_second: 13.75,
                min_gap_meters: 2.0,
                time_headway_seconds: 1.4,
                max_acceleration_meters_per_second_squared: 1.8,
                comfortable_deceleration_meters_per_second_squared: 2.0,
                emergency_deceleration_meters_per_second_squared: 4.5,
            },
        })
        .expect("profile");
    let stems = (0..64)
        .map(|index| format!("stem-{index:02}"))
        .collect::<Vec<_>>();
    for (index, key) in stems.iter().enumerate() {
        let successor = stems.get(index + 1).map_or("entry", String::as_str);
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: key,
                length_meters: 10_000.0,
                speed_limit_meters_per_second: 13.75,
                successors: &[LaneEdgeReference::local(successor)],
            })
            .expect("stem");
    }
    for (key, length, successor) in [
        ("entry", 10_000.0, "storage"),
        ("storage", 8.0, "after-release"),
        ("after-release", 12.0, "exit"),
        ("exit", 12.0, "entry"),
    ] {
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: key,
                length_meters: length,
                speed_limit_meters_per_second: 13.75,
                successors: &[LaneEdgeReference::local(successor)],
            })
            .expect("internal edge");
    }
    module
        .add_junction(JunctionInput {
            junction_key: "junction",
        })
        .expect("junction")
        .add_movement(MovementInput {
            turn_direction: None,
            movement_key: "movement",
            junction: JunctionReference::local("junction"),
            directed_entry_approach_key: "approach-in",
            directed_exit_approach_key: "approach-out",
        })
        .expect("movement")
        .add_maneuver_path(ManeuverPathInput {
            maneuver_path_key: "path",
            movement: MovementReference::local("movement"),
            entry_edge: LaneEdgeReference::local("entry"),
            internal_edges: &[
                LaneEdgeReference::local("storage"),
                LaneEdgeReference::local("after-release"),
            ],
            exit_edge: LaneEdgeReference::local("exit"),
        })
        .expect("path");
    module
        .add_stop_line(StopLineInput {
            stop_line_key: "stop-entry",
            lane_edge: LaneEdgeReference::local("entry"),
        })
        .expect("entry stop")
        .add_stop_line(StopLineInput {
            stop_line_key: "stop-release",
            lane_edge: LaneEdgeReference::local("storage"),
        })
        .expect("release stop")
        .add_maneuver_gate(ManeuverGateInput {
            maneuver_gate_key: "gate-entry",
            maneuver_path: ManeuverPathReference::local("path"),
            transition_index: 0,
            stop_line: StopLineReference::local("stop-entry"),
            signal_control: laneflow_compiler::SignalControlInput::None,
        })
        .expect("entry gate")
        .add_maneuver_gate(ManeuverGateInput {
            maneuver_gate_key: "gate-release",
            maneuver_path: ManeuverPathReference::local("path"),
            transition_index: 1,
            stop_line: StopLineReference::local("stop-release"),
            signal_control: laneflow_compiler::SignalControlInput::None,
        })
        .expect("release gate");
    module
        .add_waiting_zone(WaitingZoneInput {
            waiting_zone_key: "waiting",
            maneuver_path: ManeuverPathReference::local("path"),
            entry_gate: ManeuverGateReference::local("gate-entry"),
            release_gate: ManeuverGateReference::local("gate-release"),
            max_occupancy: 1,
        })
        .expect("main waiting zone");
    for layout in (0..count).map(|index| format!("idle-{index}")) {
        for (edge, successor) in [("entry", "storage"), ("storage", "exit"), ("exit", "entry")] {
            module
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: &format!("{layout}-{edge}"),
                    length_meters: 20.0,
                    speed_limit_meters_per_second: 13.75,
                    successors: &[LaneEdgeReference::local(&format!("{layout}-{successor}"))],
                })
                .expect("idle lane");
        }
        module
            .add_junction(JunctionInput {
                junction_key: &format!("{layout}-junction"),
            })
            .expect("idle junction")
            .add_movement(MovementInput {
                turn_direction: None,
                movement_key: &format!("{layout}-movement"),
                junction: JunctionReference::local(&format!("{layout}-junction")),
                directed_entry_approach_key: "in",
                directed_exit_approach_key: "out",
            })
            .expect("idle movement")
            .add_maneuver_path(ManeuverPathInput {
                maneuver_path_key: &format!("{layout}-path"),
                movement: MovementReference::local(&format!("{layout}-movement")),
                entry_edge: LaneEdgeReference::local(&format!("{layout}-entry")),
                internal_edges: &[LaneEdgeReference::local(&format!("{layout}-storage"))],
                exit_edge: LaneEdgeReference::local(&format!("{layout}-exit")),
            })
            .expect("idle path");
        for (gate, stop, edge, transition_index) in [
            ("entry-gate", "entry-stop", "entry", 0_u32),
            ("release-gate", "release-stop", "storage", 1_u32),
        ] {
            module
                .add_stop_line(StopLineInput {
                    stop_line_key: &format!("{layout}-{stop}"),
                    lane_edge: LaneEdgeReference::local(&format!("{layout}-{edge}")),
                })
                .expect("idle stop")
                .add_maneuver_gate(ManeuverGateInput {
                    maneuver_gate_key: &format!("{layout}-{gate}"),
                    maneuver_path: ManeuverPathReference::local(&format!("{layout}-path")),
                    transition_index,
                    stop_line: StopLineReference::local(&format!("{layout}-{stop}")),
                    signal_control: laneflow_compiler::SignalControlInput::None,
                })
                .expect("idle gate");
        }
        module
            .add_waiting_zone(WaitingZoneInput {
                waiting_zone_key: &format!("{layout}-zone"),
                maneuver_path: ManeuverPathReference::local(&format!("{layout}-path")),
                entry_gate: ManeuverGateReference::local(&format!("{layout}-entry-gate")),
                release_gate: ManeuverGateReference::local(&format!("{layout}-release-gate")),
                max_occupancy: 2,
            })
            .expect("idle zone");
    }
    let mut policy_gates = vec!["gate-entry".to_owned(), "gate-release".to_owned()];
    for index in 0..count {
        policy_gates.push(format!("idle-{index}-entry-gate"));
        policy_gates.push(format!("idle-{index}-release-gate"));
    }
    let span = module.policy_source_span();
    let source = PolicyInputSource {
        primary: &span,
        contributing: &[],
    };
    let rules: Vec<_> = policy_gates
        .iter()
        .map(|key| PolicyGateRuleInput {
            rule_key: key,
            gate: laneflow_compiler::OwnerQualifiedReference {
                target: ManeuverGateReference::local(key),
                owner_keys: &[],
            },
            participant_classes: None,
            interpretation: laneflow_compiler::GateInterpretation::Uncontrolled,
            prohibition: laneflow_compiler::GateProhibition::None,
            evidence_keys: &[],
            source,
        })
        .collect();
    module
        .add_right_of_way_policy_set(RightOfWayPolicySetInput {
            policy_set_key: "waiting-policy",
            regulation: RegulationIdentity {
                jurisdiction: "engineering",
                version: "fixture-1",
                source: Some("repository:runtime-fixture-1"),
            },
            evidence: &[],
            gap_profiles: &[],
            stream_rules: &[],
            gate_rules: &rules,
            source,
        })
        .expect("waiting policy");
    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_synthetic_module(module.finish().expect("finished module"))
        .expect("unit module");
    let output = Compiler::new()
        .compile(unit.build().expect("unit"))
        .expect("compiled");
    let provenance =
        PortableEmissionProvenance::try_new("laneflow-waiting-scale-v1").expect("provenance");
    let candidate = emit_portable_candidate(
        &output,
        &provenance,
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .expect("portable candidate");
    let checked = check_post_emission_bundle(
        candidate.canonical_artifact().bytes(),
        candidate.source_map().bytes(),
        candidate.semantic_diff().bytes(),
        candidate.expected_semantic_diff_base(),
        FormatLimits::HARD,
    )
    .expect("checked bundle");
    build_shared_network_revision(
        checked.canonical_network_input(),
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::Omit,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .expect("revision")
}

fn build_world(revision: &Arc<SharedNetworkRevision>, workers: u32, world_id: u64) -> TrafficWorld {
    let origin = *revision.canonical_origin();
    let count = VEHICLES as u32;
    let policy = RightOfWayPolicySetId::from_untyped(
        derive_canonical_stable_id_v1(
            EntityKind::RightOfWayPolicySet,
            "city/waiting-scale",
            "waiting-policy",
            &CompileLimits::p100_initial_v1(),
        )
        .expect("policy id"),
    );
    let mut world = TrafficWorld::install(
        Arc::clone(revision),
        WorldConfig::new(count, count, u64::from(count) * 3, 1, 100),
        ExecutionConfig::new(std::num::NonZeroU32::new(workers).expect("nonzero workers")),
        CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                "fixture://multi-gate",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .expect("published reference"),
        },
        world_id,
        WorldPolicySelection::Pinned(PolicyPin { policy }),
    )
    .expect("installed world");
    // 与测试夹具相同的生成：每条 3 边 idle 路线首边末端前一毫米、10 m/s。
    for raw in 0..revision.traffic().maneuvers().maneuver_path_count() {
        let edges = revision
            .traffic()
            .maneuvers()
            .maneuver_path(ManeuverPathOrdinal::from_raw(raw))
            .expect("path")
            .edges()
            .to_vec();
        if edges.len() != 3 {
            continue;
        }
        let boundary = revision.traffic().lane_lengths_millimetres()[edges[0].index()];
        let route = world
            .register_route(RouteRegisterInput::new(edges))
            .expect("route");
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                boundary - 1,
                10_000,
            ))
            .expect("vehicle");
    }
    assert_eq!(world.live_vehicles().len(), VEHICLES);
    world
}

fn route_boundaries(world: &TrafficWorld, routes: &[RouteHandle]) -> Vec<u32> {
    let lengths = world.traffic().lane_lengths_millimetres();
    routes
        .iter()
        .map(|route| lengths[world.route_edges(*route).expect("edges")[0].index()])
        .collect()
}

fn replenish(world: &mut TrafficWorld, routes: &[RouteHandle], boundaries: &[u32]) {
    // 补员：与探针相同，把已完成车辆原位替换回 Gate 前，保持稳态活动车队。
    for handle in world.live_vehicles().to_vec() {
        let state = world.vehicle(handle).expect("live handle");
        if state.status() != VehicleStatus::Completed {
            continue;
        }
        let position = routes
            .iter()
            .position(|route| *route == state.route())
            .expect("route");
        world
            .replace_completed_vehicle(
                handle,
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    routes[position],
                    0,
                    boundaries[position] - 1,
                    10_000,
                ),
            )
            .expect("replacement");
    }
}

fn routes_of(world: &TrafficWorld) -> Vec<RouteHandle> {
    world.live_routes().collect()
}

fn percentile(sorted: &[u128], fraction: f64) -> u128 {
    let index = (fraction * sorted.len() as f64).ceil() as usize;
    sorted[index.saturating_sub(1).min(sorted.len() - 1)]
}

fn main() {
    let revision = build_multi_gate_revision(VEHICLES);
    for &workers in &[1_u32, 2, 4, 8, 16] {
        for round in 0..ROUNDS {
            let mut world = build_world(&revision, workers, 705_500 + round as u64);
            let routes = routes_of(&world);
            let boundaries = route_boundaries(&world, &routes);
            for _ in 0..WARMUP_TICKS {
                replenish(&mut world, &routes, &boundaries);
                std::hint::black_box(world.step(TickInput::new(DELTA_MS)).unwrap());
            }
            let mut samples = Vec::with_capacity(MEASURED_TICKS);
            for _ in 0..MEASURED_TICKS {
                replenish(&mut world, &routes, &boundaries);
                let started = Instant::now();
                std::hint::black_box(world.step(TickInput::new(DELTA_MS)).unwrap());
                samples.push(started.elapsed().as_nanos());
            }
            let mut sorted = samples;
            sorted.sort_unstable();
            println!(
                "preview-wall scene=multi-gate-{VEHICLES} workers={workers} round={round} \
                 whole_p50_ns={} whole_p95_ns={}",
                percentile(&sorted, 0.50),
                percentile(&sorted, 0.95),
            );
        }
    }
    println!("preview-wall-done");
}

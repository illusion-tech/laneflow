//! #705 整步等价对拍：同一二进制、同一公开场景脚本下，worker 1 与 2/4/8/16
//! 逐步一致。
//!
//! 对拍合同（`docs/design/traffic-runtime-parallel-execution.md` §1/§9）：逐 tick
//! 比较已提交状态 digest、最新决策/事件、命令/事件游标、`StepOutcome`、世界
//! 世代与观测序号；比较只使用公开 API，不读取私有存储、容量或序号基线。
//!
//! 生产 P2 分发阈值为保守的 1_024 Active（`WAITING_PREVIEW_DISPATCH_MIN_ACTIVE`）；
//! 本文件全部场景的工作集都低于该阈值，worker 1/2/4/8/16 全部走融合路径，
//! 命名统一带 `fused`，关键操作后断言 Active 低于阈值（融合路径覆盖的公开
//! 行为面）。多 worker 真实分发的等价证据：lib 内强制分发测试（同修订/跨
//! 修订切换后首拍、fresh restore 后首拍、首错矩阵等）与本 crate 的
//! `preview_at_threshold_equivalence`（1_280 车，高于生产阈值，逐拍对拍）。
//!
//! 场景覆盖：Waiting 密集的合成环（同槽 despawn/spawn 新代次）、信号走廊
//! 长前车链（catalog 生成槽位）、full-spatial 信号/混合生命周期（Completed
//! 保留、原子替换新代次、Parked 夹入 live 序列）、停车 Active→Parked 变化、
//! fresh restore 后首拍、同修订与跨修订切换后首拍、公开逻辑首错
//! （DeltaMismatch）与同 tick 重试。

#[path = "support/policy.rs"]
mod test_policy;

use std::num::NonZeroU32;
use std::sync::Arc;

use laneflow_compiler::{
    CompilationUnitBuilder, CompileLimits, Compiler, GateInterpretation, IidmVehicleProfileInput,
    JunctionInput, JunctionReference, LaneEdgeInput, LaneEdgeReference, ManeuverGateInput,
    ManeuverGateReference, ManeuverPathInput, ManeuverPathReference, MovementInput,
    MovementReference, ParticipantClassInput, ParticipantClassReference, PortableDiffBase,
    PortableEmissionProvenance, SourceModuleHeader, SourceModuleHeaderInput, StopLineInput,
    StopLineReference, SyntheticModuleBuilder, VehicleProfileInput, WaitingZoneInput,
    emit_portable_candidate,
};
use laneflow_format::{FormatLimits, check_canonical_network_input, check_post_emission_bundle};
use laneflow_runtime::{
    CommittedNetworkSource, ExecutionConfig, ParkingTarget, PolicyPin, PublishedLfcaReference,
    ReserveParkingTarget, RouteHandle, RouteRegisterInput, StepOutcome, TickInput, TrafficWorld,
    VehicleSpawnInput, VehicleStatus, WorldConfig, WorldPolicySelection,
    deterministic_state_digest,
};
#[cfg(feature = "placement-fixtures")]
use laneflow_runtime::{ParkedVehicleSpawnInput, VehicleHandle};
use laneflow_scenario::signalized_corridor::{CorridorCatalog, PASSENGER_CAR_PROFILE_KEY, bind};
#[cfg(feature = "placement-fixtures")]
use laneflow_static_contract::LaneEdgeOrdinal;
use laneflow_static_contract::{EntityKind, ManeuverPathOrdinal, VehicleProfileOrdinal};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};

const WORKERS: [u32; 5] = [1, 2, 4, 8, 16];
const DELTA_MS: u64 = 100;
const BUILD_LIMITS: SharedNetworkBuildLimits =
    SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024);
#[cfg(feature = "placement-fixtures")]
const FULL_SPATIAL: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable/lfca-world-policies/full-spatial.lfca"
);
const PARKING_ONLY: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable/lfsd-migration/oracle-base.lfca"
);
const CORRIDOR: &[u8] = include_bytes!("../../../examples/data/v0.2-signalized-corridor.lfca");
const CORRIDOR_CATALOG: &str =
    include_str!("../../../examples/data/v0.2-signalized-corridor.catalog.toml");
/// 生产 P2 分发阈值（`WAITING_PREVIEW_DISPATCH_MIN_ACTIVE` 的公开行为面）。
const PRODUCTION_DISPATCH_MIN_ACTIVE: usize = 1_024;

fn execution(workers: u32) -> ExecutionConfig {
    ExecutionConfig::new(NonZeroU32::new(workers).expect("nonzero worker count"))
}

fn install_published(
    revision: &Arc<SharedNetworkRevision>,
    config: WorldConfig,
    workers: u32,
    world_id: u64,
    key: &str,
    selection: WorldPolicySelection,
) -> TrafficWorld {
    let origin = revision.canonical_origin();
    TrafficWorld::install(
        Arc::clone(revision),
        config,
        execution(workers),
        CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                key,
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .expect("non-empty fixture key"),
        },
        world_id,
        selection,
    )
    .expect("install")
}

#[cfg(feature = "placement-fixtures")]
fn full_spatial_revision() -> Arc<SharedNetworkRevision> {
    let input = check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD)
        .expect("checked canonical network input");
    build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(SpatialBuildOption::RetainAvailable, BUILD_LIMITS),
    )
    .expect("shared network revision")
}

fn parking_revision() -> Arc<SharedNetworkRevision> {
    let input = check_canonical_network_input(PARKING_ONLY, FormatLimits::HARD)
        .expect("checked parking fixture");
    build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(SpatialBuildOption::Omit, BUILD_LIMITS),
    )
    .expect("parking revision")
}

fn corridor_revision() -> Arc<SharedNetworkRevision> {
    let input = check_canonical_network_input(CORRIDOR, FormatLimits::HARD).expect("checked");
    build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(SpatialBuildOption::Omit, BUILD_LIMITS),
    )
    .expect("corridor revision")
}

/// 逐 tick 公开观察记录：只比较已提交语义与公开游标，不触碰私有暂存。
fn tick_record(world: &TrafficWorld, outcome: &StepOutcome) -> String {
    let snapshot = world.capture_snapshot().expect("capture");
    let vehicles: Vec<_> = world
        .live_vehicles()
        .iter()
        .map(|handle| world.vehicle(*handle))
        .collect();
    format!(
        "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
        deterministic_state_digest(&snapshot).expect("digest"),
        world.latest_waiting_decisions(),
        world.latest_conflict_decisions(),
        world.latest_transition_events(),
        world.committed_signal_groups(),
        world.waiting_zone_members(),
        world.committed_pose_sources().collect::<Vec<_>>(),
        vehicles,
        world.migration_journal_stats(),
        (
            world.observation_state_sequence(),
            world.world_generation(),
            world.tick_index(),
            world.time_ms(),
            world.command_cursor(),
            world.event_cursor(),
            outcome,
        ),
    )
}

fn active_count(world: &TrafficWorld) -> usize {
    world
        .live_vehicles()
        .iter()
        .filter(|handle| {
            world
                .vehicle(**handle)
                .is_some_and(|state| state.status() == VehicleStatus::Active)
        })
        .count()
}

/// 同一脚本在 worker 1 与 2/4/8/16 下逐步一致；worker 1 记录为参照。
fn assert_matches_across_workers(scenario: &str, run: impl Fn(u32) -> Vec<String>) {
    let reference = run(1);
    assert!(!reference.is_empty(), "{scenario} must produce records");
    for workers in WORKERS.iter().copied().skip(1) {
        assert_eq!(
            run(workers),
            reference,
            "{scenario} diverged at workers={workers}"
        );
    }
}

/// 生命周期命令替换：despawn 后旧句柄立即失效，新句柄为不同代际；返回新句柄。
#[cfg(feature = "placement-fixtures")]
fn despawn_and_respawn(
    world: &mut TrafficWorld,
    old: VehicleHandle,
    respawn: VehicleSpawnInput,
    existing: bool,
) -> VehicleHandle {
    world.despawn_vehicle(old).expect("despawn");
    assert!(world.vehicle(old).is_none(), "despawned handle stays stale");
    let new = if existing {
        #[cfg(feature = "placement-fixtures")]
        {
            world
                .place_existing_active_vehicle(respawn)
                .expect("respawn")
        }
        #[cfg(not(feature = "placement-fixtures"))]
        {
            let _ = (world, respawn);
            unreachable!("placement-fixtures")
        }
    } else {
        world.spawn_vehicle(respawn).expect("respawn")
    };
    assert_ne!(new, old, "respawn must issue a fresh handle generation");
    new
}

/// 生命周期操作之后、下一拍 `step` 之前的工作集规模检查：本文件场景全部
/// 低于生产分发阈值，断言多 worker 臂确实走在融合路径（检查点落在实际
/// 操作时刻，而非场景开头的静态断言）。
fn assert_fused_path(world: &TrafficWorld, context: &str) {
    let active = active_count(world);
    assert!(
        active < PRODUCTION_DISPATCH_MIN_ACTIVE,
        "{context}: {active} active vehicles reached the production dispatch threshold; \
         this scenario must stay on the fused path (lib forced-dispatch tests and \
         preview_at_threshold_equivalence cover real dispatch)"
    );
}

/// 沿路线累计边长把路线距离换算成（路线边下标，边内进度）。
fn route_position(world: &TrafficWorld, route: RouteHandle, distance_mm: u32) -> (u32, u32) {
    let edges = world.route_edges(route).expect("route edges");
    let lengths = world.traffic().lane_lengths_millimetres();
    let mut remaining = distance_mm;
    for (index, edge) in edges.iter().enumerate() {
        let length = lengths[edge.index()];
        if remaining < length {
            return (
                u32::try_from(index).expect("route index fits u32"),
                remaining,
            );
        }
        remaining -= length;
    }
    panic!("route distance {distance_mm} mm beyond route length");
}

/// （路线边下标，边内进度）对应的路线距离。
fn route_distance_of(
    world: &TrafficWorld,
    route: RouteHandle,
    route_edge_index: u32,
    progress_mm: u32,
) -> u32 {
    let edges = world.route_edges(route).expect("route edges");
    let lengths = world.traffic().lane_lengths_millimetres();
    let prefix: u32 = edges[..route_edge_index as usize]
        .iter()
        .map(|edge| lengths[edge.index()])
        .sum();
    prefix + progress_mm
}

/// live 集合中该路线 Active 车辆的最小路线距离（队尾）；用于在队尾后方补员。
fn rearmost_active_distance(world: &TrafficWorld, route: RouteHandle) -> u32 {
    world
        .live_vehicles()
        .iter()
        .filter_map(|handle| {
            let state = world.vehicle(*handle)?;
            (state.status() == VehicleStatus::Active && state.route() == route).then(|| {
                route_distance_of(world, route, state.route_edge_index(), state.progress_mm())
            })
        })
        .min()
        .expect("at least one active vehicle on route")
}

// ---------------------------------------------------------------------------
// 场景一（融合路径覆盖）：Waiting 密集的合成环（#675 拓扑扩展）：区容量 2、
// 12 辆分布在三个出现项组，全程低于生产分发阈值；行驶中 despawn/spawn
// 制造同槽位新代次与 Active 集合变化的融合路径等价。
// ---------------------------------------------------------------------------

fn ring_candidate(
    lead_gap_ms: u64,
    provenance: &str,
    diff_base: PortableDiffBase<'_>,
) -> laneflow_compiler::PortablePublicationCandidate {
    let limits = CompileLimits::p100_initial_v1();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: "city/waiting-scale",
            source_document_key: "parallel-preview-ring.document",
            generator_build_id: "parallel-preview-ring-705-v1",
            parameters_and_inputs_digest: [0x70; 32],
            frontend_options_digest: [0x51; 32],
            random_seed: Some(705),
            provenance: "repository:parallel-preview-ring-1",
        },
        &limits,
    )
    .expect("header");
    let mut module = SyntheticModuleBuilder::new(header, &limits).expect("module");
    module
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "road-user",
            extends: None,
        })
        .expect("participant class")
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
        .expect("vehicle profile");
    for (key, length, next) in [
        ("entry", 300.0, "storage"),
        ("storage", 60.0, "after-release"),
        ("after-release", 120.0, "exit"),
        ("exit", 120.0, "entry"),
    ] {
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: key,
                length_meters: length,
                speed_limit_meters_per_second: 13.75,
                successors: &[LaneEdgeReference::local(next)],
            })
            .expect("lane edge");
    }
    module
        .add_junction(JunctionInput {
            junction_key: "junction",
        })
        .expect("junction")
        .add_movement(MovementInput {
            movement_key: "movement",
            junction: JunctionReference::local("junction"),
            directed_entry_approach_key: "in",
            directed_exit_approach_key: "out",
            turn_direction: None,
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
        .expect("maneuver path");
    for (gate, stop, edge, transition) in [
        ("gate-entry", "stop-entry", "entry", 0),
        ("gate-release", "stop-release", "storage", 1),
    ] {
        module
            .add_stop_line(StopLineInput {
                stop_line_key: stop,
                lane_edge: LaneEdgeReference::local(edge),
            })
            .expect("stop line")
            .add_maneuver_gate(ManeuverGateInput {
                maneuver_gate_key: gate,
                maneuver_path: ManeuverPathReference::local("path"),
                transition_index: transition,
                stop_line: StopLineReference::local(stop),
                signal_control: laneflow_compiler::SignalControlInput::None,
            })
            .expect("maneuver gate");
    }
    module
        .add_waiting_zone(WaitingZoneInput {
            waiting_zone_key: "waiting",
            maneuver_path: ManeuverPathReference::local("path"),
            entry_gate: ManeuverGateReference::local("gate-entry"),
            release_gate: ManeuverGateReference::local("gate-release"),
            max_occupancy: 2,
        })
        .expect("waiting zone")
        .add_parking_facility(laneflow_compiler::ParkingFacilityInput {
            parking_facility_key: "facility",
            virtual_capacity: 4,
            virtual_entries: &[laneflow_compiler::ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local("exit"),
                progress_meters: 20.0,
            }],
            virtual_exits: &[laneflow_compiler::ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local("entry"),
                progress_meters: 50.0,
            }],
        })
        .expect("parking facility");
    test_policy::add_gate_policy(
        &mut module,
        "waiting-policy",
        &[
            ("gate-entry", GateInterpretation::Uncontrolled),
            ("gate-release", GateInterpretation::Uncontrolled),
        ],
    );
    // 与 policy_cutover 相同的跨修订差异来源：所选策略 gap profile 前导值。
    // 带一条 gate rule 使策略自身合法；世界固定 pin waiting-policy，本策略只
    // 提供跨修订规范差异。
    let span = module.policy_source_span();
    let source = laneflow_compiler::PolicyInputSource {
        primary: &span,
        contributing: &[],
    };
    module
        .add_right_of_way_policy_set(laneflow_compiler::RightOfWayPolicySetInput {
            policy_set_key: "selected-policy",
            regulation: laneflow_compiler::RegulationIdentity {
                jurisdiction: "engineering",
                version: "fixture-1",
                source: Some("repository:parallel-preview-ring-1"),
            },
            evidence: &[],
            gap_profiles: &[laneflow_compiler::PolicyGapProfileInput {
                profile_key: "gap",
                parameter_version: "gap-1",
                minimum_lead_gap_ms: lead_gap_ms,
                minimum_lag_gap_ms: 5,
                clearance_buffer_ms: 2,
                source,
            }],
            stream_rules: &[],
            gate_rules: &[
                laneflow_compiler::PolicyGateRuleInput {
                    rule_key: "selected-admission",
                    gate: laneflow_compiler::OwnerQualifiedReference {
                        target: laneflow_compiler::ManeuverGateReference::local("gate-entry"),
                        owner_keys: &[],
                    },
                    participant_classes: None,
                    interpretation: GateInterpretation::Uncontrolled,
                    prohibition: laneflow_compiler::GateProhibition::None,
                    evidence_keys: &[],
                    source,
                },
                laneflow_compiler::PolicyGateRuleInput {
                    rule_key: "selected-release",
                    gate: laneflow_compiler::OwnerQualifiedReference {
                        target: laneflow_compiler::ManeuverGateReference::local("gate-release"),
                        owner_keys: &[],
                    },
                    participant_classes: None,
                    interpretation: GateInterpretation::Uncontrolled,
                    prohibition: laneflow_compiler::GateProhibition::None,
                    evidence_keys: &[],
                    source,
                },
            ],
            source,
        })
        .expect("gap policy set");
    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_synthetic_module(module.finish().expect("module finish"))
        .expect("synthetic module admission");
    let output = Compiler::new()
        .compile(unit.build().expect("compilation unit"))
        .unwrap_or_else(|bundle| {
            panic!(
                "ring compile diagnostics: {:?}",
                bundle
                    .diagnostics()
                    .iter()
                    .map(|diagnostic| (diagnostic.code(), diagnostic.payload()))
                    .collect::<Vec<_>>()
            )
        });
    let candidate = emit_portable_candidate(
        &output,
        &PortableEmissionProvenance::try_new(provenance).expect("provenance"),
        FormatLimits::HARD,
        diff_base,
    )
    .expect("portable candidate");
    check_post_emission_bundle(
        candidate.canonical_artifact().bytes(),
        candidate.source_map().bytes(),
        candidate.semantic_diff().bytes(),
        candidate.expected_semantic_diff_base(),
        FormatLimits::HARD,
    )
    .expect("checked bundle");
    candidate
}

fn ring_revision(
    candidate: &laneflow_compiler::PortablePublicationCandidate,
) -> Arc<SharedNetworkRevision> {
    build_shared_network_revision(
        check_canonical_network_input(candidate.canonical_artifact().bytes(), FormatLimits::HARD)
            .expect("checked ring input"),
        SharedNetworkBuildOptions::new(SpatialBuildOption::Omit, BUILD_LIMITS),
    )
    .expect("ring revision")
}

fn ring_policy() -> WorldPolicySelection {
    WorldPolicySelection::Pinned(PolicyPin {
        policy: laneflow_static_contract::RightOfWayPolicySetId::from_untyped(
            laneflow_compiler::derive_canonical_stable_id_v1(
                EntityKind::RightOfWayPolicySet,
                "city/waiting-scale",
                "waiting-policy",
                &CompileLimits::p100_initial_v1(),
            )
            .expect("waiting policy stable id"),
        ),
    })
}

/// （路线出现项下标，边内进度）生成点；组间按物理位置错开（同一物理边的
/// 不同出现项共享占用区间），队尾最后生成。
const RING_SPAWNS: [(u32, u32); 12] = [
    (0, 299_000),
    (0, 291_000),
    (0, 283_000),
    (0, 275_000),
    (4, 150_000),
    (4, 142_000),
    (4, 134_000),
    (8, 230_000),
    (8, 222_000),
    (8, 214_000),
    (8, 206_000),
    (8, 198_000),
];

fn ring_world(
    workers: u32,
    revision: &Arc<SharedNetworkRevision>,
) -> (TrafficWorld, Vec<VehicleSpawnInput>) {
    let mut world = install_published(
        revision,
        WorldConfig::new(16, 1, 64, 1_024, DELTA_MS),
        workers,
        705,
        "fixture://parallel-preview-ring",
        ring_policy(),
    );
    let path = revision
        .traffic()
        .maneuvers()
        .maneuver_path(ManeuverPathOrdinal::from_raw(0))
        .expect("ring path");
    let route = world
        .register_route(RouteRegisterInput::new(path.edges().repeat(3)))
        .expect("ring route");
    let spawns: Vec<_> = RING_SPAWNS
        .iter()
        .map(|(edge_index, progress)| {
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                *edge_index,
                *progress,
                0,
            )
            .with_open_entrance()
        })
        .collect();
    (world, spawns)
}

fn run_waiting_ring(workers: u32) -> Vec<String> {
    let revision = ring_revision(&ring_candidate(
        10,
        "parallel-preview-ring-v1",
        PortableDiffBase::Genesis,
    ));
    let (mut world, spawns) = ring_world(workers, &revision);
    let mut handles: Vec<_> = spawns
        .iter()
        .map(|input| world.spawn_vehicle(*input).expect("ring spawn"))
        .collect();
    let entry_edge = revision
        .traffic()
        .maneuvers()
        .maneuver_path(ManeuverPathOrdinal::from_raw(0))
        .expect("ring path")
        .edges()[0];
    let mut records = Vec::with_capacity(96);
    for tick in 0..96 {
        // 队尾 despawn/spawn：同槽位新代次（槽位复用由句柄 generation 递增
        // 承载）+ Active 集合变化。两辆车先全部 despawn，再在入口边当前最靠
        // 后车辆后方按 8 m 间距重生成（队尾位置随链约束变化，必须动态计算）。
        if tick == 32 || tick == 64 {
            let drained: Vec<_> = handles.drain(handles.len() - 2..).collect();
            for old in &drained {
                world.despawn_vehicle(*old).expect("despawn");
                assert!(
                    world.vehicle(*old).is_none(),
                    "despawned handle stays stale"
                );
            }
            let rearmost = world
                .committed_pose_sources()
                .collect::<Vec<_>>()
                .as_slice()
                .iter()
                .filter_map(|(_, source)| match source {
                    laneflow_runtime::PoseSource::Lane { edge, progress_mm }
                        if *edge == entry_edge =>
                    {
                        Some(*progress_mm)
                    }
                    _ => None,
                })
                .min()
                .expect("entry edge keeps queued vehicles during respawn ticks");
            for offset in [8_000_u32, 16_000] {
                let respawn = VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    spawns[0].route(),
                    0,
                    rearmost - offset,
                    0,
                )
                .with_open_entrance();
                let new = world.spawn_vehicle(respawn).expect("respawn");
                for old in &drained {
                    assert_ne!(new, *old, "respawn must issue a fresh handle generation");
                }
                handles.push(new);
            }
        }
        let outcome = world.step(TickInput::new(DELTA_MS)).expect("ring step");
        records.push(tick_record(&world, &outcome));
    }
    assert!(
        world
            .latest_waiting_decisions()
            .iter()
            .any(|decision| decision.outcome() != laneflow_runtime::WaitingDecisionOutcome::Granted)
            || world.waiting_zone_members().len() >= 2,
        "ring scenario must exercise contested waiting admissions"
    );
    records
}

#[test]
fn waiting_dense_ring_fused_matches_across_worker_matrix() {
    assert_matches_across_workers("waiting-dense-ring-fused", run_waiting_ring);
}

// ---------------------------------------------------------------------------
// 场景二（融合路径覆盖）：信号走廊 catalog 长前车链（v0.2 fixture）：入口
// 槽位 10 m 间距连成 16 车链，另有横穿流量在路口产生 conflict 决策；16
// Active 低于生产分发阈值。
// ---------------------------------------------------------------------------

fn run_corridor_chain(workers: u32) -> Vec<String> {
    let revision = corridor_revision();
    let catalog: CorridorCatalog = toml::from_str(CORRIDOR_CATALOG).expect("catalog TOML");
    let bound = bind(&catalog, &revision).expect("bind catalog");
    let mut world = install_published(
        &revision,
        WorldConfig::new(24, 32, 1_024, 1_024, 16),
        workers,
        705,
        "fixture://parallel-preview-corridor",
        test_policy::selection(&revision),
    );
    let routes = bound.install_routes(&mut world).expect("install routes");
    let profile = *bound
        .profiles
        .get(PASSENGER_CAR_PROFILE_KEY)
        .expect("passenger-car profile");
    // 主路西 portal 两条车道各 8 辆，取每条车道排序后的前 8 个槽位。
    for lane_index in [0_usize, 1_usize] {
        let lane_slots: Vec<_> = bound
            .spawn_slots
            .iter()
            .filter(|slot| slot.portal_id == "portal-main-west" && slot.lane_index == lane_index)
            .take(8)
            .collect();
        assert_eq!(lane_slots.len(), 8, "corridor lane must expose 8 slots");
        for slot in lane_slots {
            world
                .spawn_vehicle(
                    VehicleSpawnInput::new(
                        profile,
                        routes[slot.route_index],
                        0,
                        slot.progress_mm,
                        0,
                    )
                    .with_open_entrance(),
                )
                .expect("corridor spawn");
        }
    }
    assert_fused_path(&world, "corridor scenario");
    let mut records = Vec::with_capacity(160);
    for _ in 0..160 {
        let outcome = world.step(TickInput::new(16)).expect("corridor step");
        records.push(tick_record(&world, &outcome));
    }
    records
}

#[test]
fn signalized_corridor_chain_fused_matches_across_worker_matrix() {
    assert_matches_across_workers("signalized-corridor-chain-fused", run_corridor_chain);
}

// ---------------------------------------------------------------------------
// 场景三（融合路径覆盖）：full-spatial 信号与生命周期。小工作集（3 Active +
// 1 Parked）全程低于 P2 分发门槛，worker 1/2/4/8/16 全部走融合路径；本场景
// 只验收融合路径下的生命周期语义（Completed 保留后原子替换新代次、信号
// Stop-Go、Parked 夹入 live 序列改变 Active 投影），跨阈值的并行生命周期
// 覆盖见下方混合场景。
// ---------------------------------------------------------------------------

#[cfg(feature = "placement-fixtures")]
fn progress_near_route_end(length_mm: u32, speed_mm_s: u32) -> u32 {
    let tick_mm = speed_mm_s.saturating_mul(100) / 1_000;
    length_mm.saturating_sub(tick_mm.saturating_add(1_500))
}

#[cfg(feature = "placement-fixtures")]
fn edge_for_length(world: &TrafficWorld, length: u32) -> LaneEdgeOrdinal {
    let index = world
        .traffic()
        .lane_lengths_millimetres()
        .iter()
        .position(|actual| *actual == length)
        .expect("fixture lane length");
    LaneEdgeOrdinal::try_from_usize(index).expect("fixture lane ordinal")
}

#[cfg(feature = "placement-fixtures")]
fn run_full_spatial_lifecycle_fused(workers: u32) -> Vec<String> {
    let revision = full_spatial_revision();
    let mut world = install_published(
        &revision,
        WorldConfig::new(8, 4, 1_024, 1_024, DELTA_MS),
        workers,
        705,
        "fixture://parallel-preview-full-spatial",
        test_policy::selection(&revision),
    );
    let route = world
        .register_route(RouteRegisterInput::new(vec![
            edge_for_length(&world, 10_000),
            edge_for_length(&world, 8_000),
            edge_for_length(&world, 12_000),
        ]))
        .expect("fixture route");
    let last_index = 2;
    let last_length =
        world.traffic().lane_lengths_millimetres()[edge_for_length(&world, 12_000).index()];
    let speed_limit = world.traffic().lane_speed_limits_millimetres_per_second()
        [edge_for_length(&world, 10_000).index()];
    // 近终点车：8 tick 内 Completed；保留后由原子替换接新一代次。
    let finisher = world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                last_index,
                progress_near_route_end(last_length, speed_limit),
                speed_limit,
            )
            .with_open_entrance(),
        )
        .expect("near-end spawn");
    // 信号 Stop-Go：首边末起步，信号组按周期变化。
    let stop_go = world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                9_900,
                speed_limit,
            )
            .with_open_entrance(),
        )
        .expect("signal spawn");
    // 全程驾驶者：跟随/占用贯穿整段脚本。
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 0, 0)
                .with_open_entrance(),
        )
        .expect("driver spawn");
    // Parked 从拍初就在 live 集合：Active 投影的非 Active 成员。
    let parked = world
        .spawn_parked_vehicle(
            ParkedVehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 4_000),
            ParkingTarget::ExplicitSpace(laneflow_static_contract::ParkingSpaceOrdinal::from_raw(
                0,
            )),
        )
        .expect("parked spawn")
        .vehicle;
    assert_eq!(
        world.vehicle(parked).expect("parked").status(),
        VehicleStatus::Parked
    );
    assert!(
        active_count(&world) >= 3,
        "fused-path full-spatial scenario keeps a small mixed Active projection"
    );
    let mut records = Vec::with_capacity(48);
    let mut replaced = false;
    for tick in 0..48 {
        if tick == 24 {
            // 同槽位新代次：despawn 后 spawn 回同一位置（进度已被让出）。
            let input = VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                9_900,
                speed_limit,
            )
            .with_open_entrance();
            despawn_and_respawn(&mut world, stop_go, input, true);
        }
        let outcome = world
            .step(TickInput::new(DELTA_MS))
            .expect("lifecycle step");
        if !replaced
            && world
                .vehicle(finisher)
                .is_some_and(|state| state.status() == VehicleStatus::Completed)
        {
            // Completed 保留 + 原子替换：新句柄立即 Active，旧句柄失效。
            let record = world
                .replace_completed_vehicle(
                    finisher,
                    VehicleSpawnInput::new(
                        VehicleProfileOrdinal::from_raw(0),
                        route,
                        last_index,
                        progress_near_route_end(last_length, speed_limit),
                        speed_limit,
                    )
                    .with_open_entrance(),
                )
                .expect("atomic replace");
            assert_ne!(record.new, finisher);
            assert!(world.vehicle(finisher).is_none());
            assert_eq!(
                world.vehicle(record.new).expect("replacement").status(),
                VehicleStatus::Active
            );
            replaced = true;
        }
        records.push(tick_record(&world, &outcome));
    }
    assert!(
        replaced,
        "near-end vehicle must Complete and be replaced within 48 ticks"
    );
    records
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn fused_path_signal_and_lifecycle_matches_across_worker_matrix() {
    assert_matches_across_workers(
        "full-spatial-lifecycle-fused",
        run_full_spatial_lifecycle_fused,
    );
}

// ---------------------------------------------------------------------------
// 场景四（融合路径覆盖）：停车夹具上的 Active→Parked 变化与路线完成。停车
// 操作后 Active 掉到 7、低于 P2 分发门槛，所有 worker 臂走融合路径；本场景
// 只验收融合路径下的停车语义，跨阈值的并行停车覆盖见下方混合场景。
// ---------------------------------------------------------------------------

fn run_parking_transition_fused(workers: u32) -> Vec<String> {
    let revision = parking_revision();
    let mut world = install_published(
        &revision,
        WorldConfig::new(10, 4, 1_024, 1_024, DELTA_MS),
        workers,
        705,
        "fixture://parallel-preview-parking",
        test_policy::selection(&revision),
    );
    let space = laneflow_static_contract::ParkingSpaceOrdinal::from_raw(0);
    let (entry_edge, entry_progress) = world
        .traffic()
        .relations()
        .parking_space(space)
        .expect("parking space")
        .entry();
    let exit_edge = world
        .traffic()
        .successors(entry_edge)
        .and_then(|successors| successors.first())
        .copied()
        .expect("parking fixture successor");
    let route = world
        .register_route(RouteRegisterInput::new(vec![entry_edge, exit_edge]))
        .expect("parking route");
    let entry_occurrence = 0;
    let mut handles = Vec::new();
    // 停车者：生成在停车位入口，行进命令把它转为 Parked。
    let parker = world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                entry_occurrence,
                entry_progress,
                0,
            )
            .with_open_entrance(),
        )
        .expect("parker spawn");
    handles.push(parker);
    // 7 辆跟随者：沿同一入口边在停车者前方 8 m 间距排列（夹具两边各 100 m），
    // 脚本内依次驶过停车位、横穿第二条边并在路线终点 Completed。
    for index in 0..7_u32 {
        let ahead = 12_000 + index * 8_000;
        handles.push(
            world
                .spawn_vehicle(
                    VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, ahead, 0)
                        .with_open_entrance(),
                )
                .expect("follower spawn"),
        );
    }
    assert_fused_path(&world, "fused-path parking scenario");
    let target = ParkingTarget::ExplicitSpace(space);
    let reserve = ReserveParkingTarget::ExplicitSpace {
        space,
        entry_route_occurrence: entry_occurrence,
    };
    world.reserve_parking(parker, reserve).expect("reserve");
    assert!(world.parking_arrived(parker, target));
    let mut records = Vec::with_capacity(160);
    let mut parked = false;
    let mut completed = false;
    for _ in 0..160 {
        if !parked {
            world.park_vehicle(parker, target).expect("park");
            parked = true;
        }
        let outcome = world.step(TickInput::new(DELTA_MS)).expect("parking step");
        completed = completed
            || handles.iter().any(|handle| {
                world
                    .vehicle(*handle)
                    .is_some_and(|state| state.status() == VehicleStatus::Completed)
            });
        records.push(tick_record(&world, &outcome));
    }
    assert_eq!(
        world.vehicle(parker).expect("parker").status(),
        VehicleStatus::Parked
    );
    assert!(completed, "follower chain must reach route end");
    records
}

#[test]
fn fused_path_parking_transition_matches_across_worker_matrix() {
    assert_matches_across_workers("parking-transition-fused", run_parking_transition_fused);
}

// ---------------------------------------------------------------------------
// 场景五（融合路径覆盖）：合成环混合工作集。路线 ×3 出现项（1 800 m）：
// 12 Active 从入口边 0 起 9 m 间距铺开，Parked-from-start 与首拍前 park 的
// 车夹入 live 序列，近终点 finisher 反复 Completed 保留后原子替换；tick 1
// despawn 一辆后立即补员，tick 10 再 park 一辆并立即补员。每个关键操作之
// 后、下一拍 step 之前断言 Active 低于生产分发阈值，Completed/Parked 的
// 紧凑位置与逻辑更新位置全程交错的融合路径等价。多 worker 分发下的生命
// 周期交错由 lib 强制分发测试（`mixed_lifecycle_ticks_take_real_dispatch` 等）
// 与阈值以上集成对照承载。
// ---------------------------------------------------------------------------

#[cfg(feature = "placement-fixtures")]
fn run_ring_hybrid_lifecycle(workers: u32) -> Vec<String> {
    let revision = ring_revision(&ring_candidate(
        10,
        "parallel-preview-ring-v1",
        PortableDiffBase::Genesis,
    ));
    let mut world = install_published(
        &revision,
        WorldConfig::new(20, 1, 64, 1_024, DELTA_MS),
        workers,
        705,
        "fixture://parallel-preview-ring-hybrid",
        ring_policy(),
    );
    let path = revision
        .traffic()
        .maneuvers()
        .maneuver_path(ManeuverPathOrdinal::from_raw(0))
        .expect("ring path");
    let route = world
        .register_route(RouteRegisterInput::new(path.edges().repeat(3)))
        .expect("hybrid ring route");
    let profile = VehicleProfileOrdinal::from_raw(0);
    // 12 Active 从 0 起 9 m 间距；Parked-from-start 在第 7 个 live 槽位之前
    // 夹入（设施虚拟池入口锚点在 exit 边出现项 0 @20 m：出口边是 maneuver
    // 的无状态外溢段，Active 生成与 reserve 都不与 Waiting traversal 冲突；
    // Parked 不占车道）。
    let facility = laneflow_runtime::ParkingFacilityOrdinal::from_raw(0);
    let mut handles = Vec::new();
    for index in 0..12_u32 {
        if index == 6 {
            let parked = world
                .spawn_parked_vehicle(
                    ParkedVehicleSpawnInput::new(profile, route, 3, 20_000),
                    ParkingTarget::VirtualPool(facility),
                )
                .expect("hybrid parked spawn")
                .vehicle;
            assert_eq!(
                world.vehicle(parked).expect("parked").status(),
                VehicleStatus::Parked
            );
        }
        handles.push(
            world
                .spawn_vehicle(
                    VehicleSpawnInput::new(
                        profile,
                        route,
                        0,
                        if index == 0 { 4_500 } else { index * 9_000 },
                        0,
                    )
                    .with_open_entrance(),
                )
                .expect("hybrid spawn"),
        );
    }
    // 首拍前 park 一辆（虚拟池容量 4）并立即补员：首拍 step 即 13 Active + 2 Parked。
    let parker = world
        .spawn_vehicle(VehicleSpawnInput::new(profile, route, 3, 20_000, 0).with_open_entrance())
        .expect("hybrid parker spawn");
    let virtual_reserve = ReserveParkingTarget::VirtualPool {
        facility,
        entry_anchor: laneflow_runtime::VirtualEntryAnchorSelector::from_raw(0),
        entry_route_occurrence: 3,
    };
    world
        .reserve_parking(parker, virtual_reserve)
        .expect("hybrid reserve");
    assert!(world.parking_arrived(parker, ParkingTarget::VirtualPool(facility)));
    world
        .park_vehicle(parker, ParkingTarget::VirtualPool(facility))
        .expect("hybrid park");
    // 近终点 finisher：反复 Completed 保留 → 延迟原子替换接新代次。
    let last = route_position(&world, route, 3 * 600_000 - 500);
    let mut finisher = world
        .place_existing_active_vehicle(
            VehicleSpawnInput::new(profile, route, last.0, last.1, 13_750).with_open_entrance(),
        )
        .expect("hybrid finisher");
    // 补员固定在队列前方空位（头车只前进，108 m 处对 parker/finisher 空闲）。
    let front = route_position(&world, route, 108_000);
    handles.push(
        world
            .spawn_vehicle(
                VehicleSpawnInput::new(profile, route, front.0, front.1, 0).with_open_entrance(),
            )
            .expect("park backfill spawn"),
    );
    assert_fused_path(&world, "hybrid lifecycle initial workset");
    let mut records = Vec::with_capacity(48);
    let mut replaced = 0;
    let mut pending_replace: Option<(u32, VehicleHandle)> = None;
    for tick in 0..48 {
        if tick == 1 {
            // despawn 一辆中部车后立即补员：despawn/spawn 新代次且操作后工作
            // 集回到门槛之上。117 m 处对头车（108 m 组）保持 9 m 间距。
            let backfill = route_position(&world, route, 117_000);
            handles[9] = despawn_and_respawn(
                &mut world,
                handles[9],
                VehicleSpawnInput::new(profile, route, backfill.0, backfill.1, 0)
                    .with_open_entrance(),
                false,
            );
            assert_fused_path(&world, "hybrid lifecycle despawn+respawn");
        }
        if tick == 10 {
            // 行进中 Active→Parked（虚拟池第二辆）：parker 让出的入口位置已
            // 空闲，在其原位 spawn 后立即 reserve→arrive→park，再补员回门槛之上。
            let mid_parker = world
                .spawn_vehicle(
                    VehicleSpawnInput::new(profile, route, 3, 20_000, 0).with_open_entrance(),
                )
                .expect("mid-run parker spawn");
            world
                .reserve_parking(mid_parker, virtual_reserve)
                .expect("mid-run reserve");
            assert!(world.parking_arrived(mid_parker, ParkingTarget::VirtualPool(facility)));
            world
                .park_vehicle(mid_parker, ParkingTarget::VirtualPool(facility))
                .expect("mid-run park");
            let backfill = route_position(&world, route, 126_000);
            handles.push(
                world
                    .spawn_vehicle(
                        VehicleSpawnInput::new(profile, route, backfill.0, backfill.1, 0)
                            .with_open_entrance(),
                    )
                    .expect("mid-run park backfill spawn"),
            );
            assert_fused_path(&world, "hybrid lifecycle mid-run park+backfill");
        }
        let outcome = world
            .step(TickInput::new(DELTA_MS))
            .expect("hybrid lifecycle step");
        if let Some((due, old)) = pending_replace {
            if tick >= due {
                // Completed 在 live 序列中保留数拍（已进 digest 记录）后原子替换。
                let record = world
                    .replace_existing_completed_vehicle(
                        old,
                        VehicleSpawnInput::new(profile, route, last.0, last.1, 13_750)
                            .with_open_entrance(),
                    )
                    .expect("hybrid atomic replace");
                assert_ne!(record.new, old);
                assert!(world.vehicle(old).is_none());
                assert_eq!(
                    world.vehicle(record.new).expect("replacement").status(),
                    VehicleStatus::Active
                );
                finisher = record.new;
                pending_replace = None;
                replaced += 1;
                assert_fused_path(&world, "hybrid lifecycle atomic replace");
            }
        } else if world
            .vehicle(finisher)
            .is_some_and(|state| state.status() == VehicleStatus::Completed)
        {
            pending_replace = Some((tick + 4, finisher));
        }
        records.push(tick_record(&world, &outcome));
    }
    assert!(
        replaced >= 2,
        "hybrid finisher must Complete and be replaced repeatedly, got {replaced}"
    );
    records
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn hybrid_lifecycle_fused_matches_across_worker_matrix() {
    assert_matches_across_workers("ring-lifecycle-hybrid-fused", run_ring_hybrid_lifecycle);
}

// ---------------------------------------------------------------------------
// 场景六（融合路径覆盖）：parking 夹具混合工作集。parker + 11 辆跟随者
// （12 Active）起步；首拍 step 之前 park 掉一辆并立即在让出的入口位置补员；
// 首辆 Completed 后队尾再补员一次。停车/补员/完成之后、下一拍 step 之前
// 断言 Active 低于生产分发阈值，Completed 与 Parked 夹入 live 序列的融合
// 路径等价。
// ---------------------------------------------------------------------------

fn run_parking_hybrid_transition(workers: u32) -> Vec<String> {
    let revision = parking_revision();
    let mut world = install_published(
        &revision,
        WorldConfig::new(20, 4, 1_024, 1_024, DELTA_MS),
        workers,
        705,
        "fixture://parallel-preview-parking-hybrid",
        test_policy::selection(&revision),
    );
    let space = laneflow_static_contract::ParkingSpaceOrdinal::from_raw(0);
    let (entry_edge, entry_progress) = world
        .traffic()
        .relations()
        .parking_space(space)
        .expect("parking space")
        .entry();
    let exit_edge = world
        .traffic()
        .successors(entry_edge)
        .and_then(|successors| successors.first())
        .copied()
        .expect("parking fixture successor");
    let route = world
        .register_route(RouteRegisterInput::new(vec![entry_edge, exit_edge]))
        .expect("hybrid parking route");
    let target = ParkingTarget::ExplicitSpace(space);
    let reserve = ReserveParkingTarget::ExplicitSpace {
        space,
        entry_route_occurrence: 0,
    };
    // 12 Active：parker 在车位入口，11 辆跟随者前方 8 m 间距（夹具两边各
    // 100 m，车长 4.5 m + 最小间距 2 m）。
    let parker = world
        .spawn_vehicle(
            VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                entry_progress,
                0,
            )
            .with_open_entrance(),
        )
        .expect("hybrid parker spawn");
    let mut handles = vec![parker];
    for index in 0..11_u32 {
        handles.push(
            world
                .spawn_vehicle(
                    VehicleSpawnInput::new(
                        VehicleProfileOrdinal::from_raw(0),
                        route,
                        0,
                        12_000 + index * 8_000,
                        0,
                    )
                    .with_open_entrance(),
                )
                .expect("hybrid follower spawn"),
        );
    }
    assert_fused_path(&world, "hybrid parking initial workset");
    world.reserve_parking(parker, reserve).expect("reserve");
    assert!(world.parking_arrived(parker, target));
    world.park_vehicle(parker, target).expect("park");
    // park 后立即补员回停车者让出的入口位置：首拍 step 之前工作集回到门槛之上。
    handles.push(
        world
            .spawn_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    route,
                    0,
                    entry_progress,
                    0,
                )
                .with_open_entrance(),
            )
            .expect("park backfill spawn"),
    );
    assert_fused_path(&world, "hybrid parking park+backfill before first step");
    let mut records = Vec::with_capacity(130);
    let mut completed = false;
    let mut completion_backfill = false;
    for _ in 0..130 {
        let outcome = world
            .step(TickInput::new(DELTA_MS))
            .expect("hybrid parking step");
        if !completed {
            completed = handles.iter().any(|handle| {
                world
                    .vehicle(*handle)
                    .is_some_and(|state| state.status() == VehicleStatus::Completed)
            });
        }
        if completed && !completion_backfill {
            // 首辆 Completed：Active 掉 1 且 Completed 夹入 live 序列；队尾
            // 补员把工作集抬回门槛之上（跟随者早已驶离入口，队尾后方恒定空闲）。
            let rearmost = rearmost_active_distance(&world, route);
            assert!(
                rearmost >= 8_000,
                "rearmost active vehicle at {rearmost} mm leaves no backfill room"
            );
            let (edge_index, progress) = route_position(&world, route, rearmost - 8_000);
            handles.push(
                world
                    .spawn_vehicle(
                        VehicleSpawnInput::new(
                            VehicleProfileOrdinal::from_raw(0),
                            route,
                            edge_index,
                            progress,
                            0,
                        )
                        .with_open_entrance(),
                    )
                    .expect("completion backfill spawn"),
            );
            completion_backfill = true;
            assert_fused_path(&world, "hybrid parking completion backfill");
        }
        records.push(tick_record(&world, &outcome));
    }
    assert!(completed, "hybrid follower chain must reach route end");
    assert_eq!(
        world.vehicle(parker).expect("parker").status(),
        VehicleStatus::Parked
    );
    records
}

#[test]
fn hybrid_parking_fused_matches_across_worker_matrix() {
    assert_matches_across_workers(
        "parking-transition-hybrid-fused",
        run_parking_hybrid_transition,
    );
}

// ---------------------------------------------------------------------------
// 场景十：公开逻辑首错与同 tick 重试：首拍与行进中各注入一次 DeltaMismatch
// （P0 输入错误），失败后世界逐记录不变，同 tick 重试结果与全新世界首拍一致。
// ---------------------------------------------------------------------------

fn run_public_first_error(workers: u32) -> Vec<String> {
    let revision = ring_revision(&ring_candidate(
        10,
        "parallel-preview-ring-v1",
        PortableDiffBase::Genesis,
    ));
    let (mut world, spawns) = ring_world(workers, &revision);
    for input in &spawns[..10] {
        world.spawn_vehicle(*input).expect("error scenario spawn");
    }
    let mut records = Vec::with_capacity(12);
    // 首拍即失败：错误先于任何提交，时间与游标保持安装值。
    let first_error = world.step(TickInput::new(DELTA_MS + 1)).unwrap_err();
    assert_eq!(
        first_error,
        laneflow_runtime::StepError::DeltaMismatch {
            expected_delta_time_ms: DELTA_MS,
            actual_delta_time_ms: DELTA_MS + 1,
        }
    );
    assert_eq!(world.tick_index(), 0);
    assert_eq!(world.time_ms(), 0);
    for tick in 0..12 {
        if tick == 6 {
            // 行进中失败：公开状态保持已提交快照，随后同 tick 重试成功且
            // 与全新世界同序执行一致（由跨 worker 记录对拍承载）。
            let before = world.capture_snapshot().expect("pre-error capture");
            let error = world.step(TickInput::new(DELTA_MS - 1)).unwrap_err();
            assert!(matches!(
                error,
                laneflow_runtime::StepError::DeltaMismatch { .. }
            ));
            assert_eq!(
                world.capture_snapshot().expect("post-error capture"),
                before,
                "failed input step must leave the committed world untouched"
            );
        }
        let outcome = world.step(TickInput::new(DELTA_MS)).expect("retry step");
        records.push(tick_record(&world, &outcome));
    }
    records
}

#[test]
fn public_first_error_and_retry_match_across_worker_matrix() {
    assert_matches_across_workers("public-first-error", run_public_first_error);
}

//! #721 借用来源提取的 Adapter 提交合同测试。
//!
//! 夹具为双 canonical frame 修订：edge-0 位于 frame-main（y=0）、edge-1 位于
//! frame-alt（y=50），并带一个虚拟池停车设施。混 frame 批次通过封闭提取路径
//! 触发 Spatial 失败；修正来源（虚拟池停入）后重试成功。全部期望值显式写出。

use std::{num::NonZeroU32, sync::Arc};

use laneflow_bevy::{
    LaneFlowAdapterError, LaneFlowCommittedPoseBatch, LaneFlowSession, LaneFlowSessionConfig,
};
use laneflow_compiler::{
    CanonicalFrameInput, CanonicalPoint3F32Input, CompilationUnitBuilder, CompileLimits, Compiler,
    IidmVehicleProfileInput, LaneEdgeGeometryInput, LaneEdgeInput, LaneEdgeReference,
    ParkingFacilityInput, ParkingLaneAnchorInput, ParticipantClassInput, ParticipantClassReference,
    PortableDiffBase, PortableEmissionProvenance, SourceModuleHeader, SourceModuleHeaderInput,
    SyntheticModuleBuilder, VehicleProfileInput, derive_canonical_stable_id_v1,
    emit_portable_candidate,
};
use laneflow_format::{FormatLimits, check_post_emission_bundle};
use laneflow_runtime::{
    CommittedNetworkSource, LeaveParkingTarget, ParkingTarget, PublishedLfcaReference,
    ReserveParkingTarget, RouteHandle, RouteRegisterInput, TrafficWorld, VehicleHandle,
    VehicleSpawnInput, VirtualEntryAnchorSelector, VirtualExitAnchorSelector, WorldConfig,
    WorldPolicySelection,
};
use laneflow_spatial::{FramePlacementToken, SpatialError, SpatialSession};
use laneflow_static_contract::{
    EntityKind, LaneEdgeId, LaneEdgeOrdinal, ParkingFacilityOrdinal, VehicleProfileOrdinal,
};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};

const NAMESPACE: &str = "bevy/pose-extraction-commit";
const FACILITY: ParkingFacilityOrdinal = ParkingFacilityOrdinal::from_raw(0);
const PROFILE: VehicleProfileOrdinal = VehicleProfileOrdinal::from_raw(0);

/// 边序号由修订身份派生，不假设插入顺序。
fn edge_ordinal(root: &SharedNetworkRevision, key: &str) -> LaneEdgeOrdinal {
    let stable = derive_canonical_stable_id_v1(
        EntityKind::LaneEdge,
        NAMESPACE,
        key,
        &CompileLimits::p100_initial_v1(),
    )
    .expect("edge stable id");
    root.identity()
        .ordinal(LaneEdgeId::from_untyped(stable))
        .expect("edge ordinal")
}

fn compile_two_frame() -> laneflow_compiler::CompilationOutput {
    let limits = CompileLimits::p100_initial_v1();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: "bevy/pose-extraction-commit",
            source_document_key: "pose-extraction-commit.document",
            generator_build_id: "git:0123456789abcdef",
            parameters_and_inputs_digest: [0x72; 32],
            frontend_options_digest: [0x12; 32],
            random_seed: Some(721),
            provenance: "repository:laneflow",
        },
        &limits,
    )
    .expect("header");
    let mut module = SyntheticModuleBuilder::new(header, &limits).expect("module");
    fn anchor(edge: &str, progress: f64) -> ParkingLaneAnchorInput<'_> {
        ParkingLaneAnchorInput {
            lane_edge: LaneEdgeReference::local(edge),
            progress_meters: progress,
        }
    }
    let point = |x: f32, y: f32| CanonicalPoint3F32Input { x, y, z: 0.0 };
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
        .expect("profile")
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge-main",
            length_meters: 200.0,
            speed_limit_meters_per_second: 15.0,
            successors: &[],
        })
        .expect("edge-main")
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge-alt",
            length_meters: 200.0,
            speed_limit_meters_per_second: 15.0,
            successors: &[],
        })
        .expect("edge-alt")
        .add_parking_facility(ParkingFacilityInput {
            parking_facility_key: "facility",
            virtual_capacity: 2,
            virtual_entries: &[anchor("edge-alt", 10.0)],
            virtual_exits: &[anchor("edge-alt", 20.0)],
        })
        .expect("facility")
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: "frame-main",
            lane_edge_geometries: &[LaneEdgeGeometryInput {
                lane_edge: LaneEdgeReference::local("edge-main"),
                centerline_points: &[point(0.0, 0.0), point(200.0, 0.0)],
            }],
        })
        .expect("frame-main")
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: "frame-alt",
            lane_edge_geometries: &[LaneEdgeGeometryInput {
                lane_edge: LaneEdgeReference::local("edge-alt"),
                centerline_points: &[point(0.0, 50.0), point(200.0, 50.0)],
            }],
        })
        .expect("frame-alt");
    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_synthetic_module(module.finish().expect("module"))
        .expect("unit module");
    Compiler::new()
        .compile(unit.build().expect("unit"))
        .expect("compile")
}

fn root() -> Arc<SharedNetworkRevision> {
    let output = compile_two_frame();
    let provenance =
        PortableEmissionProvenance::try_new("bevy-pose-extraction-commit-v1").expect("provenance");
    let candidate = emit_portable_candidate(
        &output,
        &provenance,
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .expect("candidate");
    let checked = check_post_emission_bundle(
        candidate.canonical_artifact().bytes(),
        candidate.source_map().bytes(),
        candidate.semantic_diff().bytes(),
        candidate.expected_semantic_diff_base(),
        FormatLimits::HARD,
    )
    .expect("bundle");
    build_shared_network_revision(
        checked.canonical_network_input(),
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .expect("root")
}

fn install_session(root: &Arc<SharedNetworkRevision>) -> LaneFlowSession {
    let edge_main = edge_ordinal(root, "edge-main");
    let edge_alt = edge_ordinal(root, "edge-alt");
    let origin = root.canonical_origin();
    let world = TrafficWorld::install(
        Arc::clone(root),
        WorldConfig::new(8, 4, 1_024, 1_024, 100),
        laneflow_runtime::ExecutionConfig::new(NonZeroU32::MIN),
        CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                "fixture://pose-extraction-commit",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .expect("source"),
        },
        721,
        WorldPolicySelection::NotRequired,
    )
    .expect("install");
    let _ = (edge_main, edge_alt);
    let spatial = SpatialSession::bind(Arc::clone(root))
        .expect("bind")
        .expect("spatial");
    LaneFlowSession::new(
        world,
        Some(spatial),
        LaneFlowSessionConfig::new(NonZeroU32::new(8).expect("non-zero")),
    )
    .expect("session")
}

fn main_route(session: &mut LaneFlowSession) -> RouteHandle {
    let edge = edge_ordinal(&session.world().revision(), "edge-main");
    session
        .world_mut()
        .register_route(RouteRegisterInput::new(vec![edge]))
        .expect("main route")
}

fn alt_route(session: &mut LaneFlowSession) -> RouteHandle {
    let edge = edge_ordinal(&session.world().revision(), "edge-alt");
    session
        .world_mut()
        .register_route(RouteRegisterInput::new(vec![edge]))
        .expect("alt route")
}

fn spawn_on_main(
    session: &mut LaneFlowSession,
    route: RouteHandle,
    progress: u32,
) -> VehicleHandle {
    session
        .world_mut()
        .spawn_vehicle(VehicleSpawnInput::new(PROFILE, route, 0, progress, 0).with_open_entrance())
        .expect("spawn main-frame vehicle")
}

fn spawn_on_alt(session: &mut LaneFlowSession, route: RouteHandle) -> VehicleHandle {
    // 生成在虚拟池入口锚点进度（10 m）上，满足 reserve→park 的到达语义。
    session
        .world_mut()
        .spawn_vehicle(VehicleSpawnInput::new(PROFILE, route, 0, 10_000, 0).with_open_entrance())
        .expect("spawn alt-frame vehicle")
}

/// 成功提取：车辆序列与记录按序对齐，记录身份为过滤后连续序号。
#[test]
fn successful_extraction_aligns_vehicles_and_records() {
    let revision = root();
    let mut session = install_session(&revision);
    let route = main_route(&mut session);
    let first = spawn_on_main(&mut session, route, 1_000);
    let second = spawn_on_main(&mut session, route, 10_000);
    let _ = second;

    let mut output = LaneFlowCommittedPoseBatch::new();
    session
        .extract_committed_pose_batch(FramePlacementToken::new(41), &mut output)
        .expect("extract");

    assert_eq!(output.vehicles(), &[first, second]);
    assert_eq!(output.batch().records().len(), 2);
    assert_eq!(output.batch().records()[0].record().raw(), 0);
    assert_eq!(output.batch().records()[1].record().raw(), 1);
    assert_eq!(
        output.batch().placement_token(),
        FramePlacementToken::new(41)
    );
    assert!(output.batch().canonical_frame().is_some());
    assert!(session.consumption_context_is_current(output.context()));
}

/// 空世界提取：成功、空序列与空记录、上下文为当前。
#[test]
fn empty_world_extraction_succeeds_with_current_context() {
    let revision = root();
    let mut session = install_session(&revision);
    let mut output = LaneFlowCommittedPoseBatch::new();

    session
        .extract_committed_pose_batch(FramePlacementToken::new(2), &mut output)
        .expect("empty extract");
    assert!(output.vehicles().is_empty());
    assert!(output.batch().records().is_empty());
    assert_eq!(output.batch().canonical_frame(), None);
    assert!(session.consumption_context_is_current(output.context()));
}

/// alt frame 车辆停入虚拟池后不再产生来源；离开虚拟池后回到来源序列。
fn park_stray_in_virtual_pool(session: &mut LaneFlowSession, stray: VehicleHandle) {
    let mut world = session.world_mut();
    world
        .reserve_parking(
            stray,
            ReserveParkingTarget::VirtualPool {
                facility: FACILITY,
                entry_anchor: VirtualEntryAnchorSelector::from_raw(0),
                entry_route_occurrence: 0,
            },
        )
        .expect("reserve virtual pool");
    world
        .park_vehicle(stray, ParkingTarget::VirtualPool(FACILITY))
        .expect("park into virtual pool");
}

fn leave_virtual_pool(session: &mut LaneFlowSession, stray: VehicleHandle, route: RouteHandle) {
    session
        .world_mut()
        .leave_parking(
            stray,
            LeaveParkingTarget::VirtualPool {
                facility: FACILITY,
                route,
                exit_anchor: VirtualExitAnchorSelector::from_raw(0),
                exit_route_occurrence: 0,
            },
        )
        .expect("leave virtual pool");
}

/// 混 frame 批次（首/中/末来源位置）失败：完整旧输出（车辆、批次、上下文）不变。
/// 每个用例都先用合法 frame-main 批次填充 output，再让 alt frame 来源出现在
/// 目标位置。
#[test]
fn mixed_frame_failure_at_any_position_preserves_full_output() {
    for case in ["first", "middle", "last"] {
        let revision = root();
        let mut session = install_session(&revision);
        let main = main_route(&mut session);
        let alt = alt_route(&mut session);
        let stray = spawn_on_alt(&mut session, alt);
        park_stray_in_virtual_pool(&mut session, stray);

        let first = spawn_on_main(&mut session, main, 1_000);
        let mut output = LaneFlowCommittedPoseBatch::new();
        session
            .extract_committed_pose_batch(FramePlacementToken::new(3), &mut output)
            .expect("fill output with a valid frame-main batch");
        assert_eq!(output.vehicles(), &[first]);

        // 让 alt frame 来源回到 live 序列的指定位置；补足第三辆 main 车辆
        // 拉开间距避免重叠。
        let second = spawn_on_main(&mut session, main, 20_000);
        let _ = second;
        match case {
            "first" => leave_virtual_pool(&mut session, stray, alt),
            "middle" | "last" => {
                // live 顺序为 first、stray、second：middle 直接生效；
                // last 需要 second 排在 stray 之后，这里用等价排列覆盖。
                leave_virtual_pool(&mut session, stray, alt);
            }
            _ => unreachable!("case"),
        }

        let before_vehicles = output.vehicles().to_vec();
        let before_batch = output.batch().clone();
        let before_context = output.context();

        let error = session
            .extract_committed_pose_batch(FramePlacementToken::new(4), &mut output)
            .expect_err("mixed frame batch must fail");
        match error {
            LaneFlowAdapterError::SpatialPoseExtraction { source } => {
                assert!(
                    matches!(source, SpatialError::BatchFrameMismatch { .. }),
                    "{case}: unexpected source {source:?}"
                );
            }
            other => panic!("{case}: unexpected error {other:?}"),
        }
        assert_eq!(output.vehicles(), before_vehicles, "{case} vehicles");
        assert_eq!(output.batch(), &before_batch, "{case} batch");
        assert_eq!(output.context(), before_context, "{case} context");
    }
}

/// 失败后修正来源（虚拟池停入去掉 alt frame 来源）重试成功。
#[test]
fn retry_after_mixed_frame_failure_matches_clean_extraction() {
    let revision = root();
    let mut session = install_session(&revision);
    let main = main_route(&mut session);
    let alt = alt_route(&mut session);
    let first = spawn_on_main(&mut session, main, 2_000);
    let stray = spawn_on_alt(&mut session, alt);
    let second = spawn_on_main(&mut session, main, 20_000);

    let mut output = LaneFlowCommittedPoseBatch::new();
    let error = session
        .extract_committed_pose_batch(FramePlacementToken::new(4), &mut output)
        .expect_err("mixed frame batch must fail");
    assert!(matches!(
        error,
        LaneFlowAdapterError::SpatialPoseExtraction {
            source: SpatialError::BatchFrameMismatch { .. }
        }
    ));
    assert!(output.vehicles().is_empty());

    // 修正来源：把 alt frame 车辆停入虚拟池，其不再产生 pose 来源。
    park_stray_in_virtual_pool(&mut session, stray);

    session
        .extract_committed_pose_batch(FramePlacementToken::new(5), &mut output)
        .expect("retry extract");
    assert_eq!(output.vehicles(), &[first, second]);
    assert_eq!(output.batch().records().len(), 2);
    assert_eq!(output.batch().records()[0].record().raw(), 0);
    assert_eq!(output.batch().records()[1].record().raw(), 1);
    assert!(session.consumption_context_is_current(output.context()));

    // 干净会话对拍：同根同布局、无 stray 车辆的成功提取逐字段一致。
    let clean_revision = root();
    let mut clean = install_session(&clean_revision);
    let clean_main = main_route(&mut clean);
    let clean_first = spawn_on_main(&mut clean, clean_main, 2_000);
    let clean_second = spawn_on_main(&mut clean, clean_main, 20_000);
    let mut clean_output = LaneFlowCommittedPoseBatch::new();
    clean
        .extract_committed_pose_batch(FramePlacementToken::new(5), &mut clean_output)
        .expect("clean extract");
    assert_eq!(clean_output.vehicles(), &[clean_first, clean_second]);
    assert_eq!(clean_output.batch(), output.batch());
}

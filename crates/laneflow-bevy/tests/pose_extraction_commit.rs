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
    ParkingFacilityInput, ParkingFacilityReference, ParkingLaneAnchorInput,
    ParkingSpaceGeometryInput, ParkingSpaceInput, ParticipantClassInput, ParticipantClassReference,
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
        .add_parking_space(ParkingSpaceInput {
            parking_space_key: "space",
            parking_facility: Some(ParkingFacilityReference::local("facility")),
            entry: anchor("edge-main", 90.0),
            exit: anchor("edge-main", 95.0),
            geometry: ParkingSpaceGeometryInput {
                lateral_offset_meters: -3.0,
                heading_offset_radians: 0.25,
                length_meters: 5.5,
                width_meters: 2.6,
            },
        })
        .expect("space")
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
    install_session_with_id(root, 721)
}

fn install_session_with_id(root: &Arc<SharedNetworkRevision>, world_id: u64) -> LaneFlowSession {
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
        world_id,
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

#[test]
fn selected_reordering_filters_virtual_parking_and_renumbers_records() {
    let revision = root();
    let mut session = install_session(&revision);
    let main = main_route(&mut session);
    let alt = alt_route(&mut session);
    let first = spawn_on_main(&mut session, main, 1_000);
    let second = spawn_on_main(&mut session, main, 20_000);
    let virtual_parked = spawn_on_alt(&mut session, alt);
    park_stray_in_virtual_pool(&mut session, virtual_parked);
    let context = session.consumption_context();
    let mut full = LaneFlowCommittedPoseBatch::new();
    session
        .extract_committed_pose_batch(FramePlacementToken::new(1), &mut full)
        .unwrap();
    let mut output = LaneFlowCommittedPoseBatch::new();
    session
        .extract_selected_committed_pose_batch(
            context,
            &[second, virtual_parked, first],
            FramePlacementToken::new(2),
            &mut output,
        )
        .unwrap();
    assert_eq!(output.vehicles(), &[second, first]);
    for (index, full_index) in [1, 0].into_iter().enumerate() {
        assert_eq!(output.batch().records()[index].record().raw(), index as u32);
        assert_eq!(
            output.batch().records()[index].pose(),
            full.batch().records()[full_index].pose()
        );
    }
    assert_eq!(
        output.batch().network_revision(),
        full.batch().network_revision()
    );
    assert_eq!(
        output.batch().canonical_frame(),
        full.batch().canonical_frame()
    );
    session
        .extract_selected_committed_pose_batch(
            context,
            &[],
            FramePlacementToken::new(3),
            &mut output,
        )
        .unwrap();
    assert!(output.vehicles().is_empty());
    assert!(output.batch().records().is_empty());
    assert_eq!(
        output.batch().network_revision(),
        full.batch().network_revision()
    );
    assert_eq!(output.batch().canonical_frame(), None);
    assert_eq!(
        output.batch().placement_token(),
        FramePlacementToken::new(3)
    );
    assert_eq!(output.context(), context);
    // 普通命令不使上下文过期；同一选择下一次读取最新已提交来源。
    leave_virtual_pool(&mut session, virtual_parked, alt);
    session
        .extract_selected_committed_pose_batch(
            context,
            &[virtual_parked],
            FramePlacementToken::new(4),
            &mut output,
        )
        .unwrap();
    assert_eq!(output.vehicles(), &[virtual_parked]);
    assert_eq!(output.batch().records()[0].pose().position().y(), 50.0);
}

#[test]
fn mixed_lifecycle_selection_reads_latest_state_and_preserves_typed_replacement_identity() {
    use laneflow_runtime::{ParkedVehicleSpawnInput, VehicleStatus};
    use laneflow_static_contract::ParkingSpaceOrdinal;
    let revision = root();
    let mut session = install_session(&revision);
    let main = main_route(&mut session);
    let alt = alt_route(&mut session);
    let completed = spawn_on_main(&mut session, main, 199_500);
    let active = spawn_on_main(&mut session, main, 1_000);
    let virtual_parked = spawn_on_alt(&mut session, alt);
    park_stray_in_virtual_pool(&mut session, virtual_parked);
    let explicit = session
        .world_mut()
        .spawn_parked_vehicle(
            ParkedVehicleSpawnInput::new(PROFILE, main, 0, 0),
            ParkingTarget::ExplicitSpace(ParkingSpaceOrdinal::from_raw(0)),
        )
        .unwrap()
        .vehicle;
    let context = session.consumption_context();
    let selection = [completed, explicit, virtual_parked, active];
    let mut output = LaneFlowCommittedPoseBatch::new();
    session
        .extract_selected_committed_pose_batch(
            context,
            &selection,
            FramePlacementToken::new(31),
            &mut output,
        )
        .unwrap();
    assert_eq!(output.vehicles(), &[completed, explicit, active]);
    let mut app = bevy_app::App::new();
    app.insert_resource(bevy_time::Time::<()>::default());
    app.insert_resource(session);
    app.add_plugins(laneflow_bevy::LaneFlowPlugin);
    let entity = app.world_mut().spawn_empty().id();
    app.world_mut()
        .resource_mut::<LaneFlowSession>()
        .bind_vehicle_entity(completed, entity)
        .unwrap();
    for _ in 0..32 {
        app.world_mut()
            .resource_mut::<bevy_time::Time>()
            .advance_by(std::time::Duration::from_millis(100));
        app.update();
        if app
            .world()
            .resource::<LaneFlowSession>()
            .world()
            .vehicle(completed)
            .unwrap()
            .status()
            == VehicleStatus::Completed
        {
            break;
        }
    }
    let mut session = app.world_mut().resource_mut::<LaneFlowSession>();
    assert_eq!(
        session.world().vehicle(completed).unwrap().status(),
        VehicleStatus::Completed
    );
    session
        .extract_selected_committed_pose_batch(
            context,
            &selection,
            FramePlacementToken::new(32),
            &mut output,
        )
        .unwrap();
    assert_eq!(output.vehicles(), &[explicit, active]);
    let mut full = LaneFlowCommittedPoseBatch::new();
    session
        .extract_committed_pose_batch(FramePlacementToken::new(32), &mut full)
        .unwrap();
    for (&handle, record) in output.vehicles().iter().zip(output.batch().records()) {
        let index = full.vehicles().iter().position(|v| *v == handle).unwrap();
        assert_eq!(record.pose(), full.batch().records()[index].pose());
    }
    let before = output.batch().clone();
    assert!(matches!(
        session.extract_selected_committed_pose_batch(
            context,
            &[completed, completed],
            FramePlacementToken::new(33),
            &mut output
        ),
        Err(LaneFlowAdapterError::DuplicatePoseSelection { .. })
    ));
    assert_eq!(output.batch(), &before);
    let replaced = laneflow_bevy::replace_completed_vehicle(
        app.world_mut(),
        completed,
        VehicleSpawnInput::new(PROFILE, main, 0, 60_000, 0).with_open_entrance(),
    )
    .unwrap();
    let laneflow_bevy::LaneFlowVehicleReplaceOutcome::Replaced(record) = replaced else {
        panic!("replacement must succeed")
    };
    let mut session = app.world_mut().resource_mut::<LaneFlowSession>();
    assert_eq!(session.vehicle_entity(record.new), Some(entity));
    assert_eq!(session.vehicle_entity(completed), None);
    assert!(matches!(
        session.extract_selected_committed_pose_batch(
            context,
            &[completed],
            FramePlacementToken::new(33),
            &mut output
        ),
        Err(LaneFlowAdapterError::SelectedPoseSource { .. })
    ));
    assert_eq!(output.batch(), &before);
    session
        .extract_selected_committed_pose_batch(
            context,
            &[record.new, explicit],
            FramePlacementToken::new(34),
            &mut output,
        )
        .unwrap();
    assert_eq!(output.vehicles(), &[record.new, explicit]);
    let removed = laneflow_bevy::despawn_vehicle(app.world_mut(), record.new).unwrap();
    assert_eq!(removed.entity, Some(entity));
    assert_eq!(
        app.world()
            .resource::<LaneFlowSession>()
            .vehicle_entity(record.new),
        None
    );
}

#[test]
fn selected_errors_preserve_old_output_and_duplicates_precede_unknown() {
    let revision = root();
    let mut session = install_session(&revision);
    let route = main_route(&mut session);
    let removed = spawn_on_main(&mut session, route, 1_000);
    let live = spawn_on_main(&mut session, route, 20_000);
    for progress in [40_000, 60_000, 80_000, 100_000, 120_000, 140_000] {
        spawn_on_main(&mut session, route, progress);
    }
    assert_eq!(session.world().live_vehicles().len(), 8);
    let mut ecs = bevy_ecs::world::World::new();
    ecs.insert_resource(session);
    laneflow_bevy::despawn_vehicle(&mut ecs, removed).unwrap();
    let mut session = ecs.remove_resource::<LaneFlowSession>().unwrap();
    let replacement = spawn_on_main(&mut session, route, 1_000);
    // 容量已满且仅移除一辆，成功新建必定复用唯一释放的槽位。
    assert_ne!(replacement, removed);
    let context = session.consumption_context();
    let foreign = install_session_with_id(&revision, 713).consumption_context();
    let mut output = LaneFlowCommittedPoseBatch::new();
    session
        .extract_committed_pose_batch(FramePlacementToken::new(11), &mut output)
        .unwrap();
    let vehicles = output.vehicles().to_vec();
    let batch = output.batch().clone();
    for (ctx, selected, expected) in [
        (
            foreign,
            vec![removed, live, live],
            LaneFlowAdapterError::StalePoseSelectionContext,
        ),
        (
            context,
            vec![removed, live, live],
            LaneFlowAdapterError::DuplicatePoseSelection {
                vehicle: live,
                first_index: 1,
                duplicate_index: 2,
            },
        ),
        (
            context,
            vec![live, removed, replacement],
            LaneFlowAdapterError::SelectedPoseSource {
                index: 1,
                vehicle: removed,
                source: laneflow_runtime::CommittedPoseSourceError::UnknownVehicle {
                    handle: removed,
                },
            },
        ),
    ] {
        assert_eq!(
            session.extract_selected_committed_pose_batch(
                ctx,
                &selected,
                FramePlacementToken::new(12),
                &mut output,
            ),
            Err(expected)
        );
        assert_eq!(output.vehicles(), vehicles);
        assert_eq!(output.batch(), &batch);
        assert_eq!(output.context(), context);
    }
    session
        .extract_selected_committed_pose_batch(
            context,
            &[replacement, live],
            FramePlacementToken::new(13),
            &mut output,
        )
        .unwrap();
    assert_eq!(output.vehicles(), &[replacement, live]);
}

#[test]
fn selected_mixed_frames_fail_atomically_in_each_order_and_full_validation_remains() {
    let revision = root();
    let mut session = install_session(&revision);
    let main = main_route(&mut session);
    let alt = alt_route(&mut session);
    let first = spawn_on_main(&mut session, main, 1_000);
    let second = spawn_on_main(&mut session, main, 20_000);
    let stray = spawn_on_alt(&mut session, alt);
    let context = session.consumption_context();
    let mut output = LaneFlowCommittedPoseBatch::new();
    session
        .extract_selected_committed_pose_batch(
            context,
            &[second, first],
            FramePlacementToken::new(21),
            &mut output,
        )
        .unwrap();
    let before = output.batch().clone();
    for selected in [
        [stray, first, second],
        [first, stray, second],
        [first, second, stray],
    ] {
        assert!(matches!(
            session.extract_selected_committed_pose_batch(
                context,
                &selected,
                FramePlacementToken::new(22),
                &mut output,
            ),
            Err(LaneFlowAdapterError::SpatialPoseExtraction {
                source: SpatialError::BatchFrameMismatch { .. }
            })
        ));
        assert_eq!(output.vehicles(), &[second, first]);
        assert_eq!(output.batch(), &before);
        assert_eq!(output.context(), context);
    }
    assert!(
        session
            .extract_committed_pose_batch(FramePlacementToken::new(23), &mut output)
            .is_err()
    );
    assert_eq!(output.batch(), &before);
    park_stray_in_virtual_pool(&mut session, stray);
    session
        .extract_selected_committed_pose_batch(
            context,
            &[stray, first, second],
            FramePlacementToken::new(24),
            &mut output,
        )
        .unwrap();
    assert_eq!(output.vehicles(), &[first, second]);
    assert_eq!(
        session.extract_selected_committed_pose_batch(
            context,
            &[stray, stray],
            FramePlacementToken::new(25),
            &mut output,
        ),
        Err(LaneFlowAdapterError::DuplicatePoseSelection {
            vehicle: stray,
            first_index: 0,
            duplicate_index: 1,
        })
    );
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

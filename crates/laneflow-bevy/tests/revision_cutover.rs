//! #534 Adapter 修订绑定生命周期：十面验收矩阵。
//!
//! 夹具为同键同长、中心线整体平移的两条边——R1 位姿 y=0、R2 位姿 y=10，
//! 几何真值按平移量精确判别，不依赖修订号 stamp。

use std::{num::NonZeroU32, sync::Arc, time::Duration};

use bevy_app::App;
use bevy_ecs::{resource::Resource, schedule::IntoScheduleConfigs, system::ResMut};
use bevy_time::{TimePlugin, TimeUpdateStrategy};
use laneflow_bevy::{
    LaneFlowAdapterError, LaneFlowCommittedPoseBatch, LaneFlowOuterFrameSet, LaneFlowPlugin,
    LaneFlowSession, LaneFlowSessionConfig, LaneFlowTargetSpatial,
};
use laneflow_compiler::{
    CanonicalFrameInput, CanonicalPoint3F32Input, CompilationUnitBuilder, CompileLimits, Compiler,
    IidmVehicleProfileInput, LaneEdgeGeometryInput, LaneEdgeInput, LaneEdgeReference,
    ParticipantClassInput, ParticipantClassReference, PortableDiffBase, PortableEmissionProvenance,
    SourceModuleHeader, SourceModuleHeaderInput, SyntheticModuleBuilder, VehicleProfileInput,
    emit_portable_candidate,
};
use laneflow_format::{FormatLimits, check_post_emission_bundle, preflight_object_values};
use laneflow_runtime::{
    CommittedNetworkSource, CutoverPreflightLimits, CutoverTransactionLimits,
    PublishedLfcaReference, RouteHandle, RouteRegisterInput, SemanticDiffOriginBinding,
    TrafficWorld, VehicleHandle, VehicleSpawnInput, WorldConfig, WorldPolicySelection,
};
use laneflow_spatial::{CanonicalPoseBatch, FramePlacementToken, PoseInput, PoseRecordId};
use laneflow_static_contract::{
    ExactByteLength, LaneEdgeOrdinal, PortableObjectKind, SEMANTIC_DIFF_FORMAT_VERSION,
    Sha256Digest, VehicleProfileOrdinal,
};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};
use sha2::Digest as _;

/// R2 中心线相对 R1 的整体 Y 平移量（米）。
const OFFSET_R2_Y: f32 = 10.0;
const PREFLIGHT: CutoverPreflightLimits = CutoverPreflightLimits::new(16 * 1_024 * 1_024);

fn compile(frame_offset_y: f32) -> laneflow_compiler::CompilationOutput {
    let limits = CompileLimits::p100_initial_v1();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: "bevy/revision-cutover",
            source_document_key: "revision-cutover.document",
            generator_build_id: "git:0123456789abcdef",
            parameters_and_inputs_digest: [0x54; 32],
            frontend_options_digest: [0x31; 32],
            random_seed: Some(534),
            provenance: "repository:laneflow",
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
    module
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge-0",
            length_meters: 200.0,
            speed_limit_meters_per_second: 15.0,
            successors: &[LaneEdgeReference::local("edge-1")],
        })
        .expect("edge-0")
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: "edge-1",
            length_meters: 200.0,
            speed_limit_meters_per_second: 15.0,
            successors: &[],
        })
        .expect("edge-1");
    let point = |x: f32, y: f32| CanonicalPoint3F32Input { x, y, z: 0.0 };
    module
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: "frame-main",
            lane_edge_geometries: &[
                LaneEdgeGeometryInput {
                    lane_edge: LaneEdgeReference::local("edge-0"),
                    centerline_points: &[point(0.0, frame_offset_y), point(200.0, frame_offset_y)],
                },
                LaneEdgeGeometryInput {
                    lane_edge: LaneEdgeReference::local("edge-1"),
                    centerline_points: &[
                        point(200.0, frame_offset_y),
                        point(400.0, frame_offset_y),
                    ],
                },
            ],
        })
        .expect("frame");
    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_synthetic_module(module.finish().expect("module"))
        .expect("unit module");
    Compiler::new()
        .compile(unit.build().expect("unit"))
        .expect("compile")
}

struct Artifact {
    bytes: Vec<u8>,
    root: Arc<SharedNetworkRevision>,
    diff: Vec<u8>,
    binding: SemanticDiffOriginBinding,
}

fn emit(output: &laneflow_compiler::CompilationOutput, base: Option<&[u8]>) -> Artifact {
    let provenance =
        PortableEmissionProvenance::try_new("bevy-revision-cutover-v1").expect("provenance");
    let artifact = emit_portable_candidate(
        output,
        &provenance,
        FormatLimits::HARD,
        match base {
            None => PortableDiffBase::Genesis,
            Some(base) => PortableDiffBase::Artifact(
                preflight_object_values(
                    base,
                    PortableObjectKind::CanonicalArtifact,
                    FormatLimits::HARD,
                )
                .expect("base values"),
            ),
        },
    )
    .expect("artifact");
    let checked = check_post_emission_bundle(
        artifact.canonical_artifact().bytes(),
        artifact.source_map().bytes(),
        artifact.semantic_diff().bytes(),
        artifact.expected_semantic_diff_base(),
        FormatLimits::HARD,
    )
    .expect("bundle");
    let diff = artifact.semantic_diff().bytes().to_vec();
    let binding = SemanticDiffOriginBinding::new(
        SEMANTIC_DIFF_FORMAT_VERSION,
        Sha256Digest::from_bytes(sha2::Sha256::digest(&diff).into()),
        ExactByteLength::new(diff.len() as u64),
    );
    Artifact {
        bytes: artifact.canonical_artifact().bytes().to_vec(),
        root: build_shared_network_revision(
            checked.canonical_network_input(),
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::RetainAvailable,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .expect("root"),
        diff,
        binding,
    }
}

/// R1（y=0）、R2（y=OFFSET_R2_Y）与 ABA 回切用的「R1 内容、R2 为 diff base」制品。
struct Fixture {
    r1: Artifact,
    r2: Artifact,
    r1_again: Artifact,
}

fn fixture() -> Fixture {
    let r1 = emit(&compile(0.0), None);
    let r2 = emit(&compile(OFFSET_R2_Y), Some(&r1.bytes));
    let r1_again = emit(&compile(0.0), Some(&r2.bytes));
    assert_ne!(
        r1.root.canonical_origin().network_revision(),
        r2.root.canonical_origin().network_revision()
    );
    assert_eq!(
        r1.root.canonical_origin().network_revision(),
        r1_again.root.canonical_origin().network_revision(),
        "ABA 制品与 R1 内容相同（canonical 部分不受 diff base 影响）"
    );
    Fixture { r1, r2, r1_again }
}

fn source(root: &SharedNetworkRevision, key: &str) -> CommittedNetworkSource {
    let origin = root.canonical_origin();
    CommittedNetworkSource::Published {
        reference: PublishedLfcaReference::new(
            key,
            origin.canonical_artifact_digest(),
            origin.canonical_artifact_byte_length(),
            origin.network_revision(),
        )
        .expect("source"),
    }
}

struct Seeded {
    session: LaneFlowSession,
    route: RouteHandle,
    vehicle: VehicleHandle,
}

/// 单车世界：路线跨 edge-0 → edge-1，车辆静止在 edge-0 progress 1_000 mm。
fn seeded(artifact: &Artifact, key: &str) -> Seeded {
    let mut world = TrafficWorld::install(
        Arc::clone(&artifact.root),
        WorldConfig::new(4, 2, 1_024, 1_024, 1, 100),
        source(&artifact.root, key),
        534,
        WorldPolicySelection::NotRequired,
    )
    .expect("install");
    let route = world
        .register_route(RouteRegisterInput::new(vec![
            LaneEdgeOrdinal::from_raw(0),
            LaneEdgeOrdinal::from_raw(1),
        ]))
        .expect("route");
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            1_000,
            0,
        ))
        .expect("spawn");
    let spatial = laneflow_spatial::SpatialSession::bind(Arc::clone(&artifact.root))
        .expect("bind")
        .expect("spatial");
    Seeded {
        session: LaneFlowSession::new(
            world,
            Some(spatial),
            LaneFlowSessionConfig::new(NonZeroU32::new(8).expect("non-zero")),
        )
        .expect("session"),
        route,
        vehicle,
    }
}

/// 独立旧根消费者：与 Session 无共享可变状态，模拟跨提交边界的旧根借用。
fn old_root_consumer(artifact: &Fixture) -> laneflow_spatial::SpatialSession {
    laneflow_spatial::SpatialSession::bind(Arc::clone(&artifact.r1.root))
        .expect("bind")
        .expect("spatial")
}

/// 单次提取便捷封装；跨帧复用场景由调用方持有 `LaneFlowCommittedPoseBatch`。
fn extract_once(session: &mut LaneFlowSession) -> LaneFlowCommittedPoseBatch {
    let mut poses = LaneFlowCommittedPoseBatch::new();
    session
        .extract_committed_pose_batch(FramePlacementToken::new(1), &mut poses)
        .expect("extract");
    poses
}

fn pose_of_vehicle(
    result: &LaneFlowCommittedPoseBatch,
    vehicle: VehicleHandle,
) -> laneflow_spatial::CanonicalPoseF32 {
    let index = result
        .vehicles()
        .iter()
        .position(|candidate| *candidate == vehicle)
        .expect("vehicle in committed sources");
    result.batch().records()[index].pose()
}

/// 成功链：存活 Session 原地跨修订切换，身份、句柄、路线与 Entity 映射全保持。
#[test]
fn live_session_cross_revision_cutover_preserves_identity_and_mappings() {
    let fixture = fixture();
    let Seeded {
        mut session,
        route,
        vehicle,
    } = seeded(&fixture.r1, "fixture://bevy-cutover/r1");
    let mut scratch = bevy_ecs::world::World::new();
    let entity = scratch.spawn(()).id();
    drop(scratch);
    session.bind_vehicle_entity(vehicle, entity).expect("bind");
    let world_id = session.world().world_id();
    let generation = session.world().world_generation();

    let target_spatial = laneflow_spatial::SpatialSession::bind(Arc::clone(&fixture.r2.root))
        .expect("bind")
        .expect("spatial");
    let mut record = session
        .cross_revision_cutover(
            Arc::clone(&fixture.r2.root),
            source(&fixture.r2.root, "fixture://bevy-cutover/r2"),
            &fixture.r2.diff,
            fixture.r2.binding,
            LaneFlowTargetSpatial::Rebind(target_spatial),
            &PREFLIGHT,
            &CutoverTransactionLimits::default(),
        )
        .expect("cutover");

    // 身份保持、世代恰好 +1；句柄与 Entity 映射不变。
    assert_eq!(session.world().world_id(), world_id);
    assert_eq!(
        record.world_binding().world_generation().get(),
        generation.get() + 1
    );
    assert!(session.world().vehicle(vehicle).is_some());
    assert!(
        session
            .world()
            .live_routes()
            .any(|candidate| candidate == route),
        "RouteHandle 跨切换保持有效"
    );
    assert_eq!(session.vehicle_entity(vehicle), Some(entity));
    // 恰一次事件交付；换出的旧 Spatial 与根同源。
    assert!(!record.events().is_empty());
    let retired = record.retired_spatial().expect("retired spatial");
    assert!(Arc::ptr_eq(&retired.revision(), &fixture.r1.root));

    // 后续帧链路：app 驱动步进 + 封闭提取正常。
    assert_eq!(record.events().as_slice().len(), 1, "恰一次事件交付");
    drop(record);
    let mut app = App::new();
    app.add_plugins((TimePlugin, LaneFlowPlugin));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
        100,
    )));
    app.insert_resource(session);
    for _ in 0..2 {
        app.update();
    }
    let mut session = app
        .world_mut()
        .get_resource_mut::<LaneFlowSession>()
        .expect("session");
    assert!(session.frame_report().steps_run() > 0);
    let extracted = extract_once(&mut session);
    assert_eq!(
        extracted.batch().network_revision(),
        Some(fixture.r2.root.canonical_origin().network_revision())
    );
    assert_eq!(session.vehicle_entity(vehicle), Some(entity));
}

/// 几何真值：切换后首个位姿落在 R2 几何（y 平移），而非仅修订号变化。
#[test]
fn post_cutover_pose_lands_on_target_geometry() {
    let fixture = fixture();
    let Seeded {
        mut session,
        vehicle,
        ..
    } = seeded(&fixture.r1, "fixture://bevy-cutover/geometry");

    let before = extract_once(&mut session);
    let pose_before = pose_of_vehicle(&before, vehicle);
    assert_eq!(pose_before.position().y(), 0.0);

    let target_spatial = laneflow_spatial::SpatialSession::bind(Arc::clone(&fixture.r2.root))
        .expect("bind")
        .expect("spatial");
    let record = session
        .cross_revision_cutover(
            Arc::clone(&fixture.r2.root),
            source(&fixture.r2.root, "fixture://bevy-cutover/geometry-r2"),
            &fixture.r2.diff,
            fixture.r2.binding,
            LaneFlowTargetSpatial::Rebind(target_spatial),
            &PREFLIGHT,
            &CutoverTransactionLimits::default(),
        )
        .expect("cutover");
    assert!(!record.events().is_empty());

    let after = extract_once(&mut session);
    let pose_after = pose_of_vehicle(&after, vehicle);
    assert_eq!(
        pose_after.position().y(),
        OFFSET_R2_Y,
        "首个切换后位姿必须落在 R2 中心线上"
    );
    assert_eq!(pose_after.position().x(), pose_before.position().x());
}

/// 封闭路径每次现采当前 committed sources；不存在缓存旧输入重放的入口。
#[test]
fn closed_path_collects_current_sources_only() {
    let fixture = fixture();
    let Seeded {
        mut session,
        vehicle,
        ..
    } = seeded(&fixture.r1, "fixture://bevy-cutover/current-sources");

    let target_spatial = laneflow_spatial::SpatialSession::bind(Arc::clone(&fixture.r2.root))
        .expect("bind")
        .expect("spatial");
    let record = session
        .cross_revision_cutover(
            Arc::clone(&fixture.r2.root),
            source(
                &fixture.r2.root,
                "fixture://bevy-cutover/current-sources-r2",
            ),
            &fixture.r2.diff,
            fixture.r2.binding,
            LaneFlowTargetSpatial::Rebind(target_spatial),
            &PREFLIGHT,
            &CutoverTransactionLimits::default(),
        )
        .expect("cutover");
    assert!(!record.events().is_empty());

    let extracted = extract_once(&mut session);
    let current: Vec<VehicleHandle> = session
        .world()
        .committed_pose_sources()
        .as_slice()
        .iter()
        .map(|(handle, _)| *handle)
        .collect();
    assert_eq!(extracted.vehicles(), current.as_slice());
    assert!(extracted.vehicles().contains(&vehicle));
    assert_eq!(
        extracted.batch().network_revision(),
        Some(fixture.r2.root.canonical_origin().network_revision())
    );
}

/// 延迟旧结果：切换前采集的批次在切换后消费上下文过期，整批拒绝。
#[test]
fn stale_consumption_context_rejected_after_cutover() {
    let fixture = fixture();
    let Seeded {
        mut session,
        vehicle,
        ..
    } = seeded(&fixture.r1, "fixture://bevy-cutover/stale-context");

    let stale = extract_once(&mut session);
    assert!(session.consumption_context_is_current(stale.context()));

    let target_spatial = laneflow_spatial::SpatialSession::bind(Arc::clone(&fixture.r2.root))
        .expect("bind")
        .expect("spatial");
    let record = session
        .cross_revision_cutover(
            Arc::clone(&fixture.r2.root),
            source(&fixture.r2.root, "fixture://bevy-cutover/stale-context-r2"),
            &fixture.r2.diff,
            fixture.r2.binding,
            LaneFlowTargetSpatial::Rebind(target_spatial),
            &PREFLIGHT,
            &CutoverTransactionLimits::default(),
        )
        .expect("cutover");
    assert!(!record.events().is_empty());

    assert!(
        !session.consumption_context_is_current(stale.context()),
        "旧世代批次不得作为当前世界结果应用"
    );
    let fresh = extract_once(&mut session);
    assert!(session.consumption_context_is_current(fresh.context()));
    assert_eq!(pose_of_vehicle(&fresh, vehicle).position().y(), OFFSET_R2_Y);
}

/// 公开可变访问逃逸闭合：目标 Spatial 配错根在入口被拒，Session 与世界不变。
#[test]
fn target_spatial_mismatch_fails_closed_leaving_session_intact() {
    let fixture = fixture();
    let Seeded {
        mut session,
        vehicle,
        ..
    } = seeded(&fixture.r1, "fixture://bevy-cutover/mismatch");
    let generation = session.world().world_generation();

    // 绑定 R1 的 Session 冒充 R2 目标配对。
    let wrong = laneflow_spatial::SpatialSession::bind(Arc::clone(&fixture.r1.root))
        .expect("bind")
        .expect("spatial");
    let error = session
        .cross_revision_cutover(
            Arc::clone(&fixture.r2.root),
            source(&fixture.r2.root, "fixture://bevy-cutover/mismatch-r2"),
            &fixture.r2.diff,
            fixture.r2.binding,
            LaneFlowTargetSpatial::Rebind(wrong),
            &PREFLIGHT,
            &CutoverTransactionLimits::default(),
        )
        .expect_err("配错根必须失败关闭");
    assert_eq!(error, LaneFlowAdapterError::TargetSpatialRevisionMismatch);

    // 世界与配对原样：根、世代、无在途事务、提取照常。
    assert!(Arc::ptr_eq(&session.world().revision(), &fixture.r1.root));
    assert_eq!(session.world().world_generation(), generation);
    assert!(session.world().migration_journal_stats().is_none());
    assert!(session.world().vehicle(vehicle).is_some());
    extract_once(&mut session);
}

/// 同修订换根：修订号不变、根 Arc 换新、世代递增，配对随世代过期。
#[test]
fn same_revision_restore_rebinds_root_and_advances_generation() {
    let fixture = fixture();
    let Seeded {
        mut session,
        vehicle,
        ..
    } = seeded(&fixture.r1, "fixture://bevy-cutover/same-revision");
    let revision = session
        .world()
        .revision()
        .canonical_origin()
        .network_revision();
    let generation = session.world().world_generation();
    let before = extract_once(&mut session);

    // 字节相同的重发布制品：同修订、新 Arc 分配。
    let republish = emit(&compile(0.0), None);
    assert!(!Arc::ptr_eq(&republish.root, &fixture.r1.root));
    let target_spatial = laneflow_spatial::SpatialSession::bind(Arc::clone(&republish.root))
        .expect("bind")
        .expect("spatial");
    let record = session
        .same_revision_restore(
            Arc::clone(&republish.root),
            source(
                &republish.root,
                "fixture://bevy-cutover/same-revision-republish",
            ),
            LaneFlowTargetSpatial::Rebind(target_spatial),
            &PREFLIGHT,
        )
        .expect("same-revision restore");

    assert_eq!(
        session
            .world()
            .revision()
            .canonical_origin()
            .network_revision(),
        revision,
        "同修订换根不得改变修订号"
    );
    assert!(Arc::ptr_eq(&session.world().revision(), &republish.root));
    assert_eq!(
        record.world_binding().world_generation().get(),
        generation.get() + 1
    );
    assert!(!record.events().is_empty());
    assert!(
        !session.consumption_context_is_current(before.context()),
        "旧世代结果在同修订换根后同样过期"
    );
    let after = extract_once(&mut session);
    assert_eq!(pose_of_vehicle(&after, vehicle).position().y(), 0.0);
}

/// ABA：R1 → R2 → 回切 R1 内容后，g0/g1 世代的结果都不得复活。
#[test]
fn aba_return_to_first_root_rejects_stale_contexts() {
    let fixture = fixture();
    let Seeded {
        mut session,
        vehicle,
        ..
    } = seeded(&fixture.r1, "fixture://bevy-cutover/aba");

    let context_r1 = extract_once(&mut session).context();

    let to_r2 = laneflow_spatial::SpatialSession::bind(Arc::clone(&fixture.r2.root))
        .expect("bind")
        .expect("spatial");
    let record = session
        .cross_revision_cutover(
            Arc::clone(&fixture.r2.root),
            source(&fixture.r2.root, "fixture://bevy-cutover/aba-r2"),
            &fixture.r2.diff,
            fixture.r2.binding,
            LaneFlowTargetSpatial::Rebind(to_r2),
            &PREFLIGHT,
            &CutoverTransactionLimits::default(),
        )
        .expect("cutover to R2");
    assert!(!record.events().is_empty());
    let context_r2 = extract_once(&mut session).context();

    // 回切到 R1 内容（以 R2 为 diff base 的制品；目标根 Arc 是新分配）。
    let back = laneflow_spatial::SpatialSession::bind(Arc::clone(&fixture.r1_again.root))
        .expect("bind")
        .expect("spatial");
    let record = session
        .cross_revision_cutover(
            Arc::clone(&fixture.r1_again.root),
            source(&fixture.r1_again.root, "fixture://bevy-cutover/aba-back"),
            &fixture.r1_again.diff,
            fixture.r1_again.binding,
            LaneFlowTargetSpatial::Rebind(back),
            &PREFLIGHT,
            &CutoverTransactionLimits::default(),
        )
        .expect("cutover back to R1");
    assert!(!record.events().is_empty());

    assert_eq!(
        session
            .world()
            .revision()
            .canonical_origin()
            .network_revision(),
        fixture.r1.root.canonical_origin().network_revision(),
        "ABA 终态修订号回到 R1"
    );
    assert!(!session.consumption_context_is_current(context_r1));
    assert!(!session.consumption_context_is_current(context_r2));
    let current = extract_once(&mut session);
    assert!(session.consumption_context_is_current(current.context()));
    assert_eq!(pose_of_vehicle(&current, vehicle).position().y(), 0.0);
}

/// 失败原子性与重试：Runtime prepare 失败被透传、世界无在途事务，合法重试成功。
#[test]
fn failed_prepare_settles_and_retry_succeeds() {
    let fixture = fixture();
    let Seeded {
        mut session,
        vehicle,
        ..
    } = seeded(&fixture.r1, "fixture://bevy-cutover/retry");
    let generation = session.world().world_generation();

    // 摘要与字节不一致的伪造绑定。
    let corrupt = SemanticDiffOriginBinding::new(
        SEMANTIC_DIFF_FORMAT_VERSION,
        Sha256Digest::from_bytes([0xBA; 32]),
        ExactByteLength::new(fixture.r2.diff.len() as u64),
    );
    let target_spatial = laneflow_spatial::SpatialSession::bind(Arc::clone(&fixture.r2.root))
        .expect("bind")
        .expect("spatial");
    let error = session
        .cross_revision_cutover(
            Arc::clone(&fixture.r2.root),
            source(&fixture.r2.root, "fixture://bevy-cutover/retry-r2"),
            &fixture.r2.diff,
            corrupt,
            LaneFlowTargetSpatial::Rebind(target_spatial),
            &PREFLIGHT,
            &CutoverTransactionLimits::default(),
        )
        .expect_err("伪造摘要必须失败");
    assert!(matches!(error, LaneFlowAdapterError::Cutover { .. }));

    assert!(Arc::ptr_eq(&session.world().revision(), &fixture.r1.root));
    assert_eq!(session.world().world_generation(), generation);
    assert!(
        session.world().migration_journal_stats().is_none(),
        "失败路径不得遗留在途事务"
    );
    assert!(session.world().vehicle(vehicle).is_some());

    // 合法重试成功。
    let target_spatial = laneflow_spatial::SpatialSession::bind(Arc::clone(&fixture.r2.root))
        .expect("bind")
        .expect("spatial");
    let record = session
        .cross_revision_cutover(
            Arc::clone(&fixture.r2.root),
            source(&fixture.r2.root, "fixture://bevy-cutover/retry-r2"),
            &fixture.r2.diff,
            fixture.r2.binding,
            LaneFlowTargetSpatial::Rebind(target_spatial),
            &PREFLIGHT,
            &CutoverTransactionLimits::default(),
        )
        .expect("retry succeeds");
    assert!(!record.events().is_empty());
    assert_eq!(
        session.world().world_generation().get(),
        generation.get() + 1
    );
}

/// 旧根借用完成与回收：独立旧根消费者跨提交边界完成读取；持有者退出后引用释放。
#[test]
fn old_root_borrow_completes_after_cutover() {
    let fixture = fixture();
    let Seeded { mut session, .. } = seeded(&fixture.r1, "fixture://bevy-cutover/old-borrow");
    let mut old_consumer = old_root_consumer(&fixture);

    let target_spatial = laneflow_spatial::SpatialSession::bind(Arc::clone(&fixture.r2.root))
        .expect("bind")
        .expect("spatial");
    let mut record = session
        .cross_revision_cutover(
            Arc::clone(&fixture.r2.root),
            source(&fixture.r2.root, "fixture://bevy-cutover/old-borrow-r2"),
            &fixture.r2.diff,
            fixture.r2.binding,
            LaneFlowTargetSpatial::Rebind(target_spatial),
            &PREFLIGHT,
            &CutoverTransactionLimits::default(),
        )
        .expect("cutover");
    let retired = record.retired_spatial().expect("retired");
    let references_after_cutover = Arc::strong_count(&fixture.r1.root);

    // 旧根在途借用可完成，且 stamp 旧修订。
    let mut batch = CanonicalPoseBatch::new();
    let inputs = [PoseInput::lane(
        PoseRecordId::new(0),
        LaneEdgeOrdinal::from_raw(0),
        1_000,
    )];
    old_consumer
        .extract_pose_batch(FramePlacementToken::new(1), &inputs, &mut batch)
        .expect("old root borrow completes");
    assert_eq!(
        batch.network_revision(),
        Some(fixture.r1.root.canonical_origin().network_revision())
    );
    assert_eq!(batch.records()[0].pose().position().y(), 0.0);
    assert!(Arc::ptr_eq(&retired.revision(), &fixture.r1.root));

    drop(retired);
    drop(old_consumer);
    assert!(
        Arc::strong_count(&fixture.r1.root) < references_after_cutover,
        "最后合法持有者退出后旧根引用释放"
    );
}

/// 调度：Administration 阶段零步进帧可达；catch-up 帧不重复执行切换请求。
#[derive(Resource)]
struct AdminCutover {
    target: Arc<SharedNetworkRevision>,
    source: CommittedNetworkSource,
    diff: Vec<u8>,
    binding: SemanticDiffOriginBinding,
    target_spatial: Option<laneflow_spatial::SpatialSession>,
    executed: bool,
}

fn admin_cutover(mut session: ResMut<LaneFlowSession>, mut request: ResMut<AdminCutover>) {
    if request.executed {
        return;
    }
    let target_spatial = request
        .target_spatial
        .take()
        .expect("行政切换请求只执行一次");
    let record = session
        .cross_revision_cutover(
            Arc::clone(&request.target),
            request.source.clone(),
            &request.diff,
            request.binding,
            LaneFlowTargetSpatial::Rebind(target_spatial),
            &PREFLIGHT,
            &CutoverTransactionLimits::default(),
        )
        .expect("administrative cutover");
    assert!(!record.events().is_empty());
    request.executed = true;
}

#[test]
fn administration_runs_on_zero_step_frames_and_cutover_not_duplicated_in_catch_up() {
    let fixture = fixture();
    let Seeded { session, .. } = seeded(&fixture.r1, "fixture://bevy-cutover/scheduling");
    let generation = session.world().world_generation();
    let target_spatial = laneflow_spatial::SpatialSession::bind(Arc::clone(&fixture.r2.root))
        .expect("bind")
        .expect("spatial");

    // 零步进帧：accumulator 不进位，fixed schedule 整体不运行，切换仍执行。
    let mut app = App::new();
    app.add_plugins((TimePlugin, LaneFlowPlugin));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    app.insert_resource(session);
    app.insert_resource(AdminCutover {
        target: Arc::clone(&fixture.r2.root),
        source: source(&fixture.r2.root, "fixture://bevy-cutover/scheduling-r2"),
        diff: fixture.r2.diff.clone(),
        binding: fixture.r2.binding,
        target_spatial: Some(target_spatial),
        executed: false,
    });
    app.add_systems(
        laneflow_bevy::LaneFlowOuterFrame,
        admin_cutover.in_set(LaneFlowOuterFrameSet::Administration),
    );
    app.update();
    {
        let session = app.world().resource::<LaneFlowSession>();
        assert_eq!(session.frame_report().steps_run(), 0, "零步进帧");
        assert_eq!(
            session.world().world_generation().get(),
            generation.get() + 1,
            "零步进帧也必须完成维护切换"
        );
        assert!(app.world().resource::<AdminCutover>().executed);
    }

    // catch-up 帧：单帧多次步进，行政阶段仍每 outer frame 恰一次。
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
        300,
    )));
    for _ in 0..3 {
        app.update();
    }
    let session = app.world().resource::<LaneFlowSession>();
    assert!(
        session.frame_report().steps_run() > 1,
        "catch-up 帧多次步进"
    );
    assert_eq!(
        session.world().world_generation().get(),
        generation.get() + 1,
        "切换请求不得随 catch-up step 重复执行"
    );
    assert!(Arc::ptr_eq(&session.world().revision(), &fixture.r2.root));
}

/// 目标无 Spatial 政策：显式 Headless 切换合法，提取失败关闭。
#[test]
fn explicit_headless_cutover_retires_spatial_and_rejects_extraction() {
    let fixture = fixture();
    let Seeded { mut session, .. } = seeded(&fixture.r1, "fixture://bevy-cutover/headless");

    let mut record = session
        .cross_revision_cutover(
            Arc::clone(&fixture.r2.root),
            source(&fixture.r2.root, "fixture://bevy-cutover/headless-r2"),
            &fixture.r2.diff,
            fixture.r2.binding,
            LaneFlowTargetSpatial::Headless,
            &PREFLIGHT,
            &CutoverTransactionLimits::default(),
        )
        .expect("explicit headless cutover");
    assert!(record.retired_spatial().is_some());
    assert!(session.spatial().is_none());
    let mut poses = LaneFlowCommittedPoseBatch::new();
    assert!(matches!(
        session.extract_committed_pose_batch(FramePlacementToken::new(1), &mut poses),
        Err(LaneFlowAdapterError::PoseExtractionWithoutSpatial)
    ));
}

/// 公开 set 不变量（adapter-api §9 / bevy 文档 §4）：无 Session 时 Administration
/// 安全跳过、schedule 无操作不 panic；插入 Session 后恢复执行。
#[derive(Resource)]
struct AdminProbeRan(bool);

fn admin_probe(mut _session: ResMut<LaneFlowSession>, mut ran: ResMut<AdminProbeRan>) {
    ran.0 = true;
}

#[test]
fn administration_set_is_safe_without_session_and_runs_with_session() {
    let fixture = fixture();
    let Seeded { session, .. } = seeded(&fixture.r1, "fixture://bevy-cutover/no-session-guard");
    let mut app = App::new();
    app.add_plugins((TimePlugin, LaneFlowPlugin));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
        16,
    )));
    app.insert_resource(AdminProbeRan(false));
    app.add_systems(
        laneflow_bevy::LaneFlowOuterFrame,
        admin_probe.in_set(LaneFlowOuterFrameSet::Administration),
    );
    // 无 Session：行政系统按 resource_exists 门控跳过，不 panic。
    app.update();
    assert!(!app.world().resource::<AdminProbeRan>().0);
    // 插入 Session 后恢复执行。
    app.insert_resource(session);
    app.update();
    assert!(app.world().resource::<AdminProbeRan>().0);
}

/// 切换记录消费语义：事件批次恰一次交付，跨帧复用缓冲在稳态保持容量。
#[test]
fn cutover_events_are_delivered_once_and_buffer_capacity_is_retained() {
    let fixture = fixture();
    let Seeded { mut session, .. } = seeded(&fixture.r1, "fixture://bevy-cutover/events-once");

    let target_spatial = laneflow_spatial::SpatialSession::bind(Arc::clone(&fixture.r2.root))
        .expect("bind")
        .expect("spatial");
    let record = session
        .cross_revision_cutover(
            Arc::clone(&fixture.r2.root),
            source(&fixture.r2.root, "fixture://bevy-cutover/events-once-r2"),
            &fixture.r2.diff,
            fixture.r2.binding,
            LaneFlowTargetSpatial::Rebind(target_spatial),
            &PREFLIGHT,
            &CutoverTransactionLimits::default(),
        )
        .expect("cutover");
    let events = record.events();
    assert_eq!(
        events.as_slice().len(),
        1,
        "v1 恰一次 RevisionCutoverCommitted"
    );

    // 同一缓冲两次提取：容量不缩减（稳定容量合同的最小断言）。
    let mut poses = LaneFlowCommittedPoseBatch::new();
    session
        .extract_committed_pose_batch(FramePlacementToken::new(1), &mut poses)
        .expect("first extract");
    let capacity = poses.vehicles().len();
    assert!(capacity > 0);
    session
        .extract_committed_pose_batch(FramePlacementToken::new(1), &mut poses)
        .expect("second extract");
    assert_eq!(poses.vehicles().len(), capacity);
}
